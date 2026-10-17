//! Mesh inspector: LLPanelObject::activateMeshFields / onCommitLOD.
//! Read-only information and temporary local rendering, also on no-mod meshes.
use super::{Env, common};
use crate::build::{BuildTool, Tool};
use crate::scene::{Scene, mesh_inspection::switch_distances};
use crate::theme::Palette;
use crate::world::{World, objects::ObjKey};
use egui::{RichText, Vec2};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Inspection {
    selection: Vec<ObjKey>,
    asset: Option<Uuid>,
    level: Option<u8>,
}

impl BuildTool {
    /// Reset on selection/asset/tool changes, retain across editing sections.
    /// Called every frame, including after the window is closed.
    pub fn sync_mesh_inspector(&mut self, world: &mut World, scene: &mut Scene) {
        let primary = self.selection.first().copied();
        let asset = primary
            .and_then(|k| world.objects.index_of(&k))
            .and_then(|i| world.objects.get(i))
            .filter(|o| self.open && self.tool == Tool::Edit && o.visual_volume().is_mesh())
            .and_then(|o| o.visual_volume().sculpt.map(|s| s.texture));
        let inspection = &mut self.ui.mesh_inspection;
        if asset.is_none() {
            inspection.selection.clear();
            inspection.asset = None;
            inspection.level = None;
        } else if inspection.selection != self.selection || inspection.asset != asset {
            inspection.selection.clone_from(&self.selection);
            inspection.asset = asset;
            inspection.level = None;
        }
        if primary
            .and_then(|key| world.objects.index_of(&key))
            .is_some_and(|idx| crate::build::select_type(world, idx) == crate::build::SelectType::Hud)
        {
            inspection.level = None;
        }
        let level = inspection.level;
        let keys: Vec<_> = if level.is_some() {
            self.sel_prims(world)
                .into_iter()
                .filter_map(|i| world.objects.get(i).map(|o| o.key))
                .collect()
        } else {
            Vec::new()
        };
        scene.set_mesh_lod_override(world, &keys, level);
        scene.inspect_mesh(
            world,
            if asset.is_some() && (self.tab == 5 || level.is_some()) {
                primary
            } else {
                None
            },
        );
    }

    /// Offline scripts use the same transient state as the UI dropdown.
    pub fn set_inspected_mesh_lod(&mut self, level: Option<u8>) {
        self.ui.mesh_inspection.level = level.filter(|&n| n < 4);
    }

    #[cfg(test)]
    pub(super) fn inspected_mesh_lod(&self) -> Option<u8> {
        self.ui.mesh_inspection.level
    }
}

fn value_row(ui: &mut egui::Ui, p: &Palette, name: &str, value: &str, active: bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(name).size(11.0).color(if active { p.violet_pale } else { p.muted }));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).size(11.0).color(if active { p.violet_pale } else { p.ink }));
        });
    });
}

