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
//! in every prim of its linkset, its masks set when Objects is ticked, its
//! scripts started again, saved back once (DeRezObject
//! DRD_SAVE_INTO_TASK_INVENTORY, "Save back to object contents") and the
//! rezzed copy deleted. The original item is
//! removed only once a saved one of its name is listed; a simulator that
//! replaces it in place leaves nothing to remove. A new object is changed or
//! deleted only once its FromTaskID says it was rezzed from these contents,
//! Stop included. A coalesced item (several objects saved as one) rezzes as
//! separate objects: they are all taken, changed and saved back together in
//! one DeRezObject, as the Legacy Viewer's Save back to object contents
//! sends the whole selection (derez_objects, llviewermenu.cpp). Scripts
//! react to their own rez before any viewer can stop them: the window says
//! so.
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
/// A coalesced item's parts are all taken once no new one came for this long.
const GROUP_SETTLE: Duration = Duration::from_secs(2);
/// A save in place whose serial moved once counts after this long with no
/// other move.
const SERIAL_QUIET: Duration = Duration::from_secs(4);
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
        .filter(|o| new_of_ours(o, before) && o.position.distance(at) <= ARRIVAL_RADIUS + o.scale.length() * 0.5)
        .min_by(|a, b| a.position.distance(at).total_cmp(&b.position.distance(at)))
        .map(|o| o.id)
}

/// The parts of a rezzed coalesced item (several objects saved as one,
/// II_FLAGS_OBJECT_HAS_MULTIPLE_ITEMS): every new root of ours created
/// selected, wherever its offset put it; their FromTaskID is checked next.
pub fn group_arrivals(seen: impl IntoIterator<Item = Seen>, before: &HashSet<Uuid>) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = seen.into_iter().filter(|o| new_of_ours(o, before)).map(|o| o.id).collect();
    ids.sort();
    ids
}

fn new_of_ours(o: &Seen, before: &HashSet<Uuid>) -> bool {
    o.root
        && !o.avatar
        && o.flags & super::FLAGS_CREATE_SELECTED != 0
        && o.flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
        && !before.contains(&o.id)
}

/// Several objects saved as one item: rezzed as separate objects, saved
/// back together in one DeRezObject.
pub fn is_group(item: &TaskItem) -> bool {
    item.flags & crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS != 0
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
    /// A new item of the original's name next to it: the original is then
    /// removed.
    Added(TaskItem),
    /// The original's place taken by a new item of its name, or of the
    /// copy's: nothing to remove.
    Replaced(TaskItem),
    /// The original's own asset changed: nothing to remove.
    InPlace,
    /// A new item of the copy's name only (a script renamed it), while the
    /// original is still listed: the original is kept next to it.
    Unnamed(TaskItem),
}

/// `candidate` is `name`, or `name` numbered as a simulator numbers a
/// duplicate name in an object's contents ("Box 1", "Box 2").
pub fn same_name(candidate: &str, name: &str) -> bool {
    candidate == name
        || candidate
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix(' '))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The save back of the copy of `original`, named `saved_as` when it was
/// sent, from the listings before and after it. Only evidence tied to that
/// save counts: a single new object item of the same owner named as the
/// original (or as the copy), or the original updated in place. Second
/// Life updates it in place, keeping its id, and hides object asset ids in
/// contents listings: there the evidence is the listing's serial, which a
/// save in place moves twice (the second bringing the item's new record,
/// its group and masks), every other item staying exactly as it was. Both
/// moves are waited for (`serial_moves`), so that the next save starts from
/// a settled listing and no late update of this one can stand for it; a
/// single move counts only once the listing has been `quiet` since. Any
/// other change of the listing (another item edited, added or removed, a
/// name that only starts with the original's, two candidates) leaves it
/// pending, and only a match by the original's name lets the original be
/// removed.
pub fn save_outcome(before: &[TaskItem], after: &[TaskItem], original: &TaskItem, saved_as: &str, serial_moves: u16, quiet: bool) -> Saved {
    let known: HashSet<Uuid> = before.iter().map(|i| i.item_id).collect();
    let new: Vec<&TaskItem> = after
        .iter()
        .filter(|i| is_object(i) && !known.contains(&i.item_id) && i.owner_id == original.owner_id)
        .collect();
    let single = |name: &str| {
        let mut found = new.iter().filter(|i| same_name(&i.name, name));
        match (found.next(), found.next()) {
            (Some(one), None) => Some((*one).clone()),
            _ => None,
        }
    };
    let named = single(&original.name);
    let by_copy = (saved_as != original.name).then(|| single(saved_as)).flatten();
    let still = after.iter().find(|i| i.item_id == original.item_id);
    match (still, named, by_copy) {
        (Some(_), Some(new), _) => Saved::Added(new),
        (None, Some(new), _) | (None, None, Some(new)) => Saved::Replaced(new),
        (Some(_), None, Some(new)) => Saved::Unnamed(new),
        (Some(now), None, None) if !original.asset_id.is_nil() && now.asset_id != original.asset_id => Saved::InPlace,
        (Some(_), None, None)
            if (serial_moves >= 2 || (serial_moves == 1 && quiet)) && others_unchanged(before, after, original.item_id) =>
        {
            Saved::InPlace
        }
        _ => Saved::Pending,
    }
}

