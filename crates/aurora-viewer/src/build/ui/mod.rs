//! The build floater (LLFloaterTools, floater_tools.xml, Firestorm) in the
//! Aurora skin: five editing sections and five tools in one navigation
//! rail. General holds object information and ownership permissions.
//! The five object sections share a Build title; edit selection/actions belong
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
mod detach;
pub(crate) mod export;
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
mod name_focus;
mod object;
pub(crate) mod particles;
mod replace;
mod second_tab;
mod shape_preview;
pub(in crate::build) mod texture;
mod texture_preview;
mod tools;
mod workspace;

use super::{BuildSettings, BuildTool, EditMode, Tool};
use crate::media::entry::ObjectMediaData;
use crate::theme::Palette;
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::{self, Floater};
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
    CopyContents {
        object: Uuid,
        items: Vec<aurora_net::build::TaskItem>,
    },
    /// Focus slider moved (LLAgentCamera::setCameraZoomFraction).
    ZoomFraction(f32),
    AboutLand,
    BuyLand,
    Profile(Uuid),
    GroupProfile(Uuid),
    /// Read a transform vector from the system clipboard (0 position, 1 size,
    /// 2 rotation); shared by Aurora's actions and the Legacy "p" button.
    PasteVector(usize),
    /// The Legacy-style "Copy Keys" button: its copy feedback, shown beside
    /// the button.
    CopyKeys {
        feedback: egui::Id,
        anchor: egui::Rect,
    },
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
    /// What the diffuse, the material and its four override maps may count
    /// on when pasted (`texture_copy::availability`).
    pub image: crate::build::texture_copy::Availability,
    pub material: crate::build::texture_copy::Availability,
    pub maps: [crate::build::texture_copy::Availability; 4],
    /// The normal / specular map could not be copied: `legacy` holds none
    /// and the paste keeps the target's own (NormMapNoCopy, SpecMapNoCopy).
    pub normal_kept: bool,
    pub specular_kept: bool,
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
    /// The projector map could not be copied (`texture_copy::can_copy`):
    /// the paste leaves the target's projector as it is, like the Legacy
    /// Viewer's clipboard without its "spot" entry.
    pub keep_light_image: bool,
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

/// The Contents list filters, after the inventory's (types and required
/// rights), the rights being the next owner's: what someone would get from
/// these items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentsFilters {
    /// One bit per inventory type (LLInventoryType); every bit: no filter.
    pub types: u32,
    /// Rights the next owner must get (copy, modify, transfer bits).
    pub next_owner: u32,
}

impl Default for ContentsFilters {
    fn default() -> Self {
        Self {
            types: u32::MAX,
            next_owner: 0,
        }
    }
}

impl ContentsFilters {
    pub fn active(&self) -> bool {
        *self != Self::default()
    }

    /// The item passes the filters. Objects stored as a coalesced set count
    /// as objects, as in the inventory; the next owner's rights are the
    /// next-owner mask within the base mask.
    pub fn matches(&self, item: &aurora_net::build::TaskItem) -> bool {
        let kind = if item.inv_type == 17 { 6 } else { i32::from(item.inv_type) };
        if self.types != u32::MAX && (!(0..32).contains(&kind) || self.types & (1_u32 << kind) == 0) {
            return false;
        }
        item.next_owner_mask & item.base_mask & self.next_owner == self.next_owner
    }
}

/// The material of the selected faces, waiting for its name (the Legacy
/// Viewer's SaveMaterialAs notification).
pub struct MaterialSaveDraft {
    pub name: String,
    pub data: Vec<u8>,
    pub rights: crate::build::texture_copy::MaterialSave,
}

