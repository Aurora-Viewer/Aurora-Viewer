//! The "Advanced" folds of Manage's region rules and terrain (newview/
//! llfloatertelehub.cpp, LLPanelRegionTerrainInfo in llfloaterregioninfo.cpp,
//! llpbrterrainfeatures.cpp, originally LGPL 2.1): the telehub and its spawn
//! points (EstateOwnerMessage `telehub` with the objects selected in the
//! world, TelehubInfo back), the terrain's PBR materials instead of its
//! textures (the same `texturedetail` slots, the region's kind found as
//! LLTerrainMaterials::getMaterialType does: textures unless only
//! materials load), their transforms (ModifyRegion) and the RAW file
//! downloaded or uploaded (`terrain` "download filename" / "upload
//! filename", then Xfer).

use super::confirm::Ask;
use super::draft::Draft;
use super::{RegionUi, View};
use crate::theme::Palette;
use crate::ui::sheet::{self, RowText, editable, line, srow, srow_ext, why};
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::estate::{RawDone, Role};
use aurora_net::estate::{MAX_SPAWNPOINTS_PER_TELEHUB, RawTransfer, TelehubVerb, TerrainSettings, TerrainTransform};
use egui::{RichText, Sense, Vec2};
use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use uuid::Uuid;

/// TERRAIN_DIRT_DETAIL, TERRAIN_GRASS_DETAIL, TERRAIN_MOUNTAIN_DETAIL,
/// TERRAIN_ROCK_DETAIL (LLVLComposition::getDefaultTextures).
const DEFAULT_TEXTURES: [Uuid; 4] = [
    Uuid::from_u128(0x0bc58228_74a0_7e83_89bc_5c23464bcec5),
    Uuid::from_u128(0x63338ede_0037_c4fd_855b_015d77112fc8),
    Uuid::from_u128(0x303cd381_8560_7579_23f1_f0a880799740),
    Uuid::from_u128(0x53a2f406_4895_1d13_d541_d2e3b86bc19c),
];
/// BLANK_MATERIAL_ASSET_ID, the materials' slots until one is chosen.
pub(crate) const BLANK_MATERIAL: Uuid = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);

/// What the terrain's four slots hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerrainKind {
    Textures,
    Materials,
}

/// Where a terrain material stands, as the app's material store says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialInfo {
    Pending,
    Missing,
    Loaded { double_sided: bool, blend: bool },
}

/// Why a material cannot cover the terrain (validateMaterials).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterialProblem {
    NotLoaded,
    LoadFailed,
    DoubleSided,
    Blend,
}

/// A file chosen, read or written off the window's thread
/// (LLFilePickerReplyThread).
enum RawFile {
    Save(String),
    Open(String, Vec<u8>),
    Written(String),
    Failed(String),
    Cancelled,
}

/// What the folds keep between frames.
#[derive(Default)]
pub(super) struct AdvancedState {
    pub rules_open: bool,
    pub terrain_open: bool,
    /// "info ui" was sent since the fold opened.
    telehub_asked: bool,
    /// The kind chosen in the window; None follows the region's.
    pub kind: Option<TerrainKind>,
    /// AURORA_DEMO_REGION: scroll to the end once the telehub is there.
    pub demo_jump: bool,
    /// … which happens after the column was drawn.
    pub demo_scroll: bool,
    /// The kind the terrain section showed last (what "Apply" sends).
    pub shown_kind: Option<TerrainKind>,
    /// The ids each kind had last (mLastSetTextures / mLastSetMaterials).
    last_textures: Option<[Uuid; 4]>,
    last_materials: Option<[Uuid; 4]>,
    pub transforms: Draft<Transforms>,
    raw_file: Option<mpsc::Receiver<RawFile>>,
    /// The last transfer's sentence, and whether it went well.
    raw_note: Option<(bool, String)>,
}

impl AdvancedState {
    pub fn reset(&mut self) {
        *self = AdvancedState::default();
    }
}

/// The four materials' transforms, as one section of changes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Transforms(pub [TerrainTransform; 4]);

impl super::draft::Changes for Transforms {
    fn changes(&self, o: &Self) -> usize {
        self.0.iter().zip(o.0.iter()).filter(|(a, b)| a != b).count()
    }
}

