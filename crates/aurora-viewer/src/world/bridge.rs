//! Aurora's private attachment factory and confirmed Movelock state.
//! The library-copy/attach/script-upload sequence follows FSLSLBridge
//! (Firestorm indra/newview/fslslbridge.cpp, originally LGPL 2.1).
use super::{World, inventory::FetchState};
use aurora_net::bridge::{Ack, Command, Endpoint, Motion};
use aurora_net::build::{BuildCmd, TaskItem, perm};
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation};
use aurora_net::scripts::{ScriptOptions, ScriptTarget};
use aurora_net::{AttachRequest, ChatMessage, ChatSourceType, ChatType, NetCommand, NetEvent};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FOLDER: &str = "#Aurora";
pub const SUBFOLDER: &str = "Bridge";
pub const NAME: &str = "#Aurora Bridge v1.0";
// Stable inventory/script names let upgrades replace the existing controller
// instead of wearing two bridges with competing physics.
const VERSION: &str = "1.20";
const DISCOVERY_CHANNEL: i32 = -184735219;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(5);
const ROCK: &str = "Rock - medium, round";
const SOURCE: &str = include_str!("bridge/bridge.lsl");

pub mod services;
const TIMEOUT: Duration = Duration::from_secs(60);
// FSLSLBridge::processDetach and FSLSLBridgeReAttachTimer wait five seconds
// after detachment. An inventory asset UUID change is not a save receipt.
const STORE_DELAY: Duration = Duration::from_secs(5);
pub mod demo;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub indicator: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            indicator: true,
        }
    }
}
impl Preferences {
    pub fn state(&self) -> &'static str {
        if self.enabled {
            aurora_i18n::tr!("movelock-on")
        } else {
            aurora_i18n::tr!("movelock-off")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Inventory,
    Folder,
    Subfolder,
    Move,
    Copy,
    Attach,
    Listing,
    Script,
    Upload,
    Starting,
    Store,
    Restore,
    Handshake,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandshakeError {
    Format,
    Version,
    Folder,
    Endpoint,
    Token,
}
impl HandshakeError {
    fn message(self) -> &'static str {
        match self {
            Self::Format | Self::Token => aurora_i18n::tr!("bridge-response-invalid"),
            Self::Version => aurora_i18n::tr!("bridge-response-version"),
            Self::Folder => aurora_i18n::tr!("bridge-response-folder"),
            Self::Endpoint => aurora_i18n::tr!("bridge-response-endpoint"),
        }
    }
}

#[derive(Default)]
pub struct Bridge {
    agent: Uuid,
    region: Option<u64>,
    folder: Uuid,
    container: Uuid,
    item: Uuid,
    object: Uuid,
    phase: Phase,
    since: Option<Instant>,
    poll: Option<Instant>,
    probe: Option<(Uuid, Instant)>,
    refresh_inventory: bool,
    user_detached: bool,
    mutation: Option<Uuid>,
    upload: Option<Uuid>,
    script: Uuid,
    restarted: bool,
    checked_script: Option<Instant>,
    detached: Option<Instant>,
    boot: Option<(bool, bool)>,
    rejection: Option<HandshakeError>,
    candidate: Option<ChatMessage>,
    accepted: Option<(ChatMessage, Instant)>,
    endpoint: Option<Endpoint>,
    sequence: u32,
    pending: Option<(Uuid, u32, (bool, bool, Motion))>,
    sent: Option<(bool, bool, Motion)>,
    pub motion: Motion,
    last_send: Option<Instant>,
    capture_sent: Option<Instant>,
    firestorm_poll: Option<Instant>,
    firestorm_detached: HashSet<Uuid>,
    ack: Ack,
    pub error: Option<String>,
    pub commands: Vec<NetCommand>,
}

impl Bridge {
    /// The production receive path: private protocol must never reach chat,
    /// while ordinary world events (copies, detach) must still be applied.
    pub fn receive(&mut self, world: &mut World, event: NetEvent, now: Instant, received: Instant) -> Option<NetEvent> {
        if self.event(world, &event, now) {
            None
        } else {
            world.apply_received(event, received)
        }
    }
    pub fn folder(&self) -> Uuid {
        self.folder
    }
    pub fn locked(&self) -> bool {
        self.phase == Phase::Ready && self.ack.locked && self.error.is_none()
    }
    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready && self.endpoint.is_some()
    }
    pub fn available(&self) -> bool {
        self.ready() && self.error.is_none()
    }
    pub fn query_endpoint(&self) -> Option<Endpoint> {
        self.available().then(|| self.endpoint.clone()).flatten()
    }
    /// A failed read can reveal a replaced HTTP-in URL while movement is idle.
    /// Rediscovery is throttled and must not reset the motor or its pending ack.
    pub fn refresh_queries(&mut self, world: &World, now: Instant) {
        if self.available() {
            self.discover(world, now);
        }
    }
    pub fn diagnostic(&self) -> Option<&str> {
        self.error.as_deref().or_else(|| self.progress())
    }
    pub fn motor_report(&self, now: Instant) -> Option<aurora_net::bridge::MotorReport> {
        (self.available() && self.capture_sent.is_some_and(|at| now.duration_since(at) < Duration::from_secs(2)))
            .then_some(self.ack.motor)
            .flatten()
    }
    pub fn passage_report(&self) -> Option<aurora_net::bridge::PassageReport> {
        self.ready().then_some(self.ack.passage).flatten()
    }
    fn acknowledge(&mut self, mut ack: Ack) {
        // OwnerSay completion can arrive before the HTTP response generated
        // at motor startup. Do not let that response erase a terminal result.
        if let Some(previous) = self.ack.passage
            && previous.finished()
            && ack
                .passage
                .is_some_and(|report| report.action == previous.action && !report.finished())
        {
            ack.passage = Some(previous);
        }
        self.ack = ack;
    }
    /// Provisioning diagnostics are separate from the user's on/off selection.
    /// Physical convergence, movement and background replies never change it.
    pub fn progress(&self) -> Option<&'static str> {
        if self.error.is_some() {
            return Some(aurora_i18n::tr!("movelock-failed"));
        }
        if self.ready() {
            return None;
        }
        if let Some(rejection) = self.rejection {
            return Some(rejection.message());
        }
        Some(match self.phase {
            Phase::Starting | Phase::Handshake => match self.boot {
                Some((false, _)) => aurora_i18n::tr!("bridge-waiting-permissions"),
                Some((true, false)) => aurora_i18n::tr!("bridge-waiting-url"),
                Some((true, true)) => aurora_i18n::tr!("bridge-waiting-handshake"),
                _ => aurora_i18n::tr!("bridge-starting"),
            },
            Phase::Store => aurora_i18n::tr!("bridge-saving"),
            _ => aurora_i18n::tr!("bridge-preparing"),
        })
    }
    /// The acknowledged swimming mode captures native translation in LSL.
    /// Stop forwarding vertical swim controls before its 1.5 s watchdog can
    /// release that capture, even when HTTP replies are delayed or lost.
    pub fn captures_motion(&self, now: Instant) -> bool {
        self.available()
            && !self.ack.seated
            && matches!(self.motion.mode, 3 | 6 | 7 | 8 | 9 | 10)
            && self.sent.is_some_and(|mode| mode.2.mode == self.motion.mode)
            && self.capture_sent.is_some_and(|at| now.duration_since(at) < Duration::from_secs(1))
    }
    fn enter(&mut self, phase: Phase, now: Instant) {
        log::info!("Aurora bridge: {:?} -> {:?}", self.phase, phase);
        self.phase = phase;
        self.since = Some(now);
        self.poll = None;
        self.checked_script = None;
        self.restarted = false;
        if phase == Phase::Upload {
            self.boot = None;
            self.rejection = None;
        }
    }
    fn fail(&mut self, reason: String) {
        // Never log protocol messages, endpoints, source or identities.
        log::warn!("Aurora bridge: timeout or failure in {:?}", self.phase);
        self.phase = Phase::Failed;
        self.error = Some(reason);
        self.ack.locked = false;
        self.pending = None;
    }
    pub fn retry(&mut self, now: Instant) {
        self.user_detached = false;
        self.mutation = None;
        self.upload = None;
        self.commands.clear();
        self.refresh_inventory = true;
        self.probe = None;
        self.endpoint = None;
        self.pending = None;
        self.sent = None;
        self.ack = Ack::default();
        self.capture_sent = None;
        self.error = None;
        self.candidate = None;
        self.accepted = None;
        self.boot = None;
        self.rejection = None;
        self.detached = None;
        self.enter(Phase::Inventory, now);
    }
    /// Distinguish an explicit inventory/outfit detach from the factory's
    /// internal save. Region/object loss alone is not evidence of user intent.
    /// Return true when the app must turn off its requested Movelock mode.
    pub fn observe_command(&mut self, command: &NetCommand, now: Instant) -> bool {
        match command {
            NetCommand::DetachAttachments(items)
                if !self.item.is_nil() && !self.user_detached && items.contains(&self.item) && self.phase != Phase::Store =>
            {
                let release = self.release();
                self.retry(now);
                self.user_detached = true;
                self.error = Some(aurora_i18n::tr!("bridge-detached").into());
                self.enter(Phase::Failed, now);
                self.commands.extend(release);
                true
            }
            NetCommand::RezAttachments(items) if self.user_detached && items.iter().any(|item| item.item_id == self.item) => {
                self.retry(now);
                false
            }
            _ => false,
        }
    }
    pub fn release(&mut self) -> Option<NetCommand> {
        let endpoint = self.endpoint.take()?;
        self.sequence = self.sequence.saturating_add(1);
        self.pending = None;
        self.commands.clear();
        self.phase = Phase::Failed;
        self.ack = Ack::default();
        self.capture_sent = None;
        Some(NetCommand::Bridge {
            request: Uuid::new_v4(),
            command: Command {
                endpoint,
                sequence: self.sequence,
                enabled: false,
                relock: true,
                motion: Motion::default(),
            },
        })
    }
    fn mutate(&mut self, change: Mutation, phase: Phase, now: Instant) {
        let request = Uuid::new_v4();
        self.mutation = Some(request);
        self.commands.push(NetCommand::EditInventory { request, change });
        self.enter(phase, now);
    }
    fn attach(&mut self, world: &World, phase: Phase, now: Instant) {
        let Some(item) = world.inventory.items.get(&self.item) else {
            return;
        };
        self.commands.push(NetCommand::RezAttachments(vec![AttachRequest {
            item_id: item.id,
            owner_id: world.agent_id,
            point: 31,
            add: true,
            flags: item.flags,
            group_mask: item.group_mask,
            everyone_mask: item.everyone_mask,
            next_owner_mask: item.next_owner_mask,
            name: item.name.clone(),
            desc: item.desc.clone(),
        }]));
        self.enter(phase, now);
    }
    fn detach_firestorm(&mut self, world: &World, now: Instant) {
        if self
            .firestorm_poll
            .is_some_and(|at| now.duration_since(at) < Duration::from_secs(1))
        {
            return;
        }
        self.firestorm_poll = Some(now);
        let inv = &world.inventory;
        let Some(folder) = inv.folders.get(&inv.root).and_then(|root| {
            root.children.iter().copied().find(|id| {
                inv.folders
                    .get(id)
                    .is_some_and(|f| f.info.parent == inv.root && f.info.name == "#Firestorm" && !f.library)
            })
        }) else {
            return;
        };
        let mut detach = Vec::new();
        for item in world.worn_attachment_items() {
            let Some(entry) = inv.items.get(&item) else { continue };
            // FS_BRIDGE_NAME and FS_BRIDGE_POINT (fslslbridge.h). Exact
            // version syntax, inventory ancestry and a worn root are required;
            // a chat tag or a similarly named ordinary HUD is insufficient.
            let version = entry.name.strip_prefix("#Firestorm LSL Bridge v").and_then(|v| v.split_once('.'));
            if entry.owner != world.agent_id
                || entry.asset_type != 6
                || !version.is_some_and(|(major, minor)| {
                    [major, minor]
                        .iter()
                        .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                })
                || !super::inventory::actions::under(inv, item, folder)
            {
                continue;
            }
            if let Some(root) = world.worn_attachment(&item).and_then(|i| world.objects.get(i))
                && root.attachment_point() == 31
                && self.firestorm_detached.insert(root.full_id)
            {
                detach.push(item);
            }
        }
        if !detach.is_empty() {
            // Detach only. Never edit/move/delete its inventory item, folders,
            // contents or script, and never authenticate the legacy bridge.
            self.commands.push(NetCommand::DetachAttachments(detach));
        }
    }
    fn request_contents(&mut self, world: &World) {
        if let Some(object) = world.objects.index_of_uuid(&self.object).and_then(|i| world.objects.get(i)) {
            self.commands.push(NetCommand::RequestTaskInventory {
                handle: object.key.region,
                local_id: object.key.local_id,
                object: object.full_id,
            });
        }
    }
    fn discover(&mut self, world: &World, now: Instant) {
        let Some(object) = world.worn_attachment(&self.item).and_then(|i| world.objects.get(i)) else {
            return;
        };
        if object.full_id != self.object
            || Some(object.key.region) != world.main_region
            || self
                .probe
                .is_some_and(|(id, at)| id == self.object && now.duration_since(at) < DISCOVERY_INTERVAL)
        {
            return;
        }
        // Firestorm really_send_chat_from_viewer (fsnearbychathub.cpp):
        // ChatFromViewer is rejected on negative channels. ScriptDialogReply
        // with the agent as ObjectID emits avatar chat without opening UI.
        // llListen is filtered to the wearer and this root's exact message.
        // No endpoint/token or public chat, and no physical command in this
        // request. A quiet attachment can be rediscovered after any login.
        self.probe = Some((self.object, now));
        self.commands.push(NetCommand::ScriptDialogReply {
            object_id: self.agent,
            channel: DISCOVERY_CHANNEL,
            index: 0,
            label: format!("auroraHello|{}", self.object),
        });
    }
    fn accept_candidate(&mut self, world: &World, now: Instant) -> bool {
        let Some(chat) = self.candidate.as_ref() else { return false };
        // An inventory name or OwnerSay on its own is insufficient: this
        // must be the exact root worn by us from the selected inventory item.
        if !world
            .worn_attachment(&self.item)
            .and_then(|i| world.objects.get(i))
            .is_some_and(|o| o.full_id == chat.source_id && Some(o.key.region) == world.main_region)
        {
            return false;
        }
        let Ok((folder, endpoint)) = handshake(&chat.message) else {
            return false;
        };
        if folder != self.folder {
            self.rejection = Some(HandshakeError::Folder);
            return false;
        }
        self.rejection = None;
        if self.ready()
            && self
                .endpoint
                .as_ref()
                .is_some_and(|current| current.url == endpoint.url && current.token == endpoint.token)
        {
            // A repeated hello can already be in flight when the first POST
            // stops the retry timer. It must not erase a confirmed lock or
            // abandon the request whose acknowledgement is still pending.
            self.candidate = None;
            return false;
        }
        if self.phase == Phase::Starting {
            // Compilation is not proof that state_entry/permissions/HTTP-in
            // have run. Let the new script finish startup before saving it.
            self.detached = None;
            self.candidate = None;
            self.endpoint = None;
            self.boot = None;
            self.commands.push(NetCommand::DetachAttachments(vec![self.item]));
            self.enter(Phase::Store, now);
            return true;
        }
        self.object = chat.source_id;
        self.accepted = Some((chat.clone(), now));
        self.endpoint = Some(endpoint);
        self.candidate = None;
        self.sent = None;
        self.pending = None;
        self.last_send = None;
        self.ack = Ack::default();
        self.capture_sent = None;
        self.error = None;
        self.rejection = None;
        self.enter(Phase::Ready, now);
        true
    }
    pub fn update(&mut self, world: &mut World, prefs: &Preferences, online: bool, now: Instant) {
        if !online || world.agent_id.is_nil() || world.main_region.is_none() {
            return;
        }
        if self.agent != world.agent_id {
            let candidate = self.candidate.take();
            *self = Self {
                agent: world.agent_id,
                candidate,
                since: Some(now),
                ..Default::default()
            };
        }
        self.detach_firestorm(world, now);
        if self.user_detached {
            self.region = world.main_region;
            return;
        }
        let region_changed = self.region.is_some() && self.region != world.main_region;
        if region_changed {
            self.probe = None;
            self.endpoint = None;
            self.pending = None;
            self.sent = None;
            self.ack = Ack::default();
            self.capture_sent = None;
            if self.candidate.is_none() {
                self.candidate = self
                    .accepted
                    .as_ref()
                    .filter(|(_, at)| now.duration_since(*at) < Duration::from_secs(3))
                    .map(|(chat, _)| chat.clone());
            }
            self.upload = None;
            // An explicit resume/repair must still discover and rez an absent
            // inventory item, even when the region changed while paused.
            self.enter(
                if self.item.is_nil() || self.phase == Phase::Inventory {
                    Phase::Inventory
                } else {
                    Phase::Attach
                },
                now,
            );
        }
        self.region = world.main_region;
        if self.refresh_inventory {
            // A timed-out mutation may have succeeded. Refetch before any
            // creation so Repair cannot duplicate an unacknowledged copy.
            for id in [world.inventory.root, self.container, self.folder] {
                if let Some(folder) = world.inventory.folders.get_mut(&id) {
                    folder.state = FetchState::Unknown;
                }
            }
            self.refresh_inventory = false;
        }
        // Ready mode only walks our attachment list; provisioning polls at
        // most four times a second, never scanning the entire inventory per frame.
        if matches!(
            self.phase,
            Phase::Attach | Phase::Listing | Phase::Starting | Phase::Handshake | Phase::Ready
        ) && self.accept_candidate(world, now)
        {
            if self.ready() {
                self.send_mode(prefs, now);
            }
            return;
        }
        if self.phase == Phase::Ready {
            if world.worn_attachment(&self.item).is_none() {
                self.endpoint = None;
                self.pending = None;
                self.ack = Ack::default();
                self.capture_sent = None;
                self.attach(world, Phase::Attach, now);
            } else {
                if self.error.is_some() {
                    self.discover(world, now);
                }
                self.send_mode(prefs, now);
            }
            return;
        }
        if self.phase == Phase::Failed {
            return;
        }
        if self.since.is_some_and(|since| now.duration_since(since) > TIMEOUT) {
            self.fail(self.timeout_message());
            return;
        }
        if self.poll.is_some_and(|poll| now.duration_since(poll) < Duration::from_millis(250)) {
            return;
        }
        self.poll = Some(now);
        match self.phase {
            Phase::Inventory | Phase::Folder | Phase::Subfolder | Phase::Move | Phase::Copy => {
                if self.mutation.is_some() {
                    return;
                }
                let worn = world.worn_attachment_items();
                let inv = &mut world.inventory;
                if inv.root.is_nil() || inv.held.is_some() {
                    return;
                }
                inv.request(inv.root);
                let Some(root) = inv.folders.get(&inv.root).filter(|f| f.state == FetchState::Fetched) else {
                    return;
                };
                let folder = root
                    .children
                    .iter()
                    .filter_map(|id| inv.folders.get(id))
                    .filter(|f| !f.library && f.info.name == FOLDER)
                    .map(|f| f.info.id)
                    .min();
                let Some(folder) = folder else {
                    if self.phase == Phase::Inventory {
                        self.mutate(
                            Mutation {
                                operations: vec![Operation::Folder {
                                    id: Uuid::new_v4(),
                                    parent: inv.root,
                                    name: FOLDER.into(),
                                    type_default: -1,
                                }],
                                refresh: vec![inv.root],
                                ..Default::default()
                            },
                            Phase::Folder,
                            now,
                        );
                    }
                    return;
                };
                self.container = folder;
                inv.request(folder);
                let Some(container) = inv.folders.get(&folder).filter(|f| f.state == FetchState::Fetched) else {
                    return;
                };
                let subfolder = container
                    .children
                    .iter()
                    .filter_map(|id| inv.folders.get(id))
                    .filter(|f| !f.library && f.info.name == SUBFOLDER)
                    .map(|f| f.info.id)
                    .min();
                let Some(folder) = subfolder else {
                    if self.phase != Phase::Subfolder {
                        self.mutate(
                            Mutation {
                                operations: vec![Operation::Folder {
                                    id: Uuid::new_v4(),
                                    parent: folder,
                                    name: SUBFOLDER.into(),
                                    type_default: -1,
                                }],
                                refresh: vec![folder],
                                ..Default::default()
                            },
                            Phase::Subfolder,
                            now,
                        );
                    }
                    return;
                };
                self.folder = folder;
                inv.request(folder);
                let Some(contents) = inv.folders.get(&folder).filter(|f| f.state == FetchState::Fetched) else {
                    return;
                };
                // Move the first version's existing bridge and strokes. Never
                // copy another worn physics controller during the migration.
                let old: Vec<_> = inv
                    .folders
                    .get(&self.container)
                    .into_iter()
                    .flat_map(|f| f.items.iter())
                    .filter_map(|id| inv.items.get(id))
                    .filter(|item| {
                        item.owner == world.agent_id
                            && ((item.asset_type == 6 && item.name == NAME)
                                || (item.asset_type == 20
                                    && (crate::movement::Style::ALL.iter().any(|style| item.name == style.name())
                                        || crate::movement::strokes::WATER_CLIPS.iter().any(|(_, name)| item.name == *name))))
                    })
                    .map(|item| item.id)
                    .collect();
                if !old.is_empty() {
                    if self.phase != Phase::Move {
                        self.mutate(
                            Mutation {
                                operations: old
                                    .into_iter()
                                    .map(|id| Operation::Patch {
                                        id,
                                        folder: false,
                                        body: aurora_llsd::llsd_map! { "parent_id" => folder },
                                    })
                                    .collect(),
                                refresh: vec![self.container, folder],
                                ..Default::default()
                            },
                            Phase::Move,
                            now,
                        );
                    }
                    return;
                }
                if let Some(item) = contents
                    .items
                    .iter()
                    .filter_map(|id| inv.items.get(id))
                    .filter(|i| i.asset_type == 6 && i.name == NAME && i.owner == world.agent_id)
                    .min_by_key(|i| (!worn.contains(&i.id), i.id))
                {
                    self.item = item.id;
                    if let Some(object) = world.worn_attachment(&self.item).and_then(|i| world.objects.get(i)) {
                        self.object = object.full_id;
                        self.enter(Phase::Attach, now);
                    } else {
                        self.attach(world, Phase::Attach, now);
                    }
                    return;
                }
                if self.phase == Phase::Copy {
                    return;
                }
                // Library layouts vary. Search its metadata only, with no
                // more than eight folder requests in flight, never user folders.
                let library: Vec<_> = inv.folders.values().filter(|f| f.library).map(|f| f.info.id).collect();
                let rock = library
                    .iter()
                    .filter_map(|id| inv.folders.get(id))
                    .flat_map(|f| f.items.iter())
                    .filter_map(|id| inv.items.get(id))
                    .find(|i| {
                        i.name == ROCK && i.asset_type == 6 && i.owner_mask & (perm::COPY | perm::MODIFY) == (perm::COPY | perm::MODIFY)
                    })
                    .cloned();
                if let Some(rock) = rock {
                    self.mutate(
                        Mutation {
                            copies: vec![CopyItem {
                                item: rock.id,
                                owner: inv.lib_owner,
                                parent: folder,
                                name: NAME.into(),
                            }],
                            refresh: vec![folder],
                            ..Default::default()
                        },
                        Phase::Copy,
                        now,
                    );
                } else if !library.is_empty()
                    && library
                        .iter()
                        .all(|id| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetched))
                {
                    self.fail(aurora_i18n::tr!("bridge-library-missing").into());
                } else {
                    let in_flight = library
                        .iter()
                        .filter(|id| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetching))
                        .count();
                    let mut waiting: Vec<_> = library
                        .iter()
                        .filter_map(|id| inv.folders.get(id))
                        .filter(|f| matches!(f.state, FetchState::Unknown | FetchState::Failed))
                        .map(|f| (f.info.name != "Objects" && !f.info.name.contains("Rock"), f.info.id))
                        .collect();
                    waiting.sort_unstable();
                    for (_, id) in waiting.into_iter().take(8usize.saturating_sub(in_flight)) {
                        inv.request(id);
                    }
                }
            }
            Phase::Attach | Phase::Restore => {
                if let Some(object) = world.worn_attachment(&self.item).and_then(|i| world.objects.get(i)) {
                    if Some(object.key.region) != world.main_region {
                        return;
                    }
                    self.object = object.full_id;
                    if self.phase == Phase::Restore {
                        self.enter(Phase::Handshake, now);
                    } else {
                        // Keep the freshly copied library object unobtrusive
                        // while the disabled script is installed and compiled.
                        self.commands.push(NetCommand::Build(BuildCmd::Transform {
                            handle: object.key.region,
                            updates: vec![aurora_net::build::TransformUpdate {
                                local_id: object.key.local_id,
                                position: None,
                                rotation: None,
                                scale: Some(glam::Vec3::splat(0.01)),
                                linked: true,
                                uniform: false,
                            }],
                        }));
                        self.enter(Phase::Listing, now);
                        self.request_contents(world);
                    }
                }
            }
            Phase::Script => {
                if self.since.is_some_and(|since| now.duration_since(since).as_millis() % 1000 < 250) {
                    self.request_contents(world);
                }
            }
            Phase::Starting | Phase::Handshake => {
                self.discover(world, now);
                // An upload's running flag can be lost when the simulator
                // restores a saved attachment. Query the actual task state;
                // one bounded restart repairs that without rebuilding it.
                if !self.script.is_nil()
                    && self.since.is_some_and(|since| now.duration_since(since) >= Duration::from_secs(2))
                    && self.checked_script.is_none_or(|at| now.duration_since(at) >= STORE_DELAY)
                    && let Some(object) = world.objects.index_of_uuid(&self.object).and_then(|i| world.objects.get(i))
                {
                    self.checked_script = Some(now);
                    self.commands.push(NetCommand::Build(BuildCmd::GetScriptRunning {
                        handle: object.key.region,
                        object_id: self.object,
                        item_id: self.script,
                    }));
                }
            }
            Phase::Store if world.worn_attachment(&self.item).is_none() => {
                let detached = *self.detached.get_or_insert_with(|| {
                    log::info!("Aurora bridge: detachment observed; waiting for simulator save delay");
                    now
                });
                if now.duration_since(detached) >= STORE_DELAY {
                    self.candidate = None;
                    self.attach(world, Phase::Restore, now);
                } else if self
                    .checked_script
                    .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(2))
                {
                    self.checked_script = Some(now);
                    self.commands.push(NetCommand::FetchItems {
                        items: vec![self.item],
                        owner: world.agent_id,
                    });
                }
            }
            _ => {}
        }
    }
    fn timeout_message(&self) -> String {
        let stage = self.rejection.map(HandshakeError::message).unwrap_or_else(|| match self.phase {
            Phase::Inventory | Phase::Folder | Phase::Subfolder | Phase::Move | Phase::Copy => aurora_i18n::tr!("bridge-stage-inventory"),
            Phase::Attach | Phase::Restore => aurora_i18n::tr!("bridge-stage-attach"),
            Phase::Listing | Phase::Script => aurora_i18n::tr!("bridge-stage-contents"),
            Phase::Upload => aurora_i18n::tr!("bridge-stage-compile"),
            Phase::Store => aurora_i18n::tr!("bridge-saving"),
            _ => match self.boot {
                Some((false, _)) => aurora_i18n::tr!("bridge-waiting-permissions"),
                Some((true, false)) => aurora_i18n::tr!("bridge-waiting-url"),
                Some((true, true)) => aurora_i18n::tr!("bridge-waiting-handshake"),
                _ => aurora_i18n::tr!("bridge-starting"),
            },
        });
        aurora_i18n::tr!("bridge-timeout-stage", stage = stage.to_owned())
    }
    fn send_mode(&mut self, prefs: &Preferences, now: Instant) {
        if self.error.is_some() && self.last_send.is_some_and(|last| now.duration_since(last) < Duration::from_secs(5)) {
            return;
        }
        let motion = if self.motion.valid() { self.motion } else { Motion::default() };
        let mode = (prefs.enabled, true, motion);
        // Release / lock transitions must not sit behind a slow HTTP reply.
        // A higher sequence supersedes the older command on the simulator;
        // its late acknowledgement is ignored by the receive path.
        if let Some((_, _, pending)) = self.pending
            && (pending.0 != mode.0 || pending.1 != mode.1 || pending.2.mode != motion.mode)
        {
            self.pending = None;
            self.sent = None;
        }
        let interval = Duration::from_millis(500);
        if self.pending.is_some()
            || (self
                .last_send
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500))
                && self
                    .sent
                    .is_some_and(|last| last.0 == mode.0 && last.1 == mode.1 && last.2.mode == mode.2.mode))
            || (self.sent == Some(mode) && (motion.mode == 0 || self.last_send.is_some_and(|last| now.duration_since(last) < interval)))
        {
            return;
        }
        let Some(endpoint) = self.endpoint.clone() else { return };
        self.sequence = self.sequence.saturating_add(1);
        let request = Uuid::new_v4();
        self.pending = Some((request, self.sequence, mode));
        self.last_send = Some(now);
        self.commands.push(NetCommand::Bridge {
            request,
            command: Command {
                endpoint,
                sequence: self.sequence,
                enabled: prefs.enabled,
                relock: true,
                motion,
            },
        });
    }
    /// Return true only for events consumed by this bridge. Never show an
    /// Aurora handshake, token, endpoint or script's raw protocol in chat.
    pub fn event(&mut self, world: &mut World, event: &NetEvent, now: Instant) -> bool {
        match event {
            NetEvent::Chat(chat)
                if chat.source_type == ChatSourceType::Object
                    && chat.chat_type == ChatType::OwnerSay
                    && ["<auroraBridge>", "<auroraMoveLock>", "<auroraBridgeError>", "<auroraBridgeStatus>"]
                        .iter()
                        .any(|tag| chat.message.starts_with(tag)) =>
            {
                if chat.owner_id == world.agent_id && chat.message.len() <= 1200 {
                    if chat.message.starts_with("<auroraBridge>") {
                        match handshake(&chat.message) {
                            Ok(_) => self.candidate = Some(chat.clone()),
                            Err(rejection) if chat.source_id == self.object && self.rejection != Some(rejection) => {
                                // A rejected URL is not a stopped script. Only
                                // record the reason, never the private reply.
                                log::warn!("Aurora bridge: rejected response ({rejection:?})");
                                self.rejection = Some(rejection);
                            }
                            Err(_) => {}
                        }
                    } else if chat.source_id == self.object {
                        if let Some(boot) = boot_status(&chat.message, self.folder)
                            && world
                                .worn_attachment(&self.item)
                                .and_then(|i| world.objects.get(i))
                                .is_some_and(|object| object.full_id == chat.source_id && Some(object.key.region) == world.main_region)
                        {
                            if self.boot != Some(boot) {
                                log::info!("Aurora bridge: script status permissions={} url={}", boot.0, boot.1);
                            }
                            self.boot = Some(boot);
                        }
                        if let Some(ack) = chat
                            .message
                            .strip_prefix("<auroraMoveLock>")
                            .and_then(|s| s.strip_suffix("</auroraMoveLock>"))
                            .and_then(Ack::parse)
                            && ack.sequence == self.sequence
                            && self.ready()
                        {
                            self.acknowledge(ack);
                        }
                        if chat.message == "<auroraBridgeError>url</auroraBridgeError>" {
                            self.error = Some(aurora_i18n::tr!("bridge-url-unavailable").into());
                        }
                    }
                }
                true
            }
            NetEvent::InventoryEdited { request, result } if self.mutation == Some(*request) => {
                match result {
                    Ok(reply) => {
                        world.inventory.apply(reply.contents.clone());
                        world.inventory.created_folders(reply.folders.clone());
                        if !reply.copies.is_empty() {
                            self.commands.push(NetCommand::FinishInventoryCopies {
                                request: *request,
                                reply: reply.clone(),
                            });
                        } else {
                            self.mutation = None;
                        }
                    }
                    Err(_) => {
                        self.mutation = None;
                        self.fail(aurora_i18n::tr!("bridge-inventory-failed").into());
                    }
                }
                true
            }
            NetEvent::TaskInventory { object, result, .. }
                if *object == self.object && matches!(self.phase, Phase::Listing | Phase::Script) =>
            {
                let Ok(items) = result else {
                    self.fail(aurora_i18n::tr!("bridge-script-failed").into());
                    return true;
                };
                let Some(root) = world.objects.index_of_uuid(object).and_then(|i| world.objects.get(i)) else {
                    return true;
                };
                if let Some(script) = items.iter().find(|i| i.asset_type == 10 && i.name == NAME) {
                    self.script = script.item_id;
                    let request = Uuid::new_v4();
                    self.upload = Some(request);
                    self.commands.push(NetCommand::SaveScript {
                        request,
                        target: ScriptTarget {
                            object: *object,
                            item: script.item_id,
                        },
                        handle: root.key.region,
                        options: ScriptOptions {
                            mono: true,
                            running: true,
                            experience: Uuid::nil(),
                        },
                        source: SOURCE.replace("__FOLDER__", &self.folder.to_string()),
                    });
                    self.enter(Phase::Upload, now);
                } else if self.phase == Phase::Listing {
                    self.commands.push(NetCommand::Build(BuildCmd::RezScript {
                        handle: root.key.region,
                        local_id: root.key.local_id,
                        enabled: false,
                        group_id: Uuid::nil(),
                        item: Box::new(TaskItem {
                            parent_id: *object,
                            creator_id: world.agent_id,
                            owner_id: world.agent_id,
                            base_mask: 0x7fff_ffff,
                            owner_mask: 0x7fff_ffff,
                            next_owner_mask: perm::COPY | perm::MODIFY | perm::MOVE,
                            asset_type: 10,
                            inv_type: 10,
                            name: NAME.into(),
                            description: "Aurora's private bridge".into(),
                            ..Default::default()
                        }),
                    }));
                    self.enter(Phase::Script, now);
                }
                true
            }
            NetEvent::ScriptSaved { request, result, .. } if self.upload == Some(*request) => {
                self.upload = None;
                if result.as_ref().is_ok_and(|r| r.compiled) {
                    // An early hello can arrive before the HTTP upload reply.
                    // Keep it; Starting validates it against the exact object.
                    self.endpoint = None;
                    self.enter(Phase::Starting, now);
                } else {
                    self.fail(aurora_i18n::tr!("bridge-script-failed").into());
                }
                true
            }
            NetEvent::ScriptRunning {
                object_id,
                item_id,
                running: false,
                ..
            } if *object_id == self.object
                && *item_id == self.script
                && matches!(self.phase, Phase::Starting | Phase::Handshake)
                && !self.restarted =>
            {
                if let Some(object) = world.objects.index_of_uuid(&self.object).and_then(|i| world.objects.get(i)) {
                    self.restarted = true;
                    self.commands.push(NetCommand::Build(BuildCmd::SetScriptRunning {
                        handle: object.key.region,
                        object_id: self.object,
                        item_id: self.script,
                        running: true,
                    }));
                }
                true
            }
            NetEvent::BridgeReply { request, result } => {
                if let Some((pending, sequence, mode)) = self.pending
                    && *request == pending
                {
                    self.pending = None;
                    match result {
                        Ok(ack) if ack.sequence == sequence && ack.enabled == mode.0 => {
                            self.acknowledge(*ack);
                            // Use the send time as a conservative lower bound
                            // for the script's motion watchdog, not response time.
                            self.capture_sent = self.last_send;
                            self.sent = Some(mode);
                            self.error = None;
                        }
                        _ => {
                            self.sent = None;
                            self.ack.locked = false;
                            self.error = Some(match result {
                                Err(reason) if !reason.is_empty() => reason.clone(),
                                _ => aurora_i18n::tr!("bridge-request-failed").into(),
                            });
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}

fn boot_status(text: &str, folder: Uuid) -> Option<(bool, bool)> {
    let fields: Vec<_> = text
        .strip_prefix("<auroraBridgeStatus>")?
        .strip_suffix("</auroraBridgeStatus>")?
        .split('|')
        .collect();
    if fields.len() != 4 || fields[0] != VERSION || fields[1].parse::<Uuid>().ok()? != folder || folder.is_nil() {
        return None;
    }
    let flag = |value| match value {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    };
    Some((flag(fields[2])?, flag(fields[3])?))
}

fn handshake(text: &str) -> Result<(Uuid, Endpoint), HandshakeError> {
    let body = text
        .strip_prefix("<auroraBridge>")
        .and_then(|text| text.strip_suffix("</auroraBridge>"))
        .ok_or(HandshakeError::Format)?;
    let parts: Vec<_> = body.split('|').collect();
    if parts.len() != 4 {
        return Err(HandshakeError::Format);
    }
    if parts[0] != VERSION {
        return Err(HandshakeError::Version);
    }
    if !aurora_net::bridge::valid_url(parts[2]) {
        return Err(HandshakeError::Endpoint);
    }
    let folder = Uuid::parse_str(parts[1])
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or(HandshakeError::Folder)?;
    let token = Uuid::parse_str(parts[3])
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or(HandshakeError::Token)?;
    Ok((
        folder,
        Endpoint {
            url: parts[2].into(),
            token,
        },
    ))
}
