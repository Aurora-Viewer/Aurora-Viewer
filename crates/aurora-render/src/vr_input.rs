//! OpenXR controller actions and the compact state shared with the viewer.
//!
//! The render worker owns OpenXR actions and spaces. It publishes only this
//! copyable snapshot, keeping OpenXR handles out of the application thread.

use glam::{Quat, Vec2, Vec3};
use std::time::Instant;

/// One tracked OpenXR controller. Positions and rotations use Aurora's
/// right-handed, Y-up render coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VrControllerState {
    pub active: bool,
    pub position: Vec3,
    pub orientation: Quat,
    pub trigger: f32,
    pub grip: f32,
    pub thumbstick: Vec2,
    pub primary: bool,
    pub secondary: bool,
    pub menu: bool,
    pub thumbstick_click: bool,
}

impl Default for VrControllerState {
    fn default() -> Self {
        Self {
            active: false,
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            trigger: 0.0,
            grip: 0.0,
            thumbstick: Vec2::ZERO,
            primary: false,
            secondary: false,
            menu: false,
            thumbstick_click: false,
        }
    }
}

/// The latest state of both OpenXR hands. A missing or inactive controller is
/// represented by a default `VrControllerState`, never a stale previous pose.
#[derive(Debug, Clone, Copy, Default)]
pub struct VrInputState {
    pub left: VrControllerState,
    pub right: VrControllerState,
    /// Horizontal and vertical world direction of the headset after Aurora's
    /// VR anchor has mapped the OpenXR local space into the scene.
    pub head_forward: Vec3,
    pub updated_at: Option<Instant>,
}

/// One of the two OpenXR hands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum VrSide {
    Left = 0,
    Right = 1,
}

impl VrInputState {
    pub fn hand(&self, side: VrSide) -> VrControllerState {
        match side {
            VrSide::Left => self.left,
            VrSide::Right => self.right,
        }
    }
}

/// What the viewer found along the pointing controller's ray, for the render
/// worker's controller overlay. The worker draws it at the controllers'
/// poses of the frame it renders, so the beam never trails the hand; the
/// length comes from the viewer's last trace along that hand's ray.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VrPointer {
    /// Draw the tracked controllers in the eyes.
    pub controllers: bool,
    /// The hand the beam leaves from; None draws no beam.
    pub hand: Option<VrSide>,
    /// Metres from the aim origin to the hit, or to the pointer's reach.
    pub length: f32,
    /// The beam ends on a surface (a hit marker is drawn there).
    pub hit: bool,
    /// That surface's object has a click action (the beam is tinted).
    pub actionable: bool,
    /// The box of the clickable object pointed at, outlined.
    pub outline: Option<VrBox>,
    /// The teleport arc being aimed.
    pub arc: Option<VrArc>,
}

/// Points along a teleport arc, the last one where it lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VrArc {
    pub points: [Vec3; VrArc::MAX],
    pub len: usize,
    /// It reaches somewhere to land.
    pub lands: bool,
}

impl VrArc {
    pub const MAX: usize = 24;
}

/// An oriented box in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VrBox {
    pub center: Vec3,
    pub rotation: Quat,
    pub half: Vec3,
}

/// Picture adjustments and comfort darkening applied to the eyes after the
/// world, in the display's encoding (VR-only preferences).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VrDisplay {
    /// Stops: the picture is multiplied by 2^exposure.
    pub exposure: f32,
    /// Added after contrast, -1..1.
    pub brightness: f32,
    /// Around mid grey; 1 leaves it.
    pub contrast: f32,
    /// 0 grey, 1 unchanged, 2 doubled.
    pub saturation: f32,
    /// Output = input^(1/gamma).
    pub gamma: f32,
    /// Comfort vignette, 0 none to 1 strongest: the edges of the view
    /// darken while moving or turning.
    pub vignette: f32,
}

impl Default for VrDisplay {
    fn default() -> Self {
        Self {
            exposure: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
            vignette: 0.0,
        }
    }
}