/// Every item but `original` listed after exactly as before, none added
/// or removed.
pub fn others_unchanged(before: &[TaskItem], after: &[TaskItem], original: Uuid) -> bool {
    let others = |list: &[TaskItem]| {
        let mut v: Vec<TaskItem> = list.iter().filter(|i| i.item_id != original).cloned().collect();
        v.sort_by_key(|i| i.item_id);
        v
    };
    others(before) == others(after)
}

/// How far a contents listing's serial moved, from `from` to `to` (it
/// wraps).
pub fn serial_moves(from: i16, to: i16) -> u16 {
    (to as u16).wrapping_sub(from as u16)
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
    /// The new object's origin could not be checked (its properties never
    /// came): it is not deleted.
    Unverified,
    /// A coalesced item came back as a single object: its parts are kept in
    /// world, to be taken together again.
    GroupSplit,
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
        Failure::NotOurs | Failure::Unverified | Failure::GroupSplit => Recovery {
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

/// A prim whose contents are processed: a selected prim, or one prim of a
/// rezzed copy's linkset.
#[derive(Debug, Clone)]
struct Prim {
    id: Uuid,
    key: ObjKey,
    /// Object items still to open.
    pending: VecDeque<TaskItem>,
    /// Scripts of a copy's prim and whether they ran when it arrived.
    scripts: Vec<(Uuid, Option<bool>)>,
    /// Items saved back here under another name: their original name.
    renames: Vec<(Uuid, String)>,
}

impl Prim {
    fn new(id: Uuid, key: ObjKey) -> Self {
        Self {
            id,
            key,
            pending: VecDeque::new(),
            scripts: Vec::new(),
            renames: Vec::new(),
        }
    }
}

/// An object whose contents are being processed: a selected prim (depth 0)
/// or a rezzed copy, with every prim of its linkset.
#[derive(Debug)]
struct Frame {
    /// The selected prim, or the copy's root.
    object: Uuid,
    key: ObjKey,
    depth: usize,
    /// The item of the parent this copy was rezzed from.
    copy: Option<TaskItem>,
    /// The parent's prim whose contents held that item: the rez came from
    /// it, and the save goes back into it.
    source: Option<(Uuid, ObjKey)>,
    /// The copy's roots: one, or the parts of a coalesced item.
    roots: Vec<(Uuid, ObjKey)>,
    /// The roots first, then a copy's child prims as they are seen.
    prims: Vec<Prim>,
    /// The prim being read, or whose objects are being opened.
    current: usize,
    /// Prims whose contents are changed (Apply done), from the first.
    applied: usize,
    /// A copy's roots checked by their FromTaskID (`Props`): nothing of a
    /// copy is ever deleted before.
    verified: bool,
}

impl Frame {
    fn new(object: Uuid, key: ObjKey, depth: usize, copy: Option<TaskItem>, source: Option<(Uuid, ObjKey)>) -> Self {
        Self::group(vec![(object, key)], depth, copy, source)
    }

    /// The parts of a coalesced item, or a single object (`roots` not empty).
    fn group(roots: Vec<(Uuid, ObjKey)>, depth: usize, copy: Option<TaskItem>, source: Option<(Uuid, ObjKey)>) -> Self {
        let (object, key) = roots[0];
        Self {
            object,
            key,
            depth,
            copy,
            source,
            prims: roots.iter().map(|(id, key)| Prim::new(*id, *key)).collect(),
            roots,
            current: 0,
            applied: 0,
            verified: false,
        }
    }

    /// The copy's child prims not listed yet (a linkset's updates may come
    /// after its root's).
    fn add_children(&mut self, world: &World) {
        if self.copy.is_none() {
            return;
        }
        for (_, key) in self.roots.clone() {
            for &idx in world.objects.children_of(&key) {
                if let Some(o) = world.objects.get(idx)
                    && !self.prims.iter().any(|p| p.id == o.full_id)
                {
                    self.prims.push(Prim::new(o.full_id, o.key));
                }
            }
        }
    }

    fn prim(&self) -> &Prim {
        &self.prims[self.current.min(self.prims.len() - 1)]
    }

    fn prim_mut(&mut self) -> &mut Prim {
        let i = self.current.min(self.prims.len() - 1);
        &mut self.prims[i]
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
    /// The rez of an arrived object whose origin is being checked: waited
    /// for again if the object turns out to be another one.
    arrived: Option<(TaskItem, HashSet<Uuid>, Vec3, u64)>,
    /// The RezObject of `rezzing` already sent: not sent again.
    rez_resumed: bool,
    /// A coalesced item's parts seen so far, and when the last one came.
    gathering: Option<(Vec<Uuid>, Instant)>,
    /// The source's listing and serial before the save back, and the
    /// copy's name then.
    snapshot: Option<(Vec<TaskItem>, i16, String)>,
    saved: Option<TaskItem>,
    /// The source listing's last serial seen during a save, and since when.
    serial_seen: Option<(i16, Instant)>,
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
            arrived: None,
            rez_resumed: false,
            gathering: None,
            snapshot: None,
            saved: None,
            serial_seen: None,
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
        self.stack.iter().flat_map(|f| f.prims.iter().map(|p| p.id)).collect()
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
            self.stack.push(Frame::new(id, o.key, 0, None, None));
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
            // a rez in flight is waited for, an arrival's origin checked
            // before anything is deleted, a save back sent is seen through
            if !self.rez_in_flight()
                && !matches!(
                    self.stage,
                    Stage::Props | Stage::Save | Stage::RemoveOriginal | Stage::Delete | Stage::Recover | Stage::Done
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
            // an arrival never checked is never deleted
            Stage::Props if elapsed > TIMEOUT => {
                self.fail(tool, world, Failure::Unverified, now);
                return;
            }
            Stage::Save if elapsed > TIMEOUT => {
                let gone = self
                    .top()
                    .and_then(|f| f.source)
                    .and_then(|(source, _)| tool.contents.objects.get(&source))
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
            Failure::Unverified => aurora_i18n::tr!("content-perms-nested-unverified", name = name.as_str()),
            Failure::GroupSplit => aurora_i18n::tr!("content-perms-nested-group-split", name = name.as_str()),
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
        self.arrived = None;
        self.rez_resumed = false;
        self.gathering = None;
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
        let verified = self.top().is_some_and(|f| f.verified);
        let next = if own_copy && rec.delete_copy && verified {
            Stage::Recover
        } else if own_copy {
            // kept in world: not ours, the only instance left, or not
            // checked yet (another object may stand among what came)
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
        // a listing error of the prim being read
        if let Some(errored) = self.inventory_error.take()
            && matches!(self.stage, Stage::Read | Stage::Apply)
            && self.top().is_some_and(|f| f.prim().id == errored)
        {
            return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-unreadable").into()));
        }
        if self.stage == Stage::Next && self.stack.is_empty() {
            return Ok(Some(self.next_prim(tool, world)));
        }
        let Some(frame) = self.stack.last() else {
            return Ok(Some(Stage::Done));
        };
        let (root, root_key, depth) = (frame.object, frame.key, frame.depth);
        let (copy, source) = (frame.copy.clone(), frame.source);
        // the prim whose contents are read, opened or changed
        let (object, key) = (frame.prim().id, frame.prim().key);
        let renames = frame.prim().renames.clone();
        let linkset: Vec<(Uuid, ObjKey)> = frame.prims.iter().map(|p| (p.id, p.key)).collect();
        let roots = frame.roots.clone();
        let group = copy.as_ref().is_some_and(is_group);
        let scripts: Vec<(Uuid, ObjKey, Uuid, Option<bool>)> = frame
            .prims
            .iter()
            .flat_map(|p| p.scripts.iter().map(move |s| (p.id, p.key, s.0, s.1)))
            .collect();
        // the copy must stay in world until it is deleted
        if copy.is_some()
            && !matches!(self.stage, Stage::Delete | Stage::Recover)
            && roots.iter().any(|(id, _)| world.objects.index_of_uuid(id).is_none())
        {
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
                    "deep permissions: level {depth} listing of prim {} of {}, {} objects to open, {} skipped",
                    self.top().map_or(0, |f| f.current) + 1,
                    linkset.len(),
                    found.eligible.len(),
                    found.skipped.len()
                );
                self.report.skipped.extend(found.skipped);
                let found_scripts: Vec<(Uuid, Option<bool>)> = items
                    .iter()
                    .filter(|i| i.asset_type == super::contents::AT_LSL_TEXT)
                    .map(|i| (i.item_id, None))
                    .collect();
                if scripts.len() + found_scripts.len() > MAX_SCRIPTS {
                    return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-too-many").into()));
                }
                let Some(frame) = self.stack.last_mut() else {
                    return Ok(Some(Stage::Done));
                };
                frame.prim_mut().pending = found.eligible.into();
                if copy.is_some() {
                    frame.prim_mut().scripts = found_scripts;
                    // every prim of the linkset read before its scripts stop
                    frame.add_children(world);
                    if frame.current + 1 < frame.prims.len() {
                        frame.current += 1;
                        return Ok(Some(Stage::Read));
                    }
                    // the first prim whose contents are not changed yet
                    frame.current = frame.applied;
                    return Ok(Some(Stage::Scripts));
                }
                if frame.prim().pending.is_empty() {
                    return Ok(Some(Stage::Apply));
                }
                Self::preflight(world, frame)?;
                Ok(Some(Stage::Next))
            }
            Stage::Scripts => {
                // (only the scripts not asked yet: those of late child
                // prims come after the others stopped and started again)
                let unknown: Vec<(Uuid, ObjKey, Uuid)> = scripts.iter().filter(|s| s.3.is_none()).map(|s| (s.0, s.1, s.2)).collect();
                if first {
                    for (prim, prim_key, item) in &unknown {
                        tool.contents.running.remove(&(*prim, *item));
                        Self::running_query(tool, *prim_key, *prim, *item);
                    }
                }
                let poll = self.poll(now);
                let mut known = true;
                for (prim, prim_key, item) in &unknown {
                    if !tool.contents.running.contains_key(&(*prim, *item)) {
                        known = false;
                        if poll {
                            Self::running_query(tool, *prim_key, *prim, *item);
                        }
                    }
                }
                if !known {
                    return Ok(None);
                }
                let mut any = false;
                if let Some(frame) = self.stack.last_mut() {
                    for p in &mut frame.prims {
                        for s in &mut p.scripts {
                            if s.1.is_none() {
                                s.1 = Some(tool.contents.running.get(&(p.id, s.0)).is_some_and(|r| r.0));
                            }
                            any |= s.1 == Some(true);
                        }
                    }
                }
                Ok(Some(if any { Stage::StopScripts } else { Stage::Next }))
            }
            Stage::StopScripts | Stage::RestoreScripts => {
                let restore = self.stage == Stage::RestoreScripts;
                let ran: Vec<(Uuid, ObjKey, Uuid)> = scripts.iter().filter(|s| s.3 == Some(true)).map(|s| (s.0, s.1, s.2)).collect();
                if first {
                    for (prim, prim_key, item) in &ran {
                        tool.contents.running.remove(&(*prim, *item));
                        tool.send(BuildCmd::SetScriptRunning {
                            handle: prim_key.region,
                            object_id: *prim,
                            item_id: *item,
                            running: restore,
                        });
                        Self::running_query(tool, *prim_key, *prim, *item);
                    }
                }
                let poll = self.poll(now);
                let mut confirmed = true;
                for (prim, prim_key, item) in &ran {
                    if tool.contents.running.get(&(*prim, *item)).is_none_or(|r| r.0 != restore) {
                        confirmed = false;
                        if poll {
                            Self::running_query(tool, *prim_key, *prim, *item);
                        }
                    }
                }
                if !confirmed {
                    return Ok(None);
                }
                Ok(Some(if restore { Stage::Rename } else { Stage::Next }))
            }
            Stage::Next => {
                let item = self.stack.last_mut().and_then(|f| f.prim_mut().pending.pop_front());
                match item {
                    Some(item) => {
                        self.rezzing = Some((item, HashSet::new(), Vec3::ZERO, 0));
                        self.rez_resumed = false;
                        Ok(Some(Stage::Rez))
                    }
                    None => Ok(Some(Stage::Apply)),
                }
            }
            Stage::Rez => {
                if first && std::mem::take(&mut self.rez_resumed) {
                    // waiting again for a rez already sent
                    return Ok(None);
                }
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
                if is_group(&item) {
                    let parts = group_arrivals(seen, &before);
                    if parts.is_empty() {
                        // (an alert may still refuse it, below)
                    } else if self.gathering.as_ref().is_none_or(|(seen, _)| *seen != parts) {
                        self.gathering = Some((parts, now));
                        return Ok(None);
                    } else if self
                        .gathering
                        .as_ref()
                        .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= GROUP_SETTLE)
                    {
                        let Some((parts, _)) = self.gathering.take() else {
                            return Ok(None);
                        };
                        let roots: Vec<(Uuid, ObjKey)> = parts
                            .iter()
                            .filter_map(|id| {
                                world
                                    .objects
                                    .index_of_uuid(id)
                                    .and_then(|i| world.objects.get(i))
                                    .map(|o| (*id, o.key))
                            })
                            .collect();
                        if roots.is_empty() {
                            return Ok(None);
                        }
                        log::info!(
                            "deep permissions: {} arrived as {} objects, checking where they come from",
                            item.name,
                            roots.len()
                        );
                        self.stack.push(Frame::group(roots, depth + 1, Some(item), Some((object, key))));
                        self.arrived = self.rezzing.take();
                        return Ok(Some(Stage::Props));
                    } else {
                        return Ok(None);
                    }
                } else if let Some(id) = arrival(seen, &before, at) {
                    let Some(o) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i)) else {
                        return Ok(None);
                    };
                    log::info!("deep permissions: {} arrived as {id}, checking where it comes from", item.name);
                    let frame = Frame::new(id, o.key, depth + 1, Some(item), Some((object, key)));
                    self.stack.push(frame);
                    self.arrived = self.rezzing.take();
                    // even when stopped: nothing is deleted before its origin is known
                    return Ok(Some(Stage::Props));
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
                let local_ids: Vec<u32> = roots.iter().map(|(_, k)| k.local_id).collect();
                if first || self.poll(now) {
                    if first {
                        for (id, _) in &roots {
                            tool.props.remove(id);
                        }
                    }
                    tool.send(BuildCmd::Select {
                        handle: root_key.region,
                        local_ids,
                    });
                }
                let Some(all) = roots
                    .iter()
                    .map(|(id, _)| tool.props.get(id).filter(|p| p.full).cloned())
                    .collect::<Option<Vec<_>>>()
                else {
                    return Ok(None);
                };
                let from = source.map(|s| s.0).unwrap_or_default();
                let (ours, others): (Vec<_>, Vec<_>) = roots.iter().zip(&all).partition(|(_, p)| p.from_task_id == from);
                if ours.is_empty() {
                    // other objects of ours that looked like the arrival:
                    // left alone, and the one asked for still waited for
                    let Some(mut waiting) = self.arrived.take() else {
                        return Err(Failure::NotOurs);
                    };
                    log::info!("deep permissions: {} objects not from {from}: left alone", others.len());
                    waiting.1.extend(roots.iter().map(|(id, _)| *id));
                    self.stack.pop();
                    self.rezzing = Some(waiting);
                    self.rez_resumed = true;
                    return Ok(Some(Stage::Rez));
                }
                if !others.is_empty() {
                    // a coalesced item's parts among other new objects: those stay out of it
                    let keep: Vec<(Uuid, ObjKey)> = ours.iter().map(|(r, _)| **r).collect();
                    log::info!("deep permissions: {} new objects not from {from}: left alone", others.len());
                    if let Some(frame) = self.stack.last_mut() {
                        let copy = frame.copy.take();
                        *frame = Frame::group(keep, frame.depth, copy, frame.source);
                    }
                }
                self.arrived = None;
                if let Some(frame) = self.stack.last_mut() {
                    frame.verified = true;
                }
                if self.user_stopped {
                    // stopped while it was on its way: straight to the trash
                    return Ok(Some(Stage::Recover));
                }
                for (_, props) in &ours {
                    if props.owner_id != world.agent_id || props.owner_mask & perm::MODIFY == 0 {
                        log::info!(
                            "deep permissions: the rezzed object grants {:#x} (base {:#x}): no modify",
                            props.owner_mask,
                            props.base_mask
                        );
                        return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-skip-no-modify").into()));
                    }
                }
                if let Some(frame) = self.stack.last_mut() {
                    frame.add_children(world);
                    log::info!(
                        "deep permissions: {root} is ours, {} objects, {} prims",
                        frame.roots.len(),
                        frame.prims.len()
                    );
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
                // the copy's next prim, then the whole linkset's masks
                if let Some(frame) = self.stack.last_mut() {
                    frame.applied = frame.applied.max(frame.current + 1);
                    if frame.current + 1 < frame.prims.len() {
                        frame.current += 1;
                        return Ok(Some(Stage::Next));
                    }
                }
                Ok(Some(if self.options.includes_type(i32::from(AT_OBJECT)) {
                    Stage::ObjectPerms
                } else {
                    Stage::RestoreScripts
                }))
            }
            Stage::ObjectPerms => {
                let (group, everyone, next) = object_masks(&self.options);
                let local_ids: Vec<u32> = linkset.iter().map(|(_, k)| k.local_id).collect();
                if first {
                    for (field, set, mask) in permission_changes(&self.options) {
                        tool.send(BuildCmd::SetPermissions {
                            handle: root_key.region,
                            local_ids: local_ids.clone(),
                            field,
                            set,
                            mask,
                        });
                    }
                }
                if first || self.poll(now) {
                    tool.send(BuildCmd::Select {
                        handle: root_key.region,
                        local_ids,
                    });
                }
                let done = linkset.iter().all(|(id, _)| {
                    tool.props.get(id).is_some_and(|p| {
                        let base = p.base_mask;
                        let g = perm::COPY | perm::MODIFY | perm::MOVE;
                        let n = perm::MODIFY | perm::COPY | perm::TRANSFER;
                        p.group_mask & g & base == group & g & base
                            && p.everyone_mask & perm::COPY & base == everyone & perm::COPY & base
                            && p.next_owner_mask & n & base == next & n & base
                    })
                });
                Ok(done.then_some(Stage::RestoreScripts))
            }
            Stage::Rename => {
                let Some(original) = copy else {
                    return Ok(Some(Stage::Save));
                };
                // a coalesced item's name is none of its parts'
                if group {
                    return Ok(Some(Stage::Save));
                }
                if first {
                    // fresh properties: a script may rename the object at any time
                    tool.props.remove(&root);
                }
                if first || self.poll(now) {
                    tool.send(BuildCmd::Select {
                        handle: root_key.region,
                        local_ids: vec![root_key.local_id],
                    });
                }
                let Some(props) = tool.props.get(&root).filter(|p| p.full) else {
                    return Ok(None);
                };
                if props.name == original.name {
                    return Ok(Some(Stage::Save));
                }
                if !self.name_sent {
                    self.name_sent = true;
                    log::info!("deep permissions: renaming the copy {:?} back to {:?}", props.name, original.name);
                    tool.send(BuildCmd::SetName {
                        handle: root_key.region,
                        local_ids: vec![root_key.local_id],
                        name: original.name.clone(),
                    });
                    tool.send(BuildCmd::Select {
                        handle: root_key.region,
                        local_ids: vec![root_key.local_id],
                    });
                }
                Ok(None)
            }
            Stage::Save => {
                let (Some((source, _)), Some(original)) = (source, copy) else {
                    return Ok(Some(Stage::Delete));
                };
                if world.objects.index_of_uuid(&source).is_none() {
                    return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()));
                }
                // child prims whose updates came after the others were read:
                // their contents and scripts too, before the one save
                if first && let Some(frame) = self.stack.last_mut() {
                    let known = frame.prims.len();
                    frame.add_children(world);
                    if frame.prims.len() > known {
                        log::info!(
                            "deep permissions: {} more prims of {root} came, read before the save",
                            frame.prims.len() - known
                        );
                        frame.current = frame.applied;
                        return Ok(Some(Stage::Read));
                    }
                }
                if first {
                    let (before, serial) = tool
                        .contents
                        .objects
                        .get(&source)
                        .map(|inv| (inv.items.iter().filter(|i| !i.is_folder).cloned().collect(), inv.serial))
                        .unwrap_or_default();
                    // the name the saved item takes: the item's, unless a
                    // script renamed the copy again
                    let saved_as = if group {
                        original.name.clone()
                    } else {
                        tool.props
                            .get(&root)
                            .filter(|p| p.full)
                            .map_or_else(|| original.name.clone(), |p| p.name.clone())
                    };
                    if roots.iter().any(|(_, k)| k.region != root_key.region) {
                        return Err(Failure::Step(aurora_i18n::tr!("content-perms-nested-gone").into()));
                    }
                    self.snapshot = Some((before, serial, saved_as));
                    self.serial_seen = None;
                    log::info!(
                        "deep permissions: saving {} back into {source} ({} objects)",
                        self.name(),
                        roots.len()
                    );
                    // a coalesced item's parts together, as the Legacy
                    // Viewer's Save back to object contents sends the whole
                    // selection (derez_objects, llviewermenu.cpp)
                    tool.send(BuildCmd::DeRez {
                        handle: root_key.region,
                        local_ids: roots.iter().map(|(_, k)| k.local_id).collect(),
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
                let Some((before, serial, saved_as)) = self.snapshot.as_ref() else {
                    return Ok(Some(Stage::Delete));
                };
                let items: Vec<TaskItem> = inv.items.iter().filter(|i| !i.is_folder).cloned().collect();
                if self.serial_seen.is_none_or(|(seen, _)| seen != inv.serial) {
                    self.serial_seen = Some((inv.serial, now));
                }
                let quiet = self
                    .serial_seen
                    .is_some_and(|(_, since)| now.saturating_duration_since(since) >= SERIAL_QUIET);
                let outcome = save_outcome(before, &items, &original, saved_as, serial_moves(*serial, inv.serial), quiet);
                if group {
                    // the saved item must still hold several objects (in
                    // place, its record came with the serial's second move)
                    let saved = match &outcome {
                        Saved::InPlace => items.iter().find(|i| i.item_id == original.item_id),
                        Saved::Added(new) | Saved::Replaced(new) | Saved::Unnamed(new) => Some(new),
                        Saved::Pending => None,
                    };
                    if saved.is_some_and(|item| !is_group(item)) {
                        return Err(Failure::GroupSplit);
                    }
                }
                if outcome != Saved::Pending {
                    log::info!("deep permissions: save of {} -> {outcome:?}", original.name);
                }
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
                // only ever after a save matched by the original's name
                let (Some((source, source_key)), Some(original), Some(_)) = (source, copy, self.saved.as_ref()) else {
                    return Ok(Some(Stage::Delete));
                };
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
            Stage::Delete | Stage::Recover if !self.top().is_some_and(|f| f.verified) => {
                // never an object whose origin was not checked
                log::warn!("deep permissions: {root} not checked as rezzed from these contents: left in world");
                self.stack.pop();
                Ok(Some(self.after_copy(tool, world)))
            }
            Stage::Delete | Stage::Recover => {
                let left: Vec<(Uuid, ObjKey)> = roots
                    .iter()
                    .filter(|(id, _)| world.objects.index_of_uuid(id).is_some())
                    .copied()
                    .collect();
                if first {
                    if !left.is_empty() {
                        log::info!("deep permissions: deleting the copy {root} ({} objects)", left.len());
                        tool.send(BuildCmd::DeRez {
                            handle: root_key.region,
                            local_ids: left.iter().map(|(_, k)| k.local_id).collect(),
                            destination: derez::TRASH,
                            destination_id: BuildTool::folder_of_type(world, TRASH_FOLDER),
                            group_id: world.groups.active,
                        });
                    }
                    return Ok(None);
                }
                if !left.is_empty() {
                    return Ok(None);
                }
                self.stack.pop();
                Ok(Some(self.after_copy(tool, world)))
            }
            Stage::Done => Ok(None),
        }
    }

    /// A save back found under another name: the parent's prim that holds
    /// it gives it its item's name back with its own contents change.
    fn rename_in_parent(&mut self, item: Uuid, name: String) {
        let Some(source) = self.top().and_then(|f| f.source).map(|s| s.0) else {
            return;
        };
        if let Some(i) = self.stack.len().checked_sub(2)
            && let Some(parent) = self.stack.get_mut(i)
            && let Some(prim) = parent.prims.iter_mut().find(|p| p.id == source)
        {
            prim.renames.push((item, name));
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
