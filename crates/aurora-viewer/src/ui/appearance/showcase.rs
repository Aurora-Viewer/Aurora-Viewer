//! The outfit showcase: Aurora's outfit gallery (Fider #447, the mockup the
//! human chose on 2026-11-10, "B v2 — Vitrine"). A rail of collections (all
//! outfits, the favorites, those without a picture, the folders of My
//! Outfits), a grid of large square pictures whose actions show on hover,
//! and the preview of the chosen outfit with what it holds. Preferences ›
//! Aurora › Outfit gallery brings back the Legacy Viewer's grid of small
//! tiles (`appearance.rs`, `gallery`).
//!
//! The data and the rules are the gallery's and the list's: the outfits of
//! `world/outfit_folders.rs::tree`, LLOutfitGallery's picture (the folder's
//! thumbnail, else a picture inside, `AppearanceUi::outfit_picture`),
//! LLOutfitContextMenu's enabled states (`gallery::menu_rules`), RLVa's
//! folder and item locks, and the wearable list's rows and menus
//! (`items.rs`, LLWearableItemsList).

use super::{Action, AppearanceUi, folders, gallery, items, model};
use crate::{
    theme::Palette,
    ui::{fonts, icons, menu, widgets},
    world::{World, outfit_folders::Entry},
};
use aurora_net::outfits::categories::Update;
use egui::{Rect, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// From this width the showcase shows its rail of collections; narrower,
/// they are a menu over the grid.
const RAIL_FROM: f32 = 820.0;
/// From this width the preview stands beside the grid; narrower, the
/// window's selection card under the list stands for it.
const PREVIEW_FROM: f32 = 600.0;
const RAIL_W: f32 = 176.0;
const PREVIEW_W: f32 = 300.0;
const GAP: f32 = 10.0;
/// The least width of a card: the grid takes as many columns as fit.
const CARD_MIN: f32 = 150.0;
/// The room between two rows of cards.
const ROW_GAP: f32 = 14.0;
/// The width the window takes, once, when the showcase first opens in a
/// narrower window: its rail, three columns and its preview.
pub(super) const OPEN_WIDTH: f32 = 1080.0;
pub(super) const OPEN_HEIGHT: f32 = 720.0;

/// What the grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Favorites,
    NoPicture,
    /// The outfits of a folder of My Outfits, its subfolders' included.
    Folder(Uuid),
}

#[derive(Default)]
pub(super) struct State {
    pub(super) filter: Filter,
    /// "N outfits have no picture yet" closed for this session.
    banner_hidden: bool,
    cache: Option<Cache>,
    /// The item count of each outfit drawn, until the inventory changes.
    counts: (u64, HashMap<Uuid, usize>),
}

/// The outfits of the grid and the rail's counts, built again only when the
/// outfits shown (filter, order, inventory) or the collection change.
struct Cache {
    generation: u64,
    shown: Vec<Uuid>,
    filter: Filter,
    outfits: Vec<Uuid>,
    favorites: usize,
    no_picture: usize,
}

impl State {
    /// The collection's outfits, in the order of `shown` (the window's
    /// outfits, filtered by the search and sorted).
    fn outfits(&mut self, st_pictures: &mut dyn FnMut(Uuid) -> bool, world: &World, shown: &[Uuid]) -> &Cache {
        let inv = &world.inventory;
        if let Filter::Folder(id) = self.filter
            && !crate::world::outfit_folders::is_group(inv, id)
        {
            self.filter = Filter::All;
        }
        let fresh = self
            .cache
            .as_ref()
            .is_some_and(|c| c.generation == inv.generation && c.filter == self.filter && c.shown == shown);
        if !fresh {
            let favorite = |id: &Uuid| inv.folders.get(id).is_some_and(|f| f.info.favorite);
            let pictured: HashSet<Uuid> = shown.iter().copied().filter(|id| st_pictures(*id)).collect();
            let within = |id: &Uuid, folder: Uuid| {
                let mut at = inv.folders.get(id).map(|f| f.info.parent);
                for _ in 0..=crate::world::outfit_folders::MAX_DEPTH {
                    match at {
                        Some(parent) if parent == folder => return true,
                        Some(parent) => at = inv.folders.get(&parent).map(|f| f.info.parent),
                        None => return false,
                    }
                }
                false
            };
            let outfits = shown
                .iter()
                .copied()
                .filter(|id| match self.filter {
                    Filter::All => true,
                    Filter::Favorites => favorite(id),
                    Filter::NoPicture => !pictured.contains(id),
                    Filter::Folder(folder) => within(id, folder),
                })
                .collect();
            self.cache = Some(Cache {
                generation: inv.generation,
                shown: shown.to_vec(),
                filter: self.filter,
                outfits,
                favorites: shown.iter().filter(|id| favorite(id)).count(),
                no_picture: shown.len() - pictured.len(),
            });
        }
        self.cache.as_ref().expect("built above")
    }

    /// How many items the outfit `id` links to.
    fn item_count(&mut self, inv: &crate::world::inventory::Inventory, id: Uuid) -> usize {
        let (generation, counts) = &mut self.counts;
        if *generation != inv.generation {
            *generation = inv.generation;
            counts.clear();
        }
        *counts
            .entry(id)
            .or_insert_with(|| model::folder_links(inv, id).into_iter().filter(|l| !l.folder).count())
    }
}

/// Whether a showcase `width` points wide shows the preview beside its
/// grid.
pub(super) fn previews(width: f32) -> bool {
    width >= PREVIEW_FROM
}

