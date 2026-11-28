//! The linkset panel of an object's script editor: every script of the
//! linkset beside the source, grouped by the prim that holds it and marked
//! with a star while its open document has unsaved text. Aurora's own layout
//! (the Legacy Viewer opens one floater per script, LLPreviewLSL /
//! LLLiveLSLEditor, indra/newview/llpreviewscript.cpp): the tree takes the
//! rows of the inventory's contents (`ui/inventory/tree.rs`).

use super::super::inventory::permissions::Permissions;
use super::super::widgets;
use super::super::{icons, inventory, menu};
use super::StageView;
use crate::object_stage::Absent;
use crate::theme::Palette;
use crate::ui::object_stage::Showing;
use aurora_net::scripts::ScriptTarget;
use egui::{Id, RichText, Vec2};
use uuid::Uuid;

/// Room the tree keeps beside the source, and the least it may be dragged to.
pub const TREE_WIDTH: f32 = 210.0;
pub const TREE_MIN: f32 = 140.0;
/// The row of the tree's foot (the settings), and the height of a row of the
/// list: the inventory sidebar's own.
const FOOT: f32 = 32.0;
const ROW: f32 = 24.0;
/// The foot's Properties section: its title row, the facts under it, and the
/// room the 3D view of the object takes above them.
const PROPERTIES_HEAD: f32 = ROW;
const PROPERTIES_BODY: f32 = 52.0;
/// The view's caption: the object it shows, and what a click does.
const STAGE_CAPTION: f32 = 18.0;
const STAGE_VIEW: f32 = 128.0;

/// What the contents window's menu offers for one script of the tree, as the
/// app decided it: the tree applies the contents window's own rules, it does
/// not read permissions itself.
#[derive(Clone, Copy)]
pub struct ScriptMenu {
    /// "Move to inventory" rather than "Copy to inventory": the item leaves
    /// the primitive (it has no Copy right).
    pub moves: bool,
    /// Why the copy is refused, when it is: the entry is greyed with it.
    pub refused: Option<&'static str>,
    /// The hint of the entry when it is offered.
    pub tip: &'static str,
    /// The prim may be modified: Delete is offered.
    pub can_delete: bool,
    /// The item may be changed in the object's contents: Stop / Run offered.
    pub can_control: bool,
}

/// A script of the tree, as the tree shows it.
#[derive(Clone)]
pub struct Script {
    /// The prim the script is in; nil for one of the agent's inventory.
    pub object: Uuid,
    pub item: Uuid,
    pub name: String,
    /// The item's flags: an SLua script gets the Lua icon.
    pub flags: u32,
    /// Whether it runs, once the simulator answered.
    pub running: Option<bool>,
    /// The rights the contents window shows for it (its C/M/T letters).
    pub permissions: Permissions,
    /// Its open document has unsaved text: the row marks it with a star, in
    /// place of a tab strip's dot.
    pub dirty: bool,
    /// The contents window's menu for it; none for a script of the inventory,
    /// which has no contents window to act in.
    pub menu: Option<ScriptMenu>,
}

/// What a group of the tree is: a prim of the linkset, or a folder of the
/// agent's inventory. The same window draws either, grouped the same way.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PrimKind {
    Prim,
    Folder,
}

/// What the object menu offers for the linkset's root, as the app decided
/// it: Take, Take copy and Delete by the object menu's own rules
/// (enable_take, enable_object_take_copy, LLSelectMgr::canDoDelete, and the
/// locks RLVa puts on them), which the tree does not read itself.
#[derive(Clone, Copy, Default)]
pub struct PrimMenu {
    pub take: bool,
    pub take_copy: bool,
    pub delete: bool,
}

/// A group of the tree: a prim of the linkset, or a folder of the inventory.
#[derive(Clone)]
pub struct Prim {
    /// The prim, or the inventory folder that groups the scripts.
    pub object: Uuid,
    pub kind: PrimKind,
    pub name: String,
    /// How far it is nested: 0 at the top, one more per prim it is linked
    /// under (or folder it is in).
    pub depth: u8,
    /// The folder's preferred type, for its icon (0 for a prim).
    pub type_default: i32,
    /// Its listing is on its way, or was refused: the group does not claim
    /// to hold no script.
    pub loading: bool,
    /// The prim may be modified: "New script" is offered.
    pub can_new_script: bool,
    /// RLVa allows a touch on it (RlvActions::canTouch): the prim's menu offers
    /// "Touch".
    pub touch: bool,
    /// The object menu's Take, Take copy and Delete: offered on the root of a
    /// linkset alone (a child cannot be taken on its own), and never for a
    /// folder of the inventory.
    pub menu: Option<PrimMenu>,
    pub scripts: Vec<Script>,
}

/// Every script one window shows, grouped: an object's linkset by prim, or
/// the agent's inventory by folder, as the app hands it to the editor each
/// frame.
pub struct Linkset {
    /// The linkset's root, or `super::INVENTORY_WINDOW`.
    pub root: Uuid,
    /// The object's name, or "Inventory" for the agent's own scripts.
    pub name: String,
    /// A quiet line above the groups: the inventory still loading, or none
    /// holding a script (none for an object's tree).
    pub note: Option<String>,
    pub prims: Vec<Prim>,
}

pub(super) fn collapsed_id(root: Uuid) -> Id {
    Id::unique(("script_linkset_collapsed", root))
}

/// The id the tree keeps the root prim's row under, as the last frame drew
/// it and on which layer: the demo opens that row's menu, which no synthetic
/// click can aim at otherwise.
pub(crate) fn root_row_id(root: Uuid) -> Id {
    Id::unique(("script_linkset_root_row", root))
}

/// The id the tree keeps a script's row under, as the last frame drew it and
/// on which layer: the demo of its menu needs the same room.
pub(crate) fn script_row_id(root: Uuid, item: Uuid) -> Id {
    Id::unique(("script_linkset_script_row", root, item))
}

/// The prims whose group is shut, kept with the window's state.
fn collapsed(ctx: &egui::Context, root: Uuid) -> Vec<Uuid> {
    ctx.data_mut(|d| d.get_persisted::<Vec<Uuid>>(collapsed_id(root)))
        .unwrap_or_default()
}

/// Opens the group of `prim` in the tree of `root`, and those of the prims
/// that hold it: a script chosen or brought to the front shows there, even
/// inside a group the reader had shut.
pub(super) fn reveal(ctx: &egui::Context, root: Uuid, linkset: &Linkset, prim: Uuid) {
    let Some(index) = linkset.prims.iter().position(|p| p.object == prim) else {
        return;
    };
    // the prim, and every prim before it that is one level less deep, back to
    // the root
    let mut open = vec![prim];
    let mut depth = linkset.prims[index].depth;
    for earlier in linkset.prims[..index].iter().rev() {
        if earlier.depth < depth {
            open.push(earlier.object);
            depth = earlier.depth;
            if depth == 0 {
                break;
            }
        }
    }
    let mut closed = collapsed(ctx, root);
    let before = closed.len();
    closed.retain(|id| !open.contains(id));
    if closed.len() != before {
        ctx.data_mut(|d| d.insert_persisted(collapsed_id(root), closed));
    }
}

