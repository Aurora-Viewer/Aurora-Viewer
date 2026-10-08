//! The build floater (LLFloaterTools, floater_tools.xml) in the Aurora skin:
//! Edit / Create / Land tools, edit options and grid, the General and
//! Object tabs, the Create shape palette and the land brush. Also the
//! build keyboard shortcuts (menu_viewer.xml).

use super::land::ACTION_NAMES;
use super::shapes::SHAPES;
use super::{BuildSettings, BuildTool, EditMode, Tool};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::ui::widgets::{Floater, flat_button, tabs};
use crate::world::World;
use aurora_net::build::{BuildCmd, land_action, perm};
use aurora_prim::params::*;
use egui::{Color32, RichText, Vec2};
use glam::{EulerRot, Quat, Vec3};
use std::time::Instant;

/// update_flags bits (llprimitive.h).
const FLAGS_USE_PHYSICS: u32 = 1 << 0;
const FLAGS_PHANTOM: u32 = 1 << 10;
const FLAGS_TEMPORARY_ON_REZ: u32 = 1 << 29;
/// OBJECT_TWIST_LINEAR_MAX / OBJECT_TWIST_MAX (degrees).
const TWIST_LINEAR_MAX: f32 = 180.0;
const TWIST_MAX: f32 = 360.0;

fn label(ui: &mut egui::Ui, p: &Palette, t: &str) {
    ui.label(RichText::new(t).size(12.0).color(p.muted));
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).size(10.5).color(p.violet_light));
    ui.add_space(2.0);
}

fn tool_button(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str, on: bool, tip: &str) -> bool {
    let fill = if on { p.violet } else { p.raised };
    let mut b = egui::Button::new(RichText::new(text).size(12.0).color(if on { Color32::WHITE } else { p.ink }))
        .fill(fill)
        .corner_radius(2)
        .min_size(Vec2::new(0.0, 24.0));
    if let Some(t) = crate::ui::icons::global(icon) {
        b = egui::Button::image_and_text(
            egui::Image::new(&t)
                .fit_to_exact_size(Vec2::splat(15.0))
                .tint(if on { Color32::WHITE } else { p.ink }),
            RichText::new(text).size(12.0).color(if on { Color32::WHITE } else { p.ink }),
        )
        .fill(fill)
        .corner_radius(2)
        .min_size(Vec2::new(0.0, 24.0));
    }
    ui.add(b).on_hover_text(tip).clicked()
}

fn choice(ui: &mut egui::Ui, p: &Palette, on: bool, text: &str, tip: &str) -> bool {
    ui.add(egui::Button::selectable(
        on,
        RichText::new(text).size(12.0).color(if on { p.ink } else { p.muted }),
    ))
    .on_hover_text(tip)
    .clicked()
}

/// A labeled row of three numbers; returns (index, value, commit) of the
/// one changed (commit when typed or when the drag ends).
fn vec_row(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    v: &mut [f32; 3],
    speed: f32,
    decimals: usize,
    range: std::ops::RangeInclusive<f32>,
) -> (bool, bool) {
    let mut changed = false;
    let mut commit = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(62.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            label(ui, p, name)
        });
        let colors = [
            Color32::from_rgb(240, 130, 130),
            Color32::from_rgb(130, 220, 130),
            Color32::from_rgb(130, 160, 245),
        ];
        for (i, x) in v.iter_mut().enumerate() {
            ui.label(RichText::new(["X", "Y", "Z"][i]).size(11.0).color(colors[i]));
            let r = ui.add_sized(
                [68.0, 20.0],
                egui::DragValue::new(x)
                    .speed(speed)
                    .range(range.clone())
                    .max_decimals(decimals)
                    .min_decimals(decimals.min(3)),
            );
            if r.changed() {
                changed = true;
                if !r.dragged() {
                    commit = true;
                }
            }
            if r.drag_stopped() || r.lost_focus() {
                commit = true;
            }
        }
    });
    (changed, commit)
}

fn number(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    v: &mut f32,
    speed: f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(118.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            label(ui, p, name)
        });
        let r = ui.add_sized(
            [80.0, 20.0],
            egui::DragValue::new(v).speed(speed).range(range).max_decimals(3).suffix(suffix),
        );
        out = (r.changed(), (r.changed() && !r.dragged()) || r.drag_stopped() || r.lost_focus());
    });
    out
}

