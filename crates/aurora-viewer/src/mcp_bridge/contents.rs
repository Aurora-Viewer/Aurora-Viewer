//! An object's contents through MCP, every change read back from the
//! simulator: `object_contents` (every item with its four permission masks,
//! and the object's own properties), `task_item_permissions` (next owner),
//! `task_item_remove`, `task_item_add` (an agent-inventory item dropped in)
//! and `task_notecard_write` (a notecard from text).
//!
//! Each request first waits for a FRESH listing of the object's contents
//! (RequestTaskInventory, never a cached one: a stale cache is how a script
//! update once landed on the wrong item), acts, then asks for the listing
//! again until the change shows, or answers a timeout. Nothing answers
//! `ok: true` on a send alone.

use super::*;
use aurora_net::build::TaskItem;

/// How long a contents request may take, fetch and read-back together.
const DEADLINE: Duration = Duration::from_secs(15);
/// Between two listings asked while waiting for a change to show.
const RELIST_EVERY: Duration = Duration::from_millis(800);

/// An item of the object's contents, by id or exact name.
#[derive(Debug, Clone)]
pub(super) enum ItemRef {
    Id(Uuid),
    Name(String),
}

impl ItemRef {
    pub(super) fn from_args(item_id: Option<Uuid>, name: Option<String>) -> Result<Self, McpError> {
        match (item_id, name.filter(|n| !n.is_empty())) {
            (Some(id), _) => Ok(Self::Id(id)),
            (None, Some(n)) => Ok(Self::Name(n)),
            (None, None) => Err(McpError::InvalidParams("give `item_id` or the item's exact `name`".into())),
        }
    }

    fn find<'a>(&self, items: &'a [TaskItem]) -> Result<&'a TaskItem, McpError> {
        let items: Vec<&TaskItem> = items.iter().filter(|it| !it.is_folder).collect();
        match self {
            Self::Id(id) => items
                .into_iter()
                .find(|it| it.item_id == *id)
                .ok_or_else(|| McpError::InvalidParams(format!("no item {id} in this object"))),
            Self::Name(n) => {
                let found: Vec<&TaskItem> = items.into_iter().filter(|it| &it.name == n).collect();
                match found[..] {
                    [one] => Ok(one),
                    [] => Err(McpError::InvalidParams(format!("no item named '{n}' in this object"))),
                    _ => Err(McpError::InvalidParams(format!(
                        "{} items are named '{n}': give its item_id (object_contents lists them)",
                        found.len()
                    ))),
                }
            }
        }
    }
}

/// lsl_write_and_compile's arguments, carried out on a fresh listing.
#[derive(Debug, Clone)]
pub(super) struct ScriptWrite {
    pub script_item_id: Option<Uuid>,
    pub script_name: String,
    pub source_code: String,
    pub mono: bool,
    pub compile_target: Option<String>,
    pub running: bool,
    pub if_exists: Option<String>,
    pub experience_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) enum ContentsOp {
    List,
    /// The listing is fresh: compile now (handle_mcp_lsl_write answers).
    Script(Box<ScriptWrite>),
    NextOwner {
        item: ItemRef,
        copy: Option<bool>,
        modify: Option<bool>,
        transfer: Option<bool>,
    },
    Remove {
        item: ItemRef,
    },
    Add {
        inventory_item: Uuid,
        running: bool,
        replace: bool,
    },
    Notecard {
        name: String,
        body: String,
        replace: bool,
    },
    /// "Copy to Inventory": the whole contents into a new folder.
    CopyToInventory,
    /// The object's own next-owner permissions.
    ObjectNextOwner {
        copy: Option<bool>,
        modify: Option<bool>,
        transfer: Option<bool>,
    },
}

/// What has to show in a later listing for the change to count as done.
enum Check {
    NextOwner {
        item_id: Uuid,
        mask: u32,
    },
    /// The object's own next-owner permissions (copy / modify / transfer).
    ObjectNextOwner {
        mask: u32,
    },
    /// A script just saved, listed under its exact name.
    Named {
        item_id: Uuid,
        name: String,
    },
    Gone {
        item_id: Uuid,
    },
    /// An item of this name that was not there before.
    Appeared {
        name: String,
        before: HashSet<Uuid>,
    },
}

