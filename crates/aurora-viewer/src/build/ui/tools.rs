//! Top of the build floater: the five tool buttons, the status line and
//! each tool's options (floater_tools.xml, LLFloaterTools::updatePopup),
//! the land tool's parcel section (LLPanelLandInfo) and the selection line
//! (LLFloaterTools::refresh).

use super::common::{check, label, small};
use super::{Confirm, Env, Request};
use crate::build::shapes::{GRASS_SPECIES, SHAPES, TREE_SPECIES};
use crate::build::{BuildSettings, BuildTool, EditMode, FocusMode, GrabMode, Mods, Tool};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::build::land_action;
use egui::{Color32, RichText, Vec2};

/// The tool buttons (icon, tooltip, tool).
const TOOLS: [(&str, &str, Tool); 5] = [
    ("magnifying-glass", "Mise au point (Ctrl+1)", Tool::Focus),
    ("hand-grabbing", "Déplacer (Ctrl+2)", Tool::Grab),
    ("cursor", "Modifier (Ctrl+3)", Tool::Edit),
    ("magic-wand", "Créer (Ctrl+4)", Tool::Create),
    ("mountains", "Terrain (Ctrl+5)", Tool::Land),
];

/// One of the five big tool buttons (35×25 in Firestorm).
fn tool_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(38.0, 26.0), egui::Sense::click());
    let fill = if on {
        p.violet
    } else if resp.hovered() {
        p.raised
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, 3.0, fill);
    let tint = if on { Color32::WHITE } else { p.ink };
    if let Some(t) = crate::ui::icons::global(icon) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(rect.center(), Vec2::splat(17.0)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp.on_hover_text(tip).clicked()
}

pub fn tool_row(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (icon, tip, t) in TOOLS {
            if tool_button(ui, p, icon, tip, tool.tool == t) && tool.tool != t {
                tool.set_tool(t);
            }
        }
    });
}

/// "text status": the hint of the current tool (floater.string status_*).
pub fn status_text(tool: &BuildTool, s: &BuildSettings, mods: Mods) -> &'static str {
    match tool.tool {
        Tool::Focus => "Cliquez et faites glisser pour bouger la caméra",
        Tool::Grab => "Faites glisser pour déplacer, appuyez sur Ctrl pour soulever, Ctrl-Maj pour pivoter",
        Tool::Edit => match tool.effective_mode(mods) {
            EditMode::Rotate => "Pour faire tourner l'objet, faites glisser les bandes de couleur.",
            EditMode::Stretch => "Pour étirer le côté sélectionné, cliquez et faites glisser.",
            EditMode::Face => "Cliquez sur une face pour la sélectionner (Maj : ajouter).",
            EditMode::Align => "Cliquez sur un cône pour aligner (Maj : empiler).",
            EditMode::Move if tool.cam.hud.is_some() => "Glissez une flèche ou la poignée du plan pour déplacer le HUD.",
            EditMode::Move => "Glissez pour déplacer, Maj-glissez pour copier.",
        },
        Tool::Create => "Cliquez dans le monde pour construire.",
        Tool::Land => {
            if s.land_action > land_action::REVERT {
                "Cliquez et faites glisser pour sélectionner le terrain."
            } else {
                "Cliquez et maintenez pour modifier le terrain."
            }
        }
    }
}

fn radio(ui: &mut egui::Ui, on: bool, text: &str) -> bool {
    ui.add(egui::RadioButton::new(on, RichText::new(text).size(12.0))).clicked()
}

