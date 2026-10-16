//! The script actions of a selection: "Reset / Run / Stop / Remove
//! scripts" of the object menu (Tools.SelectedScriptAction). Port of
//! queue_actions (indra/newview/llviewermenu.cpp) and LLFloaterScriptQueue
//! with its reset, run, not-run and delete queues
//! (indra/newview/llcompilequeue.cpp), originally LGPL 2.1: every prim of
//! the selection that is scripted and that we may modify, one after the
//! other, its inventory listed, then each of its scripts reset, set
//! running or not, or removed, with the lines of the progress window.

use super::BuildTool;
use super::contents::AT_LSL_TEXT;
use crate::ui::context::flags;
use crate::world::World;
use aurora_net::build::BuildCmd;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// QUEUE_INVENTORY_FETCH_TIMEOUT: an object whose inventory does not come
/// is skipped after this long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    Reset,
    Run,
    Stop,
    Delete,
}

impl QueueKind {
    /// ResetQueueTitle, RunQueueTitle, NotRunQueueTitle, DeleteQueueTitle.
    pub fn title(self) -> &'static str {
        match self {
            QueueKind::Reset => aurora_i18n::tr!("script-queue-reset-title"),
            QueueKind::Run => aurora_i18n::tr!("script-queue-run-title"),
            QueueKind::Stop => aurora_i18n::tr!("script-queue-stop-title"),
            QueueKind::Delete => aurora_i18n::tr!("script-queue-delete-title"),
        }
    }

    /// The [START] of "Starting [START] of [COUNT] items." (…QueueStart).
    fn start(self) -> &'static str {
        match self {
            QueueKind::Reset => aurora_i18n::tr!("script-queue-reset-start"),
            QueueKind::Run => aurora_i18n::tr!("script-queue-run-start"),
            QueueKind::Stop => aurora_i18n::tr!("script-queue-stop-start"),
            QueueKind::Delete => aurora_i18n::tr!("script-queue-delete-start"),
        }
    }

    /// The line of each script ("Resetting: name"…).
    fn line(self, script: &str) -> String {
        let script = script.to_string();
        match self {
            QueueKind::Reset => aurora_i18n::tr!("script-queue-resetting", script = script),
            QueueKind::Run => aurora_i18n::tr!("script-queue-running", script = script),
            QueueKind::Stop => aurora_i18n::tr!("script-queue-not-running", script = script),
            QueueKind::Delete => aurora_i18n::tr!("script-queue-deleting", script = script),
        }
    }

    /// Cannot<msg>SelectObjectsNoScripts / NoPermission, when no prim of the
    /// selection is both scripted and ours to modify.
    pub fn refusal(self, scripted: bool) -> String {
        let first = match self {
            QueueKind::Reset => aurora_i18n::tr!("script-queue-cannot-reset"),
            QueueKind::Run => aurora_i18n::tr!("script-queue-cannot-run"),
            QueueKind::Stop => aurora_i18n::tr!("script-queue-cannot-stop"),
            QueueKind::Delete => aurora_i18n::tr!("script-queue-cannot-delete"),
        };
        let second = if scripted {
            aurora_i18n::tr!("script-queue-select-modifiable-scripts")
        } else {
            aurora_i18n::tr!("script-queue-select-scripts")
        };
        format!("{first}\n\n{second}")
    }
}

/// queue_actions / QueueObjects: the prims `flags_of` reports scripted and
/// modifiable, with their names. Err(scripted) when there is none: whether
/// the last prim looked at was scripted picks the refusal, as there.
pub fn queue_objects(prims: &[(Uuid, u32, String)]) -> Result<Vec<(Uuid, String)>, bool> {
    let mut scripted = false;
    let mut out = Vec::new();
    for (id, update_flags, name) in prims {
        scripted = update_flags & flags::SCRIPTED != 0;
        if scripted && update_flags & flags::OBJECT_MODIFY != 0 {
            out.push((*id, name.clone()));
        }
    }
    if out.is_empty() { Err(scripted) } else { Ok(out) }
}