enum Stage {
    /// Copy to Inventory sent: waiting for the folder to hold every item.
    Copied {
        /// The id asked for; the server may answer another (reply.created), so
        /// the folder is also recognised by its name among new top-level ones.
        folder: Uuid,
        folder_name: String,
        before: HashSet<Uuid>,
        names: Vec<String>,
    },
    /// Waiting for a fresh listing before acting.
    Fetch { op: ContentsOp, asked: bool },
    /// Waiting for a listing that shows the change.
    Verify { check: Check, summary: Value, last_list: Instant },
    /// A notecard made in agent inventory first; waiting for its item, then
    /// it is dropped in (after the old one is removed, with `replace`).
    NotecardMade {
        name: String,
        before_inventory: HashSet<Uuid>,
        bytes: usize,
        replace: bool,
    },
    /// The old item of that name is being removed before the new one goes in.
    RemoveThenAdd {
        old: Uuid,
        inventory_item: Uuid,
        running: bool,
        last_list: Instant,
        note: Value,
    },
}

pub(super) struct PendingContents {
    object_uuid: Uuid,
    local_id: u32,
    started: Instant,
    stage: Stage,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

fn masks(m: u32) -> Value {
    json!({
        "copy": (m & perm::COPY) != 0,
        "modify": (m & perm::MODIFY) != 0,
        "transfer": (m & perm::TRANSFER) != 0,
        "move": (m & perm::MOVE) != 0,
        "raw": m,
    })
}

fn type_name(asset_type: i8, inv_type: i8) -> &'static str {
    match (asset_type, inv_type) {
        (10, _) | (_, 10) => "script",
        (7, _) => "notecard",
        (6, _) | (_, 6) => "object",
        (20, _) => "animation",
        (21, _) => "gesture",
        (0, _) => "texture",
        (1, _) => "sound",
        (3, _) => "landmark",
        (13, _) | (5, _) => "wearable",
        (49, _) => "mesh",
        (56, _) => "settings",
        (57, _) => "material",
        _ => "other",
    }
}

