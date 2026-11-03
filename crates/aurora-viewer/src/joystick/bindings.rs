//! Shared bindings used by both controller preference pages and input sampling.

use super::{Device, GamepadStick, Settings};

const BUTTON_PATHS: [&str; 24] = [
    "joystick.jump_button",
    "joystick.rise_button",
    "joystick.fall_button",
    "joystick.flycam_button",
    "joystick.flycam_rise_button",
    "joystick.flycam_fall_button",
    "joystick.zoom_in_button",
    "joystick.zoom_out_button",
    "joystick.zoom_hold_button",
    "joystick.gamepad_flycam_slowdown_button",
    "joystick.gamepad_flycam_speedup_button",
    "joystick.gamepad_avatar_zoom_in_button",
    "joystick.gamepad_avatar_zoom_out_button",
    "joystick.gamepad_avatar_pan_up_button",
    "joystick.gamepad_avatar_pan_down_button",
    "joystick.gamepad_avatar_next_button",
    "joystick.gamepad_avatar_previous_button",
    "joystick.first_person.jump_button",
    "joystick.first_person.rise_button",
    "joystick.first_person.fall_button",
    "joystick.first_person.zoom_in_button",
    "joystick.first_person.zoom_out_button",
    "joystick.first_person.view_button",
    "joystick.gamepad_flycam_view_button",
];

impl Settings {
    /// New automatic camera defaults must not steal controls from an older
    /// customized avatar layout. Resolve only after native device metadata exists.
    pub fn reconcile_avatar_camera_defaults(&mut self, device: &Device) -> bool {
        // A previously enabled raw default may be a SpaceMouse setup. Upgrade
        // it only after the selected native device is confirmed to be a pad.
        let mut changed = device.is_gamepad && self.untouched_raw_layout();
        if changed {
            self.gamepad();
            if self.flycam_button == 0 {
                self.flycam_button = super::AUTO_SOUTH;
            }
            if self.jump_button == 1 {
                self.jump_button = super::AUTO_EAST;
            }
        }
        let bindings = self.resolved_buttons(Some(device));
        // Released raw defaults used axis 2 for lift. On standard pads that
        // can be a trigger already claimed by zoom/travel-speed controls.
        if device.is_gamepad
            && self.mapping[2] >= 0
            && device.gamepad_triggers.contains(&Some(self.mapping[2] as usize))
            && [
                self.gamepad_avatar_zoom_in_button,
                self.gamepad_avatar_zoom_out_button,
                self.gamepad_flycam_slowdown_button,
                self.gamepad_flycam_speedup_button,
            ]
            .into_iter()
            .map(|value| super::resolve_binding(device, value))
            .any(|value| super::button_axis(value).is_some_and(|(axis, _)| axis == self.mapping[2] as usize))
        {
            self.mapping[2] = -1;
            changed = true;
        }
        for index in 11..17 {
            let binding = bindings[index];
            if *self.button_slot(index) >= super::INHERIT_BUTTON || binding < 0 {
                continue;
            }
            let action_conflict = bindings[..4].contains(&binding);
            let axis_conflict = super::button_axis(binding).is_some_and(|(axis, _)| {
                [0, 1, 2, 4, 5]
                    .into_iter()
                    .any(|channel| self.resolved_axis(device, false, channel) == Some(axis))
            });
            if action_conflict || axis_conflict {
                *self.button_slot(index) = -1;
                changed = true;
            }
        }
        changed
    }

    fn button_slot(&mut self, index: usize) -> &mut i8 {
        match index {
            0 => &mut self.jump_button,
            1 => &mut self.rise_button,
            2 => &mut self.fall_button,
            3 => &mut self.flycam_button,
            4 => &mut self.flycam_rise_button,
            5 => &mut self.flycam_fall_button,
            6 => &mut self.zoom_in_button,
            7 => &mut self.zoom_out_button,
            8 => &mut self.zoom_hold_button,
            9 => &mut self.gamepad_flycam_slowdown_button,
            10 => &mut self.gamepad_flycam_speedup_button,
            11 => &mut self.gamepad_avatar_zoom_in_button,
            12 => &mut self.gamepad_avatar_zoom_out_button,
            13 => &mut self.gamepad_avatar_pan_up_button,
            14 => &mut self.gamepad_avatar_pan_down_button,
            15 => &mut self.gamepad_avatar_next_button,
            16 => &mut self.gamepad_avatar_previous_button,
            17 => &mut self.first_person.jump_button,
            18 => &mut self.first_person.rise_button,
            19 => &mut self.first_person.fall_button,
            20 => &mut self.first_person.zoom_in_button,
            21 => &mut self.first_person.zoom_out_button,
            22 => &mut self.first_person.view_button,
            _ => &mut self.gamepad_flycam_view_button,
        }
    }