/// Small isometric drawing of a shape's outline (Create palette).
fn shape_preview(ui: &egui::Ui, rect: egui::Rect, params: &VolumeParams, rot: Quat, color: Color32) {
    let edges = super::silhouette::preview_edges(params);
    let view = Quat::from_rotation_x(-0.6) * Quat::from_rotation_z(0.75) * rot;
    let s = rect.width().min(rect.height()) * 0.62;
    let proj = |v: Vec3| {
        let q = view * v;
        egui::pos2(rect.center().x + q.x * s, rect.center().y - q.z * s)
    };
    for (a, b) in edges.iter() {
        ui.painter().line_segment([proj(*a), proj(*b)], egui::Stroke::new(1.0, color));
    }
}

/// Build shortcuts (Ctrl+B, tools, delete, duplicate, link...).
pub fn shortcuts(ctx: &egui::Context, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    use egui::Key;
    let (pressed, m) = ctx.input(|i| (i.events.clone(), i.modifiers));
    for ev in pressed {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            ..
        } = ev
        else {
            continue;
        };
        let (ctrl, shift) = (m.command || m.ctrl, m.shift);
        match (key, ctrl, shift) {
            (Key::B, true, false) => {
                if tool.open {
                    tool.close(world, s);
                } else {
                    tool.open_build(if tool.selection.is_empty() { Tool::Create } else { Tool::Edit });
                }
            }
            (Key::Num3, true, false) => tool.open_build(Tool::Edit),
            (Key::Num4, true, false) => tool.open_build(Tool::Create),
            (Key::Num5, true, false) => tool.open_build(Tool::Land),
            _ if !tool.open => {}
            (Key::Escape, false, false) => tool.close(world, s),
            (Key::Delete, false, false) => tool.delete(world),
            (Key::D, true, false) => tool.duplicate(world, Vec3::new(0.5, 0.5, 0.0), true),
            (Key::L, true, false) => tool.link(world),
            (Key::L, true, true) => tool.unlink(world),
            (Key::E, true, true) => {
                let on = !s.edit_linked;
                tool.set_edit_linked(world, s, on);
            }
            (Key::G, false, false) => s.snap = !s.snap,
            (Key::Z, true, false) if tool.tool == Tool::Land => {
                let mut cmds = Vec::new();
                tool.land.undo(&mut cmds);
                for c in cmds {
                    tool.send(c);
                }
            }
            (Key::Z, true, false) => tool.undo(world, false),
            (Key::Y, true, false) => tool.undo(world, true),
            _ => {}
        }
    }
}

