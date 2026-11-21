//! Prepares the test's inventory and its relays: the folder under `#RLV`,
//! the copies of the Library object, the link, the two worn relays and
//! their script. Everything is found again by exact name, place and owner
//! before anything is created, so that a later run reuses what the first
//! one made; nothing is ever renamed, moved or deleted.
use super::super::bridge::factory as shared;
use super::super::inventory::{FetchState, Inventory};
use super::{Ctx, FOLDER, HELLO, HELLO_EVERY, HELLO_TIMEOUT, It, LINKED, Mode, Out, RELAY_NAME, RELAY_SOURCE, SUBFOLDERS, Who, World};
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use aurora_net::{NetCommand, NetEvent, RegionHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);

pub(super) enum Poll {
    Pending,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Inventory,
    Shared,
    Folder,
    Subfolders,
    Items,
    Link,
    Wear,
    Script(usize),
    Hello(usize),
    Ready,
}

/// The relays, in the order they are prepared.
const RELAYS: [(Who, It, u8); 2] = [
    (Who::Controller, It::Controller, super::POINT_CONTROLLER),
    (Who::Second, It::A, super::POINT_A),
];

pub(super) struct Factory {
    phase: Phase,
    since: Instant,
    next: Instant,
    /// The inventory request in flight.
    request: Option<Uuid>,
    /// This phase already asked for what it misses: it never asks twice.
    made: bool,
    install: Option<ScriptInstall>,
    repaired: [bool; 2],
    said: Instant,
    failed: Option<String>,
}

fn fetched(inv: &mut Inventory, id: Uuid) -> bool {
    if inv.folders.get(&id).is_some_and(|f| f.state == FetchState::Fetched) {
        return true;
    }
    inv.request(id);
    false
}

/// The user's folder named exactly `name` right under `parent`.
fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders.get(&parent)?.children.iter().copied().find(|id| {
        inv.folders
            .get(id)
            .is_some_and(|f| !f.library && f.info.parent == parent && f.info.name == name)
    })
}

/// The agent's own object named exactly `name` right in `folder`.
fn object(inv: &Inventory, agent: Uuid, folder: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .get(&folder)?
        .items
        .iter()
        .filter_map(|id| inv.items.get(id))
        .find(|i| i.parent == folder && i.name == name && i.asset_type == 6 && i.owner == agent)
        .map(|i| i.id)
}

fn new_folder(parent: Uuid, name: &str) -> Operation {
    Operation::Folder {
        id: Uuid::new_v4(),
        parent,
        name: name.to_owned(),
        type_default: -1,
    }
}

/// A new, stopped script item named after the relay, in an object's contents.
pub(super) fn relay_item(world: &World, ctx: &Ctx, handle: RegionHandle, local_id: u32, object: Uuid) -> NetCommand {
    shared::rez_script(
        handle,
        local_id,
        object,
        ctx.agent,
        world.groups.active,
        RELAY_NAME,
        "Aurora's RLVa self-test relay",
    )
}

