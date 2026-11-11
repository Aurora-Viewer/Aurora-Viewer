//! The viewer's interface in VR: a floating panel opened with the left
//! controller's X button (or its menu button where the runtime passes it on), used with the pointer beam (the trigger is the
//! left mouse button, the right thumbstick scrolls), and an on-screen
//! keyboard while a text field has the focus.
//!
//! The render worker draws the whole interface on the panel
//! (`aurora_render::VrPanel`); here the beam's hit on it becomes ordinary
//! egui pointer events, given to the interface with the next frame's input,
//! so every window works as it does with the mouse.

use super::*;

/// The panel's width, metres, and where it opens: this far ahead of the
/// headset and this far below the eyes.
const PANEL_WIDTH: f32 = 1.1;
const PANEL_AHEAD: f32 = 0.85;
const PANEL_BELOW: f32 = 0.12;
/// Points scrolled per frame at full right-stick deflection.
const SCROLL: f32 = 10.0;

const ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm,.?"];

/// Where the panel opens, in front of the headset and facing it, level,
/// with the interface's aspect (`aspect`: height / width).
pub(super) fn place_panel(head: Vec3, head_forward: Vec3, aspect: f32) -> aurora_render::VrPanel {
    let forward = head_forward.with_z(0.0).normalize_or(Vec3::X);
    let half = PANEL_WIDTH * 0.5;
    aurora_render::VrPanel {
        center: head + forward * PANEL_AHEAD - Vec3::Z * PANEL_BELOW,
        // (Z up: the facing crossed with up is the viewer's right)
        right: forward.cross(Vec3::Z).normalize_or(Vec3::Y) * half,
        up: Vec3::Z * half * aspect.clamp(0.2, 3.0),
        uv: aurora_render::VrPanel::WHOLE,
    }
}

impl App {
    /// Left X (or the left menu button, where the runtime passes it on:
    /// Virtual Desktop keeps it for its own menu) opens and closes the
    /// panel; it is closed outside VR. Kept in the render worker's hands
    /// every frame.
    pub(super) fn vr_panel_button(&mut self, renderer: &Renderer) {
        let input = renderer.vr_input();
        if !renderer.vr_engaged() {
            self.vr_panel = None;
            self.vr_menu_click = false;
            self.vr_keyboard_target = None;
            renderer.set_vr_panel(None);
            return;
        }
        let menu = input.left.active && (input.left.primary || input.left.menu);
        if menu && !self.vr_menu_click {
            self.vr_panel = if self.vr_panel.is_some() {
                None
            } else {
                let rect = self.egui_ctx.content_rect();
                let aspect = rect.height() / rect.width().max(1.0);
                Some(place_panel(input.head_position, input.head_forward, aspect))
            };
            if self.settings.camera.vr.haptics {
                renderer.vr_haptic(aurora_render::VrSide::Left, 0.35, 0.04);
            }
        }
        self.vr_menu_click = menu;
        renderer.set_vr_panel(self.vr_panel);
    }

    /// The microphone's light on the left controller: sending or not; None
    /// while voice is off.
    pub(super) fn vr_mic_light(&self) -> Option<bool> {
        self.settings.audio.voice_enabled.then(|| self.talking())
    }

    /// The beam on the panel: pointer events for the interface. True when
    /// the ray meets the panel (the world under it is then not clicked).
    pub(super) fn vr_panel_pointer(
        &mut self,
        ray: (Vec3, Vec3),
        trigger: (bool, bool),
        scroll: f32,
    ) -> Option<f32> {
        let (pressed, released) = trigger;
        let hit = self.vr_panel.and_then(|panel| panel.hit(ray.0, ray.1));
        let Some((t, u, v)) = hit else {
            if self.vr_on_panel {
                // the beam left the panel: a button held there is let go
                if self.vr_ui_down {
                    self.vr_ui_events.push(egui::Event::PointerButton {
                        pos: self.vr_ui_pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                    self.vr_ui_down = false;
                }
                self.vr_ui_events.push(egui::Event::PointerGone);
            }
            self.vr_on_panel = false;
            return None;
        };
        let rect = self.egui_ctx.content_rect();
        let pos = rect.min + egui::vec2(u * rect.width(), v * rect.height());
        self.vr_on_panel = true;
        self.vr_ui_pos = pos;
        self.vr_ui_events.push(egui::Event::PointerMoved(pos));
        if pressed || (released && self.vr_ui_down) {
            self.vr_ui_down = pressed;
            self.vr_ui_events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        if scroll != 0.0 {
            self.vr_ui_events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, scroll * SCROLL),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Default::default(),
            });
        }
        Some(t)
    }

