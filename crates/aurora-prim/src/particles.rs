//! Second Life particle systems: wire-format parsing (`PSBlock`) and CPU
//! simulation of the particles emitted by one scripted source.
//!
//! Portions ported from the Second Life Viewer source code,
//! Copyright (C) Linden Research, Inc., originally LGPL 2.1:
//! - `llmessage/llpartdata.{h,cpp}` (`LLPartData`, `LLPartSysData` unpacking),
//! - `newview/llviewerpartsource.cpp` (`LLViewerPartSourceScript::update`,
//!   `LLViewerPartSourceScript::unpackPSS`),
//! - `newview/llviewerpartsim.cpp` (`LLViewerPartGroup::updateParticles`).
//!
//! Rendering is out of scope: [`ParticleSource::particles`] exposes everything
//! a renderer needs (position, size, color, glow, blend funcs, ribbon links).

use glam::{Quat, Vec2, Vec3};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Constants (LLPartData / LLPartSysData enums)
// ---------------------------------------------------------------------------

/// Interpolate color/alpha from start to end over the particle lifetime.
pub const LL_PART_INTERP_COLOR_MASK: u32 = 0x01;
/// Interpolate scale from start to end over the particle lifetime.
pub const LL_PART_INTERP_SCALE_MASK: u32 = 0x02;
/// Bounce off the plane at the source's Z.
pub const LL_PART_BOUNCE_MASK: u32 = 0x04;
/// Affected by wind.
pub const LL_PART_WIND_MASK: u32 = 0x08;
/// Follows the source position (no rotation following).
pub const LL_PART_FOLLOW_SRC_MASK: u32 = 0x10;
/// Particles orient themselves along their velocity.
pub const LL_PART_FOLLOW_VELOCITY_MASK: u32 = 0x20;
/// Particles steer towards the target.
pub const LL_PART_TARGET_POS_MASK: u32 = 0x40;
/// Particles move linearly from the source to the target.
pub const LL_PART_TARGET_LINEAR_MASK: u32 = 0x80;
/// Full-bright ("emissive") instead of lit.
pub const LL_PART_EMISSIVE_MASK: u32 = 0x100;
/// Beam connecting source and target.
pub const LL_PART_BEAM_MASK: u32 = 0x200;
/// Particles are joined into one continuous triangle strip.
pub const LL_PART_RIBBON_MASK: u32 = 0x400;
/// System flag: the part block carries start/end glow bytes.
pub const LL_PART_DATA_GLOW: u32 = 0x10000;
/// System flag: the part block carries blend func bytes.
pub const LL_PART_DATA_BLEND: u32 = 0x20000;
/// Viewer-side only: particle belongs to a HUD attachment.
pub const LL_PART_HUD: u32 = 0x4000_0000;
/// Viewer-side only: particle is dead.
pub const LL_PART_DEAD_MASK: u32 = 0x8000_0000;

/// Source flag: accel and velocity are relative to the object rotation
/// (not implemented by the LL viewer either).
pub const LL_PART_SRC_OBJ_REL_MASK: u32 = 0x01;
/// Source flag: use the new "correct" angle parameters.
pub const LL_PART_USE_NEW_ANGLE: u32 = 0x02;

pub const LL_PART_SRC_PATTERN_DROP: u8 = 0x01;
pub const LL_PART_SRC_PATTERN_EXPLODE: u8 = 0x02;
pub const LL_PART_SRC_PATTERN_ANGLE: u8 = 0x04;
pub const LL_PART_SRC_PATTERN_ANGLE_CONE: u8 = 0x08;
pub const LL_PART_SRC_PATTERN_ANGLE_CONE_EMPTY: u8 = 0x10;

pub const LL_PART_BF_ONE: u8 = 0;
pub const LL_PART_BF_ZERO: u8 = 1;
pub const LL_PART_BF_DEST_COLOR: u8 = 2;
pub const LL_PART_BF_SOURCE_COLOR: u8 = 3;
pub const LL_PART_BF_ONE_MINUS_DEST_COLOR: u8 = 4;
pub const LL_PART_BF_ONE_MINUS_SOURCE_COLOR: u8 = 5;
pub const LL_PART_BF_UNSUPPORTED_DEST_ALPHA: u8 = 6;
pub const LL_PART_BF_SOURCE_ALPHA: u8 = 7;
pub const LL_PART_BF_UNSUPPORTED_ONE_MINUS_DEST_ALPHA: u8 = 8;
pub const LL_PART_BF_ONE_MINUS_SOURCE_ALPHA: u8 = 9;
pub const LL_PART_BF_COUNT: u8 = 10;

pub const PS_PART_DATA_GLOW_SIZE: usize = 2;
pub const PS_PART_DATA_BLEND_SIZE: usize = 2;
pub const PS_LEGACY_PART_DATA_BLOCK_SIZE: usize = 4 + 2 + 4 + 4 + 2 + 2; // 18
pub const PS_SYS_DATA_BLOCK_SIZE: usize = 68;
/// Largest `PSBlock` this viewer understands (bigger = newer, unsupported).
pub const PS_MAX_DATA_BLOCK_SIZE: usize =
    PS_SYS_DATA_BLOCK_SIZE + PS_LEGACY_PART_DATA_BLOCK_SIZE + PS_PART_DATA_BLEND_SIZE + PS_PART_DATA_GLOW_SIZE + 8; // two S32 size fields
/// Size of the legacy fixed-layout block (also used by compressed updates).
pub const PS_LEGACY_DATA_BLOCK_SIZE: usize = PS_SYS_DATA_BLOCK_SIZE + PS_LEGACY_PART_DATA_BLOCK_SIZE; // 86

/// Max particle scale accepted by the script setters (`MAX_PART_SCALE`).
pub const MAX_PART_SCALE: f32 = 4.0;
/// Hard cap on live particles in the LL viewer (`LL_MAX_PARTICLE_COUNT`).
pub const LL_MAX_PARTICLE_COUNT: usize = 8192;
/// `LLViewerPartSim::updateSimulation` clamps its frame delta to this.
pub const MAX_PARTICLE_DT: f32 = 0.1;
/// Minimum burst interval (`LLPartSysData::unpackSystem`).
pub const MIN_BURST_RATE: f32 = 0.01;

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// Per-particle template parameters (`LLPartData`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartData {
    /// `LL_PART_*` flags.
    pub flags: u32,
    /// Particle lifetime in seconds.
    pub max_age: f32,
    /// Straight RGBA in [0, 1] (sRGB as sent by the simulator).
    pub start_color: [f32; 4],
    pub end_color: [f32; 4],
    pub start_scale: Vec2,
    pub end_scale: Vec2,
    pub start_glow: f32,
    pub end_glow: f32,
    /// `LL_PART_BF_*`.
    pub blend_func_source: u8,
    pub blend_func_dest: u8,
}