impl Factory {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            phase: Phase::Inventory,
            since: now,
            next: now,
            request: None,
            made: false,
            install: None,
            repaired: [false; 2],
            said: now,
            failed: None,
        }
    }

    pub(super) fn state(&self) -> &'static str {
        match self.phase {
            Phase::Inventory => "waiting for the inventory",
            Phase::Shared => "the #RLV folder",
            Phase::Folder | Phase::Subfolders => "the test folder",
            Phase::Items => "copying the Library object",
            Phase::Link => "the link",
            Phase::Wear => "wearing the relays",
            Phase::Script(_) => "writing the relay script",
            Phase::Hello(_) => "waiting for the relay script",
            Phase::Ready => "ready",
        }
    }

    fn enter(&mut self, phase: Phase, now: Instant) {
        self.phase = phase;
        self.since = now;
        self.next = now;
        self.made = false;
        self.request = None;
    }

    fn mutate(&mut self, change: Mutation, out: &mut Vec<Out>) {
        let request = Uuid::new_v4();
        self.request = Some(request);
        self.made = true;
        out.push(Out::Send(NetCommand::EditInventory { request, change }));
    }

    pub(super) fn step(&mut self, world: &mut World, ctx: &mut Ctx, out: &mut Vec<Out>, now: Instant) -> Poll {
        if let Some(why) = self.failed.take() {
            return Poll::Failed(why);
        }
        if self.phase == Phase::Ready {
            return Poll::Ready;
        }
        if now.saturating_duration_since(self.since) > TIMEOUT {
            return Poll::Failed(format!("timeout ({})", self.state()));
        }
        if now < self.next || self.request.is_some() {
            return Poll::Pending;
        }
        self.next = now + POLL;
        let agent = ctx.agent;
        match self.phase {
            Phase::Inventory => {
                if world.inventory_writable_with_ais() {
                    self.enter(Phase::Shared, now);
                }
            }
            Phase::Shared => {
                let root = world.inventory.root;
                if !fetched(&mut world.inventory, root) {
                    return Poll::Pending;
                }
                // (`#RLV` is created only when the avatar has none)
                match world.rlva_shared_root() {
                    Some(id) => {
                        ctx.shared = id;
                        self.enter(Phase::Folder, now);
                    }
                    None if !self.made => self.mutate(
                        Mutation {
                            operations: vec![new_folder(root, "#RLV")],
                            refresh: vec![root],
                            ..Default::default()
                        },
                        out,
                    ),
                    None => {}
                }
            }
            Phase::Folder => {
                if !fetched(&mut world.inventory, ctx.shared) {
                    return Poll::Pending;
                }
                match child(&world.inventory, ctx.shared, FOLDER) {
                    Some(id) => {
                        ctx.root = id;
                        self.enter(Phase::Subfolders, now);
                    }
                    None if !self.made => self.mutate(
                        Mutation {
                            operations: vec![new_folder(ctx.shared, FOLDER)],
                            refresh: vec![ctx.shared],
                            ..Default::default()
                        },
                        out,
                    ),
                    None => {}
                }
            }
            Phase::Subfolders => {
                if !fetched(&mut world.inventory, ctx.root) {
                    return Poll::Pending;
                }
                let mut missing = Vec::new();
                for (i, name) in SUBFOLDERS.iter().enumerate() {
                    match child(&world.inventory, ctx.root, name) {
                        Some(id) => ctx.folders[i] = id,
                        None => missing.push(new_folder(ctx.root, name)),
                    }
                }
                if missing.is_empty() {
                    self.enter(Phase::Items, now);
                } else if !self.made {
                    self.mutate(
                        Mutation {
                            operations: missing,
                            refresh: vec![ctx.root],
                            ..Default::default()
                        },
                        out,
                    );
                }
            }
            Phase::Items => {
                let mut ready = true;
                for folder in ctx.folders {
                    ready &= fetched(&mut world.inventory, folder);
                }
                if !ready {
                    return Poll::Pending;
                }
                let mut missing = Vec::new();
                for (i, it) in It::ALL.into_iter().enumerate() {
                    let folder = it.sub().map_or(ctx.root, |sub| ctx.folders[sub]);
                    match object(&world.inventory, agent, folder, it.name()) {
                        Some(id) => ctx.items[i] = id,
                        None => missing.push((folder, it.name())),
                    }
                }
                if missing.is_empty() {
                    self.enter(Phase::Link, now);
                } else if !self.made {
                    match shared::library_object(&mut world.inventory, shared::ROCK) {
                        shared::LibraryObject::Found(rock) => {
                            let owner = world.inventory.lib_owner;
                            let mut refresh: Vec<Uuid> = missing.iter().map(|(folder, _)| *folder).collect();
                            refresh.dedup();
                            self.mutate(
                                Mutation {
                                    copies: missing
                                        .into_iter()
                                        .map(|(parent, name)| CopyItem {
                                            item: rock.id,
                                            owner,
                                            parent,
                                            name: name.to_owned(),
                                        })
                                        .collect(),
                                    refresh,
                                    ..Default::default()
                                },
                                out,
                            );
                        }
                        shared::LibraryObject::Missing => {
                            return Poll::Failed(format!("the Library object \"{}\" is unavailable", shared::ROCK));
                        }
                        shared::LibraryObject::Waiting => {}
                    }
                }
            }
            Phase::Link => {
                let (folder, target) = (ctx.folders[LINKED], ctx.item(It::B));
                if !fetched(&mut world.inventory, folder) {
                    return Poll::Pending;
                }
                let inv = &world.inventory;
                let linked = inv.folders.get(&folder).is_some_and(|f| {
                    f.items
                        .iter()
                        .filter_map(|id| inv.items.get(id))
                        .any(|i| i.asset_type == 24 && i.asset_id == target)
                });
                if linked {
                    self.enter(Phase::Wear, now);
                } else if !self.made {
                    self.mutate(
                        Mutation {
                            operations: vec![Operation::Links {
                                parent: folder,
                                links: vec![OutfitLink {
                                    target,
                                    name: It::B.name().to_owned(),
                                    desc: String::new(),
                                    folder: false,
                                    inv_type: 6,
                                }],
                            }],
                            refresh: vec![folder],
                            ..Default::default()
                        },
                        out,
                    );
                }
            }
            Phase::Wear => {
                let mut wear = Vec::new();
                for (_, it, point) in RELAYS {
                    let here = world
                        .worn_attachment(&ctx.item(it))
                        .and_then(|i| world.objects.get(i))
                        .is_some_and(|o| Some(o.key.region) == world.main_region);
                    if !here && let Some(item) = world.inventory.items.get(&ctx.item(it)) {
                        wear.push(shared::attach_request(item, agent, point));
                    }
                }
                if wear.is_empty() {
                    self.enter(Phase::Script(0), now);
                } else if !self.made {
                    self.made = true;
                    out.push(Out::Send(NetCommand::RezAttachments(wear)));
                }
            }
            Phase::Script(i) => {
                let (who, _, _) = RELAYS[i];
                let Some(object) = ctx.speaker_object(world, who) else {
                    return Poll::Failed("a relay is not worn any more".to_owned());
                };
                if who == Who::Controller {
                    ctx.controller = object;
                }
                if ctx.mode == Mode::Simulated {
                    self.enter(Phase::Hello(i), now);
                    return Poll::Pending;
                }
                let resave = self.repaired[i];
                let install = self.install.get_or_insert_with(|| ScriptInstall::new(object, now, resave));
                match install.step(world, out, now) {
                    Poll::Pending => self.next = now,
                    Poll::Ready => {
                        self.install = None;
                        self.enter(Phase::Hello(i), now);
                    }
                    Poll::Failed(why) => return Poll::Failed(format!("the relay script could not be written: {why}")),
                }
            }
            Phase::Hello(i) => {
                let (who, _, _) = RELAYS[i];
                let Some(object) = ctx.speaker_object(world, who) else {
                    return Poll::Failed("a relay is not worn any more".to_owned());
                };
                if self.made && ctx.echo.is_none() {
                    if i + 1 < RELAYS.len() {
                        self.enter(Phase::Script(i + 1), now);
                    } else {
                        self.enter(Phase::Ready, now);
                        return Poll::Ready;
                    }
                } else if now.saturating_duration_since(self.since) > HELLO_TIMEOUT {
                    ctx.echo = None;
                    // a script left by an earlier run that does not answer
                    // is written again, once
                    if ctx.mode == Mode::Worn && !self.repaired[i] {
                        self.repaired[i] = true;
                        self.enter(Phase::Script(i), now);
                    } else {
                        return Poll::Failed("the relay script does not answer".to_owned());
                    }
                } else if !self.made || now.saturating_duration_since(self.said) >= HELLO_EVERY {
                    self.made = true;
                    self.said = now;
                    ctx.echo = Some((object, HELLO.to_owned()));
                    out.push(ctx.say(world, object, HELLO));
                }
            }
            Phase::Ready => {}
        }
        Poll::Pending
    }

    /// The answers to the factory's own requests. True when the event is one.
    pub(super) fn event(&mut self, world: &mut World, ctx: &Ctx, event: &NetEvent, out: &mut Vec<Out>) -> bool {
        if let Some(install) = self.install.as_mut() {
            install.event(world, ctx, event, out);
        }
        match event {
            NetEvent::InventoryEdited { request, result } if self.request == Some(*request) => {
                match result {
                    Ok(reply) => {
                        world.inventory.apply(reply.contents.clone());
                        world.inventory.created_folders(reply.folders.clone());
                        if reply.copies.is_empty() {
                            self.request = None;
                        } else {
                            // (the copies finish with the same request id)
                            out.push(Out::Send(NetCommand::FinishInventoryCopies {
                                request: *request,
                                reply: reply.clone(),
                            }));
                        }
                    }
                    Err(why) => self.failed = Some(format!("the inventory refused a change: {why}")),
                }
                true
            }
            _ => false,
        }
    }
}

