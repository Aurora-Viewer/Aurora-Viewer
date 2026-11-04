//! The keyboard shortcuts of the Legacy Viewer's menus that run an Aurora
//! feature reached otherwise from a menu, a window or the toolbar
//! (menu_viewer.xml accelerators): People on a tab, the master mute,
//! double-click teleport, selection outlines, the camera's angles and the
//! build selection's focus.

use super::App;
use crate::autopilot::DoubleClick;
use crate::camera::CameraPreset;
use std::time::Instant;

impl App {
    /// Communicate › Friends / Groups / Contact Sets (togglePeoplePanel,
    /// llviewermenu.cpp): People closes when that tab shows, otherwise it
    /// opens, or expands, on the tab.
    pub(super) fn people_tab_shortcut(&mut self, tab: u8) {
        let classic = self.settings.classic_people || (self.demo && crate::demo::people::legacy());
        let tab = people_tab_shown(tab, classic);
        let ctx = &self.egui_ctx;
        let shown = self.panels.people && self.panels.people_tab == tab && !crate::ui::widgets::floater_minimized(ctx, "people");
        if shown {
            crate::ui::widgets::toggle_floater(ctx, "people", &mut self.panels.people);
        } else {
            if self.panels.people {
                // open on another tab, or behind another window: show it
                crate::ui::widgets::bring_floater_front(ctx, "people");
            } else {
                crate::ui::widgets::open_floater(ctx, "people", &mut self.panels.people);
            }
            self.panels.people_tab = tab;
        }
    }

    /// World.ToggleMasterAudio: all sound off, or back on, as the speaker of
    /// the status bar.
    pub(super) fn toggle_master_mute(&mut self) {
        self.settings.audio.muted[0] = !self.settings.audio.muted[0];
        self.settings.save();
    }

    /// Me › Double-click on the ground, and the line the Legacy Viewer
    /// writes to the nearby chat.
    pub(super) fn set_double_click(&mut self, choice: DoubleClick) {
        self.settings.double_click = choice;
        self.settings.save();
        self.world
            .system_message(aurora_i18n::tr!("app-double-click-ground-set", action = choice.label().to_string()));
    }

    /// Advanced.SetDoubleClickAction "teleport_to" (setDoubleClickAction):
    /// teleporting on a double click is switched on, or off when it is on.
    pub(super) fn toggle_double_click_teleport(&mut self) {
        self.set_double_click(double_click_teleport_toggled(self.settings.double_click));
    }

    /// RenderHighlightSelections, as the build tools' option.
    pub(super) fn toggle_selection_outlines(&mut self) {
        self.settings.build.show_highlight = !self.settings.build.show_highlight;
        self.settings.save();
    }

    /// handle_reset_camera_angles: the view reset (Esc), then the camera
    /// and focus offsets the Ctrl / Shift wheel moves back to their defaults
    /// (CameraOffsetRearView, FocusOffsetRearView): the rear view's.
    pub(super) fn reset_camera_angles(&mut self) {
        if self.photo_escape() {
            return;
        }
        self.reset_camera_view();
        // the saved offsets are the preset's: an RLVa lock keeps them, as it
        // does for the preset menu and the Preferences
        if self.world.rlva.camera_preset_locked(uuid::Uuid::nil()) {
            return;
        }
        reset_rear_offsets(&mut self.settings.camera);
        if !self.demo {
            self.settings.save();
        }
    }

    /// Build › Focus on Selection / Zoom to Selection: only with something
    /// selected in the build tools, not a HUD (SomethingSelectedNoHUD).
    pub(super) fn look_at_selection(&mut self, zoom: bool) {
        if !self.build.open || self.build.hud_selected(&self.world) {
            return;
        }
        let now = Instant::now();
        let Some(bounds) = self.build.bounds(&self.world, now) else {
            return;
        };
        let object = self
            .build
            .selection
            .first()
            .and_then(|key| self.world.objects.index_of(key))
            .and_then(|idx| crate::camera::pick_focus_object(&self.world, idx, now));
        let aspect = self.view_aspect();
        self.camera.look_at_selection(
            bounds.center,
            bounds.half.length() * 2.0,
            object,
            zoom,
            aspect,
            &mut self.world.agent,
            &self.settings.camera,
        );
    }
}

