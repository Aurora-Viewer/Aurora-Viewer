//! Script editor actions, permission checks and offline simulated assets.

use super::*;
use crate::ui::scripts::Action;
use crate::world::inventory::actions as rules;
use aurora_net::build::{BuildCmd, perm};
use aurora_net::scripts::{ScriptSaved, ScriptTarget};
use uuid::Uuid;

pub(super) const SYNTAX_DEMO_SOURCE: &str = r#"// LSL: types, variables, functions and events
integer count = 0;
float delay = 1.5;
string greet(key resident) {
    return "Bonjour, " + llKey2Name(resident);
}
default {
    state_entry() { llSetTimerEvent(delay); }
    touch_start(integer total) {
        count += 1;
        llSay(PUBLIC_CHANNEL, greet(llDetectedKey(0)));
    }
    timer() {
        if (count > 0)
            llSetColor(<0.5, 1.0, 0.25>, ALL_SIDES);
    }
}
"#;

pub(super) const COLORS_DEMO_SOURCE: &str = r#"vector red = <1.0, 0.0, 0.0>;
vector blue = <0, .25, 1>;
list colors = [<.8, .2, .6>, <0, 1, .5>];
vector position = <128, 64, 20>;
rotation orientation = <0, 0, 0, 1>;
string example = "<1, 0, 0>";
// <0, 1, 0> is a comment
default
{
    state_entry()
    {
        llSetColor(<0.2, 0.8, 1.0>, ALL_SIDES);
    }
}
"#;

pub(super) const SLUA_DEMO_SOURCE: &str = r#"--[[ SLua: comments, strings and library calls ]]
local count = 0
local alpha = 0.5

local function greet(resident: string): string
    return `Hello, {ll.Key2Name(resident)}!`
end

LLEvents:on("touch_start", function(events)
    count += 1
    ll.Say(PUBLIC_CHANNEL, greet(events[1]:getKey()))
    ll.SetText(string.format("%d touches", count), vector(1, 1, 1), alpha)
end)
"#;

pub(super) const WORD_WRAP_DEMO_SOURCE: &str = r#"// A long comment stays readable when the editor is resized: each continuation belongs to the same source line, and saving or copying keeps the original text exactly as written.
string message = "This long string wraps visually inside the script editor without inserting any newline into the script or changing the string sent to nearby chat.";
default
{
    state_entry()
    {
        llSay(PUBLIC_CHANNEL, message);
    }
}
"#;

/// `AURORA_DEMO_SCRIPTS=lua`: a Lua (SLua) source, item subtype SST_LUA.
pub(super) const LUA_DEMO_SOURCE: &str = r#"-- A Lua script: saved for the Luau VM, never as LSL
local function greet(name: string)
    ll.Say(0, "Hello, " .. name .. "!")
end

greet("Avatar")
"#;

/// `AURORA_DEMO_SCRIPTS=lua-lints`: a Lua script the editor's own lints have
/// something to say about, one line per rule, and a `#define` the preprocessor
/// never sees.
pub(super) const LUA_LINT_DEMO_SOURCE: &str = r#"#define SPEED 2
-- Lua's own lints, one line each: an LSL name as LSL writes it, a local no
-- one reads, a global written in a function, and a string joined with +
local unused = 1
local function hurry()
    count = count + 1
    llSay(0, "speed " + tostring(SPEED))
end
hurry()
"#;

/// `AURORA_DEMO_SCRIPTS=linkset`: the scripts of the demo linkset.
pub(super) const PANEL_DEMO_SOURCE: &str = r#"// The control panel of the demo linkset
default
{
    state_entry()
    {
        llSetText("Control Panel", <1, 1, 1>, 1.0);
    }

    touch_start(integer total)
    {
        llSay(PUBLIC_CHANNEL, "Panel pressed");
    }
}
"#;

/// `AURORA_DEMO_SCRIPTS=nested`: a script of a prim linked under another
/// prim of the same object, two levels below the root.
pub(super) const NESTED_DEMO_SOURCE: &str = r#"// A prim linked under another prim of the same object
default
{
    state_entry()
    {
        llSetText("Nested", <1, 1, 1>, 1.0);
    }
}
"#;

/// A script the linter has something to say about, one line per rule of
/// `ui::scripts::lints` (`AURORA_DEMO_SCRIPTS=lints`).
pub(super) const LINT_DEMO_SOURCE: &str = r#"// The linter has something to say about this one
integer too_big = 5000000000;
string text = "";

default
{
    state_entry()
    {
        llSetTimerEvent(0.05);
        llSleep(0.5);
        llListen(0, "", NULL_KEY, "");
        llSensorRepeat("", NULL_KEY, AGENT, 10.0, PI, 0.5);
        integer i;
        for (i = 0; i < 3; ++i)
        {
            text = text + "x";
        }
        llGetTime();
        llGetTime();
        llGetTime();
        llSay(0);
        llSayy(0, "hi");
        integer j;
        if (j = 1) { }
        if (TRUE);
        while (FALSE);
        switch (i)
        {
            case 1: break;
            case 1: break;
        }
        state default;
    }

    touch_start(integer total)
    {
        llSetPrimitiveParams([]);
        llSetPrimitiveParams([]);
    }

    timer()
    {
    }

    timer()
    {
    }
}
"#;

/// The own-function case of the inlays demo: a call of the script's own
/// function names its arguments from the declaration.
pub(super) const INLAY_DEMO_SOURCE: &str = r#"integer myFunction(integer helo1, string hello2)
{
    llOwnerSay(hello2);
}

default
{
    state_entry()
    {
        llSay(0, "Hello, Avatar!");
        myFunction(123, "test");
    }

    touch_start(integer total_number)
    {
        llSay(0, "Touched.");
    }
}
"#;

pub(super) const LAMP_DEMO_SOURCE: &str = r#"// A second script of the same linkset: its own tab, its own text
float BRIGHTNESS = 0.6;

default
{
    state_entry()
    {
        llSetTimerEvent(2.0);
    }

    timer()
    {
        llSetLinkPrimitiveParamsFast(LINK_THIS,
            [PRIM_POINT_LIGHT, TRUE, <1.0, 0.8, 0.4>, BRIGHTNESS, 6.0, 0.75]);
    }
}
"#;

/// The lamp's second script, in Lua beside the first one's LSL: the tree
/// shows both languages in one group.
pub(super) const SPARKLE_LUA_SOURCE: &str = r#"-- The lamp's glow, in Lua beside the LSL script
local BRIGHTNESS: number = 0.6

local function glow()
    ll.SetTimerEvent(2.0)
    ll.SetLinkPrimitiveParamsFast(ll.LINK_THIS, {
        ll.PRIM_POINT_LIGHT, true, <1.0, 0.8, 0.4>, BRIGHTNESS, 6.0, 0.75,
    })
end

glow()
"#;

struct ScriptInfo {
    name: String,
    handle: u64,
    owner: Uuid,
    asset: Uuid,
    can_read: bool,
    can_edit: bool,
    /// The item's flags: its low byte is the script language.
    flags: u32,
}

