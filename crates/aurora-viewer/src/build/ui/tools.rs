//! Top of the build floater: the five tool buttons, the status line and
//! each tool's options (floater_tools.xml, LLFloaterTools::updatePopup),
//! the land tool's parcel section (LLPanelLandInfo) and the selection line
//! (LLFloaterTools::refresh).

use super::common::{check, label, small};
use super::{Confirm, Env, Request};
use crate::build::shapes::{grass_species, shapes, tree_species};
use crate::build::{BuildSettings, BuildTool, EditMode, FocusMode, GrabMode, Mods, Tool};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::build::land_action;
use egui::{Color32, RichText, Vec2};

/// "text status": the hint of the current tool (floater.string status_*).
pub fn status_text(tool: &BuildTool, s: &BuildSettings, mods: Mods) -> &'static str {
    match tool.tool {
        Tool::Focus => aurora_i18n::tr!("build-ui-tools-click-and-drag-to-move-the-camera"),
        Tool::Grab => aurora_i18n::tr!("build-ui-tools-drag-to-move-ctrl-to-lift-ctrl-shift-to"),
        Tool::Edit => match tool.effective_mode(mods) {
            EditMode::Rotate => aurora_i18n::tr!("build-ui-tools-drag-the-colored-bands-to-rotate-the-object"),
            EditMode::Stretch => aurora_i18n::tr!("build-ui-tools-click-and-drag-to-stretch-the-selected-side"),
            EditMode::Face => aurora_i18n::tr!("build-ui-tools-click-a-face-to-select-it-shift-add"),
            EditMode::Align => aurora_i18n::tr!("build-ui-tools-click-a-cone-to-align-shift-stack"),
            EditMode::Move if tool.cam.hud.is_some() => aurora_i18n::tr!("build-ui-tools-drag-an-arrow-or-the-plane-handle-to-move"),
            EditMode::Move => aurora_i18n::tr!("build-ui-tools-drag-to-move-shift-drag-to-copy"),
        },
        Tool::Create => aurora_i18n::tr!("build-ui-tools-click-in-the-world-to-build"),
        Tool::Land => {
            if s.land_action > land_action::REVERT {
                aurora_i18n::tr!("build-ui-tools-click-and-drag-to-select-land")
            } else {
                aurora_i18n::tr!("build-ui-tools-click-and-hold-to-edit-land")
            }
        }
    }
}

fn radio(ui: &mut egui::Ui, p: &Palette, on: bool, text: &str) -> bool {
    crate::ui::widgets::radio(ui, p, on, RichText::new(text).size(12.0)).clicked()
}

/// Focus tool (focus_radio_group, slider zoom).
pub fn focus_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, mods: Mods, env: &mut Env) {
    let shown = match (mods.ctrl, mods.shift) {
        (true, true) => FocusMode::Pan,
        (true, false) => FocusMode::Orbit,
        _ => tool.focus_mode,
    };
    ui.horizontal(|ui| {
        if radio(ui, p, shown == FocusMode::Zoom, aurora_i18n::tr!("common-zoom-2")) {
            tool.focus_mode = FocusMode::Zoom;
        }
        // slider zoom: getCameraZoomFraction() * 0.5, 0..0.5
        let mut v = env.zoom_fraction * 0.5;
        let r = ui.add_enabled(
            tool.focus_mode == FocusMode::Zoom,
            egui::Slider::new(&mut v, 0.0..=0.5)
                .custom_parser(aurora_i18n::parse_number)
                .step_by(0.01)
                .show_value(false),
        );
        if r.changed() {
            tool.ui.requests.push(Request::ZoomFraction(v * 2.0));
        }
    });
    if radio(
        ui,
        p,
        shown == FocusMode::Orbit,
        aurora_i18n::tr!("build-ui-tools-orbit-camera-ctrl"),
    ) {
        tool.focus_mode = FocusMode::Orbit;
    }
    if radio(
        ui,
        p,
        shown == FocusMode::Pan,
        aurora_i18n::tr!("build-ui-tools-pan-camera-ctrl-shift"),
    ) {
        tool.focus_mode = FocusMode::Pan;
    }
}

/// Grab tool (move_radio_group).
pub fn grab_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, mods: Mods) {
    let shown = tool.effective_grab_mode(mods);
    if radio(ui, p, shown == GrabMode::Move, aurora_i18n::tr!("build-ui-tools-move")) {
        tool.grab_mode = GrabMode::Move;
    }
    if radio(ui, p, shown == GrabMode::Lift, aurora_i18n::tr!("build-ui-tools-lift-ctrl")) {
        tool.grab_mode = GrabMode::Lift;
    }
    if radio(ui, p, shown == GrabMode::Spin, aurora_i18n::tr!("build-ui-tools-rotate-ctrl-shift")) {
        tool.grab_mode = GrabMode::Spin;
    }
}