/// Widens the window, once, to the showcase's rail, three columns and
/// preview when it first shows in a narrower window. The window is the
/// user's to make narrower again: the showcase then folds its rail into a
/// menu, then leaves its preview to the selection's card.
pub(super) fn widen_once(ctx: &egui::Context) {
    let done = egui::Id::unique("appearance_showcase_widened");
    if ctx.data_mut(|d| d.get_persisted::<bool>(done)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_persisted(done, true));
    let size = egui::AreaState::load(ctx, egui::Id::unique("appearance"))
        .and_then(|s| s.size)
        .unwrap_or(Vec2::new(580.0, 650.0));
    if size.x < OPEN_WIDTH {
        widgets::size_floater(ctx, "appearance", Vec2::new(OPEN_WIDTH, size.y.max(OPEN_HEIGHT)));
    }
}

/// The name of a collection, for the rail and the narrow window's menu.
fn filter_label(world: &World, filter: Filter) -> String {
    match filter {
        Filter::All => aurora_i18n::tr!("ui-appearance-showcase-all").into(),
        Filter::Favorites => aurora_i18n::tr!("common-favorites").into(),
        Filter::NoPicture => aurora_i18n::tr!("ui-appearance-showcase-no-picture").into(),
        Filter::Folder(id) => world.inventory.folders.get(&id).map(|f| f.info.name.clone()).unwrap_or_default(),
    }
}

/// Our own outfit, which its picture, favorite and name may change
/// (LLOutfitContextMenu's "folder" rules, `gallery::MenuRules::folder`).
fn own_outfit(world: &World, id: Uuid) -> bool {
    world
        .inventory
        .folders
        .get(&id)
        .is_some_and(|f| !f.library && f.info.type_default == model::FT_OUTFIT)
}

/// The showcase in `height` points of the window, in place of the list.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    entries: &[Entry],
    shown: &[Uuid],
    current: Option<Uuid>,
    actions: &mut Vec<Action>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    height: f32,
) {
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let wide = area.width() >= RAIL_FROM;
    let with_preview = area.width() >= PREVIEW_FROM;
    let mut grid = area;
    let rail = wide.then(|| {
        let r = Rect::from_min_size(area.min, Vec2::new(RAIL_W, area.height()));
        grid.min.x = r.right() + GAP;
        r
    });
    let preview = with_preview.then(|| {
        let r = Rect::from_min_max(egui::pos2(area.right() - PREVIEW_W, area.top()), area.max);
        grid.max.x = r.left() - GAP;
        r
    });
    let radius = ui.visuals().window_corner_radius;
    let face = p.field.gamma_multiply(0.65);
    let (no_picture, favorites, outfits) = {
        let mut state = std::mem::take(&mut st.showcase_state);
        let mut pictured = |id| !st.outfit_picture(&world.inventory, id).is_nil();
        let cache = state.outfits(&mut pictured, world, shown);
        let out = (cache.no_picture, cache.favorites, cache.outfits.clone());
        st.showcase_state = state;
        out
    };
    if let Some(rail) = rail {
        ui.painter().rect_filled(rail, radius, face);
        let mut r = ui.new_child(egui::UiBuilder::new().max_rect(rail.shrink(6.0)).id_salt("showcase_rail"));
        r.set_clip_rect(rail.intersect(ui.clip_rect()));
        rail_column(&mut r, p, world, st, entries, shown.len(), favorites, no_picture, actions);
    }
    ui.painter().rect_filled(grid, radius, face);
    let mut g = ui.new_child(egui::UiBuilder::new().max_rect(grid.shrink(8.0)).id_salt("showcase_grid"));
    g.set_clip_rect(grid.intersect(ui.clip_rect()));
    super::clipboard::list_focus(&mut g, grid);
    if !wide {
        collection_menu(&mut g, p, world, st, entries, shown.len(), favorites, no_picture);
        g.add_space(6.0);
    }
    banner(&mut g, p, world, st, shown, no_picture, actions);
    egui::ScrollArea::vertical()
        .id_salt("appearance_showcase")
        .auto_shrink([false, false])
        .show(&mut g, |ui| {
            ui.add_enabled_ui(st.pending.is_none(), |ui| {
                cards(ui, p, world, st, &outfits, current, actions, images);
                if outfits.is_empty() {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("ui-appearance-no-outfits-to-display"))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            });
        });
    if let Some(preview) = preview {
        ui.painter().rect_filled(preview, radius, p.field);
        let mut v = ui.new_child(egui::UiBuilder::new().max_rect(preview.shrink(12.0)).id_salt("showcase_preview"));
        v.set_clip_rect(preview.intersect(ui.clip_rect()));
        let picked = st
            .outfit_selection
            .focus
            .filter(|id| shown.contains(id))
            .or(current.filter(|id| shown.contains(id)));
        egui::ScrollArea::vertical()
            .id_salt("appearance_showcase_preview")
            .auto_shrink([false, false])
            .show(&mut v, |ui| match picked {
                Some(id) => preview_panel(ui, p, world, st, id, actions, images),
                None => {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new(aurora_i18n::tr!("ui-appearance-showcase-pick")).color(p.muted));
                    });
                }
            });
    }
}

