//! The build floater (LLFloaterTools, floater_tools.xml, Firestorm) in the
//! Aurora skin: five editing sections and five tools in one navigation
//! rail. General holds object information and ownership permissions.
//! The active section supplies the title; edit selection/actions belong
//! to the main panel. The title bar folds it away (FSToolboxExpanded).
//! Also the build keyboard shortcuts
//! (menu_viewer.xml) and the secondary floaters (grid options, weights,
//! media settings).

mod color;
mod common;
pub(crate) use common::swatch as texture_swatch;
mod compact;
pub(crate) mod contents;
pub(crate) mod contents_permissions;
mod contents_selection;
mod features;
mod floaters;
pub(crate) mod general;
mod item_properties;
mod keyboard;
mod layout;
mod legacy;
mod notecards;
pub use notecards::{loaded as layout_notecard_loaded, saved as layout_notecard_saved};
mod mesh;
mod object;
pub(crate) mod particles;
mod replace;
mod texture;
mod texture_preview;
mod tools;
mod workspace;

use super::{BuildSettings, BuildTool, EditMode, Tool};
use crate::media::entry::ObjectMediaData;
use crate::theme::Palette;
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::Floater;
use crate::world::World;
use crate::world::objects::ObjKey;
use egui::Vec2;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(super) use color::ColorTarget;
pub use floaters::MediaSettingsState;
pub use keyboard::Shortcut;

/// What the floater needs from the rest of the viewer.
pub struct Env<'a> {
    /// Images loaded for the interface (texture swatches).
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub media: &'a mut crate::media::MediaManager,
    pub scene: &'a mut crate::scene::Scene,
    /// Focus tool slider: the camera's zoom fraction (0 out .. 1 in).
    pub zoom_fraction: f32,
    /// The saved ShowPropertyLines switch, shared with both world menus.
    pub property_lines: &'a mut bool,
    pub demo: bool,
}

/// Something the floater asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    ExportLayoutNotecard {
        name: String,
        text: String,
    },
    LoadLayoutNotecard(Uuid),
    TransferTask(Box<super::task_inventory::TransferRequest>),
    /// Focus slider moved (LLAgentCamera::setCameraZoomFraction).
    ZoomFraction(f32),
    AboutLand,
    BuyLand,
    Profile(Uuid),
    GroupProfile(Uuid),
    /// "p" of the Object tab: read the system clipboard for this vector
    /// (0 position, 1 size, 2 rotation) and call BuildTool::paste_vector.
    PasteVector(usize),
    CopyKeys,
    Script {
        object: Uuid,
        item: Uuid,
    },
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

/// One face copied with "Copy" of the Texture tab (FSPanelFace::onCopyFaces).
#[derive(Debug, Clone)]
pub struct FaceClip {
    pub face: aurora_prim::te::TextureFace,
    pub legacy: Option<crate::scene::legacy_mat::LegacyMaterial>,
    pub gltf: Option<(Uuid, aurora_assets::material::PbrOverride)>,
}

/// Object parameters copied with "Copy" of the Object tab (LLPanelObject::onCopyParams).
#[derive(Debug, Clone, Copy)]
pub struct ShapeClip {
    pub volume: aurora_prim::VolumeParams,
    pub sculpt: Option<aurora_prim::params::SculptParams>,
}

