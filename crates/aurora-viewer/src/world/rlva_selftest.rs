//! The RLVa self-test, a Dev Toolbox tool of development builds: once the
//! human is logged in and confirms its dialog, it makes its own scripted
//! test objects in the avatar's inventory, issues real RLVa commands through
//! them, tries the restricted actions and writes a pass / fail report.
//!
//! What it may touch is narrow, and enforced here rather than left to the
//! cases:
//! - it starts from `request` (the tool) then `confirm` (the dialog) only;
//! - in the inventory: one folder, `#RLV/~AuroraSelfTest`, and what it
//!   creates inside; nothing is ever renamed, moved, trashed or deleted, and
//!   a later run reuses what it finds there (exact name, place and owner);
//! - every command of a case goes through `guard`, which only lets through
//!   what names the test's own items and, in phase 2, its own prims;
//! - whatever ends the run (failure, timeout, Stop, RLVa switched off, a
//!   teleport, a logout), `cleanup` runs and the report is written.
//!
//! The runner never sends anything itself: it queues `Out`s that the
//! application sends through the same path as the interface, so that the
//! real send boundary (`World::rlva_send_boundary`) is what is tested.
use super::World;
use super::rlva::Boundary;
use aurora_net::build::BuildCmd;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand, NetEvent, RegionHandle};
use glam::Vec3;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

mod cases;
mod cases_world;
mod factory;
mod guard;
pub mod report;
pub mod sim;
#[cfg(test)]
mod tests;

use report::{Line, Report, Verdict};

/// The test's folder, under the avatar's `#RLV`.
pub const FOLDER: &str = "~AuroraSelfTest";
/// The name of the prims phase 2 rezzes.
pub const SEAT_NAME: &str = "~AuroraSelfTest seat";
pub const RELAY_NAME: &str = "~AuroraSelfTest relay";
pub const RELAY_SOURCE: &str = include_str!("bridge/selftest_relay.lsl");
/// The handshake of a relay; anything a test object says that starts with
/// it is kept out of the chat.
const HELLO: &str = "<aurora-selftest>";
/// The attachment points the test wears its objects on: rarely used ones.
/// Hind Right Foot (controller), Hind Left Foot (object A), Tail Tip
/// (object B and the forced-wear cases).
pub const POINT_CONTROLLER: u8 = 55;
pub const POINT_A: u8 = 54;
pub const POINT_B: u8 = 44;
/// A relay repeats a command within this, or the case fails.
const SAY_TIMEOUT: Duration = Duration::from_secs(10);
/// An expected state shows up within this (a server round trip included).
const CHECK_TIMEOUT: Duration = Duration::from_secs(12);
/// A state of the viewer alone follows the command that sets it within this.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(3);
const HELLO_TIMEOUT: Duration = Duration::from_secs(20);
const HELLO_EVERY: Duration = Duration::from_secs(2);
/// Farther than this from where the run started, the avatar was teleported.
const LEASH: f32 = 12.0;

/// Development builds only (`dev`, `ci`, `debugging`): a published build has
/// neither the tool nor a way to start the runner.
pub fn available() -> bool {
    available_in(env!("AURORA_BUILD_PROFILE"))
}

fn available_in(profile: &str) -> bool {
    profile != "release"
}

/// Who repeats the commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Worn objects with the relay script: a grid.
    Worn,
    /// No script engine (demo mode, unit tests): the same text is delivered
    /// as an owner-say of the worn test object.
    Simulated,
}

/// The test's own inventory objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum It {
    Controller,
    A,
    B,
    /// Its name gives its point: `(tail tip)`.
    Tagged,
    /// Worn once to learn where the simulator puts an object with no point.
    Probe,
    /// Never given a point by the test.
    Plain,
}

impl It {
    pub(crate) const ALL: [It; 6] = [It::Controller, It::A, It::B, It::Tagged, It::Probe, It::Plain];

    pub(crate) fn name(self) -> &'static str {
        match self {
            It::Controller => "~AuroraSelfTest controller",
            It::A => "~AuroraSelfTest a",
            It::B => "~AuroraSelfTest b",
            It::Tagged => "~AuroraSelfTest tagged (tail tip)",
            It::Probe => "~AuroraSelfTest probe",
            It::Plain => "~AuroraSelfTest plain",
        }
    }

    /// Its subfolder of the test folder; none for the folder itself.
    pub(crate) fn sub(self) -> Option<usize> {
        match self {
            It::Controller => None,
            It::A => Some(0),
            It::B => Some(1),
            It::Tagged => Some(2),
            It::Probe => Some(3),
            It::Plain => Some(4),
        }
    }
}

/// The subfolders of the test folder; `linked` holds a link to object B.
pub(crate) const SUBFOLDERS: [&str; 6] = ["locked", "free", "tagged", "probe", "nopoint", "linked"];
pub(crate) const LINKED: usize = 5;