impl Default for PartData {
    /// The template set by the `LLPartSysData` constructor.
    fn default() -> Self {
        Self {
            flags: 0,
            max_age: 10.0,
            start_color: [1.0; 4],
            end_color: [1.0; 4],
            start_scale: Vec2::ONE,
            end_scale: Vec2::ONE,
            start_glow: 0.0,
            end_glow: 0.0,
            blend_func_source: LL_PART_BF_SOURCE_ALPHA,
            blend_func_dest: LL_PART_BF_ONE_MINUS_SOURCE_ALPHA,
        }
    }
}

impl PartData {
    pub fn has_glow(&self) -> bool {
        self.start_glow > 0.0 || self.end_glow > 0.0
    }

    pub fn has_blend_func(&self) -> bool {
        self.blend_func_source != LL_PART_BF_SOURCE_ALPHA || self.blend_func_dest != LL_PART_BF_ONE_MINUS_SOURCE_ALPHA
    }

    /// `LLPartData::validBlendFunc`.
    pub fn valid_blend_func(func: u8) -> bool {
        func < LL_PART_BF_COUNT && func != LL_PART_BF_UNSUPPORTED_DEST_ALPHA && func != LL_PART_BF_UNSUPPORTED_ONE_MINUS_DEST_ALPHA
    }

    /// `LLPartData::unpackLegacy` (18 bytes).
    fn unpack_legacy(&mut self, r: &mut Reader<'_>) -> Option<()> {
        self.flags = r.u32()?;
        self.max_age = r.ufixed16(8)?;
        self.start_color = r.color4u()?;
        self.end_color = r.color4u()?;
        let sx = r.ufixed8(5)?;
        let sy = r.ufixed8(5)?;
        let ex = r.ufixed8(5)?;
        let ey = r.ufixed8(5)?;
        self.start_scale = Vec2::new(sx, sy);
        self.end_scale = Vec2::new(ex, ey);
        self.start_glow = 0.0;
        self.end_glow = 0.0;
        self.blend_func_source = LL_PART_BF_SOURCE_ALPHA;
        self.blend_func_dest = LL_PART_BF_ONE_MINUS_SOURCE_ALPHA;
        Some(())
    }

    /// `LLPartData::unpack`: S32 size, legacy block, optional glow / blend.
    fn unpack(&mut self, r: &mut Reader<'_>) -> Option<()> {
        let mut size = i64::from(r.i32()?);
        self.unpack_legacy(r)?;
        size -= PS_LEGACY_PART_DATA_BLOCK_SIZE as i64;

        if self.flags & LL_PART_DATA_GLOW != 0 {
            if size < PS_PART_DATA_GLOW_SIZE as i64 {
                return None;
            }
            self.start_glow = f32::from(r.u8()?) / 255.0;
            self.end_glow = f32::from(r.u8()?) / 255.0;
            size -= PS_PART_DATA_GLOW_SIZE as i64;
        }

        if self.flags & LL_PART_DATA_BLEND != 0 {
            if size < PS_PART_DATA_BLEND_SIZE as i64 {
                return None;
            }
            self.blend_func_source = r.u8()?;
            self.blend_func_dest = r.u8()?;
            size -= PS_PART_DATA_BLEND_SIZE as i64;
        }

        // Leftover bytes are unrecognized parameters: LL refuses to show a
        // system it can't display properly.
        if size > 0 {
            return None;
        }
        Some(())
    }
}

/// Particle source parameters (`LLPartSysData`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartSysData {
    pub crc: u32,
    /// `LL_PART_SRC_OBJ_REL_MASK` / `LL_PART_USE_NEW_ANGLE`.
    pub flags: u32,
    /// `LL_PART_SRC_PATTERN_*`.
    pub pattern: u8,
    /// Source lifetime in seconds (0 = forever).
    pub max_age: f32,
    /// Age the source already had when this update was sent.
    pub start_age: f32,
    /// Radians.
    pub inner_angle: f32,
    /// Radians.
    pub outer_angle: f32,
    /// Seconds between bursts (>= [`MIN_BURST_RATE`] once parsed).
    pub burst_rate: f32,
    pub burst_part_count: u8,
    pub burst_radius: f32,
    pub burst_speed_min: f32,
    pub burst_speed_max: f32,
    /// Angular velocity of the emission axis (rad/s), a.k.a. omega.
    pub angular_velocity: Vec3,
    pub part_accel: Vec3,
    /// Texture of the particles (nil = default particle image).
    pub part_image_id: Uuid,
    pub target_id: Uuid,
    /// Template for the emitted particles.
    pub part: PartData,
}

impl Default for PartSysData {
    /// Values set by the `LLPartSysData` constructor.
    fn default() -> Self {
        Self {
            crc: 0,
            flags: 0,
            pattern: LL_PART_SRC_PATTERN_DROP,
            max_age: 0.0,
            start_age: 0.0,
            inner_angle: 0.0,
            outer_angle: 0.0,
            burst_rate: 0.1,
            burst_part_count: 1,
            burst_radius: 0.0,
            burst_speed_min: 1.0,
            burst_speed_max: 1.0,
            angular_velocity: Vec3::ZERO,
            part_accel: Vec3::ZERO,
            part_image_id: Uuid::nil(),
            target_id: Uuid::nil(),
            part: PartData::default(),
        }
    }
}

