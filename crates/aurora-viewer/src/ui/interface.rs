//! Hiding the interface: Firestorm's Show User Interface (Alt+Shift+U;
//! LLViewToggleUI, llviewermenu.cpp, and LLViewerWindow::setUIVisibility,
//! llviewerwindow.cpp, originally LGPL 2.1) and the bars in mouselook
//! (LLAgent::changeCameraToMouselook, FSShowInterfaceInMouselook). Hidden,
//! the interface keeps the world with its name tags, the HUDs, the toasts
//! and the chat lines; the bars go, and the windows open then
//! (`widgets::hide_windows`) come back with it. With the islands, a hint
//! says how to bring it back, and a screen edge hovered shows its bars for
//! as long as the pointer stays on them (the bars mockup); with Firestorm's
//! bars, its ConfirmHideUI question comes first.
//!
//! Immersive mode (#859, an Aurora addition: the Legacy Viewer has none) is
//! the same hidden interface with the chat kept: the conversation windows
//! stay, the chat lines and the chat bar (Enter) as when hidden; no edge
//! brings the bars back, no hint: small glass buttons in the top right
//! corner, faint until hovered, show or hide the avatars' name tags (hidden
//! by default, `immersive_name_tags`), the HUDs (#1190: shown by default,
//! `immersive_huds`; no button while Show HUDs hides them anyway) and leave
//! it.

use super::bars::islands::{HEIGHT, MARGIN};
use crate::theme::Palette;
use egui::{FontId, Id, Pos2, Rect, Sense, Vec2};

/// How close to a screen edge the pointer brings its bars back.
pub const EDGE: f32 = 4.0;
/// How long the hint stays once the interface is hidden (s).
pub const HINT_S: f64 = 6.0;
/// Height of the hint, as a slot of the islands.
const HINT_H: f32 = 30.0;

#[derive(Default)]
pub struct Interface {
    /// Hidden by its shortcut or its menus (HideUIControls, not saved).
    pub hidden: bool,
    /// Hidden by immersive mode (not saved).
    pub immersive: bool,
    /// When it was hidden (`ctx` time), for the hint.
    hidden_at: Option<f64>,
    /// The bars an edge brought back.
    peek_top: bool,
    peek_bottom: bool,
    /// The left and right toolbars an edge brought back.
    peek_sides: [bool; 2],
    /// ConfirmHideUI is asked (Firestorm's bars).
    pub confirm: bool,
    /// "Confirm before hiding the interface", as ticked in the question.
    pub confirm_again: bool,
    /// The room the bars left the last time they were shown, and the top of
    /// the bottom bar: the windows keep it while they are hidden.
    pub last_room: Option<(Rect, f32)>,
}

/// What a frame knows of the bars.
pub struct Frame {
    /// Firestorm's bars instead of the islands.
    pub classic: bool,
    pub mouselook: bool,
    /// FSShowInterfaceInMouselook.
    pub show_in_mouselook: bool,
    /// The chat field holds text, has the focus or is asked for (Enter).
    pub chat_active: bool,
    pub pointer: Option<Pos2>,
    pub screen: Rect,
    /// The bottom of the top islands (or of the favourites' row under them).
    pub top_rows: f32,
    /// The top of the bottom islands.
    pub bottom_top: f32,
    /// A menu is open (one of the bars' keeps them).
    pub menu_open: bool,
}

/// Which bars a frame draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    pub top: bool,
    pub bottom: bool,
}

impl Interface {
    /// Hide or show the interface, between two frames: the windows open now
    /// go with it and come back with it; the menus close. Shown, immersive
    /// mode ends too.
    pub fn set_hidden(&mut self, ctx: &egui::Context, hidden: bool) {
        self.set(ctx, hidden, false);
    }

    /// Enter immersive mode, between two frames: hidden, the chat windows
    /// kept.
    pub fn set_immersive(&mut self, ctx: &egui::Context) {
        self.set(ctx, true, true);
    }

