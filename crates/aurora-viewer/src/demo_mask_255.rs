//! AURORA_DEMO_MASK_255=<alpha>: a texture whose alpha is <alpha> (250 to
//! 254, 250 by default) on every texel, on two panels: left with no
//! material, shown whole, and right with a Mask material of cutoff 255,
//! which must hide it (#2276). With 255 the texture has no alpha channel
//! and the Mask shows it whole. The texture is a real `.j2c` in the disk
//! cache, so that it goes through the decode job and its BC7 encoding: the
//! demo's procedural textures do not.

use super::{HANDLE, MAT_MASK_255, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

const TEXTURE: Uuid = Uuid::from_u128(0xde7e_f2e5_0000_4000_8000_0000_0000_0255);
const LOCAL_ID: u32 = 81_100;
const SIDE: u32 = 256;
/// The panels arrive once the loading screen is gone.
const ARRIVE_FRAME: u64 = 300;

pub struct Mask255Demo {
    alpha: u8,
    armed: bool,
    sent: bool,
}

impl Mask255Demo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_MASK_255").ok()?;
        Some(Mask255Demo {
            alpha: parse(&value),
            armed: false,
            sent: false,
        })
    }

    /// Once a frame.
    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World, cache_dir: &Path) {
        if !self.armed {
            self.armed = true;
            write_texture(self.alpha, cache_dir);
        }
        if !self.sent && frame >= ARRIVE_FRAME {
            self.sent = true;
            world.apply(panels(world.agent.position));
            log::info!("demo mask 255: alpha {} on both panels, the right one masked", self.alpha);
        }
    }
}

/// The alpha asked for, 255 for no alpha channel; a value out of range is
/// the default.
fn parse(value: &str) -> u8 {
    value.trim().parse().ok().filter(|a| (250..=255).contains(a)).unwrap_or(250)
}

/// The texture's `.j2c` in the disk cache, without its `.bc7` of a previous
/// run: that one would be read instead of the decode to test.
fn write_texture(alpha: u8, cache_dir: &Path) {
    // (waves of colour: encoded without alpha they come out with about half
    // their texels at 255, like a real picture; a flat colour or a plain
    // gradient lands on 254 everywhere and would hide the panel unfixed too)
    let picture: Vec<u8> = (0..SIDE * SIDE)
        .flat_map(|i| {
            let wave = ((i % SIDE) as f32 / 9.0).sin() * ((i / SIDE) as f32 / 7.0).cos();
            [
                (128.0 + wave * 100.0) as u8,
                (128.0 - wave * 90.0) as u8,
                (90.0 + wave * 60.0) as u8,
                alpha,
            ]
        })
        .collect();
    let (components, picture) = if alpha == 255 {
        (3, picture.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect())
    } else {
        (4, picture)
    };
    let file = match aurora_assets::j2k::encode_texture_j2k(SIDE, SIDE, components, &picture, true) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("demo mask 255: {e}");
            return;
        }
    };
    let tex = cache_dir.join("tex");
    let _ = std::fs::remove_file(tex.join(format!("{TEXTURE}.bc7")));
    if let Err(e) = std::fs::create_dir_all(&tex).and_then(|()| std::fs::write(tex.join(format!("{TEXTURE}.j2c")), &file)) {
        log::warn!("demo mask 255: file not written ({e})");
    }
}

/// Two thin square panels ahead of the avatar, wherever it starts, facing
/// the start look-at of AgentMovementComplete, (0.8, 0.4).
fn panels(avatar: Vec3) -> NetEvent {
    let look = Vec3::new(0.8, 0.4, 0.0).normalize();
    let across = Vec3::new(-look.y, look.x, 0.0);
    let panel = |index: u32, side: f32, material: Uuid| {
        let at = avatar + look * 6.0 + across * side;
        let mut t = (*te([1.0; 4], 0, true, 0.0)).clone();
        for f in &mut t.faces {
            f.texture = TEXTURE;
            f.material_id = material;
        }
        prim(
            LOCAL_ID + index,
            Vec3::new(at.x, at.y, avatar.z.max(height(at.x, at.y) + 0.6)),
            Quat::from_rotation_z(look.y.atan2(look.x)),
            Vec3::new(0.05, 1.2, 1.2),
            shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
            Arc::new(t),
            ExtraParams::default(),
            "",
        )
    };
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![panel(0, 0.8, Uuid::nil()), panel(1, -0.8, MAT_MASK_255)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alpha_is_just_under_opaque_or_opaque() {
        assert_eq!(parse("254"), 254);
        assert_eq!(parse(" 251 "), 251);
        assert_eq!(parse("255"), 255);
        for other in ["1", "", "256", "249", "many"] {
            assert_eq!(parse(other), 250, "{other}");
        }
    }
}