/// The rail's rows: the three collections, then the folders of My Outfits
/// with their menu, and "New folder…".
#[allow(clippy::too_many_arguments)]
fn rail_column(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    entries: &[Entry],
    total: usize,
    favorites: usize,
    no_picture: usize,
    actions: &mut Vec<Action>,
) {
    egui::ScrollArea::vertical()
        .id_salt("appearance_showcase_rail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let filter = st.showcase_state.filter;
            for (choice, icon, tint, count) in [
                (Filter::All, "hanger-2-filled", p.muted, total),
                (Filter::Favorites, "star-filled", p.amber, favorites),
                (Filter::NoPicture, "photo-off", p.muted, no_picture),
            ] {
                let label = filter_label(world, choice);
                if widgets::rail_row(ui, p, icon, tint, &label, Some(count), filter == choice, 0).clicked() {
                    st.showcase_state.filter = choice;
                }
            }
            let groups: Vec<&Entry> = entries.iter().filter(|e| e.group).collect();
            if !groups.is_empty() {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-appearance-showcase-folders").to_uppercase())
                        .size(11.0)
                        .strong()
                        .color(p.muted),
                );
                ui.add_space(4.0);
            }
            for entry in groups {
                let name = filter_label(world, Filter::Folder(entry.id));
                let on = filter == Filter::Folder(entry.id);
                let response = widgets::rail_row(ui, p, "folder-filled", p.muted, &name, Some(entry.outfits), on, entry.depth);
                if response.clicked() {
                    st.showcase_state.filter = Filter::Folder(entry.id);
                }
                folders::group_menu(&response, p, world, st, entry.id, actions);
            }
            ui.add_space(8.0);
            if let Some(my) = crate::world::outfit_folders::my_outfits(&world.inventory)
                && widgets::action_button(
                    ui,
                    p,
                    "folder-plus",
                    aurora_i18n::tr!("ui-appearance-folders-new"),
                    widgets::ActionKind::Flat,
                    Vec2::new(ui.available_width(), 26.0),
                )
                .clicked()
            {
                st.begin_new_folder(my, Vec::new());
            }
        });
}

/// The collections as a menu over the grid, when the window is too narrow
/// for the rail.
#[allow(clippy::too_many_arguments)]
fn collection_menu(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    entries: &[Entry],
    total: usize,
    favorites: usize,
    no_picture: usize,
) {
    let filter = st.showcase_state.filter;
    let icon = match filter {
        Filter::All => "hanger-2-filled",
        Filter::Favorites => "star-filled",
        Filter::NoPicture => "photo-off",
        Filter::Folder(_) => "folder-filled",
    };
    let label = filter_label(world, filter);
    let response = widgets::action_menu_button(ui, p, icon, &label, false, 28.0);
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        for (choice, icon, count) in [
            (Filter::All, "hanger-2-filled", total),
            (Filter::Favorites, "star-filled", favorites),
            (Filter::NoPicture, "photo-off", no_picture),
        ] {
            let text = format!("{} ({count})", filter_label(world, choice));
            if menu::check(ui, p, icon, &text, filter == choice, true) {
                st.showcase_state.filter = choice;
            }
        }
        let groups: Vec<&Entry> = entries.iter().filter(|e| e.group).collect();
        if !groups.is_empty() {
            menu::separator(ui, p);
        }
        for entry in groups {
            let text = format!(
                "{}{} ({})",
                "   ".repeat(entry.depth),
                filter_label(world, Filter::Folder(entry.id)),
                entry.outfits
            );
            if menu::check(ui, p, "folder-filled", &text, filter == Filter::Folder(entry.id), true) {
                st.showcase_state.filter = Filter::Folder(entry.id);
            }
        }
    });
}

/// "N outfits have no picture yet", with Take the portraits (the batch
/// portraits' window, over our own outfits shown); closed for the session
/// by its cross.
fn banner(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    shown: &[Uuid],
    no_picture: usize,
    actions: &mut Vec<Action>,
) {
    let state = &mut st.showcase_state;
    if no_picture == 0 || state.banner_hidden || state.filter == Filter::NoPicture {
        return;
    }
    widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            super::icon(ui, "camera-filled", 16.0, p.violet_light);
            let text = aurora_i18n::tr!("ui-appearance-showcase-missing", count = no_picture);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if super::icon_button(ui, p, "x", aurora_i18n::tr!("ui-appearance-showcase-dismiss")).clicked() {
                    state.banner_hidden = true;
                }
                let take = aurora_i18n::tr!("ui-appearance-batch-open");
                if widgets::flat_button(ui, p, take).clicked() {
                    let own = shown.iter().copied().filter(|id| own_outfit(world, *id)).collect();
                    actions.push(Action::OutfitBatch(own));
                }
                ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(p.muted)).truncate());
            });
        });
    });
    ui.add_space(6.0);
}

/// The text under a card: its name and its item count.
fn name_font(ui: &egui::Ui) -> egui::FontId {
    egui::FontId::new(13.5, fonts::semibold(ui.ctx()))
}

fn meta_font() -> egui::FontId {
    egui::FontId::proportional(11.0)
}