/// Which relay speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Who {
    Controller,
    /// Object A: a second source, for what must outlive the controller.
    Second,
    /// The first prim of phase 2.
    Seat,
}

/// A prim phase 2 rezzed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Prim {
    pub id: Uuid,
    pub handle: RegionHandle,
    pub local_id: u32,
}

/// What the cases share: the test's own ids and what was observed.
pub(crate) struct Ctx {
    pub mode: Mode,
    pub agent: Uuid,
    pub region: RegionHandle,
    pub origin: Vec3,
    pub shared: Uuid,
    pub root: Uuid,
    pub folders: [Uuid; 6],
    pub items: [Uuid; 6],
    /// The worn controller (its object id changes each time it is worn).
    pub controller: Uuid,
    /// The positive channel the queries answer on.
    pub channel: i32,
    pub replies: Vec<String>,
    /// The relay expected to repeat `.1`.
    pub echo: Option<(Uuid, String)>,
    /// The boundary's answer to the last probe, with the command as it left.
    pub outcome: Option<(Boundary, NetCommand)>,
    pub last: Option<(Boundary, NetCommand)>,
    probing: bool,
    pub prims: Vec<Prim>,
    /// Objects flagged "created selected" before a rez: not ours.
    pub flagged: HashSet<Uuid>,
    /// Where the two prims are asked for.
    pub spots: [Vec3; 2],
    pub adds: u8,
    /// The script in the seat, when one could be put there.
    pub seat_script: Option<Uuid>,
    /// Where the simulator put the object that has no point.
    pub free_point: u8,
    pub bridge: Uuid,
    pub flew: bool,
    /// Where the avatar stood before the run's sit.
    pub stood: Option<(RegionHandle, Vec3)>,
    /// Why the in-world cases are skipped.
    pub world_skip: Option<String>,
    /// A made-up resident, for the name rules: nobody real is involved.
    pub other: Uuid,
}

impl Ctx {
    fn new(world: &World, mode: Mode, bridge: Uuid) -> Self {
        Self {
            mode,
            agent: world.agent_id,
            region: world.main_region.unwrap_or_default(),
            origin: world.agent.position,
            shared: Uuid::nil(),
            root: Uuid::nil(),
            folders: [Uuid::nil(); 6],
            items: [Uuid::nil(); 6],
            controller: Uuid::nil(),
            // (a positive channel nothing else is likely to listen on)
            channel: 0x2000_0000 + (Uuid::new_v4().as_u128() & 0x0fff_ffff) as i32,
            replies: Vec::new(),
            echo: None,
            outcome: None,
            last: None,
            probing: false,
            prims: Vec::new(),
            flagged: HashSet::new(),
            spots: [Vec3::ZERO; 2],
            adds: 0,
            seat_script: None,
            free_point: 0,
            bridge,
            flew: false,
            stood: None,
            world_skip: None,
            other: Uuid::from_u128(0x5e1f_7e57_0000_0000_0000_0000_0000_0001),
        }
    }

    pub fn item(&self, it: It) -> Uuid {
        self.items[It::ALL.iter().position(|i| *i == it).unwrap_or(0)]
    }

    pub fn own_item(&self, id: &Uuid) -> bool {
        !id.is_nil() && self.items.contains(id)
    }

    pub fn worn(&self, world: &World, it: It) -> bool {
        world.worn_attachment(&self.item(it)).is_some()
    }

    pub fn point(&self, world: &World, it: It) -> u8 {
        world
            .worn_attachment(&self.item(it))
            .and_then(|i| world.objects.get(i))
            .map_or(0, |o| o.attachment_point())
    }

    /// An object that is not the test's holds a restriction: what it already
    /// refuses cannot be told from what a case sets.
    pub fn foreign_rules(&self, world: &World) -> bool {
        let own: Vec<Uuid> = self
            .items
            .iter()
            .filter_map(|item| world.worn_attachment(item))
            .filter_map(|i| world.objects.get(i))
            .map(|o| o.full_id)
            .chain(self.prims.iter().map(|p| p.id))
            .collect();
        world.rlva.restrictions().iter().any(|(source, _)| !own.contains(source))
    }

    /// The object a relay speaks from, if it is there.
    pub fn speaker(&self, world: &World, who: Who) -> Option<Uuid> {
        let id = match who {
            Who::Controller => self.controller,
            Who::Second => world
                .worn_attachment(&self.item(It::A))
                .and_then(|i| world.objects.get(i))
                .map_or(Uuid::nil(), |o| o.full_id),
            Who::Seat => self.seat_script.and(self.prims.first()).map_or(Uuid::nil(), |p| p.id),
        };
        (!id.is_nil() && world.objects.index_of_uuid(&id).is_some()).then_some(id)
    }