/// Read and edit rights: copy and modify, or copy alone for the library
/// (read only). Any god reads any script (LLPreviewLSL::loadAsset,
/// LLLiveLSLEditor::loadAsset: isGodlike() first); editing goes through
/// LLAgent::allowOperation, so only from GOD_MAINTENANCE ([`rules::with_god`]).
fn permissions(mask: u32, library: bool, god_level: u8) -> (bool, bool) {
    let mask = rules::with_god(mask, god_level);
    let copy = mask & perm::COPY != 0;
    let modify = mask & perm::MODIFY != 0;
    (god_level > 0 || copy && (modify || library), copy && modify && !library)
}

/// Read and edit rights on a script inside an object: as [`permissions`]
/// for a resident; any god reads and edits it, whatever its level
/// (LLLiveLSLEditor::loadAsset loads it for isGodlike(), and onLoadComplete
/// turns editing on without asking LLAgent::allowOperation again). Only an
/// inventory script keeps GOD_MAINTENANCE for editing (LLPreviewLSL).
fn task_permissions(mask: u32, god_level: u8) -> (bool, bool) {
    if god_level > 0 { (true, true) } else { permissions(mask, false, 0) }
}

/// What an `#include` may take of an inventory script, from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IncludeUse {
    /// Its source cannot be read (copy and modify, as for opening it).
    Unreadable,
    /// Readable, but without transfer and made by someone else: its source
    /// would end in a script that can be given away.
    NoTransfer,
    /// Its source, without its asset id.
    Source,
    /// Its source, and its asset id in `__ASSETID__` (full permission).
    SourceAndAsset,
}

/// Script includes follow item rights: the source of an included script
/// needs the rights to read it (`permissions`) and transfer, unless the
/// agent made it, since it is copied into the saved script; its asset id
/// only goes into `__ASSETID__` for a full permission script. The Legacy
/// Viewer includes any script it finds by name and always writes its asset
/// id (fslslpreproc.cpp, found_include_directive, opened_include_file).
fn include_use(mask: u32, library: bool, creator: Uuid, agent: Uuid) -> IncludeUse {
    let full = perm::COPY | perm::MODIFY | perm::TRANSFER;
    if !permissions(mask, library, 0).0 {
        IncludeUse::Unreadable
    } else if mask & full == full {
        IncludeUse::SourceAndAsset
    } else if mask & perm::TRANSFER != 0 || !creator.is_nil() && creator == agent {
        IncludeUse::Source
    } else {
        IncludeUse::NoTransfer
    }
}

/// The prims of a linkset one after the one that holds it, each with the
/// number of prims that hold it: the set [`crate::build::family`] gives (the
/// root, its children, theirs -- a seated avatar and what it wears left out),
/// ordered depth first, which the tree indents a group by.
pub(super) fn nested_prims(world: &crate::world::World, root: usize) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    let mut stack = vec![(root, 0u8)];
    while let Some((prim, depth)) = stack.pop() {
        if out.len() >= 4096 {
            break;
        }
        let Some(object) = world.objects.get(prim) else {
            continue;
        };
        if object.is_avatar() {
            continue;
        }
        out.push((prim, depth));
        // back to front, so that the first child is drawn first
        for &child in world.objects.children_of(&object.key).iter().rev() {
            if world.objects.get(child).is_some_and(|o| !o.is_avatar()) {
                stack.push((child, depth.saturating_add(1)));
            }
        }
    }
    out
}

impl App {
    /// A line a demo object says to its owner, through the chat a running
    /// script's line takes, so that the editor's Console is fed the way a
    /// grid's is.
    /// `AURORA_DEMO_SCRIPTS=linkset`: three linked prims beside the plaza,
    /// their scripts named as the tree groups them, two of them open in the
    /// window's tabs.
    fn script_info(&self, target: ScriptTarget) -> Option<ScriptInfo> {
        if target.object.is_nil() {
            let inv = &self.world.inventory;
            let it = inv.items.get(&target.item).filter(|it| it.asset_type == 10)?;
            let library = !inv.lib_root.is_nil() && crate::world::inventory::actions::under(inv, it.id, inv.lib_root);
            let (can_read, can_edit) = permissions(rules::held(inv, it), library, self.world.god_level);
            Some(ScriptInfo {
                name: it.name.clone(),
                handle: self.world.main_region.unwrap_or_default(),
                owner: it.owner,
                asset: it.asset_id,
                can_read,
                can_edit,
                flags: it.flags,
            })
        } else {
            let idx = self.world.objects.index_of_uuid(&target.object)?;
            let object = self.world.objects.get(idx)?;
            let it = self
                .build
                .contents
                .objects
                .get(&target.object)?
                .items
                .iter()
                .find(|it| it.item_id == target.item && it.asset_type == 10)?;
            let member = !it.group_id.is_nil() && self.world.groups.is_member(&it.group_id);
            let mask = rules::task_script_mask(it, self.world.agent_id, member);
            let modify = !task_script_refused(&self.build, &self.world, target.object);
            let (can_read, can_edit) = task_permissions(mask, self.world.god_level);
            Some(ScriptInfo {
                name: it.name.clone(),
                handle: object.key.region,
                owner: it.owner_id,
                asset: it.asset_id,
                can_read: can_read && modify,
                can_edit: can_edit && modify,
                flags: it.flags,
            })
        }
    }

    pub(super) fn open_script(&mut self, target: ScriptTarget) {
        self.open_script_from(target, None);
    }

    /// The window an object's script is in: the scripts of one linkset share
    /// it, its tree and its tabs. An inventory script's is the agent's one
    /// window; nil for an object that is gone, and for every script while the
    /// preferences keep one window per script.
    fn script_linkset(&self, object: Uuid) -> Uuid {
        if self.settings.script_editor.simple {
            return Uuid::nil();
        }
        if object.is_nil() {
            return crate::ui::scripts::INVENTORY_WINDOW;
        }
        let Some(idx) = self.world.objects.index_of_uuid(&object) else {
            return Uuid::nil();
        };
        let root = crate::build::root_of(&self.world, idx);
        self.world.objects.get(root).map_or(Uuid::nil(), |o| o.full_id)
    }

