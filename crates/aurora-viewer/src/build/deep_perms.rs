//! "Include objects inside objects" of the Content permissions window
//! (`build/ui/contents_permissions.rs`): the same Group / Anyone / Next
//! owner change, applied inside the object items of the selected prims'
//! contents, at every level down to [`MAX_DEPTH`].
//!
//! An Aurora addition: LLFloaterBulkPermission (indra/newview/
//! llfloaterbulkpermission.cpp) changes one level only, its recursion into
//! objects is a commented-out TODO (doApply). An object item's contents can
//! only be changed in world, so each object is rezzed a few metres above the
//! object that holds it (RezObject with FromTaskID, like
//! LLToolDragAndDrop::dropObject from task inventory), its running scripts
//! stopped, its own contents changed (its objects first, deepest first),
//! its masks set when Objects is ticked, its scripts started again, saved
//! back (DeRezObject DRD_SAVE_INTO_TASK_INVENTORY, "Save back to object
//! contents") and the rezzed copy deleted. The original item is removed only
//! once the saved one is listed; a simulator that replaces it in place
//! leaves nothing to remove.
//!
//! Only objects the Legacy Viewer lets us rez from contents are opened
//! (dad3dRezFromObjectOnLand, lltooldraganddrop.cpp: copy for the agent and
//! transfer, `actions::task_rez_allowed`), and only those we may modify
//! (`actions::task_agent_mask`): the others keep their contents and get the
//! one-level change of their item, as without the option.
//!
//! Every stage is confirmed by what the simulator sends back (the new
//! object, its properties, the listings, ScriptRunningReply), with the stage
//! skeleton of the replacement job (`replace/transaction.rs`). A failure
//! after a rez deletes that copy and leaves its original untouched; a
//! refused or lost rez stops the whole job.

use super::BuildTool;
use super::ui::contents_permissions::BulkPerms;
use crate::world::World;
use crate::world::inventory::actions::{task_agent_mask, task_rez_allowed};
use crate::world::objects::ObjKey;
use aurora_net::build::{BuildCmd, TaskItem, derez, perm, perm_field};
use glam::{Quat, Vec3};
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[path = "deep_perms_demo.rs"]
pub mod demo;
#[cfg(test)]
#[path = "deep_perms_tests.rs"]
mod tests;

/// Levels of objects inside objects that are opened (the human's choice).
pub const MAX_DEPTH: usize = 5;
/// Above the top of the object being processed.
pub const REZ_HEIGHT: f32 = 2.0;
/// A new object this close to the rez point (plus its own half size) is
/// the one asked for.
pub const ARRIVAL_RADIUS: f32 = 2.0;
const TIMEOUT: Duration = Duration::from_secs(30);
const ARRIVAL: Duration = Duration::from_secs(15);
/// The simulator may refuse a mask the object's base does not allow: the
/// masks are sent, then the job goes on.
const PERMS_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_secs(2);
const MAX_SCRIPTS: usize = 256;
const AT_OBJECT: i8 = 6;
const TRASH_FOLDER: i32 = 14;

/// Why an object item of the contents is not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Not copyable or not transferable: the Legacy Viewer refuses to rez it
    /// from contents.
    NoCopy,
    NoModify,
    TooDeep,
}

impl Skip {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoCopy => aurora_i18n::tr!("content-perms-nested-skip-no-copy"),
            Self::NoModify => aurora_i18n::tr!("content-perms-nested-skip-no-modify"),
            Self::TooDeep => aurora_i18n::tr!("content-perms-nested-skip-depth"),
        }
    }
}

pub fn is_object(item: &TaskItem) -> bool {
    !item.is_folder && item.asset_type == AT_OBJECT
}