    /// `{ch}`, `{f}`, `{seat}`, `{layer}`… of a command's text.
    pub fn fill(&self, world: &World, text: &str) -> String {
        let seat = self.prims.first().map_or(Uuid::nil(), |p| p.id);
        text.replace("{ch}", &self.channel.to_string())
            .replace("{f}", FOLDER)
            .replace("{seat}", &seat.to_string())
            .replace("{other}", &self.other.to_string())
            .replace("{layer}", cases::worn_layer(world).map_or("shirt", |(_, name)| name))
    }

    /// `text` said through the relay of `object`: chat on its private
    /// channel (never channel 0), or the owner-say itself when simulated.
    fn say(&self, world: &World, object: Uuid, text: &str) -> Out {
        match self.mode {
            Mode::Worn => Out::Send(NetCommand::ScriptDialogReply {
                sender: None,
                object_id: self.agent,
                channel: super::bridge::private_channel(object),
                index: 0,
                label: text.to_owned(),
            }),
            Mode::Simulated => Out::Hear(NetEvent::Chat(ChatMessage {
                from_name: FOLDER.to_owned(),
                source_id: object,
                owner_id: self.agent,
                source_type: ChatSourceType::Object,
                chat_type: ChatType::OwnerSay,
                region: world.main_region,
                position: world.agent.position,
                message: text.to_owned(),
            })),
        }
    }
}

/// What the application does for the runner.
#[derive(Debug)]
pub enum Out {
    /// Sent through the same path as the interface's own commands.
    Send(NetCommand),
    /// The same, and the boundary's answer is the result (`Runner::boundary`).
    Probe(NetCommand),
    /// What a simulated relay says: delivered as if received.
    Hear(NetEvent),
    /// Take off or land in place, as the Fly button does.
    Fly(bool),
}

pub(crate) type Test = Box<dyn Fn(&World, &mut Ctx) -> Result<(), String>>;
pub(crate) type Make = Box<dyn Fn(&World, &Ctx) -> Cmd>;
pub(crate) type Skip = Box<dyn Fn(&World, &Ctx) -> Option<String>>;

