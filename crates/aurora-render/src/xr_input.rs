//! Optional offline grip input, owned by the existing OpenXR session.
//! Action setup follows Khronos hello_xr's InitializeActions/PollActions.

use crate::{GripState, stereo_view::Anchor};
use openxr as xr;

pub(crate) fn enabled() -> bool {
    let offline = std::env::var("AURORA_DEMO").as_deref() == Ok("1");
    if offline {
        matches!(std::env::var("AURORA_DEMO_VR_ARMS").as_deref(), Ok("1" | "test" | "xr"))
    } else {
        std::env::var("AURORA_LIVE_VR_ARMS").as_deref() == Ok("1")
    }
}

pub(crate) struct Input {
    spaces: [xr::Space; 2],
    grip: xr::Action<xr::Posef>,
    calibrate: xr::Action<bool>,
    toggle: xr::Action<bool>,
    set: xr::ActionSet,
    paths: [xr::Path; 2],
    pressed: [bool; 2],
    armed: bool,
    pub buttons: [u64; 2],
}

impl Input {
    pub fn new<G>(instance: &xr::Instance, session: &xr::Session<G>) -> xr::Result<Self> {
        let paths = [
            instance.string_to_path("/user/hand/left")?,
            instance.string_to_path("/user/hand/right")?,
        ];
        let set = instance.create_action_set("aurora_demo_arms", "Experimental arm tracking", 0)?;
        let grip = set.create_action::<xr::Posef>("hand_grip", "Hand grip pose", &paths)?;
        let calibrate = set.create_action::<bool>("calibrate", "Calibrate neutral pose", &paths)?;
        let toggle = set.create_action::<bool>("tracking_toggle", "Enable or disable arm tracking", &paths)?;
        // Touch Plus is optional; older runtimes expose Quest controllers through Touch.
        // Rejected optional profiles must not prevent the existing VR view from opening.
        let mut bound = false;
        for profile in [
            "/interaction_profiles/oculus/touch_controller",
            "/interaction_profiles/meta/touch_controller_plus",
        ] {
            let bindings = [
                xr::Binding::new(&grip, instance.string_to_path("/user/hand/left/input/grip/pose")?),
                xr::Binding::new(&grip, instance.string_to_path("/user/hand/right/input/grip/pose")?),
                xr::Binding::new(&calibrate, instance.string_to_path("/user/hand/left/input/x/click")?),
                xr::Binding::new(&calibrate, instance.string_to_path("/user/hand/right/input/a/click")?),
                xr::Binding::new(&toggle, instance.string_to_path("/user/hand/left/input/y/click")?),
                xr::Binding::new(&toggle, instance.string_to_path("/user/hand/right/input/b/click")?),
            ];
            match instance.suggest_interaction_profile_bindings(instance.string_to_path(profile)?, &bindings) {
                Ok(()) => bound = true,
                Err(error) => log::info!("Experimental XR arms: profile {profile} unavailable: {error}"),
            }
        }
        if !bound {
            return Err(xr::sys::Result::ERROR_PATH_UNSUPPORTED);
        }
        let spaces = [
            grip.create_space(session, paths[0], xr::Posef::IDENTITY)?,
            grip.create_space(session, paths[1], xr::Posef::IDENTITY)?,
        ];
        session.attach_action_sets(&[&set])?;
        Ok(Self {
            spaces,
            grip,
            calibrate,
            toggle,
            set,
            paths,
            pressed: [false; 2],
            armed: false,
            buttons: [0; 2],
        })
    }

    pub fn sample<G>(
        &mut self,
        session: &xr::Session<G>,
        local: &xr::Space,
        time: xr::Time,
        anchor: &Anchor,
        focused: bool,
    ) -> [GripState; 2] {
        if !focused || session.sync_actions(&[(&self.set).into()]).is_err() {
            self.armed = false;
            return [GripState::Unavailable; 2];
        }
        let buttons = [&self.calibrate, &self.toggle].map(|action| {
            self.paths
                .iter()
                .any(|path| action.state(session, *path).is_ok_and(|s| s.is_active && s.current_state))
        });
        for (i, down) in buttons.into_iter().enumerate() {
            if self.armed && down && !self.pressed[i] {
                self.buttons[i] = self.buttons[i].wrapping_add(1);
            }
            self.pressed[i] = down;
        }
        self.armed = true;
        std::array::from_fn(|i| match self.grip.is_active(session, self.paths[i]) {
            Ok(false) => GripState::Inactive,
            Err(_) => GripState::Unavailable,
            Ok(true) => match self.spaces[i].locate(local, time) {
                Ok(location) => tracked_position(location.location_flags, location.pose.position)
                    .and_then(|p| anchor.tracking_position(p))
                    .map_or(GripState::Untracked, GripState::Tracked),
                Err(_) => GripState::Untracked,
            },
        })
    }
}

fn tracked_position(flags: xr::SpaceLocationFlags, p: xr::Vector3f) -> Option<glam::Vec3> {
    let p = glam::Vec3::new(p.x, p.y, p.z);
    (flags.contains(xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::POSITION_TRACKED) && p.is_finite()).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xr_arms_require_tracked_valid_finite_position_not_just_headset_presence() {
        let p = xr::Vector3f { x: 0.2, y: 1.2, z: -0.3 };
        let valid = xr::SpaceLocationFlags::POSITION_VALID;
        assert!(tracked_position(valid, p).is_none());
        assert!(tracked_position(xr::SpaceLocationFlags::POSITION_TRACKED, p).is_none());
        assert!(tracked_position(valid | xr::SpaceLocationFlags::POSITION_TRACKED, p).is_some());
        assert!(tracked_position(valid | xr::SpaceLocationFlags::POSITION_TRACKED, xr::Vector3f { x: f32::NAN, ..p }).is_none());
    }
}
