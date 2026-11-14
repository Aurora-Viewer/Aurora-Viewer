//! Object inventories for the Contents tab (LLPanelContents,
//! LLPanelObjectInventory, LLViewerObject::requestInventory / saveScript /
//! removeInventory / updateInventory / moveInventory, LLToolDragAndDrop::
//! dropScript / dropInventory; originally LGPL 2.1).

use super::BuildTool;
use crate::world::World;
use aurora_net::build::{BuildCmd, TaskItem, perm};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// AT_LSL_TEXT / IT_LSL.
pub const AT_LSL_TEXT: i8 = 10;
pub const IT_LSL: i8 = 10;
/// AT_NOTECARD / IT_NOTECARD.
pub const AT_NOTECARD: i8 = 7;
pub const IT_NOTECARD: i8 = 7;
/// The capability that makes new contents items (LLPanelContents::getState).
pub const CREATE_TASK_ITEM_CAP: &str = "CreateTaskInventoryItem";
/// PERM_ALL (llpermissionsflags.h).
const PERM_ALL: u32 = 0x7fff_ffff;
/// DEFAULT_PRICE of new items.
const DEFAULT_PRICE: i32 = 10;
/// A request without answer is sent again after this long.
const RETRY: Duration = Duration::from_secs(8);

/// What we know of one object's inventory.
#[derive(Debug, Clone, Default)]
pub struct TaskInventory {
    /// The simulator's inventory serial of this listing.
    pub serial: i16,
    /// The ObjectProperties inventory serial the last request was made for.
    asked_serial: Option<i16>,
    pub items: Vec<TaskItem>,
    /// Waiting for (another) listing.
    pub loading: bool,
    pub requested: Option<Instant>,
    /// Reset every script once the listing comes ("Reset scripts"
    /// pressed before it was known).
    pub reset_pending: bool,
}

#[derive(Debug, Default)]
pub struct Contents {
    pub objects: HashMap<Uuid, TaskInventory>,
    /// ScriptRunningReply: (object, item) -> (running, compile target).
    pub running: HashMap<(Uuid, Uuid), (bool, Option<aurora_net::scripts::CompileTarget>)>,
}

/// "YYYY-MM-DD HH:MM:SS lsl2 script" (LLViewerAssetType::generateDescriptionFor:
/// new item descriptions carry their creation time, UTC here, and the
/// asset type's human-readable name).
fn new_item_description(unix: i64, kind: &str) -> String {
    let (y, m, d) = aurora_net::profile::civil_from_days(unix.div_euclid(86_400));
    let s = unix.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} {kind}", s / 3600, s / 60 % 60, s % 60)
}