/// The room the tree's top level keeps at the list's left edge.
const TREE_LEFT: f32 = 4.0;
/// The Properties header's icon stands level with the Settings cog below it
/// (`rail_tree_row` puts that icon 18 points in), and its facts under the
/// title's words.
const PROPERTIES_LEFT: f32 = 10.0;
const PROPERTIES_TEXT: f32 = 30.0;
/// How much a group under another indents its rows: the inventory sidebar's
/// own step (a prim linked under another, or a folder inside one).
const INDENT: f32 = 14.0;
/// The deepest level an indent shows: a linkset may nest further, and the
/// names would have no room left.
const DEEPEST: u8 = 12;
/// The room a prim's caret takes before its icon, as the inventory sidebar's
/// folder rows do.
const CARET_ROOM: f32 = 20.0;
/// How far a script's row steps in under its group's: past that caret's room,
/// where the group's own icon stands, and one step more (the step a group
/// under another adds), so its icon stands with the rows of a group nested
/// under the one it is in.
const SCRIPT_STEP: f32 = CARET_ROOM + INDENT;

/// One row of the tree, drawn as the inventory sidebar draws its folders: 24
/// points tall, the whole row -- icon and words -- lit when it is the chosen
/// one or under the pointer, a caret before the icon when the row opens a
/// group, and a quiet word at its right end (that a script runs) whose room
/// the name gives up rather than the word. Returns the row's response and the
/// caret's, with its open state.
#[allow(clippy::too_many_arguments)]
pub(super) fn row(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    tint: egui::Color32,
    mut label: egui::text::LayoutJob,
    trailing: Option<(&str, egui::TextFormat)>,
    selected: bool,
    left: f32,
    caret: Option<bool>,
    draggable: bool,
) -> (egui::Response, Option<(egui::Response, bool)>) {
    let sense = if draggable {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), sense);
    // the caret is registered after the row, so a click on it is the caret's
    let caret = caret.map(|open| {
        let hit = egui::Rect::from_center_size(egui::pos2(rect.left() + left + 7.0, rect.center().y), Vec2::splat(18.0));
        (ui.interact(hit, response.id.with("caret"), egui::Sense::click()), open)
    });
    // the face of the row, behind everything it holds
    let hot = response.hovered() || caret.as_ref().is_some_and(|(caret, _)| caret.hovered());
    if selected || hot {
        ui.painter()
            .rect_filled(rect, ui.visuals().window_corner_radius, p.row_face(selected, hot));
    }
    if let Some((caret, open)) = &caret {
        let color = if caret.hovered() {
            p.ink
        } else if selected {
            p.violet_pale
        } else {
            p.muted
        };
        widgets::paint_glyph(ui.painter(), widgets::disclosure_caret(*open), caret.rect.center(), 12.0, color);
    }
    let icon_left = rect.left() + left + if caret.is_some() { CARET_ROOM } else { 0.0 };
    if let Some(t) = icons::global(icon) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(icon_left + 8.0, rect.center().y), Vec2::splat(16.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    let text_left = icon_left + 20.0;
    // the word at the right end keeps its room: a long name truncates before
    // it, and the tooltip has the whole name anyway
    let trailing = trailing.map(|(text, format)| {
        (
            ui.painter().layout_no_wrap(text.to_owned(), format.font_id.clone(), format.color),
            format.color,
        )
    });
    let trailing_room = trailing.as_ref().map_or(0.0, |(galley, _)| galley.size().x + 6.0);
    label.wrap = egui::text::TextWrapping {
        max_width: (rect.right() - 8.0 - text_left - trailing_room).max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.painter().layout_job(label);
    widgets::paint_row_text(ui.painter(), galley, text_left, rect.center().y, p.ink);
    if let Some((galley, color)) = trailing {
        let at = rect.right() - 8.0 - galley.size().x;
        widgets::paint_row_text(ui.painter(), galley, at, rect.center().y, color);
    }
    (response, caret)
}

/// The row icon of a name of the skin's set (Tabler and inventory icons
/// alike), `16` points wide, or the room it takes when the set has none.
pub(super) fn icon(ui: &mut egui::Ui, name: &str, tint: egui::Color32) {
    if let Some(t) = icons::global(name) {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
    } else {
        ui.add_space(16.0);
    }
}

/// What a row of the tree asked for, from a click or its context menu.
pub(super) enum TreeAction {
    /// Open the script in the editor, or bring it to the front.
    Open(ScriptTarget),
    /// Close its document (middle click, or the menu's Close).
    Close(ScriptTarget),
    Take(ScriptTarget),
    Delete(ScriptTarget),
    /// The item's Properties window, as the contents menu opens it.
    Properties(ScriptTarget),
    /// Stop / Run of the script, as the contents menu offers it (`running`
    /// is what the click asks for).
    SetRunning {
        target: ScriptTarget,
        running: bool,
    },
    /// A click chose these rows, and the script to open when the click was a
    /// plain one (Ctrl and Shift change the selection alone).
    Select {
        targets: Vec<ScriptTarget>,
        open: Option<ScriptTarget>,
    },
    /// The menu's entries on a selection of several scripts.
    OpenMany(Vec<ScriptTarget>),
    TakeMany(Vec<ScriptTarget>),
    DeleteMany(Vec<ScriptTarget>),
    SaveMany(Vec<ScriptTarget>),
    /// A new script in that prim's contents.
    NewScript(Uuid),
    /// Take the linkset's root into the inventory (`copy`: Take copy, which
    /// leaves it rezzed), the object menu's own action.
    TakeObject {
        object: Uuid,
        copy: bool,
    },
    /// Delete the linkset's root, the warning the object menu shows first.
    DeleteObject(Uuid),
    /// A touch on the prim: its menu's Touch, or a click in the 3D view.
    Touch(Uuid),
    /// A script dragged out of the tree and let go over the source: the pane
    /// it was dropped on, left or right, opens it beside the one in front.
    Dropped {
        target: ScriptTarget,
        pos: egui::Pos2,
    },
    /// The prim itself, in the build floater's Edit tool.
    Edit(Uuid),
    /// The foot of the tree: the script editor's own preferences.
    Settings,
}

/// The script a target names in this tree, when the tree lists it.
fn listed(linkset: &Linkset, target: ScriptTarget) -> Option<&Script> {
    linkset
        .prims
        .iter()
        .flat_map(|prim| &prim.scripts)
        .find(|script| script.object == target.object && script.item == target.item)
}

/// The contents menu of a script of this tree, when it has one.
fn menu_of(linkset: &Linkset, target: ScriptTarget) -> Option<ScriptMenu> {
    listed(linkset, target).and_then(|script| script.menu)
}

/// Whether a script of this tree has text that the window could save.
fn dirty_of(linkset: &Linkset, target: ScriptTarget) -> bool {
    listed(linkset, target).is_some_and(|script| script.dirty)
}

/// The rows a Shift click takes: from the anchor (the script in front) to the
/// clicked one, in the list's order, both ends included.
fn shift_selection(visible: &[ScriptTarget], anchor: Option<ScriptTarget>, clicked: ScriptTarget) -> Vec<ScriptTarget> {
    let Some(anchor) = anchor.and_then(|anchor| visible.iter().position(|target| *target == anchor)) else {
        return vec![clicked];
    };
    let Some(at) = visible.iter().position(|target| *target == clicked) else {
        return vec![clicked];
    };
    let (from, to) = if anchor <= at { (anchor, at) } else { (at, anchor) };
    visible[from..=to].to_vec()
}

/// The filter field over the tree: hides the scripts whose name does not
/// contain its text (a prim whose name matches keeps all of its scripts).
pub(super) fn filter(ui: &mut egui::Ui, p: &Palette, text: &mut String) {
    // the same field as the documentation pane's and the console's; empty, it
    // carries the keys of Quick Open, the other way to a script of the window
    let width = ui.available_width();
    let field = widgets::search_field(ui, p, text, aurora_i18n::tr!("script-linkset-filter"), width);
    if text.is_empty() && !field.has_focus() {
        let binding = crate::keybinds::Binding {
            input: crate::keybinds::Input::Key("KeyP".into()),
            ctrl: true,
            shift: false,
            alt: false,
        };
        let caps = widgets::keycap::caps(ui, &binding, p.field);
        let room = widgets::keycap::row_width(&caps);
        let hint = aurora_i18n::tr!("script-linkset-filter");
        let hint = ui
            .painter()
            .layout_no_wrap(hint.to_string(), egui::TextStyle::Body.resolve(ui.style()), p.muted)
            .size()
            .x;
        // (nothing over the hint in a narrow tree)
        if hint + room + 40.0 < field.rect.width() {
            let at = egui::pos2(field.rect.right() - 8.0 - room, field.rect.center().y);
            widgets::keycap::paint_row(ui, p, &caps, at, p.field);
        }
    }
}

/// The tree of the linkset: a group per prim, its scripts under it. Returns
/// what a click or a menu entry of a row asked for. `filter_text` is the
/// field's own text, drawn at the top of the panel: the scripts whose name
/// does not contain it are hidden (a prim whose name matches keeps all of its
/// scripts).
pub(super) fn tree(
    ui: &mut egui::Ui,
    p: &Palette,
    linkset: Option<&Linkset>,
    active: Option<ScriptTarget>,
    selection: &[ScriptTarget],
    filter_text: &mut String,
    drag_id: egui::Id,
    stage: Option<&mut StageView>,
) -> Option<TreeAction> {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    // the list's own area: a drop outside it is a drop on the source
    let panel = ui.max_rect();
    // the filter's row, inside the panel, then the list, then the foot, which
    // holds one row as the inventory tree's holds its Trash
    let field = ui.spacing().interact_size.y.max(24.0) + 6.0;
    // the script in front, for the foot's Properties section: its language,
    // its rights and its state, and the window's own window of them
    let properties = linkset.and_then(|linkset| {
        let target = active?;
        let script = listed(linkset, target)?;
        Some((target, script.name.clone(), script.flags, script.permissions, script.menu.is_some()))
    });
    // the section keeps its room in a window of a linkset even while the
    // page of a new script is in front: the tree's list does not grow or
    // shrink when the page opens and closes
    // the view of the object takes its room above the facts, when the window
    // has one and the setting shows it: its caption and the studio, and the
    // two points of air around them
    let preview = if stage.is_some() { STAGE_CAPTION + STAGE_VIEW + 6.0 } else { 0.0 };
    let properties_height = if linkset.is_some() {
        PROPERTIES_HEAD + PROPERTIES_BODY + preview
    } else {
        0.0
    };
    let foot = FOOT + properties_height;
    let height = (ui.available_height() - field - 6.0 - foot).max(60.0);
    inventory::tree::frame(ui, p, |ui| {
        // the panel reaches the window's bottom, as the source's status bar
        // does, rather than stopping under its last row
        ui.set_min_height(ui.available_height());
        inventory::tree::spacing(ui);
        // the filter belongs to the list it filters: inside its panel, above
        // its first row
        filter(ui, p, filter_text);
        ui.add_space(6.0);
        let needle = filter_text.trim().to_lowercase();
        let needle = needle.as_str();
        inventory::tree::scroll(ui, height).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(linkset) = linkset else {
                ui.label(RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.0).color(p.muted));
                return;
            };
            let mut closed = collapsed(&ctx, linkset.root);
            let filtering = !needle.is_empty();

            // a quiet word above the groups: the inventory still loading, or
            // none holding a script
            if let Some(note) = &linkset.note {
                ui.label(RichText::new(note.as_str()).size(12.0).color(p.muted));
                ui.add_space(4.0);
            }
            // the scripts the list shows, in their order, with the groups
            // folded and the filter on: the range a Shift click takes
            let mut visible: Vec<ScriptTarget> = Vec::new();
            {
                let mut shut: Option<u8> = None;
                for prim in &linkset.prims {
                    if shut.is_some_and(|depth| prim.depth > depth) {
                        continue;
                    }
                    shut = None;
                    let prim_matches = prim.name.to_lowercase().contains(needle);
                    if !filtering && closed.contains(&prim.object) {
                        shut = Some(prim.depth);
                        continue;
                    }
                    for script in &prim.scripts {
                        if filtering && !prim_matches && !script.name.to_lowercase().contains(needle) {
                            continue;
                        }
                        visible.push(ScriptTarget {
                            object: script.object,
                            item: script.item,
                        });
                    }
                }
            }
            // the depth of a folded group, whose own groups under it go with it
            let mut shut_below: Option<u8> = None;
            for prim in &linkset.prims {
                if shut_below.is_some_and(|depth| prim.depth > depth) {
                    continue;
                }
                shut_below = None;
                let prim_matches = prim.name.to_lowercase().contains(needle);
                let scripts: Vec<&Script> = prim
                    .scripts
                    .iter()
                    .filter(|script| !filtering || prim_matches || script.name.to_lowercase().contains(needle))
                    .collect();
                if filtering && !prim_matches && scripts.is_empty() {
                    continue;
                }
                let open = filtering || !closed.contains(&prim.object);

                // a prim of the linkset, or a folder of the inventory, one
                // step in under the group that holds it
                let left = TREE_LEFT + f32::from(prim.depth.min(DEEPEST)) * INDENT;
                let icon = match prim.kind {
                    PrimKind::Prim => "Inv_Object",
                    PrimKind::Folder => inventory::tree::folder_icon(prim.type_default, open),
                };
                let (label, caret) = row(
                    ui,
                    p,
                    icon,
                    p.violet_light,
                    inventory::tree::name(&prim.name, p),
                    None,
                    false,
                    left,
                    Some(open),
                    false,
                );
                let caret_clicked = caret.is_some_and(|(caret, _)| caret.clicked());
                if caret_clicked || label.clicked() {
                    if open {
                        closed.push(prim.object);
                    } else {
                        closed.retain(|id| *id != prim.object);
                    }
                }
                let label = label.on_hover_text(if prim.loading {
                    aurora_i18n::tr!("script-linkset-loading", name = prim.name.as_str())
                } else {
                    aurora_i18n::tr!("script-linkset-count", name = prim.name.as_str(), count = prim.scripts.len() as i64)
                });
                // where the root's row is, on which window, for the demo of
                // its menu
                if prim.kind == PrimKind::Prim && prim.depth == 0 {
                    let row = (label.rect, ui.layer_id());
                    ctx.data_mut(|d| d.insert_temp(root_row_id(linkset.root), row));
                }
                if prim.kind == PrimKind::Prim {
                    menu::context_menu(&label, p, |ui| {
                        // the object menu's Touch, on the prim itself
                        if menu::item_if(ui, p, "click-filled", aurora_i18n::tr!("ui-context-touch"), prim.touch) {
                            chosen = Some(TreeAction::Touch(prim.object));
                        }
                        menu::separator(ui, p);
                        // the prim itself, in the build floater's Edit tool
                        if menu::item(ui, p, "pencil", aurora_i18n::tr!("common-edit")) {
                            chosen = Some(TreeAction::Edit(prim.object));
                        }
                        if menu::item_if(ui, p, "plus", aurora_i18n::tr!("common-new-script"), prim.can_new_script) {
                            chosen = Some(TreeAction::NewScript(prim.object));
                        }
                        // the object menu's entries for the linkset, with the
                        // rights the app worked out for it
                        if let Some(actions) = prim.menu {
                            menu::separator(ui, p);
                            if menu::item_if(ui, p, "briefcase-filled", aurora_i18n::tr!("ui-context-take"), actions.take) {
                                chosen = Some(TreeAction::TakeObject {
                                    object: prim.object,
                                    copy: false,
                                });
                            }
                            if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("ui-context-take-copy"), actions.take_copy) {
                                chosen = Some(TreeAction::TakeObject {
                                    object: prim.object,
                                    copy: true,
                                });
                            }
                            if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), actions.delete) {
                                chosen = Some(TreeAction::DeleteObject(prim.object));
                            }
                        }
                    });
                }
                if !open {
                    // its scripts go, and the groups under it too
                    shut_below = Some(prim.depth);
                    continue;
                }
                for script in scripts {
                    let target = ScriptTarget {
                        object: script.object,
                        item: script.item,
                    };
                    {
                        let running = script.running == Some(true);
                        let tint = if running { p.teal } else { p.muted };
                        // the script's name with the star of unsaved text
                        // (LLItemBridge::getLabelSuffix), and at the right end of
                        // the row, where the contents window shows its C/M/T
                        // letters, that it runs: its language is its icon's, and
                        // its tooltip says it in words with the rights
                        let name =
                            widgets::name_with_suffix(&script.name, if script.dirty { " *" } else { "" }, inventory::tree::font(), p.ink);
                        let badge = running.then(|| aurora_i18n::tr!("script-running").to_string());
                        let badge = badge.as_deref().map(|text| {
                            (
                                text,
                                crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 11.0, p.muted),
                            )
                        });
                        let selected = active == Some(target) || selection.contains(&target);
                        let (response, _) = row(
                            ui,
                            p,
                            inventory::item_icon(10, 10, script.flags),
                            tint,
                            name,
                            badge,
                            selected,
                            left + SCRIPT_STEP,
                            None,
                            true,
                        );
                        // where the row is, for the demo of its menu
                        {
                            let at = (response.rect, ui.layer_id());
                            ctx.data_mut(|d| d.insert_temp(script_row_id(linkset.root, script.item), at));
                        }
                        // a click: one script, one more (Ctrl), or the range
                        // from the one in front (Shift); only a plain click
                        // opens it
                        if response.clicked() {
                            let (command, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                            if shift {
                                let anchor = selection.last().copied().or(active);
                                chosen = Some(TreeAction::Select {
                                    targets: shift_selection(&visible, anchor, target),
                                    open: None,
                                });
                            } else if command {
                                let mut targets = selection.to_vec();
                                if let Some(at) = targets.iter().position(|chosen| *chosen == target) {
                                    targets.remove(at);
                                } else {
                                    targets.push(target);
                                }
                                if targets.is_empty() {
                                    targets.push(target);
                                }
                                chosen = Some(TreeAction::Select { targets, open: None });
                            } else {
                                chosen = Some(TreeAction::Select {
                                    targets: vec![target],
                                    open: Some(target),
                                });
                            }
                        }
                        if response.middle_clicked() {
                            chosen = Some(TreeAction::Close(target));
                        }
                        if response.drag_started() {
                            ctx.data_mut(|d| d.insert_temp(drag_id, Some((target, script.name.clone()))));
                        }
                        let response = response.on_hover_cursor(egui::CursorIcon::Grab);
                        // the menu acts on the selection the row is part of; a
                        // right click outside it chooses that row alone
                        let in_selection = selection.contains(&target);
                        let targets: Vec<ScriptTarget> = if in_selection && selection.len() > 1 {
                            selection.to_vec()
                        } else {
                            vec![target]
                        };
                        let many = targets.len() > 1;
                        if response.secondary_clicked() && !in_selection {
                            chosen = Some(TreeAction::Select {
                                targets: vec![target],
                                open: None,
                            });
                        }
                        // what the row says under the pointer, built only while
                        // it is on it: its name, that the script runs, the
                        // language in words, then the rights
                        let response = if response.hovered() {
                            let language = if aurora_net::scripts::is_slua_item(script.flags) {
                                aurora_i18n::tr!("script-type-lua")
                            } else {
                                aurora_i18n::tr!("script-type-lsl")
                            };
                            let state = if running {
                                format!("{}\n", aurora_i18n::tr!("script-running"))
                            } else {
                                String::new()
                            };
                            response.on_hover_text(format!("{}\n{state}{language}\n\n{}", script.name, script.permissions.tooltip()))
                        } else {
                            response
                        };
                        // the selection's rights, for the entries that act on
                        // all of it: one script's, or every one's
                        let menus: Vec<ScriptMenu> = targets.iter().filter_map(|target| menu_of(linkset, *target)).collect();
                        let can_take = menus.len() == targets.len() && menus.iter().all(|menu| menu.refused.is_none());
                        let can_delete = menus.len() == targets.len() && menus.iter().all(|menu| menu.can_delete);
                        let moves = menus.iter().any(|menu| menu.moves);
                        let any_dirty = targets.iter().any(|target| dirty_of(linkset, *target));
                        // the entries of the contents window's menu, with the
                        // rights it reads; a script of the inventory has none
                        menu::context_menu(&response, p, |ui| {
                            if menu::item(ui, p, "folder-open-filled", aurora_i18n::tr!("common-open")) {
                                chosen = Some(if many {
                                    TreeAction::OpenMany(targets.clone())
                                } else {
                                    TreeAction::Open(target)
                                });
                            }
                            if let Some(actions) = script.menu {
                                // the item's own window, as the contents menu
                                // opens it (LLInventoryAction::doToSelected,
                                // "task_properties")
                                if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("common-properties")) {
                                    chosen = Some(TreeAction::Properties(target));
                                }
                                let label = if moves {
                                    aurora_i18n::tr!("task-transfer-move-menu")
                                } else {
                                    aurora_i18n::tr!("build-ui-contents-copy-to-inventory")
                                };
                                let tip = if many {
                                    aurora_i18n::tr!("contents-copy-selection-tip")
                                } else {
                                    actions.refused.unwrap_or(actions.tip)
                                };
                                if menu::item_tip(ui, p, "briefcase-filled", label, can_take, tip) {
                                    chosen = Some(if many {
                                        TreeAction::TakeMany(targets.clone())
                                    } else {
                                        TreeAction::Take(target)
                                    });
                                }
                                if menu::item_if(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), can_delete) {
                                    chosen = Some(if many {
                                        TreeAction::DeleteMany(targets.clone())
                                    } else {
                                        TreeAction::Delete(target)
                                    });
                                }
                                // Stop / Run, the contents menu's last entry: a
                                // state not known yet reads as running, as the
                                // contents window reads it; one script at a time
                                if !many {
                                    let on = script.running.unwrap_or(true);
                                    let (icon, label) = if on {
                                        ("player-stop-filled", aurora_i18n::tr!("common-stop"))
                                    } else {
                                        ("player-play-filled", aurora_i18n::tr!("build-ui-contents-run"))
                                    };
                                    if menu::item_if(ui, p, icon, label, actions.can_control) {
                                        chosen = Some(TreeAction::SetRunning { target, running: !on });
                                    }
                                }
                            }
                            // the selected scripts this window has open and
                            // whose text changed: saved together
                            if many && menu::item_if(ui, p, "files-filled", aurora_i18n::tr!("script-save-all"), any_dirty) {
                                chosen = Some(TreeAction::SaveMany(targets.clone()));
                            }
                            menu::separator(ui, p);
                            if menu::item(ui, p, "x", aurora_i18n::tr!("common-close")) {
                                chosen = Some(TreeAction::Close(target));
                            }
                        });
                    }
                }
            }
            if closed != collapsed(&ctx, linkset.root) {
                ctx.data_mut(|d| d.insert_persisted(collapsed_id(linkset.root), closed));
            }
            // a script dragged out of the list lifts with the pointer, as a
            // tab does; let go over the source, it opens a pane there
            if let Some((target, name)) = ctx.data(|d| d.get_temp::<Option<(ScriptTarget, String)>>(drag_id)).flatten() {
                let pointer = ctx.input(|i| i.pointer.interact_pos());
                if let Some(pos) = pointer
                    && ctx.input(|i| i.pointer.any_down())
                {
                    let galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap(name.clone(), inventory::tree::font(), p.ink));
                    let lifted = egui::Rect::from_center_size(pos, galley.size() + Vec2::new(26.0, 12.0));
                    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag_id));
                    painter.rect_filled(lifted, ui.visuals().window_corner_radius, p.field);
                    painter.rect_stroke(
                        lifted,
                        ui.visuals().window_corner_radius,
                        egui::Stroke::new(1.0, p.violet),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(lifted.center() - galley.size() / 2.0, galley, p.ink);
                }
                if ctx.input(|i| i.pointer.any_released()) {
                    ctx.data_mut(|d| d.remove_temp::<Option<(ScriptTarget, String)>>(drag_id));
                    if let Some(pos) = pointer
                        && !panel.contains(pos)
                    {
                        chosen = Some(TreeAction::Dropped { target, pos });
                    }
                }
            }
        });
        // the object the script in front is in, in three dimensions: the whole
        // object from its root prim, the child prim alone, turned by dragging,
        // and a click on it touches the prim
        if let Some(stage) = stage {
            ui.add_space(4.0);
            // the object it shows, and what a click does: the view's caption
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size.y = STAGE_CAPTION;
                ui.label(
                    RichText::new(linkset.map_or("", |linkset| linkset.name.as_str()))
                        .size(11.0)
                        .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("script-preview-touch-hint"))
                            .size(10.5)
                            .color(p.muted_dim),
                    );
                });
            });
            ui.add_space(2.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STAGE_VIEW), egui::Sense::hover());
            // nothing to draw yet, or at all: the room says why, as a list
            // with nothing to show does
            if let Err(absent) = stage.view.stage.model() {
                let said = match absent {
                    Absent::Loading => aurora_i18n::tr!("common-loading-2"),
                    Absent::Nothing => aurora_i18n::tr!("script-preview-nothing"),
                    Absent::Refused => aurora_i18n::tr!("rlva-blocked"),
                };
                let mut note = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect)
                        .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
                );
                note.label(RichText::new(said).size(11.0).color(p.muted_dim));
            } else {
                let names = &stage.names;
                let spoken = aurora_i18n::tr!("script-preview-view");
                let hit = stage.view.show(
                    ui,
                    p,
                    rect,
                    0.0,
                    Showing {
                        alone: Some(stage.root),
                        outlined: Some(stage.prim),
                        pointed: None,
                    },
                    spoken.as_ref(),
                    &|hit| names.iter().find(|(id, _)| *id == hit.prim).map(|(_, name)| name.clone()),
                );
                if let Some(hit) = hit {
                    chosen = Some(TreeAction::Touch(hit.prim));
                }
            }
        }
        // the Properties of the script in front, above the tree's own foot:
        // the facts a glance needs, and a button that opens the window
        if let Some((target, name, flags, permissions, task)) = properties {
            let (header, _) = row(
                ui,
                p,
                "info-circle-filled",
                p.violet_light,
                inventory::tree::name(&name, p),
                None,
                false,
                PROPERTIES_LEFT,
                None,
                false,
            );
            let header = header.on_hover_text(name.as_str());
            // the button at the row's right end opens the item's own window
            let mut expand = None;
            if task {
                let at = egui::Rect::from_center_size(egui::pos2(header.rect.right() - 12.0, header.rect.center().y), Vec2::splat(16.0));
                let hit = ui.interact(at, header.id.with("open_properties"), egui::Sense::click());
                if let Some(icon) = icons::global("arrows-maximize") {
                    ui.painter().image(
                        icon.id(),
                        at,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        if hit.hovered() { p.ink } else { p.muted },
                    );
                }
                if hit.on_hover_text(aurora_i18n::tr!("common-properties")).clicked() {
                    expand = Some(TreeAction::Properties(target));
                }
            }
            if let Some(action) = expand {
                chosen = Some(action);
            }
            let language = if aurora_net::scripts::is_slua_item(flags) {
                aurora_i18n::tr!("script-type-lua")
            } else {
                aurora_i18n::tr!("script-type-lsl")
            };
            let mask = permissions.owner().unwrap_or(0);
            let mut rights = String::new();
            for (letter, right) in [
                ('C', aurora_net::build::perm::COPY),
                ('M', aurora_net::build::perm::MODIFY),
                ('T', aurora_net::build::perm::TRANSFER),
            ] {
                if mask & right != 0 {
                    rights.push(letter);
                }
            }
            let facts = [
                (aurora_i18n::tr!("script-properties-language").to_string(), language.to_string()),
                (aurora_i18n::tr!("script-properties-rights").to_string(), rights),
            ];
            // one column of values, as the details of a card: the longest
            // label sets it, so every value starts on the same line
            let label_width = facts.iter().fold(0.0f32, |widest, (label, _)| {
                widest.max(
                    ui.painter()
                        .layout_no_wrap(label.clone(), egui::FontId::proportional(11.0), p.muted)
                        .size()
                        .x,
                )
            }) + 8.0;
            for (label, value) in facts {
                ui.horizontal(|ui| {
                    ui.add_space(PROPERTIES_TEXT);
                    ui.allocate_ui_with_layout(
                        Vec2::new(label_width, 14.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| ui.label(RichText::new(label).size(11.0).color(p.muted)),
                    );
                    ui.label(RichText::new(value).size(11.0).color(p.ink));
                });
            }
            ui.add_space(2.0);
        } else if linkset.is_some() {
            // the page of a new script is in front: the section keeps its
            // place and says there is no script to describe
            let (header, _) = row(
                ui,
                p,
                "info-circle-filled",
                p.muted,
                inventory::tree::name(aurora_i18n::tr!("common-properties").as_ref(), p),
                None,
                false,
                PROPERTIES_LEFT,
                None,
                false,
            );
            header.on_hover_text(aurora_i18n::tr!("common-properties"));
            // the two rows the facts take, so the Settings row keeps its
            // place while the page is in front
            for line in [aurora_i18n::tr!("script-properties-none").to_string(), " ".to_owned()] {
                ui.horizontal(|ui| {
                    ui.add_space(PROPERTIES_TEXT);
                    ui.label(RichText::new(line).size(11.0).color(p.muted));
                });
            }
            ui.add_space(2.0);
        }
        // the editor's own preferences, at the foot of the list: the row the
        // inventory tree gives its Trash
        let (line, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), egui::Sense::hover());
        ui.painter().hline(line.x_range(), line.center().y, widgets::divider_stroke(p));
        if widgets::rail_row(
            ui,
            p,
            "settings-filled",
            p.muted,
            aurora_i18n::tr!("script-tree-settings"),
            None,
            false,
            0,
        )
        .clicked()
        {
            chosen = Some(TreeAction::Settings);
        }
    });
    chosen
}