/// State of the floater kept between frames.
#[derive(Default)]
pub struct FloaterState {
    /// Active presentation, mirrored before input routing (not a preference).
    legacy_window: bool,
    transform_height: Option<f32>,
    status_notice: Option<(String, f64)>,
    pub layout: layout::State,
    pub notecards: notecards::State,
    mesh_inspection: mesh::Inspection,
    material_previews: crate::ui::texture_library::previews::Previews,
    pub color_picker: crate::ui::color_picker::ColorPicker,
    color_edit: Option<color::Session>,
    color_focused: bool,
    /// Texture tab: 0 Classic (Blinn-Phong), 1 PBR, 2 Media.
    pub tex_tab: usize,
    /// A block being dragged out of the build window with Alt.
    pub detach_drag: Option<crate::build::layout::Block>,
    /// Each detached window's rect when the button went down, to tell a
    /// move by its title (put back when dropped on the build window).
    detach_press: std::collections::BTreeMap<crate::build::layout::Block, egui::Rect>,
    /// The section shown beside the active one (Aurora style).
    pub second: second_tab::State,
    /// Section tabs taken out of the build window (shown or not).
    pub detached_tabs: Vec<usize>,
    /// Section tabs shown this frame in a window of their own.
    pub detached_shown: Vec<usize>,
    /// The build window's body height last frame, and the height its shared
    /// controls need without any section (`detach::fit_window`).
    body_height: Option<f32>,
    sectionless_body: Option<f32>,
    /// Windows just taken out, fitting their height to their content for
    /// these many more frames.
    detach_fit: std::collections::BTreeMap<crate::build::layout::Block, u8>,
    /// "Save material as": the name being typed, and what Save will store.
    pub material_save: Option<MaterialSaveDraft>,
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
    /// The Contents filters: item types and the next owner's rights.
    pub contents_filters: ContentsFilters,
    pub contents_filters_open: bool,
    /// Independent from every personal/folder inventory window.
    pub contents_rights: crate::ui::inventory::permissions::Rights,
    /// The language picked for "New script"; None follows the region (Lua
    /// where SLua runs, as LLPanelContents::getState resets it).
    pub new_script_lua: Option<bool>,
    pub contents_selection: contents_selection::Selection,
    /// Whether the inventory held over the Contents tab may go in.
    pub contents_drop: crate::build::contents_drop::PlanCheck,
    /// Item being renamed in the Contents tab.
    pub rename: Option<contents_selection::Rename>,
    /// (object, item) of the contents to open in its window: a notecard,
    /// or the preview of a texture, a sound or an animation.
    pub open_notecard: Option<(Uuid, Uuid)>,
    pub weights_open: bool,
    pub grid_options_open: bool,
    pub group_picker_open: bool,
    pub media_settings: Option<MediaSettingsState>,
    pub confirm: Option<Confirm>,
    pub link_error: Option<String>,
    pub picker: TexturePicker,
    pub pick_target: Option<PickTarget>,
    /// A texture or material dropped on a Texture tab slot, applied next
    /// frame: the slot, the asset and the inventory item it came from.
    pub slot_drop: Option<(PickTarget, Uuid, Uuid)>,
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
    /// Contents items whose properties windows are open, oldest first.
    pub item_properties: Vec<item_properties::TaskProperties>,
    /// CantSelectReflectionProbe shown once.
    pub probe_warned: bool,
    /// Images the swatches want loaded (drained by the app).
    pub wanted_images: HashSet<Uuid>,
    pub reveal_faces: bool,
    /// The panel is the object profile's, not the build tools': its names
    /// follow LLSidepanelTaskInfo::refreshAll under RLVa's `@shownames`
    /// (`general::Names`).
    pub object_profile: bool,
    pub requests: Vec<Request>,
    /// The selection seen last frame (drafts are dropped when it changes).
    last_selection: Vec<ObjKey>,
    /// That selection is one object (one root, or one prim): its contents
    /// are kept listed whatever the tab (`prefetch_contents`).
    single_object: bool,
}

