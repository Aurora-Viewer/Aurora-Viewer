//! Script editor actions, permission checks and offline simulated assets.

use super::*;
use crate::ui::scripts::Action;
use crate::world::inventory::actions as rules;
use aurora_net::build::{BuildCmd, perm};
use aurora_net::scripts::{DEFAULT_SOURCE, ScriptSaved, ScriptTarget};
use uuid::Uuid;

const SYNTAX_DEMO_SOURCE: &str = r#"// LSL: types, variables, functions and events
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

const COLORS_DEMO_SOURCE: &str = r#"vector red = <1.0, 0.0, 0.0>;
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

const SLUA_DEMO_SOURCE: &str = r#"--[[ SLua: comments, strings and library calls ]]
local count = 0
local delay = 1.5

local function greet(resident: string): string
    return `Hello, {ll.Key2Name(resident)}!`
end

LLEvents:on("touch_start", function(events)
    count += 1
    ll.Say(PUBLIC_CHANNEL, greet(events[1]:getKey()))
    ll.SetText(string.format("%d touches", count), vector(1, 1, 1), 1.0)
end)
"#;

const WORD_WRAP_DEMO_SOURCE: &str = r#"// A long comment stays readable when the editor is resized: each continuation belongs to the same source line, and saving or copying keeps the original text exactly as written.
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
const LUA_DEMO_SOURCE: &str = r#"-- A Lua script: saved for the Luau VM, never as LSL
local function greet(name: string)
    ll.Say(0, "Hello, " .. name .. "!")
end

greet("Avatar")
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