/// Focus tool (focus_radio_group, slider zoom).
pub fn focus_panel(ui: &mut egui::Ui, _p: &Palette, tool: &mut BuildTool, mods: Mods, env: &mut Env) {
    let shown = match (mods.ctrl, mods.shift) {
        (true, true) => FocusMode::Pan,
        (true, false) => FocusMode::Orbit,
        _ => tool.focus_mode,
    };
    ui.horizontal(|ui| {
        if radio(ui, shown == FocusMode::Zoom, "Zoomer") {
            tool.focus_mode = FocusMode::Zoom;
        }
        // slider zoom: getCameraZoomFraction() * 0.5, 0..0.5
        let mut v = env.zoom_fraction * 0.5;
        let r = ui.add_enabled(
            tool.focus_mode == FocusMode::Zoom,
            egui::Slider::new(&mut v, 0.0..=0.5).step_by(0.01).show_value(false),
        );
        if r.changed() {
            tool.ui.requests.push(Request::ZoomFraction(v * 2.0));
        }
    });
    if radio(ui, shown == FocusMode::Orbit, "Faire tourner la caméra (Ctrl)") {
        tool.focus_mode = FocusMode::Orbit;
    }
    if radio(ui, shown == FocusMode::Pan, "Faire un panoramique (Ctrl+Maj)") {
        tool.focus_mode = FocusMode::Pan;
    }
}

/// Grab tool (move_radio_group).
pub fn grab_panel(ui: &mut egui::Ui, _p: &Palette, tool: &mut BuildTool, mods: Mods) {
    let shown = tool.effective_grab_mode(mods);
    if radio(ui, shown == GrabMode::Move, "Déplacer") {
        tool.grab_mode = GrabMode::Move;
    }
    if radio(ui, shown == GrabMode::Lift, "Soulever (Ctrl)") {
        tool.grab_mode = GrabMode::Lift;
    }
    if radio(ui, shown == GrabMode::Spin, "Faire tourner (Ctrl+Maj)") {
        tool.grab_mode = GrabMode::Spin;
    }
}

/// Edit tool: manipulators radio, options, part arrows, link, grid mode.
pub fn edit_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    let mode = tool.effective_mode(mods);
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.spacing_mut().item_spacing.y = 2.0;
        for (m, t) in [
            (EditMode::Move, "Bouger"),
            (EditMode::Rotate, "Pivoter (Ctrl)"),
            (EditMode::Stretch, "Étirer (Ctrl+Maj)"),
            (EditMode::Face, "Choisir une face"),
            (EditMode::Align, "Aligner"),
        ] {
            if radio(ui, mode == m, t) && tool.edit_mode != m {
                tool.set_edit_mode(world, s, m);
            }
        }
        if let Some(v) = check(ui, s.edit_linked, false, "Modification liée", true) {
            tool.set_edit_linked(world, s, v);
        }
        let ui = &mut cols[1];
        ui.spacing_mut().item_spacing.y = 2.0;
        if let Some(v) = check(ui, s.scale_uniform, false, "Étirer les deux côtés", true) {
            s.scale_uniform = v;
        }
        if let Some(v) = check(ui, s.scale_stretch_textures, false, "Étirer les textures", true) {
            s.scale_stretch_textures = v;
        }
        ui.horizontal(|ui| {
            if let Some(v) = check(ui, s.snap, false, "Fixer", true) {
                s.snap = v;
            }
            if super::common::icon_button(ui, p, "arrow-right", "Options de la règle (Ctrl+Maj+B)", true) {
                tool.ui.grid_options_open = !tool.ui.grid_options_open;
            }
        });
        if let Some(v) = check(ui, s.actual_root, false, "Axe d'édition à la racine", true) {
            s.actual_root = v;
        }
        if let Some(v) = check(ui, s.show_highlight, false, "Afficher le surlignage", true) {
            s.show_highlight = v;
        }
        if let Some(v) = check(ui, s.select_probes, false, "Sél. les sondes de réfl.", true) {
            s.select_probes = v;
        }
    });
    ui.horizontal(|ui| {
        let parts = !tool.selection.is_empty() && (s.edit_linked || tool.edit_mode == EditMode::Face);
        if super::common::icon_button(ui, p, "caret-left", "Sélectionner la partie ou face précédente (Ctrl+,)", parts) {
            tool.select_next_part(world, s, false, mods.shift);
        }
        if super::common::icon_button(ui, p, "caret-right", "Sélectionner la partie ou face suivante (Ctrl+.)", parts) {
            tool.select_next_part(world, s, true, mods.shift);
        }
        let can_link = !tool.hud_selected(world) && !s.edit_linked && tool.roots(world).len() >= 2;
        if ui
            .add_enabled(can_link, egui::Button::new(RichText::new("Lien").size(12.0)))
            .on_hover_text("Lier les objets sélectionnés (Ctrl+L)")
            .clicked()
        {
            tool.link(world);
        }
        if ui
            .add_enabled(
                !tool.hud_selected(world) && !tool.selection.is_empty(),
                egui::Button::new(RichText::new("Annuler le lien").size(12.0)),
            )
            .on_hover_text("Délier les objets sélectionnés (Ctrl+Maj+L)")
            .clicked()
        {
            tool.unlink(world);
        }
        let items = [("Monde", 0u8), ("Local", 1), ("Référence", 2)];
        if let Some(m) = super::common::combo(ui, "grid_mode", Some(s.grid_mode), &items, 82.0, true) {
            s.grid_mode = m;
            if m == 2 && tool.grid_objects.is_empty() {
                tool.use_selection_for_grid(world, s);
            }
        }
    });
    // link number / selected faces (link_num_obj_count)
    if let Some(info) = tool.part_info(world, s) {
        small(ui, p, &info);
    }
}

