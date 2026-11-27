// Shared by the visible surface and the refracted-light projection.
// Metre/second units; periodic micro-waves tile over 16 m without drifting seams.
// The wind's sea (water_sea.rs `Sea`): worked out once a frame on the CPU,
// which samples the same trains for buoyancy and contacts.
struct WaterSea {
    trains: array<vec4<f32>,4>, // direction xy, wavenumber, amplitude
    phase: vec4<f32>,           // per train, at frame.z
    omega: vec4<f32>,           // per train
    shore: vec4<f32>,           // x grid crossfade, yz the spectrum's tiles (m), w its Hs (0: none)
    frame: vec4<f32>,           // anchor xy, epoch, roughness
    wind: vec4<f32>,            // toward xy, speed, highest crest
    look: vec4<f32>,            // x whitecap foam, y the spectrum's chop scale (1 by default), zw the caustics' drift
    lake: vec4<f32>,            // the lakes' cascade (water_fft.rs LAKE): x its tile (0: none), y its Hs; z the wind sea's own wave's wavenumber, w its phase at frame.z (Sea::lake_wave)
};
// How much of the open sea's two cascades (x) and of the lakes' (y) the
// wind sea takes where the sea grid's exposure gives `chop` (its packed
// fetch): open water keeps the open sea; over a short fetch it gives way
// to the lakes' chop, as high as the fetch lets it grow (water_sea.rs
// `ocean_weights`).
fn water_ocean_weights(chop: f32) -> vec2<f32> {
    let open=smoothstep(0.55,0.95,chop);
    return vec2<f32>(open,(1.0-open)*clamp(chop/0.6,0.0,1.5));
}
// Where the sea is read (water_sea.rs `SeaPoint`): the wave depth and its
// gradient, the share the shelter leaves, and the main swell there: its
// spatial phase, local wave vector (refracted to face the shore) and the
// island shadow on it.
// `shelter`: how calm what stands in the water keeps it; `cap`: how much
// the clearance above leaves (water_sea_cap).
// `shadow`: how much of the open sea's swell reaches it (none in a lake);
// `chop`: how much wind chop it builds (less with a short fetch).
// `bed`: the depth of the real bed under the still water (the
// bathymetry's, not smoothed; below 0 on land): where the swash runs up
// and drains back, which must meet the real sand.
// `lake`: how much of the main wave is the wind sea's own where the swell
// does not reach (water_lake_share); past a half, `phase` and `wave` are
// that wave's (water_lake_wave), not the swell's.
struct WaterSeaAt { depth: f32, slope: vec2<f32>, shelter: f32, cap: f32, phase: f32, wave: vec2<f32>, shadow: f32, chop: f32, bed: f32, lake: f32 };
// `breaking` and `crest`: the main swell's as it breaks; `surf`: the
// beach's surf similarity (water_surf_similarity: spilling, plunging, crashing).
// The spectrum's wind sea at a point (water_fft.rs `OceanAt`, read by
// water_fx_ocean): displacement, slope, and the displacement's derivatives
// (xx, xy, yy), and the crests' folding now or lately (`foam`: the
// lingering foam's cue). All zero without a spectrum.
struct WaterOceanAt { offset: vec3<f32>, slope: vec2<f32>, jacobian: vec3<f32>, foam: f32 };
const WATER_OCEAN_NONE=WaterOceanAt(vec3<f32>(0.0),vec2<f32>(0.0),vec3<f32>(0.0),0.0);
// `swash`: how much of the surface here is the swash running up the beach
// and draining back (its foam is the lace of water_swash_foam). `linger`:
// the wind sea's folding now or in the last seconds (water_fft.rs), the
// foam its crests leave behind.
// `chop`: the scale the spectrum's maps take here (its chop, the Sea
// panel's), for the GPU's finer maps read beside them.
// `breakers`: the fold the whitecaps take, the wind chop's (a little more
// on the swell's crests), where `compression` is the whole sea's.
struct WaterSwell { offset: vec3<f32>, gradient: vec2<f32>, compression: f32, breaking: f32, crest: f32, surf: f32, swash: f32, linger: f32, chop: f32, breakers: f32 };
// How much of the wind sea's sway (its horizontal displacement) is kept
// where its derivatives, scaled as the surface takes them, are `j` (xx,
// xy, yy): all of it, but where the surface would fold over itself (the
// Jacobian's smaller eigenvalue under 0.2: Tessendorf, "Simulating Ocean
// Water", 2004), just enough to keep it from folding. A chop and crests
// turned far up looped the surface behind every crest, its triangles
// stretched into smeared streaks. Water_sea.rs `chop_sway` is the same.
fn water_chop_sway(j:vec3<f32>)->f32 {
    let a=1.0+j.x;let c=1.0+j.z;
    let low=0.5*(a+c)-sqrt(0.25*(a-c)*(a-c)+j.y*j.y);
    return min(1.0,0.8/max(1.0-low,1e-3));
}
// The beach's surf similarity (water_sea.rs `surf_similarity`): its slope
// over the square root of the deep-water steepness of the waves reaching
// it, `height` m high and `wavelength` m long. Under ~0.5 they spill, up to
// ~3 they plunge, past it they crash on what stands steep.
fn water_surf_similarity(slope: f32, height: f32, wavelength: f32) -> f32 {
    return slope/sqrt(max(height,0.02)/max(wavelength,0.1));
}
// How much of the main wave is the wind sea's own rather than the open
// sea's swell, where `shadow` of the swell reaches: all of it where none
// does (a lake, a closed cove), none where it comes in; never both, the
// swell fading out to the middle of the way, then the lake's wave in (each
// with its own phase, a mix would beat). Water_sea.rs `lake_share`.
fn water_lake_share(shadow: f32) -> f32 {
    return 1.0-smoothstep(0.02,0.2,shadow);
}
// The wind sea's own wave where the swell does not reach (Sea::lake_wave):
// the peak of the chop over a lake, running downwind. Its way (xy, unit)
// and wavenumber (z, rad/m).
fn water_lake_wave(sea: WaterSea) -> vec3<f32> {
    let w=sea.wind.xy;
    let dir=select(vec2<f32>(1.0,0.0),w/max(length(w),1e-6),dot(w,w)>1e-12);
    return vec3<f32>(dir,max(sea.lake.z,1e-3));
}
// The main wave where the sea is read as `at` (Sea::main): the open sea's
// swell, dimmed behind land; or where it does not reach, the wind sea's own
// peak wave, which the chop's spectrum draws in deep water, but whose
// crests line up into one train as the water shoals under half its length:
// it grows in while the chop gives way to it (`take`), and shoals, breaks
// and runs up the downwind shores as the swell does on the sea's. Its
// amplitude (m, before it shoals), deep-water height and length (m, for the
// surf similarity), the chop's share it takes over, and its clock: phase
// at the sea's epoch and angular frequency.
struct WaterMain { amp: f32, height: f32, wavelength: f32, take: f32, phase: f32, omega: f32 };
fn water_sea_main(sea: WaterSea, at: WaterSeaAt) -> WaterMain {
    let lake=clamp(at.lake,0.0,1.0);
    let a0=sea.trains[0].w;
    var m:WaterMain;
    if(lake>0.5){
        let weights=water_ocean_weights(at.chop);
        let rest=select(sea.trains[1].w+sea.trains[2].w+sea.trains[3].w,0.5*(sea.shore.w*weights.x+sea.lake.y*weights.y),sea.shore.w>0.0);
        // (the trains' chop takes the fetch's share in, the spectrum's has
        // it in its cascades' shares)
        let base=select(rest*at.chop,rest,sea.shore.w>0.0);
        let k=water_lake_wave(sea).z;
        let wavelength=6.2831853/k;
        let own=2.0*lake-1.0;
        let shoal=1.0-smoothstep(0.25*wavelength,0.5*wavelength,at.depth);
        m.amp=base*shoal*own;m.height=2.0*base;m.wavelength=wavelength;m.take=0.5*shoal*own; // (its peak: about half the chop's energy, water_sea.rs LAKE_TAKE)
        m.phase=sea.lake.w;m.omega=sqrt(9.81*k);
    } else {
        m.amp=a0*at.shadow*(1.0-2.0*lake);m.height=2.0*a0;m.wavelength=6.2831853/max(sea.trains[0].z,1e-4);m.take=0.0;
        m.phase=sea.phase[0];m.omega=sea.omega[0];
    }
    return m;
}
// The main wave's phase at `at` at `time` (the frame clock): its spatial
// phase there less its clock (water_sea_main).
fn water_main_theta(sea: WaterSea, at: WaterSeaAt, time: f32) -> f32 {
    let m=water_sea_main(sea,at);
    return at.phase-(m.phase+m.omega*(time-sea.frame.z));
}
// The swash's rise (x, 0..1) at the swell's phase and its rate along the
// phase (y): up the beach in a third of the period, back over the rest
// (water_sea.rs `swash_rise`).
fn water_swash_rise(theta: f32) -> vec2<f32> {
    let s=fract(-theta/6.2831853);
    if(s<0.3){let t=s/0.3;return vec2<f32>(t*t*(3.0-2.0*t),-6.0*t*(1.0-t)/0.3/6.2831853);}
    let t=(s-0.3)/0.7;
    return vec2<f32>(1.0-t*t*(3.0-2.0*t),6.0*t*(1.0-t)/0.7/6.2831853);
}
// How far up the beach the swash runs above the still water (m), where the
// sea is read as `at` (Sea::local's `reach`): higher for the bigger swell,
// for plunging waves, lower where they crash on what stands steep.
fn water_swash_reach(sea: WaterSea, at: WaterSeaAt) -> f32 {
    let m=water_sea_main(sea,at);
    let surf=water_surf_similarity(length(at.slope),m.height,m.wavelength);
    let plunge=smoothstep(0.4,1.2,surf);let crash=smoothstep(2.5,4.0,surf);
    return min(m.amp*(0.6+0.6*plunge)*(1.0-0.7*crash),1.0)*clamp(at.shelter,0.0,1.0)*clamp(at.cap,0.0,1.0);
}
// How much of a beach the swash runs up where the sea is read as `at`
// (Sea::local's `beach`): a slope gentle enough to run up (a cliff or a
// rock face takes the waves as a crash, scene/water_crash.rs) turned to
// the incoming swell (none up the back of a sandbar or a land bridge).
fn water_swash_beach(at: WaterSeaAt) -> f32 {
    let slope=length(at.slope);
    if(slope<1e-4){return 0.0;}
    let facing=smoothstep(0.0,0.35,-dot(at.wave,at.slope)/max(length(at.wave)*slope,1e-6));
    return smoothstep(0.004,0.02,slope)*(1.0-smoothstep(0.3,0.7,slope))*facing;
}
// The swash's phase where the sea is read as `at` (Sea::local's
// `shore_shift`): the swell's where it meets the shoreline (depth 0) just
// down the beach, so the whole swash runs up and drains back as one sheet.
// (Each point's own phase, refraction shortening the swell to a dozen
// metres there, drew bands of water sliding up the sand instead.)
fn water_swash_shift(at: WaterSeaAt) -> f32 {
    let fall=length(at.slope);
    let down=at.slope/max(fall,1e-4);
    return dot(at.wave,down)*clamp(-at.bed/max(fall,0.01),-60.0,60.0);
}
// The backwash, drawn on the sand (the terrain's `water_sand_backwash`):
// as the swash slides back down the beach, a thin sheet of water stays on
// the sand behind its falling front. Its upper edge slides down after the
// front, slower at first (the water up the beach drains, it does not sink
// in), so the wave is seen running back to the sea. `theta` the swell's
// phase, `run` the front's height now and `reach` its highest, `e` the
// sand's height above the still water (m). Returns the sheet's thickness
// (m), how far into the backwash the swash is (0..1) and the height of the
// sheet's edge (m). (On the sand, not the water's mesh: the mesh reads the
// smoothed sea grid, and a sheet a few centimetres thick on it tore into
// shards on the real sand.)
fn water_backwash(theta: f32, run: f32, reach: f32, e: f32) -> vec3<f32> {
    let s=fract(-theta/6.2831853);
    let back=smoothstep(0.3,0.34,s)*(1.0-smoothstep(0.9,1.0,s));
    // the edge: from the swash's highest down to the water as it falls
    // back, lagging the front (1 - u² against its 1 - u²(3 - 2u))
    let u=clamp((s-0.3)/0.7,0.0,1.0);
    let top=reach*(1.0-u*u);
    let inside=1.0-smoothstep(top-0.03-0.03*reach,top+0.005,e);
    // thicker behind the front, thinning to the edge
    let thick=mix(1.0,0.35,clamp((e-run)/max(top-run,0.01),0.0,1.0));
    let film=0.05*back*inside*thick*smoothstep(0.0,0.04,e);
    return vec3<f32>(film,back,top);
}
// How wet the sand is `e` m above the still water (0..1): under the swash's
// front, and below its highest, damp for the waves come and gone and
// wetter the nearer the front (`water_sand_wet` on the terrain).
fn water_swash_wet(theta: f32, reach: f32, e: f32) -> f32 {
    // (the backwash's sheet keeps what it covers soaked)
    let s=fract(-theta/6.2831853);
    let u=clamp((s-0.3)/0.7,0.0,1.0);
    let run=max(water_swash_rise(theta).x,select(0.0,1.0-u*u,s>0.3))*reach;
    let below=1.0-smoothstep(0.85*reach,1.05*reach+0.01,e);
    return select(below*mix(0.55,1.0,exp(-max(e-run,0.0)/max(0.35*reach,0.02))),1.0,e<=run);
}
// How much of the sea a clearance leaves (Sea::clearance_cap): crests stay
// under 60 % of it (up to an underside, or the top of a floor at the water).
fn water_sea_cap(sea: WaterSea, clearance: f32) -> f32 {
    return min(0.6*max(clearance,0.0)/max(sea.wind.w,0.01),1.0);
}
// The whole sea at `p`, read there as `at` (Sea::local and Sea::open). The
// main swell, refracted to face the shore, grows as the water shallows
// (Green's law, up to twice), dims behind islands, is capped by the depth
// (a crest at most 0.28 d), leans forward and breaks by the beach's surf
// similarity, and in the last decimetres runs up the beach as swash. The
// wind chop goes on as plane waves, capped by the depth too and dimmed
// less behind islands; with a spectrum (sea.shore.w > 0) the chop is its
// wind sea instead, `ocean` read at `p`. `footprint` (m a pixel spans; 0 for heights) fades a
// train's slope where it is too short to resolve (far out, its crests
// aliased into stripes).
fn water_sea(p: vec2<f32>, time: f32, quality: u32, sea: WaterSea, at: WaterSeaAt, ocean: WaterOceanAt, footprint: f32) -> WaterSwell {
    let d=at.depth;let slope=length(at.slope);
    // the swell, or the wind sea's own wave where it does not reach
    let m=water_sea_main(sea,at);
    let surf=water_surf_similarity(slope,m.height,m.wavelength);
    let plunge=smoothstep(0.4,1.2,surf);let crash=smoothstep(2.5,4.0,surf);
    let dry=smoothstep(0.08,0.3,d);
    let grown=m.amp*clamp(pow(20.0/max(d,0.05),0.25),1.0,2.0);
    let cap=0.28*max(d,0.0);
    // (a wave a few centimetres high laps, it does not break white)
    let breaking=smoothstep(0.35+0.25*plunge,1.0,grown/max(cap,1e-3))*dry*smoothstep(0.03,0.15,grown);
    // (the spectrum's: the open sea's or a lake's chop, by the fetch here;
    // `ocean` is read with the same shares, water_fx_ocean)
    let weights=water_ocean_weights(at.chop);
    let rest=select(sea.trains[1].w+sea.trains[2].w+sea.trains[3].w,0.5*(sea.shore.w*weights.x+sea.lake.y*weights.y),sea.shore.w>0.0);
    let reach=water_swash_reach(sea,at);
    // the swash runs up a beach that faces the swell (none over the back
    // of a sandbar or a land bridge), its whole height up to the highest it
    // reaches, then no further (Sea::local)
    let beach=water_swash_beach(at);
    // (and a little way into the shallows: the swash rushes up from the
    // water, never lowering it under the swell's own crest there)
    // (by the real bed: on the smoothed depth the swash stopped short of
    // the sand, or ran over it, by tens of centimetres)
    let b=at.bed;
    let t=smoothstep(0.3,0.9,b);let land=smoothstep(-reach-0.25,-reach,b);
    let swash=beach*land*(1.0-t);
    // shelter holds the swell back; the wind still ruffles a harbour, so
    // its chop keeps more of itself (Sea::local)
    // (under a deck or a floor the clearance caps all of it)
    let shelter=clamp(at.shelter,0.0,1.0);let held=clamp(at.cap,0.0,1.0);
    // against what stands steep (rocks, a cliff, a wall) the swell is
    // thrown back on itself and stands up the face instead of shoaling out:
    // higher, and less held down by the depth (Sea::local)
    let wall=crash*(1.0-smoothstep(0.5,6.0,d));
    let main=min(grown*(1.0+0.7*wall),cap*(1.0+1.5*wall))*dry*shelter*held;
    // (and the chop dies out over the last two metres of the real bed, and
    // the swell's sway over the last metre and a half: Sea::local's `chop`
    // and `slide`)
    // (the trains' chop takes the fetch's share in; the spectrum's has it
    // in its cascades' shares)
    // (and gives way to the wind sea's own wave as it lines up in the
    // shallows, `take`)
    let left=min(cap/max(rest,0.01),1.0)*dry*sqrt(shelter)*held*smoothstep(0.1,2.0,b)*(1.0-m.take);
    let chop=at.chop*left;
    let slide=smoothstep(-0.2,1.5,b);
    let lean=0.75*breaking*(0.4+0.6*plunge);
    let since=time-sea.frame.z;
    // the main wave
    let k=max(length(at.wave),1e-4);let dir=at.wave/k;
    let theta=at.phase-(m.phase+m.omega*since);
    let x=theta-lean*sin(theta);
    let hz=main*sin(x);
    let rise=water_swash_rise(theta+water_swash_shift(at));
    let run=rise.x*reach;
    let over=run>hz;
    var offset=vec3<f32>(dir*cos(theta)*main*0.85*(1.0-swash)*slide,hz+max(run-hz,0.0)*swash);
    let att0=1.0-smoothstep(0.5,2.0,k*footprint);
    // (the swash's share changes with the depth: its slope too)
    let dt=select(0.0,6.0*((b-0.3)/0.6)*(1.0-(b-0.3)/0.6)/0.6,b>0.3 && b<0.9);
    let ul=(b+reach+0.25)/0.25;
    let dl=select(0.0,6.0*ul*(1.0-ul)/0.25,b>-reach-0.25 && b<-reach);
    let dswash=beach*(dl*(1.0-t)-land*dt);
    // (timed from the shoreline, the swash's phase runs along the shore
    // only: across the beach it is one sheet)
    let down=at.slope/max(slope,1e-4);
    let along=at.wave-dot(at.wave,down)*down;
    let swell_slope=main*cos(x)*(1.0-lean*cos(theta))*at.wave*att0;
    var gradient=swell_slope;
    if(over){gradient=swell_slope*(1.0-swash)+reach*rise.y*swash*along*att0+(run-hz)*dswash*at.slope;}
    // (the folding that whitens the crests is filtered like the slope: from
    // far up a pixel spans several crests, and unfiltered they drew combs
    // of foam stripes)
    var jacobian=vec3<f32>(1.0,0.0,1.0)-main*0.85*(1.0-swash)*slide*sin(theta)*att0*vec3<f32>(at.wave.x*dir.x,at.wave.x*dir.y,at.wave.y*dir.y);
    // (the swell's alone, for the whitecaps: `breakers` below)
    let swell_j=jacobian;
    // the wind chop: the spectrum's, or the trains'
    let local=p-sea.frame.xy;
    var linger=0.0;
    var scaled=0.0;
    if(sea.shore.w>0.0){
        scaled=left*sea.look.y;
        linger=ocean.foam*scaled;
        offset+=vec3<f32>(ocean.offset.xy*scaled*water_chop_sway(ocean.jacobian*scaled),ocean.offset.z*scaled);
        gradient+=ocean.slope*scaled;
        jacobian+=ocean.jacobian*scaled;
    }
    for(var i=1u;i<select(min(quality+2u,4u),1u,sea.shore.w>0.0);i++) {
        let train=sea.trains[i];let td=train.xy;let tk=train.z;let a=train.w*chop;
        let phase=dot(local,td)*tk-(sea.phase[i]+sea.omega[i]*since)+f32(i)*1.73;
        let s=sin(phase);let c=cos(phase);
        offset+=vec3<f32>(td*c*a*0.85,s*a);
        let att=1.0-smoothstep(0.5,2.0,tk*footprint);
        gradient+=td*c*a*tk*att;
        jacobian-=a*tk*0.85*s*att*vec3<f32>(td.x*td.x,td.x*td.y,td.y*td.y);
    }
    let determinant=max(jacobian.x*jacobian.z-jacobian.y*jacobian.y,0.25);
    var out:WaterSwell;
    out.offset=offset;
    out.gradient=vec2<f32>(jacobian.z*gradient.x-jacobian.y*gradient.y,jacobian.x*gradient.y-jacobian.y*gradient.x)/determinant;
    out.compression=1.0-determinant;
    // What breaks white out at sea is the wind's chop, not the swell: a
    // swell, long and low, never breaks in deep water (only in the shallows,
    // `breaking` above). It squeezes the chop riding its crests, which then
    // breaks sooner there. So the whitecaps' fold is the chop's own, a
    // little more on the swell's crests; a swell with no chop under a calm
    // carries no foam (by the whole sea's fold a big swell's crests passed
    // the whitecaps' test alone and carried sheets of foam).
    let swell_fold=1.0-max(swell_j.x*swell_j.z-swell_j.y*swell_j.y,0.25);
    let chop_j=jacobian-swell_j+vec3<f32>(1.0,0.0,1.0);
    let chop_fold=1.0-max(chop_j.x*chop_j.z-chop_j.y*chop_j.y,0.25);
    out.breakers=chop_fold+max(chop_fold,0.0)*4.0*clamp(swell_fold,0.0,1.0);
    out.linger=linger;
    out.chop=scaled;
    // the breakers whiten; the swash's foam is its own lace (env's
    // water_swash_foam), as far as it really runs up: a calm lake's
    // centimetre ripples draw none (by the beach's shape alone they drew
    // thin bands sweeping in at the ripples' quick period)
    out.breaking=breaking*at.shelter*held;
    out.swash=swash*smoothstep(0.03,0.2,reach)*at.shelter*held;
    out.crest=mix(sin(x),rise.x*2.0-1.0,swash);
    out.surf=surf;
    return out;
}
// Ripples on a 16 m tile (integer wave vectors: the caustics map tiles
// without a seam). A few waves alone interfere into a lattice, which the sun
// glint and the caustics show as a grid: sixteen, spread over the
// directions (from -74 to +116 degrees of the region's east) and
// over wavelengths from 2.2 m to 0.45 m, largest first (but for the third
// and fourth, swapped: reordering them would move every quality's phases),
// so that each quality keeps the waves that matter most (water_micro_count).
const WATER_MICRO_MODES=array<vec2<f32>,16>(vec2<f32>(2.0,-7.0),vec2<f32>(3.0,7.0),vec2<f32>(10.0,1.0),vec2<f32>(5.0,-8.0),
    vec2<f32>(2.0,11.0),vec2<f32>(11.0,3.0),vec2<f32>(9.0,-10.0),vec2<f32>(-1.0,15.0),
    vec2<f32>(15.0,8.0),vec2<f32>(15.0,-11.0),vec2<f32>(-5.0,21.0),vec2<f32>(17.0,15.0),
    vec2<f32>(24.0,-10.0),vec2<f32>(-13.0,27.0),vec2<f32>(19.0,26.0),vec2<f32>(35.0,-6.0));
