//! Gamepad bindings shared by preferences and input sampling.

use super::{Device, GamepadStick, Settings};

const BUTTON_PATHS: [&str; 29] = [
    "jump_button",
    "rise_button",
    "fall_button",
    "flycam_button",
    "flycam_rise_button",
    "flycam_fall_button",
    "zoom_in_button",
    "zoom_out_button",
    "zoom_hold_button",
    "gamepad_flycam_slowdown_button",
    "gamepad_flycam_speedup_button",
    "gamepad_avatar_zoom_in_button",
    "gamepad_avatar_zoom_out_button",
    "gamepad_avatar_pan_up_button",
    "gamepad_avatar_pan_down_button",
    "gamepad_avatar_next_button",
    "gamepad_avatar_previous_button",
    "first_person.jump_button",
    "first_person.rise_button",
    "first_person.fall_button",
    "first_person.zoom_in_button",
    "first_person.zoom_out_button",
    "first_person.view_button",
    "gamepad_flycam_view_button",
    "crouch_button",
    "first_person.crouch_button",
    "reset_view_button",
    "gamepad_avatar_pan_left_button",
    "gamepad_avatar_pan_right_button",
];

impl Settings {
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
            23 => &mut self.gamepad_flycam_view_button,
            24 => &mut self.crouch_button,
            25 => &mut self.first_person.crouch_button,
            26 => &mut self.reset_view_button,
            27 => &mut self.gamepad_avatar_pan_left_button,
            _ => &mut self.gamepad_avatar_pan_right_button,
        }
    }

    fn resolved_buttons(&self, device: Option<&Device>) -> [i8; 29] {
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
            self.crouch_button,
            self.first_person.crouch_button,
            self.reset_view_button,
            self.gamepad_avatar_pan_left_button,
            self.gamepad_avatar_pan_right_button,
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
        let path = path.split_once('.').map_or(
            path,
            |(profile, field)| {
                if matches!(profile, "joystick" | "gamepad") { field } else { path }
            },
        );
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
                        if scope & 1 != 0 {
                            self.flycam_button = -1;
                        }
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
            17..=22 | 25 => 4,
            26 => 7,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::joystick::{Controller, GamepadButton, GamepadStyle, Motion, Snapshot};
    use std::time::Instant;

    fn gamepad_settings() -> Settings {
        Settings {
            enabled: true,
            ..Settings::gamepad_defaults()
        }
    }

    #[test]
    fn gamepad_shoulders_cycle_targets_without_flight_input_and_pan_uses_dpad() {
        let settings = gamepad_settings();
        let mut device = device();
        let mut controller = Controller::default();
        controller.sample_device(Some(&device), &settings, false, false, 0.1);
        for (binding, direction) in [(super::super::AUTO_RIGHT_BUMPER, 1), (super::super::AUTO_LEFT_BUMPER, -1)] {
            let index = super::super::resolve_binding(&device, binding) as usize;
            device.button(index, true);
            let motion = controller.sample_device(Some(&device), &settings, false, false, 0.1);
            assert_eq!(motion.avatar_cycle, direction);
            assert!(!motion.player_movement_input());
            assert_eq!(
                controller.sample_device(Some(&device), &settings, false, false, 0.1).avatar_cycle,
                0
            );
            device.button(index, false);
            controller.sample_device(Some(&device), &settings, false, false, 0.1);
        }
        let index = super::super::resolve_binding(&device, super::super::AUTO_DPAD_LEFT) as usize;
        device.button(index, true);
        let motion = controller.sample_device(Some(&device), &settings, false, false, 0.1);
        assert_eq!(motion.avatar_pan_side, 1.0);
        assert_eq!(motion.avatar_cycle, 0);
    }

    #[test]
    fn gamepad_layout_migrates_old_default_controls_once_and_preserves_custom_bindings() {
        use super::super::*;
        let mut old = Settings {
            gamepad_layout_version: 0,
            gamepad_avatar_pan_left_button: -1,
            gamepad_avatar_pan_right_button: -1,
            gamepad_avatar_next_button: AUTO_DPAD_RIGHT,
            gamepad_avatar_previous_button: AUTO_DPAD_LEFT,
            rise_button: AUTO_RIGHT_BUMPER,
            fall_button: AUTO_LEFT_BUMPER,
            flycam_rise_button: AUTO_DPAD_UP,
            flycam_fall_button: AUTO_DPAD_DOWN,
            ..Settings::gamepad_defaults()
        };
        let mut inherited = old.clone();
        inherited.flycam_rise_button = INHERIT_BUTTON;
        inherited.flycam_fall_button = INHERIT_BUTTON;
        inherited.migrate_gamepad_layout();
        assert_eq!(inherited, Settings::gamepad_defaults());
        old.migrate_gamepad_layout();
        assert_eq!(old, Settings::gamepad_defaults());
        old.flycam_fall_button = AUTO_DPAD_DOWN;
        old.migrate_gamepad_layout();
        assert_eq!(old.flycam_fall_button, AUTO_DPAD_DOWN, "subsequent custom choices survive");
        let mut custom = Settings {
            gamepad_layout_version: 0,
            gamepad_avatar_next_button: 8,
            gamepad_avatar_previous_button: 9,
            flycam_rise_button: 10,
            flycam_fall_button: 11,
            ..Settings::gamepad_defaults()
        };
        custom.migrate_gamepad_layout();
        assert_eq!((custom.gamepad_avatar_next_button, custom.gamepad_avatar_previous_button), (8, 9));
        assert_eq!((custom.flycam_rise_button, custom.flycam_fall_button), (10, 11));
    }

    #[test]
    fn joystick_crouch_height_and_reset_bindings_work_in_each_mode() {
        let settings = gamepad_settings();
        let mut snapshot = Snapshot {
            devices: vec![device()],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        let mut controller = Controller::default();
        for (flycam, first) in [(false, false), (false, true), (true, false)] {
            controller.sample_view(&snapshot, &settings, true, flycam, first, 0.1);
            let west = super::super::resolve_binding(&snapshot.devices[0], super::super::AUTO_WEST) as usize;
            let north = super::super::resolve_binding(&snapshot.devices[0], super::super::AUTO_NORTH) as usize;
            let east = super::super::resolve_binding(&snapshot.devices[0], super::super::AUTO_EAST) as usize;
            snapshot.devices[0].button(west, true);
            let motion = controller.sample_view(&snapshot, &settings, true, flycam, first, 0.1);
            assert_eq!(motion.crouch, !flycam);
            if flycam {
                assert!(motion.fall);
            }
            snapshot.devices[0].button(west, false);
            snapshot.devices[0].button(east, true);
            let motion = controller.sample_view(&snapshot, &settings, true, flycam, first, 0.1);
            assert_eq!(motion.rise, flycam);
            assert_eq!(motion.jump, !flycam);
            snapshot.devices[0].button(east, false);
            snapshot.devices[0].button(north, true);
            assert!(controller.sample_view(&snapshot, &settings, true, flycam, first, 0.1).reset_view);
            assert!(!controller.sample_view(&snapshot, &settings, true, flycam, first, 0.1).reset_view);
            snapshot.devices[0].button(north, false);
            controller.sample_view(&snapshot, &settings, true, flycam, first, 0.1);
        }
        // Settings written before these actions existed receive the new defaults.
        let mut saved = serde_json::to_value(&settings).expect("settings");
        saved.as_object_mut().unwrap().remove("crouch_button");
        saved.as_object_mut().unwrap().remove("reset_view_button");
        saved["first_person"].as_object_mut().unwrap().remove("crouch_button");
        let loaded: Settings = serde_json::from_value(saved).expect("old settings");
        assert_eq!(loaded.crouch_button, super::super::AUTO_WEST);
        assert_eq!(loaded.first_person.crouch_button, super::super::AUTO_WEST);
        assert_eq!(loaded.reset_view_button, super::super::AUTO_NORTH);
    }

    #[test]
    fn joystick_reset_binding_conflicts_are_resolved_in_all_modes() {
        let mut settings = gamepad_settings();
        let d = device();
        settings.bind_button("reset_view_button", 0, Some(&d));
        assert_eq!(settings.flycam_button, -1);
        assert_eq!(settings.view_button(false, true), -1);
        assert_eq!(settings.view_button(true, false), -1);
        settings.bind_button("first_person.crouch_button", 0, Some(&d));
        assert_eq!(settings.reset_view_button, -1);
        assert_eq!(settings.first_person.crouch_button, 0);
    }

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
                B::West,
                B::North,
            ],
        );
        let defaults = gamepad_settings();
        assert_eq!(resolve_binding(&d, defaults.jump_button), 4);
        assert_eq!(resolve_binding(&d, defaults.flycam_button), 2);
        assert_eq!(resolve_binding(&d, defaults.flycam_fall_button), 8);
        assert_eq!(resolve_binding(&d, defaults.flycam_rise_button), 4);
        assert_eq!(resolve_binding(&d, defaults.rise_button), -1);
        assert_eq!(resolve_binding(&d, defaults.fall_button), -1);
        assert_eq!(&defaults.avatar.gain[4..6], &[4.0, 4.0]);
        assert_eq!(&defaults.flycam.gain[4..6], &[4.0, 4.0]);
        for (flycam, first) in [(false, false), (false, true), (true, false)] {
            for (axis, digital, direction) in [(2, 1, -1.0), (5, 3, 1.0)] {
                let settings = defaults.clone();
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
        for (flycam, down_button, up_button) in [(false, 8, 4), (true, 8, 4)] {
            let mut snap = Snapshot {
                devices: vec![d.clone()],
                updated: Some(Instant::now()),
                ..Default::default()
            };
            let mut controller = Controller::default();
            controller.sample_view(&snap, &defaults, true, flycam, false, 0.1);
            snap.devices[0].button(down_button, true);
            let down = controller.sample_view(&snap, &defaults, true, flycam, false, 0.1);
            assert_eq!(down.fall, flycam);
            assert_eq!(down.crouch, !flycam);
            assert!(!down.rise);
            snap.devices[0].button(down_button, false);
            snap.devices[0].button(up_button, true);
            let up = controller.sample_view(&snap, &defaults, true, flycam, false, 0.1);
            assert_eq!(up.rise, flycam);
            assert_eq!(up.jump, !flycam);
            assert!(!up.fall);
        }
    }

    #[test]
    fn joystick_first_person_uses_its_own_axes_buttons_and_pressure_without_pan_or_targets() {
        let mut s = gamepad_settings();
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
        let mut s = gamepad_settings();
        let d = device();
        s.bind_button("first_person.jump_button", 0, Some(&d));
        assert_eq!(s.first_person.view_button, -1);
        assert_eq!(super::super::resolve_binding(&d, s.flycam_button), 0);
        assert_eq!(super::super::resolve_binding(&d, s.view_button(true, false)), 0);
        s.set_first_person_stick(Some(&d), false, GamepadStick::Right);
        assert_eq!(s.first_person.camera_stick, GamepadStick::None);
        assert_eq!(s.gamepad_camera_stick, GamepadStick::Right);
        s.bind_button("first_person.zoom_in_button", super::super::BUTTONS as i8, Some(&d));
        assert_eq!(s.first_person.move_stick, GamepadStick::None);
        assert_eq!(s.gamepad_move_stick, GamepadStick::Left);
        s.bind_button("gamepad_flycam_view_button", 1, Some(&d));
        assert_eq!(s.view_button(true, false), 1);
        assert_eq!(super::super::resolve_binding(&d, s.jump_button), 1);
        assert_eq!(super::super::resolve_binding(&d, s.view_button(false, false)), 0);
    }

    #[test]
    fn joystick_view_mode_press_is_one_edge_across_all_three_modes() {
        let s = gamepad_settings();
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
    fn joystick_bindings_trigger_and_button_actions_are_exclusive_within_each_mode() {
        let mut settings = gamepad_settings();
        let device = device();
        let trigger = super::super::gamepad_trigger_binding(&device, 1).expect("right trigger");
        settings.bind_button("zoom_in_button", trigger, Some(&device));
        assert_eq!(settings.gamepad_flycam_speedup_button, -1);
        assert!(
            sample(&settings, true, 6, 1.0).axes[6] < 0.0,
            "the gamepad zoom mapping operates its own lens"
        );
        settings.bind_button("jump_button", trigger, Some(&device));
        assert_eq!(settings.zoom_in_button, trigger, "different modes may reuse a physical control");
        assert!(
            sample(&settings, false, 6, 1.0).jump,
            "mapped analog triggers activate normal actions"
        );
        settings.bind_button("flycam_button", trigger, Some(&device));
        assert_eq!((settings.zoom_in_button, settings.jump_button), (-1, -1));
        settings.flycam_rise_button = super::super::INHERIT_BUTTON;
        settings.bind_button("rise_button", 9, Some(&device));
        assert_eq!(settings.flycam_rise_button, super::super::INHERIT_BUTTON);
        settings.bind_button("flycam_fall_button", 9, Some(&device));
        assert_eq!(settings.rise_button, 9, "flycam remapping preserves the avatar action");
        assert_eq!(settings.flycam_rise_button, -1);
    }

    #[test]
    fn joystick_profiles_select_and_enable_their_own_device_types() {
        let raw = Settings {
            enabled: true,
            ..Settings::default()
        };
        let mut gamepad = Settings::gamepad_defaults();
        let mut snapshot = Snapshot {
            devices: vec![device(), Device::new("raw".into(), "Raw joystick".into(), 8)],
            updated: Some(Instant::now()),
            ..Default::default()
        };
        assert!(!gamepad.enabled);
        assert_eq!(snapshot.selected_joystick(&raw).map(|device| device.id.as_str()), Some("raw"));
        assert!(snapshot.selected_gamepad(&gamepad).is_some_and(|device| device.is_gamepad));

        let mut controller = Controller::default();
        snapshot.devices[0].axes[0] = 1.0;
        assert!(
            !controller
                .sample_device(snapshot.selected_gamepad(&gamepad), &gamepad, false, false, 0.1)
                .moving()
        );
        gamepad.enabled = true;
        snapshot.devices[0].axes[0] = 0.0;
        controller.sample_device(snapshot.selected_gamepad(&gamepad), &gamepad, false, false, 0.1);
        snapshot.devices[0].axes[0] = 1.0;
        assert!(
            controller
                .sample_device(snapshot.selected_gamepad(&gamepad), &gamepad, false, false, 0.1)
                .moving()
        );
        gamepad.device = "raw".into();
        assert!(snapshot.selected_gamepad(&gamepad).is_none());
    }

    #[test]
    fn joystick_bindings_sensitivity_and_inversion_use_the_shared_gains() {
        let mut settings = gamepad_settings();
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
            ..gamepad_settings()
        };
        let motion = sample(&settings, true, 6, 1.0);
        assert_eq!(motion.flycam_speed, Some(50.0));
    }

    #[test]
    fn joystick_avatar_camera_uses_analog_trigger_pressure_and_releases_cleanly() {
        let settings = gamepad_settings();
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
    fn joystick_avatar_camera_dpad_hat_axes_pan_in_four_directions() {
        let settings = gamepad_settings();
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
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_pan_side, -1.0);
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_cycle, 0);
        snapshot.devices[0].axes[6] = -1.0;
        assert_eq!(controller.sample(&snapshot, &settings, true, false, 0.1).avatar_pan_side, 1.0);
    }
}