    fn resolved_buttons(&self, device: Option<&Device>) -> [i8; 24] {
        let raw = [
            self.jump_button,
            self.rise_button,
            self.fall_button,
            self.flycam_button,
            self.flycam_rise_button,
            self.flycam_fall_button,
            self.zoom_in_button,
            self.zoom_out_button,
            self.zoom_hold_button,
            self.gamepad_flycam_slowdown_button,
            self.gamepad_flycam_speedup_button,
            self.gamepad_avatar_zoom_in_button,
            self.gamepad_avatar_zoom_out_button,
            self.gamepad_avatar_pan_up_button,
            self.gamepad_avatar_pan_down_button,
            self.gamepad_avatar_next_button,
            self.gamepad_avatar_previous_button,
            self.first_person.jump_button,
            self.first_person.rise_button,
            self.first_person.fall_button,
            self.first_person.zoom_in_button,
            self.first_person.zoom_out_button,
            self.first_person.view_button,
            self.gamepad_flycam_view_button,
        ];
        std::array::from_fn(|i| {
            let binding = match raw[i] {
                super::INHERIT_BUTTON if i == 4 => self.rise_button,
                super::INHERIT_BUTTON if i == 5 => self.fall_button,
                super::INHERIT_BUTTON if i == 22 || i == 23 => self.flycam_button,
                value => value,
            };
            device.map_or(binding, |device| super::resolve_binding(device, binding))
        })
    }

    pub fn bind_button(&mut self, path: &str, binding: i8, device: Option<&Device>) {
        let Some(index) = BUTTON_PATHS.iter().position(|p| *p == path) else {
            return;
        };
        let mut scope = self.button_scope(index);
        let inherited = (index == 1 && self.flycam_rise_button == super::INHERIT_BUTTON)
            || (index == 2 && self.flycam_fall_button == super::INHERIT_BUTTON);
        if inherited {
            scope |= 2;
        }
        let previous = self.resolved_buttons(device);
        if binding >= 0 {
            for (other, value) in previous.into_iter().enumerate() {
                let mode = self.button_scope(other);
                if other != index && !(inherited && other == index + 3) && scope & mode != 0 && value == binding {
                    if other == 3 && index >= 17 {
                        if scope & 4 != 0 {
                            self.first_person.view_button = -1;
                        }
                        if scope & 2 != 0 {
                            self.gamepad_flycam_view_button = -1;
                        }
                    } else {
                        *self.button_slot(other) = -1;
                    }
                }
            }
            if let Some((axis, _)) = super::button_axis(binding)
                && let Some(device) = device.filter(|d| d.is_gamepad)
            {
                if scope & 4 != 0 {
                    self.release_first_person_axis(device, axis, None);
                }
                for flycam in [false, true] {
                    if scope & if flycam { 2 } else { 1 } != 0 {
                        self.release_axis(device, flycam, axis, None);
                        for channel in [0, 1, 4, 5] {
                            if self.mapping[channel] == axis as i8 {
                                self.mapping[channel] = -1;
                            }
                        }
                    }
                }
            }
        }
        *self.button_slot(index) = binding;
    }

    pub fn view_button(&self, flycam: bool, first_person: bool) -> i8 {
        let value = if flycam {
            self.gamepad_flycam_view_button
        } else if first_person {
            self.first_person.view_button
        } else {
            self.flycam_button
        };
        if value == super::INHERIT_BUTTON {
            self.flycam_button
        } else {
            value
        }
    }

    fn button_scope(&self, index: usize) -> u8 {
        match index {
            3 => {
                1 | if self.first_person.view_button == super::INHERIT_BUTTON {
                    4
                } else {
                    0
                } | if self.gamepad_flycam_view_button == super::INHERIT_BUTTON {
                    2
                } else {
                    0
                }
            }
            4..=10 | 23 => 2,
            17..=22 => 4,
            _ => 1,
        }
    }

    fn release_first_person_axis(&mut self, device: &Device, axis: usize, except: Option<bool>) {
        for (camera, channels) in [(false, [0, 1]), (true, [4, 5])] {
            if except != Some(camera)
                && channels.into_iter().any(|channel| {
                    self.axis_with_sticks(device, channel, self.first_person.move_stick, self.first_person.camera_stick) == Some(axis)
                })
            {
                if camera {
                    self.first_person.camera_stick = GamepadStick::None;
                } else {
                    self.first_person.move_stick = GamepadStick::None;
                }
            }
        }
        if self.mapping[2] == axis as i8 {
            self.first_person.tuning.gain[2] = 0.0;
        }
    }

