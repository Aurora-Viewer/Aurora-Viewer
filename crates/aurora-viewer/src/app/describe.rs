//! "Debug to log" / "Debug to clipboard" (Preferences › Debug › Logs, and
//! the Advanced menu of the wolf button): asks the names of the worn items the press details (an
//! object worn by another avatar has no name until ObjectProperties gives
//! it), then writes the scene's diagnostic (scene/describe.rs) once they
//! had time to come, and copies the same lines when asked to: a tester can
//! paste them in a message instead of sending their whole log.

use super::App;
use crate::scene::describe::DescribeScope;
use aurora_net::NetCommand;
use aurora_render::Renderer;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Time left for the names asked to come before the lines are written.
const NAMES_WAIT: Duration = Duration::from_secs(3);
/// Names asked per press at most (one RequestObjectPropertiesFamily each).
const NAMES_PER_PRESS: usize = 64;

/// Where a press sends its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescribeOutput {
    Log,
    /// The log, and the clipboard.
    Clipboard,
}

/// A press waiting for the names it asked.
pub(super) struct Pending {
    at: Instant,
    scope: DescribeScope,
    output: DescribeOutput,
}

/// `AURORA_DEMO_DESCRIBE=<frame>[,clipboard]`: the demo presses the button
/// at that frame.
fn demo_press() -> Option<(u64, DescribeOutput)> {
    static PRESS: std::sync::OnceLock<Option<(u64, DescribeOutput)>> = std::sync::OnceLock::new();
    *PRESS.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_DESCRIBE").ok()?;
        let (frame, output) = value.split_once(',').unwrap_or((&value, ""));
        let output = if output.trim() == "clipboard" {
            DescribeOutput::Clipboard
        } else {
            DescribeOutput::Log
        };
        Some((frame.trim().parse().ok()?, output))
    })
}

impl App {
    /// The name of an object from the properties the build tools or the
    /// object menu received.
    fn known_object_name(&self, id: &Uuid) -> Option<String> {
        self.build
            .props
            .get(id)
            .or_else(|| self.interactions.props.get(id))
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
    }

    /// A press: the selection or the avatars near are taken now, the names
    /// missing are asked, the lines follow in `finish_describe`.
    pub(super) fn start_describe(&mut self, output: DescribeOutput) {
        let selected: Vec<usize> = self.build.selection.iter().filter_map(|k| self.world.objects.index_of(k)).collect();
        let scope = self.scene.describe_scope(&self.world, &selected);
        let mut asked = 0;
        for root in scope.roots(&self.world) {
            if asked == NAMES_PER_PRESS {
                break;
            }
            let Some(o) = self.world.objects.get(root).filter(|o| !o.is_avatar()) else {
                continue;
            };
            let in_inventory = o
                .attachment_item_id()
                .is_some_and(|id| self.world.inventory.items.contains_key(&id));
            let (id, handle) = (o.full_id, o.key.region);
            if in_inventory || self.known_object_name(&id).is_some() {
                continue;
            }
            self.send(NetCommand::RequestObjectProperties { handle, object: id });
            asked += 1;
        }
        let wait = if asked > 0 { NAMES_WAIT } else { Duration::ZERO };
        log::info!("describe: {asked} object names asked, lines in {} s", wait.as_secs());
        self.options_ui.describe_note = Some(aurora_i18n::tr!("ui-options-describe-waiting").to_string());
        self.describe = Some(Pending {
            at: Instant::now() + wait,
            scope,
            output,
        });
    }

    /// Every frame: write the press due (and the demo's press).
    pub(super) fn finish_describe(&mut self, renderer: &Renderer) {
        if self.demo
            && let Some((frame, output)) = demo_press()
            && self.frame_count == frame
        {
            self.start_describe(output);
        }
        if self.describe.as_ref().is_none_or(|p| Instant::now() < p.at) {
            return;
        }
        let Some(pending) = self.describe.take() else {
            return;
        };
        let ((), text) = crate::logging::capture(|| {
            // scene space: the agent avatar's drawn center
            let me = self.scene.agent_idx.and_then(|i| self.scene.gpu.get(i)).map(|g| g.center);
            if let Some(me) = me {
                self.scene.log_nearby(&self.world, me, 12.0);
            }
            // what the avatars around wear: a missing part shows there
            let names = |id: &Uuid| self.known_object_name(id);
            self.scene.describe_worn(&self.world, renderer, &pending.scope, &names);
        });
        let note = match pending.output {
            DescribeOutput::Log => aurora_i18n::tr!("ui-options-describe-written").to_string(),
            DescribeOutput::Clipboard => {
                let lines = text.lines().count();
                log::info!("describe: {lines} lines copied to the clipboard");
                self.egui_ctx.copy_text(text);
                aurora_i18n::tr!("ui-options-describe-copied", lines = lines).to_string()
            }
        };
        self.options_ui.describe_note = Some(note);
    }
}
