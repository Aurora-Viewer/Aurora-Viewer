//! The "What will you create?" page of a linkset window: the tab a reader gets
//! when they ask for a new script, before it exists. It offers the two
//! languages the grid knows, with what tells them apart, and the choice
//! creates the script through the contents panel's own path
//! (`LLPanelContents::onNewScriptFlyoutCommit`, indra/newview/llpanelcontents.cpp,
//! whose flyout this page takes the place of).
//!
//! The page itself owns no script: the tree holds the prim the new item goes
//! into, and the draft only remembers it.

use super::Lang;

use super::super::{icons, inventory, widgets};
use crate::theme::Palette;
use aurora_net::scripts::ScriptTarget;
use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use uuid::Uuid;

/// Everything the page draws, and the mark it keeps: the prim the new script
/// goes into, the path the tree gives it, whether the region runs Lua, and the
/// scripts that prim held when the page opened -- the one that appears after
/// the choice is the new one, and opens (the app's own listing answers it).
#[derive(Clone)]
pub(in crate::ui::scripts) struct Draft {
    pub object: Uuid,
    pub prim: String,
    /// The region runs SLua: the Lua card is offered.
    pub lua: bool,
    pub before: Vec<Uuid>,
    /// A language was chosen: the item is on its way, and the page waits for
    /// it (the first script to appear in the prim opens, `show`).
    pub chosen: bool,
    /// When it was chosen (egui's time): a choice the object never answered
    /// gives the page back after a moment (`view::body`).
    pub chosen_at: f64,
}

impl Draft {
    /// The document's target: the prim, with no item yet (nil, which no item
    /// of a grid ever has), so the tabs and the window treat it as its own.
    pub(in crate::ui::scripts) fn target(&self) -> ScriptTarget {
        ScriptTarget {
            object: self.object,
            item: Uuid::nil(),
        }
    }
}

/// The page: the two cards, side by side. `Some(lua)` when the reader chose a
/// language (`true` for Lua), the page then closes on its own.
pub(super) fn page(ui: &mut egui::Ui, p: &Palette, draft: &Draft) -> Option<bool> {
    // the strip's light down the page's own edge, as it runs down a source's
    // (`view::filament_edge`); the page takes the source's place, light too
    let pane = ui.available_rect_before_wrap();
    let length = (pane.height() * 0.45).clamp(24.0, 220.0);
    let left = pane.left() - 3.0;
    crate::ui::loading::gradient_rect(
        ui.painter(),
        Rect::from_min_max(Pos2::new(left, pane.top()), Pos2::new(left + 2.0, pane.top() + length)),
        p.violet.gamma_multiply(0.55),
        egui::Color32::TRANSPARENT,
        false,
    );
    let mut chosen = None;
    let (lsl_lines, lua_lines) = (lines(Lang::Lsl), lines(Lang::Lua));
    let room = ui.available_width();
    // side by side where the two fit, one under the other in a narrow window
    // (a wide tree or a small screen), never cut off
    let beside = beside(room);
    let width = if beside {
        ((room - 40.0 - CARD_GAP) * 0.5).clamp(150.0, 340.0)
    } else {
        (room - 40.0).max(120.0)
    };
    let height = if beside {
        height_of(ui, p, width, &lsl_lines).max(height_of(ui, p, width, &lua_lines))
    } else {
        height_of(ui, p, width, &lsl_lines) + CARD_GAP + height_of(ui, p, width, &lua_lines)
    };
    // the page centers its block in the room the window gives it: on a tall
    // window it reads from the middle, not from just under the tabs
    let title = ui
        .painter()
        .layout_no_wrap(
            aurora_i18n::tr!("script-create-title").to_string(),
            egui::FontId::proportional(17.0),
            p.ink,
        )
        .size()
        .y;
    let under = ui
        .painter()
        .layout_no_wrap(
            aurora_i18n::tr!("script-create-under", prim = draft.prim.as_str()).to_string(),
            egui::FontId::proportional(12.0),
            p.muted,
        )
        .size()
        .y;
    // the line that says the script is on its way keeps its room from the
    // start: the page never recenters when a card is chosen
    let waiting = 16.0 + 15.0;
    let content = title + 6.0 + under + 24.0 + height + waiting + 20.0;
    let top = ((pane.height() - content) * 0.5).max(26.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .id_salt("script_create_page")
        .show(ui, |ui| {
            ui.add_space(top);
            ui.vertical_centered(|ui| {
                // the viewer's heading of a window's page: 15 to 17 points,
                // semibold, as every dialog's title (`buy_currency`, `context`)
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("script-create-title"))
                        .size(17.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("script-create-under", prim = draft.prim.as_str()))
                        .size(12.0)
                        .color(p.muted),
                );
            });
            ui.add_space(24.0);
            // a language chosen: the item is on its way, and neither card
            // takes another click
            let live = !draft.chosen;
            if beside {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(((ui.available_width() - width * 2.0 - CARD_GAP) * 0.5).max(0.0));
                    if card(ui, p, width, height, Lang::Lsl, live, &lsl_lines).clicked() && live {
                        chosen = Some(false);
                    }
                    ui.add_space(CARD_GAP);
                    if card(ui, p, width, height, Lang::Lua, live && draft.lua, &lua_lines).clicked() && live {
                        chosen = Some(true);
                    }
                });
            } else {
                let lsl_height = height_of(ui, p, width, &lsl_lines);
                let lua_height = height_of(ui, p, width, &lua_lines);
                ui.vertical_centered(|ui| {
                    if card(ui, p, width, lsl_height, Lang::Lsl, live, &lsl_lines).clicked() && live {
                        chosen = Some(false);
                    }
                    ui.add_space(CARD_GAP);
                    if card(ui, p, width, lua_height, Lang::Lua, live && draft.lua, &lua_lines).clicked() && live {
                        chosen = Some(true);
                    }
                });
            }
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                if draft.chosen {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("script-create-waiting"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    // the item, or the moment the page takes its choice back
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                } else {
                    ui.add_space(15.0);
                }
            });
            ui.add_space(20.0);
        });
    chosen
}