/// The grid of cards. Every row has the same height (the square picture,
/// one line of name, one of count), so the rows out of sight are left out
/// whatever the size of the wardrobe; the outfit to locate is brought into
/// view even then.
#[allow(clippy::too_many_arguments)]
fn cards(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    outfits: &[Uuid],
    current: Option<Uuid>,
    actions: &mut Vec<Action>,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    let width = ui.available_width();
    let columns = (((width + GAP) / (CARD_MIN + GAP)).floor() as usize).clamp(2, 8);
    let card = ((width - GAP * (columns - 1) as f32) / columns as f32).max(60.0);
    // the fonts are read first: `fonts::semibold` locks the context too
    let (name, meta) = (name_font(ui), meta_font());
    let (name_h, meta_h) = ui.fonts_mut(|f| (f.row_height(&name), f.row_height(&meta)));
    let text_h = 6.0 + name_h + 1.0 + meta_h;
    let pitch = card + text_h + ROW_GAP;
    let rows = outfits.len().div_ceil(columns);
    if rows == 0 {
        return;
    }
    let (all, _) = ui.allocate_exact_size(Vec2::new(width, rows as f32 * pitch - ROW_GAP), Sense::hover());
    let view = ui.clip_rect();
    let first = (((view.top() - all.top()) / pitch).floor().max(0.0) as usize).saturating_sub(1);
    let last = ((((view.bottom() - all.top()) / pitch).ceil().max(0.0) as usize) + 1).min(rows);
    let place = |index: usize| {
        let (row, col) = (index / columns, index % columns);
        let min = all.min + Vec2::new(col as f32 * (card + GAP), row as f32 * pitch);
        Rect::from_min_size(min, Vec2::new(card, card + text_h))
    };
    if let Some(located) = st.locate
        && let Some(index) = outfits.iter().position(|id| *id == located)
        && !(first * columns..last * columns).contains(&index)
    {
        st.reveal_located(ui, located, place(index), egui::Align::Center);
    }
    for index in first * columns..(last * columns).min(outfits.len()) {
        card_ui(ui, p, world, st, outfits, outfits[index], place(index), current, actions, images);
    }
}