    pub fn set_first_person_stick(&mut self, device: Option<&Device>, camera: bool, value: GamepadStick) {
        if matches!(value, GamepadStick::Left | GamepadStick::Right) {
            if let Some(device) = device {
                let index = usize::from(value == GamepadStick::Right);
                for axis in device.gamepad_sticks[index].into_iter().flatten() {
                    self.release_first_person_axis(device, axis, Some(camera));
                    for (index, binding) in self.resolved_buttons(Some(device)).into_iter().enumerate() {
                        if self.button_scope(index) & 4 != 0 && super::button_axis(binding).is_some_and(|(bound, _)| bound == axis) {
                            // Materialize an inherited view binding before clearing it,
                            // so the avatar and flycam layouts remain independent.
                            if index == 3 {
                                self.first_person.view_button = -1;
                            } else {
                                *self.button_slot(index) = -1;
                            }
                        }
                    }
                }
            }
            let other = if camera {
                &mut self.first_person.move_stick
            } else {
                &mut self.first_person.camera_stick
            };
            if *other == value {
                *other = GamepadStick::None;
            }
        }
        if camera {
            self.first_person.camera_stick = value;
        } else {
            self.first_person.move_stick = value;
        }
    }

    fn release_axis_buttons(&mut self, device: &Device, flycam: bool, axis: usize) {
        for (index, binding) in self.resolved_buttons(Some(device)).into_iter().enumerate() {
            let active = self.button_scope(index) & if flycam { 2 } else { 1 } != 0;
            if active && super::button_axis(binding).is_some_and(|(bound, _)| bound == axis) {
                *self.button_slot(index) = -1;
            }
        }
    }
    pub fn axis_binding_modified(&self, defaults: &Self, channel: usize, device: Option<&Device>) -> bool {
        device.map_or_else(
            || self.mapping[channel] != defaults.mapping[channel],
            |device| {
                [false, true]
                    .into_iter()
                    .filter(|flycam| *flycam || !matches!(channel, 3 | 6))
                    .any(|flycam| self.resolved_axis(device, flycam, channel) != defaults.resolved_axis(device, flycam, channel))
            },
        ) || match channel {
            0 | 1 => [false, true]
                .into_iter()
                .any(|mode| self.stick(mode, false) != defaults.stick(mode, false)),
            4 | 5 => [false, true]
                .into_iter()
                .any(|mode| self.stick(mode, true) != defaults.stick(mode, true)),
            _ => false,
        }
    }

    pub fn reset_axis_binding(&mut self, defaults: &Self, channel: usize, device: Option<&Device>) {
        self.mapping[channel] = defaults.mapping[channel];
        if matches!(channel, 0 | 1 | 4 | 5) {
            for mode in [false, true] {
                self.set_stick(
                    device.filter(|d| d.is_gamepad),
                    mode,
                    channel >= 4,
                    defaults.stick(mode, channel >= 4),
                );
            }
        }
    }
    pub fn stick(&self, flycam: bool, camera: bool) -> GamepadStick {
        match (flycam, camera) {
            (false, false) => self.gamepad_move_stick,
            (false, true) => self.gamepad_camera_stick,
            (true, false) => self.gamepad_flycam_move_stick,
            (true, true) => self.gamepad_flycam_camera_stick,
        }
    }

    fn store_stick(&mut self, flycam: bool, camera: bool, value: GamepadStick) {
        match (flycam, camera) {
            (false, false) => self.gamepad_move_stick = value,
            (false, true) => self.gamepad_camera_stick = value,
            (true, false) => self.gamepad_flycam_move_stick = value,
            (true, true) => self.gamepad_flycam_camera_stick = value,
        }
    }

    pub fn resolved_axis(&self, device: &Device, flycam: bool, channel: usize) -> Option<usize> {
        self.axis_with_sticks(device, channel, self.stick(flycam, false), self.stick(flycam, true))
    }

    pub(super) fn axis_with_sticks(
        &self,
        device: &Device,
        channel: usize,
        move_stick: GamepadStick,
        camera_stick: GamepadStick,
    ) -> Option<usize> {
        let group = match channel {
            0 | 1 => Some((false, usize::from(channel == 0))),
            4 | 5 => Some((true, usize::from(channel == 4))),
            _ => None,
        };
        if device.is_gamepad
            && let Some((camera, component)) = group
        {
            let stick = if camera { camera_stick } else { move_stick };
            let index = match stick {
                GamepadStick::None => return None,
                GamepadStick::Left => Some(0),
                GamepadStick::Right => Some(1),
                GamepadStick::Custom => None,
            };
            if let Some(index) = index
                && let Some(axis) = device.gamepad_sticks[index][component]
            {
                return (axis < device.axis_count).then_some(axis);
            }
        }
        usize::try_from(self.mapping[channel]).ok().filter(|axis| *axis < device.axis_count)
    }