/// What a card says, under its name: three short differences between the two
/// languages, from what the grid does with each.
fn lines(lang: Lang) -> Vec<&'static str> {
    let lua = lang.is_lua();
    if lua {
        vec![
            aurora_i18n::tr!("script-create-lua-fast").as_ref(),
            aurora_i18n::tr!("script-create-lua-region").as_ref(),
            aurora_i18n::tr!("script-create-lua-library").as_ref(),
        ]
    } else {
        vec![
            aurora_i18n::tr!("script-create-lsl-runs").as_ref(),
            aurora_i18n::tr!("script-create-lsl-compiled").as_ref(),
            aurora_i18n::tr!("script-create-lsl-known").as_ref(),
        ]
    }
}

/// Whether the two cards sit side by side in a `room` wide page, or one
/// under the other: the two least cards (150 each) and their air, the page's
/// own margins (20 each side) and the gap between them.
fn beside(room: f32) -> bool {
    room >= 2.0 * 150.0 + CARD_GAP + 40.0
}

/// The air between the two cards.
const CARD_GAP: f32 = 14.0;

const PAD: f32 = 16.0;
const TILE: f32 = 52.0;
const TITLE: f32 = 18.0;
const SUBTITLE: f32 = 12.0;
const TEXT: f32 = 12.0;
const LINE_GAP: f32 = 9.0;

/// How tall that card has to be for a `width` wide face, its lines wrapping
/// in it: the room from the tile down to the last line and the foot.
fn height_of(ui: &egui::Ui, p: &Palette, width: f32, lines: &[&str]) -> f32 {
    let wrap = (width - 2.0 * PAD - 14.0).max(40.0);
    let mut height = PAD + TILE + 14.0 + TITLE + 4.0 + SUBTITLE + 14.0;
    for line in lines {
        let galley = ui.painter().layout((*line).to_owned(), FontId::proportional(TEXT), p.muted, wrap);
        height += galley.size().y + LINE_GAP;
    }
    height + 16.0 + SUBTITLE + PAD
}

