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
    focus_move: Option<egui::FocusDirection>,
    answer_key: Option<egui::Key>,
}

impl ExitPrompt {
    pub fn request(&mut self) {
        if !self.open {
            self.focus_move = None;
            self.answer_key = None;
            self.open = true;
        }
    }
    /// Keep keys out of the background and answer them when the dialog's
    /// buttons take focus. Tab followed by Enter can arrive in one frame.
    pub fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        if self.pending.is_some() || self.answer_key.is_some() {
            return;
        }
        if key == egui::Key::Tab && (modifiers.is_none() || modifiers == egui::Modifiers::SHIFT) {
            self.focus_move = Some(if modifiers.shift {
                egui::FocusDirection::Previous
            } else {
                egui::FocusDirection::Next
            });
        } else if modifiers.is_none() && matches!(key, egui::Key::Enter | egui::Key::Space | egui::Key::Escape) {
            self.answer_key = Some(key);
        }
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
    /// Quit confirmed, the viewer waiting for the logout reply to close.
    pub fn quitting(&self) -> bool {
        matches!(self.pending, Some((Action::Quit, _)))
    }
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, can_logout: bool) -> Option<Action> {
        if !self.open {
            return None;
        }
        let id = Id::unique("viewer_exit");
        aurora_render::blurred_backdrop(ctx, egui::Modal::default_area(id).layer());
        let busy = self.pending.is_some();
        let key = self.answer_key.take();
        let mut action = (!busy && key == Some(egui::Key::Enter)).then_some(Action::Quit);
        let activate = matches!(key, Some(egui::Key::Enter | egui::Key::Space));
        let clicked = |button: egui::Response| button.clicked() || (activate && button.enabled() && button.has_focus());
        let width = (ctx.content_rect().width() - 80.0).clamp(240.0, 440.0);
        // the dialogs' surface (widgets::Modal), over this prompt's own
        // blurred backdrop instead of a dimmed one
        let response = widgets::Modal::new(id).transparent_backdrop().show(ctx, |ui| {
            if let Some(direction) = self.focus_move.take() {
                ctx.memory_mut(|m| m.move_focus(direction));
            }
            ui.set_width(width);
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            ui.horizontal_top(|ui| {
                widgets::glyph(ui, "power", p.danger, 20.0);
                ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("exit-title")).size(17.0).strong().color(p.ink)).wrap());
            });
            ui.add(
                egui::Label::new(
                    RichText::new(if busy {
                        aurora_i18n::tr!("exit-waiting")
                    } else if can_logout {
                        aurora_i18n::tr!("exit-world-body")
                    } else {
                        aurora_i18n::tr!("exit-login-body")
                    })
                    .size(13.0)
                    .color(p.muted),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            ui.add_enabled_ui(!busy, |ui| {
                let widest = [
                    ("x", aurora_i18n::tr!("common-cancel")),
                    ("logout", aurora_i18n::tr!("exit-logout")),
                    ("power", aurora_i18n::tr!("exit-quit")),
                ]
                .into_iter()
                .map(|(icon, label)| widgets::action_button_fit(ui, icon, label))
                .fold(0.0_f32, f32::max);
                let row_width = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0;
                let horizontal = widest <= row_width;
                // Keep every label whole. When three equal buttons do not
                // fit, preserve their reading and keyboard order in a stack.
                let font = widgets::control_font(ui, 12.5);
                let height = (ui.fonts_mut(|f| f.row_height(&font)) + 2.0 * ui.spacing().button_padding.y).max(32.0);
                let size = Vec2::new(if horizontal { row_width } else { ui.available_width() }, height);
                let buttons = |ui: &mut egui::Ui| {
                    if clicked(widgets::action_button(
                        ui,
                        p,
                        "x",
                        aurora_i18n::tr!("common-cancel"),
                        ActionKind::Flat,
                        size,
                    )) {
                        action = Some(Action::Cancel);
                    }
                    if clicked(
                        ui.add_enabled_ui(can_logout, |ui| {
                            widgets::action_button(ui, p, "logout", aurora_i18n::tr!("exit-logout"), ActionKind::Flat, size)
                        })
                        .inner
                        .on_hover_text(aurora_i18n::tr!("exit-logout-hint")),
                    ) {
                        action = Some(Action::Logout);
                    }
                    if clicked(widgets::action_button(
                        ui,
                        p,
                        "power",
                        aurora_i18n::tr!("exit-quit"),
                        ActionKind::Danger,
                        size,
                    )) {
                        action = Some(Action::Quit);
                    }
                };
                if horizontal {
                    ui.horizontal_top(buttons);
                } else {
                    ui.vertical(buttons);
                }
            });
        });
        // Clicking the blurred backdrop leaves the choice open. Escape is
        // cancel only, never a destructive default; ignore it while logging out.
        if !busy
            && (key == Some(egui::Key::Escape)
                || response.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
        {
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
        screen_width: f32,
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
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(self.screen_width, 600.0))),
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
                screen_width: 800.0,
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
    fn exit_labels_stay_separate_and_clickable_with_large_text_and_a_narrow_window() {
        let buttons = [
            ("common-cancel", Action::Cancel),
            ("exit-logout", Action::Logout),
            ("exit-quit", Action::Quit),
        ];
        for width in [320.0, 800.0] {
            for scale in [1.0, 2.0] {
                let mut f = Fixture::new(true);
                f.screen_width = width;
                crate::theme::Theme::default().apply(&f.ctx, scale);
                for _ in 0..3 {
                    f.show(vec![]);
                }
                let (_, shapes) = f.show(vec![]);
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
                for shape in &shapes {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() != "background"
                    {
                        let ink = widgets::ink_rect(&text.galley).translate(text.pos.to_vec2());
                        assert!(!text.galley.elided, "text shortened at {width}/{scale}");
                        assert!(
                            shape.clip_rect.contains_rect(ink) && screen.contains_rect(ink),
                            "text clipped at {width}/{scale}: {ink:?}"
                        );
                    }
                }
                let rects: Vec<_> = buttons
                    .iter()
                    .map(|(key, _)| {
                        let label = aurora_i18n::text(key);
                        shapes
                            .iter()
                            .find_map(|s| match &s.shape {
                                egui::Shape::Text(t) if t.galley.text() == label => {
                                    assert!(!t.galley.elided, "label shortened at {width}/{scale}: {label}");
                                    let ink = widgets::ink_rect(&t.galley).translate(t.pos.to_vec2());
                                    assert!(s.clip_rect.contains_rect(ink), "label clipped at {width}/{scale}: {label}");
                                    let face = shapes
                                        .iter()
                                        .filter_map(|s| match &s.shape {
                                            egui::Shape::Rect(r) if r.rect.contains(ink.center()) => Some(r.rect),
                                            _ => None,
                                        })
                                        .min_by(|a, b| a.area().total_cmp(&b.area()))
                                        .expect("button face is visible");
                                    assert!(face.contains_rect(ink), "label outside its button at {width}/{scale}: {label}");
                                    Some(ink)
                                }
                                _ => None,
                            })
                            .expect("button label is visible")
                    })
                    .collect();
                for (i, rect) in rects.iter().enumerate() {
                    assert!(
                        rect.left() >= 0.0 && rect.right() <= width,
                        "label outside screen at {width}/{scale}: {rect:?}"
                    );
                    assert!(
                        rects[i + 1..].iter().all(|other| !rect.intersects(*other)),
                        "labels overlap at {width}/{scale}: {rects:?}"
                    );
                }
                for (label, action) in buttons {
                    assert_eq!(f.click_label(aurora_i18n::text(label)), Some(action));
                }
            }
        }
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

    #[test]
    fn the_prompt_takes_enter_and_escape_before_the_screen_behind() {
        let none = egui::Modifiers::NONE;
        let mut f = Fixture::new(false);
        f.prompt.key(egui::Key::Enter, none);
        assert_eq!(f.show(vec![]).0, Some(Action::Quit));
        f.prompt.key(egui::Key::Escape, none);
        assert_eq!(f.show(vec![]).0, Some(Action::Cancel));
        f.prompt.key(egui::Key::A, none);
        assert_eq!(f.show(vec![]).0, None);
        for modifiers in [egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::ALT] {
            f.prompt.key(egui::Key::Enter, modifiers);
            f.prompt.key(egui::Key::Escape, modifiers);
            assert_eq!(f.show(vec![]).0, None);
        }
        f.prompt.confirm(Action::Quit, Instant::now());
        f.prompt.key(egui::Key::Enter, none);
        f.prompt.key(egui::Key::Escape, none);
        assert_eq!(f.show(vec![]).0, None, "waiting for the logout");
    }

    #[test]
    fn tab_then_enter_activates_the_focused_button() {
        let mut f = Fixture::new(false);
        f.ctx.memory_mut(|m| m.stop_text_input());
        f.prompt.key(egui::Key::Tab, egui::Modifiers::NONE);
        f.prompt.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(f.show(vec![]).0, Some(Action::Cancel));
        f.prompt.key(egui::Key::Tab, egui::Modifiers::NONE);
        f.prompt.key(egui::Key::Space, egui::Modifiers::NONE);
        assert_eq!(f.show(vec![]).0, Some(Action::Quit), "disabled Logout is skipped");
        f.prompt.key(egui::Key::Tab, egui::Modifiers::SHIFT);
        f.show(vec![]);
        f.prompt.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(f.show(vec![]).0, Some(Action::Cancel));
        assert!(!f.background_clicked);
    }

    #[test]
    fn quitting_only_while_a_quit_waits_for_its_reply() {
        let now = Instant::now();
        let mut prompt = ExitPrompt::default();
        prompt.request();
        assert!(!prompt.quitting());
        prompt.confirm(Action::Logout, now);
        assert!(!prompt.quitting(), "a logout keeps the viewer open");
        prompt.finish(true, now);
        prompt.request();
        prompt.confirm(Action::Quit, now);
        assert!(prompt.quitting());
        prompt.finish(true, now);
        assert!(!prompt.quitting());
    }
}
