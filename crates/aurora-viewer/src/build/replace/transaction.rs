//! Acknowledged stages: UDP reliability only confirms delivery, not that
//! the simulator applied an edit. Advance from observed objects, inventory
//! listings and ScriptRunningReply; time out into an explicit recovery.

use super::{Draft, Options, Prim, task_from_item};
use crate::build::{BuildTool, family, root_of};
use crate::world::World;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetCommand;
use aurora_net::build::{BuildCmd, TaskItem, perm};
use aurora_net::inventory::{
    InvItem,
    operations::{Mutation, Operation, Reply},
};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Each ObjectLink block count fits one byte. New children are inserted
/// before existing children, so submit the tail chunk first, root first
/// in every message, and confirm it before submitting the preceding chunk.
pub(super) fn link_batches(expected: &[Uuid]) -> Vec<Vec<Uuid>> {
    let Some(root) = expected.first() else { return Vec::new() };
    expected[1..]
        .chunks(254)
        .rev()
        .map(|chunk| std::iter::once(*root).chain(chunk.iter().copied()).collect())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Read,
    ReadScripts,
    StopScripts,
    Folder,
    CopyOut(usize),
    CopyIn(usize),
    Delink,
    Transform,
    Link,
    Verify,
    Physics,
    Retire,
    RestoreScripts,
    Cleanup,
    RecoverStop,
    RecoverDelink,
    RecoverTransform,
    RecoverLink,
    RecoverVerify,
    RecoverPhysics,
    RecoverContents,
    Done,
    Failed,
}

impl Stage {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Read => aurora_i18n::tr!("replace-stage-read"),
            Self::ReadScripts => aurora_i18n::tr!("replace-stage-read-scripts"),
            Self::StopScripts => aurora_i18n::tr!("replace-stage-stop"),
            Self::Folder => aurora_i18n::tr!("replace-stage-folder"),
            Self::CopyOut(_) => aurora_i18n::tr!("replace-stage-copy-out"),
            Self::CopyIn(_) => aurora_i18n::tr!("replace-stage-copy-in"),
            Self::Delink => aurora_i18n::tr!("replace-stage-unlink"),
            Self::Transform => aurora_i18n::tr!("replace-stage-transform"),
            Self::Link => aurora_i18n::tr!("replace-stage-link"),
            Self::Verify | Self::Physics | Self::Retire => aurora_i18n::tr!("replace-stage-verify"),
            Self::RestoreScripts => aurora_i18n::tr!("replace-stage-restore"),
            Self::Cleanup => aurora_i18n::tr!("replace-stage-cleanup"),
            Self::RecoverStop
            | Self::RecoverDelink
            | Self::RecoverTransform
            | Self::RecoverLink
            | Self::RecoverVerify
            | Self::RecoverPhysics
            | Self::RecoverContents => aurora_i18n::tr!("replace-stage-recover"),
            Self::Done => aurora_i18n::tr!("replace-stage-done"),
            Self::Failed => aurora_i18n::tr!("replace-stage-failed"),
        }
    }

    pub fn progress(self) -> f32 {
        match self {
            Self::Read | Self::ReadScripts => 0.05,
            Self::StopScripts => 0.12,
            Self::Folder | Self::CopyOut(_) | Self::CopyIn(_) => 0.25,
            Self::Delink => 0.45,
            Self::Transform => 0.55,
            Self::Link => 0.65,
            Self::Verify | Self::Physics | Self::Retire => 0.8,
            Self::RestoreScripts => 0.9,
            Self::Cleanup => 0.96,
            Self::Done => 1.0,
            _ => 0.0,
        }
    }
}

#[derive(Debug)]
struct Script {
    object: Uuid,
    item: Uuid,
    running: bool,
    /// Copied scripts are rezzed stopped; originals are touched only when
    /// the pause toggle is enabled. Never reset or start a previously off script.
    touched: bool,
    copied: bool,
}

#[derive(Debug)]
pub struct Job {
    pub stage: Stage,
    pub number: usize,
    pub target: Uuid,
    pub source: Uuid,
    pub message: String,
    pub inventory_request: Option<Uuid>,
    pub inventory_result: Option<Result<Reply, String>>,
    pub inventory_error: Option<(Uuid, String)>,
    pub physics_request: Option<u64>,
    members: Vec<Prim>,
    replacement: Prim,
    planned: Prim,
    retired: Prim,
    options: Options,
    entered: Instant,
    sent: bool,
    refresh_serials: Vec<(Uuid, u64)>,
    link_batch: usize,
    poll_at: Instant,
    mutated: bool,
    recovering: bool,
    failure: Option<String>,
    scripts: Vec<Script>,
    items: Vec<TaskItem>,
    staged: Vec<InvItem>,
    copied: Vec<Uuid>,
    destination_before: HashSet<Uuid>,
    folder: Option<Uuid>,
    requested_folder: Uuid,
}