/// The floater. Returns true when a setting changed (to save them).
pub fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: super::Mods) -> bool {
    if !tool.open {
        return false;
    }
    let before = s.clone();
    let screen = ctx.content_rect();
    let mut open = true;
    Floater::new(
        "build_tools",
        "Construire",
        egui::pos2(screen.right() - 360.0, 80.0),
        Vec2::new(340.0, 600.0),
    )
    .help("Outils de construction : Ctrl+B ouvre / ferme, Ctrl+3 / 4 / 5 : modifier / créer / terrain")
    .show(ctx, p, &mut open, |ui| {
        ui.horizontal(|ui| {
            if tool_button(
                ui,
                p,
                "arrows-out-cardinal",
                "Modifier",
                tool.tool == Tool::Edit,
                "Sélectionner et modifier des objets (Ctrl+3)",
            ) {
                tool.tool = Tool::Edit;
            }
            if tool_button(ui, p, "cube", "Créer", tool.tool == Tool::Create, "Créer des prims (Ctrl+4)") {
                tool.tool = Tool::Create;
            }
            if tool_button(
                ui,
                p,
                "globe",
                "Terrain",
                tool.tool == Tool::Land,
                "Sélectionner et modifier le terrain (Ctrl+5)",
            ) {
                tool.tool = Tool::Land;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match tool.tool {
                Tool::Edit => edit_panel(ui, p, tool, world, s, mods),
                Tool::Create => create_panel(ui, p, s),
                Tool::Land => land_panel(ui, p, tool, world, s),
            });
        if !tool.status.is_empty() {
            ui.separator();
            ui.label(RichText::new(&tool.status).size(11.5).color(p.muted));
        }
    });
    if !open {
        tool.close(world, s);
    }
    *s != before
}

fn edit_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: super::Mods) {
    let mode = tool.effective_mode(mods);
    ui.horizontal(|ui| {
        if choice(ui, p, mode == EditMode::Move, "Déplacer", "Flèches et plans de déplacement") {
            tool.edit_mode = EditMode::Move;
        }
        if choice(ui, p, mode == EditMode::Rotate, "Pivoter", "Anneaux de rotation (Ctrl)") {
            tool.edit_mode = EditMode::Rotate;
        }
        if choice(ui, p, mode == EditMode::Stretch, "Étirer", "Poignées d'étirement (Ctrl+Maj)") {
            tool.edit_mode = EditMode::Stretch;
        }
    });
    let mut linked = s.edit_linked;
    if ui
        .checkbox(&mut linked, RichText::new("Modifier les parties liées").size(12.0))
        .on_hover_text("Sélectionner les prims un par un (Ctrl+Maj+E)")
        .changed()
    {
        tool.set_edit_linked(world, s, linked);
    }
    ui.checkbox(&mut s.scale_uniform, RichText::new("Étirer des deux côtés").size(12.0));
    ui.horizontal(|ui| {
        ui.checkbox(&mut s.snap, RichText::new("Grille").size(12.0))
            .on_hover_text("Accrocher à la grille (G)");
        egui::ComboBox::from_id_salt("grid_mode")
            .width(90.0)
            .selected_text(if s.grid_mode == 1 { "Locale" } else { "Monde" })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut s.grid_mode, 0, "Monde");
                ui.selectable_value(&mut s.grid_mode, 1, "Locale");
            });
        ui.checkbox(&mut s.show_highlight, RichText::new("Surbrillance").size(12.0));
    });
    egui::CollapsingHeader::new(RichText::new("Options de la grille").size(12.0))
        .id_salt("grid_opts")
        .show(ui, |ui| {
            number(ui, p, "Unité (m)", &mut s.grid_resolution, 0.01, 0.01..=5.0, "");
            number(ui, p, "Étendue (m)", &mut s.grid_draw_size, 0.5, 1.0..=50.0, "");
            number(ui, p, "Opacité", &mut s.grid_opacity, 0.01, 0.0..=1.0, "");
            number(ui, p, "Pas de rotation", &mut s.rotation_step, 0.1, 0.01..=90.0, "°");
            ui.checkbox(&mut s.grid_sub_unit, RichText::new("Accrocher aux sous-unités").size(12.0));
        });

    // actions
    ui.horizontal_wrapped(|ui| {
        let has = !tool.selection.is_empty();
        ui.add_enabled_ui(has, |ui| {
            if flat_button(ui, p, "Lier").on_hover_text("Ctrl+L").clicked() {
                tool.link(world);
            }
            if flat_button(ui, p, "Délier").on_hover_text("Ctrl+Maj+L").clicked() {
                tool.unlink(world);
            }
            if flat_button(ui, p, "Dupliquer").on_hover_text("Ctrl+D").clicked() {
                tool.duplicate(world, Vec3::new(0.5, 0.5, 0.0), true);
            }
            if flat_button(ui, p, "Supprimer").on_hover_text("Suppr").clicked() {
                tool.delete(world);
            }
            if flat_button(ui, p, "Prendre").clicked() {
                tool.take(world, false);
            }
            if flat_button(ui, p, "Prendre une copie").clicked() {
                tool.take(world, true);
            }
            if flat_button(ui, p, "Annuler").on_hover_text("Ctrl+Z").clicked() {
                tool.undo(world, false);
            }
            if flat_button(ui, p, "Rétablir").on_hover_text("Ctrl+Y").clicked() {
                tool.undo(world, true);
            }
        });
    });

    let prims = tool.highlighted(world).len();
    let units = tool.selection.len();
    ui.label(
        RichText::new(if units == 0 {
            "Cliquez sur un objet pour le sélectionner (Maj / Ctrl+clic : ajouter, glisser : rectangle)".to_owned()
        } else {
            format!("{units} objet(s) sélectionné(s), {prims} prim(s)")
        })
        .size(11.5)
        .color(p.muted),
    );
    if units == 0 {
        return;
    }
    ui.add_space(4.0);
    tabs(ui, p, &mut tool.tab, &[("Général", true), ("Objet", true)]);
    ui.add_space(4.0);
    if tool.tab == 0 {
        general_tab(ui, p, tool, world);
    } else {
        object_tab(ui, p, tool, world, s);
    }
}