    fn set(&mut self, ctx: &egui::Context, hidden: bool, immersive: bool) {
        self.hidden = hidden;
        self.immersive = hidden && immersive;
        self.hidden_at = hidden.then(|| ctx.input(|i| i.time));
        self.peek_top = false;
        self.peek_bottom = false;
        self.peek_sides = [false; 2];
        self.confirm = false;
        super::widgets::hide_windows(ctx, hidden, self.immersive);
        if hidden {
            egui::Popup::close_all(ctx);
        }
    }

    /// The bars this frame draws. In mouselook, Firestorm's: none, the chat
    /// bar while it is used, unless the interface stays in mouselook.
    /// Hidden: none; with the islands, an edge hovered brings its bars back
    /// while the pointer stays on them, a menu open from them or the chat in
    /// use. Immersive: the chat bar in use only.
    pub fn bars(&mut self, f: &Frame) -> Shown {
        if f.mouselook {
            return if f.show_in_mouselook && !self.hidden {
                Shown { top: true, bottom: true }
            } else {
                Shown {
                    top: false,
                    bottom: f.chat_active,
                }
            };
        }
        if !self.hidden {
            return Shown { top: true, bottom: true };
        }
        if f.classic || self.immersive {
            return Shown {
                top: false,
                bottom: f.chat_active,
            };
        }
        let near = |test: &dyn Fn(Pos2) -> bool| f.pointer.is_some_and(test);
        let at_top = near(&|p| p.y <= f.screen.top() + EDGE);
        let on_top = near(&|p| p.y <= f.top_rows + MARGIN);
        let at_bottom = near(&|p| p.y >= f.screen.bottom() - EDGE);
        let on_bottom = near(&|p| p.y >= f.bottom_top - MARGIN);
        self.peek_top = at_top || (self.peek_top && (on_top || f.menu_open));
        self.peek_bottom = at_bottom || (self.peek_bottom && (on_bottom || f.menu_open));
        Shown {
            top: self.peek_top,
            bottom: self.peek_bottom || f.chat_active,
        }
    }

    /// The left and right toolbars this frame draws, as the other bars:
    /// none in mouselook unless the interface stays there; hidden, with the
    /// islands, the left or right edge hovered brings its toolbar back while
    /// the pointer stays on it (`edges`: their inner sides, last frame) or a
    /// menu is open. Immersive: none.
    pub fn sides(&mut self, f: &Frame, edges: [f32; 2]) -> [bool; 2] {
        if f.mouselook {
            let on = f.show_in_mouselook && !self.hidden;
            return [on, on];
        }
        if !self.hidden {
            return [true, true];
        }
        if f.classic || self.immersive {
            return [false, false];
        }
        let near = |test: &dyn Fn(Pos2) -> bool| f.pointer.is_some_and(test);
        let at = [near(&|p| p.x <= f.screen.left() + EDGE), near(&|p| p.x >= f.screen.right() - EDGE)];
        let on = [near(&|p| p.x <= edges[0] + MARGIN), near(&|p| p.x >= edges[1] - MARGIN)];
        for i in 0..2 {
            self.peek_sides[i] = at[i] || (self.peek_sides[i] && (on[i] || f.menu_open));
        }
        self.peek_sides
    }

    /// Seconds left to the hint at `now` (the islands' look), while shown;
    /// none in immersive mode, which has its button.
    pub fn hint_left(&self, now: f64) -> Option<f64> {
        self.hidden_at
            .filter(|_| self.hidden && !self.immersive)
            .map(|at| HINT_S - (now - at))
            .filter(|s| *s > 0.0)
    }
}