impl Job {
    pub fn inventory_ids(&self) -> Vec<Uuid> {
        self.all().map(|p| p.id).collect()
    }

    pub fn progress(&self) -> f32 {
        match self.stage {
            Stage::Folder => 0.2,
            Stage::CopyOut(i) => 0.2 + 0.2 * (2 * i) as f32 / (2 * self.items.len().max(1)) as f32,
            Stage::CopyIn(i) => 0.2 + 0.2 * (2 * i + 1) as f32 / (2 * self.items.len().max(1)) as f32,
            stage => stage.progress(),
        }
    }

    pub fn progress_text(&self) -> String {
        match self.stage {
            Stage::CopyOut(i) | Stage::CopyIn(i) => aurora_i18n::tr!(
                "replace-copy-progress",
                stage = self.stage.label(),
                current = (i + 1).to_string(),
                total = self.items.len().to_string()
            ),
            stage => stage.label().into(),
        }
    }
    pub fn new(tool: &BuildTool, world: &World, draft: &Draft, options: Options) -> Result<Self, String> {
        let target = world
            .objects
            .index_of_uuid(&draft.target)
            .ok_or_else(|| aurora_i18n::tr!("replace-missing").to_owned())?;
        let source = draft
            .source
            .and_then(|id| world.objects.index_of_uuid(&id))
            .ok_or_else(|| aurora_i18n::tr!("replace-pick-first").to_owned())?;
        let root = root_of(world, target);
        let family = family(world, root);
        // Every link batch fits the protocol's 8-bit block count; the next
        // one is sent only after the previous parent changes are observed.
        if family.len() > 256 {
            return Err(aurora_i18n::tr!("replace-too-many").into());
        }
        if family.len() < 2 || family.get(draft.number.saturating_sub(1)) != Some(&target) {
            return Err(aurora_i18n::tr!("replace-changed").into());
        }
        let so = world
            .objects
            .get(source)
            .ok_or_else(|| aurora_i18n::tr!("replace-missing").to_owned())?;
        if so.parent_id != 0 || !world.objects.children_of(&so.key).is_empty() || so.full_id == draft.target {
            return Err(aurora_i18n::tr!("replace-independent").into());
        }
        let mut members = Vec::new();
        for idx in family.into_iter().chain(std::iter::once(source)) {
            let o = world
                .objects
                .get(idx)
                .ok_or_else(|| aurora_i18n::tr!("replace-missing").to_owned())?;
            if o.is_avatar()
                || o.pcode != aurora_prim::params::LL_PCODE_VOLUME
                || !tool.replacement_can_modify(world, idx)
                || o.key.region != so.key.region
            {
                return Err(aurora_i18n::tr!("replace-modify").into());
            }
            if o.update_flags & (crate::build::edits::FLAGS_USE_PHYSICS | crate::build::edits::FLAGS_TEMPORARY_ON_REZ) != 0 {
                return Err(aurora_i18n::tr!("replace-static").into());
            }
            members.push(Prim::capture(tool, world, idx).ok_or_else(|| aurora_i18n::tr!("replace-world-only").to_owned())?);
        }
        let replacement = members.pop().ok_or_else(|| aurora_i18n::tr!("replace-missing").to_owned())?;
        let old = members
            .get(draft.number - 1)
            .ok_or_else(|| aurora_i18n::tr!("replace-changed").to_owned())?;
        let mut planned = replacement.clone();
        if options.position {
            planned.position = old.position;
        }
        if options.rotation {
            planned.rotation = old.rotation;
        }
        if options.size {
            planned.size = old.size;
        }
        let mut retired = old.clone();
        // Keep the old prim at the replacement's original world position.
        retired.position = replacement.position;
        let now = Instant::now();
        Ok(Self {
            stage: Stage::Read,
            number: draft.number,
            target: draft.target,
            source: replacement.id,
            message: String::new(),
            inventory_request: None,
            inventory_result: None,
            inventory_error: None,
            physics_request: None,
            members,
            replacement,
            planned,
            retired,
            options,
            entered: now,
            sent: false,
            refresh_serials: Vec::new(),
            link_batch: 0,
            poll_at: now,
            mutated: false,
            recovering: false,
            failure: None,
            scripts: Vec::new(),
            items: Vec::new(),
            staged: Vec::new(),
            copied: Vec::new(),
            destination_before: HashSet::new(),
            folder: None,
            requested_folder: Uuid::new_v4(),
        })
    }