fn general_tab(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let Some(id) = world.objects.get(idx).map(|o| o.full_id) else {
        return;
    };
    let Some(props) = tool.props.get(&id).cloned() else {
        label(ui, p, "Propriétés en attente du simulateur…");
        return;
    };
    let can_modify = props.owner_id == world.agent_id && props.owner_mask & perm::MODIFY != 0;
    let name_id = egui::Id::new(("build_name", id));
    let desc_id = egui::Id::new(("build_desc", id));
    let mut name = ui.data_mut(|d| d.get_temp::<String>(name_id)).unwrap_or_else(|| props.name.clone());
    let mut desc = ui
        .data_mut(|d| d.get_temp::<String>(desc_id))
        .unwrap_or_else(|| props.description.clone());
    section(ui, p, "Objet");
    ui.horizontal(|ui| {
        label(ui, p, "Nom");
        let r = ui.add_enabled(
            can_modify,
            egui::TextEdit::singleline(&mut name)
                .desired_width(ui.available_width())
                .char_limit(63),
        );
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(name_id, name.clone()));
        }
        if r.lost_focus() && name != props.name && !name.trim().is_empty() {
            tool.send(BuildCmd::SetName {
                handle: key.region,
                local_id: key.local_id,
                name: name.clone(),
            });
            if let Some(pr) = tool.props.get_mut(&id) {
                pr.name = name.clone();
            }
            ui.data_mut(|d| d.remove::<String>(name_id));
        }
    });
    ui.horizontal(|ui| {
        label(ui, p, "Description");
        let r = ui.add_enabled(
            can_modify,
            egui::TextEdit::singleline(&mut desc)
                .desired_width(ui.available_width())
                .char_limit(127),
        );
        if r.changed() {
            ui.data_mut(|d| d.insert_temp(desc_id, desc.clone()));
        }
        if r.lost_focus() && desc != props.description {
            tool.send(BuildCmd::SetDescription {
                handle: key.region,
                local_id: key.local_id,
                description: desc.clone(),
            });
            if let Some(pr) = tool.props.get_mut(&id) {
                pr.description = desc.clone();
            }
            ui.data_mut(|d| d.remove::<String>(desc_id));
        }
    });
    for nid in [props.creator_id, props.owner_id, props.last_owner_id] {
        if !nid.is_nil() {
            world.social.want_name(nid);
        }
    }
    let row = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(110.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                label(ui, p, k)
            });
            ui.label(RichText::new(v).size(12.0).color(p.ink));
        });
    };
    if props.full {
        row(ui, "Créateur", world.social.name_of(&props.creator_id));
    }
    row(
        ui,
        "Propriétaire",
        if props.group_id == props.owner_id && !props.group_id.is_nil() {
            "Groupe".into()
        } else {
            world.social.name_of(&props.owner_id)
        },
    );
    if !props.last_owner_id.is_nil() && props.last_owner_id != props.owner_id {
        row(ui, "Dernier propriétaire", world.social.name_of(&props.last_owner_id));
    }
    let perms = |m: u32| {
        let mut v = Vec::new();
        if m & perm::MODIFY != 0 {
            v.push("modifier");
        }
        if m & perm::COPY != 0 {
            v.push("copier");
        }
        if m & perm::TRANSFER != 0 {
            v.push("transférer");
        }
        if v.is_empty() { "aucun".to_owned() } else { v.join(", ") }
    };
    section(ui, p, "Droits");
    row(ui, "Vous pouvez", perms(props.owner_mask));
    row(ui, "Tout le monde", perms(props.everyone_mask));
    row(ui, "Prochain propriétaire", perms(props.next_owner_mask));
    if props.sale_type != 0 {
        row(ui, "À vendre", format!("L$ {}", props.sale_price));
    }
}

