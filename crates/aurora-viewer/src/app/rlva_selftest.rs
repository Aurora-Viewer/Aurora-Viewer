//! The RLVa self-test in the application (world/rlva_selftest.rs): the Dev
//! Toolbox tool and its confirmation, the frame step, and the hand-over of
//! what the runner asks for to the same send and receive paths as the
//! interface's own commands and the session's own events.
use super::*;
use crate::ui::rlva_selftest::{Action as UiAction, SelfTestUi};
use crate::world::rlva_selftest::{self as selftest, Mode, Out, Runner, sim::Sim};

/// The identifier of the tool in the Dev Toolbox.
pub(super) const TOOL: &str = "T1";

#[derive(Default)]
pub(super) struct SelfTest {
    pub(super) runner: Runner,
    /// What a region answers to the test's own commands in demo mode.
    sim: Sim,
    ui: SelfTestUi,
    /// The report as the window shows and copies it: the build first,
    /// residents' names hidden.
    text: Option<String>,
    /// AURORA_DEMO_RLVA=selftest: 1 once the dialog is asked for, 2 once it
    /// is answered.
    demo_step: u8,
}

impl App {
    /// The tool was pressed: its window, and the dialog that asks for a run.
    pub(super) fn request_rlva_selftest(&mut self) {
        if !selftest::available() || !self.in_world() {
            return;
        }
        self.rlva_selftest.runner.request();
        let ctx = self.egui_ctx.clone();
        crate::ui::widgets::open_floater(&ctx, "rlva_selftest", &mut self.rlva_selftest.ui.open);
    }

    /// One received event, the self-test first: a relay's handshake and the
    /// answers to the test's own inventory requests go no further.
    pub(super) fn receive_event(&mut self, event: NetEvent, sent: Instant) {
        if self.rlva_selftest.runner.event(&mut self.world, &event) {
            return;
        }
        if let Some(event) = self.bridge_runtime.receive(&mut self.world, event, Instant::now(), sent) {
            self.on_app_event(event);
        }
    }

    /// Demo mode: what the region would answer to a command of the test
    /// that the demo's own answers do not cover. True when it was answered.
    pub(super) fn rlva_selftest_demo_command(&mut self, command: &NetCommand) -> bool {
        if !self.rlva_selftest.runner.running() {
            return false;
        }
        let Some(events) = self.rlva_selftest.sim.reply(&mut self.world, command, false) else {
            return false;
        };
        for event in events {
            self.receive_event(event, Instant::now());
        }
        true
    }

    pub(super) fn update_rlva_selftest(&mut self) {
        self.demo_rlva_selftest();
        if !self.rlva_selftest.runner.running() {
            return;
        }
        let now = Instant::now();
        let online = self.in_world();
        self.rlva_selftest.runner.step(&mut self.world, online, now);
        for out in self.rlva_selftest.runner.take_out() {
            match out {
                Out::Send(command) => self.send(command),
                // (the boundary's answer comes back from `send_from`)
                Out::Probe(command) => {
                    self.rlva_selftest.runner.probe(true);
                    self.send(command);
                    self.rlva_selftest.runner.probe(false);
                }
                Out::Hear(event) => self.receive_event(event, now),
                Out::Fly(on) => {
                    if self.world.agent.flying != on {
                        self.toggle_fly();
                    }
                }
            }
        }
        if self.rlva_selftest.runner.running() {
            return;
        }
        if let Some(report) = self.rlva_selftest.runner.report() {
            let mut text = crate::dev_toolbox::header(aurora_version::version(), TOOL, None, true);
            text.push('\n');
            text.push_str(&crate::dev_toolbox::anonymize::anonymize(
                &report.text(),
                &self.residents(),
                Some(self.world.agent_id),
            ));
            self.rlva_selftest.text = Some(text);
        }
    }

    /// The confirmation dialog and the tool's window.
    pub(super) fn rlva_selftest_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if !selftest::available() {
            return;
        }
        if self.rlva_selftest.runner.confirming() {
            let mut phase2 = self.rlva_selftest.runner.phase2;
            let answer = crate::ui::rlva_selftest::confirm(ctx, p, &mut phase2, self.rlva_switch.running());
            self.rlva_selftest.runner.phase2 = phase2;
            match answer {
                Some(UiAction::Start) if self.in_world() => {
                    // no script engine in demo mode: the relays are simulated
                    let mode = if self.demo { Mode::Simulated } else { Mode::Worn };
                    if self.demo {
                        self.seed_rlva_selftest_demo();
                    }
                    let bridge = self.bridge_runtime.item();
                    self.rlva_selftest.text = None;
                    self.rlva_selftest.runner.confirm(&self.world, mode, bridge, Instant::now());
                }
                Some(UiAction::Start | UiAction::Cancel) => self.rlva_selftest.runner.cancel(),
                _ => {}
            }
        }
        let view = self.rlva_selftest.runner.view();
        let action = crate::ui::rlva_selftest::window(ctx, p, &mut self.rlva_selftest.ui, &view, self.rlva_selftest.text.as_deref());
        match action {
            Some(UiAction::Request) => self.request_rlva_selftest(),
            Some(UiAction::Stop) => self.rlva_selftest.runner.stop(),
            Some(UiAction::Copy) => {
                if let Some(text) = self.rlva_selftest.text.clone() {
                    log::info!("rlva-selftest: report copied ({} lines)", text.lines().count());
                    self.egui_ctx.copy_text(text);
                }
            }
            _ => {}
        }
    }

    /// What the demo world lacks for a run: a writable inventory and the
    /// Library object the test copies (the bridge demo's own).
    fn seed_rlva_selftest_demo(&mut self) {
        if !self.world.inventory_writable_with_ais() {
            crate::world::bridge::demo::seed(&mut self.world);
            crate::ao::demo::inventory_services(&mut self.world, true);
        }
    }

    /// AURORA_DEMO_RLVA=selftest: the one start that no human pressed, in
    /// demo mode only, where there is no grid. The dialog opens at frame
    /// 300 with the in-world cases ticked and is answered at frame 420; both
    /// phases run against the demo's simulated region, the relays simulated.
    fn demo_rlva_selftest(&mut self) {
        if !self.demo || self.rlva_selftest.demo_step > 1 || self.frame_count < 300 {
            return;
        }
        if !std::env::var("AURORA_DEMO_RLVA").is_ok_and(|mode| mode == "selftest") {
            return;
        }
        if self.rlva_selftest.demo_step == 0 {
            self.rlva_selftest.demo_step = 1;
            self.seed_rlva_selftest_demo();
            self.rlva_demo_switch(true);
            self.panels.chat = false;
            self.panels.inventory = false;
            self.panels.perf = false;
            self.rlva_selftest.runner.request();
            self.rlva_selftest.runner.phase2 = true;
            self.rlva_selftest.ui.open = true;
        } else if self.frame_count >= 420 {
            self.rlva_selftest.demo_step = 2;
            self.rlva_selftest
                .runner
                .confirm(&self.world, Mode::Simulated, uuid::Uuid::nil(), Instant::now());
        }
    }
}