    /// The hover tip's label by the beam: the part of the interface image
    /// where the tip was laid out (at the middle, `draw_hover_tips`), a
    /// little in front of the hit and above it, facing the headset, larger
    /// with distance so it reads the same.
    pub(super) fn vr_tip_label(&self, renderer: &Renderer, at: Option<Vec3>, head: Vec3) {
        let label = at.zip(self.hover_tip_rect()).and_then(|(point, tip)| {
            let screen = self.egui_ctx.content_rect();
            if screen.width() < 1.0 || screen.height() < 1.0 || tip.width() < 1.0 {
                return None;
            }
            let to_head = head - point;
            let distance = to_head.length().clamp(0.6, 12.0);
            let forward = (-to_head).with_z(0.0).normalize_or(Vec3::X);
            let metres_per_point = 0.0008 * distance;
            let half = egui::vec2(tip.width(), tip.height()) * 0.5 * metres_per_point;
            let toward = to_head.normalize_or_zero() * (0.15 * distance).min(0.6);
            let uv = [
                (tip.min.x - screen.min.x) / screen.width(),
                (tip.min.y - screen.min.y) / screen.height(),
                (tip.max.x - screen.min.x) / screen.width(),
                (tip.max.y - screen.min.y) / screen.height(),
            ];
            Some(aurora_render::VrPanel {
                center: point + toward + Vec3::Z * (half.y + 0.04 * distance),
                right: forward.cross(Vec3::Z).normalize_or(Vec3::Y) * half.x,
                up: Vec3::Z * half.y,
                uv: uv.map(|c| c.clamp(0.0, 1.0)),
            })
        });
        renderer.set_vr_tip(label);
    }

    /// The on-screen keyboard, at the bottom of the interface while the
    /// panel is open and a text field has the focus. A key's click would
    /// take the focus from the field: it is given back, and the key goes to
    /// the field with the next frame's input.
    pub(super) fn vr_keyboard(&mut self, ctx: &egui::Context) {
        if self.vr_panel.is_none() {
            self.vr_keyboard_target = None;
            return;
        }
        if ctx.egui_wants_keyboard_input()
            && let Some(focused) = ctx.memory(|m| m.focused())
        {
            self.vr_keyboard_target = Some(focused);
        }
        let Some(target) = self.vr_keyboard_target else {
            return;
        };
        let mut typed: Vec<egui::Event> = Vec::new();
        let mut close = false;
        let shift = self.vr_keyboard_shift;
        let key = |key: egui::Key| {
            [true, false].map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            })
        };
        let rect = ctx.content_rect();
        egui::Area::new(egui::Id::unique("vr-keyboard"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
            .constrain_to(rect)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let size = egui::vec2(46.0, 42.0);
                    let font = egui::FontId::proportional(20.0);
                    for row in ROWS {
                        ui.horizontal(|ui| {
                            for c in row.chars() {
                                let text = if shift { c.to_uppercase().to_string() } else { c.to_string() };
                                let label = egui::RichText::new(&text).font(font.clone());
                                if ui.add_sized(size, egui::Button::new(label)).clicked() {
                                    typed.push(egui::Event::Text(text));
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let wide = egui::vec2(size.x * 2.0, size.y);
                        let shift_label = if shift { "⇧ ABC" } else { "⇧ abc" };
                        if ui
                            .add_sized(wide, egui::Button::new(egui::RichText::new(shift_label).font(font.clone())).selected(shift))
                            .clicked()
                        {
                            self.vr_keyboard_shift = !shift;
                        }
                        if ui
                            .add_sized(egui::vec2(size.x * 5.0, size.y), egui::Button::new(egui::RichText::new("␣").font(font.clone())))
                            .clicked()
                        {
                            typed.push(egui::Event::Text(" ".into()));
                        }
                        if ui.add_sized(wide, egui::Button::new(egui::RichText::new("⌫").font(font.clone()))).clicked() {
                            typed.extend(key(egui::Key::Backspace));
                        }
                        if ui.add_sized(wide, egui::Button::new(egui::RichText::new("⏎").font(font.clone()))).clicked() {
                            typed.extend(key(egui::Key::Enter));
                        }
                        if ui.add_sized(size, egui::Button::new(egui::RichText::new("✕").font(font.clone()))).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.vr_keyboard_target = None;
            ctx.memory_mut(|m| m.surrender_focus(target));
            return;
        }
        if !typed.is_empty() {
            ctx.memory_mut(|m| m.request_focus(target));
            self.vr_ui_events.extend(typed);
            // one capital, as on a phone
            if self.vr_keyboard_shift && shift {
                self.vr_keyboard_shift = false;
            }
        } else if !ctx.egui_wants_keyboard_input() && ctx.memory(|m| m.focused()).is_none() && !self.vr_on_panel_press() {
            // the field was left (clicked elsewhere, closed): no keyboard
            self.vr_keyboard_target = None;
        }
    }

    /// A press on the panel this frame that is not on the keyboard yet:
    /// the keyboard stays while the press decides.
    fn vr_on_panel_press(&self) -> bool {
        self.vr_ui_down
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_opens_level_in_front_facing_the_head() {
        let head = Vec3::new(10.0, 20.0, 30.0);
        let panel = place_panel(head, Vec3::new(0.0, 1.0, -0.6), 0.5);
        // straight ahead along +Y, a little below the eyes
        assert!((panel.center - Vec3::new(10.0, 20.0 + PANEL_AHEAD, 30.0 - PANEL_BELOW)).length() < 1e-5);
        assert!(panel.right.z.abs() < 1e-6 && (panel.right.length() - PANEL_WIDTH * 0.5).abs() < 1e-5);
        assert!((panel.up.length() - PANEL_WIDTH * 0.25).abs() < 1e-5);
        // looking along +Y, the right is +X
        assert!(panel.right.x > 0.0);
        // the head's ray straight ahead meets its middle
        let (_, u, v) = panel.hit(head - Vec3::Z * PANEL_BELOW, Vec3::Y).expect("hit");
        assert!((u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4);
    }
}