/// The hint of the hidden interface, at the top of the screen in the middle
/// (the mockup's): "Interface hidden ·", the shortcut's keys, "or hover an
/// edge to bring it back".
pub fn hint(ctx: &egui::Context, p: &Palette, binding: Option<&crate::keybinds::Binding>) {
    let screen = ctx.content_rect();
    let id = Id::unique("interface_hidden_hint");
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, id));
    let font = FontId::proportional(12.0);
    let hidden = painter.layout_no_wrap(aurora_i18n::tr!("ui-bars-interface-hidden").to_owned(), font.clone(), p.muted);
    let recall = painter.layout_no_wrap(aurora_i18n::tr!("ui-bars-interface-recall").to_owned(), font, p.muted);
    const ICON: f32 = 15.0;
    const PAD: f32 = 10.0;
    const GAP: f32 = 8.0;
    let mut width = 0.0;
    let mut caps = Vec::new();
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(Pos2::ZERO)
        .constrain(false)
        .fade_in(false)
        .interactable(false)
        .show(ctx, |ui| {
            if let Some(b) = binding {
                caps = super::widgets::keycap::caps(ui, b, p.panel);
            }
            let caps_w = if caps.is_empty() {
                0.0
            } else {
                super::widgets::keycap::row_width(&caps) + GAP
            };
            width = PAD + ICON + GAP + hidden.size().x + GAP + caps_w + recall.size().x + PAD;
            let rect = Rect::from_min_size(
                Pos2::new(
                    (screen.center().x - width / 2.0).round(),
                    screen.top() + MARGIN + (HEIGHT - HINT_H) / 2.0,
                ),
                Vec2::new(width.ceil(), HINT_H),
            );
            ui.painter().add(super::widgets::island_shape(ui.ctx(), p, rect));
            ui.allocate_rect(rect, Sense::hover());
            let cy = rect.center().y;
            let mut x = rect.left() + PAD;
            let icon = Rect::from_center_size(Pos2::new(x + ICON / 2.0, cy), Vec2::splat(ICON));
            if let Some(t) = super::icons::global("eye-off") {
                ui.painter()
                    .image(t.id(), icon, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), p.violet_light);
            }
            x = icon.right() + GAP;
            let w = hidden.size().x;
            let at = crate::ui::widgets::row_text_origin(ui.painter(), &hidden, x, cy);
            ui.painter().galley(at, hidden.clone(), p.muted);
            x += w + GAP;
            if !caps.is_empty() {
                super::widgets::keycap::paint_row(ui, p, &caps, Pos2::new(x, cy), p.panel);
                x += super::widgets::keycap::row_width(&caps) + GAP;
            }
            let at = crate::ui::widgets::row_text_origin(ui.painter(), &recall, x, cy);
            ui.painter().galley(at, recall.clone(), p.muted);
        });
}

/// Opacity of the immersive mode's buttons at rest: there, but out of the
/// way.
const BUTTONS_REST: f32 = 0.4;

/// What the immersive mode's buttons ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmersiveAction {
    /// Leave immersive mode.
    Leave,
    /// The avatars' name tags shown or hidden.
    NameTags(bool),
    /// The HUDs shown or hidden.
    Huds(bool),
}

/// The immersive mode's buttons, square, as high as the islands, in the top
/// right corner at their margin, faint until one is hovered: the name tags
/// (`name_tags`: shown now), the HUDs (`huds`: shown now; None while Show
/// HUDs hides them anyway, no button then), then the way out.
pub fn immersive_buttons(ctx: &egui::Context, p: &Palette, name_tags: bool, huds: Option<bool>) -> Option<ImmersiveAction> {
    let screen = ctx.content_rect();
    let size = Vec2::splat(HEIGHT);
    let count = if huds.is_some() { 3.0 } else { 2.0 };
    let rect = Rect::from_min_max(
        Pos2::new(screen.right() - count * (MARGIN + HEIGHT), screen.top() + MARGIN),
        Pos2::new(screen.right() - MARGIN, screen.top() + MARGIN + HEIGHT),
    );
    let hot = ctx.pointer_hover_pos().is_some_and(|at| rect.contains(at));
    let mut action = None;
    egui::Area::new(Id::unique("immersive_buttons"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_opacity(if hot { 1.0 } else { BUTTONS_REST });
            ui.spacing_mut().item_spacing = Vec2::new(MARGIN, 0.0);
            ui.horizontal(|ui| {
                let (icon, label, tint) = if name_tags {
                    ("label-filled", aurora_i18n::tr!("ui-bars-immersive-hide-tags"), p.violet_light)
                } else {
                    ("label-off", aurora_i18n::tr!("ui-bars-immersive-show-tags"), p.ink)
                };
                if super::widgets::glass_action_icon(ui, p, icon, label, size, tint)
                    .on_hover_text(label)
                    .clicked()
                {
                    action = Some(ImmersiveAction::NameTags(!name_tags));
                }
                if let Some(huds) = huds {
                    let (icon, label, tint) = if huds {
                        (
                            "device-desktop-filled",
                            aurora_i18n::tr!("ui-bars-immersive-hide-huds"),
                            p.violet_light,
                        )
                    } else {
                        ("device-desktop-off", aurora_i18n::tr!("ui-bars-immersive-show-huds"), p.ink)
                    };
                    if super::widgets::glass_action_icon(ui, p, icon, label, size, tint)
                        .on_hover_text(label)
                        .clicked()
                    {
                        action = Some(ImmersiveAction::Huds(!huds));
                    }
                }
                let label = aurora_i18n::tr!("ui-bars-immersive-leave");
                if super::widgets::glass_action_icon(ui, p, "minimize", label, size, p.ink)
                    .on_hover_text(label)
                    .clicked()
                {
                    action = Some(ImmersiveAction::Leave);
                }
            });
        });
    action
}

