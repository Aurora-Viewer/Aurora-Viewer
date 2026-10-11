//! Script editor actions, permission checks and offline simulated assets.

use super::*;
use crate::ui::scripts::Action;
use aurora_net::build::{BuildCmd, perm};
use aurora_net::scripts::{DEFAULT_SOURCE, ScriptSaved, ScriptTarget};
use uuid::Uuid;

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
    pub(super) fn demo_script_steps(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_SCRIPTS") else { return };
        if self.frame_count == 280 {
            self.panels.perf = false;
            if !matches!(mode.as_str(), "object" | "object-save") {
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                let item = Uuid::from_u128(8102);
                if mode == "readonly"
                    && let Some(it) = self.world.inventory.items.get_mut(&item)
                {
                    it.owner_mask &= !perm::MODIFY;
                }
                self.panels.inventory = true;
                self.inventory_ui.show_original(&self.world.inventory, item);
                self.open_script(ScriptTarget { object: Uuid::nil(), item });
            }
            if matches!(mode.as_str(), "object" | "object-save" | "both") {
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
                        self.open_script(ScriptTarget { object, item });
                    }
                }
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
        if self.frame_count == 360 && matches!(mode.as_str(), "save" | "object-save") {
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
        let Some(info) = self.script_info(target) else { return };
        if let Some(request) = self.script_editors.open(target, info.name, info.can_read, info.can_edit) {
            self.send(NetCommand::LoadScript {
                request,
                target,
                handle: info.handle,
                owner: info.owner,
                asset: info.asset,
            });
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
        for action in self.script_editors.show(ctx, p) {
            self.apply_script_action(action);
        }
    }

    fn apply_script_action(&mut self, action: Action) {
        match action {
            Action::Save {
                request,
                target,
                mono,
                running,
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
                    mono,
                    running,
                    source,
                });
            }
            Action::Reload { request, target } => {
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
                mono,
                running,
                ..
            } => {
                // A named marker simulates a server diagnostic in demo mode;
                // this is deliberately not a local LSL compiler.
                let compiled = !source.contains("AURORA_DEMO_SYNTAX_ERROR");
                let asset = Uuid::new_v4();
                self.demo_script_sources.insert(*target, source.clone());
                self.build_demo.save_script(*target, asset, compiled && *running, *mono);
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