/// LLTerrainMaterials::getMaterialType: textures when one of them loads or
/// no material does, materials when only they load; textures while
/// neither is known.
pub(crate) fn detect(ids: &[Uuid; 4], texture_loaded: impl Fn(&Uuid) -> bool, materials: &HashMap<Uuid, MaterialInfo>) -> TerrainKind {
    let textures = ids.iter().any(&texture_loaded);
    let materials = ids.iter().any(|id| matches!(materials.get(id), Some(MaterialInfo::Loaded { .. })));
    if textures || !materials {
        TerrainKind::Textures
    } else {
        TerrainKind::Materials
    }
}

/// The kind the terrain section shows: the window's choice, else the
/// region's. Its ids are asked as materials too, to be told apart.
pub(super) fn kind(s: &mut RegionUi, server: &TerrainSettings, images: &HashMap<Uuid, egui::TextureHandle>) -> TerrainKind {
    for id in server.textures.iter().filter(|id| !id.is_nil()) {
        s.wanted_materials.insert(*id);
    }
    if let Some(k) = s.advanced.kind {
        return k;
    }
    detect(&server.textures, |id| images.contains_key(id), &s.material_info)
}

/// onSelectMaterialType: the slots take the other kind's last ids.
pub(super) fn switch_kind(s: &mut RegionUi, from: TerrainKind, to: TerrainKind, now: &mut TerrainSettings) {
    if from == to {
        return;
    }
    match from {
        TerrainKind::Textures => s.advanced.last_textures = Some(now.textures),
        TerrainKind::Materials => s.advanced.last_materials = Some(now.textures),
    }
    now.textures = match to {
        TerrainKind::Textures => s.advanced.last_textures.unwrap_or(DEFAULT_TEXTURES),
        TerrainKind::Materials => s.advanced.last_materials.unwrap_or([BLANK_MATERIAL; 4]),
    };
    s.advanced.kind = Some(to);
}

/// validateMaterials: the first slot whose material cannot cover the
/// terrain, and why.
pub(super) fn material_problem(s: &RegionUi, ids: &[Uuid; 4]) -> Option<(usize, MaterialProblem)> {
    ids.iter().enumerate().find_map(|(i, id)| {
        if *id == BLANK_MATERIAL {
            return None;
        }
        let problem = match s.material_info.get(id).copied().unwrap_or(MaterialInfo::Pending) {
            MaterialInfo::Pending => MaterialProblem::NotLoaded,
            MaterialInfo::Missing => MaterialProblem::LoadFailed,
            MaterialInfo::Loaded { double_sided: true, .. } => MaterialProblem::DoubleSided,
            MaterialInfo::Loaded { blend: true, .. } => MaterialProblem::Blend,
            MaterialInfo::Loaded { .. } => return None,
        };
        Some((i, problem))
    })
}

/// The transforms ModifyRegion gave, if the region has them.
pub(super) fn server_transforms(world: &World) -> Option<Transforms> {
    world.estate.transforms.as_ref()?.as_ref().ok().map(|t| Transforms(*t))
}