/// Create tool: the 15 shapes, options, tree / grass species.
pub fn create_panel(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    ui.horizontal_top(|ui| {
        let cell = Vec2::new(26.0, 26.0);
        egui::Grid::new("shapes").spacing(Vec2::splat(3.0)).show(ui, |ui| {
            for i in 0..SHAPES.len() + 2 {
                let sel = s.create_shape == i && !s.copy_selection;
                let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
                let fill = if sel {
                    p.violet
                } else if resp.hovered() {
                    p.raised
                } else {
                    p.field
                };
                ui.painter().rect_filled(rect, 3.0, fill);
                let col = if sel { Color32::WHITE } else { p.ink };
                let name = match i {
                    i if i < SHAPES.len() => {
                        let sh = &SHAPES[i];
                        super::object::shape_preview(ui, rect.shrink(3.0), &sh.params(), sh.rotation(), col);
                        sh.name
                    }
                    i if i == SHAPES.len() => {
                        tree_icon(ui, rect.shrink(4.0), col, true);
                        "Arbre"
                    }
                    _ => {
                        tree_icon(ui, rect.shrink(4.0), col, false);
                        "Herbe"
                    }
                };
                if resp.on_hover_text(name).clicked() {
                    // LLFloaterTools::setObjectType also unticks « Copier la sélection »
                    s.create_shape = i;
                    s.copy_selection = false;
                }
                if i % 5 == 4 {
                    ui.end_row();
                }
            }
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if let Some(v) = check(ui, s.keep_tool, false, "Maintenir l'outil sélect.", true) {
                s.keep_tool = v;
            }
            if let Some(v) = check(ui, s.copy_selection, false, "Copier la sélection", true) {
                s.copy_selection = v;
            }
            ui.indent("copy_opts", |ui| {
                if let Some(v) = check(ui, s.copy_centers, false, "Centrer", s.copy_selection) {
                    s.copy_centers = v;
                }
                if let Some(v) = check(ui, s.copy_rotates, false, "Pivoter", s.copy_selection) {
                    s.copy_rotates = v;
                }
            });
            // tree_grass_combo (FIRE-7802)
            let tree = s.create_shape == SHAPES.len();
            let grass = s.create_shape == SHAPES.len() + 1;
            let (list, current): (&[&str], &mut String) = if grass {
                (&GRASS_SPECIES, &mut s.last_grass)
            } else {
                (&TREE_SPECIES, &mut s.last_tree)
            };
            ui.add_enabled_ui(tree || grass, |ui| {
                egui::ComboBox::from_id_salt("tree_grass")
                    .width(120.0)
                    .selected_text(if current.is_empty() { "Random" } else { current.as_str() })
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_empty(), "Random").clicked() {
                            current.clear();
                        }
                        for n in list {
                            if ui.selectable_label(current == n, *n).clicked() {
                                *current = (*n).to_owned();
                            }
                        }
                    });
            });
        });
    });
}

