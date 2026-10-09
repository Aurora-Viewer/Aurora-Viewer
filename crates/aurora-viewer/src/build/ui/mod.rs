//! The build floater (LLFloaterTools, floater_tools.xml, Firestorm) in the
//! Aurora skin: the five tools and their options on top, the selection
//! line (land impact, capacity, « Plus d'infos »), then the General,
//! Object, Features, Texture and Contents tabs; the bottom arrow folds the
//! tabs away (FSToolboxExpanded). Also the build keyboard shortcuts
//! (menu_viewer.xml) and the secondary floaters (grid options, weights,
//! media settings).

mod common;
mod contents;
mod features;
mod floaters;
mod general;
mod object;
mod texture;
mod tools;

use super::{BuildSettings, BuildTool, EditMode, Tool};
use crate::media::entry::ObjectMediaData;
use crate::theme::Palette;
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::{Floater, tabs};
use crate::world::World;
use crate::world::objects::ObjKey;
use egui::{RichText, Vec2};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub use floaters::MediaSettingsState;

/// What the floater needs from the rest of the viewer.
pub struct Env<'a> {
    /// Images loaded for the interface (texture swatches).
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub media: &'a mut crate::media::MediaManager,
    pub scene: &'a mut crate::scene::Scene,
    /// Focus tool slider: the camera's zoom fraction (0 out .. 1 in).
    pub zoom_fraction: f32,
    pub demo: bool,
}

/// Something the floater asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// Focus slider moved (LLAgentCamera::setCameraZoomFraction).
    ZoomFraction(f32),
    AboutLand,
    BuyLand,
    Profile(Uuid),
    GroupProfile(Uuid),
    /// « p » of the Object tab: read the system clipboard for this vector
    /// (0 position, 1 size, 2 rotation) and call BuildTool::paste_vector.
    PasteVector(usize),
}

/// Where a texture chosen in the picker goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickTarget {
    Sculpt,
    LightTexture,
    /// Blinn-Phong diffuse / normal / specular.
    Diffuse,
    Normal,
    Specular,
    /// PBR material asset, or one of its four maps (base, normal, ORM, emissive).
    PbrMaterial,
    PbrMap(usize),
}

/// One face copied with « Copier » of the Texture tab (FSPanelFace::onCopyFaces).
#[derive(Debug, Clone)]
pub struct FaceClip {
    pub face: aurora_prim::te::TextureFace,
    pub legacy: Option<crate::scene::legacy_mat::LegacyMaterial>,
    pub gltf: Option<(Uuid, aurora_assets::material::PbrOverride)>,
}

/// Object parameters copied with « Copier » of the Object tab (LLPanelObject::onCopyParams).
#[derive(Debug, Clone, Copy)]
pub struct ShapeClip {
    pub volume: aurora_prim::VolumeParams,
    pub sculpt: Option<aurora_prim::params::SculptParams>,
}

/// Features copied with « Copier » of the Features tab (LLPanelVolume::onCopyFeatures).
#[derive(Debug, Clone, Copy)]
pub struct FeaturesClip {
    pub flexible: Option<aurora_prim::extra::FlexibleParams>,
    pub light: Option<aurora_prim::extra::LightParams>,
    pub light_image: Option<aurora_prim::extra::LightImageParams>,
    pub probe: Option<aurora_prim::extra::ReflectionProbeParams>,
    pub material: u8,
    pub physics: Option<aurora_net::build::PhysicsParams>,
}

/// A question to confirm before acting (notifications.xml).
#[derive(Debug, Clone, PartialEq)]
pub enum Confirm {
    /// DeedObjectToGroup.
    Deed(Uuid),
    /// LandDivideWarning / JoinLandWarning / ReleaseLandWarning.
    Divide,
    Join,
    Release,
    /// DeleteMedia.
    DeleteMedia,
    /// WaterExclusionSurfacesWarning.
    HideWater,
    /// ReflectionProbeApplied.
    Probe,
}

