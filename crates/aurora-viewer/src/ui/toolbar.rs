//! Toolbar customization after LLToolBar::handleRightMouseDown,
//! LLToolBarView::handleDropTool and LLFloaterToybox (indra/llui/lltoolbar.cpp,
//! indra/newview/{lltoolbarview,llfloatertoybox}.cpp, originally LGPL 2.1).
//! Moves commit on release; dropping back in the catalogue removes a button.
//! At the human's request, the layout and lock are shared by all accounts,
//! unlike Firestorm's per-account toolbar persistence.

use super::{
    menu,
    skin::{Layout, ToolButton},
    widgets,
};
use crate::theme::Palette;
use egui::{Rect, Response, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolbarSettings {
    /// None follows the skin; Some([]) deliberately keeps an empty toolbar.
    pub buttons: Option<Vec<ToolButton>>,
    pub locked: bool,
}

impl ToolbarSettings {
    pub fn buttons<'a>(&'a self, layout: &'a Layout) -> &'a [ToolButton] {
        self.buttons.as_deref().unwrap_or(&layout.toolbar)
    }

    pub fn sanitize(&mut self) {
        if let Some(buttons) = &mut self.buttons {
            let mut seen = std::collections::HashSet::new();
            buttons.retain(|b| !b.command.is_empty() && seen.insert(b.command.clone()));
        }
    }
}

#[derive(Clone)]
struct Drag {
    button: ToolButton,
    from_bar: bool,
}

enum Edit {
    Insert(ToolButton, usize),
    Remove(String),
    Restore,
    Clear,
    Lock,
}

#[derive(Default)]
pub struct ToolbarUi {
    pub open: bool,
    pub voice_options: bool,
    /// Where the favourites live (given by the app), for the Favorites
    /// button's menu, and the choice made there (taken by the app).
    pub favorites_access: crate::settings::FavoritesAccess,
    pub access_choice: Option<crate::settings::FavoritesAccess>,
    pending: Option<Edit>,
    /// true restores defaults, false clears all (Firestorm's confirmations).
    confirm: Option<bool>,
}

/// Built-in labels stay live when the language changes, including saved buttons.
pub fn label(button: &ToolButton) -> &str {
    // A skin may intentionally rename a command; translate only built-in labels.
    macro_rules! localized {
        ($key:literal) => {{
            if button.tooltip.is_empty()
                || aurora_i18n::available_languages().any(|(language, _)| button.tooltip == aurora_i18n::tr_in!(language, $key))
            {
                aurora_i18n::tr!($key)
            } else {
                &button.tooltip
            }
        }};
    }
    match button.command.as_str() {
        "conversations" => localized!("common-conversations"),
        "speak" => localized!("ui-skin-speak"),
        "fly" => localized!("extra-fly-land-f"),
        "mouselook" => localized!("common-mouselook-m"),
        "people" => localized!("common-people"),
        "friends" => localized!("common-friends"),
        "inventory" => localized!("common-inventory"),
        "gestures" => localized!("gesture-title"),
        "appearance" => localized!("common-appearance"),
        "ao" => localized!("ao-title"),
        "search" => localized!("search-title"),
        "places" => localized!("common-places"),
        "snapshot" => localized!("photo-title"),
        "worldmap" => localized!("common-world-map"),
        "minimap" => localized!("common-mini-map"),
        "performance" => localized!("common-performance"),
        "sit" => localized!("extra-sit-on-ground-stand-up"),
        "home" => localized!("common-go-home"),
        "favorites" => localized!("common-favorites"),
        "preferences" => localized!("common-preferences"),
        _ => &button.tooltip,
    }
}

fn catalogue(layout: &Layout) -> Vec<ToolButton> {
    let mut buttons = layout.toolbar.clone();
    buttons.extend(Layout::default().toolbar);
    for (command, icon) in [
        ("friends", "Command_People_Icon"),
        ("home", "home-filled"),
        ("favorites", "bookmarks-filled"),
    ] {
        buttons.push(ToolButton {
            command: command.into(),
            icon: icon.into(),
            tooltip: String::new(),
        });
    }
    let mut seen = std::collections::HashSet::new();
    buttons.retain(|b| seen.insert(b.command.clone()));
    buttons.sort_by(|a, b| label(a).cmp(label(b)));
    buttons
}

/// The pointer selects a boundary between buttons; use their real rectangles,
/// including the scroll offset, instead of dividing the width into equal slots.
fn insertion(rects: &[Rect], x: f32) -> usize {
    rects.iter().position(|r| x < r.center().x).unwrap_or(rects.len())
}

impl ToolbarUi {
    pub fn context(&mut self, response: &Response, p: &Palette, button: Option<&ToolButton>, locked: bool) {
        menu::context_menu(response, p, |ui| {
            if !locked && let Some(button) = button {
                if menu::item(ui, p, "trash-filled", aurora_i18n::tr!("toolbar-remove")) {
                    self.pending = Some(Edit::Remove(button.command.clone()));
                }
                menu::separator(ui, p);
            }
            if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("toolbar-edit")) {
                self.open = true;
            }
            if button.is_some_and(|b| b.command == "speak")
                && menu::item(ui, p, "microphone-filled", aurora_i18n::tr!("ui-bars-microphone-options"))
            {
                self.voice_options = true;
            }
            // the Favorites button: where the favourites live
            if button.is_some_and(|b| b.command == "favorites")
                && let Some(a) = crate::ui::bars::favorites::access_choice(ui, p, self.favorites_access)
            {
                self.access_choice = Some(a);
            }
            if menu::check(ui, p, "lock-filled", aurora_i18n::tr!("toolbar-locked"), locked, true) {
                self.pending = Some(Edit::Lock);
            }
        });
    }

    pub fn button(&mut self, response: &Response, button: &ToolButton, p: &Palette, locked: bool) {
        self.context(response, p, Some(button), locked);
        if !locked && response.drag_started_by(egui::PointerButton::Primary) {
            response.dnd_set_drag_payload(Drag {
                button: button.clone(),
                from_bar: true,
            });
        }
    }

    pub fn drop_bar(&mut self, response: &Response, rects: &[Rect], p: &Palette, locked: bool) {
        if locked || response.dnd_hover_payload::<Drag>().is_none() {
            return;
        }
        let Some(pos) = response.ctx.pointer_interact_pos() else { return };
        let rank = insertion(rects, pos.x);
        let x = rects
            .get(rank)
            .map_or_else(
                || rects.last().map_or(response.rect.left() + 3.0, |r| r.right() + 2.0),
                |r| r.left() - 2.0,
            )
            .clamp(response.rect.left() + 1.0, response.rect.right() - 1.0);
        response.ctx.layer_painter(response.layer_id).line_segment(
            [
                egui::pos2(x, response.rect.top() + 2.0),
                egui::pos2(x, response.rect.bottom() - 2.0),
            ],
            egui::Stroke::new(2.0, p.violet_light),
        );
        if let Some(drag) = response.dnd_release_payload::<Drag>() {
            self.pending = Some(Edit::Insert(drag.button.clone(), rank));
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, layout: &Layout, settings: &ToolbarSettings) {
        widgets::Floater::new(
            "toolbar_editor",
            aurora_i18n::tr!("toolbar-edit"),
            egui::pos2(180.0, 120.0),
            Vec2::new(500.0, 440.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            // Floaters inherit Extend for tables; this catalogue must wrap its
            // help and rows instead of growing wider than the viewer.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(aurora_i18n::tr!("toolbar-help"));
            let mut locked = settings.locked;
            if widgets::switch_labeled(ui, p, &mut locked, aurora_i18n::tr!("toolbar-locked")).changed() {
                self.pending = Some(Edit::Lock);
            }
            let current = settings.buttons(layout);
            // Already placed commands remain visible but cannot be added twice,
            // like LLFloaterToybox::draw. A click also appends for accessibility.
            let catalogue_height = (ui.available_height() - 75.0).clamp(80.0, 300.0);
            let list = egui::ScrollArea::vertical().max_height(catalogue_height).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for button in catalogue(layout) {
                        let placed = current.iter().any(|b| b.command == button.command);
                        let enabled = !settings.locked && !placed;
                        let response = widgets::action_button_if(
                            ui,
                            p,
                            &button.icon,
                            label(&button),
                            widgets::ActionKind::Flat,
                            Vec2::new(0.0, 30.0),
                            enabled,
                        )
                        .interact(egui::Sense::click_and_drag());
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(egui::Id::new(("toolbar_catalogue", &button.command)), response.rect));
                        if enabled {
                            if response.drag_started_by(egui::PointerButton::Primary) {
                                response.dnd_set_drag_payload(Drag {
                                    button: button.clone(),
                                    from_bar: false,
                                });
                            }
                            if response.clicked() {
                                self.pending = Some(Edit::Insert(button, current.len()));
                            }
                        } else if placed {
                            response.on_hover_text(aurora_i18n::tr!("toolbar-already-placed"));
                        }
                    }
                });
                ui.allocate_space(Vec2::new(ui.available_width(), 25.0));
            });
            let target = ui.interact(list.inner_rect, ui.id().with("catalogue_drop"), egui::Sense::hover());
            if !settings.locked
                && let Some(drag) = target.dnd_release_payload::<Drag>()
                && drag.from_bar
            {
                self.pending = Some(Edit::Remove(drag.button.command.clone()));
            }
            ui.label(aurora_i18n::tr!("toolbar-remove-help"));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!settings.locked, egui::Button::new(aurora_i18n::tr!("toolbar-restore")))
                    .clicked()
                {
                    self.confirm = Some(true);
                }
                if ui
                    .add_enabled(!settings.locked, egui::Button::new(aurora_i18n::tr!("toolbar-clear")))
                    .clicked()
                {
                    self.confirm = Some(false);
                }
            });
        });
        if let Some(restore) = self.confirm {
            let shown = widgets::Modal::new(egui::Id::new("toolbar_confirm")).show(ctx, |ui| {
                ui.label(if restore {
                    aurora_i18n::tr!("toolbar-confirm-restore")
                } else {
                    aurora_i18n::tr!("toolbar-confirm-clear")
                });
                ui.horizontal(|ui| {
                    if ui.button(aurora_i18n::tr!("common-ok")).clicked() {
                        self.pending = Some(if restore { Edit::Restore } else { Edit::Clear });
                        self.confirm = None;
                    }
                    if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                        self.confirm = None;
                    }
                });
            });
            if shown.should_close() {
                self.confirm = None;
            }
        }
        if let Some(drag) = egui::DragAndDrop::payload::<Drag>(ctx)
            && let Some(pointer) = ctx.pointer_interact_pos()
        {
            egui::Area::new(egui::Id::new("toolbar_drag_preview"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(pointer + Vec2::new(12.0, -18.0))
                .show(ctx, |ui| {
                    widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                        ui.horizontal(|ui| {
                            widgets::glyph(ui, &drag.button.icon, p.ink, 16.0);
                            ui.label(label(&drag.button));
                        });
                    });
                });
        }
    }

    /// Save once for a real edit, never on hover, during dragging or on a no-op.
    pub fn commit(&mut self, layout: &Layout, settings: &mut ToolbarSettings) -> bool {
        let Some(edit) = self.pending.take() else { return false };
        if matches!(edit, Edit::Lock) {
            settings.locked = !settings.locked;
            return true;
        }
        if settings.locked {
            return false;
        }
        if matches!(edit, Edit::Restore) {
            return settings.buttons.take().is_some();
        }
        let old = settings.buttons(layout);
        let mut next = old.to_vec();
        match edit {
            Edit::Insert(button, rank) => {
                let before = next.iter().take(rank).filter(|b| b.command == button.command).count();
                next.retain(|b| b.command != button.command);
                let rank = rank.saturating_sub(before).min(next.len());
                next.insert(rank, button);
            }
            Edit::Remove(command) => next.retain(|b| b.command != command),
            Edit::Clear => next.clear(),
            Edit::Restore | Edit::Lock => unreachable!(),
        }
        if next == old {
            return false;
        }
        settings.buttons = Some(next);
        true
    }
}