/// Features copied with "Copy" of the Features tab (LLPanelVolume::onCopyFeatures).
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
    /// Active presentation, mirrored before input routing (not a preference).
    legacy_window: bool,
    legacy_pbr_editor: bool,
    pub layout: layout::State,
    pub notecards: notecards::State,
    mesh_inspection: mesh::Inspection,
    material_previews: crate::ui::texture_library::previews::Previews,
    pub color_picker: crate::ui::color_picker::ColorPicker,
    color_edit: Option<color::Session>,
    color_focused: bool,
    /// Texture tab: 0 Classic (Blinn-Phong), 1 PBR, 2 Media.
    pub tex_tab: usize,
    /// PBR transform channel (All, Color, Normal, (O)RM, Emissive).
    pub pbr_channel: usize,
    /// Blinn-Phong UV channel (Diffuse, Normal, Specular).
    pub bp_channel: usize,
    pub face_clipboard: Option<Vec<FaceClip>>,
    pub shape_clipboard: Option<ShapeClip>,
    /// Position, size, rotation (degrees) copied with "C".
    pub vec_clipboard: [Option<Vec3>; 3],
    pub features_clipboard: Option<FeaturesClip>,
    /// Text being typed (field id -> text) until Enter / focus lost.
    pub edits: HashMap<&'static str, String>,
    /// "For sale" ticked / sale type / price not applied yet (Apply).
    pub sale_draft: Option<(bool, u8, i32)>,
    pub contents_filter: String,
    /// Independent from every personal/folder inventory window.
    pub contents_show_next_owner: bool,
    pub contents_selection: contents_selection::Selection,
    /// Whether the inventory held over the Contents tab may go in.
    pub contents_drop: crate::build::contents_drop::PlanCheck,
    /// Item being renamed in the Contents tab.
    pub rename: Option<contents_selection::Rename>,
    pub open_notecard: Option<(Uuid, Uuid)>,
    pub weights_open: bool,
    pub grid_options_open: bool,
    pub group_picker_open: bool,
    pub media_settings: Option<MediaSettingsState>,
    pub confirm: Option<Confirm>,
    pub picker: TexturePicker,
    pub pick_target: Option<PickTarget>,
    texture_edit: Option<texture_preview::Session>,
    /// ObjectMedia data fetched for the selection, and the objects asked.
    pub media_data: HashMap<Uuid, ObjectMediaData>,
    pub media_asked: HashSet<Uuid>,
    /// Blinn-Phong material just sent per (object, face), shown until the
    /// face's material id changes and its replacement loads (or is cleared):
    /// (material id when sent, material).
    pub legacy_pending: HashMap<(Uuid, u8), (Uuid, Option<crate::scene::legacy_mat::LegacyMaterial>)>,
    /// "Align planar faces".
    pub planar_align: bool,
    pub bulk_perms_open: bool,
    pub bulk: contents_permissions::BulkState,
    /// Contents item whose properties are shown.
    pub item_properties: Option<item_properties::TaskProperties>,
    /// CantSelectReflectionProbe shown once.
    pub probe_warned: bool,
    /// Images the swatches want loaded (drained by the app).
    pub wanted_images: HashSet<Uuid>,
    pub reveal_faces: bool,
    pub requests: Vec<Request>,
    /// The selection seen last frame (drafts are dropped when it changes).
    last_selection: Vec<ObjKey>,
}

/// Width of the label column of the tabs.
const LABEL_W: f32 = 112.0;
pub(crate) const INSPECTOR_ID: &str = "build_tools_inspector_c3";