    fn all(&self) -> impl Iterator<Item = &Prim> {
        self.members.iter().chain(std::iter::once(&self.replacement))
    }
    fn handle(&self) -> u64 {
        self.replacement.key.region
    }

    fn advance(&mut self, stage: Stage, now: Instant) {
        self.stage = stage;
        self.entered = now;
        self.sent = false;
        if matches!(stage, Stage::Link | Stage::RecoverLink) {
            self.link_batch = 0;
        }
    }

    fn expected(&self, recovery: bool) -> Vec<Uuid> {
        self.members
            .iter()
            .map(|p| if !recovery && p.id == self.target { self.source } else { p.id })
            .collect()
    }

    fn topology(&self, world: &World, recovery: bool) -> bool {
        let expected = self.expected(recovery);
        let Some(root) = world.objects.index_of_uuid(&expected[0]) else {
            return false;
        };
        let Some(o) = world.objects.get(root) else { return false };
        o.parent_id == 0
            && family(world, root)
                .iter()
                .filter_map(|i| world.objects.get(*i).map(|o| o.full_id))
                .eq(expected.iter().copied())
    }

    fn standalone(&self, world: &World) -> bool {
        self.all().all(|p| {
            world
                .objects
                .index_of_uuid(&p.id)
                .and_then(|i| world.objects.get(i))
                .is_some_and(|o| o.parent_id == 0)
        })
    }

    fn list_objects(&self) -> Vec<Uuid> {
        // Even with pausing disabled, remember states for a possible rollback.
        self.all().map(|p| p.id).collect()
    }

    fn request_list(tool: &mut BuildTool, world: &World, id: Uuid) {
        if let Some(idx) = world.objects.index_of_uuid(&id) {
            tool.request_contents(world, idx, true);
        }
    }

    fn query_scripts(&self, tool: &mut BuildTool, stop: bool, restore: bool) {
        for script in &self.scripts {
            if (stop && !script.running) || (restore && (!script.touched || (self.recovering && script.copied))) {
                continue;
            }
            tool.contents.running.remove(&(script.object, script.item));
            if stop || restore {
                tool.send(BuildCmd::SetScriptRunning {
                    handle: self.handle(),
                    object_id: script.object,
                    item_id: script.item,
                    running: restore && script.running,
                });
            }
            tool.send(BuildCmd::GetScriptRunning {
                handle: self.handle(),
                object_id: script.object,
                item_id: script.item,
            });
        }
    }

    fn scripts_confirmed(&self, tool: &BuildTool, restore: bool) -> bool {
        self.scripts
            .iter()
            .filter(|s| {
                if restore {
                    s.touched && !(self.recovering && s.copied)
                } else {
                    s.running
                }
            })
            .all(|s| {
                tool.contents
                    .running
                    .get(&(s.object, s.item))
                    .is_some_and(|state| state.0 == (restore && s.running))
            })
    }

    /// UDP delivery is not a script-engine acknowledgement. Re-read only
    /// missing or mismatching states; never repeat the state-changing command.
    fn poll_script_states(&mut self, tool: &mut BuildTool, now: Instant, restore: Option<bool>) {
        if now.saturating_duration_since(self.poll_at) < Duration::from_secs(1) {
            return;
        }
        self.poll_at = now;
        for script in &self.scripts {
            let state = tool.contents.running.get(&(script.object, script.item));
            let pending = match restore {
                None => state.is_none(),
                Some(true) => script.touched && !(self.recovering && script.copied) && state.is_none_or(|s| s.0 != script.running),
                Some(false) => script.running && state.is_none_or(|s| s.0),
            };
            if pending {
                tool.send(BuildCmd::GetScriptRunning {
                    handle: self.handle(),
                    object_id: script.object,
                    item_id: script.item,
                });
            }
        }
    }

    fn mutate_inventory(&mut self, tool: &mut BuildTool, change: Mutation) {
        let request = Uuid::new_v4();
        self.inventory_request = Some(request);
        self.inventory_result = None;
        tool.out.push(NetCommand::EditInventory { request, change });
    }