/// State of the floater kept between frames.
#[derive(Default)]
pub struct FloaterState {
    /// Texture tab: 0 PBR, 1 Blinn-Phong, 2 Media.
    pub tex_tab: usize,
    /// PBR transform channel (Tous, Couleur, Normale, (O)RM, Émissive).
    pub pbr_channel: usize,
    /// Blinn-Phong UV channel (Diffuse, Normale, Spéculaire).
    pub bp_channel: usize,
    pub face_clipboard: Option<Vec<FaceClip>>,
    pub shape_clipboard: Option<ShapeClip>,
    /// Position, size, rotation (degrees) copied with « C ».
    pub vec_clipboard: [Option<Vec3>; 3],
    pub features_clipboard: Option<FeaturesClip>,
    /// Text being typed (field id -> text) until Enter / focus lost.
    pub edits: HashMap<&'static str, String>,
    /// « À vendre » ticked / sale type / price not applied yet (Appliquer).
    pub sale_draft: Option<(bool, u8, i32)>,
    pub contents_filter: String,
    /// Item being renamed in the Contents tab.
    pub rename: Option<(Uuid, String)>,
    pub weights_open: bool,
    pub grid_options_open: bool,
    pub group_picker_open: bool,
    pub media_settings: Option<MediaSettingsState>,
    pub confirm: Option<Confirm>,
    pub picker: TexturePicker,
    pub pick_target: Option<PickTarget>,
    /// ObjectMedia data fetched for the selection, and the objects asked.
    pub media_data: HashMap<Uuid, ObjectMediaData>,
    pub media_asked: HashSet<Uuid>,
    /// Blinn-Phong material just sent per (object, face), shown until the
    /// face's material id changes: (material id when sent, material).
    pub legacy_pending: HashMap<(Uuid, u8), (Uuid, Option<crate::scene::legacy_mat::LegacyMaterial>)>,
    /// « Aligner les faces planaires ».
    pub planar_align: bool,
    pub bulk_perms_open: bool,
    pub bulk: floaters::BulkPerms,
    /// Contents item whose properties are shown.
    pub item_properties: Option<aurora_net::build::TaskItem>,
    /// CantSelectReflectionProbe shown once.
    pub probe_warned: bool,
    /// Images the swatches want loaded (drained by the app).
    pub wanted_images: HashSet<Uuid>,
    pub requests: Vec<Request>,
    /// The selection seen last frame (drafts are dropped when it changes).
    last_selection: Vec<ObjKey>,
}

/// Width of the label column of the tabs.
const LABEL_W: f32 = 112.0;