/// LLPanelObject base types (MI_BOX ...).
const BASE_TYPES: [(&str, u8, u8); 7] = [
    ("Boîte", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_LINE),
    ("Cylindre", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_LINE),
    ("Prisme", LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PATH_LINE),
    ("Sphère", LL_PCODE_PROFILE_CIRCLE_HALF, LL_PCODE_PATH_CIRCLE),
    ("Tore", LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PATH_CIRCLE),
    ("Tube", LL_PCODE_PROFILE_SQUARE, LL_PCODE_PATH_CIRCLE),
    ("Anneau", LL_PCODE_PROFILE_EQUALTRI, LL_PCODE_PATH_CIRCLE),
];

const HOLE_TYPES: [(&str, u8); 4] = [
    ("Par défaut", LL_PCODE_HOLE_SAME),
    ("Cercle", LL_PCODE_HOLE_CIRCLE),
    ("Carré", LL_PCODE_HOLE_SQUARE),
    ("Triangle", LL_PCODE_HOLE_TRIANGLE),
];

fn base_type(v: &VolumeParams) -> Option<usize> {
    let prof = v.profile.curve_type & LL_PCODE_PROFILE_MASK;
    let prof = if prof == LL_PCODE_PROFILE_ISOTRI || prof == LL_PCODE_PROFILE_RIGHTTRI {
        LL_PCODE_PROFILE_EQUALTRI
    } else {
        prof
    };
    let path = if v.path.curve_type == LL_PCODE_PATH_FLEXIBLE {
        LL_PCODE_PATH_LINE
    } else {
        v.path.curve_type
    };
    BASE_TYPES.iter().position(|(_, pr, pa)| *pr == prof && *pa == path)
}