/// LLFloaterTools::refresh → LLPanelObjectInventory::refresh: with one
/// object selected (one root, or one prim), the build window lists its
/// contents whatever the tab, and asks again when its inventory serial
/// changes. A no-copy or no-transfer texture or material chosen in a picker
/// or dropped on a swatch can then go into them at the first try:
/// LLToolDragAndDrop::handleDropMaterialProtections, like `admit_item`,
/// waits ("try again") only while they are still loading.
fn prefetch_contents(tool: &mut BuildTool, world: &World) {
    if tool.ui.single_object
        && let Some(idx) = tool.contents_target(world)
    {
        tool.request_contents(world, idx, false);
    }
}

/// Width of the label column of the tabs.
const LABEL_W: f32 = 112.0;
pub(crate) const INSPECTOR_ID: &str = "build_tools_inspector_c3";

/// The label of the copy feedback once "Copy UUID" or "Copy Keys" copied
/// `count` object ids.
pub(crate) fn copied_keys_label(count: usize) -> String {
    aurora_i18n::tr!("build-ui-copied-object-uuids", count = count)
}

/// The build window's rect, which keeps its copy feedback inside it.
pub(crate) fn window_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.memory(|m| m.area_rect(egui::Id::unique(INSPECTOR_ID)))
        .unwrap_or_else(|| ctx.content_rect())
}

/// LLButton::handleMouseDown (indra/llui/llbutton.cpp) takes the keyboard
/// focus on the press, so a field of the build window commits its typed
/// text to the selection it was typed for before the next part or face,
/// the edit mode or Edit linked act on the release. egui lets the focus go
/// only after the click's pass (ui/click_away_focus.rs), once the selection
/// has already changed. Only a press on the build window itself, outside
/// the focused field: a field's menu and the other windows keep that rule.
/// The world press is app.rs's (`press_while_typing`).
fn press_takes_build_field_focus(ctx: &egui::Context) {
    let Some(pos) = ctx.input(|i| i.pointer.press_origin().filter(|_| i.pointer.any_pressed())) else {
        return;
    };
    if !ctx.text_edit_focused() {
        return;
    }
    let Some(focused) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let Some(field) = ctx.read_response(focused) else {
        return;
    };
    let window = egui::Id::unique(INSPECTOR_ID);
    if field.layer_id.id != window || ctx.layer_id_at(pos).map(|layer| layer.id) != Some(window) || field.interact_rect.contains(pos) {
        return;
    }
    ctx.memory_mut(|m| m.surrender_focus(focused));
}