    fn release_axis(&mut self, device: &Device, flycam: bool, axis: usize, except: Option<bool>) {
        for (camera, channels) in [(false, [0, 1]), (true, [4, 5])] {
            if except != Some(camera) && channels.iter().any(|&i| self.resolved_axis(device, flycam, i) == Some(axis)) {
                self.store_stick(flycam, camera, GamepadStick::None);
            }
        }
        for channel in [2, 3, 6] {
            if (flycam || channel == 2) && self.mapping[channel] == axis as i8 {
                self.mapping[channel] = -1;
            }
        }
    }

    pub fn set_stick(&mut self, device: Option<&Device>, flycam: bool, camera: bool, value: GamepadStick) {
        if matches!(value, GamepadStick::Left | GamepadStick::Right) {
            if self.stick(flycam, !camera) == value {
                self.store_stick(flycam, !camera, GamepadStick::None);
            }
            if let Some(device) = device {
                let index = usize::from(value == GamepadStick::Right);
                for axis in device.gamepad_sticks[index].into_iter().flatten() {
                    self.release_axis(device, flycam, axis, Some(camera));
                    self.release_axis_buttons(device, flycam, axis);
                }
            }
        }
        self.store_stick(flycam, camera, value);
    }

    /// Raw-axis assignments are shared by the controller modes. Preserve the
    /// paired axis, then relinquish any gamepad action claiming that input.
    pub fn assign_axis(&mut self, device: Option<&Device>, channel: usize, axis: i8) {
        let group = match channel {
            0 | 1 => Some(false),
            4 | 5 => Some(true),
            _ => None,
        };
        if let Some(device) = device.filter(|d| d.is_gamepad) {
            if let Some(camera) = group {
                let pair = if camera { [4, 5] } else { [0, 1] };
                let other = pair.into_iter().find(|i| *i != channel).unwrap_or(channel);
                self.mapping[other] = self.resolved_axis(device, true, other).map_or(-1, |i| i as i8);
                for flycam in [false, true] {
                    self.store_stick(flycam, camera, GamepadStick::Custom);
                }
            }
            if axis >= 0 && !(channel == 6 && self.zoom_mode == super::ZoomMode::HoldAxis) {
                if let Some(camera) = group {
                    if camera {
                        self.first_person.camera_stick = GamepadStick::Custom;
                    } else {
                        self.first_person.move_stick = GamepadStick::Custom;
                    }
                }
                self.release_first_person_axis(device, axis as usize, group);
                for (index, binding) in self.resolved_buttons(Some(device)).into_iter().enumerate() {
                    if index >= 17
                        && self.button_scope(index) & 4 != 0
                        && super::button_axis(binding).is_some_and(|(bound, _)| bound == axis as usize)
                    {
                        *self.button_slot(index) = -1;
                    }
                }
                for flycam in [false, true] {
                    if flycam || !matches!(channel, 3 | 6) {
                        self.release_axis(device, flycam, axis as usize, group);
                        self.release_axis_buttons(device, flycam, axis as usize);
                    }
                }
                for i in 0..self.mapping.len() {
                    if i != channel && self.mapping[i] == axis {
                        self.mapping[i] = -1;
                    }
                }
            }
        }
        self.mapping[channel] = axis;
        if axis >= 0 && matches!(channel, 3 | 6) && self.flycam.gain[channel] == 0.0 {
            self.flycam.gain[channel] = 1.0f32.copysign(self.flycam.gain[channel]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::joystick::{Controller, GamepadButton, GamepadStyle, Motion, Snapshot};
    use std::time::Instant;

    #[test]
    fn joystick_native_button_order_does_not_turn_full_triggers_into_movement() {
        use crate::joystick::{GamepadButton as B, resolve_binding};
        let d = Device::gamepad(
            "native-order".into(),
            "Test pad".into(),
            6,
            [[Some(0), Some(1)], [Some(3), Some(4)]],
            [Some(2), Some(5)],
            GamepadStyle::Xbox,
            vec![
                B::RightBumper,
                B::LeftTrigger,
                B::South,
                B::RightTrigger,
                B::East,
                B::LeftBumper,
                B::DpadUp,
                B::DpadDown,
            ],
        );
        let defaults = Settings::default();
        assert_eq!(resolve_binding(&d, defaults.jump_button), 4);
        assert_eq!(resolve_binding(&d, defaults.flycam_button), 2);
        assert_eq!(resolve_binding(&d, defaults.flycam_fall_button), 7);
        assert_eq!(resolve_binding(&d, defaults.flycam_rise_button), 6);
        assert_eq!(resolve_binding(&d, defaults.rise_button), 0);
        assert_eq!(resolve_binding(&d, defaults.fall_button), 5);
        assert_eq!(&defaults.avatar.gain[4..6], &[4.0, 4.0]);
        assert_eq!(&defaults.flycam.gain[4..6], &[4.0, 4.0]);
        for (flycam, first) in [(false, false), (false, true), (true, false)] {
            for (axis, digital, direction) in [(2, 1, -1.0), (5, 3, 1.0)] {
                let mut settings = defaults.clone();
                // Simulate the old hidden vertical mapping independently of
                // the lift buttons. Camera/travel triggers own that axis now.
                settings.mapping[2] = 2;
                settings.avatar.gain[2] = 2.0;
                settings.flycam.gain[2] = 2.0;
                assert!(settings.reconcile_avatar_camera_defaults(&d));
                assert_eq!(settings.mapping[2], -1);
                let mut snap = Snapshot {
                    devices: vec![d.clone()],
                    updated: Some(Instant::now()),
                    ..Default::default()
                };
                let mut controller = Controller::default();
                controller.sample_view(&snap, &settings, true, flycam, first, 0.1);
                // Native pads may emit a digital trigger edge as well as
                // pressure. A full immediate pull must only drive zoom/speed.
                snap.devices[0].axes[axis] = 1.0;
                snap.devices[0].button(digital, true);
                let motion = controller.sample_view(&snap, &settings, true, flycam, first, 0.1);
                assert!(!motion.player_movement_input());
                assert!(!motion.jump && !motion.rise && !motion.fall);
                assert_eq!(motion.axes[2], 0.0);
                if !flycam {
                    assert_eq!(motion.avatar_zoom, direction);
                } else {
                    assert_ne!(motion.flycam_speed, Some(1.0));
                }
            }
        }
        for (flycam, down_button, up_button) in [(false, 5, 0), (true, 7, 6)] {
            let mut snap = Snapshot {
                devices: vec![d.clone()],
                updated: Some(Instant::now()),
                ..Default::default()
            };
            let mut controller = Controller::default();
            controller.sample_view(&snap, &defaults, true, flycam, false, 0.1);
            snap.devices[0].button(down_button, true);
            let down = controller.sample_view(&snap, &defaults, true, flycam, false, 0.1);
            assert!(down.fall && !down.rise);
            snap.devices[0].button(down_button, false);
            snap.devices[0].button(up_button, true);
            let up = controller.sample_view(&snap, &defaults, true, flycam, false, 0.1);
            assert!(up.rise && !up.fall);
        }
    }

    #[test]
    fn joystick_untouched_enabled_pad_layout_upgrades_without_touching_custom_raw_axes() {
        let mut settings = Settings::default();
        settings.spacemouse();
        settings.enabled = true;
        settings.jump_button = 1;
        settings.flycam_button = 0;
        assert!(settings.reconcile_avatar_camera_defaults(&device()));
        assert_eq!(settings.jump_button, super::super::AUTO_EAST);
        assert_eq!(settings.flycam_button, super::super::AUTO_SOUTH);
        assert_eq!(settings.mapping[2], -1);
        let mut custom = Settings::default();
        custom.spacemouse();
        custom.avatar.gain[5] = 0.7;
        let before = custom.avatar.clone();
        custom.reconcile_avatar_camera_defaults(&device());
        assert_eq!(custom.avatar, before);
    }

    #[test]
    fn joystick_first_person_uses_its_own_axes_buttons_and_pressure_without_pan_or_targets() {
        let mut s = Settings::default();
        s.first_person.tuning.feather = 0.0;
        s.first_person.tuning.gain[5] = -4.0;
        s.first_person.tuning.dead_zone[5] = 0.0;
        s.first_person.jump_button = 8;
        s.first_person.zoom_in_button = super::super::AUTO_RIGHT_TRIGGER;
        let mut snap = Snapshot {
            devices: vec![device()],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let mut controller = Controller::default();
        controller.sample_view(&snap, &s, true, false, true, 0.1);
        snap.devices[0].axes[0] = 0.5; // native right X
        snap.devices[0].axes[6] = 0.3; // native right trigger
        snap.devices[0].button(8, true);
        snap.devices[0].button(13, true); // D-pad up
        snap.devices[0].button(16, true); // D-pad right
        let motion = controller.sample_view(&snap, &s, true, false, true, 0.1);
        assert_eq!(motion.axes[5], 2.0);
        assert!(motion.jump && motion.jump_pressed);
        assert!((motion.avatar_zoom - 0.3).abs() < 0.001);
        assert_eq!(motion.avatar_pan, 0.0);
        assert_eq!(motion.avatar_cycle, 0);
        snap.devices[0].axes[6] = 0.9;
        assert!((controller.sample_view(&snap, &s, true, false, true, 0.1).avatar_zoom - 0.9).abs() < 0.001);
        // The same held inputs cannot replay when another mode takes ownership.
        let motion = controller.sample_view(&snap, &s, true, false, false, 0.1);
        assert_eq!(motion.axes[5], 0.0);
        assert_eq!(motion.avatar_zoom, 0.0);
        assert!(!motion.jump);
        let saved = serde_json::to_string(&s).expect("settings");
        let loaded: Settings = serde_json::from_str(&saved).expect("profile");
        assert_eq!(s.first_person, loaded.first_person);
    }

    #[test]
    fn joystick_first_person_mapping_conflicts_stay_in_their_mode() {
        let mut s = Settings::default();
        let d = device();
        s.bind_button("joystick.first_person.jump_button", 0, Some(&d));
        assert_eq!(s.first_person.view_button, -1);
        assert_eq!(super::super::resolve_binding(&d, s.flycam_button), 0);
        assert_eq!(super::super::resolve_binding(&d, s.view_button(true, false)), 0);
        s.set_first_person_stick(Some(&d), false, GamepadStick::Right);
        assert_eq!(s.first_person.camera_stick, GamepadStick::None);
        assert_eq!(s.gamepad_camera_stick, GamepadStick::Right);
        s.bind_button("joystick.first_person.zoom_in_button", super::super::BUTTONS as i8, Some(&d));
        assert_eq!(s.first_person.move_stick, GamepadStick::None);
        assert_eq!(s.gamepad_move_stick, GamepadStick::Left);
        s.bind_button("joystick.gamepad_flycam_view_button", 1, Some(&d));
        assert_eq!(s.view_button(true, false), 1);
        assert_eq!(super::super::resolve_binding(&d, s.jump_button), 1);
        assert_eq!(super::super::resolve_binding(&d, s.view_button(false, false)), 0);
    }

    #[test]
    fn joystick_view_mode_press_is_one_edge_across_all_three_modes() {
        let s = Settings::default();
        let mut snap = Snapshot {
            devices: vec![device()],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let mut c = Controller::default();
        c.sample_view(&snap, &s, true, false, false, 0.1);
        for (flycam, first) in [(false, false), (false, true), (true, false)] {
            snap.devices[0].button(0, true);
            assert!(c.sample_view(&snap, &s, true, flycam, first, 0.1).toggle_flycam);
            assert!(!c.sample_view(&snap, &s, true, !flycam, !first, 0.1).toggle_flycam);
            snap.devices[0].button(0, false);
            c.sample_view(&snap, &s, true, !flycam, !first, 0.1);
        }
    }

    fn device() -> Device {
        Device::gamepad(
            "test".into(),
            "Gamepad".into(),
            8,
            [[Some(2), Some(5)], [Some(0), Some(7)]],
            [Some(1), Some(6)],
            GamepadStyle::Xbox,
            GamepadButton::STANDARD.to_vec(),
        )
    }

    fn sample(settings: &Settings, flycam: bool, axis: usize, value: f32) -> Motion {
        let mut snapshot = Snapshot {
            devices: vec![device()],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let mut controller = Controller::default();
        controller.sample(&snapshot, settings, true, flycam, 0.1);
        snapshot.devices[0].axes[axis] = value;
        controller.sample(&snapshot, settings, true, flycam, 0.1)
    }

    #[test]
    fn joystick_bindings_roll_reclaims_a_stick_axis_and_gamepad_can_reclaim_it_back() {
        let mut settings = Settings::default();
        settings.flycam.feather = 0.0;
        let device = device();
        assert_eq!(settings.resolved_axis(&device, true, 5), Some(0));
        settings.assign_axis(Some(&device), 3, 0);
        assert_eq!(settings.gamepad_flycam_camera_stick, GamepadStick::None);
        assert_eq!(
            settings.gamepad_camera_stick,
            GamepadStick::Right,
            "avatar and flycam are separate modes"
        );
        let motion = sample(&settings, true, 0, 0.8);
        assert!(motion.axes[3].abs() > 0.1, "an explicitly captured roll axis has a usable gain");
        assert_eq!((motion.axes[4], motion.axes[5]), (0.0, 0.0));
        assert!(sample(&settings, false, 0, 0.8).axes[5].abs() > 0.1);
        settings.set_stick(Some(&device), true, true, GamepadStick::Right);
        assert_eq!(settings.mapping[3], -1);
        assert!(sample(&settings, true, 0, 0.8).axes[5].abs() > 0.1);
        assert_eq!(sample(&settings, true, 0, 0.8).axes[3], 0.0);
    }

    #[test]
    fn joystick_bindings_custom_axes_displace_conflicts_and_survive_reload() {
        let mut settings = Settings::default();
        let device = device();
        settings.assign_axis(Some(&device), 4, 5);
        for flycam in [false, true] {
            assert_eq!(settings.stick(flycam, true), GamepadStick::Custom);
            assert_eq!(settings.stick(flycam, false), GamepadStick::None);
            assert_eq!(settings.resolved_axis(&device, flycam, 4), Some(5));
            assert_eq!(settings.resolved_axis(&device, flycam, 5), Some(0));
        }
        let saved = serde_json::to_string(&settings).expect("save bindings");
        let restored: Settings = serde_json::from_str(&saved).expect("load bindings");
        assert_eq!(settings, restored);
        settings.set_stick(Some(&device), true, false, GamepadStick::Right);
        assert_eq!(settings.gamepad_flycam_camera_stick, GamepadStick::None);
        assert_eq!(settings.gamepad_camera_stick, GamepadStick::Custom);
    }

    #[test]
    fn joystick_bindings_trigger_and_button_actions_are_exclusive_within_each_mode() {
        let mut settings = Settings::default();
        let device = device();
        let trigger = super::super::gamepad_trigger_binding(&device, 1).expect("right trigger");
        settings.bind_button("joystick.zoom_in_button", trigger, Some(&device));
        assert_eq!(settings.gamepad_flycam_speedup_button, -1);
        settings.bind_button("joystick.jump_button", trigger, Some(&device));
        assert_eq!(settings.zoom_in_button, trigger, "different modes may reuse a physical control");
        assert!(
            sample(&settings, false, 6, 1.0).jump,
            "mapped analog triggers activate normal actions"
        );
        settings.bind_button("joystick.flycam_button", trigger, Some(&device));
        assert_eq!((settings.zoom_in_button, settings.jump_button), (-1, -1));
        settings.flycam_rise_button = super::super::INHERIT_BUTTON;
        settings.bind_button("joystick.rise_button", 9, Some(&device));
        assert_eq!(settings.flycam_rise_button, super::super::INHERIT_BUTTON);
        settings.bind_button("joystick.flycam_fall_button", 9, Some(&device));
        assert_eq!(settings.rise_button, 9, "flycam remapping preserves the avatar action");
        assert_eq!(settings.flycam_rise_button, -1);
    }

    #[test]
    fn joystick_bindings_auto_detection_is_visible_without_enabling_raw_hid() {
        let mut snapshot = Snapshot::default();
        let mut settings = Settings::default();
        assert!(!snapshot.input_enabled(&settings));
        snapshot.devices.push(device());
        assert!(snapshot.input_enabled(&settings));
        assert!(!settings.enabled, "gamepad hotplug does not enable raw HID enumeration");
        settings.automatic_gamepad = false;
        assert!(!snapshot.input_enabled(&settings));
    }

    #[test]
    fn joystick_bindings_resets_and_presets_reclaim_their_inputs() {
        let defaults = Settings::default();
        let mut settings = defaults.clone();
        let device = device();
        settings.assign_axis(Some(&device), 3, 0);
        assert!(settings.axis_binding_modified(&defaults, 5, Some(&device)));
        settings.reset_axis_binding(&defaults, 5, Some(&device));
        assert_eq!(settings.gamepad_flycam_camera_stick, GamepadStick::Right);
        assert_eq!(settings.mapping[3], -1, "restoring camera look unbinds conflicting roll");
        settings.spacemouse();
        assert_eq!(settings.gamepad_flycam_camera_stick, GamepadStick::Custom);
        assert_eq!(settings.resolved_axis(&device, true, 5), usize::try_from(settings.mapping[5]).ok());
        settings.gamepad();
        assert_eq!(settings.gamepad_flycam_camera_stick, GamepadStick::Right);
        assert_eq!(settings.resolved_axis(&device, true, 5), Some(0));
    }

    #[test]
    fn joystick_bindings_sensitivity_and_inversion_use_the_shared_gains() {
        let mut settings = Settings::default();
        for channel in [4, 5] {
            settings.avatar.gain[channel] = 1.5;
            settings.flycam.gain[channel] = 1.5;
        }
        settings.set_axis_inverted(5, true);
        assert_eq!(settings.flycam.gain[5], -1.5);
        assert_eq!(settings.avatar.gain[5], -1.5);
        settings.flycam.feather = 0.0;
        assert!((sample(&settings, true, 0, 0.8).axes[5] - 0.975).abs() < 1e-5);
        settings.flycam.gain[5] = -6.0;
        assert!((sample(&settings, true, 0, 0.8).axes[5] - 3.9).abs() < 1e-5);
        settings.set_axis_inverted(5, false);
        assert!(sample(&settings, true, 0, 0.8).axes[5] < 0.0);
        let saved = serde_json::to_string(&settings).expect("save tuning");
        let loaded: Settings = serde_json::from_str(&saved).expect("reload tuning");
        assert_eq!(loaded.avatar, settings.avatar);
        assert_eq!(loaded.flycam, settings.flycam);
    }

    #[test]
    fn joystick_bindings_speedup_is_limited_to_fifty_in_the_motion_path() {
        let settings = Settings {
            gamepad_flycam_speedup_speed: 100.0,
            ..Default::default()
        };
        let motion = sample(&settings, true, 6, 1.0);
        assert_eq!(motion.flycam_speed, Some(50.0 / settings.flycam_speed));
    }

    #[test]
    fn joystick_avatar_camera_uses_analog_trigger_pressure_and_releases_cleanly() {
        let settings = Settings::default();
        let mut snapshot = Snapshot {
            devices: vec![device()],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let mut controller = Controller::default();
        controller.sample(&snapshot, &settings, true, false, 0.1);
        for value in [0.1, 0.25, 0.5, 1.0, 0.0] {
            snapshot.devices[0].axes[6] = value;
            let motion = controller.sample(&snapshot, &settings, true, false, 0.1);
            assert!((motion.avatar_zoom - value).abs() < 1e-6);
            let mut movement = crate::agent::MoveInput::default();
            motion.apply_avatar(&mut movement);
            assert!(
                !movement.forward && !movement.back && !movement.up && !movement.down,
                "camera controls never move the avatar"
            );
        }
        snapshot.devices[0].axes[1] = 0.4;
        assert!((controller.sample(&snapshot, &settings, true, false, 0.1).avatar_zoom + 0.4).abs() < 1e-6);
        controller.sample(&snapshot, &settings, false, false, 0.1);
        assert_eq!(
            controller.sample(&snapshot, &settings, true, false, 0.1).avatar_zoom,
            0.0,
            "held pressure cannot replay after focus loss"
        );
        snapshot.devices[0].axes[1] = 0.0;
        controller.sample(&snapshot, &settings, true, true, 0.1);
        snapshot.devices[0].axes[6] = 1.0;
        let flycam = controller.sample(&snapshot, &settings, true, true, 0.1);
        assert_eq!(flycam.avatar_zoom, 0.0);
        assert!(flycam.flycam_speed.is_some_and(|speed| speed > 1.0));
    }

    #[test]
    fn joystick_avatar_camera_dpad_hat_axes_pan_and_cycle_without_repeating() {
        let settings = Settings::default();
        let mut device = Device::gamepad(
            "hat".into(),
            "Hat controller".into(),
            8,
            [[Some(0), Some(1)], [Some(2), Some(3)]],
            [Some(4), Some(5)],
            GamepadStyle::Generic,
            Vec::new(),
        );
        device.gamepad_dpad = [Some(6), Some(7)];
        assert_eq!(super::super::gamepad_dpad_binding(&device, 0), Some(47));
        let mut snapshot = Snapshot {
            devices: vec![device],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let mut controller = Controller::default();
        controller.sample(&snapshot, &settings, true, false, 0.1);
        snapshot.devices[0].axes[7] = -1.0;
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_pan, 1.0);
        snapshot.devices[0].axes[7] = 1.0;
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_pan, -1.0);
        snapshot.devices[0].axes[7] = 0.0;
        snapshot.devices[0].axes[6] = 1.0;
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_cycle, 1);
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_cycle, 0);
        snapshot.devices[0].axes[6] = -1.0;
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_cycle, -1);
    }

    #[test]
    fn joystick_avatar_camera_defaults_preserve_existing_custom_actions() {
        let mut settings = Settings::default();
        let device = device();
        settings.jump_button = super::super::gamepad_trigger_binding(&device, 1).expect("right trigger");
        settings.rise_button = super::super::gamepad_dpad_binding(&device, 0).expect("D-pad up");
        assert!(settings.reconcile_avatar_camera_defaults(&device));
        assert_eq!(settings.gamepad_avatar_zoom_in_button, -1);
        assert_eq!(settings.gamepad_avatar_pan_up_button, -1);
        assert_eq!(settings.gamepad_avatar_zoom_out_button, super::super::AUTO_LEFT_TRIGGER);
        assert_eq!(
            settings.gamepad_flycam_speedup_button,
            super::super::AUTO_RIGHT_TRIGGER,
            "separate modes reuse inputs"
        );
        assert!(!settings.reconcile_avatar_camera_defaults(&device), "normalization is idempotent");
    }
}