/// Build shortcuts (menu_viewer.xml): Ctrl+B, Ctrl+1..5, delete, duplicate,
/// link, edit linked parts, grid, undo...
pub fn shortcuts(ctx: &egui::Context, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    tool.ui.legacy_window = s.legacy_window;
    if tool.ui.layout.organizing || tool.ui.notecards.open.is_some() {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if tool.ui.notecards.open.is_some() {
                tool.ui.notecards.open = None;
            } else if tool.ui.layout.drag.take().is_none() {
                tool.ui.layout.organizing = false;
            }
        }
        return;
    }
    if contents_selection::shortcuts(ctx, tool, world) {
        return;
    }
    if ctx.egui_wants_keyboard_input() || egui::Popup::is_any_open(ctx) {
        return;
    }
    if tool.replacement_active() {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            tool.cancel_replacement(world);
        }
        return;
    }
    let pressed = ctx.input(|i| i.events.clone());
    for ev in pressed {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat,
            modifiers: m,
            ..
        } = ev
        else {
            continue;
        };
        let mods = crate::keybinds::Mods {
            ctrl: m.command || m.ctrl,
            shift: m.shift,
            alt: m.alt,
        };
        if let Some(command) = Shortcut::from_key(key, mods, tool.open) {
            // The picker owns Escape in stages (pipette, then dialog).
            // Leave its event intact instead of consuming a no-op Close.
            if command == Shortcut::Close && tool.ui.color_picker.is_open() {
                continue;
            }
            if !repeat || matches!(command, Shortcut::History(_)) {
                command.run(tool, world, s);
            }
            // Repeated non-repeatable commands still own their keys.
            ctx.input_mut(|i| {
                i.consume_key(m, key);
            });
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
    contents_selection::finish_if_hidden_or_blurred(ctx, tool, world);
    if !tool.open {
        tool.cancel_texture_picker(world, env);
        tool.cancel_color_picker(world);
        tool.commit_color_edit();
        tool.ui.layout.open = false;
        tool.ui.layout.organizing = false;
        tool.ui.layout.drag = None;
        return false;
    }
    let before = s.clone();
    let property_lines_before = *env.property_lines;
    // drafts belong to the selection they were typed for
    if tool.ui.last_selection != tool.selection {
        tool.cancel_color_picker(world);
        tool.commit_color_edit();
        tool.ui.last_selection = tool.selection.clone();
        tool.ui.edits.clear();
        tool.ui.sale_draft = None;
        contents_selection::finish_rename(tool, world);
        tool.ui.contents_selection = Default::default();
    }
    let screen = ctx.content_rect();
    let demo_picker = egui::Id::unique("demo_color_picker_opened");
    let picker_mode = crate::ui::color_picker::demo::mode();
    if env.demo
        && matches!(picker_mode, Some("left" | "follow" | "link" | "toggle" | "dock"))
        && !ctx.data(|d| d.get_temp::<bool>(demo_picker).unwrap_or(false))
    {
        crate::ui::widgets::place_floater(
            ctx,
            INSPECTOR_ID,
            egui::pos2(
                if picker_mode == Some("left") {
                    screen.right() - 344.0 - 12.0
                } else {
                    screen.left() + 12.0
                },
                40.0,
            ),
            Vec2::new(344.0, 500.0),
        );
    }
    workspace::prepare(ctx, env.demo, s);
    tool.ui.legacy_window = s.legacy_window;
    let preview = egui::Id::unique((INSPECTOR_ID, "compact-preview"));
    if env.demo && !ctx.data(|d| d.get_temp::<bool>(preview).unwrap_or(false)) {
        ctx.data_mut(|d| d.insert_temp(preview, true));
        let mode = std::env::var("AURORA_DEMO_BUILD_TAB").unwrap_or_default();
        if mode.split('+').any(|mode| mode == "folded") {
            s.transform_expanded = false;
        }
    }
    let width = egui::AreaState::load(ctx, egui::Id::unique(INSPECTOR_ID))
        .and_then(|state| state.size)
        .unwrap_or(workspace::DEFAULT_SIZE)
        .x;
    s.layout.normalize();
    if s.legacy_window && tool.tab == 1 {
        tool.tab = 5;
    }
    if !s.legacy_window && matches!(tool.tab, 0 | 1) {
        tool.tab = 6;
    }
    if !s.legacy_window
        && tool.tool == Tool::Edit
        && !s
            .layout
            .in_panel(crate::build::layout::Panel::Navigation)
            .iter()
            .any(|item| layout::section(*item) == Some(tool.tab))
    {
        tool.tab = s
            .layout
            .in_panel(crate::build::layout::Panel::Navigation)
            .iter()
            .find_map(|item| layout::section(*item))
            .unwrap_or(6);
    }
    tool.ui.layout.targets.clear();
    tool.ui.layout.zones.clear();
    let mut open = true;
    let mut toggle_sections = false;
    let mut legacy_toggle_sections = false;
    let mut copy_uuid = false;
    let mut toggle_settings = false;
    if s.legacy_window {
        tool.ui.layout.organizing = false;
        tool.ui.layout.drag = None;
        tool.ui.notecards.open = None;
    }
    let (title_icon, title) = if s.legacy_window {
        ("tools", aurora_i18n::tr!("build-compact-title"))
    } else {
        compact::window_heading(tool)
    };
    let default_size = if s.legacy_window {
        legacy::DEFAULT_SIZE
    } else {
        workspace::DEFAULT_SIZE
    };
    let mut floater = Floater::new(INSPECTOR_ID, title, egui::pos2(screen.left() + 12.0, 40.0), default_size)
        .help(aurora_i18n::tr!("build-compact-help"))
        .steady_size()
        .silent();
    if !s.legacy_window {
        floater = floater.framed_header(title_icon).action(
            if s.expanded { "caret-up-filled" } else { "caret-down-filled" },
            aurora_i18n::tr!("build-compact-collapse"),
            &mut toggle_sections,
        );
        floater.header_status = compact::object_status(tool, world, p);
        floater.header_status_action = Some((
            "clipboard-text-filled",
            aurora_i18n::tr!("build-compact-uuid"),
            aurora_i18n::tr!("build-ui-general-copy-object-keys-shift-all-parts"),
            &mut copy_uuid,
        ));
    }
    if !s.legacy_window && !matches!(tool.tool, Tool::Edit | Tool::Create) {
        floater = floater.action("settings-filled", aurora_i18n::tr!("build-layout-settings"), &mut toggle_settings);
    }
    floater.min_size = if s.legacy_window {
        legacy::MIN_SIZE
    } else {
        compact::minimum_size(tool, s)
    };
    if !s.expanded {
        floater.default_size.x = ctx
            .data(|d| d.get_temp::<Vec2>(egui::Id::unique(INSPECTOR_ID).with("sections_expanded_size")))
            .map_or(width, |size| size.x);
        floater = floater.auto_height();
    }
    floater.show(ctx, p, &mut open, |ui| {
        common::typography(ui);
        if s.legacy_window {
            legacy_toggle_sections = legacy::body(ui, p, tool, world, s, mods, env);
        } else {
            compact::body(ui, p, tool, world, s, mods, env);
        }
    });
    toggle_sections |= legacy_toggle_sections;
    if copy_uuid && !tool.selection.is_empty() {
        ctx.copy_text(tool.copy_keys(world, s, mods.shift));
    }
    if toggle_settings {
        tool.ui.layout.open = !tool.ui.layout.open;
    }
    if !tool.ui.layout.open {
        tool.ui.layout.organizing = false;
        tool.ui.layout.drag = None;
    }
    if tool.ui.grid_options_open {
        tool.ui.layout.open = true;
        tool.ui.grid_options_open = false;
    }
    layout::settings(ctx, p, tool, world, s, mods);
    layout::finish(ctx, p, tool, s);
    notecards::show(ctx, p, tool, world, s);
    contents_selection::finish_if_hidden_or_blurred(ctx, tool, world);
    if toggle_sections {
        let size_id = egui::Id::unique(INSPECTOR_ID).with("sections_expanded_size");
        if s.expanded {
            if let Some(rect) = ctx.memory(|m| m.area_rect(egui::Id::unique(INSPECTOR_ID))) {
                ctx.data_mut(|d| d.insert_temp(size_id, rect.size()));
            }
        } else if let Some(size) = ctx.data(|d| d.get_temp::<Vec2>(size_id)) {
            crate::ui::widgets::size_floater(ctx, INSPECTOR_ID, size);
        }
        s.expanded = !s.expanded;
    }
    floaters::secondary(ctx, p, tool, world, s, env);
    if env.demo
        && crate::ui::color_picker::demo::mode().is_some()
        && !ctx.data(|d| d.get_temp::<bool>(demo_picker).unwrap_or(false))
        && let Some((idx, face)) = tool.sel_te(world).first().copied()
        && let Some(rgb) = world
            .objects
            .get(idx)
            .and_then(|o| o.te.as_ref())
            .map(|te| te.face(face as usize).color)
    {
        tool.toggle_color_picker(world, env, ColorTarget::Diffuse, [rgb[0], rgb[1], rgb[2]]);
        ctx.data_mut(|d| d.insert_temp(demo_picker, true));
    }
    color::show(ctx, p, tool, world, s);
    color::finish_gesture(ctx, tool);
    if !open {
        contents_selection::finish_rename(tool, world);
        tool.cancel_texture_picker(world, env);
        tool.close(world, s);
    }
    *s != before || *env.property_lines != property_lines_before
}

impl BuildTool {
    /// "Use Selection for Grid" (Shift+G): the selected
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
    /// tool, else next / previous prim of the linkset with "Linked edit";
    /// `include` (Shift) adds it to the selection.
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

    pub fn select_next_link(&mut self, world: &mut World, s: &mut BuildSettings, next: bool, include: bool) {
        if !s.edit_linked {
            return;
        }
        let mode = self.edit_mode;
        self.edit_mode = EditMode::Move;
        self.select_next_part(world, s, next, include);
        self.edit_mode = mode;
        if mode == EditMode::Face {
            if !include {
                self.faces.clear();
            }
            if let Some(key) = self.selection.first().copied() {
                self.faces.insert(key, vec![0]);
            }
            self.last_face = Some(0);
        }
    }

    /// "Copy UUID": the roots' ids, or every selected prim with
    /// Shift / "Linked edit" (LLFloaterTools::onClickBtnCopyKeys).
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
