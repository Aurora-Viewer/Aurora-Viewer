//! The Dev Toolbox's tools (dev_toolbox/, ui/dev_toolbox.rs): what each
//! button turns on, builds or opens, the live values its buttons show, and
//! "Send a diagnostic" (the header, the system configuration, the hidden
//! names; Copy, Save as .txt, Log). "Debug to log / clipboard" (J1, J2) wait
//! for the names of the worn items in app/describe.rs.

use super::App;
use crate::dev_toolbox::anonymize::{self, Resident};
use crate::dev_toolbox::tools::{self, Kind, Layout};
use crate::dev_toolbox::{self, Target};
use crate::ui::dev_toolbox::{Action, Health};
use aurora_i18n::tr;
use aurora_render::Renderer;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Live values are measured again this often while the window is open.
const LIVE_EVERY: Duration = Duration::from_millis(500);
/// The nearest avatar the per-target tools take (m), as "Debug to log".
const NEAR: f32 = crate::scene::describe::DETAIL_RADIUS;
/// Slow frames (P3) are those whose scene sync takes longer than this (ms).
const SLOW_SYNC_MS: f32 = 30.0;
/// J5 copies the diagnostics' lines of this last period.
const TOOL_OUTPUT_FOR: Duration = Duration::from_secs(60);
/// J4's machine line while sysinfo reads it (J4 is built again after).
const MACHINE_PENDING: &str = "OS: (reading…)";

/// The app's side of the Dev Toolbox.
#[derive(Default)]
pub(super) struct Toolbox {
    view: crate::ui::dev_toolbox::View,
    /// The text of each tool's last result, without its header.
    results: HashMap<&'static str, String>,
    /// The tool "Debug to log / clipboard" fills once the names came.
    waiting: Option<&'static str>,
    /// J2 copies its result as soon as it is ready.
    copy_when_ready: bool,
    last_live: Option<Instant>,
    /// The surface last pointed at in the world: (object slot, face).
    hover: Option<(usize, Option<usize>)>,
    /// Aim: the surface under the cursor no longer follows the pointer.
    frozen: bool,
    /// Aim: the next press on the world picks the surface under it.
    picking: bool,
    /// That press came: the next frame takes its surface.
    pick_now: bool,
    /// What does not change during a session (OS, processor, memory).
    machine: Option<crate::system_info::Reading>,
    /// Bumped with every result, so the preview is composed again only when
    /// something it shows changed.
    generation: u64,
    preview_key: Option<(&'static str, u64, bool, bool, bool)>,
    /// AURORA_DEMO_TOOLBOX="target:tool": the tool pressed once the world shows.
    pub(super) demo_press: Option<&'static str>,
}

impl Toolbox {
    /// A result of app/describe.rs arrived.
    pub(super) fn set_result(&mut self, tool: &'static str, text: String) {
        self.put(tool, text);
        if self.waiting == Some(tool) {
            self.waiting = None;
        }
    }

    /// J1 or J2 asked the names: its text follows (and J2 copies it).
    pub(super) fn begin(&mut self, tool: &'static str) {
        if self.results.remove(tool).is_some() {
            self.generation += 1;
        }
        self.waiting = Some(tool);
        self.copy_when_ready = tool == "J2";
    }

    fn put(&mut self, tool: &'static str, text: String) {
        if self.results.get(tool) != Some(&text) {
            self.results.insert(tool, text);
            self.generation += 1;
        }
    }
}

impl App {
    /// Every frame, after the world hover: the surface under the cursor
    /// for L1 and the "Under the cursor" target, kept when the pointer
    /// leaves the world for the window.
    pub(super) fn track_toolbox_hover(&mut self, renderer: &Renderer) {
        if !self.panels.dev_toolbox || self.toolbox.frozen {
            return;
        }
        let pick = std::mem::take(&mut self.toolbox.pick_now);
        let Some((Some(idx), _)) = self.look_hover else {
            return;
        };
        if !pick && self.toolbox.hover.is_some_and(|(i, _)| i == idx) && !self.frame_count.is_multiple_of(6) {
            return;
        }
        let face = renderer
            .cursor_ray(self.cursor_pos.0, self.cursor_pos.1)
            .and_then(|ray| self.scene.touch_surface(&self.world, idx, ray))
            .and_then(|s| usize::try_from(s.face).ok());
        self.toolbox.hover = Some((idx, face));
        if pick {
            self.toolbox.picking = false;
            self.toolbox.frozen = true;
        }
    }