/// Firestorm's ConfirmHideUI, with Firestorm's bars: "This action will hide
/// all menu items and buttons…", OK / Cancel and its "Confirm before hiding"
/// box. Some(true) to hide, Some(false) to keep the interface.
pub fn confirm(ctx: &egui::Context, p: &Palette, keys: &str, again: &mut bool) -> Option<bool> {
    let mut answer = None;
    let width = (ctx.content_rect().width() - 80.0).clamp(240.0, 400.0);
    let response = super::widgets::Modal::new(Id::unique("confirm_hide_interface")).show(ctx, |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
        ui.horizontal(|ui| {
            super::widgets::glyph(ui, "eye-off", p.violet_light, 20.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-bars-hide-interface"))
                    .size(17.0)
                    .strong()
                    .color(p.ink),
            );
        });
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-bars-interface-confirm", shortcut = keys.to_string()))
                .size(13.0)
                .color(p.muted),
        );
        super::widgets::checkbox(ui, p, again, aurora_i18n::tr!("ui-bars-interface-confirm-again"));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let size = Vec2::new((width - 8.0) / 2.0, 32.0);
            if super::widgets::action_button(
                ui,
                p,
                "x",
                aurora_i18n::tr!("common-cancel"),
                super::widgets::ActionKind::Flat,
                size,
            )
            .clicked()
            {
                answer = Some(false);
            }
            if super::widgets::action_button(
                ui,
                p,
                "eye-off",
                aurora_i18n::tr!("common-ok"),
                super::widgets::ActionKind::Primary,
                size,
            )
            .clicked()
            {
                answer = Some(true);
            }
        });
    });
    if answer.is_none() && response.is_top_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        answer = Some(false);
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pointer: Option<Pos2>) -> Frame {
        Frame {
            classic: false,
            mouselook: false,
            show_in_mouselook: false,
            chat_active: false,
            pointer,
            screen: Rect::from_min_size(Pos2::ZERO, Vec2::new(1536.0, 864.0)),
            top_rows: MARGIN + HEIGHT,
            bottom_top: 806.0,
            menu_open: false,
        }
    }

    const BOTH: Shown = Shown { top: true, bottom: true };
    const NONE: Shown = Shown { top: false, bottom: false };

    /// Hidden, the islands come back while the pointer, come from a screen
    /// edge, stays on them (or a menu of theirs is open), and go once it
    /// leaves; the middle of the screen brings nothing back.
    #[test]
    fn an_edge_brings_its_bars_back_while_the_pointer_stays() {
        let mut ui = Interface {
            hidden: true,
            ..Default::default()
        };
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 400.0)))), NONE);
        // the islands' row without coming from the edge: nothing
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 30.0)))), NONE);
        let top = Shown { top: true, bottom: false };
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 2.0)))), top);
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 50.0)))), top, "on the islands");
        let mut menu = frame(Some(Pos2::new(700.0, 300.0)));
        menu.menu_open = true;
        assert_eq!(ui.bars(&menu), top, "a menu of theirs keeps them");
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 300.0)))), NONE, "gone with the pointer");
        let bottom = Shown { top: false, bottom: true };
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 862.0)))), bottom);
        assert_eq!(ui.bars(&frame(Some(Pos2::new(300.0, 820.0)))), bottom);
        assert_eq!(ui.bars(&frame(None)), NONE, "the pointer out of the window");
        // the chat in use brings the bottom back
        let mut chat = frame(Some(Pos2::new(700.0, 400.0)));
        chat.chat_active = true;
        assert_eq!(ui.bars(&chat), bottom);
    }

    /// Shown, both bars; Firestorm's bars hidden come back with the shortcut
    /// only (the chat while it is used); in mouselook, Firestorm's: only the
    /// chat in use, unless the interface stays in mouselook.
    #[test]
    fn mouselook_and_firestorms_bars_follow_firestorm() {
        let mut ui = Interface::default();
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 2.0)))), BOTH);
        let mut ml = frame(None);
        ml.mouselook = true;
        assert_eq!(ui.bars(&ml), NONE);
        ml.chat_active = true;
        assert_eq!(ui.bars(&ml), Shown { top: false, bottom: true });
        ml.show_in_mouselook = true;
        assert_eq!(ui.bars(&ml), BOTH);
        ui.hidden = true;
        let mut classic = frame(Some(Pos2::new(700.0, 2.0)));
        classic.classic = true;
        assert_eq!(ui.bars(&classic), NONE, "no edge with Firestorm's bars");
    }

    /// The left and right toolbars go with the interface; with the islands,
    /// a side edge brings its toolbar back while the pointer stays on it
    /// (its inner side last frame, 8 px past it) or a menu is open; with
    /// Firestorm's bars, the shortcut only; in mouselook, as the other bars.
    #[test]
    fn a_side_edge_brings_its_toolbar_back() {
        let edges = [50.0, 1486.0];
        let mut ui = Interface::default();
        assert_eq!(ui.sides(&frame(None), edges), [true, true]);
        ui.hidden = true;
        assert_eq!(ui.sides(&frame(Some(Pos2::new(700.0, 400.0))), edges), [false, false]);
        assert_eq!(
            ui.sides(&frame(Some(Pos2::new(40.0, 400.0))), edges),
            [false, false],
            "not come from the edge"
        );
        assert_eq!(ui.sides(&frame(Some(Pos2::new(1.0, 400.0))), edges), [true, false]);
        assert_eq!(
            ui.sides(&frame(Some(Pos2::new(55.0, 400.0))), edges),
            [true, false],
            "on the toolbar"
        );
        let mut menu = frame(Some(Pos2::new(700.0, 400.0)));
        menu.menu_open = true;
        assert_eq!(ui.sides(&menu, edges), [true, false], "a menu of its own keeps it");
        assert_eq!(ui.sides(&frame(Some(Pos2::new(700.0, 400.0))), edges), [false, false]);
        assert_eq!(ui.sides(&frame(Some(Pos2::new(1535.0, 400.0))), edges), [false, true]);
        let mut classic = frame(Some(Pos2::new(1.0, 400.0)));
        classic.classic = true;
        assert_eq!(ui.sides(&classic, edges), [false, false], "no edge with Firestorm's bars");
        let mut ml = frame(None);
        ml.mouselook = true;
        ui.hidden = false;
        assert_eq!(ui.sides(&ml, edges), [false, false]);
        ml.show_in_mouselook = true;
        assert_eq!(ui.sides(&ml, edges), [true, true]);
    }

    /// Immersive mode: hidden, the chat bar back while it is used, but no
    /// edge brings a bar or a toolbar back and there is no hint; shown again
    /// (its button or the shortcut), it ends.
    #[test]
    fn immersive_mode_keeps_only_the_chat() {
        let ctx = egui::Context::default();
        let mut ui = Interface::default();
        ui.set_immersive(&ctx);
        assert!(ui.hidden && ui.immersive);
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 2.0)))), NONE, "no top edge");
        assert_eq!(ui.bars(&frame(Some(Pos2::new(700.0, 862.0)))), NONE, "no bottom edge");
        let mut chat = frame(None);
        chat.chat_active = true;
        assert_eq!(ui.bars(&chat), Shown { top: false, bottom: true });
        assert_eq!(ui.sides(&frame(Some(Pos2::new(1.0, 400.0))), [50.0, 1486.0]), [false, false]);
        assert_eq!(ui.hint_left(ctx.input(|i| i.time)), None, "its button, no hint");
        ui.set_hidden(&ctx, false);
        assert!(!ui.hidden && !ui.immersive);
        ui.set_hidden(&ctx, true);
        assert!(!ui.immersive, "Show User Interface hides without it");
    }

    #[test]
    fn the_hint_lasts_a_few_seconds() {
        let ui = Interface {
            hidden: true,
            hidden_at: Some(10.0),
            ..Default::default()
        };
        assert!(ui.hint_left(10.0).is_some() && ui.hint_left(15.9).is_some());
        assert_eq!(ui.hint_left(16.1), None);
        assert_eq!(ui.hint_left(13.0), Some(3.0));
        let shown = Interface {
            hidden_at: Some(10.0),
            ..Default::default()
        };
        assert_eq!(shown.hint_left(11.0), None, "shown again: no hint");
    }

    /// The windows open when the interface is hidden go with it and come
    /// back with it; a window opened meanwhile shows, one closed meanwhile
    /// is forgotten (LLFloaterView::hideAllFloaters / showHiddenFloaters).
    #[test]
    fn the_windows_open_go_with_the_interface_and_come_back() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let frame = |a: &mut bool, b: &mut bool| {
            let mut shown = (false, false);
            let mut out = ctx.run_ui(Default::default(), |ui| {
                let floater = |id| super::super::widgets::Floater::new(id, id, Pos2::new(100.0, 100.0), Vec2::new(200.0, 100.0));
                shown.0 = floater("a").show(ui.ctx(), &p, a, |_| ()).is_some();
                shown.1 = floater("b").show(ui.ctx(), &p, b, |_| ()).is_some();
            });
            out.textures_delta.clear();
            shown
        };
        let (mut a, mut b) = (true, false);
        assert_eq!(frame(&mut a, &mut b), (true, false));
        let mut ui = Interface::default();
        ui.set_hidden(&ctx, true);
        assert_eq!(frame(&mut a, &mut b), (false, false), "hidden");
        assert!(a, "still open");
        b = true;
        assert_eq!(frame(&mut a, &mut b), (false, true), "opened meanwhile");
        ui.set_hidden(&ctx, false);
        assert_eq!(frame(&mut a, &mut b), (true, true), "back");
        // closed while hidden, opened again: shown
        ui.set_hidden(&ctx, true);
        frame(&mut a, &mut b);
        a = false;
        frame(&mut a, &mut b);
        a = true;
        assert!(frame(&mut a, &mut b).0, "opened again, shown");
    }

    /// Immersive mode keeps the chat windows open and hides the others; the
    /// hidden interface hides both.
    #[test]
    fn immersive_mode_keeps_the_chat_windows() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let frame = || {
            let mut shown = (false, false);
            let mut out = ctx.run_ui(Default::default(), |ui| {
                let at = Pos2::new(100.0, 100.0);
                let size = Vec2::new(200.0, 100.0);
                let (mut a, mut b) = (true, true);
                let floater = super::super::widgets::Floater::new("other", "other", at, size);
                shown.0 = floater.show(ui.ctx(), &p, &mut a, |_| ()).is_some();
                let chat = super::super::widgets::Floater::new("conversations", "chat", at, size).chat();
                shown.1 = chat.show(ui.ctx(), &p, &mut b, |_| ()).is_some();
            });
            out.textures_delta.clear();
            shown
        };
        assert_eq!(frame(), (true, true));
        let mut ui = Interface::default();
        ui.set_immersive(&ctx);
        assert_eq!(frame(), (false, true), "immersive: the chat stays");
        ui.set_hidden(&ctx, false);
        assert_eq!(frame(), (true, true));
        ui.set_hidden(&ctx, true);
        assert_eq!(frame(), (false, false), "hidden: the chat goes too");
    }
}