/// Build shortcuts (menu_viewer.xml): Ctrl+B, Ctrl+1..5, delete, duplicate,
/// link, edit linked parts, grid, undo...
pub fn shortcuts(ctx: &egui::Context, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    // egui drops a text field's focus on Escape before this pass starts, so
    // remember whether one had the keyboard on the previous pass: that
    // Escape is the field's (LLLineEditor reverts the typed text), not the
    // build tool's. Taking it here would hide it from the field, which then
    // commits its text as if focus had moved away.
    // Only the pass just before counts: this runs only while no web page or
    // media face has the keyboard, and an older mark would take an Escape.
    let typing_id = egui::Id::unique("build_shortcuts_typing");
    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
    let typing_before = ctx.data(|d| d.get_temp::<u64>(typing_id)).is_some_and(|typed| typed + 1 == pass);
    // Only a text entry: a focused button or list row leaves Escape to the
    // shortcuts, so Organize mode still closes on the first press.
    // (read before data_mut: egui's context lock is not reentrant)
    if ctx.text_edit_focused() {
        ctx.data_mut(|d| d.insert_temp(typing_id, pass));
    }
    if typing_before && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return;
    }
    if tool.ui.link_error.is_some() {
        return;
    }
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
            physical_key,
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
        // Position for 1–5; consume the logical key so AZERTY's `'` is taken.
        let shortcut_key = keyboard::position_key(key, physical_key);
        if let Some(command) = Shortcut::from_key(shortcut_key, mods, tool.open) {
            // The picker owns Escape in stages (pipette, then dialog).
            // Leave its event intact instead of consuming a no-op Close.
            if command == Shortcut::Close && (tool.ui.color_picker.is_open() || crate::ui::texture_picker::pipette::armed(ctx)) {
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
    floaters::link_error(ctx, p, tool);
    contents_selection::finish_if_hidden_or_blurred(ctx, tool, world);
    if !tool.open {
        tool.ui.status_notice = None;
        tool.status.clear();
        tool.dismiss_texture_picker();
        tool.cancel_color_picker(world);
        tool.commit_color_edit();
        tool.ui.layout.open = false;
        tool.ui.layout.organizing = false;
        tool.ui.layout.drag = None;
        detach::hide(tool);
        detach::park(ctx, s);
        return false;
    }
    // Sampled before a press can surrender the field: an empty click while
    // the window already holds the keyboard commits that field, and does
    // not select it again.
    name_focus::note(ctx);
    name_focus::keep_through_release(ctx);
    let restore_text = name_focus::pending(ctx);
    press_takes_build_field_focus(ctx);
    let before = s.clone();
    let property_lines_before = *env.property_lines;
    // drafts belong to the selection they were typed for
    if tool.ui.last_selection != tool.selection {
        tool.cancel_color_picker(world);
        tool.commit_color_edit();
        tool.ui.last_selection = tool.selection.clone();
        tool.ui.single_object = tool.selection.len() == 1 || tool.roots(world).len() == 1;
        tool.ui.edits.clear();
        tool.ui.sale_draft = None;
        contents_selection::finish_rename(tool, world);
        tool.ui.contents_selection = Default::default();
    }
    prefetch_contents(tool, world);
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
    common::set_wheel_on_hover(ctx, s.wheel_on_hover);
    let preview = egui::Id::unique((INSPECTOR_ID, "compact-preview"));
    if env.demo && !ctx.data(|d| d.get_temp::<bool>(preview).unwrap_or(false)) {
        ctx.data_mut(|d| d.insert_temp(preview, true));
        let mode = std::env::var("AURORA_DEMO_BUILD_TAB").unwrap_or_default();
        if mode.split('+').any(|mode| mode == "folded") {
            s.transform_expanded = false;
        }
    }
    let demo_notice = egui::Id::unique("build-demo-notice");
    if env.demo
        && let Some(item) = ctx.data(|data| data.get_temp::<crate::build::layout::Item>(egui::Id::unique("build-demo-layout-drag")))
    {
        tool.ui.layout.open = true;
        tool.ui.layout.organizing = true;
        if ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) >= 700 {
            tool.ui.layout.drag = Some(item);
        }
    }
    if env.demo
        && ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) >= 700
        && ctx.data(|data| data.get_temp::<bool>(demo_notice)) == Some(false)
    {
        ctx.data_mut(|data| data.insert_temp(demo_notice, true));
        tool.status = aurora_i18n::tr!("build-link-pending").into();
    }
    let width = egui::AreaState::load(ctx, egui::Id::unique(INSPECTOR_ID))
        .and_then(|state| state.size)
        .unwrap_or(workspace::DEFAULT_SIZE)
        .x;
    s.layout.normalize();
    if !s.legacy_window {
        // Aurora uses the floater's standard minimize/restore button.
        s.expanded = true;
    }
    if s.legacy_window && tool.tab == 1 {
        tool.tab = 5;
    }
    if !s.legacy_window && matches!(tool.tab, 0 | 1) {
        tool.tab = 6;
    }
    if !s.legacy_window
        && tool.tool == Tool::Edit
        && (detach::tab_detached(s, tool.tab) || !s.layout.in_bars().iter().any(|item| layout::section(*item) == Some(tool.tab)))
    {
        // A section in a window of its own leaves the build window to the
        // next one still in it (none: the build window shows no section).
        let navigation = s.layout.in_bars();
        let mut tabs = navigation.iter().filter_map(|item| layout::section(*item));
        tool.tab = tabs
            .clone()
            .find(|tab| !detach::tab_detached(s, *tab))
            .or_else(|| detach::tab_detached(s, tool.tab).then_some(tool.tab))
            .or_else(|| tabs.next())
            .unwrap_or(6);
    }
    second_tab::update(tool, s, width);
    detach::prepare(tool, s);
    detach::park(ctx, s);
    tool.ui.layout.targets.clear();
    tool.ui.layout.zones.clear();
    tool.ui.layout.footer_cells.clear();
    let mut open = true;
    let mut legacy_toggle_sections = false;
    let mut copy_uuid: Option<egui::Rect> = None;
    let mut summary_clicked = false;
    let mut settings_clicked = false;
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
        floater = floater.framed_header(title_icon);
        floater.header_status = compact::object_status(tool, world, p);
        floater.header_status_action = Some((
            "clipboard-text-filled",
            aurora_i18n::tr!("build-compact-uuid"),
            aurora_i18n::tr!("build-ui-general-copy-object-keys-shift-all-parts"),
            &mut copy_uuid,
        ));
        // the build settings, under UUID (the human's request, 2026-11-22)
        floater.header_summary_icon = Some(("settings-filled", aurora_i18n::tr!("build-layout-settings"), &mut settings_clicked));
        if tool.tool == Tool::Edit {
            floater.header_summary = Some(compact::selection_summary(p, tool, world).into());
            if !tool.selection.is_empty() {
                floater.header_summary_action = Some(&mut summary_clicked);
            }
        } else {
            // the selection line's room, so the bars do not move between tools
            floater.header_summary = Some(egui::RichText::new(" ").size(common::BODY).into());
        }
    }
    // Keep the user's Build size. Large Appearance › Text size reflows inside
    // the floater (shape fields and Legacy Texture slots); it must not shove
    // the window wider or taller on its own.
    floater.min_size = if s.legacy_window {
        legacy::MIN_SIZE
    } else {
        compact::minimum_size(tool, s)
    };
    if detach::sectionless(tool, s) {
        // Its height is set to its shared controls (detach::fit_window).
        floater.min_size.y = 100.0;
    }
    if !s.expanded {
        floater.default_size.x = ctx
            .data(|d| d.get_temp::<Vec2>(egui::Id::unique(INSPECTOR_ID).with("sections_expanded_size")))
            .map_or(width, |size| size.x);
        floater = floater.auto_height();
    }
    let uuid_feedback = egui::Id::unique(INSPECTOR_ID).with("copy_uuid");
    // Outside the viewer's window, the settings panel has no place beside
    // the build window: its window there keeps the room for it, and the
    // panel is drawn in the window's own pass, with the control an
    // organization carries, which follows that window's pointer.
    let outside = widgets::floater_outside(ctx, egui::Id::unique(INSPECTOR_ID));
    if layout::settings_shown(ctx, tool) {
        floater.side_outside = layout::SETTINGS_ROOM;
    }
    floater.show(ctx, p, &mut open, |ui| {
        common::typography(ui);
        // (in the window's own pass: outside the viewer's, it is another window)
        widgets::copied_feedback(ui.ctx(), p, uuid_feedback, window_rect(ui.ctx()));
        if s.legacy_window {
            legacy_toggle_sections = legacy::body(ui, p, tool, world, s, mods, env);
        } else {
            compact::body(ui, p, tool, world, s, mods, env);
        }
        if outside {
            let room = widgets::side_room_outside(ui.ctx(), INSPECTOR_ID);
            layout::settings(ui.ctx(), p, tool, world, s, mods, layout::Beside::Outside(room));
            layout::finish(ui.ctx(), p, tool, s);
        }
    });
    if restore_text {
        name_focus::apply(ctx);
    }
    if !s.legacy_window {
        detach::fit_window(ctx, tool, s);
    }
    if let Some(size) = widgets::take_floater_resize(ctx, INSPECTOR_ID) {
        second_tab::resized(tool, size.x);
    }
    if let Some(width) = second_tab::take_request(tool) {
        let height = egui::AreaState::load(ctx, egui::Id::unique(INSPECTOR_ID))
            .and_then(|state| state.size)
            .unwrap_or(workspace::DEFAULT_SIZE)
            .y;
        widgets::size_floater(ctx, INSPECTOR_ID, Vec2::new(width, height));
    }
    if summary_clicked {
        tool.ui.weights_open = !tool.ui.weights_open;
    }
    if settings_clicked {
        layout::toggle_settings(tool);
    }
    if let Some((target, asset, item)) = tool.ui.slot_drop.take() {
        let sel = texture::selected(tool, world);
        match tool.admit_item(world, item, &texture_preview::admit_targets(target, &sel)) {
            Ok(()) => tool.apply_texture(world, env, target, asset, &sel),
            Err(text) => world.system_notice(text, crate::world::notice_topics::Topic::Items),
        }
    }
    if let Some(anchor) = copy_uuid
        && !tool.selection.is_empty()
    {
        let (keys, count) = tool.copy_keys_with_count(world, s, mods.shift);
        if count > 0 {
            ctx.copy_text(keys);
            widgets::show_copied(ctx, uuid_feedback, anchor, copied_keys_label(count));
        }
    }
    if !tool.ui.layout.open {
        tool.ui.layout.organizing = false;
        tool.ui.layout.drag = None;
    }
    if tool.ui.grid_options_open {
        tool.ui.layout.open = true;
        tool.ui.grid_options_open = false;
    }
    detach::finish(ctx, p, tool, s);
    detach::windows(ctx, p, tool, world, s, mods, env);
    if !outside {
        layout::settings(ctx, p, tool, world, s, mods, layout::Beside::Window);
        layout::finish(ctx, p, tool, s);
    }
    notecards::show(ctx, p, tool, world, s);
    contents_selection::finish_if_hidden_or_blurred(ctx, tool, world);
    if legacy_toggle_sections {
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
        tool.dismiss_texture_picker();
        tool.close(world, s);
    }
    status_notice(ctx, p, tool);
    *s != before || *env.property_lines != property_lines_before
}