    /// Aim: a press on the world while it waits picks the surface under the
    /// pointer for the toolbox and does nothing else (no touch, no walk);
    /// true when it took the press. Pointing at a surface and then reaching
    /// the window's button would cross the world and change the target.
    pub(super) fn toolbox_pick_press(&mut self) -> bool {
        if !self.panels.dev_toolbox || !self.toolbox.picking {
            return false;
        }
        self.toolbox.pick_now = true;
        true
    }

    /// The window, drawn with the interface; its actions run at once.
    pub(super) fn dev_toolbox_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, renderer: &Renderer) {
        if !self.panels.dev_toolbox {
            return;
        }
        if self.demo
            && self.frame_count >= 600
            && let Some(id) = self.toolbox.demo_press.take()
        {
            self.press_tool(id, renderer);
        }
        if self.toolbox.machine.is_none() {
            let _ = self.system_text(renderer);
        }
        if self.toolbox.last_live.is_none_or(|t| t.elapsed() >= LIVE_EVERY) {
            self.toolbox.last_live = Some(Instant::now());
            self.refresh_toolbox_view(renderer);
        }
        self.compose_preview();
        // J2 copies its text as soon as the names came
        if self.toolbox.copy_when_ready && self.toolbox.waiting.is_none() {
            self.toolbox.copy_when_ready = false;
            if self.dev_toolbox.selected == "J2"
                && let Some(text) = self.toolbox.view.preview.clone()
            {
                self.copy_toolbox_text(text);
                self.dev_toolbox.copied_at = Some(Instant::now());
            }
        }
        let mut layout = Layout::normalized(self.settings.dev_toolbox.as_ref());
        let mut open = self.panels.dev_toolbox;
        let (actions, changed) = crate::ui::dev_toolbox::show(ctx, p, &mut self.dev_toolbox, &mut layout, &self.toolbox.view, &mut open);
        self.panels.dev_toolbox = open;
        if changed {
            self.settings.dev_toolbox = Some(layout);
            self.settings.save();
        }
        for action in actions {
            self.toolbox_action(action, renderer);
        }
    }