fn distance_row(ui: &mut egui::Ui, p: &Palette, cells: [String; 4], widths: [f32; 4], header: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, (text, width)) in cells.into_iter().zip(widths).enumerate() {
            ui.add_sized(
                Vec2::new(width, 16.0),
                egui::Label::new(
                    RichText::new(&text)
                        .size(10.0)
                        .color(if header || i == 0 { p.muted } else { p.ink }),
                )
                .truncate(),
            )
            .on_hover_text(text);
        }
    });
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, idx: usize, env: &mut Env) {
    let Some(info) = env.scene.mesh_info(world, idx) else { return };
    let levels = [
        aurora_i18n::tr!("build-mesh-high"),
        aurora_i18n::tr!("build-mesh-medium"),
        aurora_i18n::tr!("build-mesh-low"),
        aurora_i18n::tr!("build-mesh-lowest"),
    ];
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.spacing_mut().interact_size.y = 16.0;
    ui.spacing_mut().button_padding = Vec2::new(3.0, 3.0);
    ui.horizontal(|ui| {
        common::section(ui, p, aurora_i18n::tr!("build-ui-object-mesh"));
        if let Some(faces) = info.faces {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                common::small(ui, p, &aurora_i18n::tr!("face-count", count = faces));
            });
        }
    });
    value_row(
        ui,
        p,
        aurora_i18n::tr!("build-mesh-lod"),
        aurora_i18n::tr!("build-mesh-triangles"),
        false,
    );
    for (i, name) in levels.iter().enumerate() {
        let lod = 3 - i;
        let value = info.triangles[lod]
            .map(|n| aurora_i18n::integer(n as i64))
            .unwrap_or_else(|| "—".into());
        value_row(ui, p, name, &value, tool.ui.mesh_inspection.level == Some(lod as u8));
    }
    let choices = [
        (aurora_i18n::tr!("build-mesh-default"), 0),
        (levels[0], 1),
        (levels[1], 2),
        (levels[2], 3),
        (levels[3], 4),
    ];
    ui.horizontal(|ui| {
        common::label(ui, p, aurora_i18n::tr!("build-mesh-shown"));
        let current = tool.ui.mesh_inspection.level.map(|n| 4 - n as usize).unwrap_or(0);
        let enabled = crate::build::select_type(world, idx) != crate::build::SelectType::Hud;
        if let Some(choice) = common::combo(ui, "mesh_lod", Some(current), &choices, 130.0, enabled) {
            tool.set_inspected_mesh_lod(if choice == 0 { None } else { Some((4 - choice) as u8) });
        }
    })
    .response
    .on_hover_text(aurora_i18n::tr!("build-mesh-inspection-help"));
    common::small(
        ui,
        p,
        if tool.ui.mesh_inspection.level.is_some() {
            aurora_i18n::tr!("build-mesh-local")
        } else {
            aurora_i18n::tr!("build-mesh-automatic")
        },
    );
    common::section(ui, p, aurora_i18n::tr!("build-mesh-behavior"));
    let radius = info
        .radius
        .map(|r| format!("{} m", aurora_i18n::number(r as f64, 3)))
        .unwrap_or_else(|| "—".into());
    value_row(ui, p, aurora_i18n::tr!("build-mesh-radius"), &radius, false);
    if info.rigged {
        common::small(ui, p, aurora_i18n::tr!("build-mesh-avatar-radius"));
    }
    let factors = [1.25, 2.0, env.scene.lod_factor];
    let distances = factors.map(|f| info.radius.and_then(|r| switch_distances(r, f)));
    let width = ui.available_width();
    let widths = [width * 0.44, width * 0.186, width * 0.186, width * 0.188];
    distance_row(
        ui,
        p,
        [
            aurora_i18n::tr!("build-mesh-transition").into(),
            "LL".into(),
            "Legacy".into(),
            aurora_i18n::tr!("build-mesh-current").into(),
        ],
        widths,
        true,
    );
    distance_row(
        ui,
        p,
        [
            aurora_i18n::tr!("build-mesh-factor").into(),
            aurora_i18n::number(1.25, 2),
            aurora_i18n::number(2.0, 2),
            aurora_i18n::number(factors[2] as f64, 3),
        ],
        widths,
        true,
    );
    for i in 0..3 {
        let mut cells = std::array::from_fn(|_| String::new());
        cells[0] = format!("{} ↔ {}", levels[i], levels[i + 1]);
        for (column, values) in distances.iter().enumerate() {
            cells[column + 1] = values.map(|d| aurora_i18n::number(d[i] as f64, 1)).unwrap_or_else(|| "—".into());
        }
        distance_row(ui, p, cells, widths, false);
    }
    common::small(ui, p, aurora_i18n::tr!("build-mesh-distances-note")).on_hover_text(aurora_i18n::tr!("build-mesh-distances-help"));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mesh_lod_choice_is_local_retained_between_sections_and_cleared_on_selection_tool_or_close() {
        let lib = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events()
            .into_iter()
            .chain(crate::demo::complex_face_selection_events())
        {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(99621)).unwrap();
        let other = world.objects.index_of_uuid(&crate::demo::action_id(99620)).unwrap();
        let settings = crate::build::BuildSettings {
            edit_linked: true,
            ..Default::default()
        };
        let mut scene = Scene::new(Default::default(), lib);
        let mut tool = BuildTool::default();
        tool.open_build(Tool::Edit);
        tool.click_select(&mut world, &settings, Some(idx), false);
        tool.sync_mesh_inspector(&mut world, &mut scene);
        tool.out.clear();
        tool.set_inspected_mesh_lod(Some(1));
        tool.tab = 1;
        tool.sync_mesh_inspector(&mut world, &mut scene);
        assert_eq!(tool.ui.mesh_inspection.level, Some(1));
        assert!(tool.out.is_empty());
        tool.tab = 5;
        tool.sync_mesh_inspector(&mut world, &mut scene);
        assert_eq!(tool.ui.mesh_inspection.level, Some(1));
        tool.click_select(&mut world, &settings, Some(other), false);
        tool.sync_mesh_inspector(&mut world, &mut scene);
        assert_eq!(tool.ui.mesh_inspection.level, None);
        tool.click_select(&mut world, &settings, Some(idx), false);
        tool.sync_mesh_inspector(&mut world, &mut scene);
        tool.set_inspected_mesh_lod(Some(0));
        tool.set_tool(Tool::Focus);
        tool.sync_mesh_inspector(&mut world, &mut scene);
        assert_eq!(tool.ui.mesh_inspection.level, None);
        tool.set_tool(Tool::Edit);
        tool.sync_mesh_inspector(&mut world, &mut scene);
        tool.set_inspected_mesh_lod(Some(3));
        tool.open = false;
        tool.sync_mesh_inspector(&mut world, &mut scene);
        assert_eq!(tool.ui.mesh_inspection.level, None);
    }
}