impl PartSysData {
    /// Same as the free function [`parse`].
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        parse(bytes)
    }

    /// `LLPartSysData::isLegacyCompatible`.
    pub fn is_legacy_compatible(&self) -> bool {
        !self.part.has_glow() && !self.part.has_blend_func()
    }

    /// `LLPartSysData::unpackSystem` (68 bytes).
    fn unpack_system(&mut self, r: &mut Reader<'_>) -> Option<()> {
        self.crc = r.u32()?;
        self.flags = r.u32()?;
        self.pattern = r.u8()?;
        self.max_age = r.ufixed16(8)?;
        self.start_age = r.ufixed16(8)?;
        self.inner_angle = r.ufixed8(5)?;
        self.outer_angle = r.ufixed8(5)?;
        self.burst_rate = r.ufixed16(8)?.max(MIN_BURST_RATE);
        self.burst_radius = r.ufixed16(8)?;
        self.burst_speed_min = r.ufixed16(8)?;
        self.burst_speed_max = r.ufixed16(8)?;
        self.burst_part_count = r.u8()?;
        self.angular_velocity = Vec3::new(r.sfixed16_8_7()?, r.sfixed16_8_7()?, r.sfixed16_8_7()?);
        self.part_accel = Vec3::new(r.sfixed16_8_7()?, r.sfixed16_8_7()?, r.sfixed16_8_7()?);
        self.part_image_id = r.uuid()?;
        self.target_id = r.uuid()?;
        Some(())
    }
}

/// Parse an object's `PSBlock` (full `ObjectUpdate`) or the 86-byte legacy
/// block of a compressed update.
///
/// Returns `None` when there is no particle system (`LLPartSysData::isNullPS`:
/// empty block, CRC 0, oversized/unknown layout) or when the block can't be
/// decoded (`unpackBlock` failure, truncation). In both cases the LL viewer
/// kills the object's existing source.
pub fn parse(bytes: &[u8]) -> Option<PartSysData> {
    let size = bytes.len();
    // isNullPS
    if size == 0 || size > PS_MAX_DATA_BLOCK_SIZE {
        return None;
    }
    let mut r = Reader::new(bytes);
    if size > PS_LEGACY_DATA_BLOCK_SIZE {
        // Non-legacy systems pack a size before the CRC.
        let sys_size = r.i32()?;
        if sys_size > PS_SYS_DATA_BLOCK_SIZE as i32 {
            return None;
        }
    }
    if r.u32()? == 0 {
        return None;
    }

    // unpackBlock
    let mut data = PartSysData::default();
    let mut r = Reader::new(bytes);
    if size == PS_LEGACY_DATA_BLOCK_SIZE {
        data.unpack_system(&mut r)?;
        data.part.unpack_legacy(&mut r)?;
    } else {
        // LLPartSysData::unpack: an unexpected system block size means a
        // format this viewer doesn't know.
        let sys_size = r.i32()?;
        if sys_size != PS_SYS_DATA_BLOCK_SIZE as i32 {
            return None;
        }
        data.unpack_system(&mut r)?;
        data.part.unpack(&mut r)?;
    }
    Some(data)
}

/// Little-endian bounded reader (`LLDataPackerBinaryBuffer`).
struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, p: 0 }
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let end = self.p.checked_add(N)?;
        let s = self.d.get(self.p..end)?;
        self.p = end;
        s.try_into().ok()
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take::<1>()?[0])
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take()?))
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take()?))
    }

    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take()?))
    }

    fn uuid(&mut self) -> Option<Uuid> {
        Some(Uuid::from_bytes(self.take()?))
    }

    fn color4u(&mut self) -> Option<[f32; 4]> {
        let c: [u8; 4] = self.take()?;
        Some(c.map(|v| f32::from(v) / 255.0))
    }

    /// `unpackFixed(.., false, int_bits, frac_bits)` with int+frac <= 8.
    fn ufixed8(&mut self, frac_bits: u32) -> Option<f32> {
        Some(f32::from(self.u8()?) / (1u32 << frac_bits) as f32)
    }

    /// `unpackFixed(.., false, int_bits, frac_bits)` with 8 < int+frac <= 16.
    fn ufixed16(&mut self, frac_bits: u32) -> Option<f32> {
        Some(f32::from(self.u16()?) / (1u32 << frac_bits) as f32)
    }

    /// `unpackFixed(.., true, 8, 7)`: 16 bits, offset by 2^8.
    fn sfixed16_8_7(&mut self) -> Option<f32> {
        Some(f32::from(self.u16()?) / 128.0 - 256.0)
    }
}

// ---------------------------------------------------------------------------
// Simulation
// ---------------------------------------------------------------------------

/// Per-frame state of the emitting object, supplied by the viewer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceContext {
    /// Render position of the source object (region/agent space).
    pub pos: Vec3,
    /// Render rotation of the source object.
    pub rot: Quat,
    /// Velocity of the source object. LL does not inherit it into particles;
    /// kept for callers / future use.
    pub vel: Vec3,
    /// Render position of the target object, if it is known. `None` makes
    /// the source its own target (as LL does when the target is missing).
    pub target: Option<Vec3>,
    /// Wind velocity at the source (LL samples it at each particle; the
    /// difference is negligible at particle scale).
    pub wind: Vec3,
}

impl Default for SourceContext {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            vel: Vec3::ZERO,
            target: None,
            wind: Vec3::ZERO,
        }
    }
}

/// One live particle (`LLViewerPart`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    pub pos: Vec3,
    /// Ribbons: position of the neighbouring segment end towards the source
    /// (LL's `mParent`, the next-newer particle of the chain); the source
    /// position for the newest particle; own `pos` when there is no
    /// neighbour or the particle is not a ribbon.
    pub prev_pos: Vec3,
    pub vel: Vec3,
    pub accel: Vec3,
    pub size: Vec2,
    /// Straight (non-premultiplied) RGBA, sRGB-space as in SL.
    pub color: [f32; 4],
    /// Glow in [0, 1], quantized to 1/255 like LL's `mGlow`.
    pub glow: f32,
    pub age: f32,
    pub max_age: f32,
    /// `LL_PART_*` flags (emissive, ribbon, follow velocity, ...).
    pub flags: u32,
    pub blend_src: u8,
    pub blend_dst: u8,
    /// Ribbons: width axis of this segment end (source Z axis at spawn; zero
    /// for the first particle of a chain, as in LL).
    pub axis: Vec3,
    /// Ribbons: width axis at `prev_pos`.
    pub prev_axis: Vec3,
    /// Ribbons: width (x scale) at `prev_pos`.
    pub prev_width: f32,
    /// Ribbons: color at `prev_pos` (start color when there is no parent).
    pub prev_color: [f32; 4],
    /// Ribbons: glow at `prev_pos`.
    pub prev_glow: f32,

    start_color: [f32; 4],
    end_color: [f32; 4],
    start_scale: Vec2,
    end_scale: Vec2,
    start_glow: f32,
    end_glow: f32,
    /// Offset from the source for `LL_PART_FOLLOW_SRC_MASK`.
    pos_offset: Vec3,
    /// LL `mChild != NULL`: linked to the previous (older) particle in the
    /// vector.
    chained: bool,
}

