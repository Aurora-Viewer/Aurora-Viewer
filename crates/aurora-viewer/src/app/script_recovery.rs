//! The scripts an earlier session left with unsaved changes: their offer at
//! login (`ui/scripts/recovery.rs`) and their recovery into Lost and Found.
//! Port of LLScriptRecoverQueue (recoverIfNeeded, called from
//! LLAppViewer::handleLoginComplete at every login; recoverNext,
//! onCreateScript, onSavedScript; indra/newview/llfloaterscriptrecover.cpp,
//! originally LGPL 2.1).

use super::*;
use crate::ui::scripts::{backup, recovery};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// The name the server cannot take for a new script; created as
/// `New_Script`, renamed once its text is saved (fixNewScriptDefaultName,
/// onSavedScript).
const NEW_SCRIPT: &str = "New Script";

#[derive(Default)]
pub(super) struct Recovery {
    scan: Option<crossbeam_channel::Receiver<Vec<recovery::Row>>>,
    offer: Option<recovery::Offer>,
    /// Scripts being created, by request: the copy's file and its name.
    pending: HashMap<Uuid, (PathBuf, String)>,
    /// The last script recovered, shown in the inventory once all are.
    last: Option<Uuid>,
    /// The account the editors copy for.
    folder: Option<Uuid>,
}

/// The name a recovered script is created with.
fn creation_name(name: &str) -> String {
    if name == NEW_SCRIPT { name.replace(' ', "_") } else { name.to_owned() }
}

/// LLViewerAssetType::generateDescriptionFor(AT_LSL_TEXT): the local time
/// and the asset type's human readable name.
fn description(now: chrono::DateTime<chrono::Local>) -> String {
    format!("{} lsl2 script", now.format("%Y-%m-%d %H:%M:%S"))
}

/// `AURORA_DEMO_SCRIPT_RECOVERY`: the demo keeps no copies otherwise, so
/// that a demo run never offers another one's scripts.
fn demo_mode() -> Option<String> {
    std::env::var("AURORA_DEMO_SCRIPT_RECOVERY").ok()
}

impl App {
    /// The editors copy into the account's folder while in the world; the
    /// folder is looked up when the account changes, not at every frame.
    pub(super) fn sync_script_backup_dir(&mut self) {
        let agent = self.world.agent_id;
        let want = (self.in_world() && !agent.is_nil() && (!self.demo || demo_mode().is_some())).then_some(agent);
        if self.script_recovery.folder != want {
            self.script_recovery.folder = want;
            self.script_editors.backup_dir = want.map(|agent| backup::folder(&agent));
        }
    }

    /// At every login: list the copies left by an earlier session.
    pub(super) fn script_recovery_login(&mut self, agent: Uuid) {
        self.script_recovery = Recovery::default();
        if self.demo {
            let Some(mode) = demo_mode() else { return };
            if mode != "edit" {
                self.demo_script_leftover(agent);
            }
        }
        self.script_recovery.scan = Some(recovery::scan(backup::folder(&agent)));
    }

