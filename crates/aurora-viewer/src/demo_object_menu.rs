//! Offline objects of the object menu scenario (AURORA_DEMO_OBJECT_MENU):
//! a scripted bell of ours with its touch and sit texts, a crate of ours
//! beside it (Link, Take multiple), and a cloud of particles another
//! resident owns (Block particle owner). They go through the same
//! ObjectUpdate, selection and task inventory paths as objects of the grid.
use super::*;

pub const BELL: u32 = 9400;
pub const CRATE: u32 = 9401;
pub const SPARKLES: u32 = 9402;

/// "Ours": move, copy, modify, owned, scripted, touch, transfer
/// (object_flags.h).
const OURS: u32 = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 8) | (1 << 17);

pub fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_OBJECT_MENU").ok()).as_deref()
}

/// The bell's touch and sit texts (its ObjectProperties).
pub fn texts(local_id: u32) -> Option<(&'static str, &'static str)> {
    (local_id == BELL).then(|| {
        (
            aurora_i18n::tr!("demo-object-menu-touch-text"),
            aurora_i18n::tr!("demo-object-menu-sit-text"),
        )
    })
}

/// The three objects in a row across `side` (a unit vector on the ground)
/// around `center` (region metres).
pub fn events(center: Vec3, side: Vec3) -> Vec<NetEvent> {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
    let at = |along: f32, h: f32| {
        let p = center + side * along;
        Vec3::new(p.x, p.y, floor_at(p.x, p.y) + h)
    };
    let mut bell = prim(
        BELL,
        at(2.4, 0.45),
        Quat::IDENTITY,
        Vec3::splat(0.9),
        boxp,
        te([0.545, 0.361, 0.965, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-object-menu-bell"),
    );
    bell.owner_id = DEMO_AGENT;
    bell.update_flags = OURS | (1 << 6) | (1 << 7);
    let mut crate_box = prim(
        CRATE,
        at(1.2, 0.35),
        Quat::from_rotation_z(0.4),
        Vec3::splat(0.7),
        boxp,
        te([0.369, 0.918, 0.831, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-object-menu-crate"),
    );
    crate_box.owner_id = DEMO_AGENT;
    crate_box.update_flags = OURS;
    let mut sparkles = prim(
        SPARKLES,
        at(-2.2, 1.2),
        Quat::IDENTITY,
        Vec3::splat(0.15),
        sphere,
        te([0.98, 0.75, 0.15, 1.0], 0, true, 0.2),
        ExtraParams::default(),
        "",
    );
    sparkles.owner_id = DEMO_NOVA;
    sparkles.particles = aurora_net::objects::ParticleUpdate::Set(still_particles());
    vec![NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![bell, crate_box, sparkles],
    }]
}

/// Large particles resting on their emitter (a DROP pattern without
/// speed): a target the right click lands on.
fn still_particles() -> Vec<u8> {
    use aurora_prim::particles::*;
    let ufix8 = |v: f32, frac: u32| (v * (1u32 << frac) as f32) as u8;
    let ufix16 = |v: f32, frac: u32| ((v * (1u32 << frac) as f32) as u16).to_le_bytes();
    let sfix16 = |v: f32| (((v + 256.0) * 128.0) as u16).to_le_bytes();
    let mut out = Vec::with_capacity(PS_LEGACY_DATA_BLOCK_SIZE);
    out.extend_from_slice(&0xA0E0_0992u32.to_le_bytes()); // crc
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(LL_PART_SRC_PATTERN_DROP);
    out.extend_from_slice(&ufix16(0.0, 8)); // max age (forever)
    out.extend_from_slice(&ufix16(0.0, 8)); // start age
    out.push(0); // inner angle
    out.push(0); // outer angle
    out.extend_from_slice(&ufix16(0.3, 8)); // burst rate
    out.extend_from_slice(&ufix16(0.0, 8)); // burst radius
    out.extend_from_slice(&ufix16(0.0, 8)); // speed min
    out.extend_from_slice(&ufix16(0.0, 8)); // speed max
    out.push(1); // particles per burst
    for _ in 0..6 {
        out.extend_from_slice(&sfix16(0.0)); // angular velocity, acceleration
    }
    out.extend_from_slice(Uuid::nil().as_bytes()); // texture
    out.extend_from_slice(Uuid::nil().as_bytes()); // target
    let flags = LL_PART_INTERP_COLOR_MASK | LL_PART_EMISSIVE_MASK;
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&ufix16(3.0, 8)); // particle max age
    out.extend_from_slice(&[250, 190, 40, 200]); // start color (amber)
    out.extend_from_slice(&[139, 92, 246, 60]); // end color (violet)
    for _ in 0..4 {
        out.push(ufix8(1.4, 5)); // start and end scale
    }
    out
}