/// "Advanced: telehub and spawn points" of the region rules.
pub(super) fn rules_fold(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    sheet::fold_row(ui, p, &mut s.advanced.rules_open, aurora_i18n::tr!("region-rules-advanced"));
    if !s.advanced.rules_open {
        s.advanced.telehub_asked = false;
        return;
    }
    let can = v.manages();
    let objects = s.selection.clone();
    // LLFloaterTelehub::onOpen: sendTelehubInfoRequest
    if !s.advanced.telehub_asked && can {
        s.advanced.telehub_asked = true;
        world.estate.telehub(v.handle, TelehubVerb::Info, objects.clone());
    }
    why(ui, p, aurora_i18n::tr!("region-telehub-why"));
    let Some(info) = world.estate.telehub.clone() else {
        if can {
            why(ui, p, aurora_i18n::tr!("common-loading"));
        }
        return;
    };
    if std::mem::take(&mut s.advanced.demo_jump) {
        s.advanced.demo_scroll = true;
    }
    let connected = !info.object.is_nil();
    let selected = !objects.is_empty();
    let desc = if connected {
        aurora_i18n::tr!("region-telehub-connected", name = info.name.clone())
    } else {
        aurora_i18n::tr!("region-telehub-none").to_owned()
    };
    let size = Vec2::new(0.0, 26.0);
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("region-telehub"), &desc, 320.0, |ui| {
        if connected
            && widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("region-telehub-disconnect"),
                ActionKind::Flat,
                size,
                can,
            )
            .clicked()
        {
            world.estate.telehub(v.handle, TelehubVerb::Delete, objects.clone());
        }
        let r = widgets::action_button_if(
            ui,
            p,
            "map-pin-filled",
            aurora_i18n::tr!("region-telehub-connect"),
            ActionKind::Flat,
            size,
            can && selected,
        );
        if r.clicked() {
            world.estate.telehub(v.handle, TelehubVerb::Connect, objects.clone());
        }
    });
    let count = info.spawns.len();
    let name = format!(
        "{}  {}",
        aurora_i18n::tr!("region-telehub-spawns"),
        aurora_i18n::tr!("region-xp-count", count = count as i64, max = MAX_SPAWNPOINTS_PER_TELEHUB as i64)
    );
    line(ui, p);
    srow(ui, p, &name, aurora_i18n::tr!("region-telehub-spawns-desc"), 200.0, |ui| {
        let room = count < MAX_SPAWNPOINTS_PER_TELEHUB;
        let r = widgets::action_button_if(
            ui,
            p,
            "plus",
            aurora_i18n::tr!("region-telehub-add-spawn"),
            ActionKind::Flat,
            size,
            can && selected && room,
        );
        if r.clicked() {
            world.estate.telehub(v.handle, TelehubVerb::AddSpawn, objects.clone());
            // onClickAddSpawnPoint: the selection is let go
            s.action = Some(super::RegionAction::Deselect);
        }
    });
    for (i, pos) in info.spawns.iter().enumerate() {
        let text = aurora_i18n::tr!(
            "region-telehub-spawn",
            n = (i + 1) as i64,
            x = aurora_i18n::number(f64::from(pos[0]), 1),
            y = aurora_i18n::number(f64::from(pos[1]), 1),
            z = aurora_i18n::number(f64::from(pos[2]), 1)
        );
        if spawn_row(ui, p, &text, can) {
            // onClickRemoveSpawnPoint: by its index, without the selection
            world.estate.telehub(v.handle, TelehubVerb::RemoveSpawn(i as u32), Vec::new());
        }
    }
    if can && !selected {
        ui.add_space(4.0);
        ui.label(RichText::new(aurora_i18n::tr!("region-telehub-select")).size(11.5).color(p.muted));
    }
}

/// A spawn point: its position from the telehub and the x that removes
/// it. True when the x was clicked.
fn spawn_row(ui: &mut egui::Ui, p: &Palette, text: &str, removable: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    if r.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.35));
    }
    sheet::icon_at(
        ui,
        "map-pin-filled",
        egui::pos2(rect.left() + 14.0, rect.center().y),
        14.0,
        p.violet_light,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(12.5),
        p.ink,
    );
    if !removable {
        return false;
    }
    let x = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), Vec2::splat(20.0));
    let xr = ui.interact(x, ui.scope_id().with(("region_spawn_remove", rect.top() as i32)), Sense::click());
    sheet::icon_at(ui, "x", x.center(), 14.0, if xr.hovered() { p.danger } else { p.muted });
    xr.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("common-remove"))
        .clicked()
}

/// "Advanced: PBR transforms, RAW file" of the terrain.
pub(super) fn terrain_fold(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, kind: TerrainKind) {
    poll_raw(s, world);
    sheet::fold_row(ui, p, &mut s.advanced.terrain_open, aurora_i18n::tr!("region-terrain-advanced"));
    if !s.advanced.terrain_open {
        return;
    }
    let can = v.manages();
    let owner = v.shown == Role::Owner;
    if kind == TerrainKind::Materials {
        transforms(ui, p, s, world, can);
        local_paintmap(ui, p, v, world, std::mem::take(&mut s.surface.demo_paint_scroll));
    }
    // the RAW file is the estate owner's, as the bake (owner_or_god)
    let busy = world.estate.raw_busy || s.advanced.raw_file.is_some();
    let lock = (can && !owner).then_some(aurora_i18n::tr!("region-owner-only"));
    let desc = if busy {
        aurora_i18n::tr!("region-raw-busy")
    } else {
        aurora_i18n::tr!("region-raw-desc")
    };
    let text = RowText {
        name: aurora_i18n::tr!("region-raw"),
        lock,
        desc,
        link: None,
    };
    line(ui, p);
    srow_ext(ui, p, &text, 260.0, |ui| {
        let size = Vec2::new(0.0, 26.0);
        let enabled = can && owner && !busy;
        if widgets::action_button_if(
            ui,
            p,
            "file-upload-filled",
            aurora_i18n::tr!("region-raw-upload"),
            ActionKind::Flat,
            size,
            enabled,
        )
        .clicked()
        {
            s.advanced.raw_note = None;
            s.advanced.raw_file = Some(pick(false));
        }
        if widgets::action_button_if(
            ui,
            p,
            "download-filled",
            aurora_i18n::tr!("region-raw-download"),
            ActionKind::Flat,
            size,
            enabled,
        )
        .clicked()
        {
            s.advanced.raw_note = None;
            s.advanced.raw_file = Some(pick(true));
        }
    });
    if let Some((ok, note)) = &s.advanced.raw_note {
        let color = if *ok { p.muted } else { p.danger };
        ui.add(egui::Label::new(RichText::new(note).size(11.5).color(color)).wrap());
    }
}