/// One language, as a card the reader clicks: its mark, its name, what it is,
/// the differences, and what a click does (or why there is none).
#[allow(clippy::too_many_arguments)]
fn card(ui: &mut egui::Ui, p: &Palette, width: f32, height: f32, lang: Lang, enabled: bool, lines: &[&str]) -> egui::Response {
    let lua = lang.is_lua();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hot = enabled && response.hovered();
    let radius = ui.visuals().window_corner_radius;
    let accent = if lua { p.teal } else { p.violet };
    let painter = ui.painter();
    // a face of its own over the source's, lifted under the pointer: the
    // page is the window's, its cards are the choices on it
    let face = if !enabled {
        p.field
    } else if hot {
        p.raised.lerp_to_gamma(accent, 0.10)
    } else {
        p.raised
    };
    painter.rect_filled(rect, radius, face);
    painter.rect_stroke(
        rect,
        radius,
        widgets::border_stroke(p, Stroke::new(1.0, if hot { accent } else { p.raised.lerp_to_gamma(p.ink, 0.06) })),
        egui::StrokeKind::Inside,
    );
    if hot {
        // the light of the choice: a soft wash under the mark, as the login
        // screen puts behind its own
        super::super::aurora_bg::radial_glow(
            painter,
            rect.left_top() + Vec2::new(PAD + TILE * 0.5, PAD + TILE * 0.5),
            TILE * 1.4,
            accent,
            0.10,
        );
    }
    let dim = if enabled { 1.0 } else { 0.45 };
    let tile = Rect::from_min_size(rect.left_top() + Vec2::splat(PAD), Vec2::splat(TILE));
    painter.rect_filled(tile, radius, accent.gamma_multiply(0.16 * dim));
    let uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
    // the existing script images at the size of a card's mark, and the skin's
    // own colours: the mark of an SLua script is the tree's
    if let Some(texture) = icons::global(inventory::item_icon(10, 10, u32::from(lua))) {
        painter.image(
            texture.id(),
            Rect::from_center_size(tile.center(), Vec2::splat(30.0)),
            uv,
            inventory::item_icon_color(10, 10, p).gamma_multiply(dim),
        );
    }
    let left = rect.left() + PAD;
    let mut top = tile.bottom() + 14.0;
    let title = if lua {
        aurora_i18n::tr!("script-create-lua").to_string()
    } else {
        aurora_i18n::tr!("build-ui-contents-script-lsl").to_string()
    };
    let title = painter.layout_no_wrap(title, FontId::proportional(TITLE), p.ink);
    painter.galley(Pos2::new(left, top), title.clone(), if enabled { p.ink } else { p.muted });
    top += title.size().y + 4.0;
    let subtitle = if lua {
        aurora_i18n::tr!("script-create-lua-subtitle").to_string()
    } else {
        aurora_i18n::tr!("script-create-lsl-subtitle").to_string()
    };
    let subtitle = painter.layout_no_wrap(subtitle, FontId::proportional(SUBTITLE), p.muted);
    painter.galley(Pos2::new(left, top), subtitle.clone(), p.muted.gamma_multiply(dim));
    top += subtitle.size().y + 14.0;
    let wrap = (width - 2.0 * PAD - 14.0).max(40.0);
    for line in lines {
        painter.circle_filled(Pos2::new(left + 3.0, top + TEXT * 0.6), 2.0, accent.gamma_multiply(0.8 * dim));
        let galley = painter.layout((*line).to_owned(), FontId::proportional(TEXT), p.muted, wrap);
        let height = galley.size().y;
        painter.galley(Pos2::new(left + 14.0, top), galley, p.muted.gamma_multiply(dim));
        top += height + LINE_GAP;
    }
    // the foot: what a click does, or why the card is shut
    let (text, tint) = if enabled {
        // the viewer's accent text for a way on (`friend_prompt`'s name), the
        // light violet reading on the cards' faces
        (aurora_i18n::tr!("script-create-choose").to_string(), p.violet_light)
    } else {
        (aurora_i18n::tr!("script-create-unavailable").to_string(), p.warn)
    };
    painter.text(
        Pos2::new(left, rect.bottom() - PAD - SUBTITLE * 0.5),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(SUBTITLE),
        tint.gamma_multiply(if enabled { if hot { 1.0 } else { 0.85 } } else { 0.9 }),
    );
    if hot {
        response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::NotAllowed
    })
}

/// Drawing helpers the tests below can call without a screen.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    /// The page's height only grows with narrower cards: a wrapped line takes
    /// another row, never less room than it had.
    /// The page keeps its block where it is when the item is on its way: the
    /// waiting line has its room from the start.
    #[test]
    fn the_page_keeps_its_place_when_the_script_is_being_created() {
        let title = aurora_i18n::tr!("script-create-title").to_string();
        let y = |chosen: bool| {
            let ctx = egui::Context::default();
            let p = Theme::default().palette();
            let draft = Draft {
                object: uuid::Uuid::nil(),
                prim: "Prim".into(),
                lua: true,
                before: Vec::new(),
                chosen,
                chosen_at: 0.0,
            };
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let _ = page(ui, &p, &draft);
                },
            );
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == title => Some(text.pos.y),
                    _ => None,
                })
                .expect("the title")
        };
        assert!((y(false) - y(true)).abs() < 0.5, "the title stays put: {} / {}", y(false), y(true));
    }

    #[test]
    fn a_narrow_card_asks_for_more_room() {
        let ctx = egui::Context::default();
        let p = Theme::default().palette();
        let lines = lines(Lang::Lsl);
        let mut wide = 0.0;
        let mut narrow = 0.0;
        let _ = ctx.run_ui(Default::default(), |ui| {
            wide = height_of(ui, &p, 320.0, &lines);
            narrow = height_of(ui, &p, 150.0, &lines);
        });
        assert!(narrow >= wide, "narrow {narrow} against wide {wide}");
    }

    /// The Lua card names Luau, the runtime it is written for, so a reader
    /// knows which one it is; the LSL card does not.
    #[test]
    fn each_language_says_what_it_is() {
        assert!(lines(Lang::Lua).iter().any(|line| line.contains("Luau")));
        assert!(lines(Lang::Lsl).iter().all(|line| !line.contains("Luau")));
    }

    /// The cards stand side by side while the window is wide enough for two
    /// whole ones, and stack when it is not: a page never cuts a choice off.
    #[test]
    fn the_cards_stack_in_a_narrow_page() {
        assert!(beside(760.0), "a window's source column");
        assert!(beside(2.0 * 150.0 + CARD_GAP + 40.0), "exactly the two least cards");
        assert!(!beside(2.0 * 150.0 + CARD_GAP + 39.0), "one point short of them");
        assert!(!beside(320.0), "a narrow pane");
    }
}