/// One card: the picture (or a hanger), the worn tag, the favorite star,
/// the hover actions, the name and the item count.
#[allow(clippy::too_many_arguments)]
fn card_ui(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    outfits: &[Uuid],
    id: Uuid,
    rect: Rect,
    current: Option<Uuid>,
    actions: &mut Vec<Action>,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    let inv = &world.inventory;
    let Some(folder) = inv.folders.get(&id) else { return };
    let picture = Rect::from_min_size(rect.min, Vec2::splat(rect.width()));
    let response = ui.interact(rect, egui::Id::unique(("showcase_card", id)), Sense::click());
    st.select_outfit(&response, id, outfits);
    st.reveal_located(ui, id, picture, egui::Align::Center);
    let hot = ui.rect_contains_pointer(rect) || response.has_focus();
    let selected = st.outfit_selection.contains(id);
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter();
    painter.rect_filled(picture, radius, p.field);
    let image = st.outfit_picture(inv, id);
    if !image.is_nil() {
        st.wanted_images.insert(image);
    }
    // an outfit of ours with no picture at all: the auto portrait at once
    let portrait = image.is_nil() && own_outfit(world, id);
    if let Some(t) = images.get(&image) {
        let (image_rect, uv) = super::preview_rects(picture.shrink(2.0), t.size_vec2(), st.fill_previews);
        painter.image(t.id(), image_rect, uv, widgets::PICTURE_TINT);
    } else if let Some(t) = icons::global("hanger-2-filled") {
        let (center, side) = if portrait {
            (picture.center() - Vec2::new(0.0, picture.height() * 0.12), 0.3)
        } else {
            (picture.center(), 0.42)
        };
        painter.image(
            t.id(),
            Rect::from_center_size(center, Vec2::splat(picture.width() * side)),
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            p.muted_dim,
        );
    }
    let stroke = if selected {
        egui::Stroke::new(2.0, p.violet)
    } else if hot {
        egui::Stroke::new(1.0, p.violet_light)
    } else {
        egui::Stroke::new(1.0, p.raised)
    };
    painter.rect_stroke(picture, radius, stroke, egui::StrokeKind::Inside);
    let flash = st.flash(ui, id);
    if flash > 0.0 {
        ui.painter().rect_filled(picture, radius, p.violet.gamma_multiply(0.3 * flash));
    }
    if current == Some(id) {
        widgets::paint_row_tag(
            ui,
            p,
            aurora_i18n::tr!("ui-appearance-worn"),
            true,
            picture.left() + 8.0,
            picture.top() + 17.0,
        );
    }
    let own = own_outfit(world, id);
    let favorite = folder.info.favorite;
    if favorite || (hot && own) {
        let star = Rect::from_min_size(picture.right_top() + Vec2::new(-36.0, 6.0), Vec2::splat(30.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(star).id_salt(("showcase_star", id)));
        let label = if favorite {
            aurora_i18n::tr!("ui-appearance-gallery-remove-from-favorites")
        } else {
            aurora_i18n::tr!("common-add-to-favorites")
        };
        let tint = if favorite { p.amber } else { p.ink };
        let enabled = own && st.pending.is_none();
        let clicked = child
            .add_enabled_ui(enabled, |ui| {
                widgets::glass_action_icon(ui, p, "star-filled", label, Vec2::splat(30.0), tint)
            })
            .inner
            .on_hover_text(label)
            .clicked();
        if clicked && enabled {
            actions.push(Action::Category(id, Update::Favorite(!favorite)));
        }
    }
    if portrait && !hot {
        let label = aurora_i18n::tr!("ui-appearance-portrait-auto");
        let width = (widgets::action_button_fit(ui, "user-scan", label)).min(picture.width() - 16.0);
        let at = Rect::from_center_size(
            egui::pos2(picture.center().x, picture.center().y + picture.height() * 0.16),
            Vec2::new(width, 30.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at).id_salt(("showcase_portrait", id)));
        let enabled = st.pending.is_none();
        if widgets::action_button_if(&mut child, p, "user-scan", label, widgets::ActionKind::Primary, at.size(), enabled).clicked()
            && enabled
        {
            actions.push(Action::OutfitPortrait(id));
        }
    }
    if hot {
        hover_actions(ui, p, world, st, id, picture, actions);
    }
    if super::items::double_clicked(&response) {
        st.open_outfit(id);
    }
    gallery::context_menu(&response, p, world, st, id, actions, true);
    let text_top = picture.bottom() + 6.0;
    let width = rect.width();
    let mut job = egui::text::LayoutJob::single_section(
        folder.info.name.clone(),
        egui::TextFormat::simple(name_font(ui), if current == Some(id) { p.violet_light } else { p.ink }),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let name = ui.painter().layout_job(job);
    let name_h = name.size().y;
    ui.painter().galley(egui::pos2(rect.left(), text_top), name, p.ink);
    let count = st.showcase_state.item_count(inv, id);
    let meta = ui
        .painter()
        .layout_no_wrap(aurora_i18n::tr!("workspace-outfit-items", count = count), meta_font(), p.muted);
    ui.painter().galley(egui::pos2(rect.left(), text_top + name_h + 1.0), meta, p.muted);
    response.on_hover_text(&folder.info.name);
}

/// Wear, Add and Take a snapshot over the bottom of a hovered card, enabled
/// as the outfit menu's entries are.
fn hover_actions(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, id: Uuid, picture: Rect, actions: &mut Vec<Action>) {
    let worn = items::worn_items(world);
    let Some(mut rules) = gallery::menu_rules(&world.inventory, &worn, st.pending.is_some(), id) else {
        return;
    };
    let locks = world.rlva_wear_locks();
    rules.replace &= locks.folder_allows(id, true);
    rules.add &= locks.folder_allows(id, true);
    let strip = Rect::from_min_max(egui::pos2(picture.left(), picture.bottom() - 42.0), picture.max);
    ui.painter().rect_filled(
        strip.shrink(1.0),
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            ..ui.visuals().window_corner_radius
        },
        p.panel.gamma_multiply(0.88),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(strip.shrink(6.0))
            .layout(egui::Layout::right_to_left(egui::Align::Center))
            .id_salt(("showcase_actions", id)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    let side = Vec2::splat(30.0);
    // a new photo: the auto portrait (the quick snapshot of the view stays
    // in the menu)
    let new_photo = aurora_i18n::tr!("ui-appearance-portrait-new-photo");
    if child
        .add_enabled_ui(rules.folder, |ui| {
            widgets::glass_action_icon(ui, p, "camera-filled", new_photo, side, p.ink)
        })
        .inner
        .on_hover_text(new_photo)
        .clicked()
        && rules.folder
    {
        actions.push(Action::OutfitPortrait(id));
    }
    let add = aurora_i18n::tr!("ui-appearance-gallery-add-to-your-outfit");
    if child
        .add_enabled_ui(rules.add, |ui| widgets::glass_action_icon(ui, p, "plus", add, side, p.ink))
        .inner
        .on_hover_text(add)
        .clicked()
        && rules.add
    {
        actions.push(Action::Wear(id, true));
    }
    let wear = aurora_i18n::tr!("common-wear");
    let size = Vec2::new(child.available_width(), 30.0);
    if child
        .add_enabled_ui(rules.replace, |ui| widgets::primary_button_sized(ui, p, wear, size))
        .inner
        .clicked()
        && rules.replace
    {
        actions.push(Action::Wear(id, false));
    }
}

/// The preview of the outfit `id`: its picture and the ways to set one, its
/// folder and name, Wear / Add / Take off (Edit for the current outfit),
/// and its items by kind.
#[allow(clippy::too_many_arguments)]
fn preview_panel(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    id: Uuid,
    actions: &mut Vec<Action>,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    let inv = &world.inventory;
    let Some(folder) = inv.folders.get(&id) else { return };
    let worn = items::worn_items(world);
    let locks = world.rlva_wear_locks();
    let rules = gallery::menu_rules(inv, &worn, st.pending.is_some(), id).map(|mut rules| {
        rules.replace &= locks.folder_allows(id, true);
        rules.add &= locks.folder_allows(id, true);
        rules.remove &= locks.folder_allows(id, false);
        rules
    });
    let side = ui.available_width();
    let (picture, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(picture, radius, p.panel);
    // the outfit's paid HD picture once it has loaded (`outfit_hd`), else
    // its picture; the cards keep the thumbnail
    let hd = model::outfit_hd_picture(inv, id);
    if let Some(hd) = hd {
        st.wanted_images.insert(hd);
    }
    let image = st.outfit_picture(inv, id);
    if !image.is_nil() {
        st.wanted_images.insert(image);
    }
    if let Some(t) = hd.and_then(|hd| images.get(&hd)).or_else(|| images.get(&image)) {
        let (image_rect, uv) = super::preview_rects(picture.shrink(2.0), t.size_vec2(), st.fill_previews);
        ui.painter().image(t.id(), image_rect, uv, widgets::PICTURE_TINT);
    } else {
        if let Some(t) = icons::global("hanger-2-filled") {
            ui.painter().image(
                t.id(),
                Rect::from_center_size(picture.center() - Vec2::new(0.0, side * 0.12), Vec2::splat(side * 0.28)),
                Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                p.muted_dim,
            );
        }
        let mut job = egui::text::LayoutJob::default();
        job.append(
            aurora_i18n::tr!("ui-appearance-showcase-no-picture-yet"),
            0.0,
            egui::TextFormat::simple(egui::FontId::new(14.0, fonts::semibold(ui.ctx())), p.ink),
        );
        job.append(
            &format!("\n{}", aurora_i18n::tr!("ui-appearance-showcase-no-picture-hint")),
            0.0,
            egui::TextFormat::simple(egui::FontId::proportional(12.0), p.muted),
        );
        job.wrap.max_width = side - 40.0;
        job.halign = egui::Align::Center;
        let text = ui.painter().layout_job(job);
        ui.painter()
            .galley(egui::pos2(picture.center().x, picture.center().y + side * 0.06), text, p.ink);
    }
    ui.painter()
        .rect_stroke(picture, radius, egui::Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
    let own = rules.is_some_and(|r| r.folder);
    // the auto portrait, the square photo mode and an image of the
    // inventory, along the bottom of the picture
    let buttons = Rect::from_min_max(
        egui::pos2(picture.left() + 10.0, picture.bottom() - 46.0),
        picture.max - Vec2::new(10.0, 10.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(egui::Layout::right_to_left(egui::Align::Center))
            .id_salt(("showcase_picture_buttons", id)),
    );
    child.spacing_mut().item_spacing.x = 6.0;
    for (icon, label, action) in [
        ("photo-filled", aurora_i18n::tr!("common-image-2"), Action::OutfitImage(id)),
        (
            "camera-filled",
            aurora_i18n::tr!("ui-appearance-portrait-photo-mode"),
            Action::OutfitPhotoMode(id),
        ),
    ] {
        if child
            .add_enabled_ui(own, |ui| widgets::glass_action_icon(ui, p, icon, label, Vec2::splat(34.0), p.ink))
            .inner
            .on_hover_text(label)
            .clicked()
            && own
        {
            actions.push(action);
        }
    }
    let portrait = aurora_i18n::tr!("ui-appearance-portrait-auto");
    let size = Vec2::new(child.available_width(), 36.0);
    let enabled = own && st.pending.is_none();
    if widgets::action_button_if(&mut child, p, "user-scan", portrait, widgets::ActionKind::Primary, size, enabled).clicked() && enabled {
        actions.push(Action::OutfitPortrait(id));
    }
    ui.add_space(10.0);
    let parent = folder.info.parent;
    let mut path = aurora_i18n::tr!("common-my-outfits").to_string();
    if crate::world::outfit_folders::is_group(inv, parent) {
        path = format!("{path} › {}", crate::world::outfit_folders::path(inv, parent));
    }
    ui.add(egui::Label::new(egui::RichText::new(path).size(11.0).color(p.muted)).truncate());
    ui.horizontal(|ui| {
        let more = super::icon_button(ui, p, "dots-filled", aurora_i18n::tr!("ui-appearance-showcase-more"));
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&folder.info.name)
                        .size(18.0)
                        .family(fonts::semibold(ui.ctx()))
                        .color(p.ink),
                )
                .truncate(),
            )
            .on_hover_text(&folder.info.name);
        });
        gallery::dropdown(&more, p, world, st, id, actions);
    });
    ui.add_space(6.0);
    if let Some(rules) = rules {
        use widgets::{ActionKind, action_button_if};
        // a long Take off goes under Wear and Add rather than out of the panel
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::new(0.0, 30.0);
            if action_button_if(
                ui,
                p,
                "shirt-filled",
                aurora_i18n::tr!("common-wear"),
                ActionKind::Primary,
                size,
                rules.replace,
            )
            .clicked()
            {
                actions.push(Action::Wear(id, false));
            }
            if action_button_if(ui, p, "plus", aurora_i18n::tr!("common-add"), ActionKind::Flat, size, rules.add).clicked() {
                actions.push(Action::Wear(id, true));
            }
            if rules.current {
                if action_button_if(
                    ui,
                    p,
                    "edit-filled",
                    aurora_i18n::tr!("common-edit"),
                    ActionKind::Flat,
                    size,
                    rules.folder,
                )
                .clicked()
                {
                    st.open(st.tab, true);
                }
            } else if action_button_if(
                ui,
                p,
                "minus",
                aurora_i18n::tr!("ui-appearance-gallery-take-off"),
                ActionKind::Flat,
                size,
                rules.remove,
            )
            .clicked()
            {
                actions.push(Action::RemoveOutfit(id));
            }
        });
    }
    ui.add_space(8.0);
    contents(ui, p, world, st, id, actions);
    ui.add_space(8.0);
    let copy = aurora_i18n::tr!("inventory-copy-outfit-list");
    if widgets::action_button_if(
        ui,
        p,
        "clipboard-text-filled",
        copy,
        widgets::ActionKind::Flat,
        Vec2::new(ui.available_width(), 26.0),
        gallery::can_copy_list(inv, id),
    )
    .clicked()
    {
        gallery::copy_list(ui.ctx(), world, st, id);
    }
}