impl Particle {
    pub fn is_ribbon(&self) -> bool {
        self.flags & LL_PART_RIBBON_MASK != 0
    }

    pub fn is_emissive(&self) -> bool {
        self.flags & LL_PART_EMISSIVE_MASK != 0
    }

    /// Normalized age in [0, 1].
    pub fn age_frac(&self) -> f32 {
        if self.max_age > 0.0 {
            (self.age / self.max_age).clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
}

/// Small deterministic PRNG (xorshift64*, seeded through splitmix64).
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Rng(if z == 0 { 0x2545_F491_4F6C_DD1D } else { z })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// `ll_frand()`: uniform in [0, 1).
    fn frand(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// LL stores glow in a U8 (`ll_round(glow * 255)`).
fn quantize_glow(g: f32) -> f32 {
    if g.is_finite() {
        (g * 255.0).round().clamp(0.0, 255.0) / 255.0
    } else {
        0.0
    }
}

/// `LLViewerPartSim::put` rejects particles beyond this squared distance.
const MAX_PART_POS_MAG2: f32 = 1_000_000.0 * 1_000_000.0;

/// A scripted particle source and its live particles
/// (`LLViewerPartSourceScript` + the particles it owns).
#[derive(Debug, Clone)]
pub struct ParticleSource {
    data: PartSysData,
    particles: Vec<Particle>,
    rng: Rng,
    /// Emission-axis rotation accumulated from `angular_velocity`.
    rotation: Quat,
    last_update_time: f32,
    last_part_time: f32,
    /// Source no longer emits (max age reached or stopped). Particles live on.
    source_dead: bool,
    /// LL `mLastPart != NULL`: the newest spawned particle is still alive
    /// (it is then the last element of `particles`).
    has_last_part: bool,
    pos: Vec3,
    rot: Quat,
    target_pos: Vec3,
    wind: Vec3,
    /// Context has been received at least once.
    have_pos: bool,
}

impl ParticleSource {
    pub fn new(data: PartSysData, seed: u64) -> Self {
        Self {
            data,
            particles: Vec::new(),
            rng: Rng::new(seed),
            rotation: Quat::IDENTITY,
            last_update_time: 0.0,
            last_part_time: 0.0,
            source_dead: false,
            has_last_part: false,
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            target_pos: Vec3::ZERO,
            wind: Vec3::ZERO,
            have_pos: false,
        }
    }

    /// Apply new parameters from an object update, keeping live particles
    /// (`LLViewerPartSourceScript::unpackPSS` on an existing source).
    ///
    /// Like LL, the source clock restarts when the new system has a max age
    /// and its max age or start age changed. A source that was dead (expired
    /// or [`stop`](Self::stop)ped) is restarted as a fresh one, as LL
    /// replaces a dead source with a new `LLViewerPartSourceScript`.
    pub fn set_data(&mut self, data: PartSysData) {
        if self.source_dead {
            self.source_dead = false;
            self.last_update_time = 0.0;
            self.last_part_time = 0.0;
            self.rotation = Quat::IDENTITY;
            self.has_last_part = false;
        } else if data.max_age != 0.0 && (self.data.max_age != data.max_age || self.data.start_age != data.start_age) {
            self.last_update_time = 0.0;
            self.last_part_time = 0.0;
        }
        self.data = data;
    }

    /// Stop emitting; existing particles live out their lifetime
    /// (`LLViewerObject::deleteParticleSource` / `setDead`).
    pub fn stop(&mut self) {
        self.source_dead = true;
    }

    /// Remove every live particle at once (e.g. owner muted).
    pub fn clear_particles(&mut self) {
        self.particles.clear();
        self.has_last_part = false;
    }

    /// Advance by `dt` seconds (clamped to [`MAX_PARTICLE_DT`] like
    /// `LLViewerPartSim::updateSimulation`): emit bursts, simulate existing
    /// particles and drop expired ones. Each spawned particle decrements
    /// `budget`; nothing is spawned once it reaches 0.
    pub fn update(&mut self, dt: f32, ctx: &SourceContext, budget: &mut usize) {
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_PARTICLE_DT) } else { 0.0 };
        if ctx.wind.is_finite() {
            self.wind = ctx.wind;
        }
        if !self.source_dead {
            self.update_source(dt, ctx, budget);
        }
        self.update_particles(dt);
        self.link_ribbons();
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    /// Particle texture (`part_image_id`; nil = default particle image).
    pub fn texture(&self) -> Uuid {
        self.data.part_image_id
    }

    pub fn data(&self) -> &PartSysData {
        &self.data
    }

    /// The source still emits (not expired, not stopped).
    pub fn is_emitting(&self) -> bool {
        !self.source_dead
    }

    /// Source max age elapsed (or stopped) and no live particles remain.
    pub fn is_dead(&self) -> bool {
        self.source_dead && self.particles.is_empty()
    }

    /// `LLViewerPartSourceScript::update`.
    fn update_source(&mut self, dt: f32, ctx: &SourceContext, budget: &mut usize) {
        let old_update_time = self.last_update_time;
        self.last_update_time += dt;
        let mut dt_update = self.last_update_time - self.last_part_time;

        if ctx.pos.is_finite() {
            self.pos = ctx.pos;
            self.have_pos = true;
        }
        let rot = ctx.rot.normalize();
        self.rot = if rot.is_finite() { rot } else { Quat::IDENTITY };
        self.target_pos = match ctx.target {
            Some(t) if t.is_finite() => t,
            _ => self.pos,
        };

        let sd = &self.data;
        if sd.max_age != 0.0 && sd.start_age + self.last_update_time + dt_update > sd.max_age {
            // Outlived its max age.
            self.source_dead = true;
            return;
        }
        if !self.have_pos {
            return;
        }

        let mut first_run = old_update_time <= 0.0;
        // Guard against hand-built data: unpack already clamps this.
        let burst_rate = self.data.burst_rate.max(MIN_BURST_RATE);
        let max_time = (10.0 * burst_rate).max(1.0);
        dt_update = dt_update.min(max_time);

        while dt_update > burst_rate || first_run {
            first_run = false;

            // Rotate the emission axis by the angular velocity.
            let av = self.data.angular_velocity;
            let av_mag = av.length();
            if av_mag != 0.0 && av_mag.is_finite() {
                let dquat = Quat::from_axis_angle(av / av_mag, dt * av_mag);
                self.rotation = (dquat * self.rotation).normalize();
            } else {
                self.rotation = Quat::IDENTITY;
            }

            if *budget == 0 {
                // Above the particle limit: give up for this frame.
                self.last_part_time = self.last_update_time;
                break;
            }

            for _ in 0..self.data.burst_part_count {
                if *budget == 0 {
                    break;
                }
                if self.spawn() {
                    *budget -= 1;
                }
            }

            self.last_part_time = self.last_update_time;
            dt_update -= burst_rate;
        }
    }

    /// Create one particle (inner loop of `LLViewerPartSourceScript::update`).
    fn spawn(&mut self) -> bool {
        let sd = self.data;
        let pd = sd.part;
        let flags = pd.flags;

        let chained = flags & LL_PART_RIBBON_MASK != 0 && self.has_last_part;
        let axis = if chained { self.rot * Vec3::Z } else { Vec3::ZERO };

        let mut pos = self.pos;
        let mut vel = Vec3::ZERO;
        if sd.pattern & LL_PART_SRC_PATTERN_DROP != 0 {
            // At the source, no velocity.
        } else if sd.pattern & LL_PART_SRC_PATTERN_EXPLODE != 0 {
            let dir = self.random_direction();
            pos += sd.burst_radius * dir;
            let speed = sd.burst_speed_min + self.rng.frand() * (sd.burst_speed_max - sd.burst_speed_min);
            vel = dir * speed;
        } else if sd.pattern & (LL_PART_SRC_PATTERN_ANGLE | LL_PART_SRC_PATTERN_ANGLE_CONE) != 0 {
            let inner = sd.inner_angle;
            let outer = sd.outer_angle;
            // Random angle between inner and outer, on a random side.
            let mut angle = inner + self.rng.frand() * (outer - inner);
            if self.rng.frand() < 0.5 {
                angle = -angle;
            }
            // Both patterns rotate around the x axis first...
            let mut dir = Quat::from_rotation_x(angle) * Vec3::Z;
            // ...cones then spin around z.
            if sd.pattern & LL_PART_SRC_PATTERN_ANGLE_CONE != 0 {
                dir = Quat::from_rotation_z(self.rng.frand() * 4.0 * std::f32::consts::PI) * dir;
            }
            if sd.flags & LL_PART_USE_NEW_ANGLE == 0 {
                // Deprecated angle convention.
                dir = Quat::from_rotation_x(outer) * dir;
            }
            dir = self.rot * dir;
            dir = self.rotation * dir;
            pos += sd.burst_radius * dir;
            let speed = sd.burst_speed_min + self.rng.frand() * (sd.burst_speed_max - sd.burst_speed_min);
            vel = dir * speed;
        }
        // Unknown patterns (incl. ANGLE_CONE_EMPTY, which LL never
        // implemented): at the source, no velocity.

        if flags & (LL_PART_FOLLOW_SRC_MASK | LL_PART_TARGET_LINEAR_MASK) != 0 {
            // SVC-193, VWR-717: LL zeroes the radius after the first spawn.
            self.data.burst_radius = 0.0;
        }

        // LLViewerPartSim::addPart / put: drop out-of-range particles.
        if !pos.is_finite() || pos.length_squared() > MAX_PART_POS_MAG2 || !vel.is_finite() {
            return false;
        }

        self.particles.push(Particle {
            pos,
            prev_pos: pos,
            vel,
            accel: sd.part_accel,
            size: pd.start_scale,
            color: pd.start_color,
            glow: quantize_glow(pd.start_glow),
            age: 0.0,
            max_age: pd.max_age,
            flags,
            blend_src: pd.blend_func_source,
            blend_dst: pd.blend_func_dest,
            axis,
            prev_axis: axis,
            prev_width: pd.start_scale.x,
            prev_color: pd.start_color,
            prev_glow: quantize_glow(pd.start_glow),
            start_color: pd.start_color,
            end_color: pd.end_color,
            start_scale: pd.start_scale,
            end_scale: pd.end_scale,
            start_glow: pd.start_glow,
            end_glow: pd.end_glow,
            pos_offset: Vec3::ZERO,
            chained,
        });
        self.has_last_part = true;
        true
    }

    /// Uniform random unit vector by rejection sampling (EXPLODE pattern).
    fn random_direction(&mut self) -> Vec3 {
        for _ in 0..64 {
            let v = Vec3::new(
                self.rng.frand() * 2.0 - 1.0,
                self.rng.frand() * 2.0 - 1.0,
                self.rng.frand() * 2.0 - 1.0,
            );
            let mvs = v.length_squared();
            if (0.01..=1.0).contains(&mvs) {
                return v / mvs.sqrt();
            }
        }
        Vec3::Z
    }

    /// `LLViewerPartGroup::updateParticles` for this source's particles.
    /// Order-preserving in-place compaction keeps the ribbon chain in spawn
    /// order without allocating.
    fn update_particles(&mut self, dt: f32) {
        let src_pos = self.pos;
        let target = self.target_pos;
        let wind = self.wind;
        let n = self.particles.len();
        let mut w = 0;
        // `Some(chained)` when the previous element was removed.
        let mut removed_prev: Option<bool> = None;

        for r in 0..n {
            let mut p = self.particles[r];

            // Patch up holes in the ribbon (~LLViewerPart).
            if let Some(c) = removed_prev
                && p.chained
            {
                p.chained = c;
            }

            let cur_time = p.age + dt;
            let frac = if p.max_age > 0.0 { cur_time / p.max_age } else { 1.0 };

            if p.flags & LL_PART_FOLLOW_SRC_MASK != 0 {
                p.pos = src_pos + p.pos_offset;
            }

            if p.flags & LL_PART_WIND_MASK != 0 {
                p.vel *= 1.0 - 0.1 * dt;
                p.vel += 0.1 * dt * wind;
            }

            if p.flags & LL_PART_TARGET_POS_MASK != 0 {
                let remaining = p.max_age - p.age;
                if remaining > 0.0 {
                    let step = (dt / remaining).clamp(0.0, 0.1) * 5.0;
                    let delta = (target - p.pos) / remaining;
                    p.vel = p.vel * (1.0 - step) + step * delta;
                }
            }

            if p.flags & LL_PART_TARGET_LINEAR_MASK != 0 {
                let delta = target - src_pos;
                p.pos = src_pos + frac * delta;
                p.vel = delta;
            } else {
                p.pos += dt * p.vel;
                p.pos += 0.5 * dt * dt * p.accel;
                p.vel += p.accel * dt;
            }

            if p.flags & LL_PART_BOUNCE_MASK != 0 {
                // LL only checks against the source height.
                let dz = p.pos.z - src_pos.z;
                if dz < 0.0 {
                    p.pos.z += -2.0 * dz;
                    p.vel.z *= -0.75;
                }
            }

            if p.flags & LL_PART_FOLLOW_SRC_MASK != 0 {
                p.pos_offset = p.pos - src_pos;
            }

            if p.flags & LL_PART_INTERP_COLOR_MASK != 0 {
                p.color = lerp4(p.start_color, p.end_color, frac);
            }

            if p.flags & LL_PART_INTERP_SCALE_MASK != 0 {
                p.size = p.start_scale * (1.0 - frac) + frac * p.end_scale;
            }

            p.glow = quantize_glow(p.start_glow + (p.end_glow - p.start_glow) * frac);
            p.age = cur_time;

            // (NaN max age from hand-built data counts as expired.)
            let alive = p.age <= p.max_age && p.pos.is_finite() && p.vel.is_finite() && p.pos.length_squared() <= MAX_PART_POS_MAG2;
            if alive {
                self.particles[w] = p;
                w += 1;
                removed_prev = None;
            } else {
                removed_prev = Some(p.chained);
                if r + 1 == n {
                    // The newest particle died: next spawn starts a new chain.
                    self.has_last_part = false;
                }
            }
        }
        self.particles.truncate(w);
    }

    /// Fill the `prev_*` fields from the ribbon chain (what
    /// `LLVOPartGroup::getGeometry` reads through `mParent`).
    fn link_ribbons(&mut self) {
        let n = self.particles.len();
        let src_alive = !self.source_dead && self.have_pos;
        let src_pos = self.pos;
        let src_axis = self.rot * Vec3::Z;
        for i in 0..n {
            let parent = self.particles.get(i + 1).filter(|q| q.chained).copied();
            let p = &mut self.particles[i];
            if p.flags & LL_PART_RIBBON_MASK == 0 {
                p.prev_pos = p.pos;
                p.prev_axis = p.axis;
                p.prev_width = p.size.x;
                p.prev_color = p.color;
                p.prev_glow = p.glow;
                continue;
            }
            match parent {
                Some(q) => {
                    p.prev_pos = q.pos;
                    p.prev_axis = q.axis;
                    p.prev_width = q.size.x;
                    p.prev_color = q.color;
                    p.prev_glow = q.glow;
                }
                None => {
                    if src_alive {
                        p.prev_pos = src_pos;
                        p.prev_axis = src_axis;
                        p.prev_width = p.start_scale.x;
                    } else {
                        p.prev_pos = p.pos;
                        p.prev_axis = p.axis;
                        p.prev_width = p.size.x;
                    }
                    p.prev_color = p.start_color;
                    p.prev_glow = quantize_glow(p.start_glow);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Encoders mirroring LLDataPacker::packFixed / LLPartSysData::pack.
    fn ufix8(v: f32, frac: u32) -> u8 {
        (v * (1u32 << frac) as f32) as u8
    }
    fn ufix16(v: f32, frac: u32) -> [u8; 2] {
        ((v * (1u32 << frac) as f32) as u16).to_le_bytes()
    }
    fn sfix16(v: f32) -> [u8; 2] {
        (((v + 256.0) * 128.0) as u16).to_le_bytes()
    }

    struct Sys {
        crc: u32,
        flags: u32,
        pattern: u8,
        max_age: f32,
        start_age: f32,
        inner: f32,
        outer: f32,
        burst_rate: f32,
        burst_radius: f32,
        speed_min: f32,
        speed_max: f32,
        count: u8,
        omega: [f32; 3],
        accel: [f32; 3],
        image: Uuid,
        target: Uuid,
    }

    fn sys_default() -> Sys {
        Sys {
            crc: 0x1234_5678,
            flags: LL_PART_USE_NEW_ANGLE,
            pattern: LL_PART_SRC_PATTERN_EXPLODE,
            max_age: 0.0,
            start_age: 0.0,
            inner: 0.5,
            outer: 1.25,
            burst_rate: 0.5,
            burst_radius: 0.25,
            speed_min: 1.0,
            speed_max: 2.5,
            count: 10,
            omega: [0.0, 0.0, 1.5],
            accel: [0.0, 0.0, -9.75],
            image: Uuid::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888),
            target: Uuid::nil(),
        }
    }

    fn pack_sys(s: &Sys, out: &mut Vec<u8>) {
        out.extend_from_slice(&s.crc.to_le_bytes());
        out.extend_from_slice(&s.flags.to_le_bytes());
        out.push(s.pattern);
        out.extend_from_slice(&ufix16(s.max_age, 8));
        out.extend_from_slice(&ufix16(s.start_age, 8));
        out.push(ufix8(s.inner, 5));
        out.push(ufix8(s.outer, 5));
        out.extend_from_slice(&ufix16(s.burst_rate, 8));
        out.extend_from_slice(&ufix16(s.burst_radius, 8));
        out.extend_from_slice(&ufix16(s.speed_min, 8));
        out.extend_from_slice(&ufix16(s.speed_max, 8));
        out.push(s.count);
        for v in s.omega {
            out.extend_from_slice(&sfix16(v));
        }
        for v in s.accel {
            out.extend_from_slice(&sfix16(v));
        }
        out.extend_from_slice(s.image.as_bytes());
        out.extend_from_slice(s.target.as_bytes());
    }

    fn pack_part_legacy(flags: u32, max_age: f32, out: &mut Vec<u8>) {
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&ufix16(max_age, 8));
        out.extend_from_slice(&[255, 128, 0, 255]); // start color
        out.extend_from_slice(&[0, 0, 255, 0]); // end color
        out.push(ufix8(0.5, 5));
        out.push(ufix8(0.25, 5));
        out.push(ufix8(2.0, 5));
        out.push(ufix8(3.5, 5));
    }

    fn legacy_block(s: &Sys, part_flags: u32, part_max_age: f32) -> Vec<u8> {
        let mut v = Vec::new();
        pack_sys(s, &mut v);
        pack_part_legacy(part_flags, part_max_age, &mut v);
        v
    }

    fn new_block(s: &Sys, part_flags: u32, glow: Option<(u8, u8)>, blend: Option<(u8, u8)>) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(PS_SYS_DATA_BLOCK_SIZE as i32).to_le_bytes());
        pack_sys(s, &mut v);
        let mut flags = part_flags;
        let mut size = PS_LEGACY_PART_DATA_BLOCK_SIZE;
        if glow.is_some() {
            flags |= LL_PART_DATA_GLOW;
            size += PS_PART_DATA_GLOW_SIZE;
        }
        if blend.is_some() {
            flags |= LL_PART_DATA_BLEND;
            size += PS_PART_DATA_BLEND_SIZE;
        }
        v.extend_from_slice(&(size as i32).to_le_bytes());
        pack_part_legacy(flags, 4.0, &mut v);
        if let Some((a, b)) = glow {
            v.extend_from_slice(&[a, b]);
        }
        if let Some((a, b)) = blend {
            v.extend_from_slice(&[a, b]);
        }
        v
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn parse_legacy_block() {
        let s = sys_default();
        let bytes = legacy_block(&s, LL_PART_INTERP_COLOR_MASK | LL_PART_EMISSIVE_MASK, 3.0);
        assert_eq!(bytes.len(), PS_LEGACY_DATA_BLOCK_SIZE);
        let d = parse(&bytes).expect("legacy block parses");
        assert_eq!(d.crc, 0x1234_5678);
        assert_eq!(d.flags, LL_PART_USE_NEW_ANGLE);
        assert_eq!(d.pattern, LL_PART_SRC_PATTERN_EXPLODE);
        assert!(close(d.inner_angle, 0.5));
        assert!(close(d.outer_angle, 1.25));
        assert!(close(d.burst_rate, 0.5));
        assert!(close(d.burst_radius, 0.25));
        assert!(close(d.burst_speed_min, 1.0));
        assert!(close(d.burst_speed_max, 2.5));
        assert_eq!(d.burst_part_count, 10);
        assert!(close(d.angular_velocity.z, 1.5));
        assert!(close(d.part_accel.z, -9.75));
        assert_eq!(d.part_image_id, s.image);
        assert!(d.target_id.is_nil());
        assert_eq!(d.part.flags, LL_PART_INTERP_COLOR_MASK | LL_PART_EMISSIVE_MASK);
        assert!(close(d.part.max_age, 3.0));
        assert!(close(d.part.start_color[1], 128.0 / 255.0));
        assert_eq!(d.part.end_color, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(d.part.start_scale, Vec2::new(0.5, 0.25));
        assert_eq!(d.part.end_scale, Vec2::new(2.0, 3.5));
        assert_eq!(d.part.blend_func_source, LL_PART_BF_SOURCE_ALPHA);
        assert_eq!(d.part.blend_func_dest, LL_PART_BF_ONE_MINUS_SOURCE_ALPHA);
        assert!(d.is_legacy_compatible());
    }

    #[test]
    fn parse_new_format_block() {
        let s = sys_default();
        let bytes = new_block(&s, LL_PART_RIBBON_MASK, Some((255, 51)), Some((LL_PART_BF_ONE, LL_PART_BF_ONE)));
        assert_eq!(bytes.len(), PS_MAX_DATA_BLOCK_SIZE);
        let d = parse(&bytes).expect("new block parses");
        assert_eq!(d.crc, s.crc);
        assert_eq!(d.burst_part_count, 10);
        assert_eq!(d.part.flags & LL_PART_RIBBON_MASK, LL_PART_RIBBON_MASK);
        assert!(close(d.part.start_glow, 1.0));
        assert!(close(d.part.end_glow, 0.2));
        assert_eq!(d.part.blend_func_source, LL_PART_BF_ONE);
        assert_eq!(d.part.blend_func_dest, LL_PART_BF_ONE);
        assert!(!d.is_legacy_compatible());

        // Glow only.
        let d = parse(&new_block(&s, 0, Some((10, 20)), None)).expect("glow only");
        assert!(close(d.part.start_glow, 10.0 / 255.0));
        assert_eq!(d.part.blend_func_source, LL_PART_BF_SOURCE_ALPHA);

        // Neither.
        let plain = new_block(&s, 0, None, None);
        assert_eq!(plain.len(), 94);
        assert!(parse(&plain).is_some());
    }

    #[test]
    fn parse_rejects_null_and_bad_blocks() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0u8; PS_LEGACY_DATA_BLOCK_SIZE]).is_none());
        let mut s = sys_default();
        s.crc = 0;
        assert!(parse(&legacy_block(&s, 0, 1.0)).is_none());

        let s = sys_default();
        // Too big.
        let mut big = new_block(&s, 0, Some((1, 1)), Some((1, 1)));
        big.push(0);
        assert!(parse(&big).is_none());
        // Unknown system block size.
        let mut bad = new_block(&s, 0, None, None);
        bad[0] = 67;
        assert!(parse(&bad).is_none());
        // Glow flag but no room for glow bytes.
        let mut v = new_block(&s, 0, None, None);
        let flags_at = 4 + PS_SYS_DATA_BLOCK_SIZE + 4;
        v[flags_at + 2] |= 0x01; // LL_PART_DATA_GLOW
        assert!(parse(&v).is_none());
        // Leftover unknown bytes in the part block.
        let mut v = new_block(&s, 0, None, None);
        let size_at = 4 + PS_SYS_DATA_BLOCK_SIZE;
        v[size_at] = 20;
        v.extend_from_slice(&[0, 0]);
        assert!(parse(&v).is_none());
        // Every truncation is handled.
        let full = new_block(&s, 0, Some((1, 2)), Some((3, 4)));
        for n in 0..full.len() {
            let _ = parse(&full[..n]);
        }
        let legacy = legacy_block(&s, 0, 1.0);
        for n in 0..legacy.len() {
            let _ = parse(&legacy[..n]);
        }
    }