/// A queue under way and the lines of its window.
#[derive(Debug, Clone)]
pub struct ScriptQueue {
    pub kind: QueueKind,
    pub lines: Vec<String>,
    /// Every object went through ("Done.", the Close button enabled).
    pub done: bool,
    objects: VecDeque<(Uuid, String)>,
    /// The object whose inventory is awaited, and since when.
    current: Option<(Uuid, String, Instant)>,
}

impl ScriptQueue {
    /// onScriptModifyConfirmation: "Starting [START] of [COUNT] items."
    pub fn new(kind: QueueKind, objects: Vec<(Uuid, String)>) -> ScriptQueue {
        let line = aurora_i18n::tr!("script-queue-starting", start = kind.start().to_string(), count = objects.len());
        ScriptQueue {
            kind,
            lines: vec![line],
            done: false,
            objects: objects.into(),
            current: None,
        }
    }

    /// objectScriptProcessingQueueCoro, one frame of it: the next object's
    /// inventory is asked for ("Loading inventory for: …"); once it is
    /// there, its scripts get the action.
    pub fn step(&mut self, build: &mut BuildTool, world: &World, now: Instant) {
        while !self.done {
            let Some((id, name, since)) = self.current.clone() else {
                let Some((id, name)) = self.objects.pop_front() else {
                    self.lines.push(aurora_i18n::tr!("script-queue-done").into());
                    self.done = true;
                    return;
                };
                // an object gone since is skipped without a line
                let Some(idx) = world.objects.index_of_uuid(&id) else {
                    continue;
                };
                build.request_contents(world, idx, false);
                self.lines.push(aurora_i18n::tr!("script-queue-loading", object = name.clone()));
                self.current = Some((id, name, now));
                continue;
            };
            let Some(idx) = world.objects.index_of_uuid(&id) else {
                self.current = None;
                continue;
            };
            let listed = build
                .contents
                .objects
                .get(&id)
                .is_some_and(|inv| !inv.loading && inv.requested.is_some());
            if !listed {
                if now.duration_since(since) > FETCH_TIMEOUT {
                    self.lines.push(aurora_i18n::tr!("script-queue-timeout", object = name));
                    self.current = None;
                    continue;
                }
                // asked again when an answer got lost (contents::RETRY)
                build.request_contents(world, idx, false);
                return;
            }
            self.current = None;
            let scripts: Vec<(Uuid, String)> = build.contents.objects[&id]
                .items
                .iter()
                .filter(|i| i.asset_type == AT_LSL_TEXT)
                .map(|i| (i.item_id, i.name.clone()))
                .collect();
            let Some(o) = world.objects.get(idx) else { continue };
            let handle = o.key.region;
            for (item_id, script) in scripts {
                self.lines.push(self.kind.line(&script));
                match self.kind {
                    QueueKind::Reset => build.send(BuildCmd::ScriptReset {
                        handle,
                        object_id: id,
                        item_id,
                    }),
                    QueueKind::Run | QueueKind::Stop => build.set_script_running(world, idx, item_id, self.kind == QueueKind::Run),
                    QueueKind::Delete => build.remove_item(world, idx, item_id),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_scripted_prims_we_may_modify_are_queued() {
        let (a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let scripted = flags::SCRIPTED | flags::OBJECT_MODIFY;
        let prims = [
            (a, scripted, "Door".to_string()),
            (b, flags::OBJECT_MODIFY, "Frame".to_string()),
            (c, scripted, "Handle".to_string()),
        ];
        assert_eq!(queue_objects(&prims), Ok(vec![(a, "Door".to_string()), (c, "Handle".to_string())]));
        // nothing scripted / scripted but not ours to modify
        assert_eq!(queue_objects(&[(b, flags::OBJECT_MODIFY, String::new())]), Err(false));
        assert_eq!(queue_objects(&[(a, flags::SCRIPTED, String::new())]), Err(true));
    }

    #[test]
    fn the_refusal_names_the_action_and_what_to_select() {
        let text = QueueKind::Reset.refusal(false);
        assert!(text.starts_with(aurora_i18n::tr!("script-queue-cannot-reset")));
        assert!(text.ends_with(aurora_i18n::tr!("script-queue-select-scripts")));
        assert!(
            QueueKind::Delete
                .refusal(true)
                .ends_with(aurora_i18n::tr!("script-queue-select-modifiable-scripts"))
        );
    }
}