/// Build shortcuts (menu_viewer.xml): Ctrl+B, Ctrl+1..5, delete, duplicate,
/// link, edit linked parts, grid, undo...
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
                    // LLToolMgr::enterBuildMode selects the Create tool
                    tool.open_build(Tool::Create);
                }
            }
            (Key::Num1, true, false) => tool.open_build(Tool::Focus),
            (Key::Num2, true, false) => tool.open_build(Tool::Grab),
            (Key::Num3, true, false) => tool.open_build(Tool::Edit),
            (Key::Num4, true, false) => tool.open_build(Tool::Create),
            (Key::Num5, true, false) => tool.open_build(Tool::Land),
            (Key::B, true, true) => tool.ui.grid_options_open = !tool.ui.grid_options_open,
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
            (Key::G, false, true) => tool.use_selection_for_grid(world, s),
            (Key::X, false, true) => tool.snap_xy_to_grid(world, s),
            (Key::Period, true, sh) => tool.select_next_part(world, s, true, sh),
            (Key::Comma, true, sh) => tool.select_next_part(world, s, false, sh),
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
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: super::Mods,
    env: &mut Env,
) -> bool {
    if !tool.open {
        return false;
    }
    let before = s.clone();
    // drafts belong to the selection they were typed for
    if tool.ui.last_selection != tool.selection {
        tool.ui.last_selection = tool.selection.clone();
        tool.ui.edits.clear();
        tool.ui.sale_draft = None;
        tool.ui.rename = None;
    }
    let screen = ctx.content_rect();
    let mut open = true;
    let height = (screen.height() - 110.0).clamp(420.0, 760.0);
    Floater::new(
        "build_tools",
        "Construire",
        egui::pos2(screen.left() + 12.0, 40.0),
        Vec2::new(330.0, height),
    )
    .help("Outils de construction : Ctrl+B ouvre / ferme ; Ctrl+1 à 5 : mise au point, déplacer, modifier, créer, terrain")
    // floater_tools is silent in Firestorm (sound_flags="0")
    .silent()
    .show(ctx, p, &mut open, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        tools::tool_row(ui, p, tool);
        ui.label(RichText::new(tools::status_text(tool, s, mods)).size(11.5).color(p.muted));
        ui.add_space(2.0);
        match tool.tool {
            Tool::Focus => tools::focus_panel(ui, p, tool, mods, env),
            Tool::Grab => tools::grab_panel(ui, p, tool, mods),
            Tool::Edit => tools::edit_panel(ui, p, tool, world, s, mods),
            Tool::Create => tools::create_panel(ui, p, s),
            Tool::Land => tools::land_panel(ui, p, tool, world, s),
        }
        ui.add_space(2.0);
        if tool.tool == Tool::Land {
            if s.expanded {
                ui.separator();
                tools::parcel_panel(ui, p, tool, world, s);
            }
        } else {
            tools::selection_line(ui, p, tool, world);
            if s.expanded {
                tabs_panel(ui, p, tool, world, s, env);
            }
        }
        if !tool.status.is_empty() {
            ui.label(RichText::new(&tool.status).size(11.0).color(p.warn));
        }
        // FSToolboxExpanded: the arrow at the bottom folds the tabs
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 12.0), egui::Sense::click());
        ui.painter().rect_filled(rect, 2.0, if resp.hovered() { p.raised } else { p.field });
        let icon = if s.expanded { "caret-up" } else { "caret-down" };
        if let Some(t) = crate::ui::icons::global(icon) {
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(11.0)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                p.ink,
            );
        }
        if resp.on_hover_text(if s.expanded { "Replier" } else { "Déplier" }).clicked() {
            s.expanded = !s.expanded;
        }
    });
    floaters::secondary(ctx, p, tool, world, s, env);
    if !open {
        tool.close(world, s);
    }
    *s != before
}

/// The five tabs under the tools.
fn tabs_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    ui.add_space(2.0);
    tabs(
        ui,
        p,
        &mut tool.tab,
        &[
            ("Général", true),
            ("Objet", true),
            ("Attributs", true),
            ("Texture", true),
            ("Contenu", true),
        ],
    );
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("build_tab_scroll")
        .auto_shrink([false, false])
        .max_height(ui.available_height() - 18.0)
        .show(ui, |ui| {
            // nothing selected: empty tabs (the line above says so)
            if tool.selection.is_empty() {
                return;
            }
            match tool.tab {
                0 => general::show(ui, p, tool, world, s),
                1 => object::show(ui, p, tool, world, s, env),
                2 => features::show(ui, p, tool, world, s, env),
                3 => texture::show(ui, p, tool, world, s, env),
                _ => contents::show(ui, p, tool, world),
            }
        });
}

impl BuildTool {
    /// « Utiliser la sélection pour la grille » (Shift+G): the selected
    /// roots become the reference grid (LLSelectMgr::addGridObject).
    pub fn use_selection_for_grid(&mut self, world: &World, s: &mut BuildSettings) {
        self.grid_objects = self
            .roots(world)
            .into_iter()
            .filter_map(|r| world.objects.get(r).map(|o| o.key))
            .collect();
        if !self.grid_objects.is_empty() {
            s.grid_mode = 2;
        }
    }