    #[test]
    fn burst_rate_is_clamped() {
        let mut s = sys_default();
        s.burst_rate = 0.0;
        let d = parse(&legacy_block(&s, 0, 1.0)).expect("parses");
        assert!(close(d.burst_rate, MIN_BURST_RATE));
    }

    fn explode_data(count: u8, part_max_age: f32) -> PartSysData {
        let mut s = sys_default();
        s.count = count;
        s.burst_rate = 100.0;
        s.omega = [0.0; 3];
        s.accel = [0.0; 3];
        parse(&legacy_block(
            &s,
            LL_PART_INTERP_COLOR_MASK | LL_PART_INTERP_SCALE_MASK,
            part_max_age,
        ))
        .expect("parses")
    }

    #[test]
    fn explode_spawns_ages_and_expires() {
        let data = explode_data(10, 1.0);
        let mut src = ParticleSource::new(data, 42);
        let ctx = SourceContext {
            pos: Vec3::new(128.0, 128.0, 25.0),
            ..Default::default()
        };
        let mut budget = 100;
        src.update(0.05, &ctx, &mut budget);
        assert_eq!(src.particles().len(), 10);
        assert_eq!(budget, 90);
        for p in src.particles() {
            // Spawned at burst radius, moving outward at 1..2.5 m/s.
            let speed = p.vel.length();
            assert!((0.99..=2.51).contains(&speed), "speed {speed}");
            let off = p.pos - ctx.pos;
            assert!(off.length() > 0.2 && off.length() < 0.5);
            assert!(close(p.age, 0.05));
        }
        // No new burst before burst_rate elapses; particles age and fade.
        for _ in 0..10 {
            src.update(0.05, &ctx, &mut budget);
        }
        assert_eq!(src.particles().len(), 10);
        assert_eq!(budget, 90);
        let p = src.particles()[0];
        assert!(close(p.age, 0.55));
        assert!(p.color[3] < 1.0);
        assert!(p.size.x > 0.5);
        // Expire.
        for _ in 0..20 {
            src.update(0.05, &ctx, &mut budget);
        }
        assert!(src.particles().is_empty());
        assert!(!src.is_dead()); // source itself has no max age
        src.stop();
        assert!(src.is_dead());
    }