/// A tab of the strip: one open script.
pub(super) struct Tab<'a> {
    pub target: ScriptTarget,
    pub name: &'a str,
    /// The prim that holds it: the tab's tooltip names it, as two prims may
    /// hold scripts of the same name.
    pub prim: &'a str,
    pub lua: bool,
    pub dirty: bool,
    /// A script that does not exist yet (the page that asks which language):
    /// its mark is the plus, not a script's.
    pub draft: bool,
}

impl Tab<'_> {
    /// The tab's name: its unsaved star is drawn in the room every tab keeps
    /// after the name, so the strip does not reflow while the text changes
    /// (LLItemBridge::getLabelSuffix).
    fn label(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed(self.name)
    }

    /// What the tab says under the pointer: the prim that holds the script and
    /// its name, and, for the name of the pane that is not in front, that the
    /// two are side by side.
    fn tooltip(&self, front: bool, side_by_side: bool) -> String {
        let mut said = format!("{} › {}", self.prim, self.name);
        if side_by_side && !front {
            said.push('\n');
            said.push_str(aurora_i18n::tr!("script-linkset-side-by-side").as_ref());
        }
        said
    }
}

/// What a click or a menu entry of the strip asked for.
pub(super) enum Tabs {
    None,
    Select(ScriptTarget),
    /// A tab dragged over another one, which takes its place.
    Reorder {
        from: ScriptTarget,
        to: ScriptTarget,
    },
    Close(ScriptTarget),
    CloseOthers(ScriptTarget),
    CloseAll,
    /// Show that script beside the one in front.
    Split(ScriptTarget),
    /// A tab dragged by its strip and let go over the source: the half it was
    /// dropped on, left or right, gives the split its pane.
    Dropped {
        target: ScriptTarget,
        pos: egui::Pos2,
    },
    /// Back to one script at a time.
    Unsplit,
    /// The plus past the strip: a new script in this window's object, which
    /// opens the page that asks which language (`linkset::Tab::draft`).
    NewScript,
}