fn object_tab(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let now = Instant::now();
    let Some((wp, wr, _)) = Scene::object_transform(world, idx, now, 0) else {
        return;
    };
    let Some(off) = world.region_offset(key.region) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let (flags, scale, volume, own) = (o.update_flags, o.scale, o.volume, o.owner_id == world.agent_id);
    let is_root = o.parent_id == 0;

    section(ui, p, "Placement");
    let mut pos = (wp - off).to_array();
    let (ch, commit) = vec_row(ui, p, "Position", &mut pos, 0.01, 3, -256.0..=4096.0);
    if ch {
        tool.set_transform(world, s, Some(Vec3::from_array(pos)), None, None, commit);
    }
    let mut size = scale.to_array();
    let (ch, commit) = vec_row(ui, p, "Taille", &mut size, 0.005, 3, super::MIN_PRIM_SCALE..=super::MAX_PRIM_SCALE);
    if ch {
        tool.set_transform(world, s, None, Some(Vec3::from_array(size)), None, commit);
    }
    let (z, y, x) = wr.to_euler(EulerRot::ZYX);
    let mut rot = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
    let (ch, commit) = vec_row(ui, p, "Rotation", &mut rot, 0.5, 2, -360.0..=360.0);
    if ch {
        let q = Quat::from_euler(EulerRot::ZYX, rot[2].to_radians(), rot[1].to_radians(), rot[0].to_radians());
        tool.set_transform(world, s, None, None, Some(q), commit);
    }

    if is_root && own {
        section(ui, p, "Comportement");
        let mut physics = flags & FLAGS_USE_PHYSICS != 0;
        let mut temporary = flags & FLAGS_TEMPORARY_ON_REZ != 0;
        let mut phantom = flags & FLAGS_PHANTOM != 0;
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut physics, RichText::new("Physique").size(12.0)).changed();
            changed |= ui.checkbox(&mut temporary, RichText::new("Temporaire").size(12.0)).changed();
            changed |= ui.checkbox(&mut phantom, RichText::new("Fantôme").size(12.0)).changed();
        });
        if changed {
            tool.send(BuildCmd::SetFlags {
                handle: key.region,
                local_id: key.local_id,
                physics,
                temporary,
                phantom,
            });
            if let Some(o) = world.objects.get_mut(idx) {
                let set = |f: u32, bit: u32, on: bool| if on { f | bit } else { f & !bit };
                o.update_flags = set(
                    set(set(o.update_flags, FLAGS_USE_PHYSICS, physics), FLAGS_TEMPORARY_ON_REZ, temporary),
                    FLAGS_PHANTOM,
                    phantom,
                );
            }
        }
    }

    if volume.sculpt.is_some() {
        section(ui, p, "Forme");
        label(
            ui,
            p,
            if volume.is_mesh() {
                "Mesh : forme non modifiable ici."
            } else {
                "Sculptie : forme non modifiable ici."
            },
        );
        return;
    }
    let Some(t) = base_type(&volume) else {
        section(ui, p, "Forme");
        label(ui, p, "Type de prim avancé (non modifiable ici).");
        return;
    };
    section(ui, p, "Forme");
    let mut v = volume;
    let mut acc = (false, false);
    fn track(acc: &mut (bool, bool), r: (bool, bool)) {
        acc.0 |= r.0;
        acc.1 |= r.1;
    }
    let mut ty = t;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(118.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            label(ui, p, "Type")
        });
        egui::ComboBox::from_id_salt("prim_type")
            .width(120.0)
            .selected_text(BASE_TYPES[t].0)
            .show_ui(ui, |ui| {
                for (i, (n, _, _)) in BASE_TYPES.iter().enumerate() {
                    ui.selectable_value(&mut ty, i, *n);
                }
            });
    });
    if ty != t {
        let (prof, path) = (BASE_TYPES[ty].1, BASE_TYPES[ty].2);
        let was_line = BASE_TYPES[t].2 == LL_PCODE_PATH_LINE;
        let is_line = path == LL_PCODE_PATH_LINE;
        v.profile.curve_type = prof | (v.profile.curve_type & LL_PCODE_HOLE_MASK);
        v.path.curve_type = path;
        if was_line != is_line {
            v.path.scale = if is_line { [1.0, 1.0] } else { [1.0, 0.25] };
            v.path.begin = 0.0;
            v.path.end = 1.0;
        }
        acc = (true, true);
    }
    let circular = BASE_TYPES[ty].2 == LL_PCODE_PATH_CIRCLE;
    let sphere_like = matches!(ty, 3..=6);
    // "Path cut" is the profile cut (S) of boxes / cylinders / prisms and
    // the path cut (T) of spheres and tori (LLPanelObject::getVolumeParams)
    {
        let (mut b, mut e) = if sphere_like {
            (v.path.begin, v.path.end)
        } else {
            (v.profile.begin, v.profile.end)
        };
        track(&mut acc, number(ui, p, "Coupe début", &mut b, 0.005, 0.0..=0.98, ""));
        track(&mut acc, number(ui, p, "Coupe fin", &mut e, 0.005, 0.02..=1.0, ""));
        if sphere_like {
            v.path.begin = b;
            v.path.end = e;
        } else {
            v.profile.begin = b;
            v.profile.end = e;
        }
    }
    let mut hollow = v.profile.hollow * 100.0;
    track(&mut acc, number(ui, p, "Creux", &mut hollow, 0.5, 0.0..=95.0, " %"));
    v.profile.hollow = hollow / 100.0;
    let hole = v.profile.curve_type & LL_PCODE_HOLE_MASK;
    let mut h = HOLE_TYPES.iter().position(|(_, c)| *c == hole).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(118.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            label(ui, p, "Forme du creux")
        });
        egui::ComboBox::from_id_salt("hole_type")
            .width(120.0)
            .selected_text(HOLE_TYPES[h].0)
            .show_ui(ui, |ui| {
                for (i, (n, _)) in HOLE_TYPES.iter().enumerate() {
                    ui.selectable_value(&mut h, i, *n);
                }
            });
    });
    if HOLE_TYPES[h].1 != hole {
        v.profile.curve_type = (v.profile.curve_type & LL_PCODE_PROFILE_MASK) | HOLE_TYPES[h].1;
        acc = (true, true);
    }
    let tmax = if circular { TWIST_MAX } else { TWIST_LINEAR_MAX };
    let mut tb = v.path.twist_begin * tmax;
    let mut te = v.path.twist_end * tmax;
    track(&mut acc, number(ui, p, "Torsion début", &mut tb, 1.0, -tmax..=tmax, "°"));
    track(&mut acc, number(ui, p, "Torsion fin", &mut te, 1.0, -tmax..=tmax, "°"));
    v.path.twist_begin = tb / tmax;
    v.path.twist_end = te / tmax;
    if !circular {
        // "Taper" of line paths is 1 - path scale
        let mut tx = 1.0 - v.path.scale[0];
        let mut tyy = 1.0 - v.path.scale[1];
        track(&mut acc, number(ui, p, "Effilement X", &mut tx, 0.01, -1.0..=1.0, ""));
        track(&mut acc, number(ui, p, "Effilement Y", &mut tyy, 0.01, -1.0..=1.0, ""));
        v.path.scale = [1.0 - tx, 1.0 - tyy];
        let (mut sx, mut sy) = (v.path.shear[0], v.path.shear[1]);
        track(&mut acc, number(ui, p, "Cisaillement X", &mut sx, 0.01, -0.5..=0.5, ""));
        track(&mut acc, number(ui, p, "Cisaillement Y", &mut sy, 0.01, -0.5..=0.5, ""));
        v.path.shear = [sx, sy];
    } else {
        // profile cut ("Dimple" of spheres)
        let (mut b, mut e) = (v.profile.begin, v.profile.end);
        let name = if ty == 3 {
            ("Fossette début", "Fossette fin")
        } else {
            ("Coupe profil début", "Coupe profil fin")
        };
        track(&mut acc, number(ui, p, name.0, &mut b, 0.005, 0.0..=0.98, ""));
        track(&mut acc, number(ui, p, name.1, &mut e, 0.005, 0.02..=1.0, ""));
        v.profile.begin = b;
        v.profile.end = e;
        if ty != 3 {
            let (mut hx, mut hy) = (v.path.scale[0], v.path.scale[1]);
            track(&mut acc, number(ui, p, "Taille du trou X", &mut hx, 0.01, 0.05..=1.0, ""));
            track(&mut acc, number(ui, p, "Taille du trou Y", &mut hy, 0.01, 0.05..=0.5, ""));
            v.path.scale = [hx, hy];
            let (mut sx, mut sy) = (v.path.shear[0], v.path.shear[1]);
            track(&mut acc, number(ui, p, "Cisaillement X", &mut sx, 0.01, -0.5..=0.5, ""));
            track(&mut acc, number(ui, p, "Cisaillement Y", &mut sy, 0.01, -0.5..=0.5, ""));
            v.path.shear = [sx, sy];
            let (mut ax, mut ay) = (v.path.taper[0], v.path.taper[1]);
            track(&mut acc, number(ui, p, "Effilement X", &mut ax, 0.01, -1.0..=1.0, ""));
            track(&mut acc, number(ui, p, "Effilement Y", &mut ay, 0.01, -1.0..=1.0, ""));
            v.path.taper = [ax, ay];
            track(
                &mut acc,
                number(ui, p, "Décalage du rayon", &mut v.path.radius_offset, 0.01, -1.0..=1.0, ""),
            );
            track(
                &mut acc,
                number(ui, p, "Révolutions", &mut v.path.revolutions, 0.015, 1.0..=4.0, ""),
            );
            track(&mut acc, number(ui, p, "Inclinaison", &mut v.path.skew, 0.01, -0.95..=0.95, ""));
        }
    }
    let (changed, commit) = acc;
    if changed {
        v.constrain();
        if let Some(o) = world.objects.get_mut(idx)
            && o.volume != v
        {
            o.volume = v;
            o.shape_dirty = true;
        }
        if commit {
            tool.send(BuildCmd::SetShape {
                handle: key.region,
                local_ids: vec![key.local_id],
                shape: v.to_raw(),
            });
        }
    }
}