    pub(super) fn open_script_from(&mut self, target: ScriptTarget, owner: Option<egui::Id>) {
        if self.world.rlva.has("viewscript") {
            self.rlva_view_blocked(10);
            return;
        }
        // LLTaskLSLBridge::openItem (llpanelobjectinventory.cpp): the scripts
        // of an attachment that cannot be taken off stay closed.
        if self
            .world
            .objects
            .index_of_uuid(&target.object)
            .is_some_and(|i| self.world.rlva_locked_attachment(i))
        {
            self.rlva_view_blocked(10);
            return;
        }
        if task_script_refused(&self.build, &self.world, target.object) {
            self.world.system_notice(
                aurora_i18n::tr!("script-object-no-modify"),
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        let Some(info) = self.script_info(target) else { return };
        let linkset = self.script_linkset(target.object);
        let request = self
            .script_editors
            .open(target, info.name, info.can_read, info.can_edit, info.flags);
        if let Some(editor) = self.script_editors.documents.iter_mut().find(|e| e.target == target) {
            editor.inventory_owner = owner;
            editor.linkset = linkset;
        }
        if let Some(request) = request {
            // an SLua script, or one that may become one: its keywords are
            // read while the source loads, not on the frame that draws it
            if aurora_net::scripts::is_slua_item(info.flags) || self.world.main().is_some_and(|r| r.lua_scripts) {
                crate::lsl_keywords::warm_lua();
            }
            self.send(NetCommand::LoadScript {
                request,
                target,
                handle: info.handle,
                owner: info.owner,
                asset: info.asset,
            });
            if let Some(action) = self.script_editors.load_experiences(target) {
                self.apply_script_action(action);
            }
            if !target.object.is_nil() {
                self.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                    handle: info.handle,
                    object_id: target.object,
                    item_id: target.item,
                }));
            }
        }
    }

    /// The scripts of every linkset a window shows, grouped by prim: the tree
    /// beside the source. Asks for the contents of each prim it has no
    /// listing of yet.
    /// The trees of the windows that have a script open: an object's linkset
    /// by prim, or the agent's inventory by folder. Asks for each prim's
    /// contents when it has no current listing.
    fn script_linkset_views(&mut self) -> Vec<crate::ui::scripts::Linkset> {
        use crate::build::contents::AT_LSL_TEXT;
        use crate::build::task_inventory::TransferKind;
        use crate::ui::inventory::permissions::Permissions;
        use crate::ui::scripts::{INVENTORY_WINDOW, Linkset, LinksetPrim, LinksetScript, LinksetScriptMenu, PrimKind};
        let mut roots: Vec<Uuid> = Vec::new();
        let mut inventory = false;
        for e in &self.script_editors.documents {
            if !e.is_open() {
                continue;
            }
            if e.linkset == INVENTORY_WINDOW {
                inventory = true;
            } else if !e.linkset.is_nil() && !roots.contains(&e.linkset) {
                roots.push(e.linkset);
            }
        }
        let mut views = Vec::new();
        for root in roots {
            let mut view = Linkset {
                root,
                name: crate::build::replace::object_name(&self.build, &self.world, root),
                note: None,
                prims: Vec::new(),
            };
            // an object gone from the region keeps its window open, with the
            // text its scripts still hold, and an empty tree
            let Some(idx) = self.world.objects.index_of_uuid(&root) else {
                views.push(view);
                continue;
            };
            // the family depth first, the prim that holds another before it:
            // the tree indents a prim under the one it hangs from
            for (prim, depth) in nested_prims(&self.world, idx) {
                let Some(object) = self.world.objects.get(prim).map(|o| o.full_id) else {
                    continue;
                };
                // RequestTaskInventory, unless a listing is current or on its
                // way: the tree says what the linkset holds, not only what the
                // last Open window asked for.
                self.build.request_contents(&self.world, prim, false);
                let listing = self.build.contents.objects.get(&object);
                let editable = self.build.contents_editable(&self.world, prim);
                // the object menu's Touch, as RLVa allows it on this very prim
                let touch = self
                    .world
                    .objects
                    .get(prim)
                    .is_some_and(|o| self.world.rlva_can_touch(o.key.region, o.key.local_id));
                let mut entry = LinksetPrim {
                    object,
                    kind: PrimKind::Prim,
                    name: crate::build::replace::object_name(&self.build, &self.world, object),
                    depth,
                    type_default: 0,
                    loading: listing.is_none_or(|listing| listing.loading),
                    can_new_script: editable,
                    touch,
                    // the object menu's Take, Take copy and Delete, on the
                    // linkset's root alone: a child goes with it
                    menu: (depth == 0).then(|| self.script_prim_menu(object)),
                    scripts: Vec::new(),
                };
                for item in listing
                    .iter()
                    .flat_map(|listing| listing.items.iter())
                    .filter(|item| item.asset_type == AT_LSL_TEXT)
                {
                    // the contents window's menu entries, from its own rules
                    let transfer = self.build.task_transfer_kind(&self.world, object, item);
                    entry.scripts.push(LinksetScript {
                        object,
                        item: item.item_id,
                        name: item.name.clone(),
                        flags: item.flags,
                        permissions: Permissions::new(item.owner_mask, item.next_owner_mask),
                        running: self
                            .build
                            .contents
                            .running
                            .get(&(object, item.item_id))
                            .map(|(running, _)| *running),
                        dirty: self
                            .script_editors
                            .documents
                            .iter()
                            .any(|e| e.is_open() && e.target.object == object && e.target.item == item.item_id && e.unsaved()),
                        menu: Some(LinksetScriptMenu {
                            moves: matches!(transfer, Ok(kind) if kind != TransferKind::Copy),
                            refused: transfer.err(),
                            tip: aurora_i18n::tr!("contents-copy-selection-tip"),
                            can_delete: editable,
                            can_control: editable,
                        }),
                    });
                }
                view.prims.push(entry);
            }
            views.push(view);
        }
        if inventory {
            views.push(self.inventory_script_view());
        }
        views
    }

    /// The window of the agent's own scripts: every script of the inventory
    /// under the folder that holds it, built again when the inventory changes
    /// and stamped with the unsaved stars every frame.
    fn inventory_script_view(&mut self) -> crate::ui::scripts::Linkset {
        use crate::ui::scripts::{INVENTORY_WINDOW, Linkset};
        let inventory = &self.world.inventory;
        if self.script_inventory_generation != Some(inventory.generation) {
            self.script_inventory_prims = inventory_prims(inventory);
            self.script_inventory_generation = Some(inventory.generation);
        }
        // a star on the scripts whose open text is unsaved
        for prim in &mut self.script_inventory_prims {
            for script in &mut prim.scripts {
                script.dirty = self
                    .script_editors
                    .documents
                    .iter()
                    .any(|e| e.is_open() && e.target.object.is_nil() && e.target.item == script.item && e.unsaved());
            }
        }
        let left = inventory_folders_left(inventory);
        let note = if left > 0 {
            Some(aurora_i18n::tr!("script-inventory-loading", count = left as i64).to_string())
        } else if self.script_inventory_prims.is_empty() {
            Some(aurora_i18n::tr!("script-inventory-empty").to_string())
        } else {
            None
        };
        Linkset {
            root: INVENTORY_WINDOW,
            name: aurora_i18n::tr!("script-inventory-name").to_string(),
            note,
            prims: self.script_inventory_prims.clone(),
        }
    }

    pub(super) fn show_scripts(&mut self, ctx: &egui::Context, p: &Palette) {
        // the editor's own font, installed when the preference changes
        crate::ui::fonts::set_editor_font(ctx, &self.settings.script_editor.font);
        self.script_editors.agent = self.world.agent_id;
        self.script_editors.agent_name = self.world.social.name_of(&self.world.agent_id);
        // shortcut: the agent's region, not the object's (have_lua_enabled); fine
        // until objects in a neighbour region are edited across a SLua border
        self.script_editors.lua_scripts = self.world.main().is_some_and(|r| r.lua_scripts);
        let rights: Vec<_> = self
            .script_editors
            .documents
            .iter()
            .map(|e| (e.target, self.script_info(e.target)))
            .collect();
        for (target, info) in rights {
            if let Some(e) = self.script_editors.documents.iter_mut().find(|e| e.target == target) {
                e.can_read = info.as_ref().is_some_and(|i| i.can_read);
                e.can_edit = info.as_ref().is_some_and(|i| i.can_edit);
                e.can_reset = info.is_some() && !target.object.is_nil();
                if let Some(info) = info {
                    e.name = info.name;
                }
            }
        }
        // a change of the layout setting moves every open script to the
        // window it belongs in now: its own, or its object's (or the
        // inventory's) shared one
        if self.script_editors.simple != self.settings.script_editor.simple {
            self.script_editors.simple = self.settings.script_editor.simple;
            let windows: Vec<Uuid> = self
                .script_editors
                .documents
                .iter()
                .map(|e| self.script_linkset(e.target.object))
                .collect();
            for (e, linkset) in self.script_editors.documents.iter_mut().zip(windows) {
                e.linkset = linkset;
            }
        }
        // a script in a window of its own (the simple layout) names the prim
        // that holds it, as the path under the source always did for a linkset
        self.script_editors
            .name_own_scripts(|target| crate::build::replace::object_name(&self.build, &self.world, target.object));
        // the windows' trees
        let linksets = self.script_linkset_views();
        self.script_stages(ctx, &linksets, std::time::Instant::now());
        for action in self.script_editors.show(
            ctx,
            p,
            &mut self.settings.script_editor,
            &mut self.settings.build.color_picker,
            &linksets,
        ) {
            self.apply_script_action(action);
        }
        // the copies of the unsaved scripts (ui/scripts/backup.rs)
        self.sync_script_backup_dir();
        self.script_editors.back_up();
        self.script_editors.show_debug(ctx, p, &mut self.world.script_debug);
    }

    /// Keep the trees' 3D views on the object each window is about: the prim
    /// the script in front is in, inside the object from its root's whole
    /// linkset, or that child alone (`stage_group`'s family). A window with no
    /// object, and the setting off, shows none.
    fn script_stages(&mut self, ctx: &egui::Context, linksets: &[crate::ui::scripts::Linkset], now: std::time::Instant) {
        let mut shown: Vec<(Uuid, Uuid, Uuid, Vec<usize>, Vec<(Uuid, String)>)> = Vec::new();
        if self.settings.script_editor.preview {
            for view in linksets {
                if view.root.is_nil() || view.root == crate::ui::scripts::INVENTORY_WINDOW {
                    continue;
                }
                let Some(target) = self.script_editors.script_in_front(view.root) else {
                    continue;
                };
                let Some(idx) = self.world.objects.index_of_uuid(&target.object) else {
                    continue;
                };
                let root = crate::build::root_of(&self.world, idx);
                let Some(root_id) = self.world.objects.get(root).map(|o| o.full_id) else {
                    continue;
                };
                let prims = if root == idx {
                    crate::build::family(&self.world, root)
                } else {
                    vec![idx]
                };
                let names = view.prims.iter().map(|prim| (prim.object, prim.name.clone())).collect();
                shown.push((view.root, root_id, target.object, prims, names));
            }
        }
        // the views of windows gone, or where the setting went off, free their
        // models off the main thread
        let wanted: std::collections::HashSet<Uuid> = shown.iter().map(|(window, ..)| *window).collect();
        let gone: Vec<Uuid> = self
            .script_editors
            .stages
            .keys()
            .copied()
            .filter(|window| !wanted.contains(window))
            .collect();
        for window in gone {
            if let Some(mut view) = self.script_editors.stages.remove(&window) {
                view.view.stage.clear(&self.scene);
            }
        }
        let world = &self.world;
        let scene = &self.scene;
        for (window, root, prim, prims, names) in shown {
            let stage = self.script_editors.stages.entry(window).or_default();
            stage.root = root;
            stage.prim = prim;
            stage.names = names;
            stage.view.stage.update(
                world,
                scene,
                &[root],
                |root| {
                    Some(crate::object_stage::Group {
                        root: *root,
                        prims: prims.clone(),
                    })
                },
                crate::object_stage::Place::World,
                ctx,
                now,
            );
        }
    }

    /// A touch on a prim from the script window: the object menu's own touch
    /// (a grab and a release at its surface), refused by RLVa where it is.
    fn script_touch(&mut self, object: Uuid) {
        let Some(key) = self
            .world
            .objects
            .index_of_uuid(&object)
            .and_then(|idx| self.world.objects.get(idx))
            .map(|o| o.key)
        else {
            return;
        };
        if !self.world.rlva_can_touch(key.region, key.local_id) {
            return;
        }
        let surface = aurora_net::TouchSurface::default();
        self.send(NetCommand::ObjectGrab {
            handle: key.region,
            local_id: key.local_id,
            offset: glam::Vec3::ZERO,
            surface,
        });
        self.send(NetCommand::ObjectRelease {
            handle: key.region,
            local_id: key.local_id,
            surface,
        });
    }

    pub(super) fn apply_script_action(&mut self, action: Action) {
        match action {
            Action::Touch(object) => self.script_touch(object),
            Action::Rename { target, name } => {
                // Permissions are rechecked when the UI action is executed.
                if self.script_info(target).is_some_and(|info| info.can_edit) {
                    if target.object.is_nil() {
                        let inv = &self.world.inventory;
                        if rules::renameable(inv, target.item)
                            && let Ok(change) = rules::patch(inv, target.item, aurora_llsd::llsd_map! { "name" => name })
                        {
                            self.edit_inventory(change, false, Uuid::nil());
                        }
                    } else if let Some(idx) = self.world.objects.index_of_uuid(&target.object) {
                        self.build.rename_item(&self.world, idx, target.item, &name);
                    }
                }
            }
            Action::Include { request, name } => {
                // of the scripts of that name, the one the include may use
                // most (FSLSLPreprocessor::findInventoryByName takes the
                // first one); read on a user action (a save or a preview)
                let inv = &self.world.inventory;
                let found = inv
                    .items
                    .values()
                    .filter(|item| item.asset_type == 10 && item.name == name)
                    .map(|item| {
                        let library = !inv.lib_root.is_nil() && crate::world::inventory::actions::under(inv, item.id, inv.lib_root);
                        (
                            item.id,
                            include_use(
                                crate::world::inventory::actions::held(inv, item),
                                library,
                                item.creator,
                                self.world.agent_id,
                            ),
                        )
                    })
                    .max_by_key(|(_, use_)| *use_);
                let target = found.filter(|(_, use_)| *use_ >= IncludeUse::Source).map(|(item, use_)| {
                    let target = ScriptTarget { object: Uuid::nil(), item };
                    (target, use_)
                });
                let info = target.and_then(|(target, use_)| {
                    self.script_info(target)
                        .filter(|info| info.can_read)
                        .map(|info| (target, use_, info))
                });
                if let Some((target, use_, info)) = info {
                    // __ASSETID__: the asset only of a full permission script
                    let asset = if use_ == IncludeUse::SourceAndAsset {
                        info.asset
                    } else {
                        Uuid::nil()
                    };
                    self.script_editors.include_asset(request, asset);
                    self.send(NetCommand::LoadScript {
                        request,
                        target,
                        handle: info.handle,
                        owner: info.owner,
                        asset: info.asset,
                    });
                } else {
                    let reason = if found.is_some_and(|(_, use_)| use_ == IncludeUse::NoTransfer) {
                        aurora_i18n::tr!("script-preproc-include-no-transfer", name = name)
                    } else {
                        aurora_i18n::tr!("script-preproc-include-missing", name = name)
                    };
                    self.script_editors.loaded(
                        request,
                        ScriptTarget {
                            object: Uuid::nil(),
                            item: Uuid::nil(),
                        },
                        Err(reason),
                    );
                }
            }
            Action::SettingsChanged => {
                if !self.demo {
                    self.settings.save();
                }
            }
            Action::BarChange(change) => {
                let settings = &mut self.settings.script_editor;
                match change {
                    crate::ui::scripts::BarChange::FontSize(size) => settings.font_size = size.clamp(8.0, 32.0),
                    crate::ui::scripts::BarChange::WordWrap => settings.word_wrap = !settings.word_wrap,
                    crate::ui::scripts::BarChange::Minimap => settings.minimap = !settings.minimap,
                    crate::ui::scripts::BarChange::Simple => settings.simple = !settings.simple,
                }
                if !self.demo {
                    self.settings.save();
                }
            }
            Action::Paste(target) => {
                if let Some(text) = self.gfx.as_mut().and_then(|g| g.egui_state.clipboard_text()) {
                    self.script_editors.paste(&self.egui_ctx, target, &text);
                }
            }
            Action::Save {
                request,
                target,
                options,
                source,
            } => {
                // LLLiveLSLEditor::saveIfNeeded: a locked attachment's script
                // is not saved, and the editor keeps its text
                if !self.world.rlva_can_save_script(target.object) {
                    let blocked = aurora_i18n::tr!("rlva-blocked");
                    self.script_editors.saved(request, target, &Err(blocked.into()));
                    self.world.system_notice(blocked, crate::world::notice_topics::Topic::Warning);
                    return;
                }
                let Some(info) = self.script_info(target).filter(|i| i.can_edit) else {
                    self.script_editors
                        .saved(request, target, &Err(aurora_i18n::tr!("script-no-permission").into()));
                    return;
                };
                self.send(NetCommand::SaveScript {
                    request,
                    target,
                    handle: info.handle,
                    options,
                    source,
                });
            }
            Action::Experiences { request, target } => {
                if let Some(info) = self.script_info(target).filter(|i| i.can_read) {
                    self.send(NetCommand::LoadScriptExperiences {
                        request,
                        target,
                        handle: info.handle,
                    });
                }
            }
            Action::Reload { request, target } => {
                if let Some(action) = self.script_editors.load_experiences(target) {
                    self.apply_script_action(action);
                }
                let Some(info) = self.script_info(target).filter(|i| i.can_read) else {
                    self.script_editors
                        .loaded(request, target, Err(aurora_i18n::tr!("script-no-permission").into()));
                    return;
                };
                self.send(NetCommand::LoadScript {
                    request,
                    target,
                    handle: info.handle,
                    owner: info.owner,
                    asset: info.asset,
                });
            }
            Action::Running { target, running } => {
                if let Some(info) = self.script_info(target).filter(|i| i.can_edit) {
                    self.send(NetCommand::Build(BuildCmd::SetScriptRunning {
                        handle: info.handle,
                        object_id: target.object,
                        item_id: target.item,
                        running,
                    }));
                    self.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                        handle: info.handle,
                        object_id: target.object,
                        item_id: target.item,
                    }));
                }
            }
            Action::Reset(target) => {
                if let Some(info) = self.script_info(target).filter(|_| !target.object.is_nil()) {
                    self.send(NetCommand::Build(BuildCmd::ScriptReset {
                        handle: info.handle,
                        object_id: target.object,
                        item_id: target.item,
                    }));
                    self.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                        handle: info.handle,
                        object_id: target.object,
                        item_id: target.item,
                    }));
                }
            }
            Action::QueryRunning(target) => {
                if let Some(info) = self.script_info(target).filter(|i| i.can_read) {
                    self.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                        handle: info.handle,
                        object_id: target.object,
                        item_id: target.item,
                    }));
                }
            }
            Action::OpenScript(target) => self.open_script(target),
            Action::TakeScript { object, item } => self.take_task_script(object, item),
            Action::DeleteScript { object, item } => self.remove_task_items(object, &[item]),
            Action::Properties(target) => self.open_task_script_properties(target),
            Action::NewScript { object, lua } => self.new_task_script(object, lua),
            Action::EditObject(object) => self.edit_task_object(object),
            // the object menu's own actions, so that their rights, the warning
            // Delete shows first and the RLVa test at the send stay in one place
            Action::TakeObject { object, copy } => self.on_ctx_action(if copy {
                crate::ui::context::CtxAction::TakeCopy(object)
            } else {
                crate::ui::context::CtxAction::Take(object)
            }),
            Action::DeleteObject(object) => self.on_ctx_action(crate::ui::context::CtxAction::Delete(object)),
            Action::SaveAll { linkset, target } => self.save_task_scripts(linkset, target),
            // a cancelled close prompt abandons a quit that waited for it
            Action::CloseCancelled => self.exit_pending = None,
        }
    }

    /// The script tree's Edit: the build floater, its Edit tool, with that
    /// prim chosen.
    fn edit_task_object(&mut self, object: Uuid) {
        let Some(index) = self.world.objects.iter().find(|(_, o)| o.full_id == object).map(|(index, _)| index) else {
            return;
        };
        self.build.click_select(&mut self.world, &self.settings.build, Some(index), false);
        self.build.open_build(crate::build::Tool::Edit);
    }

    /// "Save all": every open script of the window that has something to save
    /// (`linkset` nil: the one script, as Save).
    fn save_task_scripts(&mut self, linkset: Uuid, target: ScriptTarget) {
        for action in self.script_editors.save_all(linkset, target) {
            self.apply_script_action(action);
        }
    }

    /// "Copy to inventory" (or Move) of a script's contents menu: the same
    /// folder copy and move warning as the contents window, for that one item.
    fn take_task_script(&mut self, object: Uuid, item: Uuid) {
        let Some(task) = self
            .build
            .contents
            .objects
            .get(&object)
            .and_then(|listing| listing.items.iter().find(|it| it.item_id == item))
            .cloned()
        else {
            return;
        };
        self.copy_selected_contents(object, vec![task]);
    }

    /// "Properties" of a script's row in the tree: the item's own window, as
    /// the contents menu opens it (LLInventoryAction::doToSelected,
    /// "task_properties", for the one item).
    fn open_task_script_properties(&mut self, target: ScriptTarget) {
        let Some(item) = self
            .build
            .contents
            .objects
            .get(&target.object)
            .and_then(|listing| listing.items.iter().find(|it| it.item_id == target.item))
            .cloned()
        else {
            return;
        };
        self.build.open_task_items_properties(target.object, &[item]);
    }

    /// "New script" of a prim's menu in the script editor's tree, and the
    /// language the page of the strip's plus chose: the item the Contents
    /// panel creates (LLPanelContents::onNewScriptFlyoutCommit), in that
    /// language -- Lua only where the prim's region runs SLua.
    fn new_task_script(&mut self, object: Uuid, lua: bool) {
        let Some(idx) = self.world.objects.index_of_uuid(&object) else {
            return;
        };
        if !self.build.contents_editable(&self.world, idx) {
            return;
        }
        let region_lua = self
            .world
            .objects
            .get(idx)
            .and_then(|o| self.world.regions.get(&o.key.region))
            .is_some_and(|r| r.lua_scripts);
        self.build.new_script(&self.world, idx, lua && region_lua);
        self.flush_build();
    }

    /// AURORA_DEMO_EDITOR_SYNC=1|<port>: the sync turned on for the run,
    /// the demo script sent to the external editor, and a local editor
    /// (aurora_editor_sync::demo_client) that proves itself, follows it and
    /// gets the result of a save with an error. Everything stays on
    /// 127.0.0.1.
    /// Once a frame: the external script editor sync (editor_sync.rs).
    pub(super) fn update_editor_sync(&mut self) {
        let compiled = self.script_editors.take_compiled();
        let settings = &self.settings.script_editor;
        let in_world = self.in_world() && !self.world.agent_id.is_nil();
        let agent_name = self
            .world
            .login
            .as_ref()
            .map(|l| crate::editor_sync::agent_name(&l.first_name, &l.last_name))
            .unwrap_or_else(|| self.world.social.name_of(&self.world.agent_id));
        let frame = crate::editor_sync::Frame {
            enabled: settings.websocket_sync,
            port: settings.websocket_port,
            forward_debug: settings.websocket_forward_debug,
            logged_in: in_world,
            scripts: self.script_editors.synced().collect(),
            compiled,
            hello: aurora_editor_sync::Hello {
                viewer_name: "Aurora Viewer".into(),
                viewer_version: aurora_version::version().to_owned(),
                agent_id: self.world.agent_id,
                agent_name,
                syntax_id: crate::lsl_keywords::current_id().unwrap_or_default(),
                commands: false,
            },
            now: std::time::Instant::now(),
        };
        let objects = &self.world.objects;
        let root = |prim: Uuid| {
            let idx = objects.index_of_uuid(&prim)?;
            objects.get(objects.linkset_root(idx)).map(|o| o.full_id)
        };
        let tips = self
            .editor_sync
            .update(self.net.runtime(), frame, &mut self.world.editor_sync_chat, root);
        self.world.editor_sync_chat.on = self.editor_sync.wants_chat(settings.websocket_forward_debug);
        for tip in tips {
            // ExternalEditorServerStarted / Failed / Stopped
            let title = match tip {
                crate::editor_sync::Tip::Started(port) => aurora_i18n::tr!("script-sync-started", port = port.to_string()),
                crate::editor_sync::Tip::Failed(port) => aurora_i18n::tr!("script-sync-failed", port = port.to_string()),
                crate::editor_sync::Tip::Stopped => aurora_i18n::tr!("script-sync-stopped").into(),
            };
            self.world.notifications.push(
                crate::world::notifications::Kind::Info,
                title,
                "",
                crate::world::notifications::Data::None,
            );
        }
    }

    pub(super) fn script_saved(&mut self, request: Uuid, target: ScriptTarget, result: Result<ScriptSaved, String>) {
        if !self.script_editors.saved(request, target, &result) {
            return;
        }
        if let Ok(saved) = result {
            if target.object.is_nil() {
                if let Some(mut item) = self.world.inventory.items.get(&target.item).cloned() {
                    item.asset_id = saved.asset;
                    self.world.inventory.add_items(vec![item]);
                }
                if !self.demo {
                    self.send(NetCommand::FetchItems {
                        items: vec![target.item],
                        owner: self.world.agent_id,
                    });
                }
            } else {
                if let Some(inv) = self.build.contents.objects.get_mut(&target.object)
                    && let Some(item) = inv.items.iter_mut().find(|it| it.item_id == target.item)
                {
                    item.asset_id = saved.asset;
                }
                if let Some(idx) = self.world.objects.index_of_uuid(&target.object) {
                    self.build.request_contents(&self.world, idx, true);
                    let handle = self.world.objects.get(idx).map(|o| o.key.region).unwrap_or_default();
                    self.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                        handle,
                        object_id: target.object,
                        item_id: target.item,
                    }));
                    self.flush_build();
                }
            }
        }
    }
}