/// The strip of open scripts, flush above the source: the tab in front takes
/// the editor's own background, so it sits on its top edge. A tab marks its
/// unsaved text with a star, as the tree does, and its cross closes it (the
/// middle button too); its menu closes the others or all of them.
///
/// A split window's two scripts share one tab, their names side by side with
/// a separator between them: the one in front in ink, the one beside it
/// dimmed, a cross for each. The other tabs follow.
/// The room the strip gives the button that lists the tabs when they do
/// not fit.
const OVERFLOW_BUTTON: f32 = 24.0;

/// Whether the tabs, their air and the button past the strip's room: the
/// strip then keeps the button, and its list shows every tab.
fn strip_overflows(widths: &[f32], spacing: f32, space: f32, button: f32) -> bool {
    !widths.is_empty() && widths.iter().sum::<f32>() + spacing * widths.len().saturating_sub(1) as f32 + spacing + button > space
}

pub(super) fn tabs(
    ui: &mut egui::Ui,
    p: &Palette,
    tabs: &[Tab<'_>],
    active: Option<ScriptTarget>,
    background: egui::Color32,
    split: Option<(ScriptTarget, ScriptTarget)>,
    drag_id: egui::Id,
    new_script: bool,
) -> Tabs {
    let mut answer = Tabs::None;
    // where each tab was drawn, and whether a drag began on it: the places a
    // tab dragged by its strip can be dropped on
    let mut zones: Vec<(ScriptTarget, egui::Rect, bool)> = Vec::new();
    let font = widgets::control_font(ui, 12.0);
    // as tall as the tree's filter field, which shares its row
    let height = ui.spacing().interact_size.y.max(24.0);
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    // the two panes in their own order, left then right, whichever is in
    // front: the strip stays put when the focus moves from one to the other
    let pair = split.and_then(|(left, right)| {
        let left = tabs.iter().find(|tab| tab.target == left)?;
        let right = tabs.iter().find(|tab| tab.target == right)?;
        (left.target != right.target).then_some((left, right))
    });
    let split_open = split.is_some();
    // how wide the strip would be: a tab is its icon, its name and its
    // cross; the split's is both names in one tab. Past the room, a
    // button lists every tab, the ones the strip cannot show too.
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.muted).size().x;
    let mut widths: Vec<f32> = Vec::new();
    if let Some((left, right)) = pair {
        // the split's tab is both names in one surface; the others take the
        // widget's own measure, the one they are drawn with
        widths.push(102.0 + measure(&left.label()) + measure(&right.label()) + 2.0 * STAR_ROOM);
        for tab in tabs.iter().filter(|tab| tab.target != left.target && tab.target != right.target) {
            widths.push(tab_width(ui, p, tab, &font));
        }
    } else {
        for tab in tabs {
            widths.push(tab_width(ui, p, tab, &font));
        }
    }
    let space = ui.available_width();
    // the strip keeps the room of the plus, and of the list of tabs when they
    // do not fit; when the tabs fit, it stops after the last one, so the plus
    // sits right beside them as a browser's does. A window of the agent's own
    // inventory offers no plus: a script is made there in the inventory's own
    // window, not beside a tree of an object's contents.
    let plus_room = if new_script { OVERFLOW_BUTTON + 2.0 } else { 0.0 };
    let overflow = strip_overflows(&widths, 2.0, space - plus_room, plus_room);
    let used = widths.iter().sum::<f32>() + 2.0 * widths.len().saturating_sub(1) as f32;
    let strip_width = used.min(space - plus_room - if overflow { plus_room } else { 0.0 }).max(40.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.allocate_ui_with_layout(
            Vec2::new(strip_width, height),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("script_linkset_tabs")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if let Some((left, right)) = pair {
                                let _ = split_tab(ui, p, left, right, active, background, &font, height, uv, &mut answer);
                                for tab in tabs.iter().filter(|tab| tab.target != left.target && tab.target != right.target) {
                                    let _ = single_tab(ui, p, tab, false, background, &font, height, split_open, &mut answer, None);
                                }
                            } else {
                                for tab in tabs {
                                    let _ = single_tab(
                                        ui,
                                        p,
                                        tab,
                                        active == Some(tab.target),
                                        background,
                                        &font,
                                        height,
                                        split_open,
                                        &mut answer,
                                        Some(&mut zones),
                                    );
                                }
                            }
                        });
                    });
            },
        );
        if overflow {
            let tip = aurora_i18n::tr!("script-tabs-all");
            let button = widgets::icon_button_sized_response(ui, p, "chevron-down", tip, true, OVERFLOW_BUTTON);
            menu::dropdown(&button, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                for tab in tabs {
                    let label = format!("{}  ·  {}", tab.prim, tab.label());
                    if menu::check(ui, p, "file-code", &label, active == Some(tab.target), true) {
                        answer = Tabs::Select(tab.target);
                    }
                }
            });
        }
        // a new script in this window's object: the page that asks which
        // language it will be written in
        if new_script {
            let plus = widgets::icon_button_sized_response(ui, p, "plus", aurora_i18n::tr!("extra-new-script"), true, OVERFLOW_BUTTON);
            if plus.clicked() {
                answer = Tabs::NewScript;
            }
        }
    });
    // where the strip ends: a tab let go under it was dropped over the source
    let strip = ui.min_rect();
    // A tab dragged by its strip lifts with the pointer and takes the place of
    // the tab it is dropped on; let go over the source, it opens a pane there
    // (not with a split open: the pair is one tab).
    if pair.is_none() && zones.len() > 1 {
        let ctx = ui.ctx().clone();
        let drag = ui.scope_id().with("script_tabs_reorder");
        for (index, (_, _, started)) in zones.iter().enumerate() {
            if *started {
                ctx.data_mut(|d| {
                    d.insert_temp(drag, index);
                    d.insert_temp(drag_id, Some(zones[index].0));
                });
            }
        }
        if let Some(from) = ctx.data(|d| d.get_temp::<usize>(drag))
            && from < zones.len()
        {
            let pointer = ctx.input(|i| i.pointer.interact_pos());
            if let Some(pos) = pointer
                && ctx.input(|i| i.pointer.any_down())
                && let Some(tab) = tabs.iter().find(|tab| tab.target == zones[from].0)
            {
                let galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap(tab.label().into_owned(), font.clone(), p.ink));
                let lifted = egui::Rect::from_center_size(pos, galley.size() + Vec2::new(26.0, 12.0));
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, drag));
                painter.rect_filled(lifted, ui.visuals().window_corner_radius, p.field);
                painter.rect_stroke(
                    lifted,
                    ui.visuals().window_corner_radius,
                    egui::Stroke::new(1.0, p.violet),
                    egui::StrokeKind::Inside,
                );
                painter.galley(lifted.center() - galley.size() / 2.0, galley, p.ink);
            }
            if ctx.input(|i| i.pointer.any_released()) {
                let target = zones[from].0;
                let to = pointer.and_then(|pos| zones.iter().position(|(_, rect, _)| pos.x >= rect.left() && pos.x <= rect.right()));
                ctx.data_mut(|d| {
                    d.remove_temp::<usize>(drag);
                    d.remove_temp::<Option<ScriptTarget>>(drag_id);
                });
                match to {
                    Some(to) if to != from => {
                        answer = Tabs::Reorder {
                            from: target,
                            to: zones[to].0,
                        };
                    }
                    None => {
                        if let Some(pos) = pointer.filter(|pos| pos.y > strip.bottom()) {
                            answer = Tabs::Dropped { target, pos };
                        }
                    }
                    Some(_) => {}
                }
            }
        }
    }
    answer
}