impl Ctx {
    /// The worn object of a relay, script or not.
    fn speaker_object(&self, world: &World, who: Who) -> Option<Uuid> {
        let it = RELAYS.iter().find(|(w, _, _)| *w == who).map(|(_, it, _)| *it)?;
        world
            .worn_attachment(&self.item(it))
            .and_then(|i| world.objects.get(i))
            .map(|o| o.full_id)
    }
}

/// Puts the relay script in an object of the test: lists its contents, adds
/// the script item if it is not there, writes the source and waits for the
/// compilation (the bridge's own sequence, bridge/factory.rs).
pub(super) struct ScriptInstall {
    object: Uuid,
    phase: Install,
    since: Instant,
    ask: Instant,
    item: Uuid,
    upload: Option<Uuid>,
    /// Write the source even when the script is already there.
    resave: bool,
    failed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Install {
    List,
    Rez,
    Upload,
    Done,
}

impl ScriptInstall {
    pub(super) fn new(object: Uuid, now: Instant, resave: bool) -> Self {
        Self {
            object,
            phase: Install::List,
            since: now,
            ask: now,
            item: Uuid::nil(),
            upload: None,
            resave,
            failed: None,
        }
    }

    pub(super) fn item(&self) -> Uuid {
        self.item
    }

    pub(super) fn step(&mut self, world: &World, out: &mut Vec<Out>, now: Instant) -> Poll {
        if let Some(why) = self.failed.take() {
            return Poll::Failed(why);
        }
        if self.phase == Install::Done {
            return Poll::Ready;
        }
        let Some(object) = world.objects.index_of_uuid(&self.object).and_then(|i| world.objects.get(i)) else {
            return Poll::Failed("the object is gone".to_owned());
        };
        if now.saturating_duration_since(self.since) > TIMEOUT {
            return Poll::Failed(format!("timeout ({:?})", self.phase));
        }
        if matches!(self.phase, Install::List | Install::Rez) && now >= self.ask {
            self.ask = now + Duration::from_secs(2);
            out.push(Out::Send(NetCommand::RequestTaskInventory {
                handle: object.key.region,
                local_id: object.key.local_id,
                object: self.object,
            }));
        }
        Poll::Pending
    }