impl VrDisplay {
    /// Whether the picture is changed at all (the vignette apart).
    pub fn adjusts(&self) -> bool {
        let d = Self::default();
        (self.exposure - d.exposure).abs() > 1e-3
            || (self.brightness - d.brightness).abs() > 1e-3
            || (self.contrast - d.contrast).abs() > 1e-3
            || (self.saturation - d.saturation).abs() > 1e-3
            || (self.gamma - d.gamma).abs() > 1e-3
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
mod openxr_input {
    use super::{Quat, Vec2, Vec3, VrControllerState, VrInputState};
    use openxr as xr;
    use std::time::Instant;

    pub(crate) struct Actions {
        instance: xr::Instance,
        set: xr::ActionSet,
        aim_pose: xr::Action<xr::Posef>,
        trigger: xr::Action<f32>,
        grip: xr::Action<f32>,
        thumbstick: xr::Action<xr::Vector2f>,
        primary: xr::Action<bool>,
        secondary: xr::Action<bool>,
        menu: xr::Action<bool>,
        thumbstick_click: xr::Action<bool>,
        haptic: xr::Action<xr::Haptic>,
        aim_spaces: [xr::Space; 2],
        hands: [xr::Path; 2],
        /// A small live record makes an unfamiliar runtime's controller
        /// profile diagnosable without flooding the normal renderer log.
        next_diagnostic: std::sync::Mutex<Instant>,
    }

    impl Actions {
        pub(crate) fn new<G: xr::Graphics>(instance: &xr::Instance, session: &xr::Session<G>) -> xr::Result<Self> {
            let hands = [
                instance.string_to_path("/user/hand/left")?,
                instance.string_to_path("/user/hand/right")?,
            ];
            let set = instance.create_action_set("aurora_vr", "Aurora VR", 0)?;
            let aim_pose = set.create_action::<xr::Posef>("aim_pose", "Aim pose", &hands)?;
            let trigger = set.create_action::<f32>("trigger", "Trigger", &hands)?;
            let grip = set.create_action::<f32>("grip", "Grip", &hands)?;
            let thumbstick = set.create_action::<xr::Vector2f>("thumbstick", "Thumbstick", &hands)?;
            let primary = set.create_action::<bool>("primary", "Primary button", &hands)?;
            let secondary = set.create_action::<bool>("secondary", "Secondary button", &hands)?;
            let menu = set.create_action::<bool>("menu", "Menu button", &hands)?;
            let thumbstick_click = set.create_action::<bool>("thumbstick_click", "Thumbstick click", &hands)?;
            let haptic = set.create_action::<xr::Haptic>("haptic", "Vibration", &hands)?;

            let touch_bindings = |profile: &str| -> xr::Result<()> {
                let path = |suffix: &str| instance.string_to_path(suffix);
                instance.suggest_interaction_profile_bindings(
                    instance.string_to_path(profile)?,
                    &[
                        xr::Binding::new(&aim_pose, path("/user/hand/left/input/aim/pose")?),
                        xr::Binding::new(&aim_pose, path("/user/hand/right/input/aim/pose")?),
                        xr::Binding::new(&trigger, path("/user/hand/left/input/trigger/value")?),
                        xr::Binding::new(&trigger, path("/user/hand/right/input/trigger/value")?),
                        xr::Binding::new(&grip, path("/user/hand/left/input/squeeze/value")?),
                        xr::Binding::new(&grip, path("/user/hand/right/input/squeeze/value")?),
                        xr::Binding::new(&thumbstick, path("/user/hand/left/input/thumbstick")?),
                        xr::Binding::new(&thumbstick, path("/user/hand/right/input/thumbstick")?),
                        xr::Binding::new(&primary, path("/user/hand/left/input/x/click")?),
                        xr::Binding::new(&primary, path("/user/hand/right/input/a/click")?),
                        xr::Binding::new(&secondary, path("/user/hand/left/input/y/click")?),
                        xr::Binding::new(&secondary, path("/user/hand/right/input/b/click")?),
                        xr::Binding::new(&menu, path("/user/hand/left/input/menu/click")?),
                        xr::Binding::new(&thumbstick_click, path("/user/hand/left/input/thumbstick/click")?),
                        xr::Binding::new(&thumbstick_click, path("/user/hand/right/input/thumbstick/click")?),
                        xr::Binding::new(&haptic, path("/user/hand/left/output/haptic")?),
                        xr::Binding::new(&haptic, path("/user/hand/right/output/haptic")?),
                    ],
                )
            };
            // Quest Touch and Touch Plus runtimes commonly expose the Oculus
            // Touch profile. Do not put Touch-only paths in the simple profile:
            // OpenXR rejects suggested paths that profile does not define.
            touch_bindings("/interaction_profiles/oculus/touch_controller")?;
            // Quest 3's Touch Plus controllers can instead report the Meta
            // profile. It is optional because a 1.0 runtime without the Meta
            // extension must reject its profile path; the Oculus Touch
            // bindings above remain the compatibility fallback.
            if let Err(e) = touch_bindings("/interaction_profiles/meta/touch_controller_plus") {
                log::info!("OpenXR Touch Plus bindings unavailable: {e}");
            }
            instance.suggest_interaction_profile_bindings(
                instance.string_to_path("/interaction_profiles/khr/simple_controller")?,
                &[
                    xr::Binding::new(&aim_pose, instance.string_to_path("/user/hand/left/input/aim/pose")?),
                    xr::Binding::new(&aim_pose, instance.string_to_path("/user/hand/right/input/aim/pose")?),
                    xr::Binding::new(&primary, instance.string_to_path("/user/hand/left/input/select/click")?),
                    xr::Binding::new(&primary, instance.string_to_path("/user/hand/right/input/select/click")?),
                    xr::Binding::new(&haptic, instance.string_to_path("/user/hand/left/output/haptic")?),
                    xr::Binding::new(&haptic, instance.string_to_path("/user/hand/right/output/haptic")?),
                ],
            )?;
            session.attach_action_sets(&[&set])?;
            let aim_spaces = [
                aim_pose.create_space(session, hands[0], xr::Posef::IDENTITY)?,
                aim_pose.create_space(session, hands[1], xr::Posef::IDENTITY)?,
            ];
            Ok(Self {
                instance: instance.clone(),
                set,
                aim_pose,
                trigger,
                grip,
                thumbstick,
                primary,
                secondary,
                menu,
                thumbstick_click,
                haptic,
                aim_spaces,
                hands,
                next_diagnostic: std::sync::Mutex::new(Instant::now()),
            })
        }

        pub(crate) fn sync<G: xr::Graphics>(
            &self,
            session: &xr::Session<G>,
            local: &xr::Space,
            time: xr::Time,
        ) -> xr::Result<VrInputState> {
            session.sync_actions(&[(&self.set).into()])?;
            let state = VrInputState {
                left: self.controller(session, local, time, 0)?,
                right: self.controller(session, local, time, 1)?,
                head_forward: Vec3::ZERO,
                updated_at: Some(Instant::now()),
            };
            self.diagnose(session, state);
            Ok(state)
        }

        /// One vibration pulse on a hand (0 left, 1 right).
        pub(crate) fn pulse<G: xr::Graphics>(
            &self,
            session: &xr::Session<G>,
            hand: usize,
            amplitude: f32,
            seconds: f32,
        ) -> xr::Result<()> {
            let nanos = (seconds.clamp(0.0, 1.0) * 1e9) as i64;
            let event = xr::HapticVibration::new()
                .amplitude(amplitude.clamp(0.0, 1.0))
                .duration(xr::Duration::from_nanos(nanos.max(1)))
                .frequency(xr::FREQUENCY_UNSPECIFIED);
            self.haptic.apply_feedback(session, self.hands[hand], &event)
        }

        fn diagnose<G: xr::Graphics>(&self, session: &xr::Session<G>, state: VrInputState) {
            let now = Instant::now();
            let Ok(mut next) = self.next_diagnostic.lock() else {
                return;
            };
            if *next > now {
                return;
            }
            *next = now + std::time::Duration::from_secs(2);
            let profile = |hand| {
                session
                    .current_interaction_profile(hand)
                    .ok()
                    .and_then(|path| self.instance.path_to_string(path).ok())
                    .filter(|path| !path.is_empty())
                    .unwrap_or_else(|| "none".into())
            };
            log::info!(
                "OpenXR controllers: left profile={} active={} stick={:.2},{:.2}; right profile={} active={} stick={:.2},{:.2}",
                profile(self.hands[0]),
                state.left.active,
                state.left.thumbstick.x,
                state.left.thumbstick.y,
                profile(self.hands[1]),
                state.right.active,
                state.right.thumbstick.x,
                state.right.thumbstick.y,
            );
        }

        fn controller<G: xr::Graphics>(
            &self,
            session: &xr::Session<G>,
            local: &xr::Space,
            time: xr::Time,
            hand: usize,
        ) -> xr::Result<VrControllerState> {
            let path = self.hands[hand];
            let active = self.aim_pose.is_active(session, path)?;
            let location = self.aim_spaces[hand].locate(local, time)?;
            let valid = location
                .location_flags
                .contains(xr::SpaceLocationFlags::POSITION_VALID | xr::SpaceLocationFlags::ORIENTATION_VALID);
            let trigger = self.trigger.state(session, path)?;
            let grip = self.grip.state(session, path)?;
            let thumbstick = self.thumbstick.state(session, path)?;
            let primary = self.primary.state(session, path)?;
            let secondary = self.secondary.state(session, path)?;
            let menu = self.menu.state(session, path)?;
            let thumbstick_click = self.thumbstick_click.state(session, path)?;
            let pose = location.pose;
            Ok(VrControllerState {
                active: active && valid,
                position: Vec3::new(pose.position.x, pose.position.y, pose.position.z),
                orientation: Quat::from_xyzw(pose.orientation.x, pose.orientation.y, pose.orientation.z, pose.orientation.w),
                trigger: trigger.current_state.clamp(0.0, 1.0),
                grip: grip.current_state.clamp(0.0, 1.0),
                thumbstick: Vec2::new(thumbstick.current_state.x, thumbstick.current_state.y),
                primary: primary.current_state,
                secondary: secondary.current_state,
                menu: menu.current_state,
                thumbstick_click: thumbstick_click.current_state,
            })
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub(crate) use openxr_input::Actions;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_controller_is_inactive_and_neutral() {
        let controller = VrControllerState::default();
        assert!(!controller.active);
        assert_eq!(controller.position, Vec3::ZERO);
        assert_eq!(controller.thumbstick, Vec2::ZERO);
        assert!(!controller.primary && !controller.secondary && !controller.menu);
    }
}