/// The Legacy Viewer's experimental local paintmap create/delete actions.
/// This freezes composition locally; Apply never publishes the map.
fn local_paintmap(ui: &mut egui::Ui, p: &Palette, v: &View, world: &mut World, demo_scroll: bool) {
    let Some(region) = world.regions.get_mut(&v.handle) else { return };
    let loaded = region.heightmap.received_fraction() >= 1.0;
    let state = &mut region.terrain_surface;
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-paintmap"),
        aurora_i18n::tr!("region-paintmap-desc"),
        260.0,
        |ui| {
            if ui
                .add_enabled(
                    state.paintmap.is_some() || state.paint_busy(),
                    egui::Button::new(aurora_i18n::tr!("common-reset")),
                )
                .clicked()
            {
                state.clear_paint();
            }
            let label = if state.paint_busy() {
                aurora_i18n::tr!("common-loading")
            } else {
                aurora_i18n::tr!("region-paintmap-create")
            };
            if ui.add_enabled(loaded && !state.paint_busy(), egui::Button::new(label)).clicked() {
                state.request_paint();
            }
        },
    );
    if state.paint_failed {
        why(ui, p, aurora_i18n::tr!("region-paintmap-failed"));
    }
    if demo_scroll {
        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
    }
}

/// The four materials' scale, rotation and offset (the transforms panel,
/// shown when ModifyRegion answered).
fn transforms(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, world: &mut World, can: bool) {
    let Some(server) = server_transforms(world) else {
        line(ui, p);
        why(
            ui,
            p,
            match &world.estate.transforms {
                None => aurora_i18n::tr!("common-loading"),
                Some(_) => aurora_i18n::tr!("region-transforms-unavailable"),
            },
        );
        return;
    };
    let mut now = s.advanced.transforms.shown(&server);
    let before = now;
    for (i, t) in now.0.iter_mut().enumerate() {
        line(ui, p);
        let name = aurora_i18n::tr!("region-transforms-material", n = (i + 1) as i64);
        srow(ui, p, &name, aurora_i18n::tr!("region-transforms-desc"), 420.0, |ui| {
            // right to left: offset v, offset u, rotation, scale v, scale u
            let field = |ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f64>, tip: &str| {
                editable(ui, can, |ui| {
                    let mut v = f64::from(*value);
                    let r = ui
                        .add_sized(
                            [72.0, 24.0],
                            egui::DragValue::new(&mut v)
                                .range(range)
                                .speed(0.01)
                                .custom_parser(aurora_i18n::parse_number)
                                .custom_formatter(|v, _| aurora_i18n::number(v, 2)),
                        )
                        .on_hover_text(tip);
                    if r.changed() {
                        *value = v as f32;
                    }
                });
            };
            field(ui, &mut t.offset[1], -999.0..=999.0, aurora_i18n::tr!("region-transforms-offset-v"));
            field(ui, &mut t.offset[0], -999.0..=999.0, aurora_i18n::tr!("region-transforms-offset-u"));
            let mut degrees = t.rotation.to_degrees();
            field(ui, &mut degrees, -360.0..=360.0, aurora_i18n::tr!("region-transforms-rotation"));
            t.rotation = degrees.to_radians();
            field(ui, &mut t.scale[1], -100.0..=100.0, aurora_i18n::tr!("region-transforms-scale-v"));
            field(ui, &mut t.scale[0], -100.0..=100.0, aurora_i18n::tr!("region-transforms-scale-u"));
        });
    }
    if now != before {
        s.advanced.transforms.set(&server, now);
    }
}