/// The room a tab of the strip takes, as the widget that draws it measures it.
fn tab_width(ui: &mut egui::Ui, p: &Palette, tab: &Tab<'_>, font: &egui::FontId) -> f32 {
    let label = tab.label();
    let pill = widgets::TabPill {
        label: label.as_ref(),
        room: f32::INFINITY,
        icon: Some(tab_icon(tab, p)),
        star: Some(tab.dirty),
        active: false,
        closable: true,
    };
    widgets::tab_pill_width(ui, font, &pill)
}

/// The mark before a tab's name: the plus of a script that does not exist yet,
/// or the item's icon tinted as the tree draws it.
fn tab_icon(tab: &Tab<'_>, p: &Palette) -> (&'static str, egui::Color32) {
    if tab.draft { ("plus", p.violet) } else { script_icon(tab.lua, p) }
}

/// The mark of a script: the item's own icon, the Lua one for an SLua script,
/// tinted as the tree draws it. The tabs and the quick open share it, so a
/// script wears the same mark wherever its name stands.
pub(super) fn script_icon(lua: bool, p: &Palette) -> (&'static str, egui::Color32) {
    (inventory::item_icon(10, 10, u32::from(lua)), inventory::item_icon_color(10, 10, p))
}

/// One tab of the strip: its icon, its name (starred when unsaved), its cross
/// and its menu, drawn with the widget every strip of closable tabs uses.
fn single_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    tab: &Tab<'_>,
    selected: bool,
    background: egui::Color32,
    font: &egui::FontId,
    height: f32,
    split_open: bool,
    answer: &mut Tabs,
    zones: Option<&mut Vec<(ScriptTarget, egui::Rect, bool)>>,
) -> egui::Rect {
    let label = tab.label();
    let pill = widgets::TabPill {
        label: label.as_ref(),
        room: f32::INFINITY,
        icon: Some(tab_icon(tab, p)),
        star: Some(tab.dirty),
        active: selected,
        closable: true,
    };
    let (rect, response, closed) = widgets::tab_pill(ui, p, &pill, height, font, background);
    if let Some(zones) = zones {
        zones.push((tab.target, rect, response.drag_started()));
    }
    if closed {
        *answer = Tabs::Close(tab.target);
    } else if response.clicked() && !selected {
        *answer = Tabs::Select(tab.target);
    }
    tab_menu(p, &response, tab, split_open, answer);
    // what it says under the pointer, built only while it is on it
    if response.hovered() {
        response.on_hover_text(tab.tooltip(true, false));
    }
    rect
}