fn new_script_description(unix: i64) -> String {
    new_item_description(unix, "lsl2 script")
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl BuildTool {
    /// The object the Contents tab shows: the primary selected prim
    /// (its root unless "Linked edit").
    pub fn contents_target(&self, world: &World) -> Option<usize> {
        let key = self.selection.first()?;
        world.objects.index_of(key)
    }

    /// RequestTaskInventory, unless a listing is current or on its way
    /// (LLPanelObjectInventory::refresh: the ObjectProperties inventory
    /// serial changed, or nothing yet).
    pub fn request_contents(&mut self, world: &World, idx: usize, force: bool) {
        let Some(o) = world.objects.get(idx) else { return };
        let id = o.full_id;
        let serial = self.props.get(&id).map(|p| p.inventory_serial);
        let inv = self.contents.objects.entry(id).or_default();
        // asked again when the properties announce another serial (once)
        let stale = (serial.is_some() && serial != inv.asked_serial) || inv.requested.is_none();
        let waiting = inv.loading && inv.requested.is_some_and(|t| t.elapsed() < RETRY);
        if waiting || !(force || stale) {
            return;
        }
        inv.loading = true;
        inv.requested = Some(Instant::now());
        inv.asked_serial = serial;
        let (handle, local_id) = (o.key.region, o.key.local_id);
        self.send(BuildCmd::RequestTaskInventory {
            handle,
            local_id,
            object_id: id,
        });
    }

    /// NetEvent::TaskInventory.
    pub fn on_task_inventory(&mut self, world: &World, object_id: Uuid, serial: Option<i16>, items: Vec<TaskItem>) {
        let inv = self.contents.objects.entry(object_id).or_default();
        if let Some(s) = serial {
            inv.serial = s;
        }
        inv.items = items;
        inv.loading = false;
        // A newer successful listing supersedes an earlier request error.
        if let Some(job) = self.replacement.job.as_mut()
            && job.inventory_error.as_ref().is_some_and(|(id, _)| *id == object_id)
        {
            job.inventory_error = None;
        }
        if std::mem::take(&mut inv.reset_pending)
            && let Some(idx) = world.objects.index_of_uuid(&object_id)
        {
            self.reset_scripts_of(world, idx);
        }
        // ask whether the scripts run (LLPanelObjectInventory shows it)
        let scripts: Vec<Uuid> = self.contents.objects[&object_id]
            .items
            .iter()
            .filter(|i| i.asset_type == AT_LSL_TEXT)
            .map(|i| i.item_id)
            .collect();
        if let Some(o) = world.objects.index_of_uuid(&object_id).and_then(|i| world.objects.get(i)) {
            let handle = o.key.region;
            for item_id in scripts.into_iter().take(64) {
                self.send(BuildCmd::GetScriptRunning {
                    handle,
                    object_id,
                    item_id,
                });
            }
        }
        if self.contents.objects.len() > 256 {
            let keep: Vec<Uuid> = self
                .selection
                .iter()
                .filter_map(|k| world.objects.index_of(k).and_then(|i| world.objects.get(i)).map(|o| o.full_id))
                .collect();
            let mut keep = keep;
            if let Some(job) = self.replacement.job.as_ref().filter(|job| !job.stage.terminal()) {
                keep.extend(job.inventory_ids());
            }
            self.contents.objects.retain(|k, _| keep.contains(k));
        }
    }

    /// LLPanelContents::getState: may we add to / reset this object's contents?
    /// A god may, whatever the object (isGodlike() first).
    pub fn contents_editable(&self, world: &World, idx: usize) -> bool {
        if world.god_level > 0 {
            return world.objects.get(idx).is_some();
        }
        if !self.can_modify(world, idx) {
            return false;
        }
        let Some(o) = world.objects.get(idx) else { return false };
        let Some(p) = self.props.get(&o.full_id) else {
            return o.owner_id == world.agent_id;
        };
        let member = !p.group_id.is_nil() && world.groups.is_member(&p.group_id);
        p.owner_id == world.agent_id || member
    }

    /// Whether the object's region makes new contents items itself
    /// (CreateTaskInventoryItem; LLPanelContents::getState enables "New
    /// Notecard" only then).
    pub fn contents_create_cap(world: &World, idx: usize) -> bool {
        world
            .objects
            .get(idx)
            .and_then(|o| world.regions.get(&o.key.region))
            .is_some_and(|r| r.caps.contains_key(CREATE_TASK_ITEM_CAP))
    }

    /// A new item of ours for the contents of `idx`, as
    /// LLPanelContents::createTaskInventoryItemHelper fills it: full rights
    /// for us, the creation defaults of its category added on its way out
    /// (`creation_permissions::Defaults::prepare`).
    fn new_task_item(world: &World, parent: Uuid, asset_type: i8, inv_type: i8, name: String, description: String) -> TaskItem {
        let now = now_unix();
        TaskItem {
            item_id: Uuid::nil(),
            parent_id: parent,
            creator_id: world.agent_id,
            owner_id: world.agent_id,
            base_mask: PERM_ALL,
            owner_mask: PERM_ALL,
            group_mask: 0,
            everyone_mask: 0,
            // ScriptsNextOwner* defaults: move and transfer
            next_owner_mask: perm::MOVE | perm::TRANSFER,
            asset_type,
            inv_type,
            sale_price: DEFAULT_PRICE,
            name,
            description,
            creation_date: now as i32,
            ..Default::default()
        }
    }

    /// "New script" (LLPanelContents::onNewScriptFlyoutCommit +
    /// createTaskInventoryItemHelper): the region writes its default script
    /// through CreateTaskInventoryItem, running, in Luau for SLua and Mono
    /// otherwise. Without the capability, RezScript of the same item
    /// (LLViewerObject::saveScript), which uses the active group,
    /// independent of the parcel's rez group (indra/newview/llviewerobject.cpp);
    /// an SLua script carries SST_LUA in its flags and the server then
    /// writes its default SLua script.
    pub fn new_script(&mut self, world: &World, idx: usize, lua: bool) {
        let Some(o) = world.objects.get(idx) else { return };
        let mut item = Self::new_task_item(
            world,
            o.full_id,
            AT_LSL_TEXT,
            IT_LSL,
            aurora_i18n::tr!("extra-new-script").into(),
            new_script_description(now_unix()),
        );
        item.flags = u32::from(lua);
        let (handle, local_id, object_id) = (o.key.region, o.key.local_id, o.full_id);
        if Self::contents_create_cap(world, idx) {
            self.create_task_item(handle, item);
            return;
        }
        self.send(BuildCmd::RezScript {
            handle,
            local_id,
            enabled: true,
            group_id: world.groups.active,
            item: Box::new(item),
        });
        self.contents_changed(object_id);
    }

    /// "New Notecard" (LLPanelContents::onNewNotecardCommit): only through
    /// CreateTaskInventoryItem, which writes the empty notecard; there is no
    /// fallback, the button is disabled without the capability.
    pub fn new_notecard(&mut self, world: &World, idx: usize) {
        let Some(o) = world.objects.get(idx) else { return };
        if !Self::contents_create_cap(world, idx) {
            return;
        }
        let item = Self::new_task_item(
            world,
            o.full_id,
            AT_NOTECARD,
            IT_NOTECARD,
            aurora_i18n::tr!("extra-new-notecard").into(),
            new_item_description(now_unix(), "note card"),
        );
        self.create_task_item(o.key.region, item);
    }

    fn create_task_item(&mut self, handle: aurora_net::RegionHandle, item: TaskItem) {
        let tag = self.next_tag;
        self.next_tag += 1;
        self.caps.insert(tag, super::edits::CapPurpose::CreateTaskItem(item.parent_id));
        self.send(BuildCmd::CreateTaskItem {
            handle,
            item: Box::new(item),
            tag,
        });
    }

    /// The CreateTaskInventoryItem answer: on success the listing is asked
    /// again (LLViewerObject::createInventoryItemCoro: dirtyInventory +
    /// requestInventory); a failure is logged, like the Legacy Viewer, and
    /// also said in the status line.
    pub(super) fn on_task_item_created(&mut self, object_id: Uuid, result: Result<aurora_llsd::Llsd, String>) {
        match result {
            Ok(v) if aurora_net::task_inventory::create_item_succeeded(&v) => self.contents_changed(object_id),
            Ok(v) => {
                let message = v["message"].as_str().to_owned();
                log::warn!("CreateTaskInventoryItem failed: {message}");
                self.status = aurora_i18n::tr!("build-contents-create-failed", message = message);
            }
            Err(e) => {
                log::warn!("CreateTaskInventoryItem failed: {e}");
                self.status = aurora_i18n::tr!("build-contents-create-failed", message = e);
            }
        }
    }

    /// RemoveTaskInventory (LLViewerObject::removeInventory).
    pub fn remove_item(&mut self, world: &World, idx: usize, item_id: Uuid) {
        let Some(o) = world.objects.get(idx) else { return };
        let (handle, local_id, object_id) = (o.key.region, o.key.local_id, o.full_id);
        self.send(BuildCmd::RemoveTaskInventory { handle, local_id, item_id });
        if let Some(inv) = self.contents.objects.get_mut(&object_id) {
            inv.items.retain(|i| i.item_id != item_id);
        }
        self.contents_changed(object_id);
    }

    /// UpdateTaskInventory with a new name (LLTaskInvFVBridge::renameItem).
    pub fn rename_item(&mut self, world: &World, idx: usize, item_id: Uuid, name: &str) {
        let Some(o) = world.objects.get(idx) else { return };
        let (handle, local_id, object_id) = (o.key.region, o.key.local_id, o.full_id);
        let Some(inv) = self.contents.objects.get_mut(&object_id) else {
            return;
        };
        let Some(item) = inv.items.iter_mut().find(|i| i.item_id == item_id) else {
            return;
        };
        item.name = name.chars().filter(|c| *c != '|').take(63).collect();
        let item = Box::new(item.clone());
        self.send(BuildCmd::UpdateTaskInventory { handle, local_id, item });
        self.contents_changed(object_id);
    }

    /// LLPreview::onCommit (indra/newview/llpreview.cpp): update metadata
    /// through UpdateTaskInventory, keeping the notecard asset unchanged.
    pub fn describe_item(&mut self, world: &World, idx: usize, item_id: Uuid, description: &str) {
        let Some(o) = world.objects.get(idx) else { return };
        let (handle, local_id, object_id) = (o.key.region, o.key.local_id, o.full_id);
        let Some(inv) = self.contents.objects.get_mut(&object_id) else {
            return;
        };
        let Some(item) = inv.items.iter_mut().find(|i| i.item_id == item_id) else {
            return;
        };
        item.description = description.chars().filter(|c| *c != '|').take(127).collect();
        let item = Box::new(item.clone());
        self.send(BuildCmd::UpdateTaskInventory { handle, local_id, item });
        self.contents_changed(object_id);
    }

    /// Aurora's selected-items menu uses the Open window's folder copy
    /// (LLFloaterOpenObject::moveToInventory), rather than sorting by type.
    pub fn copy_contents_to_inventory(&mut self, world: &World, idx: usize, items: &[TaskItem]) {
        let Some(o) = world.objects.get(idx) else { return };
        self.ui.requests.push(super::ui::Request::CopyContents {
            object: o.full_id,
            items: items.to_vec(),
        });
    }

    /// "Reset scripts": ScriptReset for every script of every selected
    /// object (the reset queue), listing them first when needed.
    pub fn reset_scripts(&mut self, world: &World) {
        let targets: Vec<usize> = if self.selection_linked {
            self.sel_prims(world)
        } else {
            self.linkset_prims(world)
        };
        for idx in targets {
            let Some(id) = world.objects.get(idx).map(|o| o.full_id) else {
                continue;
            };
            match self.contents.objects.get_mut(&id) {
                Some(inv) if !inv.loading && inv.requested.is_some() => self.reset_scripts_of(world, idx),
                _ => {
                    self.contents.objects.entry(id).or_default().reset_pending = true;
                    self.request_contents(world, idx, true);
                }
            }
        }
    }

    fn reset_scripts_of(&mut self, world: &World, idx: usize) {
        let Some(o) = world.objects.get(idx) else { return };
        let (handle, object_id) = (o.key.region, o.full_id);
        let scripts: Vec<Uuid> = self
            .contents
            .objects
            .get(&object_id)
            .map(|inv| {
                inv.items
                    .iter()
                    .filter(|i| i.asset_type == AT_LSL_TEXT)
                    .map(|i| i.item_id)
                    .collect()
            })
            .unwrap_or_default();
        for item_id in scripts {
            self.send(BuildCmd::ScriptReset {
                handle,
                object_id,
                item_id,
            });
        }
    }

    /// Script running / not running ("Running" of the menu).
    pub fn set_script_running(&mut self, world: &World, idx: usize, item_id: Uuid, running: bool) {
        let Some(o) = world.objects.get(idx) else { return };
        let (handle, object_id) = (o.key.region, o.full_id);
        self.send(BuildCmd::SetScriptRunning {
            handle,
            object_id,
            item_id,
            running,
        });
        self.contents.running.insert((object_id, item_id), (running, None));
    }

    /// Inventory dropped on the Contents tab, every item already accepted
    /// (`contents_drop::plan`): sent in order, no-copy items leaving the
    /// inventory (LLToolDragAndDrop::dropInventory / dropScript).
    pub fn drop_inventory(&mut self, world: &mut World, idx: usize, drops: Vec<super::contents_drop::ContentsDrop>) {
        let Some(object_id) = world.objects.get(idx).map(|o| o.full_id) else {
            return;
        };
        for drop in drops {
            self.send(drop.command);
            if let Some(id) = drop.consume {
                world.inventory.consume_item(id);
            }
        }
        self.contents_changed(object_id);
    }

    /// The object's contents changed on our side: list them again soon
    /// (the simulator sends new properties with a new serial).
    pub(crate) fn contents_changed(&mut self, object_id: Uuid) {
        if let Some(inv) = self.contents.objects.get_mut(&object_id) {
            inv.loading = false;
            inv.requested = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_description_date() {
        // 2023-11-14 22:13:20 UTC
        assert_eq!(new_script_description(1_700_000_000), "2023-11-14 22:13:20 lsl2 script");
    }

    fn fixture(cap: bool) -> (World, BuildTool, usize) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(118)).unwrap();
        let region = world.objects.get(idx).unwrap().key.region;
        let caps = std::sync::Arc::make_mut(&mut world.regions.get_mut(&region).unwrap().caps);
        if cap {
            caps.insert(CREATE_TASK_ITEM_CAP.into(), "https://cap.invalid/create".into());
        } else {
            caps.remove(CREATE_TASK_ITEM_CAP);
        }
        (world, BuildTool::default(), idx)
    }

    fn created(tool: &BuildTool) -> Vec<(u64, TaskItem)> {
        tool.out
            .iter()
            .filter_map(|c| match c {
                aurora_net::NetCommand::Build(BuildCmd::CreateTaskItem { tag, item, .. }) => Some((*tag, (**item).clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn new_items_go_through_create_task_inventory_item_when_the_region_has_it() {
        let (world, mut tool, idx) = fixture(true);
        let object = world.objects.get(idx).unwrap().full_id;
        assert!(BuildTool::contents_create_cap(&world, idx));
        tool.new_notecard(&world, idx);
        tool.new_script(&world, idx, false);
        tool.new_script(&world, idx, true);
        let items = created(&tool);
        assert_eq!(items.len(), 3, "no RezScript next to the capability");
        assert!(
            !tool
                .out
                .iter()
                .any(|c| matches!(c, aurora_net::NetCommand::Build(BuildCmd::RezScript { .. })))
        );
        let (_, note) = &items[0];
        assert_eq!((note.asset_type, note.inv_type, note.flags), (AT_NOTECARD, IT_NOTECARD, 0));
        assert_eq!(note.name, aurora_i18n::tr!("extra-new-notecard"));
        assert!(note.description.ends_with(" note card"));
        assert_eq!(
            (note.parent_id, note.owner_id, note.creator_id),
            (object, world.agent_id, world.agent_id)
        );
        assert_eq!((note.base_mask, note.owner_mask), (PERM_ALL, PERM_ALL));
        let (_, lsl) = &items[1];
        assert_eq!((lsl.asset_type, lsl.inv_type), (AT_LSL_TEXT, IT_LSL));
        assert!(!aurora_net::scripts::is_slua_item(lsl.flags));
        assert!(aurora_net::scripts::is_slua_item(items[2].1.flags));
        // the answers: success lists the contents again, a refusal says why
        tool.contents.objects.entry(object).or_default().requested = Some(Instant::now());
        tool.on_task_item_created(object, Ok(aurora_llsd::llsd_map! { "success" => true }));
        assert!(tool.contents.objects[&object].requested.is_none(), "listed again");
        tool.contents.objects.get_mut(&object).unwrap().requested = Some(Instant::now());
        tool.on_task_item_created(object, Ok(aurora_llsd::llsd_map! { "success" => false, "message" => "Denied" }));
        assert!(tool.contents.objects[&object].requested.is_some());
        assert!(tool.status.contains("Denied"));
        tool.on_task_item_created(object, Err("timeout".into()));
        assert!(tool.status.contains("timeout"));
    }

    #[test]
    fn without_the_capability_scripts_use_rez_script_and_notecards_are_not_made() {
        let (world, mut tool, idx) = fixture(false);
        assert!(!BuildTool::contents_create_cap(&world, idx));
        tool.new_notecard(&world, idx);
        assert!(tool.out.is_empty(), "New Notecard needs CreateTaskInventoryItem");
        tool.new_script(&world, idx, true);
        assert!(created(&tool).is_empty());
        assert!(tool.out.iter().any(|c| matches!(
            c,
            aurora_net::NetCommand::Build(BuildCmd::RezScript { enabled: true, item, .. })
                if aurora_net::scripts::is_slua_item(item.flags)
        )));
    }

    #[test]
    fn the_answer_of_a_creation_reaches_its_object() {
        let (mut world, mut tool, idx) = fixture(true);
        let object = world.objects.get(idx).unwrap().full_id;
        tool.new_notecard(&world, idx);
        let (tag, _) = created(&tool)[0];
        tool.contents.objects.entry(object).or_default().requested = Some(Instant::now());
        tool.on_cap_reply(&mut world, tag, Ok(aurora_llsd::llsd_map! { "success" => true }));
        assert!(tool.contents.objects[&object].requested.is_none());
        assert!(!tool.caps.contains_key(&tag));
    }
}