    fn physics(&self, tool: &mut BuildTool, world: &World, standalone: bool, recovery: bool) {
        for p in self.all() {
            let Some(mut params) = p.physics else { continue };
            if standalone && params.shape_type == 1 {
                params.shape_type = 2;
            }
            if !standalone && !recovery && p.id == self.target && params.shape_type == 1 {
                params.shape_type = 2;
            }
            let phantom = p.flags & crate::build::edits::FLAGS_PHANTOM != 0;
            if tool
                .physics
                .get(&p.key.local_id)
                .is_some_and(|actual| actual.shape_type == params.shape_type)
            {
                let flags = crate::build::edits::FLAGS_USE_PHYSICS
                    | crate::build::edits::FLAGS_TEMPORARY_ON_REZ
                    | crate::build::edits::FLAGS_PHANTOM;
                if world
                    .objects
                    .index_of_uuid(&p.id)
                    .and_then(|i| world.objects.get(i))
                    .is_some_and(|o| o.update_flags & flags != p.flags & crate::build::edits::FLAGS_PHANTOM)
                {
                    tool.send(BuildCmd::SetFlags {
                        handle: p.key.region,
                        local_id: p.key.local_id,
                        physics: false,
                        temporary: false,
                        phantom,
                    });
                }
                continue;
            }
            tool.send(BuildCmd::SetPhysicsParams {
                handle: p.key.region,
                local_id: p.key.local_id,
                physics: false,
                temporary: false,
                phantom,
                params,
            });
            // Track the requested shape for restoring it after relinking.
            // The final fresh capability reply still confirms server state.
            tool.physics.insert(p.key.local_id, params);
        }
    }

    fn content_stage(&self) -> Stage {
        if self.items.is_empty() { Stage::Delink } else { Stage::Folder }
    }

    fn fail(&mut self, error: String, now: Instant) {
        if self.recovering {
            self.message = format!(
                "{} {}",
                self.failure.as_deref().unwrap_or(""),
                aurora_i18n::tr!("replace-recovery-incomplete", reason = error)
            );
            self.advance(Stage::Failed, now);
        } else {
            self.failure = Some(error);
            self.recovering = true;
            self.advance(if self.mutated { Stage::RecoverStop } else { Stage::RecoverContents }, now);
        }
    }

    pub fn tick(&mut self, tool: &mut BuildTool, world: &mut World, now: Instant) {
        if let Some((_, error)) = self.inventory_error.take() {
            self.fail(error, now);
            return;
        }
        if self.all().any(|p| {
            world
                .objects
                .index_of_uuid(&p.id)
                .and_then(|i| world.objects.get(i))
                .is_none_or(|o| o.key != p.key)
        }) {
            self.message = aurora_i18n::tr!("replace-identity-lost").into();
            self.advance(Stage::Failed, now);
            return;
        }
        if now.saturating_duration_since(self.entered) > TIMEOUT {
            if self.stage == Stage::Cleanup {
                self.folder = None;
                self.inventory_request = None;
                self.message = aurora_i18n::tr!("replace-backup-kept").into();
                self.entered = now;
            } else {
                self.fail(aurora_i18n::tr!("replace-timeout", stage = self.stage.label()).to_owned(), now);
                return;
            }
        }
        match self.step(tool, world, now) {
            Ok(Some(next)) => self.advance(next, now),
            Ok(None) => {}
            Err(error) => self.fail(error, now),
        }
    }