    /// The demo's leftover copy, as a crash would leave it.
    fn demo_script_leftover(&self, agent: Uuid) {
        let target = aurora_net::scripts::ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(8102),
        };
        let copy = backup::Copy {
            name: aurora_i18n::tr!("script-recovery-demo-name").into(),
            source: aurora_net::scripts::DEFAULT_SOURCE.replace("Hello, Avatar!", "Unsaved before the crash"),
            saved: chrono::Local::now().timestamp(),
        };
        backup::apply(backup::Job::Write(backup::file(&backup::folder(&agent), target), copy));
    }

    pub(super) fn show_script_recovery(&mut self, ctx: &egui::Context, p: &Palette) {
        if let Some(rows) = self.script_recovery.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.script_recovery.scan = None;
            if !rows.is_empty() {
                self.script_recovery.offer = Some(recovery::Offer { rows });
            }
        }
        let Some(offer) = self.script_recovery.offer.as_mut() else { return };
        if let Some(answer) = recovery::window(ctx, p, offer) {
            self.script_recovery_answer(answer);
        }
    }

    /// The copy goes, unless an editor open now writes it (the same script
    /// opened again before the offer was answered).
    fn forget_script_copy(&self, path: PathBuf) {
        if !self.script_editors.backups.owns(&path) {
            backup::run(vec![backup::Job::Remove(path)]);
        }
    }

    pub(super) fn script_recovery_answer(&mut self, answer: recovery::Answer) {
        let Some(offer) = self.script_recovery.offer.take() else { return };
        match answer {
            recovery::Answer::Later => log::info!("script recovery: left for the next login"),
            recovery::Answer::Cancel => {
                log::info!("script recovery: {} copy(ies) dropped", offer.rows.len());
                for row in offer.rows {
                    self.forget_script_copy(row.path);
                }
            }
            recovery::Answer::Recover => {
                // LLScriptRecoverQueue::recoverNext: into Lost and Found
                let Some(folder) = crate::world::inventory::actions::system(&self.world.inventory, 16) else {
                    self.world.system_notice(
                        aurora_i18n::tr!("script-recovery-no-folder"),
                        crate::world::notice_topics::Topic::Warning,
                    );
                    self.script_recovery.offer = Some(offer);
                    return;
                };
                let description = description(chrono::Local::now());
                for row in offer.rows {
                    if !row.chosen {
                        self.forget_script_copy(row.path);
                        continue;
                    }
                    let name = recovery::display_name(&row.name);
                    let request = Uuid::new_v4();
                    self.script_recovery.pending.insert(request, (row.path, name.clone()));
                    self.send(NetCommand::CreateScriptItem {
                        request,
                        parent: folder,
                        name: creation_name(&name),
                        description: description.clone(),
                        source: row.source,
                    });
                }
            }
        }
    }

    /// The answer to a recovered script's creation; false for another
    /// request.
    pub(super) fn script_recovery_saved(&mut self, request: Uuid, item: Uuid, result: &Result<aurora_net::inventory::InvItem, String>) -> bool {
        let Some((path, name)) = self.script_recovery.pending.remove(&request) else {
            return false;
        };
        match result {
            Ok(saved) => {
                log::info!("script recovery: a script recovered into Lost and Found");
                self.world.inventory.add_items(vec![saved.clone()]);
                if name == NEW_SCRIPT
                    && let Ok(change) =
                        crate::world::inventory::actions::patch(&self.world.inventory, saved.id, aurora_llsd::llsd_map! { "name" => name.as_str() })
                {
                    self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(change));
                }
                if !self.demo {
                    self.send(NetCommand::FetchItems {
                        items: vec![saved.id],
                        owner: self.world.agent_id,
                    });
                }
                self.forget_script_copy(path);
                self.script_recovery.last = Some(saved.id);
            }
            Err(reason) => {
                // onSavedScript: the empty item goes to the Trash, the copy
                // stays for the next login
                log::warn!("script recovery: not recovered: {reason}");
                if !item.is_nil()
                    && self.world.inventory.items.contains_key(&item)
                    && let Some(trash) = crate::world::inventory::actions::system(&self.world.inventory, 14)
                    && let Ok(change) = crate::world::inventory::actions::move_selection(
                        &self.world.inventory,
                        &[item],
                        trash,
                        &self.settings.inventory.protected,
                        &HashSet::new(),
                    )
                {
                    self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(change));
                }
                self.world.system_notice(
                    aurora_i18n::tr!("script-recovery-failed", name = name.as_str(), reason = reason.as_str()),
                    crate::world::notice_topics::Topic::Warning,
                );
            }
        }
        // the queue done: Lost and Found shown in the inventory
        if self.script_recovery.pending.is_empty()
            && let Some(last) = self.script_recovery.last.take()
        {
            self.panels.inventory = true;
            self.inventory_ui.show_original(&self.world.inventory, last);
        }
        true
    }

    /// The demo's simulator for a recovered script: the item, at once.
    pub(super) fn demo_create_script(&mut self, request: Uuid, parent: Uuid, name: &str, description: &str, source: &str) {
        let item = aurora_net::inventory::InvItem {
            id: Uuid::new_v4(),
            parent,
            name: name.to_owned(),
            desc: description.to_owned(),
            asset_type: 10,
            inv_type: 10,
            asset_id: Uuid::new_v4(),
            owner: self.world.agent_id,
            creator: self.world.agent_id,
            base_mask: 0x7fff_ffff,
            owner_mask: 0x7fff_ffff,
            next_owner_mask: 0x0008_2000,
            ..Default::default()
        };
        self.demo_script_sources.insert(
            aurora_net::scripts::ScriptTarget {
                object: Uuid::nil(),
                item: item.id,
            },
            source.to_owned(),
        );
        self.world.inventory.add_items(vec![item.clone()]);
        self.on_app_event(NetEvent::InventoryContentSaved {
            request,
            item: item.id,
            result: Ok(item),
        });
    }

    /// AURORA_DEMO_SCRIPT_RECOVERY: `recover` answers the offer; `edit`
    /// types in the demo script without saving, so that its copy is
    /// written.
    pub(super) fn demo_script_recovery_steps(&mut self) {
        let Some(mode) = demo_mode() else { return };
        match (mode.as_str(), self.frame_count) {
            ("recover", 400) => self.script_recovery_answer(recovery::Answer::Recover),
            ("edit", 280) => {
                crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                self.open_script(aurora_net::scripts::ScriptTarget {
                    object: Uuid::nil(),
                    item: Uuid::from_u128(8102),
                });
            }
            ("edit", 320) => {
                let target = aurora_net::scripts::ScriptTarget {
                    object: Uuid::nil(),
                    item: Uuid::from_u128(8102),
                };
                let source = aurora_net::scripts::DEFAULT_SOURCE.replace("Hello, Avatar!", "Typed, not saved");
                let _ = self.script_editors.demo_edit(target, source, false);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recovered_script_keeps_its_name_except_the_default_one() {
        assert_eq!(creation_name("Door"), "Door");
        assert_eq!(creation_name("New Script"), "New_Script");
        assert_eq!(creation_name("New Script 2"), "New Script 2");
    }

    #[test]
    fn the_description_is_the_legacy_one() {
        use chrono::TimeZone;
        let at = chrono::Local.with_ymd_and_hms(2026, 11, 11, 9, 5, 3).single().expect("a local time");
        assert_eq!(description(at), "2026-11-11 09:05:03 lsl2 script");
    }
}