/// A small tree / grass glyph for the last two shape buttons.
fn tree_icon(ui: &egui::Ui, r: egui::Rect, col: Color32, tree: bool) {
    let painter = ui.painter();
    let s = egui::Stroke::new(1.2, col);
    if tree {
        let top = egui::pos2(r.center().x, r.top());
        painter.line_segment([top, egui::pos2(r.left() + 2.0, r.bottom() - 5.0)], s);
        painter.line_segment([top, egui::pos2(r.right() - 2.0, r.bottom() - 5.0)], s);
        painter.line_segment(
            [
                egui::pos2(r.left() + 2.0, r.bottom() - 5.0),
                egui::pos2(r.right() - 2.0, r.bottom() - 5.0),
            ],
            s,
        );
        painter.line_segment(
            [egui::pos2(r.center().x, r.bottom() - 5.0), egui::pos2(r.center().x, r.bottom())],
            s,
        );
    } else {
        for k in 0..4 {
            let x = r.left() + 2.0 + k as f32 * (r.width() - 4.0) / 3.0;
            painter.line_segment([egui::pos2(x, r.bottom()), egui::pos2(x + 2.0 - k as f32, r.top() + 3.0)], s);
        }
    }
}

/// Land tool: brush radio, size / force, apply.
pub fn land_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.spacing_mut().item_spacing.y = 2.0;
        if radio(ui, s.land_action > land_action::REVERT, "Choisir le terrain") {
            s.land_action = 6;
        }
        for (i, n) in crate::build::land::ACTION_NAMES.iter().enumerate() {
            if radio(ui, s.land_action as usize == i, n) {
                s.land_action = i as u8;
            }
        }
        let ui = &mut cols[1];
        label(ui, p, "Bulldozer :");
        ui.horizontal(|ui| {
            label(ui, p, "Taille");
            ui.add(egui::Slider::new(&mut s.land_brush_size, 1.0..=11.0).show_value(false));
        });
        ui.horizontal(|ui| {
            label(ui, p, "Force");
            // logarithmic: -1..2 = 0.1..100 (LandBrushForce = 10^v)
            let mut lg = s.land_brush_force.clamp(0.1, 100.0).log10();
            if ui.add(egui::Slider::new(&mut lg, -1.0..=2.0).show_value(false)).changed() {
                s.land_brush_force = 10f32.powf(lg);
            }
        });
        let can_apply = s.land_action <= land_action::REVERT && tool.land.selection.is_some();
        if ui
            .add_enabled(can_apply, egui::Button::new(RichText::new("Appliquer").size(12.0)))
            .on_hover_text("Modifier le terrain sélectionné")
            .clicked()
        {
            let mut cmds = Vec::new();
            tool.land.apply_to_selection(world, s, &mut cmds);
            for c in cmds {
                tool.send(c);
            }
        }
    });
    if let Some(m) = tool.land.take_message() {
        tool.status = m;
    }
}

/// GP_LAND_DIVIDE_JOIN, GP_LAND_RELEASE (llagent / roles_constants.h).
const GP_LAND_DIVIDE_JOIN: u64 = 1 << 19;
const GP_LAND_RELEASE: u64 = 1 << 20;
/// REGION_FLAGS_ALLOW_PARCEL_CHANGES.
const REGION_FLAGS_ALLOW_PARCEL_CHANGES: u32 = 1 << 26;
/// PARCEL_UNIT_AREA (m²).
const PARCEL_UNIT_AREA: f32 = 16.0;