/// The tab of a split window: both scripts in one surface, their names in the
/// order of the panes -- left, then right -- with a separator between them.
/// The pane in front is in ink, the other dimmed, so that the strip says which
/// script the tree, the dock and the status bar follow. Each name brings its
/// script to the front, and its cross closes it.
#[allow(clippy::too_many_arguments)]
fn split_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    left: &Tab<'_>,
    right: &Tab<'_>,
    active: Option<ScriptTarget>,
    background: egui::Color32,
    font: &egui::FontId,
    height: f32,
    uv: egui::Rect,
    answer: &mut Tabs,
) -> egui::Rect {
    // pad, icon, name, gap, cross, the separator, then the same for the second name
    const PAD: f32 = 8.0;
    const ICON: f32 = 15.0;
    const GAP: f32 = 6.0;
    const CROSS: f32 = 15.0;
    const MARK: f32 = 14.0;
    let measure = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.muted).size().x;
    let left_width = measure(&left.label());
    let right_width = measure(&right.label());
    let after_name = ICON + GAP + CROSS;
    let width = PAD + after_name + left_width + STAR_ROOM + MARK + after_name + right_width + STAR_ROOM + PAD;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click_and_drag());
    let fill = if response.hovered() {
        background.lerp_to_gamma(p.raised, 0.18)
    } else {
        background
    };
    // rounded at the top only: the tab's bottom sits flush on the source
    // below, the chosen tab's face being the source's own colour
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: radius.nw,
            ne: radius.ne,
            sw: 0,
            se: 0,
        },
        fill,
    );
    // the left pane is the one in front unless the right one is
    let left_front = active != Some(right.target);
    let mut x = rect.left() + PAD;
    let (next, left_hit, left_closed) = split_side(
        ui,
        p,
        left,
        left.label().into_owned(),
        left_front,
        x,
        rect,
        uv,
        font,
        response.id.with("left"),
        answer,
    );
    x = next;
    // a separator between the two names, as a file tab strip's would be
    let middle = x + MARK / 2.0;
    ui.painter().vline(
        middle,
        egui::Rangef::new(rect.center().y - 7.0, rect.center().y + 7.0),
        egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.7)),
    );
    x += MARK;
    let (_, right_hit, right_closed) = split_side(
        ui,
        p,
        right,
        right.label().into_owned(),
        !left_front,
        x,
        rect,
        uv,
        font,
        response.id.with("right"),
        answer,
    );
    if !left_closed && !right_closed {
        if response.middle_clicked() && left_hit.hovered() {
            *answer = Tabs::Close(left.target);
        } else if response.middle_clicked() && right_hit.hovered() {
            *answer = Tabs::Close(right.target);
        } else if left_hit.clicked() {
            *answer = Tabs::Select(left.target);
        } else if right_hit.clicked() {
            *answer = Tabs::Select(right.target);
        }
    }
    tab_menu(p, &left_hit, left, true, answer);
    tab_menu(p, &right_hit, right, true, answer);
    if left_hit.hovered() {
        left_hit.on_hover_text(left.tooltip(left_front, true));
    }
    if right_hit.hovered() {
        right_hit.on_hover_text(right.tooltip(!left_front, true));
    }
    rect
}