/// The RAW file's dialogs, then what they and the transfer gave.
fn poll_raw(s: &mut RegionUi, world: &mut World) {
    let picked = s.advanced.raw_file.as_ref().and_then(|rx| rx.try_recv().ok());
    if let Some(picked) = picked {
        s.advanced.raw_file = None;
        let handle = world.estate.open;
        match picked {
            RawFile::Save(path) => {
                if let Some(handle) = handle {
                    world.estate.terrain_raw(handle, RawTransfer::Download { path });
                }
            }
            RawFile::Open(path, data) => {
                if let Some(handle) = handle {
                    world.estate.terrain_raw(
                        handle,
                        RawTransfer::Upload {
                            path,
                            data: Arc::new(data),
                        },
                    );
                    // RawUploadStarted
                    s.advanced.raw_note = Some((true, aurora_i18n::tr!("region-raw-upload-started").to_owned()));
                }
            }
            RawFile::Written(path) => {
                s.advanced.raw_note = Some((true, aurora_i18n::tr!("region-raw-downloaded", path = path)));
            }
            RawFile::Failed(why) => s.advanced.raw_note = Some((false, why)),
            RawFile::Cancelled => {}
        }
    }
    match world.estate.raw_done.take() {
        // callback_download_complete: the file is written, then said
        Some(RawDone::Downloaded { path, data }) => s.advanced.raw_file = Some(write(path, data)),
        Some(RawDone::Uploaded) => s.advanced.raw_note = Some((true, aurora_i18n::tr!("region-raw-uploaded").to_owned())),
        Some(RawDone::Failed(why)) => s.advanced.raw_note = Some((false, why)),
        None => {}
    }
}

/// FFSAVE_RAW "terrain.raw" or FFLOAD_RAW, on their own thread.
fn pick(save: bool) -> mpsc::Receiver<RawFile> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let dialog = rfd::FileDialog::new().add_filter("RAW", &["raw"]);
        let done = if save {
            match dialog.set_file_name("terrain.raw").save_file() {
                Some(path) => RawFile::Save(path.display().to_string()),
                None => RawFile::Cancelled,
            }
        } else {
            match dialog.pick_file() {
                None => RawFile::Cancelled,
                Some(path) => match std::fs::read(&path) {
                    Ok(data) => RawFile::Open(path.display().to_string(), data),
                    Err(e) => {
                        log::warn!("raw terrain file not read: {e}");
                        RawFile::Failed(aurora_i18n::tr!("ui-land-files-could-not-read-the-file").into())
                    }
                },
            }
        };
        let _ = tx.send(done);
    });
    rx
}

/// The downloaded file written where it was asked.
fn write(path: String, data: Arc<Vec<u8>>) -> mpsc::Receiver<RawFile> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let done = match std::fs::write(&path, data.as_slice()) {
            Ok(()) => RawFile::Written(path),
            Err(e) => {
                log::warn!("raw terrain file not written: {e}");
                RawFile::Failed(aurora_i18n::tr!("ui-land-files-could-not-save-the-file").into())
            }
        };
        let _ = tx.send(done);
    });
    rx
}

/// The question of a material that cannot cover the terrain.
pub(super) fn problem_ask(slot: usize, problem: MaterialProblem) -> Ask {
    Ask::TerrainMaterial { slot: slot as u8, problem }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// getMaterialType: a texture that loads wins, materials only when they
    /// alone load.
    #[test]
    fn the_kind_follows_what_loads() {
        let ids = [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4)];
        let mut materials = HashMap::new();
        assert_eq!(detect(&ids, |_| false, &materials), TerrainKind::Textures, "nothing known yet");
        materials.insert(
            ids[0],
            MaterialInfo::Loaded {
                double_sided: false,
                blend: false,
            },
        );
        assert_eq!(detect(&ids, |_| false, &materials), TerrainKind::Materials);
        assert_eq!(detect(&ids, |id| *id == ids[2], &materials), TerrainKind::Textures);
    }

    /// The transforms count a change per material.
    #[test]
    fn transforms_count_per_material() {
        use super::super::draft::Changes;
        let a = Transforms::default();
        let mut b = a;
        b.0[1].scale = [2.0, 2.0];
        b.0[1].rotation = 1.0;
        b.0[3].offset[0] = 0.5;
        assert_eq!(b.changes(&a), 2);
    }
}