/// The tab People shows for the one asked: the classic layout has no Contact
/// sets pane and shows Friends instead (ui/people.rs), so the same shortcut
/// pressed again finds its tab showing and closes People.
fn people_tab_shown(tab: u8, classic: bool) -> u8 {
    use crate::ui::people::{TAB_FRIENDS, TAB_SETS};
    if classic && tab == TAB_SETS { TAB_FRIENDS } else { tab }
}

/// What double-click teleport becomes: on, or off when it was on. Walking or
/// running on a double click gives way to teleporting, as the Legacy
/// Viewer's binding takes the double click.
fn double_click_teleport_toggled(current: DoubleClick) -> DoubleClick {
    if current == DoubleClick::Teleport {
        DoubleClick::Nothing
    } else {
        DoubleClick::Teleport
    }
}

/// The offsets back to the rear view's defaults. The preset becomes the
/// rear view unless the distance was changed too.
fn reset_rear_offsets(s: &mut crate::camera::CameraSettings) {
    let Some((camera, focus, scale)) = CameraPreset::Rear.values() else {
        return;
    };
    s.camera_offset = camera;
    s.focus_offset = focus;
    s.preset = if (s.offset_scale - scale).abs() < 1e-4 {
        CameraPreset::Rear
    } else {
        CameraPreset::Custom
    };
}

#[cfg(test)]
mod tests {
    use super::{double_click_teleport_toggled, people_tab_shown, reset_rear_offsets};
    use crate::autopilot::DoubleClick;
    use crate::camera::{CameraPreset, CameraSettings};
    use crate::ui::people::{TAB_FRIENDS, TAB_GROUPS, TAB_SETS};

    #[test]
    fn contact_sets_asks_the_classic_layout_for_the_friends_it_shows() {
        assert_eq!(people_tab_shown(TAB_SETS, true), TAB_FRIENDS);
        assert_eq!(people_tab_shown(TAB_SETS, false), TAB_SETS);
        assert_eq!(people_tab_shown(TAB_GROUPS, true), TAB_GROUPS);
    }

    #[test]
    fn reset_camera_angles_puts_the_wheel_offsets_back_to_the_rear_view() {
        let mut s = CameraSettings::default();
        s.camera_offset[2] += 0.5;
        s.focus_offset[2] -= 0.3;
        s.preset = CameraPreset::Custom;
        reset_rear_offsets(&mut s);
        let (camera, focus, _) = CameraPreset::Rear.values().expect("rear");
        assert_eq!((s.camera_offset, s.focus_offset, s.preset), (camera, focus, CameraPreset::Rear));
        // a distance changed by hand stays, and so the preset stays custom
        s.offset_scale = 1.4;
        s.preset = CameraPreset::Front;
        reset_rear_offsets(&mut s);
        assert_eq!(s.preset, CameraPreset::Custom);
        assert_eq!(s.camera_offset, camera);
    }

    #[test]
    fn zoom_to_selection_frames_its_box_with_the_wider_angle() {
        use crate::camera::selection_distance;
        // a 2 m diagonal, 1 radian of view, square: 2 * 1.75 / atan(1)
        assert!((selection_distance(2.0, 1.0, 1.0) - 3.5 / 1.0f32.atan()).abs() < 1e-5);
        // wider than tall: the horizontal angle (fov * aspect) frames it
        assert!((selection_distance(2.0, 1.0, 1.5) - 3.5 / 1.5f32.atan()).abs() < 1e-5);
        // never below 0.1 radian
        assert!((selection_distance(2.0, 0.01, 1.0) - 3.5 / 0.1f32.atan()).abs() < 1e-3);
    }

    #[test]
    fn double_click_teleport_switches_on_and_off() {
        assert_eq!(double_click_teleport_toggled(DoubleClick::Nothing), DoubleClick::Teleport);
        assert_eq!(double_click_teleport_toggled(DoubleClick::Walk), DoubleClick::Teleport);
        assert_eq!(double_click_teleport_toggled(DoubleClick::Teleport), DoubleClick::Nothing);
    }
}