    /// Shift+X: XY of every modifiable root rounded to the grid unit
    /// (handle_snap_xy_to_grid), sent as positions.
    pub fn snap_xy_to_grid(&mut self, world: &mut World, s: &BuildSettings) {
        let unit = s.grid_resolution.max(0.001);
        let mut by_region: HashMap<aurora_net::RegionHandle, Vec<aurora_net::build::TransformUpdate>> = HashMap::new();
        for r in self.roots(world) {
            if !self.can_modify(world, r) {
                continue;
            }
            let Some(off) = world.objects.get(r).and_then(|o| world.region_offset(o.key.region)) else {
                continue;
            };
            let Some(o) = world.objects.get_mut(r) else { continue };
            // global X / Y: the region offset is a multiple of the unit in practice
            let g = o.position + off;
            let snapped = Vec3::new((g.x / unit).round() * unit, (g.y / unit).round() * unit, g.z) - off;
            o.position = snapped;
            o.render.needs_records = true;
            by_region.entry(o.key.region).or_default().push(aurora_net::build::TransformUpdate {
                local_id: o.key.local_id,
                position: Some(snapped),
                rotation: None,
                scale: None,
                linked: true,
                uniform: false,
            });
        }
        for (handle, updates) in by_region {
            self.send(aurora_net::build::BuildCmd::Transform { handle, updates });
        }
    }

    /// ◄ ► (LLToolsSelectNextPartFace): next / previous face with the face
    /// tool, else next / previous prim of the linkset with « Modification
    /// liée »; `include` (Shift) adds it to the selection.
    pub fn select_next_part(&mut self, world: &mut World, s: &mut BuildSettings, next: bool, include: bool) {
        let face_mode = self.tool == Tool::Edit && self.edit_mode == EditMode::Face;
        if !(s.edit_linked || face_mode) {
            return;
        }
        let Some(key) = self.selection.first().copied() else { return };
        let Some(idx) = world.objects.index_of(&key) else { return };
        if face_mode {
            let n = self.num_faces(world, idx) as i32;
            let cur = self.last_face.map(|f| f as i32).unwrap_or(-1);
            let all = self.faces.get(&key).is_none_or(|f| f.len() as i32 >= n);
            let f = if all && next {
                0
            } else if next {
                cur + 1
            } else {
                cur - 1
            };
            if (0..n).contains(&f) {
                let entry = self.faces.entry(key).or_default();
                if !include {
                    entry.clear();
                }
                if !entry.contains(&(f as u8)) {
                    entry.push(f as u8);
                }
                self.last_face = Some(f as u8);
                return;
            }
            if !s.edit_linked {
                // wrap around the faces of the same prim
                let f = if next { 0 } else { n - 1 };
                self.faces.insert(key, vec![f as u8]);
                self.last_face = Some(f as u8);
                return;
            }
        }
        // next / previous prim: root first, then the children in order
        let root = super::root_of(world, idx);
        let list: Vec<usize> = super::family(world, root)
            .into_iter()
            .filter(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar()))
            .collect();
        let Some(pos) = list.iter().position(|&i| i == idx) else { return };
        let n = list.len();
        let target = if next { list[(pos + 1) % n] } else { list[(pos + n - 1) % n] };
        let Some(unit) = world.objects.get(target).map(|o| o.key) else {
            return;
        };
        if !include {
            self.deselect_all(world);
        }
        self.selection_linked = true;
        self.add(world, unit);
        if face_mode {
            let f = if next {
                0
            } else {
                self.num_faces(world, target).saturating_sub(1) as u8
            };
            self.faces.insert(unit, vec![f]);
            self.last_face = Some(f);
        }
    }

    /// « Copier l'UUID »: the roots' ids, or every selected prim with
    /// Shift / « Modification liée » (LLFloaterTools::onClickBtnCopyKeys).
    pub fn copy_keys(&self, world: &World, s: &BuildSettings, all_prims: bool) -> String {
        let idxs = if all_prims || s.edit_linked {
            self.sel_prims(world)
        } else {
            self.roots(world)
        };
        idxs.iter()
            .filter_map(|&i| world.objects.get(i).map(|o| o.full_id.to_string()))
            .collect::<Vec<_>>()
            .join(&s.copy_key_separator)
    }
}