/// LLTaskLSLBridge::openItem: a script in an object opens only when the
/// object may be modified. Checked once per open, on the root's flags.
fn task_script_refused(build: &crate::build::BuildTool, world: &crate::world::World, object: Uuid) -> bool {
    !object.is_nil()
        && world
            .objects
            .index_of_uuid(&object)
            .is_some_and(|idx| !build.can_modify(world, idx))
}

/// The agent's own scripts, one group per folder that holds one: the folder's
/// row, its scripts under it, its subfolders one step further in. The Library
/// is not ours and stays out; a folder holding no script, and none under it,
/// is not drawn.
fn inventory_prims(inv: &crate::world::inventory::Inventory) -> Vec<crate::ui::scripts::LinksetPrim> {
    use crate::ui::scripts::LinksetPrim;
    let mut prims: Vec<LinksetPrim> = Vec::new();
    let Some(root) = inv.folders.get(&inv.root) else {
        return prims;
    };
    // the root's own scripts, when it holds some: the "My Inventory" row
    let scripts = folder_scripts(inv, root);
    if !scripts.is_empty() {
        prims.push(folder_prim(root, 0, scripts));
    }
    let base = u8::from(!prims.is_empty());
    for child in sorted_folders(inv, root) {
        prims.extend(folder_groups(inv, child, base));
    }
    prims
}

