//! Wind-driven sea state of enhanced water: the open-water swell trains and
//! the shore train, worked out once a frame on the CPU from the wind and
//! shared as is with the shaders (`WaterSea` in water_spectrum.wgsl), so the
//! drawn surface, the caustics, the buoyancy and the visual contacts meet.
//!
//! The Legacy Viewer has no such thing: its water only scrolls the region's
//! two normal maps (`wave1`/`wave2`), which this leaves as they are.
//!
//! Each train's phase is integrated over time instead of `k·x − ω·t`: when
//! the wind lengthens or turns the waves, they only speed up or turn, they
//! never race through the phases they would have had since the clock began.
//! Their spatial phase is anchored on the camera, moved every frame with
//! the phases taking the move, so a changing wavelength stretches the sea
//! around the viewer instead of around the region's corner.

use crate::water_effects::OPEN_DEPTH;
use crate::water_fft::{OCEAN_TILES, Ocean, OceanMaps, Spectrum};
use glam::Vec2;
use std::sync::Arc;

/// Open-water trains (`WaterEffects::quality` draws 2 to 4 of them).
pub const SEA_TRAINS: usize = 4;
const G: f32 = 9.81;
const TAU: f32 = std::f32::consts::TAU;
/// Wavelength and steepness of each train, relative to the first, and its
/// angle off the wind at full spread (radians).
const TRAIN_LENGTH: [f32; SEA_TRAINS] = [1.0, 0.63, 0.41, 0.27];
const TRAIN_STEEPNESS: [f32; SEA_TRAINS] = [1.0, 0.8, 0.57, 0.39];
const TRAIN_ANGLE: [f32; SEA_TRAINS] = [0.0, 0.45, -0.75, 1.3];
/// The highest the swash runs up a beach above the still water (m).
pub const SWASH_REACH: f32 = 1.0;
/// Wind speed (m/s) at which the sea is as rough as it gets.
pub const STORM_WIND: f32 = 20.0;

/// What the shaders read (`WaterSea`): 11 × vec4.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Sea {
    /// Per open train: travel direction xy (unit), wavenumber (rad/m),
    /// amplitude (m).
    pub trains: [[f32; 4]; SEA_TRAINS],
    /// Per open train: phase at `frame[2]` (rad, in 0..2π).
    pub phase: [f32; 4],
    /// Per open train: angular frequency (rad/s).
    pub omega: [f32; 4],
    /// x: how far the sea grid's crossfade has gone (0 the previous grid,
    /// 1 the new one, `WaterGround::sea_prev`); y, z: the spectrum's
    /// cascades' tiles (m); w: its significant height (m), 0 without one
    /// (the wind chop is then trains 1 to 3).
    pub shore: [f32; 4],
    /// Spatial anchor xy (m), epoch (the frame clock the phases are at),
    /// roughness 0 (glassy) to 1 (storm).
    pub frame: [f32; 4],
    /// Wind it was worked out for: direction it blows toward xy (unit),
    /// speed (m/s), the highest crest the sea stacks (m).
    pub wind: [f32; 4],
    /// x: how much foam the whitecaps make (`SeaTune::foam`, 1 by
    /// default); y: the scale of the spectrum's chop (`SeaTune::chop` under
    /// the height limit; its maps are drawn unscaled, so a change shows the
    /// same frame); z, w: how far the caustics' ripples have drifted with
    /// the wind (m, `caustic_drift`).
    pub look: [f32; 4],
    /// The lakes' cascade (water_fft.rs `LAKE`): x its tile (m, 0 without
    /// a spectrum), y its significant height (m, scaled as `shore[3]`).
    pub lake: [f32; 4],
}

/// How much of the open sea's two cascades (x) and of the lakes' (y) the
/// wind sea takes where the sea grid's exposure gives `chop` (its packed
/// fetch: water_shelter.rs `exposure`, sqrt(fetch / 800 m) times 0.6 to 1
/// with the swell that comes in, up to 1). Open water keeps the open sea;
/// over a short fetch it gives way to the lakes' chop, as high as the
/// fetch lets it grow (a fetch-limited sea's height grows as its square
/// root: the lakes' cascade is a sea `LAKE_FETCH` across, which `chop`
/// 0.6 stands for). Same as `water_ocean_weights`.
pub fn ocean_weights(chop: f32) -> [f32; 2] {
    let open = smoothstep(0.55, 0.95, chop);
    [open, (1.0 - open) * (chop / 0.6).clamp(0.0, 1.5)]
}

/// How much of the main wave is the wind sea's own (`Sea::lake_wave`)
/// rather than the open sea's swell, where `shadow` of the swell reaches
/// (`SwellPoint::shadow`): all of it where none does (a lake, a closed
/// cove), none where it comes in. The two are never drawn together: the
/// swell fades out to the middle of the way, then the lake's wave in (each
/// with its own phase, a mix would beat). Same as `water_lake_share`.
pub fn lake_share(shadow: f32) -> f32 {
    1.0 - smoothstep(0.02, 0.2, shadow)
}

/// How much of the chop the wind sea's own wave takes over as it lines up
/// in the shallows (`Sea::main`): its peak, about half of the chop's
/// energy; the shorter waves go on riding it. (Taking all of it left the
/// water before a lake's shore glassy.) Same as in `water_sea_main`.
const LAKE_TAKE: f32 = 0.5;

impl Default for Sea {
    /// A light breeze, as the sea looked before the wind drove it.
    fn default() -> Self {
        let mut sea: Self = bytemuck::Zeroable::zeroed();
        sea.look[0] = 1.0;
        sea.look[1] = 1.0;
        sea.shape(Vec2::new(0.88, 0.475), 5.0);
        sea
    }
}

/// How fast the caustics' ripples drift downwind (m/s) on a sea of
/// `rough`ness: a slow wander when glassy, a quick one in a gale.
pub fn caustic_drift(rough: f32) -> f32 {
    0.08 + 0.5 * rough.clamp(0.0, 1.0)
}

/// How hard the ripples' lenses focus the sun into caustics
/// (water_caustics.wgsl), on a sea of `rough`ness whose chop is scaled by
/// `chop`: soft, broad light on glassy water, sharp tight lines under a
/// breeze, tighter in a gale (whose light the bed's lookup then breaks up).
pub fn caustic_focus(rough: f32, chop: f32) -> f32 {
    let rough = if rough.is_finite() { rough.clamp(0.0, 1.0) } else { 0.0 };
    let chop = if chop.is_finite() { chop.clamp(0.0, 2.0) } else { 1.0 };
    (0.5 + 0.6 * rough) * (0.6 + 0.4 * chop.min(1.5))
}