    #[test]
    fn zero_budget_spawns_nothing() {
        let mut src = ParticleSource::new(explode_data(10, 1.0), 7);
        let ctx = SourceContext::default();
        let mut budget = 0;
        src.update(0.05, &ctx, &mut budget);
        assert!(src.particles().is_empty());
        assert_eq!(budget, 0);

        let mut src = ParticleSource::new(explode_data(10, 1.0), 7);
        let mut budget = 4;
        src.update(0.05, &ctx, &mut budget);
        assert_eq!(src.particles().len(), 4);
        assert_eq!(budget, 0);
    }

    #[test]
    fn source_max_age_kills_source() {
        let mut data = explode_data(2, 0.5);
        data.max_age = 0.3;
        data.burst_rate = 0.1;
        let mut src = ParticleSource::new(data, 1);
        let ctx = SourceContext::default();
        let mut budget = usize::MAX;
        for _ in 0..40 {
            src.update(0.05, &ctx, &mut budget);
        }
        assert!(!src.is_emitting());
        assert!(src.is_dead());
        // A new update with a different start age restarts it.
        let mut d2 = data;
        d2.start_age = 0.1;
        src.set_data(d2);
        src.update(0.05, &ctx, &mut budget);
        assert!(src.is_emitting());
        assert_eq!(src.particles().len(), 2);
    }