    fn toolbox_action(&mut self, action: Action, renderer: &Renderer) {
        match action {
            Action::Press(id) => self.press_tool(id, renderer),
            Action::Copy => {
                if let Some(text) = self.toolbox.view.preview.clone() {
                    self.copy_toolbox_text(text);
                }
            }
            Action::Save => {
                if let Some(text) = self.toolbox.view.preview.clone() {
                    let secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs());
                    let name = dev_toolbox::file_name(self.dev_toolbox.selected, secs);
                    let title = tr!("dev-toolbox-save").to_string();
                    // the dialog waits for the user: never on the frame's thread
                    std::thread::spawn(move || {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_title(title)
                            .set_file_name(&name)
                            .add_filter("Text", &["txt"])
                            .save_file()
                        {
                            match std::fs::write(&path, text) {
                                Ok(()) => log::info!("dev toolbox: saved {}", path.display()),
                                Err(e) => log::warn!("dev toolbox: could not save {}: {e}", path.display()),
                            }
                        }
                    });
                }
            }
            Action::Log => {
                if let Some(text) = &self.toolbox.view.preview {
                    log::info!("dev toolbox {}:\n{text}", self.dev_toolbox.selected);
                }
            }
            Action::Aim => {
                // waiting or frozen: back to following the pointer
                let follow = self.toolbox.frozen || self.toolbox.picking;
                self.toolbox.frozen = false;
                self.toolbox.picking = !follow;
            }
            Action::Health(h) => {
                use crate::ui::context::CtxAction;
                let me = self.world.agent_id;
                self.on_ctx_action(match h {
                    Health::StopAnimations => CtxAction::StopAnimationsKeepPermissions,
                    Health::Undeform => CtxAction::Undeform,
                    Health::Rebake => CtxAction::Rebake,
                    Health::RefreshAttachments => CtxAction::RefreshAttachments,
                    Health::ResetSkeleton => CtxAction::ResetSkeleton {
                        avatar: me,
                        animations: false,
                    },
                });
            }
        }
    }

    fn copy_toolbox_text(&mut self, text: String) {
        log::info!("dev toolbox: {} copied ({} lines)", self.dev_toolbox.selected, text.lines().count());
        self.egui_ctx.copy_text(text);
    }

    /// A tool was pressed.
    fn press_tool(&mut self, id: &'static str, renderer: &Renderer) {
        let Some(tool) = tools::find(id) else { return };
        match tool.kind {
            Kind::Toggle => self.flip_toggle(id),
            Kind::Copy | Kind::Live => self.build_result(id, renderer),
            Kind::Window => self.open_tool_window(id),
        }
    }

    /// Whether a toggle is on.
    fn toggle_on(&self, id: &str) -> bool {
        let d = &self.settings.debug;
        match id {
            "R1" => d.show_alpha,
            "R2" => d.show_alpha_rigged,
            "R3" => d.pass_view,
            "R4" => d.wireframe,
            "R5" => d.glow_view,
            "R6" => d.shadow_view,
            "R7" => d.ao_view,
            "R8" => d.glow_objects,
            "R9" => d.lights,
            "R10" => d.probes,
            "R11" => d.check_light_sets,
            "R12" => d.check_probe_sets,
            "O1" => d.bounds,
            "O2" => d.lod_info,
            "O3" => d.culling,
            "O4" => d.freeze_culling,
            "O5" => d.skeletons,
            "O6" => d.complexity_tags,
            "O9" => self.scene.sculpt_watch_target().is_some(),
            "P3" => self.scene.slow_sync().is_some(),
            "P4" => self.scene.still.is_some(),
            "P5" => self.scene.sync_check_on(),
            "P6" => self.frame_profile.enabled(),
            "M1" => crate::media::debug_on(),
            _ => false,
        }
    }

    fn flip_toggle(&mut self, id: &'static str) {
        let on = !self.toggle_on(id);
        let before = self.settings.debug;
        let d = &mut self.settings.debug;
        match id {
            "R1" => d.show_alpha = on,
            // the rigged faces only show with the transparency overlay
            "R2" => {
                d.show_alpha_rigged = on;
                d.show_alpha |= on;
            }
            "R3" => d.pass_view = on,
            "R4" => d.wireframe = on,
            "R5" => d.glow_view = on,
            "R6" => d.shadow_view = on,
            "R7" => d.ao_view = on,
            "R8" => d.glow_objects = on,
            "R9" => d.lights = on,
            "R10" => d.probes = on,
            "R11" => d.check_light_sets = on,
            "R12" => d.check_probe_sets = on,
            "O1" => d.bounds = on,
            "O2" => d.lod_info = on,
            "O3" => d.culling = on,
            "O4" => d.freeze_culling = on,
            "O5" => d.skeletons = on,
            "O6" => d.complexity_tags = on,
            "O9" => {
                let target = on.then(|| self.target_avatar()).flatten();
                if on && target.is_none() {
                    self.world.system_message(tr!("dev-toolbox-no-avatar-target"));
                }
                self.scene.set_sculpt_watch(target);
            }
            "P3" => self.scene.set_slow_sync(on.then_some(SLOW_SYNC_MS)),
            "P4" => self.scene.set_still_check(on),
            "P5" => self.scene.set_sync_check(on),
            "P6" => {
                self.frame_profile = crate::frame_profile::FrameProfile::new(on);
                self.net_profile = crate::net_profile::NetProfile::new(on);
            }
            "M1" => crate::media::set_debug(on),
            _ => {}
        }
        self.settings.debug.keep_one_view(&before);
        log::info!("dev toolbox: {id} {}", if on { "on" } else { "off" });
    }

    fn open_tool_window(&mut self, id: &'static str) {
        let ctx = self.egui_ctx.clone();
        match id {
            "J6" => crate::logging::open_logs_dir(),
            "J7" => self.script_editors.open_debug(),
            "O7" => match self.target_object() {
                Some(TargetObject::Avatar(avatar)) => self.inspect_avatar(avatar),
                Some(TargetObject::Object(object)) => self.inspect_object(object),
                None => self.world.system_message(tr!("dev-toolbox-no-target")),
            },
            "N1" => {
                crate::ui::widgets::open_floater(&ctx, "perf", &mut self.panels.perf);
                crate::ui::perf::show_network(&ctx);
            }
            "P1" => {
                crate::ui::perf::set_full(&ctx, false);
                crate::ui::widgets::open_floater(&ctx, "perf", &mut self.panels.perf);
            }
            "P2" => {
                crate::ui::perf::set_full(&ctx, true);
                crate::ui::widgets::open_floater(&ctx, "perf", &mut self.panels.perf);
            }
            _ => {}
        }
    }

    /// The scene slots "Debug to log" details for the current target.
    fn target_slots(&self) -> Vec<usize> {
        match self.dev_toolbox.target {
            Target::Nearest => Vec::new(),
            Target::Selection => self.build.selection.iter().filter_map(|k| self.world.objects.index_of(k)).collect(),
            Target::UnderCursor => self.toolbox.hover.map(|(i, _)| vec![i]).unwrap_or_default(),
        }
    }

    /// The avatar the target stands for: the nearest one, or the wearer of
    /// the selected or pointed-at object (or that avatar itself).
    fn target_avatar(&self) -> Option<Uuid> {
        match self.dev_toolbox.target {
            Target::Nearest => crate::scene::Scene::avatars_near(&self.world, NEAR).first().map(|(_, id)| *id),
            _ => self.target_slots().into_iter().find_map(|i| self.wearer_of(i)),
        }
    }

    fn wearer_of(&self, mut idx: usize) -> Option<Uuid> {
        for _ in 0..16 {
            let o = self.world.objects.get(idx)?;
            if o.is_avatar() {
                return Some(o.full_id);
            }
            idx = self.world.objects.parent_of(o)?;
        }
        None
    }

    fn target_object(&self) -> Option<TargetObject> {
        match self.dev_toolbox.target {
            Target::Nearest => self.target_avatar().map(TargetObject::Avatar),
            _ => {
                let idx = *self.target_slots().first()?;
                let o = self.world.objects.get(idx)?;
                Some(if o.is_avatar() {
                    TargetObject::Avatar(o.full_id)
                } else {
                    TargetObject::Object(o.full_id)
                })
            }
        }
    }

    /// Build the text of a copy or a live view.
    fn build_result(&mut self, id: &'static str, renderer: &Renderer) {
        let text = match id {
            "J1" | "J2" => {
                if self.dev_toolbox.target == Target::Selection && self.build.selection.is_empty() {
                    Some(tr!("dev-toolbox-target-selection-none").to_string())
                } else {
                    self.toolbox.begin(id);
                    let slots = self.target_slots();
                    self.start_describe(id, slots);
                    None
                }
            }
            "J3" => Some(warnings_text(50)),
            "J4" => Some(self.system_text(renderer)),
            "J5" => Some(tool_output_text()),
            "J8" => {
                let edit = self.ik_editor.request();
                Some(
                    self.world
                        .describe_ik(edit.active, edit.suppress_script_ik, edit.goals.len(), Instant::now()),
                )
            }
            "N2" => Some(self.region_text()),
            "L1" => Some(self.cursor_text(renderer)),
            "L2" => Some(self.worn_text()),
            "L3" => Some(warnings_text(20)),
            "N3" => Some(self.loads_text(renderer)),
            _ => None,
        };
        if let Some(text) = text {
            self.toolbox.put(id, text);
        }
    }

    /// "Send a diagnostic": header, system configuration, the text, names
    /// hidden.
    fn compose_preview(&mut self) {
        let id = self.dev_toolbox.selected;
        let key = (
            id,
            self.toolbox.generation,
            self.dev_toolbox.hide_names,
            self.dev_toolbox.with_system,
            self.toolbox.waiting == Some(id),
        );
        if self.toolbox.preview_key == Some(key) {
            return;
        }
        self.toolbox.preview_key = Some(key);
        let view = &mut self.toolbox.view;
        if self.toolbox.waiting == Some(id) {
            view.preview = None;
            view.waiting = true;
            return;
        }
        view.waiting = false;
        let Some(body) = self.toolbox.results.get(id) else {
            view.preview = None;
            return;
        };
        let per_target = matches!(id, "J1" | "J2" | "L1" | "L2");
        let mut text = dev_toolbox::header(
            aurora_version::version(),
            id,
            per_target.then_some(self.dev_toolbox.target),
            self.dev_toolbox.hide_names,
        );
        text.push('\n');
        let mut rest = String::new();
        if self.dev_toolbox.with_system
            && id != "J4"
            && let Some(system) = self.toolbox.results.get("J4")
        {
            rest.push_str(system);
            if !system.ends_with('\n') {
                rest.push('\n');
            }
        }
        // the faces of a mesh body name the same textures again and again
        if matches!(id, "J1" | "J2") {
            rest.push_str(&dev_toolbox::legend::shorten(body));
        } else {
            rest.push_str(body);
        }
        // the header names the build: only what follows is anonymised
        if self.dev_toolbox.hide_names {
            rest = anonymize::anonymize(&rest, &self.residents(), Some(self.world.agent_id));
        }
        text.push_str(&rest);
        self.toolbox.view.preview = Some(text);
    }

    /// The residents a copy may name: the avatars in the scene and us.
    fn residents(&self) -> Vec<Resident> {
        let mut out = Vec::new();
        for (_, o) in self.world.objects.avatars() {
            let mut names = Vec::new();
            if let Some(n) = self.world.social.avatar_names.get(&o.full_id) {
                names.push(format!("{} ({})", n.display_name, n.username));
                names.push(n.display_name.clone());
                names.push(n.username.clone());
                names.push(format!("{} {}", n.legacy_first, n.legacy_last));
            }
            names.push(self.world.social.name_of(&o.full_id));
            if let Some(legacy) = o.display_name() {
                names.push(legacy);
            }
            names.retain(|n| n.trim().chars().count() >= 3 && n != "…");
            out.push(Resident { id: o.full_id, names });
        }
        out
    }

    /// The window's live values, the target line and the last warnings.
    fn refresh_toolbox_view(&mut self, renderer: &Renderer) {
        let ids: Vec<&'static str> = tools::TOOLS.iter().filter(|t| t.kind == Kind::Toggle).map(|t| t.id).collect();
        let on = ids.into_iter().filter(|id| self.toggle_on(id)).collect();
        let mut live: HashMap<&'static str, String> = HashMap::new();
        let warnings = crate::logging::recent_warnings(200);
        let recent = warnings.iter().filter(|w| w.at.elapsed() < Duration::from_secs(120)).count();
        live.insert("J3", tr!("dev-toolbox-live-warnings", count = recent));
        live.insert("L3", tr!("dev-toolbox-live-warnings-total", count = warnings.len()));
        let lines = crate::logging::tool_lines(TOOL_OUTPUT_FOR).len();
        live.insert("J5", tr!("dev-toolbox-live-lines", count = lines));
        if let Some((idx, face)) = self.toolbox.hover {
            let brief = face.and_then(|f| self.scene.face_brief(renderer, idx, f));
            live.insert(
                "L1",
                match brief {
                    Some((pass, alpha)) => format!("{pass:?} · alpha {alpha:.3}"),
                    None => tr!("dev-toolbox-live-not-drawn").to_string(),
                },
            );
        } else {
            live.insert("L1", tr!("dev-toolbox-live-nothing-pointed").to_string());
        }
        if let Some(avatar) = self.target_avatar() {
            let names = |id: &Uuid| self.known_object_name(id);
            let left: usize = self.scene.worn_rows(&self.world, avatar, &names).iter().map(|r| r.left_out).sum();
            live.insert("L2", tr!("dev-toolbox-live-left-out", count = left));
        }
        let used_mb = renderer.textures.page_bytes() / (1024 * 1024);
        let target_mb = self.scene.textures.stats.target_bytes / (1024 * 1024);
        live.insert("N3", format!("{used_mb} / {target_mb} MB"));
        let loss = crate::net_profile::loss_percent(&self.net_profile.last);
        live.insert(
            "N1",
            format!("{:.0} kB/s · {loss:.0} %", (self.perf.net_rate.2 + self.perf.net_rate.3) / 8.0),
        );
        let worst = self.perf.frame_ms.iter().copied().fold(0.0f32, f32::max);
        live.insert(
            "P1",
            tr!(
                "dev-toolbox-live-frame",
                ms = format!("{:.1}", self.perf.frames.avg),
                worst = format!("{worst:.0}")
            ),
        );
        live.insert("P3", tr!("dev-toolbox-live-threshold", ms = SLOW_SYNC_MS as u32));
        live.insert("O7", tr!("dev-toolbox-live-on-target").to_string());
        live.insert("O9", tr!("dev-toolbox-live-of-target").to_string());
        live.insert("O8", tr!("dev-toolbox-live-health").to_string());
        let view = &mut self.toolbox.view;
        view.on = on;
        view.live = live;
        view.frozen = self.toolbox.frozen;
        view.picking = self.toolbox.picking;
        view.shortcut = self.settings.keybinds.get(crate::keybinds::Action::DevToolbox)[0].clone();
        view.warnings = warnings
            .iter()
            .rev()
            .take(2)
            .rev()
            .map(|w| {
                (
                    w.clock.clone(),
                    w.level == log::Level::Error,
                    w.text.lines().next().unwrap_or("").to_owned(),
                )
            })
            .collect();
        let target = self.target_text();
        self.toolbox.view.target_text = if self.dev_toolbox.hide_names {
            anonymize::anonymize(&target, &self.residents(), Some(self.world.agent_id))
        } else {
            target
        };
        // J4 once the machine was read, and while it is attached to the copies
        let machine_ready = self.toolbox.machine.as_ref().is_some_and(|m| m.get().is_some());
        let stale = self.toolbox.results.get("J4").is_some_and(|t| t.contains(MACHINE_PENDING)) && machine_ready;
        if stale || (self.dev_toolbox.with_system && !self.toolbox.results.contains_key("J4")) {
            let text = self.system_text(renderer);
            self.toolbox.put("J4", text);
        }
        // a live view shown in "Send a diagnostic" follows
        let selected = self.dev_toolbox.selected;
        if tools::find(selected).is_some_and(|t| t.kind == Kind::Live) {
            self.build_result(selected, renderer);
        }
    }

    /// What the target bar names.
    fn target_text(&self) -> String {
        match self.dev_toolbox.target {
            Target::Nearest => match crate::scene::Scene::avatars_near(&self.world, NEAR).first() {
                Some((d, id)) => {
                    let worn = self
                        .world
                        .objects
                        .index_of_uuid(id)
                        .and_then(|i| self.world.objects.get(i))
                        .map_or(0, |o| self.world.objects.children_of(&o.key).len());
                    tr!(
                        "dev-toolbox-target-avatar",
                        name = self.world.social.display_name_of(id),
                        distance = format!("{d:.1}"),
                        worn = worn
                    )
                }
                None => tr!("dev-toolbox-target-nearest-none").to_string(),
            },
            Target::Selection => {
                let slots = self.target_slots();
                match slots.first().and_then(|&i| self.world.objects.get(i)) {
                    Some(o) => {
                        let name = self.known_object_name(&o.full_id).unwrap_or_else(|| o.full_id.to_string());
                        tr!("dev-toolbox-target-selected", count = slots.len(), name = name)
                    }
                    None => tr!("dev-toolbox-target-selection-none").to_string(),
                }
            }
            Target::UnderCursor => match self.toolbox.hover.and_then(|(i, f)| Some((self.world.objects.get(i)?, f))) {
                Some((o, face)) => {
                    let kind = if o.visual_volume().is_mesh() { "mesh" } else { "prim" };
                    let face = face.map_or_else(|| "?".to_string(), |f| f.to_string());
                    tr!("dev-toolbox-target-surface", id = short(&o.full_id), kind = kind, face = face)
                }
                None => tr!("dev-toolbox-target-cursor-none").to_string(),
            },
        }
    }

    /// J4: what the build runs on.
    fn system_text(&mut self, renderer: &Renderer) -> String {
        let machine = self.toolbox.machine.get_or_insert_with(crate::system_info::Reading::start).clone();
        let machine = machine.get().map_or_else(
            || MACHINE_PENDING.to_owned(),
            |m| format!("OS: {} · CPU: {} · RAM {} GB", m.os, m.cpu, m.memory_mb.div_ceil(1024)),
        );
        let info = &renderer.info;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Aurora {} (commit {}, {} build)",
            aurora_version::version(),
            aurora_version::commit(),
            env!("AURORA_BUILD_PROFILE")
        );
        let _ = writeln!(out, "{machine}");
        let _ = writeln!(
            out,
            "GPU: {} ({}, driver {}, {} MB VRAM, {})",
            info.name,
            info.backend,
            info.driver,
            info.vram_mb.map_or_else(|| "?".to_string(), |v| v.to_string()),
            if info.integrated { "integrated" } else { "discrete" }
        );
        let (w, h) = renderer.size();
        let _ = writeln!(out, "Screen: {w}×{h} at {:.0} Hz", self.screen_rate.hz());
        let aa = ["none", "FXAA", "MSAA 2x", "MSAA 4x", "MSAA 8x", "TAA", "SMAA"]
            .get(self.settings.antialiasing as usize)
            .copied()
            .unwrap_or("?");
        let _ = writeln!(
            out,
            "Graphics: shadows {}, AA {aa}, draw distance {:.0} m, texture budget {} MB",
            if self.settings.shadows { "on" } else { "off" },
            self.settings.draw_distance,
            self.settings.texture_budget(info.vram_mb, info.integrated)
        );
        let in_world = self.move_complete_at.map_or(0, |t| t.elapsed().as_secs() / 60);
        let _ = writeln!(out, "Language: {} · in world for {in_world} min", aurora_i18n::language());
        out
    }

    /// N2: the region's state, without its name nor any address.
    fn region_text(&self) -> String {
        use aurora_net::estate::sim_stat;
        let mut out = String::new();
        match &self.world.estate.stats {
            Some((handle, stats)) => {
                let version = self.world.estate.versions.get(handle).map_or("unknown", String::as_str);
                let _ = writeln!(out, "simulator: {version}");
                let stat = |id| stats.get(id).map_or_else(|| "?".to_string(), |v| format!("{v:.2}"));
                let _ = writeln!(
                    out,
                    "time dilation {} · sim fps {} · physics fps {}",
                    stat(sim_stat::TIME_DILATION),
                    stat(sim_stat::FPS),
                    stat(sim_stat::PHYSFPS)
                );
                let count = |id| stats.get(id).map_or_else(|| "?".to_string(), |v| format!("{v:.0}"));
                let _ = writeln!(
                    out,
                    "avatars {} (+{} near) · objects {} · active scripts {}",
                    count(sim_stat::NUMAGENTMAIN),
                    count(sim_stat::NUMAGENTCHILD),
                    count(sim_stat::NUMTASKS),
                    count(sim_stat::NUMSCRIPTSACTIVE)
                );
            }
            None => {
                let _ = writeln!(out, "simulator statistics not received yet");
            }
        }
        let neighbours = self.net.stats.simulators.lock().len();
        let silence = self.net.stats.simulator_motion.lock().packet_age();
        let _ = writeln!(out, "simulators connected {neighbours} · last packet {silence:.1} s ago");
        let loss = crate::net_profile::loss_percent(&self.net_profile.last);
        let ping = self.net.stats.ping_ms.load(std::sync::atomic::Ordering::Relaxed);
        let _ = writeln!(out, "ping {ping} ms · loss {loss:.1} %");
        out
    }

    /// L1: the surface last pointed at, every face of its prim.
    fn cursor_text(&self, renderer: &Renderer) -> String {
        match self.toolbox.hover {
            Some((idx, face)) => {
                let mut out = String::new();
                if let Some(avatar) = self.wearer_of(idx) {
                    let _ = writeln!(out, "worn by '{}' ({avatar})", self.world.social.name_of(&avatar));
                }
                let _ = writeln!(out, "face pointed at: {}", face.map_or_else(|| "?".to_string(), |f| f.to_string()));
                out.push_str(&self.scene.describe_prim(&self.world, renderer, idx).unwrap_or_default());
                out.push('\n');
                out
            }
            None => tr!("dev-toolbox-target-cursor-none").to_string(),
        }
    }

    /// L2: what the target avatar wears, the most left out first.
    fn worn_text(&self) -> String {
        let Some(avatar) = self.target_avatar() else {
            return tr!("dev-toolbox-no-avatar-target").to_string();
        };
        let names = |id: &Uuid| self.known_object_name(id);
        let rows = self.scene.worn_rows(&self.world, avatar, &names);
        let mut out = format!(
            "worn by '{}' ({avatar}): {} attachments\n",
            self.world.social.name_of(&avatar),
            rows.len()
        );
        for r in rows {
            let _ = writeln!(
                out,
                "'{}' {} point {}: {} prims, {} faces drawn, {} left out",
                r.name.as_deref().unwrap_or("?"),
                r.root,
                r.point,
                r.prims,
                r.faces_drawn,
                r.left_out
            );
        }
        out
    }

    /// N3: textures, meshes, materials and downloads.
    fn loads_text(&self, renderer: &Renderer) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let t = &self.scene.textures.stats;
        let mb = |b: u64| b / (1024 * 1024);
        let net = &self.net.stats;
        let (m_ready, m_fetching, m_failed) = self.scene.meshes.counts();
        let (g_loaded, g_pending, g_missing) = self.scene.materials.counts();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "texture memory {} / {} MB (pages {} MB), pressure {}, reduced {}",
            mb(renderer.textures.bytes()),
            mb(t.target_bytes),
            mb(renderer.textures.page_bytes()),
            t.pressure,
            t.reduced
        );
        let _ = writeln!(
            out,
            "textures {} in use: {} loaded, {} downloading, {} decoding, {} to upload, {} failing",
            t.total, t.loaded, t.fetching, t.decoding, t.pending_upload, t.failing
        );
        let _ = writeln!(out, "meshes {m_ready} ready, {m_fetching} downloading, {m_failed} failed");
        let _ = writeln!(out, "glTF materials {g_loaded} loaded, {g_pending} pending, {g_missing} missing");
        let _ = writeln!(
            out,
            "downloads {} queued, {} in flight, {} failed since login",
            net.http_queued.load(Relaxed),
            net.http_in_flight.load(Relaxed),
            net.http_failed.load(Relaxed)
        );
        out
    }
}

/// O7's target.
enum TargetObject {
    Avatar(Uuid),
    Object(Uuid),
}

/// The start of an id, enough to tell objects apart on screen.
fn short(id: &Uuid) -> String {
    let s = id.to_string();
    format!("{}…{}", &s[..4], &s[s.len() - 4..])
}

/// J3 / L3: the last warnings and errors.
fn warnings_text(n: usize) -> String {
    let warnings = crate::logging::recent_warnings(n);
    if warnings.is_empty() {
        return tr!("dev-toolbox-no-warning").to_string();
    }
    let mut out = String::new();
    for w in warnings {
        let level = if w.level == log::Level::Error { "ERROR" } else { "WARN" };
        let _ = writeln!(out, "{} {level} {}", w.clock, w.text);
    }
    out
}

/// J5: what the diagnostics turned on wrote in the last minute.
fn tool_output_text() -> String {
    let lines = crate::logging::tool_lines(TOOL_OUTPUT_FOR);
    if lines.is_empty() {
        return tr!("dev-toolbox-no-tool-output").to_string();
    }
    let mut out = String::new();
    for (tool, line) in lines {
        let _ = writeln!(out, "{} [{tool}] {}", line.clock, line.text);
    }
    out
}