/// The folders under `folder`, by name, as the tree lists them.
fn sorted_folders<'a>(
    inv: &'a crate::world::inventory::Inventory,
    folder: &'a crate::world::inventory::Folder,
) -> Vec<&'a crate::world::inventory::Folder> {
    let mut children: Vec<&crate::world::inventory::Folder> = folder.children.iter().filter_map(|id| inv.folders.get(id)).collect();
    children.sort_by_key(|folder| folder.info.name.to_lowercase());
    children
}

/// The scripts a folder holds itself, by name: a link is listed as it is named
/// and opens the item it points at, and a broken one is left out.
fn folder_scripts(
    inv: &crate::world::inventory::Inventory,
    folder: &crate::world::inventory::Folder,
) -> Vec<crate::ui::scripts::LinksetScript> {
    use crate::ui::inventory::permissions::Permissions;
    use crate::ui::scripts::LinksetScript;
    use crate::world::inventory::actions as rules;
    let mut items: Vec<&aurora_net::inventory::InvItem> = folder.items.iter().filter_map(|id| inv.items.get(id)).collect();
    items.sort_by_key(|item| item.name.to_lowercase());
    items
        .into_iter()
        .filter_map(|item| {
            let original = rules::original(inv, item.id)?;
            let script = inv.items.get(&original).filter(|it| it.asset_type == 10)?;
            Some(LinksetScript {
                object: Uuid::nil(),
                item: original,
                name: item.name.clone(),
                flags: script.flags,
                permissions: Permissions::new(script.owner_mask, script.next_owner_mask),
                running: None,
                dirty: false,
                menu: None,
            })
        })
        .collect()
}