/// The room a tab keeps after its name for the unsaved star.
const STAR_ROOM: f32 = 12.0;

/// One name of a split tab, from `x` rightwards: its icon, the name in ink
/// when it is the pane in front, then its cross. Returns where the next part
/// starts, the name's response, and whether its cross asked to close it.
#[allow(clippy::too_many_arguments)]
fn split_side(
    ui: &mut egui::Ui,
    p: &Palette,
    tab: &Tab<'_>,
    label: String,
    front: bool,
    x: f32,
    rect: egui::Rect,
    uv: egui::Rect,
    font: &egui::FontId,
    id: Id,
    answer: &mut Tabs,
) -> (f32, egui::Response, bool) {
    let center = rect.center().y;
    let mut left = x;
    let icon_at = egui::Rect::from_min_size(egui::pos2(left, rect.top()), Vec2::new(15.0, rect.height()));
    if let Some(t) = icons::global(inventory::item_icon(10, 10, u32::from(tab.lua))) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(icon_at.center(), Vec2::splat(13.0)),
            uv,
            inventory::item_icon_color(10, 10, p),
        );
    }
    left += 15.0;
    let name = ui
        .painter()
        .layout_no_wrap(label, font.clone(), if front { p.ink } else { p.muted });
    let text = egui::Rect::from_min_size(egui::pos2(left, center - name.size().y / 2.0), name.size());
    ui.painter().galley(text.min, name.clone(), if front { p.ink } else { p.muted });
    if tab.dirty {
        ui.painter().text(
            egui::pos2(left + name.size().x + 2.0, center),
            egui::Align2::LEFT_CENTER,
            "*",
            font.clone(),
            if front { p.violet_light } else { p.muted },
        );
    }
    let hit = ui.interact(text, id, egui::Sense::click());
    left += name.size().x + 6.0 + STAR_ROOM;
    let close = ui.interact(
        egui::Rect::from_center_size(egui::pos2(left + 7.5, center), Vec2::splat(15.0)),
        id.with("close"),
        egui::Sense::click(),
    );
    if let Some(t) = icons::global("x") {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(close.rect.center(), Vec2::splat(10.0)),
            uv,
            if close.hovered() { p.ink } else { p.muted },
        );
    }
    if close.clicked() {
        *answer = Tabs::Close(tab.target);
    }
    (left + 15.0, hit, close.clicked())
}

