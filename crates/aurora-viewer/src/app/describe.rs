//! The Dev Toolbox's "Debug to log" (J1) and "Debug to clipboard" (J2):
//! asks the names of the worn items the press details (an object worn by
//! another avatar has no name until ObjectProperties gives it), then writes
//! the scene's diagnostic (scene/describe.rs) once they had time to come,
//! and hands the same lines to the toolbox (app/dev_toolbox.rs), which shows
//! them in "Send a diagnostic" and, for J2, copies them: a tester can paste
//! them in a message instead of sending their whole log.

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

/// A press waiting for the names it asked.
pub(super) struct Pending {
    at: Instant,
    scope: DescribeScope,
    /// J1 or J2.
    tool: &'static str,
}

/// `AURORA_DEMO_DESCRIBE=<frame>[,clipboard]`: the demo opens the Dev
/// Toolbox and presses J1 (J2 with `clipboard`) at that frame.
fn demo_press() -> Option<(u64, &'static str)> {
    static PRESS: std::sync::OnceLock<Option<(u64, &'static str)>> = std::sync::OnceLock::new();
    *PRESS.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_DESCRIBE").ok()?;
        let (frame, output) = value.split_once(',').unwrap_or((&value, ""));
        let tool = if output.trim() == "clipboard" { "J2" } else { "J1" };
        Some((frame.trim().parse().ok()?, tool))
    })
}

impl App {
    /// The name of an object from the properties the build tools or the
    /// object menu received.
    pub(super) fn known_object_name(&self, id: &Uuid) -> Option<String> {
        self.build
            .props
            .get(id)
            .or_else(|| self.interactions.props.get(id))
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
    }

    /// A press: `selected` (the scene slots of the target; empty: the
    /// avatars within 3 m) is taken now, the names missing are asked, the
    /// lines follow in `finish_describe`.
    pub(super) fn start_describe(&mut self, tool: &'static str, selected: Vec<usize>) {
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
        self.describe = Some(Pending {
            at: Instant::now() + wait,
            scope,
            tool,
        });
    }

    /// Every frame: write the press due (and the demo's press).
    pub(super) fn finish_describe(&mut self, renderer: &Renderer) {
        if self.demo
            && let Some((frame, tool)) = demo_press()
            && self.frame_count == frame
        {
            crate::ui::widgets::open_floater(&self.egui_ctx, crate::ui::dev_toolbox::WINDOW, &mut self.panels.dev_toolbox);
            self.dev_toolbox.selected = tool;
            self.toolbox.begin(tool);
            self.start_describe(tool, Vec::new());
        }
        if self.describe.as_ref().is_none_or(|p| Instant::now() < p.at) {
            return;
        }
        let Some(pending) = self.describe.take() else {
            return;
        };
        let ((), text) = crate::logging::capture(|| {
            // scene space: the agent avatar's drawn center; the objects
            // around only when the press has no target (a copy with a
            // target stays short enough to paste)
            let me = self.scene.agent_idx.and_then(|i| self.scene.gpu.get(i)).map(|g| g.center);
            if let Some(me) = me.filter(|_| pending.scope.is_empty()) {
                self.scene.log_nearby(&self.world, me, 12.0);
            }
            // what the avatars around wear: a missing part shows there
            let names = |id: &Uuid| self.known_object_name(id);
            self.scene.describe_worn(&self.world, renderer, &pending.scope, &names);
        });
        log::info!("describe: {} lines for {}", text.lines().count(), pending.tool);
        self.toolbox.set_result(pending.tool, text);
    }
}