/// The row of one folder of the inventory, its scripts under it.
fn folder_prim(
    folder: &crate::world::inventory::Folder,
    depth: u8,
    scripts: Vec<crate::ui::scripts::LinksetScript>,
) -> crate::ui::scripts::LinksetPrim {
    use crate::ui::scripts::PrimKind;
    crate::ui::scripts::LinksetPrim {
        object: folder.info.id,
        kind: PrimKind::Folder,
        name: folder.info.name.clone(),
        depth,
        type_default: folder.info.type_default,
        loading: folder.state != crate::world::inventory::FetchState::Fetched,
        can_new_script: false,
        touch: false,
        menu: None,
        scripts,
    }
}

/// One folder's group and every group under it, or none when nothing in it
/// holds a script.
fn folder_groups(
    inv: &crate::world::inventory::Inventory,
    folder: &crate::world::inventory::Folder,
    depth: u8,
) -> Vec<crate::ui::scripts::LinksetPrim> {
    // the Library is another tree, with another owner and its own rules
    if folder.library {
        return Vec::new();
    }
    let scripts = folder_scripts(inv, folder);
    let mut under = Vec::new();
    for child in sorted_folders(inv, folder) {
        under.extend(folder_groups(inv, child, depth.saturating_add(1)));
    }
    if scripts.is_empty() && under.is_empty() {
        return Vec::new();
    }
    let mut out = vec![folder_prim(folder, depth, scripts)];
    out.extend(under);
    out
}