/// The menu of a tab: close it or the others, open it beside or back in one
/// pane.
fn tab_menu(p: &Palette, response: &egui::Response, tab: &Tab<'_>, split_open: bool, answer: &mut Tabs) {
    menu::context_menu_of_button(response, p, |ui| {
        if menu::item(ui, p, "x", aurora_i18n::tr!("common-close")) {
            *answer = Tabs::Close(tab.target);
        }
        if menu::item(ui, p, "tabs", aurora_i18n::tr!("script-linkset-close-others")) {
            *answer = Tabs::CloseOthers(tab.target);
        }
        if menu::item(ui, p, "circle-x", aurora_i18n::tr!("script-linkset-close-all")) {
            *answer = Tabs::CloseAll;
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "arrows-split", aurora_i18n::tr!("script-linkset-split")) {
            *answer = Tabs::Split(tab.target);
        }
        if split_open && menu::item(ui, p, "layout-sidebar-right-filled", aurora_i18n::tr!("script-linkset-unsplit")) {
            *answer = Tabs::Unsplit;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip keeps its rows when they fit, and the button that lists every
    /// tab when they do not (the air and the button count too).
    #[test]
    fn the_strip_lists_its_tabs_when_they_do_not_fit() {
        assert!(!strip_overflows(&[], 2.0, 400.0, 26.0));
        assert!(!strip_overflows(&[100.0, 100.0], 2.0, 400.0, 26.0));
        assert!(strip_overflows(&[100.0, 100.0, 100.0, 100.0], 2.0, 400.0, 26.0));
        assert!(
            strip_overflows(&[120.0, 120.0], 2.0, 260.0, 26.0),
            "the button takes the room past the edge"
        );
    }

    /// A Shift click takes the rows between the script in front and the
    /// clicked one, in the list's order, both ends included.
    #[test]
    fn a_shift_click_takes_the_rows_between_the_ends() {
        let target = |item: u128| ScriptTarget {
            object: Uuid::from_u128(500),
            item: Uuid::from_u128(item),
        };
        let (a, b, c) = (target(1), target(2), target(3));
        let visible = vec![a, b, c];
        assert_eq!(shift_selection(&visible, Some(a), c), vec![a, b, c]);
        assert_eq!(shift_selection(&visible, Some(c), a), vec![a, b, c], "both ways");
        assert_eq!(shift_selection(&visible, Some(b), b), vec![b]);
        assert_eq!(shift_selection(&visible, None, b), vec![b], "no anchor: the click alone");
        assert_eq!(shift_selection(&visible, Some(a), target(9)), vec![target(9)]);
    }
}