impl Sea {
    /// Roughness 0..1 for a wind speed (m/s).
    pub fn roughness(speed: f32) -> f32 {
        let t = ((speed - 1.0) / (STORM_WIND - 3.0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Lay the trains out for a wind blowing toward `heading` (unit) at
    /// `speed` m/s, keeping the phases. Stylised on a fully developed sea
    /// (peak wavelength growing with the wind, Pierson-Moskowitz), capped
    /// for regions of 256 m: a breeze makes ~30 m swells of a quarter
    /// metre, a gale ~90 m ones of two metres, with whitecaps.
    fn shape(&mut self, heading: Vec2, speed: f32) {
        self.shape_crossed(heading, heading, speed, speed);
    }

    /// `shape` with the main swell going toward `swell` and the wind chop
    /// toward `heading` (the wind now): a swell keeps the direction of the
    /// wind that raised it for a while after the wind turns.
    /// `swell_speed`: the wind the main swell was raised by (the wind of the
    /// last minutes: a gust changes the chop, never the swell's length).
    fn shape_crossed(&mut self, swell: Vec2, heading: Vec2, speed: f32, swell_speed: f32) {
        let speed = speed.max(0.0);
        let rough = Self::roughness(speed);
        // the trains fan out from the wind more in light airs
        let spread = 1.0 - 0.55 * rough;
        for i in 0..SEA_TRAINS {
            let raised = if i == 0 { swell_speed.max(0.0) } else { speed };
            let rough = Self::roughness(raised);
            let peak = (6.0 + 2.2 * raised.powf(1.5)).min(90.0);
            // k·a: nearly flat when calm, the folding crests of a gale (the
            // Gerstner jacobian, so the crest foam) when not
            let steep = 0.005 + 0.3 * rough * (1.0 - 0.45 * rough);
            let k = TAU / (peak * TRAIN_LENGTH[i]);
            let a = (steep * TRAIN_STEEPNESS[i] / k).min(1.0);
            let d = Vec2::from_angle(TRAIN_ANGLE[i] * spread).rotate(if i == 0 { swell } else { heading });
            self.trains[i] = [d.x, d.y, k, a];
            self.omega[i] = (G * k).sqrt();
        }
        // (shore[0], the sea grid's crossfade, is the scene's to set; the
        // rest the spectrum's, SeaDriver::advance)
        self.shore[1..].fill(0.0);
        // the lake's wave: the peak of the chop over the lakes' fetch,
        // keeping its phase
        let peak = crate::water_fft::jonswap_peak(speed.max(1.0), crate::water_fft::LAKE_FETCH);
        self.lake = [0.0, 0.0, peak * peak / G, self.lake[3]];
        self.frame[3] = rough;
        // the highest crest the trains can stack (the main swell growing up
        // to twice in the shallows), for the shelter's cap
        let peak: f32 = self.trains.iter().map(|t| t[3]).sum::<f32>() + self.trains[0][3];
        self.wind = [heading.x, heading.y, speed, peak];
    }

    /// The wind chop's amplitude (m) the depth caps: half the spectrum's
    /// significant height (the open sea's and the lakes' by their shares,
    /// `weights`: `ocean_weights`), or the trains'. Same as in `water_sea`.
    fn chop_height(&self, weights: [f32; 2]) -> f32 {
        if self.shore[3] > 0.0 {
            0.5 * (self.shore[3] * weights[0] + self.lake[1] * weights[1])
        } else {
            self.trains[1..].iter().map(|t| t[3]).sum()
        }
    }

    /// How much of the sea a `clearance` leaves (0..1): the metres from
    /// the still water up to the lowest underside above it, or the top of
    /// what crosses it (a floor at the water); crests stay under 60 % of it.
    /// Same as `water_sea_cap`.
    pub fn clearance_cap(&self, clearance: f32) -> f32 {
        (0.6 * clearance.max(0.0) / self.wind[3].max(0.01)).min(1.0)
    }

    fn anchor(&self) -> Vec2 {
        Vec2::new(self.frame[0], self.frame[1])
    }

    /// The wind sea's own wave, where the open sea's swell does not reach
    /// (`lake_share`): the peak of the chop a wind raises over a lake
    /// (`water_fft::LAKE_FETCH`; 1 to 4.5 m long from a light air to a
    /// gale), running downwind. Its way (unit), wavenumber (rad/m) and
    /// angular frequency (rad/s). Same as `water_lake_wave`.
    pub fn lake_wave(&self) -> (Vec2, f32, f32) {
        let dir = Vec2::new(self.wind[0], self.wind[1]).normalize_or(Vec2::X);
        let k = self.lake[2].max(1e-3);
        (dir, k, (G * k).sqrt())
    }

    /// The main wave where the sea is read as `at`, with `shadow` of the
    /// open sea's swell, `rest` the wind chop's amplitude (`chop_height`)
    /// and `reach_chop` its share there (`SwellPoint::chop`). Same as
    /// `water_sea_main`.
    ///
    /// The open sea's swell, dimmed by `shadow`; or where it does not reach,
    /// the wind sea's own peak wave (`lake_wave`): in deep water the chop's
    /// spectrum draws it, but as the water shoals under half its length its
    /// crests line up into one train, which grows in while the chop gives
    /// way to it (`take`), and shoals, breaks and runs up the downwind
    /// shores as the swell does on the sea's.
    fn main(&self, at: &SeaPoint, shadow: f32, rest: f32, reach_chop: f32) -> Main {
        let lake = at.swell.map_or(0.0, |s| s.lake).clamp(0.0, 1.0);
        let a0 = self.trains[0][3];
        let ocean = (1.0 - 2.0 * lake).max(0.0);
        let own = (2.0 * lake - 1.0).max(0.0);
        let (_, k, omega) = self.lake_wave();
        let length = TAU / k;
        let shoal = 1.0 - smoothstep(0.25 * length, 0.5 * length, at.depth);
        // (the trains' chop takes the fetch's share in, the spectrum's has
        // it in its cascades' shares)
        let base = if self.shore[3] > 0.0 { rest } else { rest * reach_chop };
        if own > 0.0 {
            Main {
                amp: base * shoal * own,
                height: 2.0 * base,
                length,
                take: LAKE_TAKE * shoal * own,
                phase: self.lake[3],
                omega,
            }
        } else {
            Main {
                amp: a0 * shadow * ocean,
                height: 2.0 * a0,
                length: TAU / self.trains[0][2].max(1e-4),
                take: 0.0,
                phase: self.phase[0],
                omega: self.omega[0],
            }
        }
    }

    /// The main swell where the sea is read as `at`, at `xy`: its spatial
    /// phase there, its local wave vector and the island shadow on it. The
    /// refracted swell of the sea grid (`SwellPoint`), or out of it the
    /// open sea's plane wave.
    fn swell_at(&self, xy: Vec2, at: &SeaPoint) -> SwellPoint {
        at.swell.unwrap_or_else(|| {
            let [dx, dy, k, _] = self.trains[0];
            let d = Vec2::new(dx, dy);
            SwellPoint {
                phase: (xy - self.anchor()).dot(d) * k,
                wave: d * k,
                shadow: 1.0,
                chop: 1.0,
                lake: 0.0,
            }
        })
    }

    /// How the sea meets the bed where it is read as `at` (same as
    /// `water_sea_local`). The main swell, refracted to face the shore
    /// (water_shelter.rs), grows as the water shallows (Green's law, up to
    /// twice), dims behind islands, is capped by the depth (a crest at most
    /// 0.28 d) and breaks by the beach's surf similarity; in the last
    /// decimetres it runs up the beach as swash. The shorter wind chop is
    /// capped by the depth too, and dims less behind islands (the wind
    /// raises it again).
    pub fn local(&self, at: &SeaPoint, shadow: f32, reach_chop: f32) -> Local {
        let d = at.depth;
        let slope = at.slope.length();
        // the open sea's chop, or the lakes' over a short fetch
        let weights = ocean_weights(reach_chop);
        let rest = self.chop_height(weights);
        // the swell, or the wind sea's own wave where it does not reach
        let wave_now = self.main(at, shadow, rest, reach_chop);
        let surf = surf_similarity(slope, wave_now.height, wave_now.length);
        let plunge = smoothstep(0.4, 1.2, surf);
        let crash = smoothstep(2.5, 4.0, surf);
        let dry = smoothstep(0.08, 0.3, d);
        let grown = wave_now.amp * (OPEN_DEPTH / d.max(0.05)).powf(0.25).clamp(1.0, 2.0);
        let cap = 0.28 * d.max(0.0);
        // (a wave a few centimetres high laps; it does not break white, however
        // shallow the water: without this a lake's ripples ringed it with surf)
        let breaking = smoothstep(0.35 + 0.25 * plunge, 1.0, grown / cap.max(1e-3)) * dry * smoothstep(0.03, 0.15, grown);
        let wall = crash * (1.0 - smoothstep(0.5, 6.0, d));
        // shelter (piers, walls) holds the swell back; the wind still
        // ruffles a harbour, so its chop keeps more of itself. Under a deck
        // or a floor the clearance caps all of it.
        let shelter = at.shelter.clamp(0.0, 1.0);
        let held = at.cap.clamp(0.0, 1.0);
        let reach = (wave_now.amp * (0.6 + 0.6 * plunge) * (1.0 - 0.7 * crash)).min(SWASH_REACH) * shelter * held;
        // the swash runs up a beach that faces the swell (none over the
        // back of a sandbar or a land bridge), its whole height up to the
        // highest it reaches, then no further: the water seen at the top
        // of the beach is the swash's own front, which slides back down
        let wave = at
            .swell
            .map_or(Vec2::new(self.trains[0][0], self.trains[0][1]) * self.trains[0][2], |s| s.wave);
        // (a gentle slope only: a cliff takes the waves as a crash; same as
        // `water_swash_beach`)
        let facing = if slope > 1e-4 {
            smoothstep(0.0, 0.35, -wave.dot(at.slope) / (wave.length() * slope).max(1e-6))
        } else {
            0.0
        };
        let beach = smoothstep(0.004, 0.02, slope) * (1.0 - smoothstep(0.3, 0.7, slope)) * facing;
        // (and a little way into the shallows: the swash rushes up from the
        // water, never lowering it under the swell's own crest there)
        // (by the real bed: on the smoothed depth the swash stopped short of
        // the sand, or ran over it, by tens of centimetres)
        let b = at.bed;
        let swash = beach * smoothstep(-reach - 0.25, -reach, b) * (1.0 - smoothstep(0.3, 0.9, b));
        // (and it dies out over the last two metres of the real bed: by
        // the smoothed depth alone, a cliff's foot kept its whole chop,
        // whose crests and sway climbed the land beside it)
        // (and gives way to the wind sea's own wave as it lines up in the
        // shallows, `Main::take`)
        let left = (cap / rest.max(0.01)).min(1.0) * dry * shelter.sqrt() * held * smoothstep(0.1, 2.0, at.bed) * (1.0 - wave_now.take);
        Local {
            phase: wave_now.phase,
            omega: wave_now.omega,
            // against what stands steep (rocks, a cliff, a wall) the swell is
            // thrown back on itself and stands up the face instead of
            // shoaling out: higher, and less held down by the depth
            main: (grown * (1.0 + 0.7 * wall)).min(cap * (1.0 + 1.5 * wall)) * dry * shelter * held,
            chop: reach_chop * left,
            weights: [weights[0], weights[0], weights[1]],
            spectrum: left,
            breaking,
            lean: 0.75 * breaking * (0.4 + 0.6 * plunge),
            swash,
            reach,
            surf,
            // the swell's phase where it meets the shoreline just down the
            // beach (same as `water_swash_shift`): the whole swash runs up
            // and drains back as one sheet, instead of each point taking
            // its own phase (refraction shortens the swell to a dozen
            // metres there, which drew bands sliding up the sand)
            slide: smoothstep(-0.2, 1.5, at.bed),
            shore_shift: if slope > 0.0 {
                wave.dot(at.slope / slope.max(1e-4)) * (-b / slope.max(0.01)).clamp(-60.0, 60.0)
            } else {
                0.0
            },
        }
    }

    /// The main swell's phase at `xy` at `time` where the sea is read as
    /// `at` (rad; its crest at π/2, the phase falling as time goes on): for
    /// what happens as a crest arrives (water_crash.rs).
    pub fn crest_phase(&self, xy: Vec2, time: f32, at: &SeaPoint) -> f32 {
        let swell = self.swell_at(xy, at);
        // (the wind sea's own wave where the swell does not reach, `main`)
        let (phase, omega) = if swell.lake > 0.5 {
            (self.lake[3], self.lake_wave().2)
        } else {
            (self.phase[0], self.omega[0])
        };
        swell.phase - (phase + omega * (time - self.frame[2]))
    }

    /// The trains' Gerstner offset (xyz) at `p` near `xy`, where the sea is
    /// read as `at`, as `water_swell` draws it: the main swell from its
    /// phase at `xy` and local wave vector, leaning forward as it breaks,
    /// becoming the swash on the beach; the wind chop as plane waves.
    fn open(
        &self,
        p: Vec2,
        xy: Vec2,
        time: f32,
        trains: usize,
        swell: &SwellPoint,
        local: &Local,
        ocean: Option<&OceanMaps>,
    ) -> glam::Vec3 {
        let anchor = self.anchor();
        let since = time - self.frame[2];
        let mut offset = glam::Vec3::ZERO;
        let k = swell.wave.length().max(1e-4);
        let theta = swell.phase + swell.wave.dot(p - xy) - (local.phase + local.omega * since);
        let x = theta - local.lean * theta.sin();
        let dir = swell.wave / k;
        let main = (dir * theta.cos() * local.main * 0.85).extend(local.main * x.sin());
        let run = swash_rise(theta + local.shore_shift) * local.reach;
        offset += (main.truncate() * (1.0 - local.swash) * local.slide).extend(main.z + (run - main.z).max(0.0) * local.swash);
        if self.shore[3] > 0.0 {
            // the spectrum's wind sea (water_fft.rs); none without its maps
            // (its cascades by the fetch here: the open sea's, or a lake's)
            let Some(at) = ocean.map(|o| o.sample_weighted(p, local.weights)) else {
                return offset;
            };
            let scale = local.spectrum * self.look[1];
            let sway = chop_sway(at.jacobian * scale);
            return offset + (at.offset.truncate() * scale * sway).extend(at.offset.z * scale);
        }
        for i in 1..trains.min(SEA_TRAINS) {
            let [dx, dy, k, a] = self.trains[i];
            let d = Vec2::new(dx, dy);
            let a = a * local.chop;
            let phase = (p - anchor).dot(d) * k - (self.phase[i] + self.omega[i] * since) + i as f32 * 1.73;
            offset += (d * phase.cos() * a * 0.85).extend(phase.sin() * a);
        }
        offset
    }

    /// The surf similarity (Iribarren number) of a beach sloping by
    /// `slope` for the waves reaching it: the slope over the square root of
    /// their deep-water steepness. Under ~0.5 they spill (a gentle beach:
    /// foam rolls down their front over a wide surf zone, then the swash
    /// slides up the sand); up to ~3 they plunge; past that they crash on
    /// what stands steep (rocks, a wall). Same as `water_sea_surf`.
    pub fn surf(&self, slope: f32) -> f32 {
        surf_similarity(slope, 2.0 * self.trains[0][3], TAU / self.trains[0][2].max(1e-4))
    }

    /// Height of the displayed macro surface over a still `level`, at
    /// rendered world `xy`, where the sea is read as `at`
    /// (`WaterGround::sea_point`).
    pub fn surface(&self, xy: Vec2, time: f32, trains: usize, level: f32, at: &SeaPoint) -> f32 {
        let swell = self.swell_at(xy, at);
        let local = self.local(at, swell.shadow, swell.chop);
        let mut source = xy;
        for _ in 0..3 {
            source = xy - self.open(source, xy, time, trains, &swell, &local, at.ocean.as_deref()).truncate();
        }
        level + self.open(source, xy, time, trains, &swell, &local, at.ocean.as_deref()).z
    }

    /// How fast the water at rendered world `xy` moves sideways at `time`
    /// (m/s), where the sea is read as `at`: the trains' orbital motion as
    /// `surface` places it, forward under a crest and back in a trough. What
    /// a body standing in the water feels pushing past it
    /// (scene/water_contacts.rs). (The spectrum's maps are this frame's
    /// alone: its chop gives none.)
    pub fn flow(&self, xy: Vec2, time: f32, trains: usize, at: &SeaPoint) -> Vec2 {
        let swell = self.swell_at(xy, at);
        let local = self.local(at, swell.shadow, swell.chop);
        let ocean = at.ocean.as_deref();
        let mut source = xy;
        for _ in 0..3 {
            source = xy - self.open(source, xy, time, trains, &swell, &local, ocean).truncate();
        }
        const H: f32 = 0.05;
        let ahead = self.open(source, xy, time + H, trains, &swell, &local, ocean);
        let behind = self.open(source, xy, time - H, trains, &swell, &local, ocean);
        let flow = (ahead - behind).truncate() / (2.0 * H);
        if flow.is_finite() { flow } else { Vec2::ZERO }
    }
}

/// How much of the wind sea's sway (its horizontal displacement) is kept
/// where its derivatives, scaled as the surface takes them, are `j` (xx,
/// xy, yy): all of it, but where the surface would fold over itself (the
/// Jacobian's smaller eigenvalue under 0.2; Tessendorf, "Simulating Ocean
/// Water", 2004), just enough to keep it from folding (a chop and crests
/// turned far up looped the surface behind every crest). Same as
/// water_spectrum.wgsl's `water_chop_sway`.
pub fn chop_sway(j: glam::Vec3) -> f32 {
    let (a, c) = (1.0 + j.x, 1.0 + j.z);
    let low = 0.5 * (a + c) - (0.25 * (a - c) * (a - c) + j.y * j.y).sqrt();
    (0.8 / (1.0 - low).max(1e-3)).min(1.0)
}

/// The surf similarity (Iribarren number, `Sea::surf`) of a beach sloping by
/// `slope` for waves `height` m high and `length` m long in deep water.
/// Same as `water_surf_similarity`.
pub fn surf_similarity(slope: f32, height: f32, length: f32) -> f32 {
    slope / (height.max(0.02) / length.max(0.1)).sqrt()
}

/// The main wave where the sea is read (`Sea::main`): its amplitude (m,
/// before it shoals), its deep-water height and length (m, for the surf
/// similarity), how much of the chop it takes over (0..1), and its clock:
/// its phase at the sea's epoch and angular frequency (rad, rad/s).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Main {
    amp: f32,
    height: f32,
    length: f32,
    take: f32,
    phase: f32,
    omega: f32,
}

/// The swash's rise (0..1) at the swell's phase `theta`: the broken wave
/// runs up the beach in a third of its period and slides back over the
/// rest (same as `water_swash_rise`).
pub fn swash_rise(theta: f32) -> f32 {
    let s = (-theta / TAU).rem_euclid(1.0);
    if s < 0.3 {
        smoothstep(0.0, 0.3, s)
    } else {
        1.0 - smoothstep(0.3, 1.0, s)
    }
}

/// How the sea meets the bed at a point (`Sea::local`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Local {
    /// The main wave's clock (`Sea::main`): the open sea's swell's, or the
    /// wind sea's own wave's where the swell does not reach. Its phase at
    /// the sea's epoch (rad) and angular frequency (rad/s).
    pub phase: f32,
    pub omega: f32,
    /// The main swell's amplitude (m).
    pub main: f32,
    /// The wind chop's share of its amplitude (the trains', the fetch's
    /// share in).
    pub chop: f32,
    /// The spectrum's cascades' shares here (`ocean_weights`: the open
    /// sea's twice, the lakes'), and what the depth, the shelter and the
    /// clearance leave of all of them.
    pub weights: [f32; 3],
    pub spectrum: f32,
    /// How hard the main swell breaks (0..1).
    pub breaking: f32,
    /// How far its front leans (the profile's skew).
    pub lean: f32,
    /// The swash's share of the surface (0..1) and how high it runs (m).
    pub swash: f32,
    pub reach: f32,
    /// The beach's surf similarity.
    pub surf: f32,
    /// What the swash's phase adds to the swell's here (rad): the swell's
    /// phase where it meets the shoreline.
    pub shore_shift: f32,
    /// How much of the swell's sway across the water is left (0..1): none
    /// in the last metre or so of real depth, so the surface does not slide
    /// over the land beside it.
    pub slide: f32,
}

/// The main swell at a point of the sea grid (`WaterGround::sea_point`):
/// its spatial phase (rad), local wave vector (rad/m, the way it travels),
/// how much of the open sea's swell reaches it (`shadow`: 1 open, less
/// downwave of land, none in a lake) and how much wind chop it builds
/// (`chop`: less with a short fetch). `lake`: how much of the main wave is
/// the wind sea's own (`lake_share`); past a half, `phase` and `wave` are
/// that wave's (`Sea::lake_wave`), not the swell's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwellPoint {
    pub phase: f32,
    pub wave: Vec2,
    pub shadow: f32,
    pub chop: f32,
    pub lake: f32,
}