/// How many folders under the agent's root are not read yet: the tree says so
/// while the inventory is on its way.
fn inventory_folders_left(inv: &crate::world::inventory::Inventory) -> usize {
    use crate::world::inventory::FetchState;
    let mut left = 0;
    let mut todo = vec![inv.root];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else {
            continue;
        };
        if folder.library {
            continue;
        }
        if folder.state != FetchState::Fetched {
            left += 1;
        }
        todo.extend(folder.children.iter().copied());
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One folder of the fixture inventory.
    fn folder(id: u128, parent: u128, name: &str) -> crate::world::inventory::Folder {
        crate::world::inventory::Folder {
            info: aurora_net::inventory::InvFolder {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(parent),
                name: name.into(),
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: crate::world::inventory::FetchState::Fetched,
            library: false,
        }
    }

    /// An inventory with a script in a folder and in one under it, a link to
    /// the first one, a folder holding no script, and the Library with one
    /// that is not ours.
    fn inventory_with_scripts() -> crate::world::inventory::Inventory {
        use aurora_net::inventory::InvItem;
        let mut inv = crate::world::inventory::Inventory {
            root: Uuid::from_u128(1),
            lib_root: Uuid::from_u128(6),
            ..Default::default()
        };
        for (id, parent, name) in [
            (1, 0, "My Inventory"),
            (2, 1, "Weapons"),
            (3, 2, "Tools"),
            (4, 1, "Empty"),
            (5, 1, "Scripts"),
            (6, 1, "Library"),
        ] {
            let mut entry = folder(id, parent, name);
            entry.library = id == 6;
            inv.folders.insert(Uuid::from_u128(id), entry);
        }
        inv.folders.get_mut(&Uuid::from_u128(1)).unwrap().children =
            vec![Uuid::from_u128(2), Uuid::from_u128(4), Uuid::from_u128(5), Uuid::from_u128(6)];
        inv.folders.get_mut(&Uuid::from_u128(2)).unwrap().children = vec![Uuid::from_u128(3)];
        let item = |id: u128, parent: u128, name: &str, asset_type: i32| InvItem {
            id: Uuid::from_u128(id),
            parent: Uuid::from_u128(parent),
            name: name.into(),
            asset_type,
            inv_type: asset_type,
            owner: Uuid::from_u128(9),
            creator: Uuid::from_u128(9),
            owner_mask: 0x7fff_ffff,
            next_owner_mask: 0x7fff_ffff,
            base_mask: 0x7fff_ffff,
            ..Default::default()
        };
        let mut items = vec![
            item(10, 2, "Sword", 10),
            item(11, 2, "Not a script", 7),
            item(12, 3, "Wrench", 10),
            item(13, 4, "Not drawn either", 6),
            item(14, 5, "Sword link", 24),
            item(15, 6, "Library script", 10),
        ];
        // a link points at the item it shows
        items[4].asset_id = Uuid::from_u128(10);
        inv.add_items(items);
        inv
    }

    /// A new script is Lua where the region runs SLua and the reader asks for
    /// it; a region without SLua always gets LSL.
    #[test]
    fn a_new_script_is_lua_only_where_the_region_runs_it() {
        // the app's own line of `new_task_script`, which the page's choice
        // reaches: the region's answer decides
        let choose = |region_runs_lua: bool, asked: bool| asked && region_runs_lua;
        assert!(choose(true, true));
        assert!(!choose(true, false), "the reader chose LSL");
        assert!(!choose(false, true), "a region without SLua falls back to LSL");
    }

    /// The agent's own scripts, grouped by the folder that holds each: the
    /// folders that hold one, nested one step per level, a link through the
    /// item it points at, and no folder (nor Library) that holds no script.
    #[test]
    fn the_inventory_tree_groups_scripts_by_folder() {
        use crate::ui::scripts::PrimKind;
        let inv = inventory_with_scripts();
        let prims = inventory_prims(&inv);
        let groups: Vec<(&str, u8)> = prims.iter().map(|prim| (prim.name.as_str(), prim.depth)).collect();
        assert_eq!(groups, [("Scripts", 0), ("Weapons", 0), ("Tools", 1)], "by name, nested");
        assert!(prims.iter().all(|prim| prim.kind == PrimKind::Folder));
        assert!(prims.iter().all(|prim| prim.scripts.iter().all(|script| script.object.is_nil())));
        assert_eq!(prims[0].scripts.len(), 1, "the link alone");
        assert_eq!(prims[0].scripts[0].name, "Sword link");
        assert_eq!(prims[0].scripts[0].item, Uuid::from_u128(10), "the item it points at");
        assert_eq!(prims[1].scripts.len(), 1, "the folder's own scripts, not the one under it");
        assert_eq!(prims[1].scripts[0].name, "Sword");
        assert_eq!(prims[2].scripts[0].name, "Wrench");
        assert!(prims.iter().all(|prim| prim.scripts[0].menu.is_none()), "no contents menu");
        assert!(!prims.iter().any(|prim| prim.name == "Empty" || prim.name == "Library"));
    }

    /// The tree says how much of the inventory is still on its way, and that
    /// no folder holds a script once it is all there.
    #[test]
    fn the_inventory_note_counts_the_folders_left() {
        let mut inv = inventory_with_scripts();
        inv.folders.get_mut(&Uuid::from_u128(3)).unwrap().state = crate::world::inventory::FetchState::Unknown;
        assert_eq!(inventory_folders_left(&inv), 1);
        inv.folders.get_mut(&Uuid::from_u128(3)).unwrap().state = crate::world::inventory::FetchState::Fetched;
        assert_eq!(inventory_folders_left(&inv), 0);
    }

    #[test]
    fn script_source_requires_copy_and_modify_except_read_only_library() {
        assert_eq!(permissions(perm::COPY | perm::MODIFY, false, 0), (true, true));
        assert_eq!(permissions(perm::COPY, false, 0), (false, false));
        assert_eq!(permissions(perm::MODIFY, false, 0), (false, false));
        assert_eq!(permissions(perm::COPY, true, 0), (true, false));
        assert_eq!(permissions(perm::COPY | perm::MODIFY, true, 0), (true, false));
        // any god reads any script; only from GOD_MAINTENANCE does it edit
        // one without modify (LLAgent::allowOperation), OpenSim's 200 to 249 not
        for level in [1, 200, 225, rules::GOD_MAINTENANCE - 1] {
            assert_eq!(permissions(0, false, level), (true, false), "level {level}");
            assert_eq!(permissions(perm::COPY | perm::MODIFY, false, level), (true, true), "level {level}");
        }
        assert_eq!(permissions(0, false, rules::GOD_MAINTENANCE), (true, true));
        assert_eq!(
            permissions(0, true, rules::GOD_MAINTENANCE),
            (true, false),
            "the library stays read only"
        );
        // a script inside an object: any god reads and edits it
        // (LLLiveLSLEditor), a resident needs copy and modify
        for level in [1, 200, 225, rules::GOD_MAINTENANCE] {
            assert_eq!(task_permissions(0, level), (true, true), "level {level}");
        }
        assert_eq!(task_permissions(perm::COPY, 0), (false, false));
        assert_eq!(task_permissions(perm::COPY | perm::MODIFY, 0), (true, true));
    }

    /// A copy of the unsaved text (ui/scripts/backup.rs) is kept only for a
    /// script its editor may edit: the editing right of [`permissions`],
    /// read from the agent's mask (rules::agent_mask).
    #[test]
    fn an_unsaved_script_is_copied_only_when_it_may_be_edited() {
        use aurora_net::inventory::InvItem;
        let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let full = perm::COPY | perm::MODIFY | perm::TRANSFER;
        let item = |owner: Uuid, owner_mask: u32, everyone_mask: u32, group_owned: bool| InvItem {
            owner,
            owner_mask,
            everyone_mask,
            group_owned,
            asset_type: 10,
            ..Default::default()
        };
        let copied = |it: &InvItem, library: bool| permissions(rules::agent_mask(it, me), library, 0).1;
        assert!(copied(&item(me, full, 0, false), false), "full permissions");
        assert!(
            copied(&item(me, perm::COPY | perm::MODIFY, 0, false), false),
            "no transfer, as the Legacy Viewer"
        );
        assert!(!copied(&item(me, perm::MODIFY | perm::TRANSFER, 0, false), false), "no copy");
        assert!(!copied(&item(me, perm::COPY | perm::TRANSFER, 0, false), false), "no modify");
        assert!(!copied(&item(other, full, 0, false), false), "someone else's, nothing for everyone");
        assert!(copied(&item(other, 0, full, false), false), "someone else's, editable by everyone");
        assert!(!copied(&item(me, full, 0, true), false), "group-owned: the everyone mask");
        assert!(!copied(&item(other, 0, full, false), true), "a Library script is read only");
        // in an object: copy and modify, as for opening it
        assert!(task_permissions(perm::COPY | perm::MODIFY, 0).1);
        assert!(!task_permissions(perm::COPY, 0).1 && !task_permissions(perm::MODIFY, 0).1);
    }

    /// The editor sync (editor_sync.rs) offers a script to an outside
    /// editor only when it was sent to the external editor while it may be
    /// edited: `Synced::can_edit` is the editing right of [`permissions`]
    /// (LLScriptEdCore's external editor button, enabled with the editor),
    /// the agent's mask from rules::agent_mask, as the export itself.
    #[test]
    fn a_script_is_offered_to_the_editor_sync_only_when_it_may_be_edited() {
        use aurora_net::inventory::InvItem;
        let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let full = perm::COPY | perm::MODIFY | perm::TRANSFER;
        let item = |owner: Uuid, owner_mask: u32, everyone_mask: u32, group_owned: bool| InvItem {
            owner,
            owner_mask,
            everyone_mask,
            group_owned,
            asset_type: 10,
            ..Default::default()
        };
        let offered = |it: &InvItem, library: bool| permissions(rules::agent_mask(it, me), library, 0).1;
        assert!(offered(&item(me, full, 0, false), false), "the owner's, full permissions");
        assert!(!offered(&item(me, full, 0, true), false), "group-owned, nothing for everyone");
        assert!(!offered(&item(other, full, 0, false), false), "someone else's");
        assert!(!offered(&item(other, 0, full, false), true), "a Library script");
        assert!(!offered(&item(me, perm::MODIFY | perm::TRANSFER, 0, false), false), "no copy");
        assert!(
            offered(&item(me, perm::COPY | perm::MODIFY, 0, false), false),
            "no transfer, as the Legacy Viewer"
        );
        assert!(!offered(&item(me, perm::COPY | perm::TRANSFER, 0, false), false), "no modify");
        // in an object, as for opening it: copy and modify
        assert!(task_permissions(full, 0).1);
        assert!(!task_permissions(perm::COPY | perm::TRANSFER, 0).1, "no modify");
        assert!(!task_permissions(perm::MODIFY | perm::TRANSFER, 0).1, "no copy");
    }

    #[test]
    fn a_task_script_opens_only_in_a_modifiable_object() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            world.apply(ev);
        }
        let object = crate::demo::action_id(970);
        let idx = world.objects.index_of_uuid(&object).expect("demo action object");
        world.objects.get_mut(idx).expect("object").parent_id = 0;
        let build = crate::build::BuildTool::default();
        world.objects.get_mut(idx).expect("object").update_flags = crate::ui::context::flags::OBJECT_MODIFY;
        assert!(!task_script_refused(&build, &world, object));
        world.objects.get_mut(idx).expect("object").update_flags = 0;
        assert!(task_script_refused(&build, &world, object));
        // an agent inventory script has no object
        assert!(!task_script_refused(&build, &world, Uuid::nil()));
    }

    #[test]
    fn script_includes_follow_item_rights() {
        let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let full = perm::COPY | perm::MODIFY | perm::TRANSFER;
        assert_eq!(include_use(full, false, other, me), IncludeUse::SourceAndAsset);
        // without transfer: refused, unless the agent made it (no asset id)
        let no_trans = perm::COPY | perm::MODIFY;
        assert_eq!(include_use(no_trans, false, other, me), IncludeUse::NoTransfer);
        assert_eq!(include_use(no_trans, false, Uuid::nil(), Uuid::nil()), IncludeUse::NoTransfer);
        assert_eq!(include_use(no_trans, false, me, me), IncludeUse::Source);
        // still copy and modify to read it, even for its creator
        assert_eq!(include_use(perm::COPY | perm::TRANSFER, false, me, me), IncludeUse::Unreadable);
        assert_eq!(include_use(perm::MODIFY | perm::TRANSFER, false, me, me), IncludeUse::Unreadable);
        // a library script: readable with copy, its asset id only when full
        assert_eq!(include_use(perm::COPY | perm::TRANSFER, true, other, me), IncludeUse::Source);
        assert_eq!(include_use(perm::COPY, true, other, me), IncludeUse::NoTransfer);
        // the most usable of several scripts of that name wins
        assert!(IncludeUse::SourceAndAsset > IncludeUse::Source && IncludeUse::Source > IncludeUse::NoTransfer);
        assert!(IncludeUse::NoTransfer > IncludeUse::Unreadable);
    }
}
