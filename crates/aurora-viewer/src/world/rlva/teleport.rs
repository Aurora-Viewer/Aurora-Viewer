//! RlvHandler::onTeleportCallback and LLAgent::doTeleportViaLocation/LookAt
//! (rlvhandler.cpp, llagent.cpp, LGPL 2.1). Named offsets become global before
//! choosing the current variable-size region or the remote 256-metre tile.
use aurora_net::{NetCommand, RegionHandle, handle_to_origin, origin_to_handle};
use glam::{DVec3, Vec3};

fn destination(
    handle: RegionHandle,
    position: Vec3,
    look_at: Vec3,
    current: Option<(RegionHandle, u32, u32)>,
) -> Option<(RegionHandle, Vec3, Vec3)> {
    let (x, y) = handle_to_origin(handle);
    let global = DVec3::new(f64::from(x), f64::from(y), 0.0) + position.as_dvec3();
    if !global.is_finite()
        || !look_at.is_finite()
        || global.x < 0.0
        || global.y < 0.0
        || global.x >= f64::from(u32::MAX)
        || global.y >= f64::from(u32::MAX)
    {
        return None;
    }
    let current = current
        .filter(|(handle, width, height)| {
            let (x, y) = handle_to_origin(*handle);
            global.x >= f64::from(x)
                && global.y >= f64::from(y)
                && global.x < f64::from(x) + f64::from(*width)
                && global.y < f64::from(y) + f64::from(*height)
        })
        .map(|(handle, _, _)| handle);
    let handle = current.unwrap_or_else(|| origin_to_handle((global.x as u32 / 256) * 256, (global.y as u32 / 256) * 256));
    let (x, y) = handle_to_origin(handle);
    let position = Vec3::new((global.x - f64::from(x)) as f32, (global.y - f64::from(y)) as f32, global.z as f32);
    // A zero vector is the parser's no-angle marker. Explicit angle zero
    // remains the unit X vector, as the native look-at overload requires.
    let look_at = if look_at == Vec3::ZERO { position + Vec3::X } else { look_at };
    Some((handle, position, look_at))
}

impl crate::world::World {
    pub(crate) fn rlva_normalize_teleport(&self, command: &mut NetCommand) -> bool {
        let NetCommand::TeleportTo { handle, position, look_at } = command else {
            return true;
        };
        let current = self.main_region.and_then(|handle| {
            self.regions
                .get(&handle)
                .map(|region| (handle, region.heightmap.size_x, region.heightmap.size_y))
        });
        let Some((resolved, local, facing)) = destination(*handle, *position, *look_at, current) else {
            return false;
        };
        *handle = resolved;
        *position = local;
        *look_at = facing;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rlva_teleport_offsets_normalize_before_sending_and_no_angle_keeps_the_native_look_field() {
        let base = origin_to_handle(1024, 2048);
        let (handle, position, look) = destination(base, Vec3::new(-0.5, 257.25, 30.0), Vec3::ZERO, None).unwrap();
        assert_eq!(handle_to_origin(handle), (768, 2304));
        assert_eq!(position, Vec3::new(255.5, 1.25, 30.0));
        assert_eq!(look, position + Vec3::X);
        assert_eq!(destination(base, Vec3::new(-0.5, 257.25, 30.0), Vec3::X, None).unwrap().2, Vec3::X);
        assert!(destination(origin_to_handle(0, 0), Vec3::NEG_X, Vec3::ZERO, None).is_none());
        assert!(destination(base, Vec3::splat(f32::NAN), Vec3::ZERO, None).is_none());
        assert_eq!(
            destination(handle, position, look, None),
            Some((handle, position, look)),
            "normalization is idempotent at the second send guard"
        );
    }

    #[test]
    fn rlva_teleports_preserve_current_large_region_coordinates_and_quantize_remote_regions() {
        let base = origin_to_handle(1024, 2048);
        let tile = origin_to_handle(1280, 2304);
        let (handle, position, _) = destination(tile, Vec3::new(10.125, 20.5, 42.0), Vec3::Y, Some((base, 1024, 512))).unwrap();
        assert_eq!(handle, base);
        assert_eq!(position, Vec3::new(266.125, 276.5, 42.0));
        let remote = origin_to_handle(4096, 4096);
        let (handle, position, _) = destination(remote, Vec3::new(513.125, 700.5, 42.0), Vec3::Y, Some((base, 1024, 512))).unwrap();
        assert_eq!(handle_to_origin(handle), (4608, 4608));
        assert_eq!(position, Vec3::new(1.125, 188.5, 42.0));
    }
}