    fn step(&mut self, tool: &mut BuildTool, world: &mut World, now: Instant) -> Result<Option<Stage>, String> {
        let first = !self.sent;
        self.sent = true;
        let handle = self.handle();
        match self.stage {
            Stage::Read => {
                if first {
                    for p in self.all() {
                        tool.props.remove(&p.id);
                    }
                    tool.send(BuildCmd::Select {
                        handle,
                        local_ids: self.all().map(|p| p.key.local_id).collect(),
                    });
                    let ids: Vec<Uuid> = self.all().map(|p| p.id).collect();
                    self.physics_request = Some(tool.cap(
                        handle,
                        "GetObjectPhysicsData",
                        false,
                        llsd_map! { "object_ids" => Llsd::Array(ids.iter().map(|id| Llsd::from(*id)).collect()) },
                        crate::build::edits::CapPurpose::PhysicsData(ids),
                    ));
                    for id in self.list_objects() {
                        Self::request_list(tool, world, id);
                    }
                }
                if self.physics_request.is_some()
                    || self
                        .all()
                        .any(|p| tool.props.get(&p.id).is_none_or(|props| !props.full) || !tool.physics.contains_key(&p.key.local_id))
                {
                    return Ok(None);
                }
                // None must temporarily become Convex Hull for delinking.
                // Its material must be known before sending ExtraPhysics:
                // substituting defaults would change custom physical values.
                if self.all().any(|p| {
                    tool.physics.get(&p.key.local_id).is_some_and(|params| params.shape_type == 1)
                        && !tool.physics_materials_known.contains(&p.key.local_id)
                }) {
                    return Ok(None);
                }
                let masks = self
                    .members
                    .iter()
                    .filter(|p| p.id != self.target)
                    .chain(std::iter::once(&self.replacement))
                    .filter_map(|p| tool.props.get(&p.id))
                    .fold(u32::MAX, |m, p| m & p.owner_mask);
                if masks & (perm::COPY | perm::TRANSFER) == 0 {
                    return Err(aurora_i18n::tr!("replace-incompatible-rights").into());
                }
                for p in self.all() {
                    let Some(idx) = world.objects.index_of_uuid(&p.id) else {
                        return Err(aurora_i18n::tr!("replace-missing").into());
                    };
                    if !tool.can_modify(world, idx)
                        || tool
                            .props
                            .get(&p.id)
                            .is_none_or(|props| props.owner_id != world.agent_id || props.owner_mask & perm::MODIFY == 0)
                        || world.objects.get(idx).is_none_or(|o| o.parent_id != p.parent)
                        || !p.matches(world)
                    {
                        return Err(aurora_i18n::tr!("replace-changed").into());
                    }
                }
                for p in self.members.iter_mut().chain(std::iter::once(&mut self.replacement)) {
                    p.physics = tool.physics.get(&p.key.local_id).copied();
                }
                for id in self.list_objects() {
                    if tool
                        .contents
                        .objects
                        .get(&id)
                        .is_none_or(|inv| inv.loading || inv.requested.is_none())
                    {
                        return Ok(None);
                    }
                }
                if self.options.contents {
                    self.items = tool
                        .contents
                        .objects
                        .get(&self.target)
                        .map(|i| i.items.iter().filter(|i| !i.is_folder).cloned().collect())
                        .unwrap_or_default();
                    let destination = &tool.contents.objects[&self.source].items;
                    self.destination_before = destination.iter().map(|i| i.item_id).collect();
                    let mut names: HashSet<String> = destination.iter().filter(|i| !i.is_folder).map(|i| i.name.clone()).collect();
                    for item in &self.items {
                        if item.owner_id != world.agent_id || item.owner_mask & perm::COPY == 0 {
                            return Err(aurora_i18n::tr!("replace-no-copy", name = item.name.clone()));
                        }
                        if !names.insert(item.name.clone()) {
                            return Err(aurora_i18n::tr!("replace-name-conflict", name = item.name.clone()));
                        }
                    }
                    if self.items.len() > 1024 {
                        return Err(aurora_i18n::tr!("replace-too-much-content").into());
                    }
                }
                let mut keys = HashSet::new();
                for id in self.list_objects() {
                    if let Some(inv) = tool.contents.objects.get(&id) {
                        for item in &inv.items {
                            if item.asset_type == crate::build::contents::AT_LSL_TEXT && keys.insert((id, item.item_id)) {
                                self.scripts.push(Script {
                                    object: id,
                                    item: item.item_id,
                                    running: false,
                                    touched: false,
                                    copied: false,
                                });
                            }
                        }
                    }
                }
                if self.scripts.len() > 1024 {
                    return Err(aurora_i18n::tr!("replace-too-much-content").into());
                }
                Ok(Some(Stage::ReadScripts))
            }
            Stage::ReadScripts => {
                if first {
                    self.query_scripts(tool, false, false);
                    self.poll_at = now;
                }
                if self
                    .scripts
                    .iter()
                    .any(|s| !tool.contents.running.contains_key(&(s.object, s.item)))
                {
                    self.poll_script_states(tool, now, None);
                    return Ok(None);
                }
                for script in &mut self.scripts {
                    let Some(state) = tool.contents.running.get(&(script.object, script.item)) else {
                        return Ok(None);
                    };
                    script.running = state.0;
                    script.touched = self.options.pause_scripts && state.0;
                }
                Ok(Some(if self.options.pause_scripts {
                    Stage::StopScripts
                } else {
                    self.content_stage()
                }))
            }
            Stage::StopScripts => {
                if first {
                    self.query_scripts(tool, true, false);
                    self.poll_at = now;
                }
                let confirmed = self.scripts_confirmed(tool, false);
                if !confirmed {
                    self.poll_script_states(tool, now, Some(false));
                }
                Ok(confirmed.then(|| self.content_stage()))
            }
            Stage::Folder => {
                if first {
                    if world.inventory.root.is_nil() {
                        return Err(aurora_i18n::tr!("replace-inventory-unavailable").into());
                    }
                    self.mutate_inventory(
                        tool,
                        Mutation {
                            operations: vec![Operation::Folder {
                                id: self.requested_folder,
                                parent: world.inventory.root,
                                name: aurora_i18n::tr!("replace-backup-folder").into(),
                                type_default: -1,
                            }],
                            refresh: vec![world.inventory.root, self.requested_folder],
                            ..Default::default()
                        },
                    );
                }
                if let Some(result) = self.inventory_result.take() {
                    self.inventory_request = None;
                    let reply = match result {
                        Ok(reply) => reply,
                        Err(reason) => {
                            tool.out.push(NetCommand::FetchInventory {
                                folders: vec![world.inventory.root],
                                owner: world.agent_id,
                                library: false,
                            });
                            return Err(aurora_i18n::tr!("replace-folder-failed", reason = reason));
                        }
                    };
                    let id = reply
                        .created
                        .get(&self.requested_folder)
                        .copied()
                        .ok_or_else(|| aurora_i18n::tr!("replace-inventory-unavailable").to_owned())?;
                    self.folder = Some(id);
                    world.inventory.apply(reply.contents);
                    return Ok(Some(Stage::CopyOut(0)));
                }
                Ok(None)
            }
            Stage::CopyOut(index) => {
                let folder = self
                    .folder
                    .ok_or_else(|| aurora_i18n::tr!("replace-inventory-unavailable").to_owned())?;
                let item = &self.items[index];
                if first {
                    tool.send(BuildCmd::MoveTaskInventory {
                        handle,
                        local_id: self.members[self.number - 1].key.local_id,
                        item_id: item.item_id,
                        folder_id: folder,
                    });
                    self.poll_at = now;
                }
                if let Some(copy) = world.inventory.items.values().find(|i| {
                    i.parent == folder
                        && i.name == item.name
                        && i.asset_type == item.asset_type as i32
                        && !self.staged.iter().any(|old| old.id == i.id)
                }) {
                    if copy.owner_mask & perm::COPY == 0 {
                        return Err(aurora_i18n::tr!("replace-no-copy", name = copy.name.clone()));
                    }
                    self.staged.push(copy.clone());
                    return Ok(Some(Stage::CopyIn(index)));
                }
                if now.saturating_duration_since(self.poll_at) >= Duration::from_secs(1) {
                    tool.out.push(NetCommand::FetchInventory {
                        folders: vec![folder],
                        owner: world.agent_id,
                        library: false,
                    });
                    self.poll_at = now;
                }
                Ok(None)
            }
            Stage::CopyIn(index) => {
                let original = &self.items[index];
                if first {
                    let item = Box::new(task_from_item(&self.staged[index], self.source));
                    if original.asset_type == crate::build::contents::AT_LSL_TEXT {
                        tool.send(BuildCmd::RezScript {
                            handle,
                            local_id: self.replacement.key.local_id,
                            enabled: false,
                            group_id: world.groups.active,
                            item,
                        });
                    } else {
                        tool.send(BuildCmd::UpdateTaskInventory {
                            handle,
                            local_id: self.replacement.key.local_id,
                            item,
                        });
                    }
                    Self::request_list(tool, world, self.source);
                    self.poll_at = now;
                }
                if let Some(inv) = tool.contents.objects.get(&self.source).filter(|i| !i.loading)
                    && let Some(item) = inv.items.iter().find(|i| {
                        i.name == original.name
                            && i.asset_type == original.asset_type
                            && !self.destination_before.contains(&i.item_id)
                            && !self.copied.contains(&i.item_id)
                    })
                {
                    self.copied.push(item.item_id);
                    if original.asset_type == crate::build::contents::AT_LSL_TEXT {
                        let running = self
                            .scripts
                            .iter()
                            .find(|s| s.object == self.target && s.item == original.item_id)
                            .is_some_and(|s| s.running);
                        self.scripts.push(Script {
                            object: self.source,
                            item: item.item_id,
                            running,
                            touched: true,
                            copied: true,
                        });
                    }
                    return Ok(Some(if index + 1 == self.items.len() {
                        Stage::Delink
                    } else {
                        Stage::CopyOut(index + 1)
                    }));
                }
                if now.saturating_duration_since(self.poll_at) >= Duration::from_secs(1) {
                    Self::request_list(tool, world, self.source);
                    self.poll_at = now;
                }
                Ok(None)
            }
            Stage::RecoverStop => {
                if first {
                    for script in &mut self.scripts {
                        if script.running {
                            script.touched = true;
                        }
                    }
                    self.query_scripts(tool, true, false);
                    self.poll_at = now;
                }
                let confirmed = self.scripts_confirmed(tool, false);
                if !confirmed {
                    self.poll_script_states(tool, now, Some(false));
                }
                Ok(confirmed.then_some(Stage::RecoverDelink))
            }
            Stage::Delink | Stage::RecoverDelink => {
                if first {
                    if !self.recovering && (self.all().any(|p| !p.matches(world)) || !self.topology(world, true)) {
                        return Err(aurora_i18n::tr!("replace-changed").into());
                    }
                    self.mutated = true;
                    self.physics(tool, world, true, self.recovering);
                    // Rebuild the full ordered set, keeping the root independent.
                    // This makes all child-parent changes observable, rather than
                    // assigning guessed link numbers in the local object list.
                    self.refresh_serials = self
                        .all()
                        .filter_map(|p| {
                            world
                                .objects
                                .index_of_uuid(&p.id)
                                .and_then(|i| world.objects.get(i))
                                .map(|o| (p.id, o.full_update_serial))
                        })
                        .collect();
                    let local_ids: Vec<_> = if self.recovering {
                        self.all().filter(|p| p.id != self.members[0].id).map(|p| p.key.local_id).collect()
                    } else {
                        self.members.iter().skip(1).map(|p| p.key.local_id).collect()
                    };
                    for chunk in local_ids.chunks(255) {
                        tool.send(BuildCmd::Delink {
                            handle,
                            local_ids: chunk.to_vec(),
                        });
                    }
                    tool.out.push(NetCommand::RequestObjects {
                        handle,
                        local_ids: self.all().map(|p| p.key.local_id).collect(),
                    });
                }
                let refreshed = self.refresh_serials.iter().all(|(id, before)| {
                    world
                        .objects
                        .index_of_uuid(id)
                        .and_then(|i| world.objects.get(i))
                        .is_some_and(|o| o.full_update_serial != *before)
                });
                Ok((refreshed && self.standalone(world)).then_some(if self.recovering {
                    Stage::RecoverTransform
                } else {
                    Stage::Transform
                }))
            }
            Stage::Transform | Stage::RecoverTransform => {
                if first {
                    let updates = if self.recovering {
                        self.all().map(Prim::update).collect()
                    } else {
                        vec![self.planned.update()]
                    };
                    tool.send(BuildCmd::Transform { handle, updates });
                }
                let matches = if self.recovering {
                    self.all().all(|p| p.matches(world))
                } else {
                    self.planned.matches(world)
                };
                Ok(matches.then_some(if self.recovering { Stage::RecoverLink } else { Stage::Link }))
            }
            Stage::Link | Stage::RecoverLink => {
                let expected = self.expected(self.recovering);
                let batches = link_batches(&expected);
                if first {
                    let local_ids = batches[self.link_batch]
                        .iter()
                        .map(|id| {
                            world
                                .objects
                                .index_of_uuid(id)
                                .and_then(|i| world.objects.get(i))
                                .map(|o| o.key.local_id)
                        })
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| aurora_i18n::tr!("replace-missing").to_owned())?;
                    tool.send(BuildCmd::Link { handle, local_ids });
                }
                let linked: Vec<_> = std::iter::once(expected[0])
                    .chain(
                        batches[..=self.link_batch]
                            .iter()
                            .rev()
                            .flat_map(|batch| batch[1..].iter().copied()),
                    )
                    .collect();
                if !world.objects.remember_link_order(linked[0], &linked[1..]) {
                    return Ok(None);
                }
                if self.link_batch + 1 < batches.len() {
                    self.link_batch += 1;
                    self.sent = false;
                    self.entered = now;
                    return Ok(None);
                }
                Ok(self
                    .topology(world, self.recovering)
                    .then_some(if self.recovering { Stage::RecoverVerify } else { Stage::Verify }))
            }
            Stage::Verify | Stage::RecoverVerify => {
                if !self.topology(world, self.recovering) {
                    return Ok(None);
                }
                if self
                    .members
                    .iter()
                    .filter(|p| self.recovering || p.id != self.target)
                    .any(|p| !p.matches(world))
                    || !(if self.recovering {
                        self.replacement.matches(world)
                    } else {
                        self.planned.matches(world)
                    })
                {
                    return Err(aurora_i18n::tr!("replace-transform-failed").into());
                }
                self.physics(tool, world, false, self.recovering);
                Ok(Some(if self.recovering { Stage::RecoverPhysics } else { Stage::Physics }))
            }
            Stage::Physics | Stage::RecoverPhysics => {
                if first {
                    let ids: Vec<_> = self.all().map(|p| p.id).collect();
                    self.physics_request = Some(tool.cap(
                        handle,
                        "GetObjectPhysicsData",
                        false,
                        llsd_map! { "object_ids" => Llsd::Array(ids.iter().map(|id| Llsd::from(*id)).collect()) },
                        crate::build::edits::CapPurpose::PhysicsData(ids),
                    ));
                }
                if self.physics_request.is_some() {
                    return Ok(None);
                }
                let confirmed = self.all().all(|p| {
                    let expected = p.physics.map(|v| {
                        if !self.recovering && p.id == self.target && v.shape_type == 1 {
                            2
                        } else {
                            v.shape_type
                        }
                    });
                    tool.physics
                        .get(&p.key.local_id)
                        .is_some_and(|actual| Some(actual.shape_type) == expected)
                });
                Ok(confirmed.then_some(if self.recovering { Stage::RecoverContents } else { Stage::Retire }))
            }
            Stage::Retire => {
                if first {
                    tool.send(BuildCmd::Transform {
                        handle,
                        updates: vec![self.retired.update()],
                    });
                }
                Ok(self.retired.matches(world).then_some(Stage::RestoreScripts))
            }
            Stage::RecoverContents => {
                if self.staged.is_empty() && self.copied.is_empty() {
                    return Ok(Some(Stage::RestoreScripts));
                }
                if first {
                    if let Some(inv) = tool.contents.objects.get_mut(&self.source) {
                        inv.loading = false;
                    }
                    Self::request_list(tool, world, self.source);
                    self.poll_at = now;
                }
                if let Some(inv) = tool.contents.objects.get(&self.source).filter(|i| !i.loading) {
                    // A copy may have succeeded while its acknowledgement was lost.
                    // Protect every pre-existing item; match only submitted names.
                    let added: Vec<_> = inv
                        .items
                        .iter()
                        .filter(|item| {
                            !self.destination_before.contains(&item.item_id)
                                && self
                                    .staged
                                    .iter()
                                    .any(|staged| staged.name == item.name && staged.asset_type == item.asset_type as i32)
                        })
                        .map(|item| item.item_id)
                        .collect();
                    if added.is_empty() {
                        return Ok(Some(Stage::RestoreScripts));
                    }
                    for item_id in added {
                        tool.send(BuildCmd::RemoveTaskInventory {
                            handle,
                            local_id: self.replacement.key.local_id,
                            item_id,
                        });
                    }
                    Self::request_list(tool, world, self.source);
                    self.poll_at = now;
                } else if now.saturating_duration_since(self.poll_at) >= Duration::from_secs(1) {
                    Self::request_list(tool, world, self.source);
                    self.poll_at = now;
                }
                Ok(None)
            }
            Stage::RestoreScripts => {
                if first {
                    self.query_scripts(tool, false, true);
                    self.poll_at = now;
                }
                if !self.scripts_confirmed(tool, true) {
                    self.poll_script_states(tool, now, Some(true));
                    return Ok(None);
                }
                if self.recovering {
                    self.message = if self.mutated {
                        aurora_i18n::tr!("replace-restored-after-error", reason = self.failure.clone().unwrap_or_default())
                    } else {
                        aurora_i18n::tr!("replace-stopped-before-unlink", reason = self.failure.clone().unwrap_or_default())
                    };
                    return Ok(Some(Stage::Failed));
                }
                Ok(Some(Stage::Cleanup))
            }
            Stage::Cleanup => {
                if let Some(folder) = self.folder {
                    if first {
                        let trash = world
                            .inventory
                            .folders
                            .values()
                            .find(|f| !f.library && f.info.type_default == 14)
                            .map(|f| f.info.id);
                        if let Some(trash) = trash {
                            self.mutate_inventory(
                                tool,
                                Mutation {
                                    operations: vec![Operation::Patch {
                                        id: folder,
                                        folder: true,
                                        body: llsd_map! { "parent_id" => trash },
                                    }],
                                    refresh: vec![world.inventory.root, trash],
                                    ..Default::default()
                                },
                            );
                        } else {
                            self.message = aurora_i18n::tr!("replace-backup-kept").into();
                        }
                    }
                    if self.inventory_request.is_some() {
                        let Some(result) = self.inventory_result.take() else {
                            return Ok(None);
                        };
                        self.inventory_request = None;
                        match result {
                            Ok(reply) => world.inventory.apply(reply.contents),
                            Err(_) => self.message = aurora_i18n::tr!("replace-backup-kept").into(),
                        }
                    }
                }
                tool.send(BuildCmd::Deselect {
                    handle,
                    local_ids: self.all().map(|p| p.key.local_id).collect(),
                });
                let idx = world.objects.index_of_uuid(&self.source);
                let settings = crate::build::BuildSettings {
                    edit_linked: true,
                    ..Default::default()
                };
                tool.click_select(world, &settings, idx, false);
                tool.costs_stale();
                Ok(Some(Stage::Done))
            }
            Stage::Done | Stage::Failed => Ok(None),
        }
    }
}
