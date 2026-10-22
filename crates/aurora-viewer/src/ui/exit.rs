//! Explicit quit / logout choice. LogoutRequest and the six-second fallback
//! follow LLAppViewer::requestQuit/sendLogoutRequest (llappviewer.cpp,
//! originally LGPL 2.1); the three-way prompt is Aurora's requested behavior.

use super::widgets::{self, ActionKind};
use crate::theme::Palette;
use egui::{Id, RichText, Vec2};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    Logout,
    Cancel,
}

#[derive(Default)]
pub struct ExitPrompt {
    pub open: bool,
    pending: Option<(Action, Instant)>,
}

impl ExitPrompt {
    pub fn request(&mut self) {
        self.open = true;
    }
    pub fn confirm(&mut self, action: Action, now: Instant) -> bool {
        if self.pending.is_some() {
            return false;
        }
        if action == Action::Cancel {
            self.open = false;
            return false;
        }
        self.pending = Some((action, now));
        true
    }
    pub fn finish(&mut self, disconnected: bool, now: Instant) -> Option<Action> {
        let (action, started) = self.pending?;
        if !disconnected && now.duration_since(started) < Duration::from_secs(6) {
            return None;
        }
        self.pending = None;
        self.open = false;
        Some(action)
    }
    pub fn show(&self, ctx: &egui::Context, p: &Palette, can_logout: bool) -> Option<Action> {
        if !self.open {
            return None;
        }
        let id = Id::unique("viewer_exit");
        aurora_render::blurred_backdrop(ctx, egui::Modal::default_area(id).layer());
        let busy = self.pending.is_some();
        let mut action = None;
        let width = (ctx.content_rect().width() - 80.0).clamp(240.0, 440.0);
        // the dialogs' surface (widgets::Modal), over this prompt's own
        // blurred backdrop instead of a dimmed one
        let response = widgets::Modal::new(id).transparent_backdrop().show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            ui.horizontal(|ui| {
                widgets::glyph(ui, "power", p.danger, 20.0);
                ui.label(RichText::new(aurora_i18n::tr!("exit-title")).size(17.0).strong().color(p.ink));
            });
            ui.label(
                RichText::new(if busy {
                    aurora_i18n::tr!("exit-waiting")
                } else if can_logout {
                    aurora_i18n::tr!("exit-world-body")
                } else {
                    aurora_i18n::tr!("exit-login-body")
                })
                .size(13.0)
                .color(p.muted),
            );
            ui.add_space(6.0);
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    let size = Vec2::new((width - 16.0) / 3.0, 32.0);
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        action = Some(Action::Cancel);
                    }
                    if ui
                        .add_enabled_ui(can_logout, |ui| {
                            widgets::action_button(ui, p, "logout", aurora_i18n::tr!("exit-logout"), ActionKind::Flat, size)
                        })
                        .inner
                        .on_hover_text(aurora_i18n::tr!("exit-logout-hint"))
                        .clicked()
                    {
                        action = Some(Action::Logout);
                    }
                    if widgets::action_button(ui, p, "power", aurora_i18n::tr!("exit-quit"), ActionKind::Danger, size).clicked() {
                        action = Some(Action::Quit);
                    }
                });
            });
        });
        // Clicking the blurred backdrop leaves the choice open. Escape is
        // cancel only, never a destructive default; ignore it while logging out.
        if !busy && response.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            action = Some(Action::Cancel);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        ctx: egui::Context,
        prompt: ExitPrompt,
        frame: usize,
        can_logout: bool,
        background_clicked: bool,
    }

    impl Fixture {
        fn show(&mut self, events: Vec<egui::Event>) -> (Option<Action>, Vec<egui::epaint::ClippedShape>) {
            self.frame += 1;
            let p = crate::theme::Theme::default().palette();
            let mut action = None;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    time: Some(self.frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    self.background_clicked |= ui.button("background").clicked();
                    action = self.prompt.show(ui.ctx(), &p, self.can_logout);
                },
            );
            output.textures_delta.clear();
            (action, output.shapes)
        }
        fn new(can_logout: bool) -> Self {
            let mut f = Self {
                ctx: egui::Context::default(),
                prompt: ExitPrompt::default(),
                frame: 0,
                can_logout,
                background_clicked: false,
            };
            f.prompt.request();
            for _ in 0..5 {
                f.show(vec![]);
            }
            f
        }
        fn click_label(&mut self, label: &str) -> Option<Action> {
            let (_, shapes) = self.show(vec![]);
            let pos = shapes
                .into_iter()
                .find_map(|s| match s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                })
                .expect("button label is visible");
            self.click(pos)
        }
        fn click(&mut self, pos: egui::Pos2) -> Option<Action> {
            self.show(vec![egui::Event::PointerMoved(pos)]);
            self.show(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            self.show(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }])
            .0
        }
    }

    #[test]
    fn actual_buttons_choose_quit_logout_and_cancel_with_logout_disabled_at_login() {
        for (label, action) in [
            ("exit-quit", Action::Quit),
            ("exit-logout", Action::Logout),
            ("common-cancel", Action::Cancel),
        ] {
            let mut f = Fixture::new(true);
            assert_eq!(f.click_label(aurora_i18n::text(label)), Some(action));
        }
        let mut f = Fixture::new(false);
        assert_eq!(f.click_label(aurora_i18n::text("exit-logout")), None);
        assert!(f.prompt.open);
    }

    #[test]
    fn backdrop_blocks_background_and_escape_cancels_without_quitting() {
        let mut f = Fixture::new(true);
        assert_eq!(f.click(egui::pos2(30.0, 15.0)), None);
        assert!(!f.background_clicked);
        assert!(f.prompt.open);
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(f.show(vec![escape.clone()]).0, Some(Action::Cancel));
        f.prompt.confirm(Action::Quit, Instant::now());
        assert_eq!(f.show(vec![escape]).0, None, "pending logout cannot be cancelled");
    }

    #[test]
    fn cancel_has_no_pending_logout_and_quit_waits_for_reply_or_timeout() {
        let now = Instant::now();
        let mut prompt = ExitPrompt::default();
        prompt.request();
        assert!(!prompt.confirm(Action::Cancel, now));
        assert!(!prompt.open);
        assert_eq!(prompt.finish(true, now), None);
        for action in [Action::Quit, Action::Logout] {
            prompt.request();
            assert!(prompt.confirm(action, now));
            assert!(!prompt.confirm(action, now), "logout must be sent once");
            assert_eq!(prompt.finish(false, now + Duration::from_secs(5)), None);
            assert_eq!(prompt.finish(true, now + Duration::from_secs(5)), Some(action));
            assert!(!prompt.open);
            prompt.request();
            prompt.confirm(action, now);
            assert_eq!(prompt.finish(false, now + Duration::from_secs(6)), Some(action));
        }
    }
}