/// Create tool: the 15 shapes, options, tree / grass species.
pub fn create_panel(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    ui.vertical(|ui| {
        let cell = Vec2::splat((ui.available_width() - 24.0) / 5.0);
        egui::Grid::new("shapes")
            .min_col_width(cell.x)
            .spacing(Vec2::splat(6.0))
            .show(ui, |ui| {
                for i in 0..shapes().len() + 2 {
                    let sel = s.create_shape == i && !s.copy_selection;
                    let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
                    let fill = if sel {
                        p.violet
                    } else if resp.hovered() {
                        p.raised
                    } else {
                        p.field
                    };
                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
                    let col = if sel { p.on_accent } else { p.ink };
                    let name = match i {
                        i if i < shapes().len() => {
                            let sh = &shapes()[i];
                            super::object::shape_preview(ui, rect.shrink(3.0), &sh.params(), sh.rotation(), col);
                            sh.name
                        }
                        i if i == shapes().len() => {
                            tree_icon(ui, rect.shrink(4.0), col, true);
                            aurora_i18n::tr!("build-ui-tools-tree")
                        }
                        _ => {
                            tree_icon(ui, rect.shrink(4.0), col, false);
                            aurora_i18n::tr!("build-ui-tools-grass")
                        }
                    };
                    if resp.on_hover_text(name).clicked() {
                        // LLFloaterTools::setObjectType also unticks "Copy selection"
                        s.create_shape = i;
                        s.copy_selection = false;
                    }
                    if i % 5 == 4 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            if let Some(v) = check(
                ui,
                p,
                s.keep_tool,
                false,
                aurora_i18n::tr!("build-ui-tools-keep-selection-tool"),
                true,
            ) {
                s.keep_tool = v;
            }
            if let Some(v) = check(
                ui,
                p,
                s.copy_selection,
                false,
                aurora_i18n::tr!("build-ui-tools-copy-selection"),
                true,
            ) {
                s.copy_selection = v;
            }
            ui.indent("copy_opts", |ui| {
                if let Some(v) = check(
                    ui,
                    p,
                    s.copy_centers,
                    false,
                    aurora_i18n::tr!("build-ui-tools-center"),
                    s.copy_selection,
                ) {
                    s.copy_centers = v;
                }
                if let Some(v) = check(ui, p, s.copy_rotates, false, aurora_i18n::tr!("common-rotate"), s.copy_selection) {
                    s.copy_rotates = v;
                }
            });
            // tree_grass_combo (FIRE-7802)
            let tree = s.create_shape == shapes().len();
            let grass = s.create_shape == shapes().len() + 1;
            let (list, current): (&[&str], &mut String) = if grass {
                (&grass_species(), &mut s.last_grass)
            } else {
                (&tree_species(), &mut s.last_tree)
            };
            ui.add_enabled_ui(tree || grass, |ui| {
                egui::ComboBox::from_id_salt("tree_grass")
                    .width(120.0)
                    .selected_text(if current.is_empty() {
                        aurora_i18n::tr!("build-ui-tools-random")
                    } else {
                        current.as_str()
                    })
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(current.is_empty(), aurora_i18n::tr!("build-ui-tools-random"))
                            .clicked()
                        {
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
        if radio(
            ui,
            p,
            s.land_action > land_action::REVERT,
            aurora_i18n::tr!("build-ui-tools-select-land"),
        ) {
            s.land_action = 6;
        }
        for (i, n) in crate::build::land::action_names().iter().enumerate() {
            if radio(ui, p, s.land_action as usize == i, n) {
                s.land_action = i as u8;
            }
        }
        let ui = &mut cols[1];
        label(ui, p, aurora_i18n::tr!("build-ui-tools-bulldozer"));
        ui.horizontal(|ui| {
            label(ui, p, aurora_i18n::tr!("build-ui-tools-size"));
            ui.add(
                egui::Slider::new(&mut s.land_brush_size, 1.0..=11.0)
                    .custom_parser(aurora_i18n::parse_number)
                    .show_value(false),
            );
        });
        ui.horizontal(|ui| {
            label(ui, p, aurora_i18n::tr!("build-ui-tools-force"));
            // logarithmic: -1..2 = 0.1..100 (LandBrushForce = 10^v)
            let mut lg = s.land_brush_force.clamp(0.1, 100.0).log10();
            if ui
                .add(
                    egui::Slider::new(&mut lg, -1.0..=2.0)
                        .custom_parser(aurora_i18n::parse_number)
                        .show_value(false),
                )
                .changed()
            {
                s.land_brush_force = 10f32.powf(lg);
            }
        });
        let can_apply = s.land_action <= land_action::REVERT && tool.land.selection.is_some();
        if ui
            .add_enabled(
                can_apply,
                egui::Button::new(RichText::new(aurora_i18n::tr!("common-apply")).size(12.0)),
            )
            .on_hover_text(aurora_i18n::tr!("build-ui-tools-edit-selected-land"))
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
pub fn parcel_panel(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    property_lines: &mut bool,
) {
    let parcel = tool.land.parcel.clone().map(|(_, pi)| pi).or_else(|| world.parcel.clone());
    let sel_area = tool.land.selection_area();
    super::common::section(ui, p, aurora_i18n::tr!("build-ui-tools-parcel-information"));
    match parcel.as_ref() {
        Some(pi) => {
            let whole = tool.land.selection.is_none() || (sel_area - pi.area as f32).abs() < 1.0;
            let public = pi.owner_id.is_nil();
            let for_sale = pi.flags & aurora_net::parcel_flags::FOR_SALE != 0;
            if !public && for_sale && whole {
                label(
                    ui,
                    p,
                    &aurora_i18n::tr!(
                        "build-ui-tools-price-l-for-m2",
                        value0 = format!("{}", pi.sale_price),
                        value1 = format!("{}", pi.area)
                    ),
                );
            } else {
                let area = if tool.land.selection.is_some() { sel_area as i32 } else { pi.area };
                label(ui, p, &aurora_i18n::tr!("build-ui-tools-area-m2", area = format!("{}", area)));
            }
        }
        None => {
            label(ui, p, aurora_i18n::tr!("common-no-parcel-selected"));
        }
    }
    if ui
        .add_enabled(
            parcel.is_some(),
            egui::Button::new(RichText::new(aurora_i18n::tr!("common-about-land")).size(12.0)),
        )
        .clicked()
    {
        tool.ui.requests.push(Request::AboutLand);
    }
    if let Some(v) = check(ui, p, *property_lines, false, aurora_i18n::tr!("common-show-boundaries"), true) {
        *property_lines = v;
    }
    if let Some(v) = check(
        ui,
        p,
        s.show_parcel_owners,
        false,
        aurora_i18n::tr!("build-ui-tools-show-owners"),
        true,
    ) {
        s.show_parcel_owners = v;
    }
    if s.show_parcel_owners {
        ui.horizontal_wrapped(|ui| {
            for (c, t) in [
                (p.success, aurora_i18n::tr!("build-ui-tools-yours")),
                (p.teal, aurora_i18n::tr!("build-ui-tools-owner-group")),
                (p.danger, aurora_i18n::tr!("build-ui-tools-owner-others")),
                (p.amber, aurora_i18n::tr!("build-ui-tools-for-sale")),
                (p.violet_light, aurora_i18n::tr!("build-ui-tools-auctions")),
                (p.muted, aurora_i18n::tr!("build-ui-tools-owner-public")),
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
    super::common::section(ui, p, aurora_i18n::tr!("build-ui-tools-edit-parcel"));
    ui.horizontal(|ui| {
        let divide = owner_divide && region_allows && tool.land.selection.is_some() && !whole;
        if ui
            .add_enabled(
                divide,
                egui::Button::new(RichText::new(aurora_i18n::tr!("common-subdivide")).size(12.0)),
            )
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Divide);
        }
        let join = sel_area > PARCEL_UNIT_AREA && !whole && owner_divide;
        if ui
            .add_enabled(join, egui::Button::new(RichText::new(aurora_i18n::tr!("common-join")).size(12.0)))
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Join);
        }
    });
    super::common::section(ui, p, aurora_i18n::tr!("build-ui-tools-transactions"));
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                can_buy && !mine,
                egui::Button::new(RichText::new(aurora_i18n::tr!("build-ui-tools-buy-land")).size(12.0)),
            )
            .clicked()
        {
            tool.ui.requests.push(Request::BuyLand);
        }
        if ui
            .add_enabled(
                owner_release && parcel.is_some(),
                egui::Button::new(RichText::new(aurora_i18n::tr!("common-abandon-land")).size(12.0)),
            )
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Release);
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

    /// Numeric footer indicators refer to the primary (latest selected) prim.
    pub fn selected_link_number(&self, world: &World) -> Option<usize> {
        let idx = world.objects.index_of(self.selection.first()?)?;
        let o = world.objects.get(idx)?;
        Some(if crate::build::root_of(world, idx) == idx {
            if world.objects.children_of(&o.key).is_empty() { 0 } else { 1 }
        } else {
            let root = crate::build::root_of(world, idx);
            crate::build::family(world, root)
                .iter()
                .position(|&i| i == idx)
                .map(|i| i + 1)
                .unwrap_or(0)
        })
    }

    pub fn selected_face_numbers(&self, world: &World) -> String {
        let Some(idx) = self.selection.first().and_then(|key| world.objects.index_of(key)) else {
            return "—".into();
        };
        let Some(o) = world.objects.get(idx) else { return "—".into() };
        let mut faces: Vec<usize> = match self.faces.get(&o.key).filter(|f| !f.is_empty()) {
            Some(f) => f.iter().map(|&n| n as usize).collect(),
            None => (0..self.num_faces(world, idx)).collect(),
        };
        faces.sort_unstable();
        faces.dedup();
        faces.into_iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
    }
}