/// Offline scenarios use the real widgets and pointer events, as the human does.
pub fn demo_events(ctx: &egui::Context, frame: u64, state: &mut ToolbarUi) -> Vec<egui::Event> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(mode) = MODE.get_or_init(|| std::env::var("AURORA_DEMO_TOOLBAR").ok()).as_deref() else {
        return Vec::new();
    };
    if frame == 300 {
        widgets::place_floater(ctx, "toolbar_editor", egui::pos2(180.0, 120.0), Vec2::new(500.0, 440.0));
        state.open = true;
    }
    if mode == "1" {
        return Vec::new();
    }
    let bar = |command| ctx.read_response(super::bars::tool_button_id(command)).map(|r| r.rect.center());
    let source = if mode == "reorder" || mode == "menu" {
        bar("preferences")
    } else {
        ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("toolbar_catalogue", "friends"))))
            .map(|r| r.center())
    };
    let Some(source) = source else { return Vec::new() };
    let Some(target) = bar("inventory") else { return Vec::new() };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: if mode == "menu" {
            egui::PointerButton::Secondary
        } else {
            egui::PointerButton::Primary
        },
        pressed,
        modifiers: Default::default(),
    };
    match frame {
        650 => vec![egui::Event::PointerMoved(source), button(source, true)],
        651 if mode == "menu" => vec![button(source, false)],
        651 if mode != "1" => vec![egui::Event::PointerMoved(source + Vec2::new(12.0, 0.0))],
        652..=655 if !matches!(mode, "1" | "menu") => vec![egui::Event::PointerMoved(target)],
        656 if !matches!(mode, "1" | "menu" | "drag") => vec![button(target, false)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