/// Whether an object item held at `depth` (0: the selected prim's own
/// contents) is rezzed and opened.
pub fn eligibility(item: &TaskItem, agent: Uuid, active_group: Uuid, depth: usize) -> Result<(), Skip> {
    if depth >= MAX_DEPTH {
        return Err(Skip::TooDeep);
    }
    if !task_rez_allowed(item, agent, active_group) {
        return Err(Skip::NoCopy);
    }
    if task_agent_mask(item, agent) & perm::MODIFY == 0 {
        return Err(Skip::NoModify);
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scan {
    pub eligible: Vec<TaskItem>,
    pub skipped: Vec<(String, Skip)>,
}

/// The object items of one listing, opened or skipped with their reason.
pub fn scan(items: &[TaskItem], agent: Uuid, active_group: Uuid, depth: usize) -> Scan {
    let mut out = Scan::default();
    for item in items.iter().filter(|i| is_object(i)) {
        match eligibility(item, agent, active_group, depth) {
            Ok(()) => out.eligible.push(item.clone()),
            Err(skip) => out.skipped.push((item.name.clone(), skip)),
        }
    }
    out
}

/// The rez point above a prim: its centre, [`REZ_HEIGHT`] above the
/// highest corner of its box.
pub fn rez_point(position: Vec3, rotation: Quat, scale: Vec3) -> Vec3 {
    let half = scale.abs() * 0.5;
    let top = (rotation * Vec3::X).z.abs() * half.x + (rotation * Vec3::Y).z.abs() * half.y + (rotation * Vec3::Z).z.abs() * half.z;
    Vec3::new(position.x, position.y, position.z + top + REZ_HEIGHT)
}

/// An object of the world, as the arrival test reads it.
#[derive(Debug, Clone, Copy)]
pub struct Seen {
    pub id: Uuid,
    pub root: bool,
    pub avatar: bool,
    pub flags: u32,
    pub position: Vec3,
    pub scale: Vec3,
}

/// The object the simulator rezzed for our request: a root that was not
/// there before, flagged "created selected" (addressed to the client that
/// asked) and ours, near the rez point; the nearest if several.
pub fn arrival(seen: impl IntoIterator<Item = Seen>, before: &HashSet<Uuid>, at: Vec3) -> Option<Uuid> {
    seen.into_iter()
        .filter(|o| {
            o.root
                && !o.avatar
                && o.flags & super::FLAGS_CREATE_SELECTED != 0
                && o.flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                && !before.contains(&o.id)
                && o.position.distance(at) <= ARRIVAL_RADIUS + o.scale.length() * 0.5
        })
        .min_by(|a, b| a.position.distance(at).total_cmp(&b.position.distance(at)))
        .map(|o| o.id)
}

/// Words of a refusal, matched as whole words.
const REFUSAL_WORDS: [&str; 6] = ["can't", "can\u{2019}t", "cannot", "unable", "denied", "full"];
/// Phrases of a refusal, matched as they are.
const REFUSAL_PHRASES: [&str; 4] = ["can not", "not allowed", "not permitted", "no permission"];

/// A simulator alert that answers the rez of `name`: there is no typed
/// refusal, so an alert counts only when it speaks of rezzing and names
/// the object or says no (Second Life's "Can't rez object 'Box' at …
/// because the owner of this land does not allow it.", OpenSim's
/// "Can't rez object because the parcel is full.").
pub fn rez_refusal(alert: &str, name: &str) -> bool {
    let alert = alert.to_lowercase();
    let words: Vec<&str> = alert
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .filter(|w| !w.is_empty())
        .collect();
    if !words.iter().any(|w| w.starts_with("rez")) {
        return false;
    }
    let name = name.trim().to_lowercase();
    (!name.is_empty() && alert.contains(&name))
        || words.iter().any(|w| REFUSAL_WORDS.contains(w))
        || REFUSAL_PHRASES.iter().any(|p| alert.contains(p))
}

/// What a listing of the source says about a save back.
#[derive(Debug, Clone, PartialEq)]
pub enum Saved {
    Pending,
    /// A new item next to the original, which is then removed.
    Added(TaskItem),
    /// The original's place taken by a new item: nothing to remove.
    Replaced(TaskItem),
    /// The original updated in place (its asset or the listing's serial
    /// changed): nothing to remove.
    InPlace,
    /// Only one new object, of another name, while the original is still
    /// listed: probably the save, but the original is kept next to it.
    Unnamed(TaskItem),
}

/// The save back of the copy of `original`, from the listings before and
/// after it. The new item has the original's name, or starts with it (a
/// simulator may number a duplicate name), or is the only new object; the
/// original is removed only after a match by name.
pub fn save_outcome(before: &[TaskItem], after: &[TaskItem], original: &TaskItem, serial_moved: bool) -> Saved {
    let known: HashSet<Uuid> = before.iter().map(|i| i.item_id).collect();
    let new: Vec<&TaskItem> = after.iter().filter(|i| is_object(i) && !known.contains(&i.item_id)).collect();
    let named = new
        .iter()
        .find(|i| i.name == original.name)
        .or_else(|| new.iter().find(|i| i.name.starts_with(&original.name)))
        .map(|i| (*i).clone());
    let only = (new.len() == 1).then(|| new[0].clone());
    let still = after.iter().find(|i| i.item_id == original.item_id);
    match (still, named, only) {
        (Some(_), Some(new), _) => Saved::Added(new),
        (None, Some(new), _) | (None, None, Some(new)) => Saved::Replaced(new),
        (Some(_), None, Some(new)) => Saved::Unnamed(new),
        (Some(now), None, None) if (!original.asset_id.is_nil() && now.asset_id != original.asset_id) || serial_moved => Saved::InPlace,
        _ => Saved::Pending,
    }
}

/// Why processing an object stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    /// The checks before the first rez (land, capacity, RLVa).
    Preflight(String),
    /// The simulator answered the rez with an alert.
    RezRefused(String),
    /// Nothing came: it may still come, nothing more is rezzed.
    RezTimeout,
    /// The new object does not come from the source: it is not deleted.
    NotOurs,
    /// A stage of a rezzed copy failed or timed out.
    Step(String),
    /// The save back was not seen and the original is gone: the copy is
    /// the only one left.
    OriginalGone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    pub delete_copy: bool,
    pub stop: bool,
}

/// After a failure: delete the copy whose original is intact, keep one
/// that may be the only instance left or not ours, and stop the whole job
/// when rezzing itself failed.
pub fn recovery(failure: &Failure) -> Recovery {
    match failure {
        Failure::Preflight(_) | Failure::RezRefused(_) | Failure::RezTimeout => Recovery {
            delete_copy: false,
            stop: true,
        },
        Failure::NotOurs => Recovery {
            delete_copy: false,
            stop: true,
        },
        Failure::Step(_) => Recovery {
            delete_copy: true,
            stop: false,
        },
        Failure::OriginalGone => Recovery {
            delete_copy: false,
            stop: false,
        },
    }
}

/// The masks the Objects choice gives a rezzed copy, as the one-level pass
/// gives an object item.
pub fn object_masks(options: &BulkPerms) -> (u32, u32, u32) {
    let mut item = TaskItem {
        asset_type: AT_OBJECT,
        ..Default::default()
    };
    options.apply(&mut item);
    (item.group_mask, item.everyone_mask, item.next_owner_mask)
}

/// ObjectPermissions set / clear pairs for those masks: the group's share,
/// everyone's copy and the next owner's modify, copy and transfer.
pub fn permission_changes(options: &BulkPerms) -> Vec<(u8, bool, u32)> {
    let (group, everyone, next) = object_masks(options);
    let mut out = Vec::new();
    for (field, managed, wanted) in [
        (perm_field::GROUP, perm::COPY | perm::MODIFY | perm::MOVE, group),
        (perm_field::EVERYONE, perm::COPY, everyone),
        (perm_field::NEXT_OWNER, perm::MODIFY | perm::COPY | perm::TRANSFER, next),
    ] {
        if managed & wanted != 0 {
            out.push((field, true, managed & wanted));
        }
        if managed & !wanted != 0 {
            out.push((field, false, managed & !wanted));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Read,
    Scripts,
    StopScripts,
    Next,
    Rez,
    Props,
    Apply,
    ObjectPerms,
    RestoreScripts,
    /// The copy takes its item's name back before the save (a script may
    /// have renamed it), so that the saved item is found by its name.
    Rename,
    Save,
    RemoveOriginal,
    Delete,
    Recover,
    Done,
}

/// An object whose contents are being processed: a selected prim (depth 0)
/// or a rezzed copy.
#[derive(Debug)]
struct Frame {
    object: Uuid,
    key: ObjKey,
    depth: usize,
    /// Object items still to open.
    pending: VecDeque<TaskItem>,
    /// The item of the parent this copy was rezzed from.
    copy: Option<TaskItem>,
    /// Scripts of the copy and whether they ran when it arrived.
    scripts: Vec<(Uuid, Option<bool>)>,
    /// Items saved back here under another name: their original name.
    renames: Vec<(Uuid, String)>,
}

impl Frame {
    fn new(object: Uuid, key: ObjKey, depth: usize, copy: Option<TaskItem>) -> Self {
        Self {
            object,
            key,
            depth,
            pending: VecDeque::new(),
            copy,
            scripts: Vec::new(),
            renames: Vec::new(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub updated: usize,
    pub objects: usize,
    pub skipped: Vec<(String, Skip)>,
    pub failures: Vec<String>,
    pub prims_skipped: usize,
    /// Saved items found only as the one new object: the original kept.
    pub kept_next: usize,
    pub stopped: Option<String>,
}

#[derive(Debug)]
pub struct Job {
    options: BulkPerms,
    prims: VecDeque<Uuid>,
    stack: Vec<Frame>,
    pub stage: Stage,
    entered: Instant,
    sent: bool,
    poll_at: Instant,
    /// The item being rezzed, the objects before, the rez point and the
    /// last alert then.
    rezzing: Option<(TaskItem, HashSet<Uuid>, Vec3, u64)>,
    /// The source's listing and serial before the save back.
    snapshot: Option<(Vec<TaskItem>, i16)>,
    saved: Option<TaskItem>,
    stopping: bool,
    /// Stop pressed, and taken into account.
    stop_requested: bool,
    user_stopped: bool,
    /// SetName sent in the Rename stage.
    name_sent: bool,
    pub inventory_error: Option<Uuid>,
    pub report: Report,
}

impl Job {
    pub fn new(options: BulkPerms, prims: Vec<Uuid>, now: Instant) -> Self {
        Self {
            options,
            prims: prims.into(),
            stack: Vec::new(),
            stage: Stage::Next,
            entered: now,
            sent: false,
            poll_at: now,
            rezzing: None,
            snapshot: None,
            saved: None,
            stopping: false,
            stop_requested: false,
            user_stopped: false,
            name_sent: false,
            inventory_error: None,
            report: Report::default(),
        }
    }

    pub fn done(&self) -> bool {
        self.stage == Stage::Done
    }

    /// Objects whose listings the job reads (kept by the contents cache).
    pub fn objects(&self) -> Vec<Uuid> {
        self.stack.iter().map(|f| f.object).collect()
    }

    fn top(&self) -> Option<&Frame> {
        self.stack.last()
    }

    fn name(&self) -> String {
        self.rezzing
            .as_ref()
            .map(|r| r.0.name.clone())
            .or_else(|| self.top().and_then(|f| f.copy.as_ref()).map(|i| i.name.clone()))
            .unwrap_or_default()
    }

    /// Stop: the rezzed copies still in world are deleted, a rez in
    /// flight is waited for to delete it too, nothing more is changed; a
    /// save back already sent is seen through.
    pub fn request_stop(&mut self) {
        self.stop_requested = true;
    }

    /// A RezObject sent whose object has not arrived yet.
    pub fn rez_in_flight(&self) -> bool {
        self.stage == Stage::Rez && self.sent
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_requested
    }

    pub(crate) fn level(&self) -> usize {
        self.top().map_or(0, |f| f.depth)
    }

    pub fn status(&self) -> String {
        let name = self.name();
        match self.stage {
            Stage::Read | Stage::Next => aurora_i18n::tr!("content-perms-reading").into(),
            Stage::Rez | Stage::Props => aurora_i18n::tr!("content-perms-nested-rezzing", name = name, level = self.level() + 1),
            Stage::Scripts | Stage::StopScripts => aurora_i18n::tr!("content-perms-nested-stopping", name = name),
            Stage::Apply | Stage::ObjectPerms => aurora_i18n::tr!("content-perms-nested-applying", name = name),
            Stage::RestoreScripts => aurora_i18n::tr!("content-perms-nested-restoring", name = name),
            Stage::Rename | Stage::Save | Stage::RemoveOriginal => aurora_i18n::tr!("content-perms-nested-saving", name = name),
            Stage::Delete | Stage::Recover => aurora_i18n::tr!("content-perms-nested-deleting", name = name),
            Stage::Done => self.report_text(false),
        }
    }

    /// The final report; `details` adds what was skipped and why (the
    /// window's preview line already lists the first level).
    pub fn report_text(&self, details: bool) -> String {
        let r = &self.report;
        let mut text = aurora_i18n::tr!(
            "content-perms-nested-finished",
            count = r.updated,
            objects = r.objects,
            skipped = r.skipped.len() + r.prims_skipped
        );
        if r.kept_next > 0 {
            text.push(' ');
            text.push_str(&aurora_i18n::tr!("content-perms-nested-kept-next", kept = r.kept_next));
        }
        let sep = aurora_i18n::tr!("content-perms-nested-separator");
        if details && !r.skipped.is_empty() {
            let list: Vec<String> = r
                .skipped
                .iter()
                .take(6)
                .map(|(name, why)| aurora_i18n::tr!("content-perms-nested-entry", name = name.as_str(), reason = why.label()))
                .collect();
            text.push(' ');
            text.push_str(&list.join(sep));
        }
        for failure in r.failures.iter().take(4) {
            text.push(' ');
            text.push_str(failure);
        }
        if let Some(stopped) = &r.stopped {
            text.push(' ');
            text.push_str(stopped);
        }
        text
    }

    fn advance(&mut self, stage: Stage, now: Instant) {
        log::info!("deep permissions: {:?} -> {stage:?} (level {})", self.stage, self.level());
        self.stage = stage;
        self.entered = now;
        self.sent = false;
        self.name_sent = false;
        self.poll_at = now;
    }

    fn poll(&mut self, now: Instant) -> bool {
        if now.saturating_duration_since(self.poll_at) >= POLL {
            self.poll_at = now;
            true
        } else {
            false
        }
    }

    /// The next selected prim, or the end.
    fn next_prim(&mut self, tool: &BuildTool, world: &World) -> Stage {
        while let Some(id) = self.prims.pop_front() {
            let Some(idx) = world.objects.index_of_uuid(&id) else {
                self.report.prims_skipped += 1;
                continue;
            };
            let (Some(o), true) = (world.objects.get(idx), tool.contents_editable(world, idx)) else {
                self.report.prims_skipped += 1;
                continue;
            };
            self.stack.push(Frame::new(id, o.key, 0, None));
            return Stage::Read;
        }
        log::info!(
            "deep permissions: finished, {} items in {} objects, {} skipped",
            self.report.updated,
            self.report.objects,
            self.report.skipped.len() + self.report.prims_skipped
        );
        Stage::Done
    }

    pub fn tick(&mut self, tool: &mut BuildTool, world: &World, now: Instant) {
        if self.stage == Stage::Done {
            return;
        }
        if self.stop_requested && !self.user_stopped {
            self.user_stopped = true;
            self.stopping = true;
            log::info!("deep permissions: stop asked at {:?} (level {})", self.stage, self.level());
            if self.report.stopped.is_none() {
                self.report.stopped = Some(aurora_i18n::tr!("content-perms-nested-stopped").into());
            }
            // a rez in flight is waited for, a save back sent is seen through
            if !self.rez_in_flight()
                && !matches!(
                    self.stage,
                    Stage::Save | Stage::RemoveOriginal | Stage::Delete | Stage::Recover | Stage::Done
                )
            {
                let next = if self.top().is_some_and(|f| f.copy.is_some()) {
                    Stage::Recover
                } else {
                    Stage::Done
                };
                self.advance(next, now);
                return;
            }
        }
        let elapsed = now.saturating_duration_since(self.entered);
        // soft limits: the change was sent, the job goes on
        match self.stage {
            Stage::ObjectPerms if elapsed > PERMS_WAIT => {
                log::warn!("deep permissions: masks of {} not confirmed, going on", self.name());
                self.advance(Stage::RestoreScripts, now);
                return;
            }
            Stage::Rename if elapsed > PERMS_WAIT => {
                log::warn!("deep permissions: the name of {} not confirmed, saving anyway", self.name());
                self.advance(Stage::Save, now);
                return;
            }
            Stage::RemoveOriginal if elapsed > TIMEOUT => {
                let name = self.name();
                self.report
                    .failures
                    .push(aurora_i18n::tr!("content-perms-nested-original-kept", name = name.as_str()));
                self.advance(Stage::Delete, now);
                return;
            }
            Stage::Delete | Stage::Recover if elapsed > TIMEOUT => {
                let name = self.name();
                self.report
                    .failures
                    .push(aurora_i18n::tr!("content-perms-nested-copy-left", name = name.as_str()));
                self.stack.pop();
                let next = self.after_copy(tool, world);
                self.advance(next, now);
                return;
            }
            Stage::Rez if elapsed > ARRIVAL => {
                self.fail(tool, world, Failure::RezTimeout, now);
                return;
            }
            Stage::Save if elapsed > TIMEOUT => {
                let gone = self
                    .parent()
                    .and_then(|p| tool.contents.objects.get(&p.object))
                    .zip(self.top().and_then(|f| f.copy.as_ref()))
                    .is_some_and(|(inv, original)| !inv.items.iter().any(|i| i.item_id == original.item_id));
                let failure = if gone {
                    Failure::OriginalGone
                } else {
                    Failure::Step(aurora_i18n::tr!("content-perms-nested-save-unconfirmed").into())
                };
                self.fail(tool, world, failure, now);
                return;
            }
            Stage::Next | Stage::Done => {}
            _ if elapsed > TIMEOUT => {
                self.fail(
                    tool,
                    world,
                    Failure::Step(aurora_i18n::tr!("content-perms-nested-timeout").into()),
                    now,
                );
                return;
            }
            _ => {}
        }
        match self.step(tool, world, now) {
            Ok(Some(next)) => self.advance(next, now),
            Ok(None) => {}
            Err(failure) => self.fail(tool, world, failure, now),
        }
    }

    fn parent(&self) -> Option<&Frame> {
        self.stack.len().checked_sub(2).and_then(|i| self.stack.get(i))
    }

    /// The stage after a copy frame was popped.
    fn after_copy(&mut self, tool: &BuildTool, world: &World) -> Stage {
        if self.stopping {
            if self.top().is_some_and(|f| f.copy.is_some()) {
                Stage::Recover
            } else {
                Stage::Done
            }
        } else if self.stack.is_empty() {
            self.next_prim(tool, world)
        } else {
            Stage::Next
        }
    }

    fn fail(&mut self, tool: &mut BuildTool, world: &World, failure: Failure, now: Instant) {
        let rec = recovery(&failure);
        let name = self.name();
        let reason = match &failure {
            Failure::Preflight(text) => text.clone(),
            Failure::RezRefused(alert) => {
                aurora_i18n::tr!("content-perms-nested-rez-refused", name = name.as_str(), reason = alert.as_str())
            }
            Failure::RezTimeout => aurora_i18n::tr!("content-perms-nested-rez-timeout", name = name.as_str()),
            Failure::NotOurs => aurora_i18n::tr!("content-perms-nested-not-ours", name = name.as_str()),
            Failure::Step(text) => aurora_i18n::tr!("content-perms-nested-failed", name = name.as_str(), reason = text.as_str()),
            Failure::OriginalGone => aurora_i18n::tr!("content-perms-nested-original-gone", name = name.as_str()),
        };
        log::warn!(
            "deep permissions: {failure:?} at {:?} (level {}): {reason}",
            self.stage,
            self.level()
        );
        // while rezzing, the top frame is the container, not the failing copy
        let own_copy = self.rezzing.is_none() && self.top().is_some_and(|f| f.copy.is_some());
        self.rezzing = None;
        self.snapshot = None;
        self.saved = None;
        if rec.stop {
            self.stopping = true;
            if self.report.stopped.is_none() {
                self.report.stopped = Some(reason);
            } else {
                self.report.failures.push(reason);
            }
        } else {
            self.report.failures.push(reason);
        }
        let top_is_copy = self.top().is_some_and(|f| f.copy.is_some());
        let next = if own_copy && rec.delete_copy {
            Stage::Recover
        } else if own_copy {
            // kept in world: not ours, or the only instance left
            self.stack.pop();
            self.after_copy(tool, world)
        } else if self.stopping {
            // the copies holding the one that failed go to the trash
            if top_is_copy { Stage::Recover } else { Stage::Done }
        } else if top_is_copy {
            Stage::Recover
        } else {
            // a selected prim that could not be read
            self.stack.pop();
            self.report.prims_skipped += 1;
            self.next_prim(tool, world)
        };
        self.advance(next, now);
    }

    fn listing(tool: &BuildTool, object: Uuid) -> Option<&crate::build::contents::TaskInventory> {
        tool.contents
            .objects
            .get(&object)
            .filter(|inv| !inv.loading && inv.requested.is_some())
    }

    /// A fresh listing of `object`: asked once, again after the contents
    /// cache's retry delay while it does not come.
    fn read(tool: &mut BuildTool, world: &World, object: Uuid, first: bool) -> Result<bool, Failure> {
        let Some(idx) = world.objects.index_of_uuid(&object) else {
            return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()));
        };
        if first {
            tool.contents_changed(object);
            tool.request_contents(world, idx, true);
            return Ok(false);
        }
        if Self::listing(tool, object).is_some() {
            return Ok(true);
        }
        tool.request_contents(world, idx, true);
        Ok(false)
    }

    fn running_query(tool: &mut BuildTool, key: ObjKey, object: Uuid, item: Uuid) {
        tool.send(BuildCmd::GetScriptRunning {
            handle: key.region,
            object_id: object,
            item_id: item,
        });
    }

    fn preflight(world: &World, frame: &Frame) -> Result<(), Failure> {
        let at =
            Self::rez_point_of(world, frame.object).ok_or_else(|| Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()))?;
        match world.build_right_at(frame.key.region, at, None) {
            crate::world::land::BuildRight::Denied => {
                return Err(Failure::Preflight(aurora_i18n::tr!("content-perms-nested-land-denied").into()));
            }
            crate::world::land::BuildRight::Unknown => {
                log::info!("deep permissions: parcel at the rez point not known, the first rez decides");
            }
            crate::world::land::BuildRight::Allowed => {}
        }
        if let Some(p) = world.known_parcel_at(frame.key.region, at, None)
            && p.max_prims > 0
            && p.max_prims - p.total_prims <= 0
        {
            return Err(Failure::Preflight(aurora_i18n::tr!("content-perms-nested-parcel-full").into()));
        }
        if !world.rlva_can_rez(None) {
            return Err(Failure::Preflight(aurora_i18n::tr!("content-perms-nested-rlva").into()));
        }
        Ok(())
    }

    fn rez_point_of(world: &World, object: Uuid) -> Option<Vec3> {
        let idx = world.objects.index_of_uuid(&object)?;
        let (p, r) = super::replace::region_transform(world, idx)?;
        let o = world.objects.get(idx)?;
        Some(rez_point(p, r, o.scale))
    }

    fn step(&mut self, tool: &mut BuildTool, world: &World, now: Instant) -> Result<Option<Stage>, Failure> {
        let first = !self.sent;
        self.sent = true;
        // a listing error of the object being read
        if let Some(object) = self.inventory_error.take()
            && matches!(self.stage, Stage::Read | Stage::Apply)
            && self.top().is_some_and(|f| f.object == object)
        {
            return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-unreadable").into()));
        }
        if self.stage == Stage::Next && self.stack.is_empty() {
            return Ok(Some(self.next_prim(tool, world)));
        }
        let Some(frame) = self.stack.last() else {
            return Ok(Some(Stage::Done));
        };
        let (object, key, depth) = (frame.object, frame.key, frame.depth);
        let (copy, scripts, renames) = (frame.copy.clone(), frame.scripts.clone(), frame.renames.clone());
        // the copy must stay in world until it is deleted
        if copy.is_some() && !matches!(self.stage, Stage::Delete | Stage::Recover) && world.objects.index_of_uuid(&object).is_none() {
            return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()));
        }
        match self.stage {
            Stage::Read => {
                if !Self::read(tool, world, object, first)? {
                    return Ok(None);
                }
                let items: Vec<TaskItem> = Self::listing(tool, object)
                    .map(|inv| inv.items.iter().filter(|i| !i.is_folder).cloned().collect())
                    .unwrap_or_default();
                let found = scan(&items, world.agent_id, world.groups.active, depth);
                log::info!(
                    "deep permissions: level {depth} listing, {} objects to open, {} skipped",
                    found.eligible.len(),
                    found.skipped.len()
                );
                self.report.skipped.extend(found.skipped);
                let found_scripts: Vec<(Uuid, Option<bool>)> = items
                    .iter()
                    .filter(|i| i.asset_type == super::contents::AT_LSL_TEXT)
                    .map(|i| (i.item_id, None))
                    .collect();
                if found_scripts.len() > MAX_SCRIPTS {
                    return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-too-many").into()));
                }
                if let Some(frame) = self.stack.last_mut() {
                    frame.pending = found.eligible.into();
                    if copy.is_some() {
                        frame.scripts = found_scripts;
                    }
                }
                if copy.is_some() {
                    return Ok(Some(Stage::Scripts));
                }
                if self.stack.last().is_some_and(|f| f.pending.is_empty()) {
                    return Ok(Some(Stage::Apply));
                }
                if let Some(frame) = self.stack.last() {
                    Self::preflight(world, frame)?;
                }
                Ok(Some(Stage::Next))
            }
            Stage::Scripts => {
                let scripts: Vec<Uuid> = scripts.iter().map(|s| s.0).collect();
                if first {
                    for item in &scripts {
                        tool.contents.running.remove(&(object, *item));
                        Self::running_query(tool, key, object, *item);
                    }
                }
                let poll = self.poll(now);
                let mut known = true;
                for item in &scripts {
                    if !tool.contents.running.contains_key(&(object, *item)) {
                        known = false;
                        if poll {
                            Self::running_query(tool, key, object, *item);
                        }
                    }
                }
                if !known {
                    return Ok(None);
                }
                let mut any = false;
                if let Some(frame) = self.stack.last_mut() {
                    for s in &mut frame.scripts {
                        let running = tool.contents.running.get(&(object, s.0)).is_some_and(|r| r.0);
                        s.1 = Some(running);
                        any |= running;
                    }
                }
                Ok(Some(if any { Stage::StopScripts } else { Stage::Next }))
            }
            Stage::StopScripts | Stage::RestoreScripts => {
                let restore = self.stage == Stage::RestoreScripts;
                let ran: Vec<Uuid> = scripts.iter().filter(|s| s.1 == Some(true)).map(|s| s.0).collect();
                if first {
                    for item in &ran {
                        tool.contents.running.remove(&(object, *item));
                        tool.send(BuildCmd::SetScriptRunning {
                            handle: key.region,
                            object_id: object,
                            item_id: *item,
                            running: restore,
                        });
                        Self::running_query(tool, key, object, *item);
                    }
                }
                let poll = self.poll(now);
                let mut confirmed = true;
                for item in &ran {
                    if tool.contents.running.get(&(object, *item)).is_none_or(|r| r.0 != restore) {
                        confirmed = false;
                        if poll {
                            Self::running_query(tool, key, object, *item);
                        }
                    }
                }
                if !confirmed {
                    return Ok(None);
                }
                Ok(Some(if restore { Stage::Rename } else { Stage::Next }))
            }
            Stage::Next => {
                let item = self.stack.last_mut().and_then(|f| f.pending.pop_front());
                match item {
                    Some(item) => {
                        self.rezzing = Some((item, HashSet::new(), Vec3::ZERO, 0));
                        Ok(Some(Stage::Rez))
                    }
                    None => Ok(Some(Stage::Apply)),
                }
            }
            Stage::Rez => {
                if first {
                    let at = Self::rez_point_of(world, object)
                        .ok_or_else(|| Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()))?;
                    let Some((item, before, point, alert)) = self.rezzing.as_mut() else {
                        return Ok(Some(Stage::Next));
                    };
                    let inv = item
                        .to_inv_item()
                        .ok_or_else(|| Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()))?;
                    *before = world.objects.iter().map(|(_, o)| o.full_id).collect();
                    *point = at;
                    *alert = world.recent_alerts.back().map_or(0, |a| a.0);
                    log::info!(
                        "deep permissions: rezzing {} from {object} at {at} (level {})",
                        item.name,
                        depth + 1
                    );
                    tool.send(BuildCmd::RezInventory(Box::new(aurora_net::inventory::rez::RezInventory {
                        handle: key.region,
                        group: world.group_for_rez(true),
                        ray_start: at,
                        ray_end: at,
                        target: Uuid::nil(),
                        selected: true,
                        remove: task_agent_mask(item, world.agent_id) & perm::COPY == 0,
                        source_folder: inv.parent,
                        from_task: object,
                        notecard: None,
                        item: inv,
                    })));
                    return Ok(None);
                }
                let Some((item, before, at, alert)) = self.rezzing.clone() else {
                    return Ok(Some(Stage::Next));
                };
                let seen = world.objects.iter().map(|(_, o)| Seen {
                    id: o.full_id,
                    root: o.parent_id == 0,
                    avatar: o.is_avatar(),
                    flags: o.update_flags,
                    position: o.position,
                    scale: o.scale,
                });
                if let Some(id) = arrival(seen, &before, at) {
                    let Some(o) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i)) else {
                        return Ok(None);
                    };
                    log::info!("deep permissions: {} arrived as {id}", item.name);
                    let frame = Frame::new(id, o.key, depth + 1, Some(item));
                    self.stack.push(frame);
                    self.rezzing = None;
                    // stopped while it was on its way: straight to the trash
                    return Ok(Some(if self.user_stopped { Stage::Recover } else { Stage::Props }));
                }
                if let Some((_, message)) = world
                    .recent_alerts
                    .iter()
                    .find(|(n, text)| *n > alert && rez_refusal(text, &item.name))
                {
                    return Err(Failure::RezRefused(message.clone()));
                }
                Ok(None)
            }
            Stage::Props => {
                if first || self.poll(now) {
                    if first {
                        tool.props.remove(&object);
                    }
                    tool.send(BuildCmd::Select {
                        handle: key.region,
                        local_ids: vec![key.local_id],
                    });
                }
                let Some(props) = tool.props.get(&object).filter(|p| p.full) else {
                    return Ok(None);
                };
                let source = self.parent().map(|p| p.object).unwrap_or_default();
                if props.from_task_id != source {
                    return Err(Failure::NotOurs);
                }
                if props.owner_id != world.agent_id || props.owner_mask & perm::MODIFY == 0 {
                    return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-skip-no-modify").into()));
                }
                Ok(Some(Stage::Read))
            }
            Stage::Apply => {
                if !Self::read(tool, world, object, first)? {
                    return Ok(None);
                }
                let items: Vec<TaskItem> = Self::listing(tool, object)
                    .map(|inv| inv.items.iter().filter(|i| !i.is_folder).cloned().collect())
                    .unwrap_or_default();
                for mut item in items {
                    let rename = renames.iter().find(|(id, _)| *id == item.item_id).map(|r| r.1.clone());
                    let include = self.options.includes(&item);
                    if !include && rename.is_none() {
                        continue;
                    }
                    if include {
                        self.options.apply(&mut item);
                        self.report.updated += 1;
                    }
                    if let Some(name) = rename {
                        item.name = name;
                    }
                    tool.send(BuildCmd::UpdateTaskInventory {
                        handle: key.region,
                        local_id: key.local_id,
                        item: Box::new(item),
                    });
                }
                tool.contents_changed(object);
                if depth == 0 {
                    self.stack.pop();
                    return Ok(Some(self.next_prim(tool, world)));
                }
                Ok(Some(if self.options.includes_type(i32::from(AT_OBJECT)) {
                    Stage::ObjectPerms
                } else {
                    Stage::RestoreScripts
                }))
            }
            Stage::ObjectPerms => {
                let (group, everyone, next) = object_masks(&self.options);
                if first {
                    for (field, set, mask) in permission_changes(&self.options) {
                        tool.send(BuildCmd::SetPermissions {
                            handle: key.region,
                            local_ids: vec![key.local_id],
                            field,
                            set,
                            mask,
                        });
                    }
                }
                if first || self.poll(now) {
                    tool.send(BuildCmd::Select {
                        handle: key.region,
                        local_ids: vec![key.local_id],
                    });
                }
                let done = tool.props.get(&object).is_some_and(|p| {
                    let base = p.base_mask;
                    let g = perm::COPY | perm::MODIFY | perm::MOVE;
                    let n = perm::MODIFY | perm::COPY | perm::TRANSFER;
                    p.group_mask & g & base == group & g & base
                        && p.everyone_mask & perm::COPY & base == everyone & perm::COPY & base
                        && p.next_owner_mask & n & base == next & n & base
                });
                Ok(done.then_some(Stage::RestoreScripts))
            }
            Stage::Rename => {
                let Some(original) = copy else {
                    return Ok(Some(Stage::Save));
                };
                if first {
                    // fresh properties: a script may rename the object at any time
                    tool.props.remove(&object);
                }
                if first || self.poll(now) {
                    tool.send(BuildCmd::Select {
                        handle: key.region,
                        local_ids: vec![key.local_id],
                    });
                }
                let Some(props) = tool.props.get(&object).filter(|p| p.full) else {
                    return Ok(None);
                };
                if props.name == original.name {
                    return Ok(Some(Stage::Save));
                }
                if !self.name_sent {
                    self.name_sent = true;
                    log::info!("deep permissions: renaming the copy {:?} back to {:?}", props.name, original.name);
                    tool.send(BuildCmd::SetName {
                        handle: key.region,
                        local_ids: vec![key.local_id],
                        name: original.name.clone(),
                    });
                    tool.send(BuildCmd::Select {
                        handle: key.region,
                        local_ids: vec![key.local_id],
                    });
                }
                Ok(None)
            }
            Stage::Save => {
                let Some(parent) = self.parent() else {
                    return Ok(Some(Stage::Delete));
                };
                let source = parent.object;
                if world.objects.index_of_uuid(&source).is_none() {
                    return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()));
                }
                if first {
                    let before = tool
                        .contents
                        .objects
                        .get(&source)
                        .map(|inv| (inv.items.clone(), inv.serial))
                        .unwrap_or_default();
                    self.snapshot = Some(before);
                    log::info!("deep permissions: saving {} back into {source}", self.name());
                    tool.send(BuildCmd::DeRez {
                        handle: key.region,
                        local_ids: vec![key.local_id],
                        destination: derez::SAVE_INTO_TASK_INVENTORY,
                        destination_id: source,
                        group_id: world.groups.active,
                    });
                    Self::read(tool, world, source, true)?;
                    return Ok(None);
                }
                let Some(inv) = Self::listing(tool, source) else {
                    Self::read(tool, world, source, false)?;
                    return Ok(None);
                };
                let (Some((before, serial)), Some(original)) = (self.snapshot.as_ref(), copy) else {
                    return Ok(Some(Stage::Delete));
                };
                let items: Vec<TaskItem> = inv.items.iter().filter(|i| !i.is_folder).cloned().collect();
                let outcome = save_outcome(before, &items, &original, inv.serial != *serial);
                log::info!("deep permissions: save of {} -> {outcome:?}", original.name);
                match outcome {
                    Saved::Pending => {
                        if self.poll(now) {
                            Self::read(tool, world, source, true)?;
                        }
                        Ok(None)
                    }
                    Saved::Added(new) => {
                        self.report.objects += 1;
                        self.saved = Some(new);
                        Ok(Some(Stage::RemoveOriginal))
                    }
                    Saved::Replaced(new) => {
                        self.report.objects += 1;
                        if new.name != original.name {
                            self.rename_in_parent(new.item_id, original.name.clone());
                        }
                        Ok(Some(Stage::Delete))
                    }
                    Saved::InPlace => {
                        self.report.objects += 1;
                        Ok(Some(Stage::Delete))
                    }
                    Saved::Unnamed(new) => {
                        log::warn!(
                            "deep permissions: {} saved as {:?}, the original is kept next to it",
                            original.name,
                            new.name
                        );
                        self.report.objects += 1;
                        self.report.kept_next += 1;
                        Ok(Some(Stage::Delete))
                    }
                }
            }
            Stage::RemoveOriginal => {
                let (Some(parent), Some(original)) = (self.parent(), copy) else {
                    return Ok(Some(Stage::Delete));
                };
                let (source, source_key) = (parent.object, parent.key);
                if first {
                    tool.send(BuildCmd::RemoveTaskInventory {
                        handle: source_key.region,
                        local_id: source_key.local_id,
                        item_id: original.item_id,
                    });
                    Self::read(tool, world, source, true)?;
                    return Ok(None);
                }
                let Some(inv) = Self::listing(tool, source) else {
                    Self::read(tool, world, source, false)?;
                    return Ok(None);
                };
                if inv.items.iter().any(|i| i.item_id == original.item_id) {
                    if self.poll(now) {
                        Self::read(tool, world, source, true)?;
                    }
                    return Ok(None);
                }
                if let Some(new) = self.saved.take()
                    && new.name != original.name
                {
                    self.rename_in_parent(new.item_id, original.name.clone());
                }
                Ok(Some(Stage::Delete))
            }
            Stage::Delete | Stage::Recover => {
                if first {
                    if world.objects.index_of_uuid(&object).is_some() {
                        log::info!("deep permissions: deleting the copy {object}");
                        tool.send(BuildCmd::DeRez {
                            handle: key.region,
                            local_ids: vec![key.local_id],
                            destination: derez::TRASH,
                            destination_id: BuildTool::folder_of_type(world, TRASH_FOLDER),
                            group_id: world.groups.active,
                        });
                    }
                    return Ok(None);
                }
                if world.objects.index_of_uuid(&object).is_some() {
                    return Ok(None);
                }
                self.stack.pop();
                Ok(Some(self.after_copy(tool, world)))
            }
            Stage::Done => Ok(None),
        }
    }

    fn rename_in_parent(&mut self, item: Uuid, name: String) {
        if let Some(i) = self.stack.len().checked_sub(2)
            && let Some(parent) = self.stack.get_mut(i)
        {
            parent.renames.push((item, name));
        }
    }
}

impl BuildTool {
    pub(crate) fn deep_inventory_error(&mut self, object: Uuid) {
        if let Some(job) = self.ui.bulk.deep.as_mut() {
            job.inventory_error = Some(object);
        }
    }

    /// Once a frame, from `update`; the report replaces the status at the end.
    pub(crate) fn advance_deep_permissions(&mut self, world: &World, now: Instant) {
        let Some(mut job) = self.ui.bulk.deep.take() else { return };
        job.tick(self, world, now);
        if job.done() {
            let text = job.report_text(true);
            log::info!("deep permissions: {text}");
            self.ui.bulk.status = job.report_text(false);
            self.status = text;
        } else {
            self.ui.bulk.status = job.status();
            self.ui.bulk.deep = Some(job);
        }
    }
}