/// The land info panel (LLPanelLandInfo::refresh).
pub fn parcel_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    let parcel = tool.land.parcel.clone().map(|(_, pi)| pi).or_else(|| world.parcel.clone());
    let sel_area = tool.land.selection_area();
    super::common::section(ui, p, "Informations sur la parcelle");
    match parcel.as_ref() {
        Some(pi) => {
            let whole = tool.land.selection.is_none() || (sel_area - pi.area as f32).abs() < 1.0;
            let public = pi.owner_id.is_nil();
            let for_sale = pi.flags & aurora_net::parcel_flags::FOR_SALE != 0;
            if !public && for_sale && whole {
                label(ui, p, &format!("Prix : {} L$ pour {} m²", pi.sale_price, pi.area));
            } else {
                let area = if tool.land.selection.is_some() { sel_area as i32 } else { pi.area };
                label(ui, p, &format!("Surface : {area} m²"));
            }
        }
        None => {
            label(ui, p, "Aucune parcelle sélectionnée.");
        }
    }
    if ui
        .add_enabled(parcel.is_some(), egui::Button::new(RichText::new("À propos du terrain").size(12.0)))
        .clicked()
    {
        tool.ui.requests.push(Request::AboutLand);
    }
    if let Some(v) = check(ui, s.show_parcel_owners, false, "Afficher les propriétaires", true) {
        s.show_parcel_owners = v;
    }
    if s.show_parcel_owners {
        ui.horizontal_wrapped(|ui| {
            for (c, t) in [
                (Color32::from_rgb(0, 200, 0), "à vous"),
                (Color32::from_rgb(0, 200, 200), "groupe"),
                (Color32::from_rgb(220, 0, 0), "autres"),
                (Color32::from_rgb(230, 230, 0), "à vendre"),
                (Color32::from_rgb(170, 60, 220), "enchères"),
                (Color32::from_gray(140), "public"),
            ] {
                crate::ui::widgets::status_dot(ui, c);
                small(ui, p, t);
            }
        });
    }
    // who may do what (LLPanelLandInfo::refresh)
    let (mine, owner_divide, owner_release, can_buy) = match parcel.as_ref() {
        Some(pi) => {
            let group_power = |power: u64| pi.is_group_owned && world.groups.group(&pi.group_id).is_some_and(|g| g.powers & power != 0);
            let mine = pi.owner_id == world.agent_id;
            let for_sale = pi.flags & aurora_net::parcel_flags::FOR_SALE != 0;
            let can_buy = for_sale
                && !mine
                && (pi.sale_price > 0 || !pi.auth_buyer.is_nil())
                && (pi.auth_buyer.is_nil() || pi.auth_buyer == world.agent_id);
            (
                mine,
                mine || group_power(GP_LAND_DIVIDE_JOIN),
                mine || group_power(GP_LAND_RELEASE),
                can_buy || pi.owner_id.is_nil(),
            )
        }
        None => (false, false, false, false),
    };
    let region_allows = tool
        .land
        .selection_local(world)
        .and_then(|(h, ..)| world.regions.get(&h))
        .and_then(|r| r.info.as_ref())
        .is_none_or(|i| i.region_flags & REGION_FLAGS_ALLOW_PARCEL_CHANGES != 0);
    let whole = parcel.as_ref().is_some_and(|pi| (sel_area - pi.area as f32).abs() < 1.0);
    super::common::section(ui, p, "Modifier la parcelle");
    ui.horizontal(|ui| {
        let divide = owner_divide && region_allows && tool.land.selection.is_some() && !whole;
        if ui
            .add_enabled(divide, egui::Button::new(RichText::new("Sous-diviser").size(12.0)))
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Divide);
        }
        let join = sel_area > PARCEL_UNIT_AREA && !whole && owner_divide;
        if ui
            .add_enabled(join, egui::Button::new(RichText::new("Fusionner").size(12.0)))
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Join);
        }
    });
    super::common::section(ui, p, "Transactions");
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_buy && !mine, egui::Button::new(RichText::new("Acheter du terrain").size(12.0)))
            .clicked()
        {
            tool.ui.requests.push(Request::BuyLand);
        }
        if ui
            .add_enabled(
                owner_release && parcel.is_some(),
                egui::Button::new(RichText::new("Abandonner le terrain").size(12.0)),
            )
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Release);
        }
    });
}