/// The section of an item in the preview, as the wearable list groups them
/// (`items::sort_links`): body parts, clothing and layers, attachments.
fn section_of(asset_type: i32) -> usize {
    match asset_type {
        13 => 0,
        5 => 1,
        6 => 2,
        _ => 3,
    }
}

/// The items of the outfit `id`, by section, with the list's rows, menus and
/// double clicks; a hovered item not worn offers to be worn alone.
fn contents(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, id: Uuid, actions: &mut Vec<Action>) {
    let inv = &world.inventory;
    let mut links = model::folder_links(inv, id);
    items::sort_links(inv, &mut links, false);
    let found: Vec<_> = links
        .iter()
        .filter(|l| !l.folder)
        .filter_map(|l| inv.items.get(&l.target))
        .collect();
    if found.is_empty() {
        ui.label(
            egui::RichText::new(if model::loaded(inv, id) {
                aurora_i18n::tr!("ui-appearance-empty-outfit")
            } else {
                aurora_i18n::tr!("common-loading-2")
            })
            .size(12.0)
            .color(p.muted),
        );
        return;
    }
    let order: Vec<Uuid> = found.iter().map(|it| it.id).collect();
    st.retain_items(id, &order);
    let worn = items::worn_labels(world);
    let locks = world.rlva_wear_locks();
    let titles = [
        aurora_i18n::tr!("ui-appearance-showcase-body"),
        aurora_i18n::tr!("ui-appearance-showcase-clothing"),
        aurora_i18n::tr!("ui-appearance-showcase-attachments"),
        aurora_i18n::tr!("ui-appearance-showcase-other"),
    ];
    for (section, title) in titles.iter().enumerate() {
        let rows: Vec<_> = found.iter().filter(|it| section_of(it.asset_type) == section).copied().collect();
        if rows.is_empty() {
            continue;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).strong().color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(rows.len().to_string()).size(11.0).color(p.muted));
            });
        });
        for it in rows {
            ui.push_id(("showcase_item", id, it.id), |ui| {
                let label = worn.get(&it.id).map(String::as_str);
                let response = super::row(ui, p, it, "", st.item_selected(id, it.id), label);
                if response.clicked() || response.secondary_clicked() {
                    st.outfit_selection.single(id);
                }
                st.select_item(&response, id, it.id, &order);
                items::double_click(&response, world, it, label.is_some(), actions);
                let selected = st.selected_items(id, it.id, &order);
                items::context_menu(&response, p, world, st, id, it, &selected, actions);
                // the menu's Wear, for this item alone (wear_action, replace)
                if label.is_none()
                    && matches!(it.asset_type, 5 | 6 | 13)
                    && ui.rect_contains_pointer(response.rect)
                    && locks.wear(inv, it.id, 0).replace
                {
                    let wear = aurora_i18n::tr!("common-wear");
                    let width = widgets::action_button_fit(ui, "", wear);
                    let at = Rect::from_min_max(
                        egui::pos2(response.rect.right() - width - 2.0, response.rect.top() + 2.0),
                        response.rect.right_bottom() - Vec2::new(2.0, 2.0),
                    );
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at).id_salt("showcase_item_wear"));
                    if widgets::flat_button_sized(&mut child, p, wear, at.size())
                        .on_hover_text(aurora_i18n::tr!("ui-appearance-showcase-wear-item"))
                        .clicked()
                    {
                        actions.push(Action::WearItem {
                            item: it.id,
                            replace: true,
                            point: 0,
                        });
                    }
                }
            });
            super::row_gap(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{draw, draw_output, wardrobe};
    use super::*;

    /// A world with the demo outfits and the demo groups of My Outfits
    /// (Club, Beach › Night, Spring), and an Appearance window on the
    /// showcase.
    fn grouped() -> (World, AppearanceUi) {
        let mut world = wardrobe(0);
        crate::world::outfit_folders::seed_demo(&mut world.inventory, world.agent_id);
        let st = AppearanceUi {
            showcase: true,
            ..Default::default()
        };
        (world, st)
    }

    fn collection(world: &World, st: &mut AppearanceUi, filter: Filter) -> (Vec<Uuid>, usize, usize) {
        st.showcase_state.filter = filter;
        let shown = st.shown_outfits(&world.inventory);
        let mut state = std::mem::take(&mut st.showcase_state);
        let mut pictured = |id| !st.outfit_picture(&world.inventory, id).is_nil();
        let cache = state.outfits(&mut pictured, world, &shown);
        let out = (cache.outfits.clone(), cache.favorites, cache.no_picture);
        st.showcase_state = state;
        out
    }

    /// Favorites, No picture and a folder pick from the outfits the window
    /// shows; a folder holds its subfolders' outfits; the counts follow the
    /// inventory.
    #[test]
    fn collections_pick_their_outfits_and_count_them() {
        let (mut world, mut st) = grouped();
        let (beach_day, beach_night, club_red) = (Uuid::from_u128(746), Uuid::from_u128(747), Uuid::from_u128(744));
        let (all, favorites, no_picture) = collection(&world, &mut st, Filter::All);
        assert_eq!(all.len(), 8);
        assert_eq!((favorites, no_picture), (0, 8));
        let (beach, _, _) = collection(&world, &mut st, Filter::Folder(Uuid::from_u128(741)));
        assert_eq!(beach.len(), 2);
        assert!(
            beach.contains(&beach_day) && beach.contains(&beach_night),
            "Night's outfit is in Beach"
        );
        // a favorite and a picture
        let inv = &mut world.inventory;
        inv.folders.get_mut(&club_red).expect("Club Red").info.favorite = true;
        inv.folders.get_mut(&beach_day).expect("Beach Day").info.thumbnail = Uuid::from_u128(9_000);
        inv.generation += 1;
        let (favorite, favorites, no_picture) = collection(&world, &mut st, Filter::Favorites);
        assert_eq!(favorite, vec![club_red]);
        assert_eq!((favorites, no_picture), (1, 7));
        let (missing, _, _) = collection(&world, &mut st, Filter::NoPicture);
        assert_eq!(missing.len(), 7);
        assert!(!missing.contains(&beach_day));
        // a closed group keeps its outfits in the showcase, which has no header to open it by
        st.toggle_group(Uuid::from_u128(741));
        assert_eq!(collection(&world, &mut st, Filter::All).0.len(), 8);
        st.showcase = false;
        assert_eq!(st.shown_outfits(&world.inventory).len(), 6, "the Legacy gallery hides them");
        // a folder deleted meanwhile falls back to all the outfits
        st.showcase = true;
        world.inventory.folders.remove(&Uuid::from_u128(740));
        world.inventory.generation += 1;
        collection(&world, &mut st, Filter::Folder(Uuid::from_u128(740)));
        assert_eq!(st.showcase_state.filter, Filter::All);
    }

    /// Every text drawn, with where its galley starts.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Pos2)> {
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.pos)),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        shapes.iter().for_each(|s| collect(&s.shape, &mut out));
        out
    }

    fn frame(
        ctx: &egui::Context,
        world: &mut World,
        st: &mut AppearanceUi,
        events: Vec<egui::Event>,
    ) -> (Vec<Action>, Vec<(String, egui::Pos2)>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            events,
            ..Default::default()
        };
        let (actions, shapes) = draw_output(ctx, world, st, input);
        (actions, texts(&shapes))
    }

    fn click(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, at: egui::Pos2) -> Vec<Action> {
        let mut actions = frame(ctx, world, st, vec![egui::Event::PointerMoved(at)]).0;
        for pressed in [true, false] {
            let button = egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            actions.extend(frame(ctx, world, st, vec![button]).0);
        }
        actions
    }

    /// The first time it shows, the showcase widens the window to its rail,
    /// grid and preview; the cards' star, hover Wear and the preview's
    /// Wear push the actions of the outfit menu.
    #[test]
    fn cards_and_preview_act_on_their_outfit() {
        let (mut world, mut st) = grouped();
        let club_red = Uuid::from_u128(744);
        world.inventory.folders.get_mut(&club_red).expect("Club Red").info.favorite = true;
        world.inventory.generation += 1;
        // the Club folder: its two outfits on the first row, in sight
        st.showcase_state.filter = Filter::Folder(Uuid::from_u128(740));
        let ctx = egui::Context::default();
        for _ in 0..4 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let window = ctx.memory(|m| m.area_rect(egui::Id::unique("appearance"))).expect("window");
        assert!(window.width() >= OPEN_WIDTH - 1.0, "widened once: {window:?}");
        let (_, shown) = frame(&ctx, &mut world, &mut st, Vec::new());
        let at = |name: &str| shown.iter().find(|(t, _)| t == name).map(|(_, p)| *p).expect(name);
        assert!(
            shown.iter().any(|(t, _)| t == aurora_i18n::tr!("ui-appearance-showcase-all")),
            "the rail"
        );
        // the card's geometry from the names of two neighbours on one row
        let (red, black) = (at("Club Red"), at("Club Black"));
        assert!((red.y - black.y).abs() < 0.5, "Club Red and Club Black share a row");
        let card = (black.x - red.x).abs() - GAP;
        let picture_top = red.y - 6.0 - card;
        // the favorite's star, shown without a hover
        let star = egui::pos2(red.x + card - 21.0, picture_top + 21.0);
        let actions = click(&ctx, &mut world, &mut st, star);
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::Category(id, Update::Favorite(false)) if *id == club_red)),
            "the star unmarks the favorite: {actions:?}"
        );
        // a hovered card's Wear
        let inside = egui::pos2(red.x + card / 2.0, picture_top + card / 2.0);
        frame(&ctx, &mut world, &mut st, vec![egui::Event::PointerMoved(inside)]);
        let (_, hovered) = frame(&ctx, &mut world, &mut st, vec![egui::Event::PointerMoved(inside)]);
        let wear = hovered
            .iter()
            .filter(|(t, p)| t == aurora_i18n::tr!("common-wear") && p.x >= red.x && p.x < red.x + card && p.y < red.y)
            .map(|(_, p)| *p + Vec2::new(3.0, 5.0))
            .next()
            .expect("the card's Wear");
        let actions = click(&ctx, &mut world, &mut st, wear);
        assert!(
            actions.iter().any(|a| matches!(a, Action::Wear(id, false) if *id == club_red)),
            "{actions:?}"
        );
        // a click on its picture picks it: the preview shows it and wears it
        // (its Add is off: what it holds is worn already, getCanAddToCOF)
        click(&ctx, &mut world, &mut st, inside);
        assert_eq!(st.outfit_selection.focus, Some(club_red));
        let (_, shown) = frame(&ctx, &mut world, &mut st, Vec::new());
        let wear = shown
            .iter()
            .filter(|(t, p)| t == aurora_i18n::tr!("common-wear") && p.x > window.right() - PREVIEW_W - 20.0)
            .map(|(_, p)| *p + Vec2::new(3.0, 5.0))
            .next()
            .expect("the preview's Wear");
        let actions = click(&ctx, &mut world, &mut st, wear);
        assert!(
            actions.iter().any(|a| matches!(a, Action::Wear(id, false) if *id == club_red)),
            "{actions:?}"
        );
    }

    /// The window keeps its size and its footer in view with the showcase,
    /// wide (rail, grid, preview), medium (menu, grid, preview) and narrow
    /// (grid alone), at every zoom and text size, with 200 outfits.
    #[test]
    fn the_showcase_window_keeps_its_size() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            "appearance",
            &[Vec2::new(1080.0, 720.0), Vec2::new(700.0, 650.0), Vec2::new(480.0, 600.0)],
            aurora_i18n::tr!("ui-appearance-save-as"),
            || {
                let world = wardrobe(200);
                let st = AppearanceUi {
                    showcase: true,
                    ..Default::default()
                };
                (world, st)
            },
            |(world, st), ctx, p| {
                // the size under test, not the first opening's
                ctx.data_mut(|d| d.insert_persisted(egui::Id::unique("appearance_showcase_widened"), true));
                super::super::show(ctx, p, world, st, &mut true, 0, &HashMap::new());
            },
        );
    }
}