/// Where the sea is read (`WaterGround::sea_point`): the wave depth, its
/// gradient (the bed's slope as the waves feel it), how calm the shelter
/// keeps it and how much the clearance above it leaves (`clearance_cap`),
/// and the refracted main swell (none: the open sea's), and the spectrum's
/// maps of the wind sea (none: without them, the trains' chop or none).
#[derive(Debug, Clone, PartialEq)]
pub struct SeaPoint {
    pub depth: f32,
    pub slope: Vec2,
    pub shelter: f32,
    pub cap: f32,
    pub swell: Option<SwellPoint>,
    pub ocean: Option<Arc<OceanMaps>>,
    /// The depth of the real bed under the still water (the bathymetry's,
    /// not smoothed; below 0 on land), where the swash runs (`WaterSeaAt`).
    pub bed: f32,
}

impl SeaPoint {
    /// Open, flat-bottomed water `depth` deep.
    pub fn open(depth: f32) -> Self {
        Self {
            depth,
            slope: Vec2::ZERO,
            shelter: 1.0,
            cap: 1.0,
            swell: None,
            ocean: None,
            bed: depth,
        }
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How long the wind chop and the swell take to turn with the wind (s).
const CHOP_TURN: f32 = 20.0;
const SWELL_TURN: f32 = 240.0;
/// How long the swell takes to follow the wind's speed (s).
const SWELL_EASE: f32 = 90.0;

/// How the Sea panel shapes the sea the wind raises (the viewer's
/// `SeaSettings`): each 1 by default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeaTune {
    /// The long swell's height, and the wind chop's.
    pub swell: f32,
    pub chop: f32,
    /// How far the wind blows over open water (m): a short fetch keeps the
    /// chop small and short, a long one lets it grow.
    pub fetch: f32,
    /// How sharp the crests are.
    pub choppiness: f32,
    /// How much foam the whitecaps make.
    pub foam: f32,
    /// How much water the waves throw into the air (1: as a real sea
    /// does; the viewer's scene/water_spindrift.rs: more of it, and from
    /// calmer seas, the higher).
    pub spray: f32,
    /// The wind chop as a spectrum, or (none) as the stylised crossing
    /// trains.
    pub spectrum: Option<Spectrum>,
    /// The most the open sea stands from trough to crest (m; infinite: no
    /// limit). The swell and the chop are scaled down together to keep
    /// under it (the shallows still raise the swell a little).
    pub max_height: f32,
}

impl Default for SeaTune {
    fn default() -> Self {
        Self {
            swell: 1.0,
            chop: 1.0,
            fetch: crate::water_fft::FETCH,
            choppiness: 1.0,
            foam: 1.0,
            spray: 1.0,
            spectrum: Some(Spectrum::Jonswap),
            max_height: f32::INFINITY,
        }
    }
}

/// The spectrum's state: at rest between frames, or working out the next
/// maps on a worker thread (it hands itself back with them).
#[derive(Debug)]
enum OceanJob {
    Idle(Box<Ocean>),
    Busy(std::thread::JoinHandle<(Box<Ocean>, Arc<OceanMaps>)>),
}

/// Follows the wind over time and keeps the trains' phases (CPU only).
#[derive(Debug)]
pub struct SeaDriver {
    sea: Sea,
    /// The way the wind chop goes (the wind's, eased over ~20 s) and the
    /// way the swell goes (over minutes).
    heading: Vec2,
    swell: Vec2,
    speed: f32,
    /// The wind the swell was raised by: the wind's speed over about a
    /// minute and a half (a gusty region's wind once changed its length and
    /// height every few seconds, and baked its refraction again each time).
    swell_speed: f32,
    last: Option<f32>,
    /// The wind sea as a spectrum, when on (`set_spectrum`), and its maps.
    ocean: Option<(OceanJob, Spectrum)>,
    maps: Arc<OceanMaps>,
    tune: SeaTune,
}

impl Default for SeaDriver {
    fn default() -> Self {
        let sea = Sea::default();
        Self {
            heading: Vec2::new(sea.wind[0], sea.wind[1]),
            swell: Vec2::new(sea.wind[0], sea.wind[1]),
            speed: sea.wind[2],
            swell_speed: sea.wind[2],
            sea,
            last: None,
            ocean: None,
            maps: Arc::default(),
            tune: SeaTune {
                spectrum: None,
                ..SeaTune::default()
            },
        }
    }
}

impl SeaDriver {
    /// The sea now.
    pub fn sea(&self) -> Sea {
        self.sea
    }