    #[test]
    fn ribbon_chain_links_to_newer_particle() {
        let data = PartSysData {
            crc: 1,
            pattern: LL_PART_SRC_PATTERN_DROP,
            burst_rate: 0.05,
            burst_part_count: 1,
            part_accel: Vec3::new(0.0, 0.0, 1.0),
            part: PartData {
                flags: LL_PART_RIBBON_MASK,
                max_age: 10.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut src = ParticleSource::new(data, 3);
        let mut budget = usize::MAX;
        let mut ctx = SourceContext::default();
        for i in 0..8 {
            ctx.pos = Vec3::new(i as f32, 0.0, 0.0);
            src.update(0.06, &ctx, &mut budget);
        }
        let ps = src.particles();
        assert!(ps.len() >= 4);
        for w in ps.windows(2) {
            assert_eq!(w[0].prev_pos, w[1].pos);
        }
        let newest = ps[ps.len() - 1];
        assert_eq!(newest.prev_pos, ctx.pos);
        assert_eq!(newest.axis, Vec3::Z);
        // First of the chain has no width axis (as in LL).
        assert_eq!(ps[0].axis, Vec3::ZERO);
    }

    #[test]
    fn angle_cone_emits_within_cone() {
        let data = PartSysData {
            crc: 1,
            flags: LL_PART_USE_NEW_ANGLE,
            pattern: LL_PART_SRC_PATTERN_ANGLE_CONE,
            inner_angle: 0.0,
            outer_angle: 0.3,
            burst_part_count: 50,
            burst_rate: 100.0,
            burst_speed_min: 1.0,
            burst_speed_max: 1.0,
            ..Default::default()
        };
        let mut src = ParticleSource::new(data, 9);
        let mut budget = usize::MAX;
        src.update(0.0, &SourceContext::default(), &mut budget);
        assert_eq!(src.particles().len(), 50);
        for p in src.particles() {
            let d = p.vel.normalize();
            assert!(d.angle_between(Vec3::Z) <= 0.3 + 1e-4);
        }
    }

    #[test]
    fn target_linear_reaches_target() {
        let data = PartSysData {
            crc: 1,
            burst_rate: 100.0,
            part: PartData {
                flags: LL_PART_TARGET_LINEAR_MASK,
                max_age: 1.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut src = ParticleSource::new(data, 5);
        let ctx = SourceContext {
            target: Some(Vec3::new(10.0, 0.0, 0.0)),
            ..Default::default()
        };
        let mut budget = usize::MAX;
        for _ in 0..10 {
            src.update(0.095, &ctx, &mut budget);
        }
        let p = src.particles()[0];
        assert!(close(p.pos.x, 10.0 * p.age));
    }
}