impl App {
    /// Queue a contents request; `poll_mcp_contents` carries it out.
    pub(super) fn start_mcp_contents(
        &mut self,
        object_local_id: u32,
        op: ContentsOp,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let (idx, object_uuid) = match self.resolve_prim_by_local_id(object_local_id) {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_tx.send(Err(e));
                return;
            }
        };
        let handle = self.world.objects.get(idx).map(|o| o.key.region).unwrap_or_default();
        // the object's own properties (owner, group, masks) come with it
        self.send(NetCommand::RequestObjectProperties {
            handle,
            object: object_uuid,
        });
        self.mcp_bridge.pending_contents.push_back(PendingContents {
            object_uuid,
            local_id: object_local_id,
            started: Instant::now(),
            stage: Stage::Fetch { op, asked: false },
            reply_tx,
        });
    }

    fn relist(&mut self, object_uuid: Uuid) {
        if let Some(idx) = self.world.objects.index_of_uuid(&object_uuid) {
            self.build.request_contents(&self.world, idx, true);
            self.flush_build();
            if let Some(handle) = self.world.objects.get(idx).map(|o| o.key.region) {
                self.send(NetCommand::RequestObjectProperties {
                    handle,
                    object: object_uuid,
                });
            }
        }
    }

    /// A compiled script's answer, held until a fresh listing shows it under
    /// its exact name (the next tool may look for it by name at once).
    pub(super) fn verify_script_listed(
        &mut self,
        object_uuid: Uuid,
        local_id: u32,
        item_id: Uuid,
        name: String,
        summary: Value,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        self.relist(object_uuid);
        self.mcp_bridge.pending_contents.push_back(PendingContents {
            object_uuid,
            local_id,
            started: Instant::now(),
            stage: Stage::Verify {
                check: Check::Named { item_id, name },
                summary,
                last_list: Instant::now(),
            },
            reply_tx,
        });
    }

    fn listing(&self, object_uuid: Uuid) -> Option<&[TaskItem]> {
        self.build
            .contents
            .objects
            .get(&object_uuid)
            .filter(|inv| !inv.loading)
            .map(|inv| inv.items.as_slice())
    }

    /// Every frame: move each contents request on.
    pub(super) fn poll_mcp_contents(&mut self) {
        let now = Instant::now();
        let pending = std::mem::take(&mut self.mcp_bridge.pending_contents);
        for mut p in pending {
            // a box's whole contents may take a while (the window allows 60 s)
            let deadline = if matches!(p.stage, Stage::Copied { .. }) {
                Duration::from_secs(60)
            } else {
                DEADLINE
            };
            if now.duration_since(p.started) > deadline {
                let what = match &p.stage {
                    Stage::Fetch { .. } => "the object's contents never arrived",
                    Stage::Copied { .. } => "the folder was made but not every item arrived in it",
                    Stage::Verify { .. } => "the change was sent but never showed in the object's contents",
                    Stage::NotecardMade { .. } => "the notecard was made but never appeared in inventory",
                    Stage::RemoveThenAdd { .. } => "the old item was never removed",
                };
                let _ = p
                    .reply_tx
                    .send(Err(McpError::Internal(format!("timed out: {what} (object {})", p.local_id))));
                continue;
            }
            match self.step_mcp_contents(&mut p, now) {
                Some(answer) => {
                    let _ = p.reply_tx.send(answer);
                }
                None => self.mcp_bridge.pending_contents.push_back(p),
            }
        }
    }

    /// One step of a request; Some(answer) when it is finished.
    fn step_mcp_contents(&mut self, p: &mut PendingContents, now: Instant) -> Option<Result<ViewerResponse, McpError>> {
        match &mut p.stage {
            Stage::Fetch { op, asked } => {
                if !*asked {
                    *asked = true;
                    // a fresh listing: force, whatever the cache holds
                    if let Some(inv) = self.build.contents.objects.get_mut(&p.object_uuid) {
                        inv.loading = false;
                        inv.requested = None;
                    }
                    self.relist(p.object_uuid);
                    return None;
                }
                let items = self.listing(p.object_uuid)?.to_vec();
                let op = op.clone();
                self.act(p, op, items, now)
            }
            Stage::Verify { check, summary, last_list } => {
                if let Some(items) = self.listing(p.object_uuid) {
                    let done = match check {
                        Check::NextOwner { item_id, mask } => items
                            .iter()
                            .find(|it| it.item_id == *item_id)
                            .map(|it| (it.next_owner_mask & (perm::COPY | perm::MODIFY | perm::TRANSFER)) == *mask),
                        Check::Gone { item_id } => Some(!items.iter().any(|it| it.item_id == *item_id)),
                        Check::ObjectNextOwner { mask } => self
                            .build
                            .props
                            .get(&p.object_uuid)
                            .map(|pr| (pr.next_owner_mask & (perm::COPY | perm::MODIFY | perm::TRANSFER)) == *mask),
                        // listed under its name AND the simulator has said whether it runs
                        Check::Named { item_id, name } => Some(
                            items.iter().any(|it| it.item_id == *item_id && &it.name == name)
                                && self.build.contents.running.contains_key(&(p.object_uuid, *item_id)),
                        ),
                        Check::Appeared { name, before } => Some(
                            items
                                .iter()
                                .any(|it| !it.is_folder && &it.name == name && !before.contains(&it.item_id)),
                        ),
                    };
                    if done == Some(true) {
                        let mut out = summary.clone();
                        if let Check::Appeared { name, before } = check
                            && let Some(new) = items.iter().find(|it| &it.name == name && !before.contains(&it.item_id))
                        {
                            out["item_id"] = json!(new.item_id.to_string());
                            out["type"] = json!(type_name(new.asset_type, new.inv_type));
                        }
                        if let Check::Named { item_id, .. } = check
                            && let Some((running, target)) = self.build.contents.running.get(&(p.object_uuid, *item_id)).copied()
                        {
                            // what the simulator says it is, not what was asked for
                            out["running"] = json!(running);
                            out["mono"] = json!(matches!(target, Some(aurora_net::scripts::CompileTarget::Mono)));
                            out["compile_target"] = json!(target.map(|t| match t {
                                aurora_net::scripts::CompileTarget::Lsl2 => "lsl2",
                                aurora_net::scripts::CompileTarget::Mono => "mono",
                                aurora_net::scripts::CompileTarget::Luau => "luau",
                                aurora_net::scripts::CompileTarget::LslLuau => "lsl_luau",
                            }));
                        }
                        out["ok"] = json!(true);
                        out["verified"] = json!(true);
                        return Some(Ok(ViewerResponse::Json(out)));
                    }
                }
                if now.duration_since(*last_list) >= RELIST_EVERY {
                    *last_list = now;
                    self.relist(p.object_uuid);
                }
                None
            }
            Stage::Copied {
                folder,
                folder_name,
                before,
                names,
            } => {
                let inv = &self.world.inventory;
                // the folder asked for, or a new top-level one of its name
                let candidates: Vec<Uuid> = inv
                    .folders
                    .values()
                    .filter(|f| {
                        f.info.id == *folder
                            || (!f.library && f.info.parent == inv.root && &f.info.name == folder_name && !before.contains(&f.info.id))
                    })
                    .map(|f| f.info.id)
                    .collect();
                let (found, arrived) = candidates.into_iter().find_map(|fid| {
                    let arrived: Vec<&aurora_net::inventory::InvItem> = inv.items.values().filter(|it| it.parent == fid).collect();
                    names
                        .iter()
                        .all(|n| arrived.iter().any(|it| &it.name == n))
                        .then_some((fid, arrived))
                })?;
                let folder = &found;
                let items: Vec<Value> = arrived
                    .iter()
                    .map(|it| json!({ "item_id": it.id.to_string(), "name": it.name, "asset_type": it.asset_type }))
                    .collect();
                Some(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "verified": true,
                    "folder_id": folder.to_string(),
                    "folder_path": self.inventory_folder_path(*folder),
                    "item_count": items.len(),
                    "items": items,
                }))))
            }
            Stage::NotecardMade {
                name,
                before_inventory,
                bytes,
                replace,
            } => {
                let made = self
                    .world
                    .inventory
                    .items
                    .values()
                    .find(|it| it.asset_type == 7 && &it.name == name && !before_inventory.contains(&it.id) && !it.asset_id.is_nil())
                    .map(|it| it.id)?;
                let (name, bytes, replace) = (name.clone(), *bytes, *replace);
                let items = self.listing(p.object_uuid).map(<[TaskItem]>::to_vec).unwrap_or_default();
                let note = json!({ "name": name, "bytes": bytes, "inventory_copy": made.to_string() });
                self.add_item(p, made, false, replace, items, note, now)
            }
            Stage::RemoveThenAdd {
                old,
                inventory_item,
                running,
                last_list,
                note,
            } => {
                if let Some(items) = self.listing(p.object_uuid)
                    && !items.iter().any(|it| it.item_id == *old)
                {
                    let (inv_item, running, note) = (*inventory_item, *running, note.clone());
                    let items = items.to_vec();
                    return self.add_item(p, inv_item, running, false, items, note, now);
                }
                if now.duration_since(*last_list) >= RELIST_EVERY {
                    *last_list = now;
                    self.relist(p.object_uuid);
                }
                None
            }
        }
    }

    fn act(
        &mut self,
        p: &mut PendingContents,
        op: ContentsOp,
        items: Vec<TaskItem>,
        now: Instant,
    ) -> Option<Result<ViewerResponse, McpError>> {
        let Some(idx) = self.world.objects.index_of_uuid(&p.object_uuid) else {
            return Some(Err(McpError::InvalidParams(format!("object {} is gone", p.local_id))));
        };
        match op {
            ContentsOp::List => Some(Ok(self.contents_json(p.local_id, p.object_uuid, &items))),
            ContentsOp::Script(w) => {
                // hand the answer over to the compile, which replies when the
                // simulator has saved (and compiled) the script
                let (spare, _) = oneshot::channel();
                let reply_tx = std::mem::replace(&mut p.reply_tx, spare);
                let w = *w;
                self.handle_mcp_lsl_write(
                    p.local_id,
                    w.script_item_id,
                    w.script_name,
                    w.source_code,
                    w.mono,
                    w.compile_target,
                    w.running,
                    w.if_exists,
                    w.experience_id,
                    reply_tx,
                );
                // finished here; the spare's answer goes nowhere
                Some(Ok(ViewerResponse::Json(json!({}))))
            }
            ContentsOp::NextOwner {
                item,
                copy,
                modify,
                transfer,
            } => {
                let it = match item.find(&items) {
                    Ok(it) => it.clone(),
                    Err(e) => return Some(Err(e)),
                };
                if it.owner_id != self.world.agent_id {
                    return Some(Err(McpError::PermissionDenied(format!("'{}' is not yours", it.name))));
                }
                if (it.owner_mask & perm::MODIFY) == 0 {
                    // Second Life's rule (the Legacy Viewer greys the boxes out too)
                    return Some(Err(McpError::PermissionDenied(format!(
                        "'{}' is no-modify for you (owner: copy {}, transfer {}), and Second Life only lets you set next-owner permissions on what you can modify. Set them on the original before it is copied (object_set_permissions on the object, or task_item_permissions where you can modify it).",
                        it.name,
                        (it.owner_mask & perm::COPY) != 0,
                        (it.owner_mask & perm::TRANSFER) != 0
                    ))));
                }
                let set = |m: u32, bit: u32, on: Option<bool>| match on {
                    Some(true) => m | bit,
                    Some(false) => m & !bit,
                    None => m,
                };
                let mut next = it.next_owner_mask;
                next = set(next, perm::COPY, copy);
                next = set(next, perm::MODIFY, modify);
                next = set(next, perm::TRANSFER, transfer);
                // never more than the owner may give
                next &= it.base_mask | perm::MOVE;
                let mut updated = it.clone();
                updated.next_owner_mask = next;
                let (handle, local_id) = {
                    let o = self.world.objects.get(idx)?;
                    (o.key.region, o.key.local_id)
                };
                self.send(NetCommand::Build(BuildCmd::UpdateTaskInventory {
                    handle,
                    local_id,
                    item: Box::new(updated),
                }));
                self.flush_build();
                self.relist(p.object_uuid);
                p.stage = Stage::Verify {
                    check: Check::NextOwner {
                        item_id: it.item_id,
                        mask: next & (perm::COPY | perm::MODIFY | perm::TRANSFER),
                    },
                    summary: json!({
                        "item_id": it.item_id.to_string(),
                        "name": it.name,
                        "next_owner": masks(next),
                    }),
                    last_list: now,
                };
                None
            }
            ContentsOp::ObjectNextOwner { copy, modify, transfer } => {
                let Some(pr) = self.build.props.get(&p.object_uuid).cloned() else {
                    return None; // the properties are on their way
                };
                if pr.owner_id != self.world.agent_id {
                    return Some(Err(McpError::PermissionDenied("this object is not yours".into())));
                }
                if (pr.owner_mask & perm::MODIFY) == 0 {
                    return Some(Err(McpError::PermissionDenied(
                        "this object is no-modify for you, and Second Life only lets you set next-owner permissions on what you can modify"
                            .into(),
                    )));
                }
                let (handle, local_id) = {
                    let o = self.world.objects.get(idx)?;
                    (o.key.region, o.key.local_id)
                };
                let mut next = pr.next_owner_mask;
                for (bit, on) in [(perm::COPY, copy), (perm::MODIFY, modify), (perm::TRANSFER, transfer)] {
                    if let Some(on) = on {
                        self.send(NetCommand::Build(BuildCmd::SetPermissions {
                            handle,
                            local_ids: vec![local_id],
                            field: aurora_net::build::perm_field::NEXT_OWNER,
                            set: on,
                            mask: bit,
                        }));
                        if on {
                            next |= bit;
                        } else {
                            next &= !bit;
                        }
                    }
                }
                next &= pr.base_mask | perm::MOVE;
                self.flush_build();
                self.relist(p.object_uuid);
                p.stage = Stage::Verify {
                    check: Check::ObjectNextOwner {
                        mask: next & (perm::COPY | perm::MODIFY | perm::TRANSFER),
                    },
                    summary: json!({ "object": pr.name, "next_owner": masks(next) }),
                    last_list: now,
                };
                None
            }
            ContentsOp::CopyToInventory => {
                let items: Vec<TaskItem> = items.into_iter().filter(|it| !it.is_folder).collect();
                if items.is_empty() {
                    return Some(Err(McpError::InvalidParams("the object's contents are empty".into())));
                }
                let name = self
                    .build
                    .props
                    .get(&p.object_uuid)
                    .map(|pr| pr.name.clone())
                    .filter(|n| !n.is_empty())?; // its name is on its way
                let names: Vec<String> = items.iter().map(|it| it.name.clone()).collect();
                let folder_name = crate::build::task_inventory::contents_folder_name(&name);
                let inv = &self.world.inventory;
                let before: HashSet<Uuid> = inv
                    .folders
                    .values()
                    .filter(|f| f.info.parent == inv.root && f.info.name == folder_name)
                    .map(|f| f.info.id)
                    .collect();
                match self.mcp_copy_contents(p.object_uuid, name, items) {
                    Ok(folder) => {
                        p.stage = Stage::Copied {
                            folder,
                            folder_name,
                            before,
                            names,
                        };
                        None
                    }
                    Err(reason) => Some(Err(McpError::PermissionDenied(reason))),
                }
            }
            ContentsOp::Remove { item } => {
                let it = match item.find(&items) {
                    Ok(it) => it.clone(),
                    Err(e) => return Some(Err(e)),
                };
                self.build.remove_item(&self.world, idx, it.item_id);
                self.flush_build();
                self.relist(p.object_uuid);
                p.stage = Stage::Verify {
                    check: Check::Gone { item_id: it.item_id },
                    summary: json!({ "removed": it.name, "item_id": it.item_id.to_string() }),
                    last_list: now,
                };
                None
            }
            ContentsOp::Add {
                inventory_item,
                running,
                replace,
            } => self.add_item(p, inventory_item, running, replace, items, json!({}), now),
            ContentsOp::Notecard { name, body, replace } => {
                if !replace && items.iter().any(|it| it.name == name) {
                    return Some(Err(McpError::InvalidParams(format!(
                        "'{name}' is already in this object: pass replace: true to replace it"
                    ))));
                }
                let encoded = match aurora_net::inventory::notecard::Notecard::default().encode(&body) {
                    Ok(e) => e,
                    Err(e) => return Some(Err(McpError::InvalidParams(e))),
                };
                let parent = self
                    .world
                    .inventory
                    .folders
                    .values()
                    .find(|f| !f.library && f.info.type_default == 7)
                    .map_or(self.world.inventory.root, |f| f.info.id);
                let before_inventory: HashSet<Uuid> = self
                    .world
                    .inventory
                    .items
                    .values()
                    .filter(|it| it.name == name)
                    .map(|it| it.id)
                    .collect();
                self.send(NetCommand::CreateNotecardItem {
                    request: Uuid::new_v4(),
                    parent,
                    name: name.clone(),
                    data: encoded.clone(),
                    permissions: None,
                });
                p.stage = Stage::NotecardMade {
                    name,
                    before_inventory,
                    bytes: encoded.len(),
                    replace,
                };
                None
            }
        }
    }

    /// Drop an agent-inventory item into the object; with `replace`, the item
    /// of the same name already there is removed first (else the simulator
    /// would add "<name> 1" beside it).
    #[allow(clippy::too_many_arguments)]
    fn add_item(
        &mut self,
        p: &mut PendingContents,
        inventory_item: Uuid,
        running: bool,
        replace: bool,
        items: Vec<TaskItem>,
        note: Value,
        now: Instant,
    ) -> Option<Result<ViewerResponse, McpError>> {
        let Some(item) = self.world.inventory.items.get(&inventory_item).cloned() else {
            return Some(Err(McpError::InvalidParams(format!(
                "no item {inventory_item} in your inventory (inventory_search finds it)"
            ))));
        };
        let Some(idx) = self.world.objects.index_of_uuid(&p.object_uuid) else {
            return Some(Err(McpError::InvalidParams(format!("object {} is gone", p.local_id))));
        };
        if let Some(old) = items.iter().find(|it| !it.is_folder && it.name == item.name) {
            if !replace {
                return Some(Err(McpError::InvalidParams(format!(
                    "'{}' is already in this object: pass replace: true to replace it",
                    item.name
                ))));
            }
            let old = old.item_id;
            self.build.remove_item(&self.world, idx, old);
            self.flush_build();
            self.relist(p.object_uuid);
            let mut note = note;
            note["replaced"] = json!(old.to_string());
            p.stage = Stage::RemoveThenAdd {
                old,
                inventory_item,
                running,
                last_list: now,
                note,
            };
            return None;
        }
        let Some(drop) = crate::build::contents_drop::item_drop(&self.world, idx, &item, running) else {
            return Some(Err(McpError::InvalidParams(format!("'{}' cannot go into this object", item.name))));
        };
        let before: HashSet<Uuid> = items.iter().map(|it| it.item_id).collect();
        self.build.drop_inventory(&mut self.world, idx, vec![drop]);
        self.flush_build();
        self.relist(p.object_uuid);
        let mut summary = note;
        summary["name"] = json!(item.name);
        summary["from_inventory_item"] = json!(inventory_item.to_string());
        p.stage = Stage::Verify {
            check: Check::Appeared { name: item.name, before },
            summary,
            last_list: now,
        };
        None
    }

    fn contents_json(&mut self, local_id: u32, object_uuid: Uuid, items: &[TaskItem]) -> ViewerResponse {
        let handle = self
            .world
            .objects
            .index_of_uuid(&object_uuid)
            .and_then(|i| self.world.objects.get(i))
            .map(|o| o.key.region);
        let list: Vec<Value> = items
            .iter()
            .filter(|it| !it.is_folder)
            .map(|it| {
                let kind = type_name(it.asset_type, it.inv_type);
                let mut v = json!({
                    "item_id": it.item_id.to_string(),
                    "name": it.name,
                    "type": kind,
                    "description": it.description,
                    // nil when the listing does not say (shown as null, never as a creator)
                    "creator_id": (!it.creator_id.is_nil()).then(|| it.creator_id.to_string()),
                    "owner_id": it.owner_id.to_string(),
                    "base": masks(it.base_mask),
                    "owner": masks(it.owner_mask),
                    "group": masks(it.group_mask),
                    "everyone": masks(it.everyone_mask),
                    "next_owner": masks(it.next_owner_mask),
                });
                if kind == "script" {
                    // the simulator's own answer (GetScriptRunning), or null
                    // while it has not come: never a guess
                    match self.build.contents.running.get(&(object_uuid, it.item_id)) {
                        Some((running, _)) => v["running"] = json!(running),
                        None => v["running"] = Value::Null,
                    }
                }
                v
            })
            .collect();
        let unknown_running = list.iter().filter(|v| v.get("running").is_some_and(Value::is_null)).count();
        if unknown_running > 0
            && let Some(handle) = handle
        {
            // ask about the ones not answered yet (the listing asks for 64 at most)
            let missing: Vec<Uuid> = items
                .iter()
                .filter(|it| it.asset_type == 10 && !self.build.contents.running.contains_key(&(object_uuid, it.item_id)))
                .map(|it| it.item_id)
                .collect();
            for item_id in missing {
                self.send(NetCommand::Build(BuildCmd::GetScriptRunning {
                    handle,
                    object_id: object_uuid,
                    item_id,
                }));
            }
        }
        let props = self.build.props.get(&object_uuid);
        ViewerResponse::Json(json!({
            "ok": true,
            "object_local_id": local_id,
            "object_id": object_uuid.to_string(),
            "object": props.map(|p| json!({
                "name": p.name,
                "description": p.description,
                "owner_id": p.owner_id.to_string(),
                // the family properties carry no creator: null rather than a nil key
                "creator_id": (p.full && !p.creator_id.is_nil()).then(|| p.creator_id.to_string()),
                "group_id": p.group_id.to_string(),
                "last_owner_id": p.last_owner_id.to_string(),
                "base": masks(p.base_mask),
                "owner": masks(p.owner_mask),
                "group": masks(p.group_mask),
                "everyone": masks(p.everyone_mask),
                "next_owner": masks(p.next_owner_mask),
                "inventory_serial": p.inventory_serial,
            })),
            "item_count": list.len(),
            "items": list,
            "running_unknown": unknown_running,
            "note": if unknown_running > 0 {
                "running is null where the simulator has not answered yet: call again in a moment"
            } else {
                ""
            },
        }))
    }
}