impl App {
    pub(super) fn demo_script_input(&self, raw: &mut egui::RawInput) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_SCRIPTS") else { return };
        if matches!(mode.as_str(), "insert-filter" | "insert-empty") {
            raw.events
                .extend(self.script_editors.demo_insertion_events(&self.egui_ctx, self.frame_count, &mode));
        }
        if matches!(mode.as_str(), "folding" | "object-folding") {
            raw.events.extend(self.script_editors.demo_folding_events(self.frame_count));
        }
        if self.frame_count == 321
            && matches!(
                mode.as_str(),
                "comment-lines" | "comment-block" | "multiline-indent" | "multiline-outdent"
            )
        {
            let (key, modifiers) = match mode.as_str() {
                "comment-lines" => (egui::Key::K, egui::Modifiers::COMMAND),
                "comment-block" => (egui::Key::K, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
                "multiline-outdent" => (egui::Key::Tab, egui::Modifiers::SHIFT),
                _ => (egui::Key::Tab, egui::Modifiers::NONE),
            };
            raw.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }
        if matches!(mode.as_str(), "color-picker" | "color-accept" | "color-cancel") {
            raw.events.extend(self.script_editors.demo_color_events(self.frame_count, &mode));
        }
        if matches!(mode.as_str(), "close-brace" | "object-close-brace") {
            if matches!(self.frame_count, 321 | 323) {
                raw.events.push(egui::Event::Text("}".into()));
            }
            if self.frame_count == 322 {
                raw.events.push(egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        }
        if self.frame_count == 323 && matches!(mode.as_str(), "tabstops" | "object-tabstops") {
            for key in [egui::Key::Backspace, egui::Key::Tab] {
                raw.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            raw.events.push(egui::Event::Text("// tab stops".into()));
            raw.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        if self.frame_count == 321 && matches!(mode.as_str(), "indent" | "object-indent") {
            raw.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            raw.events.push(egui::Event::Text("// indentation".into()));
        }
    }

    pub(super) fn demo_script_steps(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_SCRIPTS") else { return };
        let syntax_demo = matches!(
            mode.as_str(),
            "syntax"
                | "object-syntax"
                | "syntax-readonly"
                | "workbench"
                | "object-workbench"
                | "insert-filter"
                | "insert-empty"
                | "search"
                | "search-left"
                | "search-inline"
                | "search-overlay"
                | "search-outside"
                | "preferences"
                | "help"
                | "colors"
                | "color-picker"
                | "color-accept"
                | "color-cancel"
                | "comment-lines"
                | "comment-block"
                | "multiline-indent"
                | "multiline-outdent"
                | "folding"
                | "object-folding"
                | "word-wrap"
                | "no-word-wrap"
                | "word-wrap-preferences"
                | "slua"
        );
        if self.frame_count == 280 {
            self.panels.perf = false;
            if matches!(mode.as_str(), "word-wrap" | "no-word-wrap" | "word-wrap-preferences") {
                self.settings.script_editor.word_wrap = mode != "no-word-wrap";
            }
            if matches!(mode.as_str(), "colors" | "color-picker" | "color-accept" | "color-cancel") {
                self.world.notifications.list.clear();
            }
            if !matches!(
                mode.as_str(),
                "object"
                    | "object-readonly"
                    | "object-save"
                    | "object-syntax"
                    | "object-indent"
                    | "object-tabstops"
                    | "object-close-brace"
                    | "object-folding"
                    | "experience"
                    | "experience-save"
                    | "object-workbench"
                    | "object-preprocessor"
            ) {
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                let item = Uuid::from_u128(8102);
                if mode == "slua" {
                    // SST_LUA in the item's flags, on a region that runs SLua
                    if let Some(it) = self.world.inventory.items.get_mut(&item) {
                        it.flags |= 1;
                    }
                    if let Some(region) = self.world.main_region.and_then(|h| self.world.regions.get_mut(&h)) {
                        region.lua_scripts = true;
                    }
                }
                if mode == "readonly"
                    && let Some(it) = self.world.inventory.items.get_mut(&item)
                {
                    it.owner_mask &= !perm::MODIFY;
                }
                if mode == "syntax-readonly"
                    && let Some(mut it) = self.world.inventory.items.get(&item).cloned()
                {
                    // Copy-only library source uses the real read-only permission path.
                    it.parent = self.world.inventory.lib_root;
                    it.owner = self.world.inventory.lib_owner;
                    it.owner_mask &= !perm::MODIFY;
                    // (what the agent holds on a Library item: rules::agent_mask)
                    it.everyone_mask = it.owner_mask;
                    self.world.inventory.add_items(vec![it]);
                }
                self.panels.inventory = true;
                self.inventory_ui.show_original(&self.world.inventory, item);
                let target = ScriptTarget { object: Uuid::nil(), item };
                if mode == "lua"
                    && let Some(it) = self.world.inventory.items.get_mut(&item)
                {
                    it.flags = (it.flags & !0xff) | 1;
                    self.demo_script_sources.insert(target, LUA_DEMO_SOURCE.into());
                }
                if syntax_demo {
                    self.demo_script_sources.insert(
                        target,
                        if matches!(mode.as_str(), "colors" | "color-picker" | "color-accept" | "color-cancel") {
                            COLORS_DEMO_SOURCE
                        } else if matches!(mode.as_str(), "word-wrap" | "no-word-wrap" | "word-wrap-preferences") {
                            WORD_WRAP_DEMO_SOURCE
                        } else if mode == "slua" {
                            SLUA_DEMO_SOURCE
                        } else {
                            SYNTAX_DEMO_SOURCE
                        }
                        .into(),
                    );
                }
                self.open_script(target);
            }
            if matches!(
                mode.as_str(),
                "object"
                    | "object-readonly"
                    | "object-save"
                    | "object-syntax"
                    | "object-indent"
                    | "object-tabstops"
                    | "object-close-brace"
                    | "object-folding"
                    | "experience"
                    | "experience-save"
                    | "both"
                    | "object-workbench"
                    | "object-preprocessor"
            ) {
                let idx = self
                    .world
                    .objects
                    .iter()
                    // The plaza torus; DemoSim supplies its editable properties.
                    .find(|(_, o)| o.key.local_id == 117 && Some(o.key.region) == self.world.main_region)
                    .map(|(i, _)| i);
                if let Some(idx) = idx {
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.build.open_build(crate::build::Tool::Edit);
                    self.build.tab = 4;
                    self.build.request_contents(&self.world, idx, true);
                    self.flush_build();
                    if let Some(object) = self.world.objects.get(idx).map(|o| o.full_id)
                        && let Some(item) = self
                            .build
                            .contents
                            .objects
                            .get(&object)
                            .and_then(|inv| inv.items.iter().find(|i| i.asset_type == 10))
                            .map(|it| it.item_id)
                    {
                        let target = ScriptTarget { object, item };
                        if mode == "object-readonly"
                            && let Some(inv) = self.build.contents.objects.get_mut(&object)
                            && let Some(script) = inv.items.iter_mut().find(|it| it.item_id == item)
                        {
                            script.owner_mask &= !perm::MODIFY;
                        }
                        if syntax_demo {
                            self.demo_script_sources.insert(target, SYNTAX_DEMO_SOURCE.into());
                        }
                        self.open_script(target);
                    }
                }
            }
        }
        if self.frame_count == 320
            && matches!(
                mode.as_str(),
                "comment-lines" | "comment-block" | "multiline-indent" | "multiline-outdent"
            )
        {
            self.script_editors.demo_selection(&self.egui_ctx, mode == "multiline-outdent");
        }
        if self.frame_count == 300 && matches!(mode.as_str(), "preprocessor" | "processed" | "object-preprocessor") {
            crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
            if let Some(mut include) = self.world.inventory.items.get(&Uuid::from_u128(8102)).cloned() {
                include.id = Uuid::from_u128(8103);
                include.name = "library.lsl".into();
                let target = ScriptTarget {
                    object: Uuid::nil(),
                    item: include.id,
                };
                self.world.inventory.add_items(vec![include]);
                self.demo_script_sources.insert(
                    target,
                    "#pragma once\ngreet(integer channel) { llSay(channel, \"Bonjour, Aurora !\"); }".into(),
                );
            }
        }
        if self.frame_count == 320
            && matches!(
                mode.as_str(),
                "search"
                    | "search-left"
                    | "search-inline"
                    | "search-overlay"
                    | "search-outside"
                    | "preferences"
                    | "word-wrap-preferences"
                    | "help"
                    | "preprocessor"
                    | "processed"
                    | "object-preprocessor"
                    | "preprocessor-error"
            )
            && let Some(target) = self.script_editors.documents.first().map(|e| e.target)
        {
            self.script_editors
                .demo_tools(target, &self.egui_ctx, &mode, &mut self.settings.script_editor);
        }
        if self.frame_count == 430 && mode == "processed" {
            self.script_editors.demo_processed(&self.egui_ctx);
        }
        if self.frame_count == 320
            && matches!(
                mode.as_str(),
                "indent"
                    | "object-indent"
                    | "tabstops"
                    | "object-tabstops"
                    | "experience"
                    | "experience-save"
                    | "close-brace"
                    | "object-close-brace"
            )
            && let Some(target) = self.script_editors.documents.first().map(|e| e.target)
        {
            if matches!(mode.as_str(), "close-brace" | "object-close-brace") {
                self.script_editors
                    .demo_close_brace(target, &self.egui_ctx, mode == "object-close-brace");
            } else if matches!(mode.as_str(), "tabstops" | "object-tabstops") {
                self.script_editors.demo_tabstops(target, &self.egui_ctx);
            } else if matches!(mode.as_str(), "indent" | "object-indent") {
                self.script_editors.demo_indent(target, &self.egui_ctx, mode == "object-indent");
            } else if let Some(action) = self.script_editors.demo_experience(target, Uuid::from_u128(9001)) {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 320 && mode == "lua" {
            let target = ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(8102),
            };
            if let Some(action) = self
                .script_editors
                .demo_edit(target, LUA_DEMO_SOURCE.replace("Avatar", "Aurora"), true)
            {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 320 && matches!(mode.as_str(), "error" | "save" | "object-save") {
            let target = if mode == "object-save" {
                let Some(target) = self
                    .script_editors
                    .documents
                    .iter()
                    .find(|e| !e.target.object.is_nil())
                    .map(|e| e.target)
                else {
                    return;
                };
                target
            } else {
                ScriptTarget {
                    object: Uuid::nil(),
                    item: Uuid::from_u128(8102),
                }
            };
            let source = if mode == "error" {
                DEFAULT_SOURCE.replace("llSay(0, \"Hello, Avatar!\");", "AURORA_DEMO_SYNTAX_ERROR")
            } else {
                DEFAULT_SOURCE.replace("Hello, Avatar!", "Bonjour, Aurora !")
            };
            if let Some(action) = self.script_editors.demo_edit(target, source, true) {
                self.apply_script_action(action);
            }
        }
        if self.frame_count == 360
            && matches!(
                mode.as_str(),
                "save" | "object-save" | "experience-save" | "preprocessor" | "processed" | "object-preprocessor"
            )
        {
            let Some(target) = self.script_editors.documents.first().map(|e| e.target) else {
                return;
            };
            if self.script_editors.demo_close_clean(target) {
                // A listing may redact the asset UUID after a successful save.
                // Reopening must still load the saved source via the item.
                if target.object.is_nil() {
                    if let Some(item) = self.world.inventory.items.get_mut(&target.item) {
                        item.asset_id = Uuid::nil();
                    }
                } else if let Some(inv) = self.build.contents.objects.get_mut(&target.object)
                    && let Some(item) = inv.items.iter_mut().find(|i| i.item_id == target.item)
                {
                    item.asset_id = Uuid::nil();
                }
                self.open_script(target);
                log::info!("demo scripts: reopened saved source with a hidden asset UUID");
            }
        }
    }

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
        if self.world.rlva.has("viewscript") {
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
        if let Some(request) = self
            .script_editors
            .open(target, info.name, info.can_read, info.can_edit, info.flags)
        {
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

    pub(super) fn show_scripts(&mut self, ctx: &egui::Context, p: &Palette) {
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
        for action in self
            .script_editors
            .show(ctx, p, &mut self.settings.script_editor, &mut self.settings.build.color_picker)
        {
            self.apply_script_action(action);
        }
        // the copies of the unsaved scripts (ui/scripts/backup.rs)
        self.sync_script_backup_dir();
        self.script_editors.back_up();
        self.script_editors.show_debug(ctx, p, &mut self.world.script_debug);
    }

    pub(super) fn apply_script_action(&mut self, action: Action) {
        match action {
            Action::Debug => self.script_editors.open_debug(),
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

    pub(super) fn demo_script_command(&mut self, cmd: &NetCommand) -> bool {
        match cmd {
            NetCommand::LoadScriptExperiences { request, target, .. } => {
                use aurora_net::scripts::experiences::{ScriptExperience, ScriptExperiences};
                let selected = self.demo_script_experiences.get(target).copied().unwrap_or_default();
                self.on_app_event(NetEvent::ScriptExperiences {
                    request: *request,
                    target: *target,
                    result: ScriptExperiences {
                        associated: Ok(selected),
                        available: Ok(vec![ScriptExperience {
                            id: Uuid::from_u128(9001),
                            name: aurora_i18n::tr!("script-experience-demo").into(),
                            selectable: true,
                        }]),
                    },
                });
            }
            NetCommand::LoadScript { request, target, .. } => {
                let source = self
                    .demo_script_sources
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_SOURCE.into());
                self.on_app_event(NetEvent::ScriptLoaded {
                    request: *request,
                    target: *target,
                    result: Ok(source),
                });
            }
            NetCommand::SaveScript {
                request,
                target,
                source,
                options,
                ..
            } => {
                // A named marker simulates a server diagnostic in demo mode;
                // this is deliberately not a local LSL compiler.
                let compiled = !source.contains("AURORA_DEMO_SYNTAX_ERROR");
                log::info!("demo scripts: saved {} for target {}", target.item, options.compile.name());
                let asset = Uuid::new_v4();
                self.demo_script_sources.insert(*target, source.clone());
                self.demo_script_experiences.insert(*target, options.experience);
                self.build_demo
                    .save_script(*target, asset, compiled && options.running, options.compile);
                let result = Ok(ScriptSaved {
                    asset,
                    compiled,
                    checked: true,
                    errors: if compiled {
                        vec![]
                    } else {
                        vec!["(5, 9) : ERROR : Syntax error".into()]
                    },
                });
                self.on_app_event(NetEvent::ScriptSaved {
                    request: *request,
                    target: *target,
                    result,
                });
            }
            _ => return false,
        }
        true
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

#[cfg(test)]
mod tests {
    use super::*;

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