    pub(super) fn event(&mut self, world: &World, ctx: &Ctx, event: &NetEvent, out: &mut Vec<Out>) {
        match event {
            NetEvent::TaskInventory {
                object, result: Ok(items), ..
            } if *object == self.object && matches!(self.phase, Install::List | Install::Rez) => {
                let Some(root) = world.objects.index_of_uuid(&self.object).and_then(|i| world.objects.get(i)) else {
                    return;
                };
                match items.iter().find(|i| i.name == RELAY_NAME && i.asset_type == 10) {
                    Some(script) => {
                        self.item = script.item_id;
                        if self.phase == Install::Rez || self.resave {
                            let request = Uuid::new_v4();
                            self.upload = Some(request);
                            self.phase = Install::Upload;
                            out.push(Out::Send(shared::save_script(
                                request,
                                root.key.region,
                                self.object,
                                script.item_id,
                                RELAY_SOURCE.to_owned(),
                            )));
                        } else {
                            self.phase = Install::Done;
                        }
                    }
                    None if self.phase == Install::List => {
                        self.phase = Install::Rez;
                        out.push(Out::Send(relay_item(world, ctx, root.key.region, root.key.local_id, self.object)));
                    }
                    None => {}
                }
            }
            NetEvent::ScriptSaved { request, result, .. } if self.upload == Some(*request) => {
                if result.as_ref().is_ok_and(|saved| saved.compiled) {
                    self.phase = Install::Done;
                } else {
                    self.failed = Some("the relay script did not compile".to_owned());
                }
            }
            _ => {}
        }
    }
}