/// "selection_count" / "more info label" (LLFloaterTools::refresh):
/// linksets, land impact, prims, remaining capacity.
pub fn selection_line(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World) {
    let roots = tool.roots(world).len();
    if roots == 0 {
        small(ui, p, "Aucune sélection effectuée.");
        return;
    }
    let prims = tool.sel_prims(world).len();
    let impact = tool
        .land_impact(world)
        .map(|v| format!("{}", v as i32))
        .unwrap_or_else(|| "…".into());
    small(
        ui,
        p,
        &format!("{roots} objets sélectionnés, impact sur le terrain {impact}, prims {prims}"),
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if let Some((_, remaining)) = tool.costs.capacity(world) {
            small(ui, p, &format!("Capacité restante {remaining}."));
        }
        if ui
            .add(egui::Label::new(RichText::new("Plus d'infos").size(11.0).color(p.violet_light).underline()).sense(egui::Sense::click()))
            .on_hover_text("Poids de la sélection et impact sur le terrain")
            .clicked()
        {
            tool.ui.weights_open = !tool.ui.weights_open;
        }
    });
}

impl BuildTool {
    /// LLFloaterTools::setTool.
    pub fn set_tool(&mut self, t: Tool) {
        self.manip.cancel();
        self.land.cancel();
        self.tool = t;
        // LLToolGrab::handleSelect resets the radio
        if t == Tool::Grab {
            self.grab_mode = GrabMode::Move;
        }
    }

    /// The edit radio: face mode selects prims individually, align whole
    /// linksets (LLToolFace::handleSelect / QToolAlign::handleSelect).
    pub fn set_edit_mode(&mut self, world: &mut World, s: &mut BuildSettings, m: EditMode) {
        let was_face = self.edit_mode == EditMode::Face;
        self.edit_mode = m;
        if was_face && m != EditMode::Face {
            self.faces.clear();
        }
        if m == EditMode::Align && s.edit_linked {
            self.set_edit_linked(world, s, false);
        }
    }

    /// "link_num_obj_count": « Numéro de lien : N » with « Modification
    /// liée », « Faces : … » with the face tool, one prim selected.
    pub fn part_info(&self, world: &World, s: &BuildSettings) -> Option<String> {
        let prims = self.sel_prims(world);
        if prims.len() != 1 {
            return None;
        }
        let idx = prims[0];
        let o = world.objects.get(idx)?;
        if self.tool == Tool::Edit && self.edit_mode == EditMode::Face {
            let faces = self.faces.get(&o.key);
            let list = match faces {
                Some(f) if !f.is_empty() && f.len() < self.num_faces(world, idx) => {
                    f.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")
                }
                _ => "toutes".into(),
            };
            return Some(format!("Faces : {list}"));
        }
        if s.edit_linked {
            let n = if o.parent_id == 0 {
                if world.objects.children_of(&o.key).is_empty() { 0 } else { 1 }
            } else {
                let root = crate::build::root_of(world, idx);
                crate::build::family(world, root)
                    .iter()
                    .position(|&i| i == idx)
                    .map(|i| i + 1)
                    .unwrap_or(0)
            };
            return Some(format!("Numéro de lien : {n}"));
        }
        None
    }
}
