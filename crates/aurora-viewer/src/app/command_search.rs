use super::{App, Screen};
use crate::keybinds::{Input, Mods};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::KeyCode;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

impl App {
    pub(super) fn handle_command_search(&mut self, action: crate::ui::command_search::Action) {
        use crate::ui::command_search::Action;
        match action {
            Action::Environment(action) if !self.world.rlva.has("setenv") && !self.world.rlva.has("setsphere") => {
                self.env_actions([action]);
            }
            Action::Preferences(page, tab) => {
                self.options_ui.open(page, tab);
                crate::ui::widgets::open_floater(&self.egui_ctx, crate::ui::options::WINDOW, &mut self.panels.settings);
            }
            Action::Teleport { asset, name } if !self.world.rlva.has("showloc") && self.world.rlva_can_teleport_landmark() => {
                self.places_ui.confirm_landmark(asset, name);
            }
            Action::ShowOnMap(asset)
                if !self.world.rlva.has("showloc") && !self.rlva_refuses_shortcut(crate::keybinds::Action::WorldMap) =>
            {
                self.world
                    .landmarks
                    .request_action(asset, crate::world::landmarks::Action::ShowOnMap, std::time::Instant::now());
            }
            _ => {}
        }
    }

    pub(super) fn command_search_input(&mut self, code: KeyCode, event: &KeyEvent, mods: Mods) -> bool {
        let open = self.command_search.is_open();
        let demo = self.demo.then(crate::ui::command_search::demo_query).flatten();
        let enabled = match demo {
            Some("closed") => true,
            Some("off") => false,
            _ => self.settings.command_search,
        };
        if !enabled && !open {
            return false;
        }
        let input = Input::key(code);
        let pressed = event.state == ElementState::Pressed;
        let requested =
            pressed && !event.repeat && !self.altgr_text_input(event) && self.settings.keybinds.command_search_key(input.clone(), mods);
        if open || (requested && self.command_search_can_open(code, event, mods)) {
            if requested {
                self.command_search.toggle(&self.egui_ctx);
                self.down.clear();
                self.check_temp_run();
                self.egui_ctx.request_repaint();
            } else {
                self.down.remove(&input);
                if !pressed && let Some(key) = crate::gestures::keys::from_key(code, &event.key_without_modifiers()) {
                    self.gesture_state.manager.release(Some(key));
                }
                if pressed && code == KeyCode::Escape {
                    self.command_search.dismiss(&self.egui_ctx);
                    self.egui_ctx.request_repaint();
                }
            }
            return true;
        }
        false
    }

    fn command_search_can_open(&self, code: KeyCode, event: &KeyEvent, mods: Mods) -> bool {
        if !matches!(self.screen, Screen::World)
            || self.director_playing()
            || self.viewfinder_on()
            || self.photo_full_frame()
            || self.camera.mouselook()
            || self.cursor_hold.held()
            || self.web_has_focus()
            || self.media.focus.is_some()
            || self.egui_ctx.memory(|m| m.top_modal_layer().is_some())
            || egui::Popup::is_any_open(&self.egui_ctx)
            || (self.egui_ctx.egui_wants_keyboard_input()
                && (!mods.ctrl && !mods.alt || crate::keybinds::text_field_keeps_key(code, Some(&event.logical_key), mods)))
        {
            return false;
        }
        let mask = u32::from(mods.ctrl) | if mods.shift { 4 } else { 0 };
        mods.alt
            || !crate::gestures::keys::from_key(code, &event.key_without_modifiers())
                .is_some_and(|key| self.gesture_state.manager.uses_key(key, mask))
    }
}