    /// Shape the sea as the Sea panel says (`SeaTune`).
    pub fn set_tune(&mut self, tune: SeaTune) {
        self.set_spectrum(tune.spectrum);
        self.tune = tune;
    }

    /// The wind sea as a `spectrum` (water_fft.rs), or as the trains'
    /// crossing chop (none).
    pub fn set_spectrum(&mut self, spectrum: Option<Spectrum>) {
        self.tune.spectrum = spectrum;
        match (spectrum, &mut self.ocean) {
            (Some(kind), Some((_, now))) => *now = kind,
            (Some(kind), None) => self.ocean = Some((OceanJob::Idle(Box::default()), kind)),
            (None, _) => {
                self.ocean = None;
                self.maps = Arc::default();
            }
        }
    }

    /// The spectrum's maps now (empty without one).
    pub fn maps(&self) -> Arc<OceanMaps> {
        self.maps.clone()
    }

    /// Move on to `time` (the frame clock), under `wind` (m/s, blowing
    /// toward), around the camera at `anchor`. A new clock (back in time,
    /// or the first call) takes the wind as it is, without easing.
    pub fn advance(&mut self, time: f32, wind: Vec2, anchor: Vec2) -> Sea {
        let wind = if wind.is_finite() { wind } else { Vec2::ZERO };
        let dt = match self.last {
            Some(last) if time >= last => (time - last).min(1.0),
            last => {
                if let Some(last) = last {
                    log::info!("water sea: the clock went back ({last:.1} s to {time:.1} s), the sea starts again from the wind");
                }
                self.speed = wind.length();
                self.swell_speed = self.speed;
                if let Some(h) = wind.try_normalize() {
                    self.heading = h;
                    self.swell = h;
                }
                self.sea.frame[2] = time;
                0.0
            }
        };
        self.last = Some(time);
        // the sea builds in seconds and calms down slower and averages the
        // gusts out. Its wind chop turns with the wind in some 20 s; its
        // swell remembers the wind of the last minutes and turns over about
        // four: a wind that keeps drifting round (the region's often does)
        // leaves a steady swell, only a lasting change turns it
        let target = wind.length();
        let tau = if target > self.speed { 6.0 } else { 15.0 };
        self.speed += (target - self.speed) * (1.0 - (-dt / tau).exp());
        self.swell_speed += (self.speed - self.swell_speed) * (1.0 - (-dt / SWELL_EASE).exp());
        if let Some(to) = wind.try_normalize() {
            let turn = |from: Vec2, tau: f32| {
                let k = 1.0 - (-dt / tau).exp();
                Vec2::from_angle(from.angle_to(to) * k).rotate(from).normalize_or(to)
            };
            self.heading = turn(self.heading, CHOP_TURN);
            self.swell = turn(self.swell, SWELL_TURN);
        }
        let sea = &mut self.sea;
        // the phases reach `time` at the old speeds
        for i in 0..SEA_TRAINS {
            sea.phase[i] = (sea.phase[i] + sea.omega[i] * (time - sea.frame[2])).rem_euclid(TAU);
        }
        // (and the lakes' wave, `Sea::lake_wave`)
        let (lake_dir, lake_k, lake_omega) = sea.lake_wave();
        sea.lake[3] = (sea.lake[3] + lake_omega * (time - sea.frame[2])).rem_euclid(TAU);
        sea.frame[2] = time;
        // re-anchored on the camera, the phases taking the move
        let anchor = if anchor.is_finite() { anchor } else { Vec2::ZERO };
        let moved = anchor - Vec2::new(sea.frame[0], sea.frame[1]);
        for i in 0..SEA_TRAINS {
            let [dx, dy, k, _] = sea.trains[i];
            sea.phase[i] = (sea.phase[i] - k * moved.dot(Vec2::new(dx, dy))).rem_euclid(TAU);
        }
        sea.lake[3] = (sea.lake[3] - lake_k * moved.dot(lake_dir)).rem_euclid(TAU);
        sea.frame[0] = anchor.x;
        sea.frame[1] = anchor.y;
        sea.shape_crossed(self.swell, self.heading, self.speed, self.swell_speed);
        // the Sea panel's shaping: the swell's and the chop's heights, held
        // together under the height limit (the open sea's trough to crest:
        // twice the swell's amplitude and the chop's significant height)
        let tune = self.tune;
        let finite = |v: f32, or: f32| if v.is_finite() { v.max(0.0) } else { or };
        let (swell_scale, chop_scale) = (finite(tune.swell, 1.0), finite(tune.chop, 1.0));
        // the spectrum's crests sharpen with the wind, as the trains'
        let chop = (0.6 + 0.4 * sea.frame[3]) * finite(tune.choppiness, 1.0);
        let fetch = finite(tune.fetch, crate::water_fft::FETCH);
        let (heading, speed) = (self.heading, self.speed);
        // its first maps at once (later ones come from the worker): the sea
        // never starts out without its chop, and the limit knows its height
        if self.maps.texels.is_empty()
            && let Some((OceanJob::Idle(ocean), kind)) = &mut self.ocean
        {
            self.maps = ocean.update(f64::from(time), heading, speed, *kind, chop, fetch, 1.0);
        }
        let chop_height = if self.ocean.is_some() {
            if self.maps.texels.is_empty() {
                0.0
            } else {
                self.maps.height / self.maps.scale.max(1e-6)
            }
        } else {
            2.0 * sea.trains[1..].iter().map(|t| t[3]).sum::<f32>()
        };
        let open = 2.0 * sea.trains[0][3] * swell_scale + chop_height * chop_scale;
        let limit = if tune.max_height.is_finite() && open > tune.max_height.max(0.05) {
            tune.max_height.max(0.05) / open
        } else {
            1.0
        };
        sea.trains[0][3] *= swell_scale * limit;
        for train in &mut sea.trains[1..] {
            train[3] *= chop_scale * limit;
        }
        sea.wind[3] = sea.trains.iter().map(|t| t[3]).sum::<f32>() + sea.trains[0][3];
        sea.look[0] = finite(tune.foam, 1.0);
        sea.look[1] = chop_scale * limit;
        // the caustics' ripples drift downwind, faster as it blows harder
        let drift = heading * caustic_drift(sea.frame[3]) * dt;
        sea.look[2] += drift.x;
        sea.look[3] += drift.y;
        if let Some((job, kind)) = self.ocean.take() {
            // Its FFTs take a millisecond or so: on a worker, during the
            // frame before, for this frame's time (the last step taken as
            // the next one's), and always waited for (it is nearly always
            // done by then). (Taking whatever the worker had finished made
            // the waves stall and jump back and forth by frames whenever
            // it fell behind.)
            let job = match job {
                OceanJob::Busy(work) => match work.join() {
                    Ok((ocean, maps)) => {
                        self.maps = maps;
                        OceanJob::Idle(ocean)
                    }
                    Err(_) => {
                        log::warn!("water sea: the spectrum's worker failed");
                        OceanJob::Idle(Box::default())
                    }
                },
                idle => idle,
            };
            let next = f64::from(time) + f64::from(dt.clamp(0.0, 0.1));
            let job = match job {
                OceanJob::Idle(mut ocean) => OceanJob::Busy(std::thread::spawn(move || {
                    let maps = ocean.update(next, heading, speed, kind, chop, fetch, 1.0);
                    (ocean, maps)
                })),
                busy => busy,
            };
            self.ocean = Some((job, kind));
            // (the maps' own height, scaled as the shaders scale them)
            let height = self.maps.height * sea.look[1];
            sea.shore[1] = OCEAN_TILES[0];
            sea.shore[2] = OCEAN_TILES[1];
            // the lakes' chop (water_fft.rs `LAKE`)
            sea.lake = [
                OCEAN_TILES[crate::water_fft::LAKE],
                self.maps.lake_height * sea.look[1],
                sea.lake[2],
                sea.lake[3],
            ];
            sea.shore[3] = height.max(1e-4);
            // the highest crest: the swell growing up to twice in the
            // shallows, on the wind sea's highest (about its Hs)
            sea.wind[3] = 2.0 * sea.trains[0][3] + height;
        }
        self.sea
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Open water keeps the open sea; a lake 800 m across (the exposure's
    /// 0.6) takes the lakes' chop in full, one of 200 m (0.3) half as
    /// high, as a fetch-limited sea grows (as the square root of its
    /// fetch); a bay between them takes some of both.
    #[test]
    fn a_short_fetch_gives_the_lakes_chop_as_high_as_it_lets_it_grow() {
        let [open, lake] = ocean_weights(1.0);
        assert!((open - 1.0).abs() < 1e-6 && lake.abs() < 1e-6, "{open} {lake}");
        let [open, lake] = ocean_weights(0.6);
        assert!(open < 0.05 && lake > 0.95, "{open} {lake}");
        let [open, lake] = ocean_weights(0.3);
        assert!(open == 0.0 && (lake - 0.5).abs() < 1e-6, "{open} {lake}");
        let [open, lake] = ocean_weights(0.8);
        assert!(open > 0.3 && open < 0.9 && lake > 0.1, "{open} {lake}");
        // the depth caps a lake's chop by its own height, not the ocean's
        let mut driver = SeaDriver::default();
        let sea = driver.advance(1.0, Vec2::new(20.0, 0.0), Vec2::ZERO);
        let mut lake_sea = sea;
        lake_sea.shore[3] = 4.0;
        lake_sea.lake[1] = 0.3;
        assert!((lake_sea.chop_height([0.0, 1.0]) - 0.15).abs() < 1e-6);
        assert!((lake_sea.chop_height([1.0, 0.0]) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn the_water_flows_forward_under_a_crest_and_back_in_a_trough() {
        let mut driver = SeaDriver::default();
        for i in 0..400 {
            driver.advance(i as f32 * 0.5, Vec2::new(14.0, 0.0), Vec2::ZERO);
        }
        let sea = driver.sea();
        let at = SeaPoint::open(60.0);
        let toward = Vec2::new(sea.trains[0][0], sea.trains[0][1]);
        let period = TAU / sea.omega[0].max(1e-3);
        let (mut top, mut low) = ((f32::MIN, 0.0), (f32::MAX, 0.0));
        let mut fastest = 0.0f32;
        for i in 0..200 {
            let t = 200.0 + period * i as f32 / 200.0;
            let z = sea.surface(Vec2::ZERO, t, 1, 0.0, &at);
            let along = sea.flow(Vec2::ZERO, t, 1, &at).dot(toward);
            fastest = fastest.max(along.abs());
            if z > top.0 {
                top = (z, along);
            }
            if z < low.0 {
                low = (z, along);
            }
        }
        // as fast as the swell's orbits: some aω
        let orbit = sea.trains[0][3] * sea.omega[0];
        assert!(fastest > 0.3 * orbit && fastest < 1.5 * orbit, "{fastest} {orbit}");
        assert!(top.1 > 0.5 * fastest && low.1 < -0.5 * fastest, "{top:?} {low:?}");
    }

    #[test]
    fn a_chop_turned_far_up_does_not_fold_the_surface_over() {
        let low = |j: glam::Vec3| {
            let (a, c) = (1.0 + j.x, 1.0 + j.z);
            0.5 * (a + c) - (0.25 * (a - c) * (a - c) + j.y * j.y).sqrt()
        };
        // a sea that does not fold keeps all its sway
        assert_eq!(chop_sway(glam::Vec3::new(-0.3, 0.1, -0.2)), 1.0);
        // one folding over keeps just enough not to: its fold held at 0.2
        for j in [glam::Vec3::new(-1.5, 0.0, 0.0), glam::Vec3::new(-0.9, 0.6, -1.1)] {
            let k = chop_sway(j);
            assert!(k < 1.0 && low(j) < 0.0);
            assert!((low(j * k) - 0.2).abs() < 0.03, "{}", low(j * k));
        }
    }

    #[test]
    fn caustics_focus_and_drift_with_the_sea() {
        // glassy water focuses softly, a breeze sharper, a gale sharper still
        let (glassy, breeze, gale) = (caustic_focus(0.0, 1.0), caustic_focus(0.3, 1.0), caustic_focus(1.0, 1.0));
        assert!(glassy < breeze && breeze < gale, "{glassy} {breeze} {gale}");
        assert!(caustic_focus(0.3, 0.0) < breeze, "no chop, softer");
        assert!(caustic_focus(f32::NAN, f32::NAN).is_finite());
        assert!(caustic_drift(0.0) > 0.0 && caustic_drift(1.0) > caustic_drift(0.2));
        // the ripples drift downwind as the sea advances
        let mut driver = SeaDriver::default();
        driver.advance(0.0, Vec2::new(8.0, 0.0), Vec2::ZERO);
        let mut sea = driver.advance(0.5, Vec2::new(8.0, 0.0), Vec2::ZERO);
        for i in 1..20 {
            sea = driver.advance(0.5 + i as f32 * 0.5, Vec2::new(8.0, 0.0), Vec2::ZERO);
        }
        assert!(sea.look[2] > 0.5 && sea.look[3].abs() < 0.05, "{:?}", sea.look);
    }

    #[test]
    fn the_spectrum_takes_over_the_chop_and_the_depth_still_caps_it() {
        let mut driver = SeaDriver::default();
        driver.set_spectrum(Some(Spectrum::Jonswap));
        driver.advance(0.0, Vec2::new(16.0, 0.0), Vec2::ZERO);
        let sea = driver.advance(2.0, Vec2::new(16.0, 0.0), Vec2::ZERO);
        let maps = driver.maps();
        assert!(sea.shore[3] > 0.5 && sea.shore[1] == OCEAN_TILES[0], "{:?}", sea.shore);
        let open = SeaPoint {
            ocean: Some(maps.clone()),
            ..SeaPoint::open(OPEN_DEPTH)
        };
        // (the main swell alone: the same point without the maps)
        let bare = SeaPoint::open(OPEN_DEPTH);
        let differs = (0..50)
            .map(|i| Vec2::new(i as f32 * 3.1, i as f32 * 1.7))
            .filter(|&p| (sea.surface(p, 2.0, 4, 0.0, &open) - sea.surface(p, 2.0, 4, 0.0, &bare)).abs() > 0.05)
            .count();
        assert!(differs > 10, "the wind sea shows: {differs}");
        // a few centimetres of water keep it down to their depth
        let shallow = SeaPoint {
            ocean: Some(maps),
            ..SeaPoint::open(0.3)
        };
        for i in 0..50 {
            let h = sea.surface(Vec2::new(i as f32 * 2.3, 0.0), 2.0, 4, 0.0, &shallow);
            assert!(h.abs() < 0.3 * 0.28 * 2.5, "{h}");
        }
        // off again: the trains' chop
        driver.set_spectrum(None);
        let sea = driver.advance(2.1, Vec2::new(16.0, 0.0), Vec2::ZERO);
        assert_eq!(sea.shore[3], 0.0);
        assert!(driver.maps().texels.is_empty());
    }

    #[test]
    fn the_tune_shapes_the_sea_and_the_limit_holds_it_down() {
        let gale = Vec2::new(20.0, 0.0);
        let open = |tune: SeaTune| {
            let mut driver = SeaDriver::default();
            driver.set_tune(tune);
            driver.advance(0.0, gale, Vec2::ZERO);
            let sea = driver.advance(0.1, gale, Vec2::ZERO);
            (2.0 * sea.trains[0][3] + sea.shore[3], sea)
        };
        let (free, sea) = open(SeaTune::default());
        assert!(free > 3.0 && sea.look[0] == 1.0, "{free}");
        let (held, _) = open(SeaTune {
            max_height: 1.5,
            ..SeaTune::default()
        });
        assert!(held <= 1.5 * 1.05 && held > 1.2, "{held}");
        let (no_swell, sea) = open(SeaTune {
            swell: 0.0,
            foam: 0.3,
            ..SeaTune::default()
        });
        assert!(sea.trains[0][3] == 0.0 && no_swell > 0.5 && sea.look[0] == 0.3);
        // the stylised trains: no spectrum, the limit still holds
        let (trains, sea) = open(SeaTune {
            spectrum: None,
            max_height: 1.0,
            ..SeaTune::default()
        });
        assert_eq!(sea.shore[3], 0.0);
        let stacked = 2.0 * sea.trains.iter().map(|t| t[3]).sum::<f32>();
        assert!(stacked <= 1.0 + 1e-3 && trains > 0.0, "{stacked}");
    }

    #[test]
    fn calm_is_glassy_and_a_gale_is_rough() {
        let calm = {
            let mut s = Sea::default();
            s.shape(Vec2::X, 0.0);
            s
        };
        let gale = {
            let mut s = Sea::default();
            s.shape(Vec2::X, 20.0);
            s
        };
        assert!(calm.trains[0][3] < 0.01, "calm: {:?}", calm.trains[0]);
        assert!(gale.trains[0][3] > 0.9, "gale: {:?}", gale.trains[0]);
        assert!(gale.trains[0][2] < calm.trains[0][2], "a gale's waves are longer");
        assert_eq!(gale.frame[3], 1.0);
        // the main train runs with the wind
        assert!((gale.trains[0][0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_default_sea_keeps_the_former_breeze() {
        let sea = Sea::default();
        let [_, _, k, a] = sea.trains[0];
        assert!((k - 0.19).abs() < 0.04, "{k}");
        assert!((a - 0.24).abs() < 0.08, "{a}");
    }

    #[test]
    fn re_anchoring_and_integrating_leave_the_surface_in_place() {
        let mut driver = SeaDriver::default();
        driver.advance(10.0, Vec2::new(6.0, 2.0), Vec2::ZERO);
        let before = driver.sea();
        let p = Vec2::new(31.0, -12.0);
        let t = 10.4;
        let open = SeaPoint::open(OPEN_DEPTH);
        let h0 = before.surface(p, t, 4, 0.0, &open);
        assert!((before.clearance_cap(6.0) - 1.0).abs() < 1e-6, "open water");
        assert_eq!(before.clearance_cap(0.0), 0.0, "a deck at the water");
        // same wind (eased already), a moved camera and a later epoch
        driver.speed = before.wind[2];
        let after = driver.advance(
            10.2,
            Vec2::new(before.wind[0], before.wind[1]) * before.wind[2],
            Vec2::new(40.0, 25.0),
        );
        let h1 = after.surface(p, t, 4, 0.0, &open);
        assert!((h0 - h1).abs() < 2e-3, "{h0} vs {h1}");
    }

    #[test]
    fn a_drifting_wind_turns_the_chop_and_leaves_the_swell() {
        // the region's wind going round at 1° a second for 100 s
        let mut driver = SeaDriver::default();
        let at = |t: f32| Vec2::from_angle((t * 1.0).to_radians()) * 14.0;
        driver.advance(0.0, at(0.0), Vec2::ZERO);
        for i in 1..=600 {
            let t = i as f32 / 6.0;
            driver.advance(t, at(t), Vec2::ZERO);
        }
        let sea = driver.sea();
        let angle = |d: [f32; 4]| Vec2::new(d[0], d[1]).to_angle().to_degrees();
        let swell = angle(sea.trains[0]);
        let wind = Vec2::new(sea.wind[0], sea.wind[1]).to_angle().to_degrees();
        assert!(swell < 30.0, "the swell stays: {swell}°");
        assert!(wind > 70.0, "the chop follows: {wind}°");
    }

    #[test]
    fn the_wind_eases_in_and_a_new_clock_takes_it_at_once() {
        let mut driver = SeaDriver::default();
        driver.advance(0.0, Vec2::ZERO, Vec2::ZERO);
        assert_eq!(driver.sea().wind[2], 0.0);
        driver.advance(1.0, Vec2::new(15.0, 0.0), Vec2::ZERO);
        let eased = driver.sea().wind[2];
        assert!(eased > 0.5 && eased < 5.0, "{eased}");
        // back in time: a new clock
        driver.advance(0.5, Vec2::new(15.0, 0.0), Vec2::ZERO);
        assert_eq!(driver.sea().wind[2], 15.0);
    }

    #[test]
    fn a_lake_s_own_waves_line_up_in_the_shallows_and_lap_its_downwind_shores() {
        let mut sea = Sea::default();
        sea.shape(Vec2::X, 14.0);
        let (dir, k, _) = sea.lake_wave();
        assert_eq!(dir, Vec2::X, "it runs downwind");
        let length = TAU / k;
        assert!((2.5..3.5).contains(&length), "a lake's chop is some 3 m long in a gale: {length}");
        // a lake: no swell, a short fetch; a beach rising downwind
        let lake = |depth: f32, slope: Vec2| SeaPoint {
            depth,
            slope,
            swell: Some(SwellPoint {
                phase: 0.0,
                wave: dir * k,
                shadow: 0.0,
                chop: 0.4,
                lake: lake_share(0.0),
            }),
            ..SeaPoint::open(depth)
        };
        let up = -Vec2::X * 0.06;
        let deep = sea.local(&lake(10.0, Vec2::ZERO), 0.0, 0.4);
        assert_eq!(deep.main, 0.0, "the chop's spectrum draws it in deep water");
        assert!(deep.chop > 0.39, "{deep:?}");
        // as the water shoals under half its length it lines up and breaks,
        // the chop giving way to it
        let shoal = sea.local(&lake(0.4, up), 0.0, 0.4);
        assert!(shoal.main > 0.02 && shoal.breaking > 0.0, "{shoal:?}");
        // (against the same water with the swell's main wave: the depth
        // caps both alike, the lake's wave takes half of what is left)
        let swell_there = sea.local(
            &SeaPoint {
                depth: 0.4,
                slope: up,
                ..SeaPoint::open(0.4)
            },
            1.0,
            0.4,
        );
        assert!(
            (shoal.chop / swell_there.chop - LAKE_TAKE).abs() < 1e-3,
            "it keeps its shorter waves: {shoal:?} against {swell_there:?}"
        );
        // and runs up the downwind beach as swash, timed by its own clock
        let beach = sea.local(&lake(-0.05, up), 0.0, 0.4);
        assert!(beach.swash > 0.5 && beach.reach > 0.03, "{beach:?}");
        assert_eq!((beach.phase, beach.omega), (sea.lake[3], (G * k).sqrt()));
        // none up the upwind shore, nor on a calm lake
        assert_eq!(sea.local(&lake(-0.05, -up), 0.0, 0.4).swash, 0.0, "the upwind shore");
        let mut calm = Sea::default();
        calm.shape(Vec2::X, 0.0);
        assert!(calm.local(&lake(0.4, up), 0.0, 0.4).main < 0.01);
        // the open sea's shores keep the swell, the swell's clock
        let open = SeaPoint {
            depth: 0.4,
            slope: up,
            ..SeaPoint::open(0.4)
        };
        let sea_shore = sea.local(&open, 1.0, 1.0);
        assert_eq!((sea_shore.phase, sea_shore.omega), (sea.phase[0], sea.omega[0]));
        // never both: the swell fades out, then the lake's wave in
        assert_eq!(lake_share(1.0), 0.0);
        assert_eq!(lake_share(0.0), 1.0);
        let mid = SeaPoint {
            swell: Some(SwellPoint {
                shadow: 0.1,
                lake: 0.5,
                ..lake(0.4, up).swell.expect("a lake's swell point")
            }),
            ..lake(0.4, up)
        };
        assert_eq!(sea.local(&mid, 0.1, 0.4).main, 0.0, "half way");
    }

    #[test]
    fn the_lake_s_wave_keeps_its_place_as_the_sea_re_anchors() {
        let mut driver = SeaDriver::default();
        driver.advance(10.0, Vec2::new(12.0, 3.0), Vec2::ZERO);
        driver.speed = driver.sea().wind[2];
        let before = driver.sea();
        let p = Vec2::new(31.0, -12.0);
        let (dir, k, _) = before.lake_wave();
        let at = |sea: &Sea| SeaPoint {
            swell: Some(SwellPoint {
                phase: k * (p - sea.anchor()).dot(dir),
                wave: dir * k,
                shadow: 0.0,
                chop: 0.4,
                lake: 1.0,
            }),
            slope: -dir * 0.06,
            ..SeaPoint::open(0.4)
        };
        let h0 = before.surface(p, 10.4, 4, 0.0, &at(&before));
        let after = driver.advance(
            10.2,
            Vec2::new(before.wind[0], before.wind[1]) * before.wind[2],
            Vec2::new(40.0, 25.0),
        );
        let h1 = after.surface(p, 10.4, 4, 0.0, &at(&after));
        assert!(h0.abs() > 1e-3, "a wave there: {h0}");
        assert!((h0 - h1).abs() < 5e-3, "{h0} vs {h1}");
    }

    #[test]
    fn the_swell_grows_breaks_runs_up_and_dims_behind_land() {
        let mut sea = Sea::default();
        sea.shape(Vec2::X, 12.0);
        // (a beach rising the way the swell goes: deeper behind it)
        let at = |depth: f32, slope: f32| SeaPoint {
            depth,
            slope: -Vec2::X * slope,
            ..SeaPoint::open(depth)
        };
        let deep = sea.local(&at(OPEN_DEPTH, 0.0), 1.0, 1.0);
        assert_eq!(deep.main, sea.trains[0][3]);
        assert_eq!((deep.breaking, deep.swash), (0.0, 0.0));
        let shallow = sea.local(&at(3.0, 0.05), 1.0, 1.0);
        assert!(shallow.main <= 0.28 * 3.0 + 1e-5 && shallow.breaking > 0.0, "{shallow:?}");
        assert!(sea.local(&at(0.6, 0.1), 1.0, 1.0).breaking > 0.5, "breaking near the shore");
        assert!(sea.local(&at(-0.5, 0.1), 1.0, 1.0).swash > 0.5, "the swash runs up the beach");
        assert_eq!(sea.local(&at(-SWASH_REACH - 0.3, 0.1), 1.0, 1.0).swash, 0.0, "dry, past the swash");
        assert_eq!(sea.local(&at(0.01, 0.1), 1.0, 1.0).main, 0.0);
        assert_eq!(sea.local(&at(1.0, 0.0), 1.0, 1.0).swash, 0.0, "flat shallows: no swash");
        // the back of a land bridge, turned away from the swell: none
        let back = SeaPoint {
            slope: Vec2::X * 0.1,
            ..at(-0.3, 0.1)
        };
        assert_eq!(sea.local(&back, 1.0, 1.0).swash, 0.0, "no swash up the lee side");
        // nor up a cliff: it takes the waves as a crash (water_crash.rs)
        assert_eq!(sea.local(&at(-0.3, 1.2), 1.0, 1.0).swash, 0.0, "no swash up a cliff");
        // downwave of an island the swell dims, the wind chop less
        let lee = sea.local(&at(OPEN_DEPTH, 0.0), 0.25, 0.7);
        assert!(lee.main < deep.main * 0.3 && lee.chop > 0.6 * deep.chop, "{lee:?}");
        // a gentle beach spills, a steep one plunges, a wall crashes
        assert!(
            sea.surf(0.01) < 0.5 && sea.surf(0.15) > 0.5 && sea.surf(1.0) > 3.0,
            "{} {} {}",
            sea.surf(0.01),
            sea.surf(0.15),
            sea.surf(1.0)
        );
        // the open sea's plane wave and the same wave given as refracted
        let xy = Vec2::new(31.0, -12.0);
        let [dx, dy, k, _] = sea.trains[0];
        let plane = SwellPoint {
            phase: (xy - sea.anchor()).dot(Vec2::new(dx, dy)) * k,
            wave: Vec2::new(dx, dy) * k,
            shadow: 1.0,
            chop: 1.0,
            lake: 0.0,
        };
        let a = sea.surface(xy, 3.0, 4, 20.0, &SeaPoint::open(OPEN_DEPTH));
        let b = sea.surface(
            xy,
            3.0,
            4,
            20.0,
            &SeaPoint {
                swell: Some(plane),
                ..SeaPoint::open(OPEN_DEPTH)
            },
        );
        assert!((a - b).abs() < 1e-4, "{a} vs {b}");
    }
}