fn create_panel(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    label(ui, p, "Choisissez une forme puis cliquez sur le sol ou un objet.");
    ui.add_space(4.0);
    let cell = Vec2::new(58.0, 58.0);
    egui::Grid::new("shapes").spacing(Vec2::splat(4.0)).show(ui, |ui| {
        for (i, sh) in SHAPES.iter().enumerate() {
            let sel = s.create_shape == i;
            let (rect, resp) = ui.allocate_exact_size(cell, egui::Sense::click());
            let fill = if sel {
                p.violet.gamma_multiply(0.55)
            } else if resp.hovered() {
                p.raised
            } else {
                p.field
            };
            ui.painter().rect_filled(rect, 3.0, fill);
            if sel {
                ui.painter()
                    .rect_stroke(rect, 3.0, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
            }
            let icon_rect = egui::Rect::from_min_size(rect.min + Vec2::new(4.0, 2.0), Vec2::new(cell.x - 8.0, cell.y - 18.0));
            shape_preview(ui, icon_rect, &sh.params(), sh.rotation(), if sel { Color32::WHITE } else { p.ink });
            ui.painter().text(
                egui::pos2(rect.center().x, rect.bottom() - 8.0),
                egui::Align2::CENTER_CENTER,
                sh.name,
                egui::FontId::proportional(10.0),
                p.muted,
            );
            if resp.on_hover_text(sh.name).clicked() {
                s.create_shape = i;
            }
            if i % 5 == 4 {
                ui.end_row();
            }
        }
    });
    ui.add_space(6.0);
    ui.checkbox(&mut s.keep_tool, RichText::new("Garder l'outil sélectionné").size(12.0));
    let mut size = s.new_prim_size;
    let (ch, _) = vec_row(ui, p, "Taille", &mut size, 0.01, 3, super::MIN_PRIM_SCALE..=super::MAX_PRIM_SCALE);
    if ch {
        s.new_prim_size = size;
    }
}

fn land_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    let select = s.land_action > land_action::REVERT;
    if choice(
        ui,
        p,
        select,
        "Sélectionner le terrain",
        "Glisser pour sélectionner une zone (grille de 4 m)",
    ) {
        s.land_action = 6;
    }
    ui.horizontal_wrapped(|ui| {
        for (i, n) in ACTION_NAMES.iter().enumerate() {
            if choice(ui, p, s.land_action as usize == i, n, "Maintenir le clic pour modifier le terrain") {
                s.land_action = i as u8;
            }
        }
    });
    section(ui, p, "Bulldozer");
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(80.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            label(ui, p, "Taille")
        });
        ui.add(egui::Slider::new(&mut s.land_brush_size, 1.0..=11.0).step_by(0.5));
    });
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(80.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            label(ui, p, "Force")
        });
        // logarithmic like LL: slider -1..2 = force 0.1..100
        let mut lg = s.land_brush_force.clamp(0.1, 100.0).log10();
        if ui.add(egui::Slider::new(&mut lg, -1.0..=2.0).show_value(false)).changed() {
            s.land_brush_force = 10f32.powf(lg);
        }
        ui.label(RichText::new(format!("{:.2}", s.land_brush_force)).size(11.5).color(p.muted));
    });
    ui.horizontal(|ui| {
        let can_apply = !select && tool.land.selection.is_some();
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
        if ui
            .add_enabled(
                !tool.land.last_regions.is_empty(),
                egui::Button::new(RichText::new("Annuler la modification").size(12.0)),
            )
            .on_hover_text("Ctrl+Z")
            .clicked()
        {
            let mut cmds = Vec::new();
            tool.land.undo(&mut cmds);
            for c in cmds {
                tool.send(c);
            }
        }
    });
    if let Some(m) = tool.land.take_message() {
        tool.status = m;
    }
    section(ui, p, "Parcelle");
    if let Some(parcel) = world.parcel.as_ref() {
        label(ui, p, &format!("{} — {} m²", parcel.name, parcel.area));
        let ok = parcel.flags & aurora_net::parcel_flags::ALLOW_TERRAFORM != 0 || parcel.owner_id == world.agent_id;
        label(
            ui,
            p,
            if ok {
                "Modification du terrain autorisée"
            } else {
                "Modification du terrain non autorisée ici"
            },
        );
    }
    if let Some((lo, hi)) = tool.land.selection {
        let d = hi - lo;
        label(ui, p, &format!("Sélection : {:.0} × {:.0} m ({:.0} m²)", d.x, d.y, d.x * d.y));
    }
}