const WATER_MICRO_AMPLITUDES=array<f32,16>(0.01543,0.01459,0.01031,0.01116,0.00902,0.00880,0.00716,0.00624,0.00534,0.00478,0.00397,0.00373,0.00314,0.00264,0.00240,0.00213);
// 6, 8, 12 and 16 waves for the qualities low to ultra
fn water_micro_count(quality:u32)->u32 {
    let q=min(quality,3u);
    return select(12u+4u*(q-2u),6u+2u*q,q<2u);
}
// xy slope, z height. The same normals bend the rays in the caustics map.
fn water_micro(p: vec2<f32>, time: f32, quality: u32, footprint: f32) -> vec3<f32> {
    var wave=vec3<f32>(0.0);
    for(var i=0u;i<water_micro_count(quality);i++) {
        let k=WATER_MICRO_MODES[i]*0.3926990817;let frequency=length(k);
        let phase=dot(k,p)-time*sqrt(9.81*frequency)+f32(i)*2.31;
        let attenuation=1.0-smoothstep(0.6,2.5,frequency*footprint);
        wave+=vec3<f32>(k*cos(phase),sin(phase))*WATER_MICRO_AMPLITUDES[i]*attenuation;
    }
    return wave;
}

// xx, xy, yy second derivatives of the same surface, for its optical Jacobian.
fn water_micro_curvature(p:vec2<f32>,time:f32,quality:u32)->vec3<f32> {
    var curvature=vec3<f32>(0.0);
    for(var i=0u;i<water_micro_count(quality);i++) {
        let k=WATER_MICRO_MODES[i]*0.3926990817;
        let phase=dot(k,p)-time*sqrt(9.81*length(k))+f32(i)*2.31;
        curvature-=vec3<f32>(k.x*k.x,k.x*k.y,k.y*k.y)*sin(phase)*WATER_MICRO_AMPLITUDES[i];
    }
    return curvature;
}