/// The command of a probe.
pub(crate) enum Cmd {
    Send(Box<NetCommand>),
    /// Nothing to do (the object is already worn, already off).
    Nothing,
    /// It cannot be built: the step fails.
    Missing(&'static str),
}

impl Cmd {
    pub(crate) fn send(command: NetCommand) -> Self {
        Cmd::Send(Box::new(command))
    }
}

pub(crate) enum Step {
    SkipIf(Skip),
    /// Issued through a relay, and repeated by it.
    Say(Who, &'static str),
    /// A relay answers its handshake (after it was worn or rezzed).
    Hello(Who),
    /// True within `CHECK_TIMEOUT`.
    Expect(&'static str, Test),
    /// True within `LOCAL_TIMEOUT`: nothing but the viewer is waited for.
    Soon(&'static str, Test),
    /// True now.
    Now(&'static str, Test),
    /// Sent through the interface's path: the boundary must not let it out.
    /// Never sent when the boundary's own rule says it would pass.
    Refused(&'static str, Make),
    /// Sent through the interface's path: it must leave.
    Sent(&'static str, Make),
    Pause(Duration),
    Fly(bool),
    /// A copy of the relay script in the seat (phase 2), when possible.
    InstallSeat,
}

pub(crate) struct Case {
    pub id: &'static str,
    pub title: &'static str,
    pub phase: u8,
    /// Its failure skips the rest of its phase.
    pub gate: bool,
    /// The issue that already covers its failure.
    pub known: Option<&'static str>,
    pub steps: Vec<Step>,
}

struct Active {
    case: Case,
    at: usize,
    entered: Option<Instant>,
    resaid: Instant,
    line: Line,
    spoke: Vec<Who>,
    /// The closing `@clear`s still to say.
    closing: Option<Vec<Who>>,
}

enum Stage {
    Setup,
    Cases,
    Cleanup(Clean, Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clean {
    Start,
    Clear,
    /// A relay did not answer `@clear`: it is taken off, and its rules go
    /// with it.
    Release,
    Stand,
    Prims,
    Detach,
}

struct Run {
    ctx: Ctx,
    phase2: bool,
    stage: Stage,
    factory: factory::Factory,
    install: Option<factory::ScriptInstall>,
    cases: VecDeque<Case>,
    active: Option<Active>,
    lines: Vec<Line>,
    notes: Vec<String>,
    /// Why the run stops before its last case.
    abort: Option<String>,
    online: bool,
    /// The cleanup asked a second time for what was still worn.
    detached_again: bool,
    /// The prims the cleanup asked the simulator to remove.
    removal_asked: HashSet<Uuid>,
}

/// What the tool's window shows while the run goes on.
pub struct View {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub current: String,
}

#[derive(Default)]
pub struct Runner {
    confirming: bool,
    /// The dialog's choice: the in-world cases (rez, sit, fly). Off by default.
    pub phase2: bool,
    run: Option<Box<Run>>,
    report: Option<Report>,
    out: Vec<Out>,
    total: usize,
}

impl Runner {
    /// The tool was pressed: the confirmation dialog opens. Nothing else
    /// leads to a run.
    pub fn request(&mut self) {
        if available() && self.run.is_none() {
            self.confirming = true;
            self.phase2 = false;
        }
    }

    pub fn confirming(&self) -> bool {
        self.confirming
    }

    pub fn cancel(&mut self) {
        self.confirming = false;
    }

    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    pub fn report(&self) -> Option<&Report> {
        self.report.as_ref()
    }

    pub fn view(&self) -> View {
        let run = self.run.as_deref();
        View {
            running: run.is_some(),
            done: run.map_or(0, |r| r.lines.len()),
            total: self.total,
            current: run.map_or_else(String::new, |r| match (&r.stage, &r.active) {
                (Stage::Setup, _) => format!("setup: {}", r.factory.state()),
                (Stage::Cleanup(..), _) => "cleanup".to_owned(),
                (Stage::Cases, Some(a)) => format!("{} {}", a.case.id, a.case.title),
                (Stage::Cases, None) => String::new(),
            }),
        }
    }

    /// The human confirmed the dialog. `bridge` is the viewer's own bridge
    /// item (nil without one), which the attachment queries leave out.
    pub fn confirm(&mut self, world: &World, mode: Mode, bridge: Uuid, now: Instant) {
        if !self.confirming || !available() || self.run.is_some() {
            return;
        }
        self.confirming = false;
        self.report = None;
        let mut cases: VecDeque<Case> = cases::phase1().into();
        if self.phase2 {
            cases.extend(cases_world::phase2());
        }
        self.total = cases.len();
        let mut run = Box::new(Run {
            ctx: Ctx::new(world, mode, bridge),
            phase2: self.phase2,
            stage: Stage::Setup,
            factory: factory::Factory::new(now),
            install: None,
            cases,
            active: None,
            lines: Vec::new(),
            notes: Vec::new(),
            abort: None,
            online: true,
            detached_again: false,
            removal_asked: HashSet::new(),
        });
        log::info!(
            "rlva-selftest: started ({mode:?} controller, phase 2 {})",
            if self.phase2 { "chosen" } else { "not chosen" }
        );
        if !world.rlva.enabled() {
            run.abort = Some("RLVa is switched off".to_owned());
        }
        self.run = Some(run);
    }

    /// Stop was pressed: the run ends in its cleanup.
    pub fn stop(&mut self) {
        if let Some(run) = self.run.as_deref_mut()
            && run.abort.is_none()
        {
            run.abort = Some("stopped".to_owned());
        }
    }

    /// What the runner asks the application to do, in order.
    pub fn take_out(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    /// The send boundary answered a command: the answer to the probe the
    /// runner is waiting for, if it is.
    pub fn boundary(&mut self, outcome: Boundary, command: &NetCommand) {
        if let Some(run) = self.run.as_deref_mut()
            && run.ctx.probing
        {
            run.ctx.probing = false;
            run.ctx.outcome = Some((outcome, command.clone()));
        }
    }

    /// The application is about to send a probe / has sent it. A probe that
    /// never reached the boundary is seen at the step's timeout.
    pub fn probe(&mut self, armed: bool) {
        if let Some(run) = self.run.as_deref_mut() {
            run.ctx.probing = armed;
        }
    }

    /// RLVa answered a query on a chat channel.
    pub fn reply(&mut self, channel: i32, message: &str) {
        if let Some(run) = self.run.as_deref_mut()
            && channel == run.ctx.channel
        {
            run.ctx.replies.push(message.to_owned());
        }
    }

    /// Sees an event before the world does. True when it is the test's own
    /// (the answer to its inventory requests, a relay's handshake) and goes
    /// no further.
    pub fn event(&mut self, world: &mut World, event: &NetEvent) -> bool {
        let Some(run) = self.run.as_deref_mut() else {
            return false;
        };
        if let NetEvent::Chat(chat) = event
            && chat.source_type == ChatSourceType::Object
            && chat.chat_type == ChatType::OwnerSay
            && chat.owner_id == run.ctx.agent
        {
            if run
                .ctx
                .echo
                .as_ref()
                .is_some_and(|(object, text)| *object == chat.source_id && *text == chat.message)
            {
                run.ctx.echo = None;
            }
            // (the handshake is the test's own: neither chat nor RLVa)
            return chat.message.starts_with(HELLO);
        }
        if let Some(install) = run.install.as_mut() {
            install.event(world, &run.ctx, event, &mut self.out);
        }
        run.factory.event(world, &run.ctx, event, &mut self.out)
    }

    /// One frame. `online` is false once the session is gone: nothing can be
    /// sent any more, the run ends at once.
    pub fn step(&mut self, world: &mut World, online: bool, now: Instant) {
        let Some(run) = self.run.as_deref_mut() else {
            return;
        };
        run.online = online;
        if run.abort.is_none() && !matches!(run.stage, Stage::Cleanup(..)) {
            run.abort = if !online {
                Some("logged out".to_owned())
            } else if world.agent_id != run.ctx.agent {
                Some("another session".to_owned())
            } else if !world.rlva.enabled() {
                Some("RLVa was switched off".to_owned())
            } else if world.main_region != Some(run.ctx.region) || world.agent.position.distance(run.ctx.origin) > LEASH {
                Some("the avatar was teleported or moved away".to_owned())
            } else {
                None
            };
        }
        if let Some(why) = run.abort.clone()
            && !matches!(run.stage, Stage::Cleanup(..))
        {
            run.skip_rest(&why);
            run.notes.push(format!("the run ended early: {why}"));
            run.stage = Stage::Cleanup(Clean::Start, now);
        }
        match run.stage {
            Stage::Setup => match run.factory.step(world, &mut run.ctx, &mut self.out, now) {
                factory::Poll::Pending => {}
                factory::Poll::Ready => run.stage = Stage::Cases,
                factory::Poll::Failed(why) => run.abort = Some(format!("the test objects could not be prepared: {why}")),
            },
            Stage::Cases => {
                if !run.cases_step(world, &mut self.out, now) {
                    run.stage = Stage::Cleanup(Clean::Start, now);
                }
            }
            Stage::Cleanup(..) => {
                if run.cleanup(world, &mut self.out, now) {
                    self.finish();
                }
            }
        }
    }

    fn finish(&mut self) {
        let Some(run) = self.run.take() else { return };
        let report = Report {
            lines: run.lines,
            notes: run.notes,
            controller: match run.ctx.mode {
                Mode::Worn => "worn relay script",
                Mode::Simulated => "simulated (no script engine)",
            },
            phase2: run.phase2,
        };
        for line in report.text().lines() {
            log::info!("rlva-selftest: {line}");
        }
        self.report = Some(report);
    }
}

fn line_of(case: &Case) -> Line {
    Line {
        verdict: Verdict::Pass,
        id: case.id,
        title: case.title,
        commands: Vec::new(),
        expected: String::new(),
        observed: String::new(),
    }
}

/// Whether a step goes on at once or waits for the next frame.
enum Flow {
    Next,
    Wait,
    Fail(String, String),
}

/// A command of the cleanup, through the same gate as the cases': the test's
/// own objects only.
fn send_own(command: NetCommand, ctx: &Ctx, world: &World, out: &mut Vec<Out>) {
    if guard::allows(&command, ctx, world, false) {
        out.push(Out::Send(command));
    } else {
        log::error!("rlva-selftest: the cleanup wanted a command outside the test's own objects");
    }
}

impl Run {
    /// Every case not run yet is reported skipped; the one in progress too.
    fn skip_rest(&mut self, why: &str) {
        if let Some(active) = self.active.take() {
            let mut line = active.line;
            if line.verdict == Verdict::Pass {
                line.verdict = Verdict::Skipped(format!("interrupted: {why}"));
            }
            self.lines.push(line);
        }
        for case in std::mem::take(&mut self.cases) {
            let mut line = line_of(&case);
            line.verdict = Verdict::Skipped(why.to_owned());
            self.lines.push(line);
        }
        self.install = None;
        self.ctx.echo = None;
    }

    /// False when no case is left.
    fn cases_step(&mut self, world: &mut World, out: &mut Vec<Out>, now: Instant) -> bool {
        loop {
            if self.active.is_none() {
                let Some(case) = self.cases.pop_front() else {
                    return false;
                };
                let mut line = line_of(&case);
                if case.phase == 2
                    && let Some(why) = &self.ctx.world_skip
                {
                    line.verdict = Verdict::Skipped(why.clone());
                    self.lines.push(line);
                    continue;
                }
                self.ctx.replies.clear();
                self.active = Some(Active {
                    case,
                    at: 0,
                    entered: None,
                    resaid: now,
                    line,
                    spoke: Vec::new(),
                    closing: None,
                });
            }
            if !self.case_step(world, out, now) {
                return true;
            }
            let Some(active) = self.active.take() else { continue };
            let mut line = active.line;
            if let (Verdict::Fail, Some(issue)) = (&line.verdict, active.case.known) {
                line.verdict = Verdict::Known(issue);
            } else if let (Verdict::Pass, Some(issue)) = (&line.verdict, active.case.known) {
                line.observed = format!("passes although {issue} is open");
            }
            if active.case.gate && line.verdict != Verdict::Pass {
                self.ctx.world_skip = Some(match &line.verdict {
                    Verdict::Skipped(why) => why.clone(),
                    _ => format!("{} failed: {}", line.id, line.observed),
                });
            }
            if line.verdict == Verdict::Pass && line.observed.is_empty() {
                line.observed = "as expected".to_owned();
            }
            self.lines.push(line);
        }
    }

    /// Advances the case in progress. True when it is over.
    fn case_step(&mut self, world: &mut World, out: &mut Vec<Out>, now: Instant) -> bool {
        loop {
            let Some(active) = self.active.as_mut() else {
                return true;
            };
            // the case is over, or failed: what it restricted is released
            // through each relay it spoke to, whatever happened
            if active.at >= active.case.steps.len() || active.line.verdict != Verdict::Pass {
                let closing = active.closing.get_or_insert_with(|| {
                    active.entered = None;
                    self.ctx.echo = None;
                    std::mem::take(&mut active.spoke)
                });
                let first = active.entered.is_none();
                let entered = *active.entered.get_or_insert(now);
                if !first && self.ctx.echo.is_some() && now.saturating_duration_since(entered) < SAY_TIMEOUT {
                    return false;
                }
                if !first && self.ctx.echo.take().is_some() && closing.first() == Some(&Who::Controller) {
                    self.abort = Some("the controller stopped answering".to_owned());
                }
                if !first {
                    closing.remove(0);
                    active.entered = None;
                }
                match closing.first().copied() {
                    None => return true,
                    Some(who) => {
                        if let Some(object) = self.ctx.speaker(world, who) {
                            self.ctx.echo = Some((object, "@clear".to_owned()));
                            out.push(self.ctx.say(world, object, "@clear"));
                        }
                        active.entered = Some(now);
                        if self.ctx.echo.is_none() {
                            // (the relay is gone, and its restrictions with it)
                            closing.remove(0);
                            active.entered = None;
                            continue;
                        }
                        return false;
                    }
                }
            }
            let first = active.entered.is_none();
            let entered = *active.entered.get_or_insert(now);
            let waited = now.saturating_duration_since(entered);
            let ctx = &mut self.ctx;
            let flow = match &active.case.steps[active.at] {
                Step::SkipIf(skip) => match skip(world, ctx) {
                    Some(why) => {
                        active.line.verdict = Verdict::Skipped(why);
                        continue;
                    }
                    None => Flow::Next,
                },
                Step::Say(who, text) => {
                    if first {
                        match ctx.speaker(world, *who) {
                            Some(object) => {
                                let filled = ctx.fill(world, text);
                                active.line.commands.push((*text).to_owned());
                                if !active.spoke.contains(who) {
                                    active.spoke.push(*who);
                                }
                                ctx.echo = Some((object, filled.clone()));
                                out.push(ctx.say(world, object, &filled));
                                Flow::Wait
                            }
                            None => Flow::Fail((*text).to_owned(), "the test object that issues it is not there".to_owned()),
                        }
                    } else if ctx.echo.is_none() {
                        Flow::Next
                    } else if waited > SAY_TIMEOUT {
                        ctx.echo = None;
                        if *who == Who::Controller {
                            self.abort = Some("the controller stopped answering".to_owned());
                        }
                        Flow::Fail((*text).to_owned(), "the relay did not repeat the command".to_owned())
                    } else {
                        Flow::Wait
                    }
                }
                Step::Hello(who) => {
                    if (!first && ctx.echo.is_none()) || (*who == Who::Seat && ctx.seat_script.is_none()) {
                        Flow::Next
                    } else if waited > HELLO_TIMEOUT {
                        ctx.echo = None;
                        if *who == Who::Seat {
                            // the copy in the seat is optional: what needs
                            // it is skipped
                            ctx.seat_script = None;
                            Flow::Next
                        } else {
                            Flow::Fail("the relay answers".to_owned(), "no answer from the relay script".to_owned())
                        }
                    } else {
                        if (first || now.saturating_duration_since(active.resaid) >= HELLO_EVERY)
                            && let Some(object) = ctx.speaker(world, *who)
                        {
                            active.resaid = now;
                            ctx.echo = Some((object, HELLO.to_owned()));
                            out.push(ctx.say(world, object, HELLO));
                        }
                        Flow::Wait
                    }
                }
                Step::Expect(label, test) => match test(world, ctx) {
                    Ok(()) => Flow::Next,
                    Err(observed) if waited > CHECK_TIMEOUT => Flow::Fail((*label).to_owned(), observed),
                    Err(_) => Flow::Wait,
                },
                Step::Soon(label, test) => match test(world, ctx) {
                    Ok(()) => Flow::Next,
                    Err(observed) if waited > LOCAL_TIMEOUT => Flow::Fail((*label).to_owned(), observed),
                    Err(_) => Flow::Wait,
                },
                Step::Now(label, test) => match test(world, ctx) {
                    Ok(()) => Flow::Next,
                    Err(observed) => Flow::Fail((*label).to_owned(), observed),
                },
                Step::Pause(time) => {
                    if waited >= *time {
                        Flow::Next
                    } else {
                        Flow::Wait
                    }
                }
                Step::Fly(on) => {
                    ctx.flew |= *on;
                    out.push(Out::Fly(*on));
                    Flow::Next
                }
                Step::InstallSeat => match self.install.as_mut() {
                    None if first => match (ctx.mode, ctx.prims.first().copied()) {
                        (_, None) => Flow::Next,
                        (Mode::Simulated, Some(prim)) => {
                            // no script engine: the item alone, for the
                            // contents rules
                            out.push(Out::Send(factory::relay_item(world, ctx, prim.handle, prim.local_id, prim.id)));
                            ctx.seat_script = Some(Uuid::new_v4());
                            Flow::Next
                        }
                        (Mode::Worn, Some(prim)) => {
                            self.install = Some(factory::ScriptInstall::new(prim.id, now, false));
                            Flow::Wait
                        }
                    },
                    None => Flow::Next,
                    Some(install) => match install.step(world, out, now) {
                        factory::Poll::Pending => Flow::Wait,
                        factory::Poll::Ready => {
                            ctx.seat_script = Some(install.item());
                            self.install = None;
                            Flow::Next
                        }
                        factory::Poll::Failed(why) => {
                            // not a failure of the case: what needs the
                            // script is skipped
                            log::info!("rlva-selftest: no script in the seat: {why}");
                            self.install = None;
                            Flow::Next
                        }
                    },
                },
                Step::Refused(label, make) | Step::Sent(label, make) => {
                    let refused = matches!(active.case.steps[active.at], Step::Refused(..));
                    if first {
                        match make(world, ctx) {
                            Cmd::Nothing => Flow::Next,
                            Cmd::Missing(what) => Flow::Fail((*label).to_owned(), format!("{what} is missing")),
                            Cmd::Send(command) => {
                                let command = *command;
                                if !guard::allows(&command, ctx, world, refused) {
                                    log::error!("rlva-selftest: {} wanted a command outside the test's own objects", active.case.id);
                                    Flow::Fail(
                                        (*label).to_owned(),
                                        "the command names something the test did not create: not sent".to_owned(),
                                    )
                                } else if refused && world.rlva_send_boundary(&mut command.clone(), Uuid::nil()) == Boundary::Send {
                                    // never "try" what the rule itself would let through
                                    Flow::Fail((*label).to_owned(), "the send boundary lets it through (not sent)".to_owned())
                                } else {
                                    if matches!(command, NetCommand::Build(BuildCmd::Add(_))) && !refused {
                                        ctx.adds += 1;
                                    }
                                    ctx.outcome = None;
                                    out.push(Out::Probe(command));
                                    Flow::Wait
                                }
                            }
                        }
                    } else {
                        match ctx.outcome.take() {
                            Some((outcome, command)) => {
                                let ok = (outcome == Boundary::Send) != refused;
                                ctx.last = Some((outcome, command));
                                if ok {
                                    Flow::Next
                                } else {
                                    Flow::Fail(
                                        (*label).to_owned(),
                                        match outcome {
                                            Boundary::Send => "it left: the boundary did not refuse it",
                                            Boundary::Refused => "refused at the send boundary",
                                            Boundary::Dropped => "dropped at the send boundary",
                                        }
                                        .to_owned(),
                                    )
                                }
                            }
                            None if waited > Duration::from_secs(2) => {
                                Flow::Fail((*label).to_owned(), "the command did not reach the send boundary".to_owned())
                            }
                            None => Flow::Wait,
                        }
                    }
                }
            };
            match flow {
                Flow::Next => {
                    let label = match &active.case.steps[active.at] {
                        Step::Expect(label, _)
                        | Step::Soon(label, _)
                        | Step::Now(label, _)
                        | Step::Refused(label, _)
                        | Step::Sent(label, _) => *label,
                        _ => "",
                    };
                    if !label.is_empty() && !active.line.expected.contains(label) {
                        if !active.line.expected.is_empty() {
                            active.line.expected.push_str(", ");
                        }
                        active.line.expected.push_str(label);
                    }
                    active.at += 1;
                    active.entered = None;
                }
                Flow::Wait => return false,
                Flow::Fail(expected, observed) => {
                    active.line.verdict = Verdict::Fail;
                    active.line.expected = expected;
                    active.line.observed = observed;
                    active.entered = None;
                }
            }
        }
    }

    /// Releases and takes back everything the run put in place. It reads the
    /// world and the test's own ids only, never the step that failed. True
    /// when it is over.
    fn cleanup(&mut self, world: &mut World, out: &mut Vec<Out>, now: Instant) -> bool {
        let Stage::Cleanup(stage, since) = self.stage else {
            return true;
        };
        let waited = now.saturating_duration_since(since);
        let ctx = &mut self.ctx;
        cases_world::adopt(world, ctx);
        // (a rez that has not come back yet)
        let awaited = usize::from(ctx.adds) > ctx.prims.len();
        let relays: Vec<Uuid> = [Who::Controller, Who::Second, Who::Seat]
            .into_iter()
            .filter_map(|who| ctx.speaker(world, who))
            .collect();
        let own_seat = world.agent.seated && ctx.prims.iter().any(|p| p.id == world.rlva_sit_id());
        let prims: Vec<Prim> = ctx.prims.iter().copied().filter(|p| guard::still_own_prim(world, ctx, p)).collect();
        let worn: Vec<Uuid> = ctx
            .items
            .iter()
            .copied()
            .filter(|id| !id.is_nil() && world.worn_attachment(id).is_some())
            .collect();
        if !self.online {
            if !worn.is_empty() || !prims.is_empty() {
                self.notes.push(format!(
                    "the session ended before the cleanup: {} test object(s) worn and {} prim(s) named \"{SEAT_NAME}\" may be left",
                    worn.len(),
                    prims.len()
                ));
            }
            return true;
        }
        let next = match stage {
            Clean::Start => {
                ctx.echo = None;
                self.install = None;
                for object in &relays {
                    out.push(ctx.say(world, *object, "@clear"));
                }
                Some(Clean::Clear)
            }
            Clean::Clear | Clean::Release => {
                let held = world.rlva.restrictions().iter().any(|(source, _)| relays.contains(source));
                let patience = if stage == Clean::Clear { 4 } else { 8 };
                if held && waited < Duration::from_secs(patience) {
                    None
                } else if held && stage == Clean::Clear && !worn.is_empty() {
                    send_own(NetCommand::DetachAttachments(worn.clone()), ctx, world, out);
                    Some(Clean::Release)
                } else {
                    if held {
                        self.notes
                            .push("restrictions of the test objects were still set when the cleanup went on".to_owned());
                    }
                    if ctx.flew && world.agent.flying {
                        out.push(Out::Fly(false));
                    }
                    if own_seat {
                        send_own(NetCommand::OneShotControl(aurora_net::control::STAND_UP), ctx, world, out);
                    }
                    Some(Clean::Stand)
                }
            }
            Clean::Stand => {
                if (own_seat && waited < Duration::from_secs(6)) || (awaited && waited < Duration::from_secs(5)) {
                    None
                } else {
                    if own_seat {
                        self.notes
                            .push("the avatar still sits on the test's prim: stand up by hand".to_owned());
                    }
                    // DeRez to the Trash: the one deletion of the test, and
                    // of its own prims only
                    let trash = super::inventory::actions::system(&world.inventory, 14).unwrap_or_default();
                    self.removal_asked.extend(prims.iter().map(|p| p.id));
                    for handle in prims.iter().map(|p| p.handle).collect::<HashSet<_>>() {
                        let remove = NetCommand::Build(BuildCmd::DeRez {
                            handle,
                            local_ids: prims.iter().filter(|p| p.handle == handle).map(|p| p.local_id).collect(),
                            destination: aurora_net::build::derez::TRASH,
                            destination_id: trash,
                            group_id: world.groups.active,
                        });
                        send_own(remove, ctx, world, out);
                    }
                    Some(Clean::Prims)
                }
            }
            Clean::Prims => {
                if !prims.is_empty() && waited < Duration::from_secs(10) {
                    None
                } else {
                    for prim in &prims {
                        let at = world
                            .objects
                            .index_of_uuid(&prim.id)
                            .and_then(|i| world.objects.get(i))
                            .map_or(Vec3::ZERO, |o| o.position);
                        self.notes.push(format!(
                            "a prim named \"{SEAT_NAME}\" could not be removed, at <{:.1}, {:.1}, {:.1}>: remove it by hand",
                            at.x, at.y, at.z
                        ));
                    }
                    // (out of sight when the cleanup came: another region)
                    let unreached = ctx.prims.iter().filter(|p| !self.removal_asked.contains(&p.id)).count();
                    if unreached > 0 {
                        let at = ctx.spots[0];
                        self.notes.push(format!(
                            "{unreached} prim(s) named \"{SEAT_NAME}\" could not be reached for removal, near <{:.1}, {:.1}, {:.1}> of the region the run started in: remove them by hand",
                            at.x, at.y, at.z
                        ));
                    }
                    if !worn.is_empty() {
                        send_own(NetCommand::DetachAttachments(worn.clone()), ctx, world, out);
                    }
                    Some(Clean::Detach)
                }
            }
            Clean::Detach => {
                if !worn.is_empty() && waited < Duration::from_secs(10) {
                    // an object a rule of the controller kept on comes off
                    // once the controller is gone, and its rules with it
                    if waited >= Duration::from_secs(4) && !self.detached_again {
                        self.detached_again = true;
                        send_own(NetCommand::DetachAttachments(worn.clone()), ctx, world, out);
                    }
                    None
                } else {
                    if !worn.is_empty() {
                        self.notes.push(format!(
                            "{} test object(s) of the folder {FOLDER} are still worn: take them off by hand",
                            worn.len()
                        ));
                    }
                    return true;
                }
            }
        };
        if let Some(next) = next {
            self.stage = Stage::Cleanup(next, now);
        }
        false
    }
}