fn status_notice(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool) {
    let now = ctx.input(|input| input.time);
    if !tool.status.is_empty() {
        tool.ui.status_notice = Some((std::mem::take(&mut tool.status), now + 6.0));
    }
    if !tool.open || tool.ui.status_notice.as_ref().is_some_and(|(_, until)| now >= *until) {
        tool.ui.status_notice = None;
    }
    let Some((text, until)) = &tool.ui.status_notice else { return };
    let Some(anchor) = ctx.memory(|memory| memory.area_rect(egui::Id::unique(INSPECTOR_ID))) else {
        return;
    };
    // An opaque themed surface keeps text legible over emissive objects and
    // bright skies without changing the editor's measured content height.
    let opaque = Palette {
        panel_alpha: u8::MAX,
        ..*p
    };
    egui::Area::new(egui::Id::unique("construction-status-notice"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom() + egui::vec2(0.0, 6.0))
        .constrain_to(ctx.content_rect().shrink(6.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_width(anchor.width().max(120.0));
            widgets::overlay(ui, &opaque, egui::Margin::same(8), |ui| {
                common::text(ui, &opaque, text);
            });
        });
    ctx.request_repaint_after(std::time::Duration::from_secs_f64((*until - now).max(0.0)));
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

    /// Ctrl+, / Ctrl+. and the Legacy style's ◄ ► (LLToolsSelectNextPartFace,
    /// indra/newview/lltoolmgr.cpp): with "Edit linked" outside "Select
    /// face", the next / previous prim of the linkset; else the face arrows'
    /// step (`select_next_face`), as their ◄ ► do in the Aurora style (the
    /// human's choice, 2026-11-23). `include` (Shift) adds to the selection.
    pub fn select_next_part(&mut self, world: &mut World, s: &mut BuildSettings, next: bool, include: bool) {
        let face_mode = self.tool == Tool::Edit && self.edit_mode == EditMode::Face;
        if s.edit_linked && !face_mode {
            let Some(idx) = self.selection.first().and_then(|key| world.objects.index_of(key)) else {
                return;
            };
            self.step_link(world, idx, next, include, false);
        } else {
            self.select_next_face(world, s.edit_linked, next, include);
        }
    }

    /// The faces of the one prim that has faces picked, one by one
    /// (LLToolsSelectNextPartFace's face step): past its last face, the next
    /// / previous prim of the linkset with "Edit linked", else around the
    /// same prim. A sole selected face is already the cursor, not a bulk
    /// selection.
    pub(in crate::build) fn step_prim_face(&mut self, world: &mut World, idx: usize, edit_linked: bool, next: bool, include: bool) {
        let Some(key) = world.objects.get(idx).map(|o| o.key) else { return };
        let n = self.num_faces(world, idx) as i32;
        let cur = self.last_face.map(|f| f as i32).unwrap_or(-1);
        let all = self.faces.get(&key).is_none_or(|f| f.len() as i32 >= n);
        let f = if all && next && (n > 1 || self.selection.len() > 1) {
            0
        } else if next {
            cur + 1
        } else {
            cur - 1
        };
        if (0..n).contains(&f) {
            if !include {
                let others: Vec<_> = self.selection.iter().copied().filter(|other| *other != key).collect();
                self.deselect(world, &others);
                // the prims left with it (its linkset kept whole) keep no face
                for prim in self.sel_prims(world) {
                    if let Some(other) = world.objects.get(prim).map(|o| o.key).filter(|other| *other != key) {
                        self.faces.insert(other, Vec::new());
                    }
                }
                self.faces.insert(key, Vec::new());
            }
            let entry = self.faces.entry(key).or_default();
            if !entry.contains(&(f as u8)) {
                entry.push(f as u8);
            }
            self.last_face = Some(f as u8);
        } else if edit_linked {
            self.step_link(world, idx, next, include, true);
        } else {
            // around the faces of the same prim
            let f = if next { 0 } else { n - 1 };
            self.faces.insert(key, vec![f as u8]);
            self.last_face = Some(f as u8);
        }
    }

    /// The next / previous prim of `idx`'s linkset, root first, then the
    /// children in order; `with_face`: picked with its first / last face.
    fn step_link(&mut self, world: &mut World, idx: usize, next: bool, include: bool, with_face: bool) {
        let root = super::root_of(world, idx);
        let list: Vec<usize> = super::family(world, root)
            .into_iter()
            .filter(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar()))
            .collect();
        let Some(pos) = list.iter().position(|&i| i == idx) else { return };
        let n = list.len();
        // LLToolsSelectNextPartFace skips selected links when extending a selection.
        let target = (1..=n)
            .map(|step| list[if next { (pos + step) % n } else { (pos + n - step) % n }])
            .find(|&idx| !include || world.objects.get(idx).is_some_and(|o| !self.is_selected(&o.key)));
        let Some(target) = target else { return };
        let Some(unit) = world.objects.get(target).map(|o| o.key) else {
            return;
        };
        if !include {
            self.deselect_all(world);
        }
        self.selection_linked = true;
        self.add(world, unit);
        if with_face {
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
        let Some(idx) = self.selection.first().and_then(|key| world.objects.index_of(key)) else {
            return;
        };
        self.step_link(world, idx, next, include, false);
        if self.edit_mode == EditMode::Face {
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
    /// Also returns how many ids it holds, for the copy feedback.
    pub fn copy_keys_with_count(&self, world: &World, s: &BuildSettings, all_prims: bool) -> (String, usize) {
        let idxs = if all_prims || s.edit_linked {
            self.sel_prims(world)
        } else {
            self.roots(world)
        };
        let ids: Vec<String> = idxs
            .iter()
            .filter_map(|&i| world.objects.get(i).map(|o| o.full_id.to_string()))
            .collect();
        let count = ids.len();
        (ids.join(&s.copy_key_separator), count)
    }
}
