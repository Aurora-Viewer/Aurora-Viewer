//! Script editor actions, permission checks and offline simulated assets.

use super::*;
use crate::ui::scripts::Action;
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

struct ScriptInfo {
    name: String,
    handle: u64,
    owner: Uuid,
    asset: Uuid,
    can_read: bool,
    can_edit: bool,
}

fn permissions(mask: u32, library: bool) -> (bool, bool) {
    let copy = mask & perm::COPY != 0;
    let modify = mask & perm::MODIFY != 0;
    (copy && (modify || library), copy && modify && !library)
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
                    self.world.inventory.add_items(vec![it]);
                }
                self.panels.inventory = true;
                self.inventory_ui.show_original(&self.world.inventory, item);
                let target = ScriptTarget { object: Uuid::nil(), item };
                if syntax_demo {
                    self.demo_script_sources.insert(
                        target,
                        if matches!(mode.as_str(), "colors" | "color-picker" | "color-accept" | "color-cancel") {
                            COLORS_DEMO_SOURCE
                        } else if matches!(mode.as_str(), "word-wrap" | "no-word-wrap" | "word-wrap-preferences") {
                            WORD_WRAP_DEMO_SOURCE
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
            let (can_read, can_edit) = permissions(it.owner_mask, library);
            Some(ScriptInfo {
                name: it.name.clone(),
                handle: self.world.main_region.unwrap_or_default(),
                owner: it.owner,
                asset: it.asset_id,
                can_read,
                can_edit,
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
            let mask = if it.owner_id == self.world.agent_id || it.group_owned && member {
                it.owner_mask
            } else {
                it.everyone_mask | if member { it.group_mask } else { 0 }
            };
            let (can_read, can_edit) = permissions(mask, false);
            Some(ScriptInfo {
                name: it.name.clone(),
                handle: object.key.region,
                owner: it.owner_id,
                asset: it.asset_id,
                can_read,
                can_edit,
            })
        }
    }

    pub(super) fn open_script(&mut self, target: ScriptTarget) {
        if self.world.rlva.has("viewscript") {
            return;
        }
        let Some(info) = self.script_info(target) else { return };
        if let Some(request) = self.script_editors.open(target, info.name, info.can_read, info.can_edit) {
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
        self.script_editors.show_debug(ctx, p, &mut self.world.script_debug);
    }

    pub(super) fn apply_script_action(&mut self, action: Action) {
        match action {
            Action::Debug => self.script_editors.open_debug(),
            Action::Include { request, name } => {
                let target = self
                    .world
                    .inventory
                    .items
                    .values()
                    .find(|item| item.asset_type == 10 && item.name == name)
                    .map(|item| ScriptTarget {
                        object: Uuid::nil(),
                        item: item.id,
                    });
                let info = target.and_then(|target| self.script_info(target).filter(|info| info.can_read).map(|info| (target, info)));
                if let Some((target, info)) = info {
                    self.script_editors.include_asset(request, info.asset);
                    self.send(NetCommand::LoadScript {
                        request,
                        target,
                        handle: info.handle,
                        owner: info.owner,
                        asset: info.asset,
                    });
                } else {
                    self.script_editors.loaded(
                        request,
                        ScriptTarget {
                            object: Uuid::nil(),
                            item: Uuid::nil(),
                        },
                        Err(aurora_i18n::tr!("script-preproc-include-missing", name = name)),
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
                if let Some(info) = self.script_info(target).filter(|i| i.can_edit) {
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
                let asset = Uuid::new_v4();
                self.demo_script_sources.insert(*target, source.clone());
                self.demo_script_experiences.insert(*target, options.experience);
                self.build_demo
                    .save_script(*target, asset, compiled && options.running, options.mono);
                let result = Ok(ScriptSaved {
                    asset,
                    compiled,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_source_requires_copy_and_modify_except_read_only_library() {
        assert_eq!(permissions(perm::COPY | perm::MODIFY, false), (true, true));
        assert_eq!(permissions(perm::COPY, false), (false, false));
        assert_eq!(permissions(perm::MODIFY, false), (false, false));
        assert_eq!(permissions(perm::COPY, true), (true, false));
        assert_eq!(permissions(perm::COPY | perm::MODIFY, true), (true, false));
    }
}
