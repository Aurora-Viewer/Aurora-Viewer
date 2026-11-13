//! Main-thread bridge between the `aurora-mcp` async server and `App` (`World`,
//! `Scene`, `Camera`, `Settings`, `Poser`, `Build`, and `Renderer`).
//!
//! Enforces a strict `1.5ms` per-frame execution budget so MCP tool traffic
//! never degrades the viewer's frame rate.

use super::*;
use aurora_mcp::{
    AppearanceOperation, BoneTransform, CameraCommand, CaptureBufferType, ChatKind,
    FRAME_BUDGET_MICROS, LinksetDataStore, LsdOperation, McpError, McpRequestEnvelope,
    McpServerHandle, PrimTransformSpec, RawCapturedFrame, RollbackAction, ViewerRequest,
    ViewerResponse, create_bridge_channels, spawn_server,
};
use aurora_net::build::{BuildCmd, NewPrim, TransformUpdate, perm};
use aurora_net::land::{AccessEntry, LandCommand, ParcelUpdate, AL_ACCESS, AL_BAN};
use aurora_net::scripts::{ScriptOptions, ScriptTarget};
use glam::{EulerRot, Quat, Vec3};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Hard per-frame budget for draining MCP requests on the main UI thread.
const MCP_FRAME_BUDGET: Duration = Duration::from_micros(FRAME_BUDGET_MICROS);

/// Maximum number of subscribed viewer events buffered for `events_read`.
const MAX_EVENTS_BUFFER: usize = 500;

/// Entry in the serialized COF (Current Outfit Folder) mutation queue.
pub struct QueuedCofMutation {
    pub operation: AppearanceOperation,
    pub reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Active in-flight COF mutation waiting for the viewer's outfit state to settle.
pub struct ActiveCofMutation {
    pub operation: AppearanceOperation,
    pub started_at: Instant,
    pub reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// State machine ensuring COF (Current Outfit Folder) mutations never overlap.
#[derive(Default)]
pub struct CofMutationQueue {
    pub queue: VecDeque<QueuedCofMutation>,
    pub active: Option<ActiveCofMutation>,
}

impl CofMutationQueue {
    pub fn enqueue(
        &mut self,
        operation: AppearanceOperation,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        self.queue.push_back(QueuedCofMutation {
            operation,
            reply_tx,
        });
    }

    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }
}

/// Pending object rez transaction awaiting simulator `ObjectUpdate` confirmation.
pub struct PendingRezTransaction {
    pub expected_pos: [f32; 3],
    pub name: Option<String>,
    pub known_local_ids: HashSet<u32>,
    pub started_at: Instant,
    pub reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Correlates asynchronous simulator object creation (`BuildCmd::Add`) back to the
/// originating MCP `prim_rez_new` request.
#[derive(Default)]
pub struct BuildTransactionManager {
    pub pending_rez: VecDeque<PendingRezTransaction>,
}

impl BuildTransactionManager {
    pub fn register_rez(
        &mut self,
        expected_pos: [f32; 3],
        name: Option<String>,
        known_local_ids: HashSet<u32>,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        self.pending_rez.push_back(PendingRezTransaction {
            expected_pos,
            name,
            known_local_ids,
            started_at: Instant::now(),
            reply_tx,
        });
    }

    /// Checks whether a candidate object matches the oldest pending rez within `0.5m`.
    pub fn try_match_candidate(
        &mut self,
        local_id: u32,
        object_uuid: Uuid,
        pos: [f32; 3],
    ) -> Option<(Option<String>, oneshot::Sender<Result<ViewerResponse, McpError>>)> {
        let idx = self.pending_rez.iter().position(|tx| {
            if tx.known_local_ids.contains(&local_id) {
                return false;
            }
            let dx = tx.expected_pos[0] - pos[0];
            let dy = tx.expected_pos[1] - pos[1];
            let dz = (tx.expected_pos[2] - pos[2]).abs();
            let xy_dist = (dx * dx + dy * dy).sqrt();
            // Simulators (and DemoSim::build_rez) may offset Z by +scale.z * 0.5 on surface rez.
            xy_dist <= 0.5 && dz <= 2.5
        })?;
        let tx = self.pending_rez.remove(idx)?;
        let _ = object_uuid;
        Some((tx.name, tx.reply_tx))
    }
}

/// Pending viewport capture request awaiting the next GPU frame readback.
struct PendingCapture {
    include_ui: bool,
    dispatched_to_renderer: bool,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Pending LSL script source read request.
struct PendingScriptRead {
    local_id: u32,
    object_uuid: Uuid,
    item_id: Uuid,
    script_name: String,
    started_at: Instant,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Pending LSL script compile/save request.
struct PendingScriptCompile {
    local_id: u32,
    object_uuid: Uuid,
    item_id: Uuid,
    script_name: String,
    started_at: Instant,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Pending task inventory fetch for `lsl_inspect_object_scripts`.
struct PendingScriptInspect {
    local_id: u32,
    object_uuid: Uuid,
    started_at: Instant,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Pending asynchronous `RezScript` creation awaiting simulator task inventory update
/// before dispatching `NetCommand::SaveScript`.
struct PendingScriptCreation {
    handle: u64,
    local_id: u32,
    object_uuid: Uuid,
    known_script_ids: HashSet<Uuid>,
    script_name: String,
    source_code: String,
    mono: bool,
    running: bool,
    started_at: Instant,
    last_requested: Instant,
    reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// Main-thread state for the Model Context Protocol (MCP) bridge.
#[derive(Default)]
pub struct McpBridge {
    rx: Option<mpsc::Receiver<McpRequestEnvelope>>,
    server_handle: Option<McpServerHandle>,
    subscribed_sources: HashSet<String>,
    events_buffer: VecDeque<Value>,
    next_event_seq: u64,
    pending_captures: VecDeque<PendingCapture>,
    pub cof_queue: CofMutationQueue,
    pub build_tx_mgr: BuildTransactionManager,
    pending_script_inspects: VecDeque<PendingScriptInspect>,
    pending_script_creations: VecDeque<PendingScriptCreation>,
    pending_script_reads: HashMap<Uuid, PendingScriptRead>,
    pending_script_compiles: HashMap<Uuid, PendingScriptCompile>,
    lsd_stores: HashMap<Uuid, LinksetDataStore>,
    mcp_poses: HashMap<Uuid, Arc<crate::ik_edit::poser::SubjectPose>>,
    local_mesh_auto_reload: bool,
    pub sse_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub last_voice_speaking: HashMap<Uuid, bool>,
}

impl McpBridge {
    /// Applies any active MCP poser bone overrides on top of the scene's pose map.
    pub fn apply_poses(
        &self,
        poser_map: &mut HashMap<Uuid, Arc<crate::ik_edit::poser::SubjectPose>>,
    ) {
        for (subject_id, pose) in &self.mcp_poses {
            poser_map.insert(*subject_id, Arc::clone(pose));
        }
    }

    /// Pushes a viewer event into the bounded subscription ring buffer if its
    /// source category is currently subscribed.
    pub fn push_event(&mut self, source: &str, event_type: &str, payload: Value) {
        if !self.subscribed_sources.contains("*")
            && !self.subscribed_sources.contains("all")
            && !self.subscribed_sources.contains(source)
        {
            return;
        }
        self.next_event_seq += 1;
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let event_json = json!({
            "seq": self.next_event_seq,
            "timestamp_ms": ts_ms,
            "source": source,
            "type": event_type,
            "data": payload,
        });
        if let Some(sse_tx) = &self.sse_tx {
            let notification = json!({
                "jsonrpc": "2.0",
                "method": "notifications/message",
                "params": event_json.clone()
            });
            let _ = sse_tx.send(notification.to_string());
        }
        if self.events_buffer.len() >= MAX_EVENTS_BUFFER {
            self.events_buffer.pop_front();
        }
        self.events_buffer.push_back(event_json);
    }
}

impl App {
    /// Initializes the background MCP server if requested via `--mcp` or `AURORA_MCP`.
    pub(super) fn init_mcp_bridge(&mut self) {
        if self.mcp_bridge.server_handle.is_some() {
            return;
        }
        let cli = crate::cli::get();
        let env_enabled = std::env::var("AURORA_MCP")
            .ok()
            .is_some_and(|v| v != "0" && !v.eq_ignore_ascii_case("false"));
        if !cli.mcp && !env_enabled && cli.mcp_port.is_none() {
            return;
        }

        let (tx, rx) = create_bridge_channels();
        match spawn_server(self.net.runtime(), tx, cli.mcp_port) {
            Ok(handle) => {
                log::info!("MCP server listening on http://{}/mcp", handle.bind_addr);
                self.mcp_bridge.sse_tx = Some(handle.sse_tx.clone());
                self.mcp_bridge.rx = Some(rx);
                self.mcp_bridge.server_handle = Some(handle);
            }
            Err(err) => {
                log::error!("Failed to start MCP server: {err}");
            }
        }
    }

    /// Observes incoming `NetEvent`s for MCP script reads/compiles and event subscriptions.
    pub(super) fn mcp_observe_event(&mut self, ev: &NetEvent) {
        match ev {
            NetEvent::Chat(msg) => {
                self.mcp_bridge.push_event(
                    "chat",
                    "ChatFromSimulator",
                    json!({
                        "from_name": msg.from_name,
                        "source_id": msg.source_id.to_string(),
                        "owner_id": msg.owner_id.to_string(),
                        "chat_type": format!("{:?}", msg.chat_type),
                        "source_type": format!("{:?}", msg.source_type),
                        "message": msg.message,
                        "position": [msg.position.x, msg.position.y, msg.position.z],
                    }),
                );
            }
            NetEvent::InstantMessage(im) => {
                self.mcp_bridge.push_event(
                    "im",
                    "InstantMessage",
                    json!({
                        "from_agent_id": im.from_agent_id.to_string(),
                        "from_name": im.from_name,
                        "message": im.message,
                        "dialog": im.dialog,
                        "session_id": im.session_id.to_string(),
                    }),
                );
            }
            NetEvent::AgentMovementComplete { position, .. } => {
                self.mcp_bridge.push_event(
                    "teleport",
                    "AgentMovementComplete",
                    json!({
                        "position": [position.x, position.y, position.z],
                    }),
                );
            }
            NetEvent::TeleportFailed { reason } => {
                self.mcp_bridge.push_event(
                    "teleport",
                    "TeleportFailed",
                    json!({
                        "reason": reason,
                    }),
                );
            }
            NetEvent::ScriptLoaded {
                request,
                target,
                result,
            } => {
                let pending = self
                    .mcp_bridge
                    .pending_script_reads
                    .remove(request)
                    .or_else(|| self.mcp_bridge.pending_script_reads.remove(&target.item));
                if let Some(pending) = pending {
                    let resp = match result {
                        Ok(text) => Ok(ViewerResponse::Json(json!({
                            "object_local_id": pending.local_id,
                            "object_uuid": pending.object_uuid.to_string(),
                            "script_item_id": pending.item_id.to_string(),
                            "script_name": pending.script_name,
                            "source_code": text,
                        }))),
                        Err(err) => Err(McpError::Internal(format!(
                            "Failed to load script '{}': {err}",
                            pending.script_name
                        ))),
                    };
                    let _ = pending.reply_tx.send(resp);
                }
            }
            NetEvent::ScriptSaved {
                request,
                target,
                result,
            } => {
                let pending = self
                    .mcp_bridge
                    .pending_script_compiles
                    .remove(request)
                    .or_else(|| self.mcp_bridge.pending_script_compiles.remove(&target.item));
                if let Some(pending) = pending {
                    let resp = match result {
                        Ok(saved) => Ok(ViewerResponse::Json(json!({
                            "compiled": saved.compiled,
                            "object_local_id": pending.local_id,
                            "object_uuid": pending.object_uuid.to_string(),
                            "script_item_id": pending.item_id.to_string(),
                            "script_name": pending.script_name,
                            "asset_id": saved.asset.to_string(),
                            "errors": saved.errors,
                        }))),
                        Err(err) => Ok(ViewerResponse::Json(json!({
                            "compiled": false,
                            "object_local_id": pending.local_id,
                            "object_uuid": pending.object_uuid.to_string(),
                            "script_item_id": pending.item_id.to_string(),
                            "script_name": pending.script_name,
                            "errors": [err],
                        }))),
                    };
                    let _ = pending.reply_tx.send(resp);
                }
            }
            _ => {}
        }
    }

    /// Intercepts completed GPU frame captures requested by MCP before other consumers.
    pub(super) fn mcp_on_captured(&mut self, renderer: &mut Renderer) {
        if let Some(front) = self.mcp_bridge.pending_captures.front()
            && front.dispatched_to_renderer
            && renderer.captured.is_some()
        {
            if let Some((width, height, rgba_bytes)) = renderer.captured.take()
                && let Some(pending) = self.mcp_bridge.pending_captures.pop_front()
            {
                let frame = RawCapturedFrame {
                    width,
                    height,
                    buffer_type: CaptureBufferType::ColorRgb,
                    rgba_bytes,
                    near_clip: self.camera.near,
                    far_clip: self.settings.draw_distance,
                    camera_position: [
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z,
                    ],
                    camera_focus: [
                        self.camera.target.x,
                        self.camera.target.y,
                        self.camera.target.z,
                    ],
                    in_demo_mode: self.demo,
                };
                let _ = pending.reply_tx.send(Ok(ViewerResponse::FrameCaptured(frame)));
            }
        }
    }

    /// Drains queued MCP requests and progresses multi-frame state machines within `1.5ms`.
    pub(super) fn update_mcp_bridge(&mut self, mut renderer: Option<&mut Renderer>) {
        self.init_mcp_bridge();
        let start = Instant::now();

        self.progress_mcp_state_machines(renderer.as_deref_mut());

        while start.elapsed() < MCP_FRAME_BUDGET {
            let Some(rx) = self.mcp_bridge.rx.as_mut() else {
                break;
            };
            let Ok(envelope) = rx.try_recv() else {
                break;
            };
            self.handle_mcp_envelope(envelope, renderer.as_deref_mut());
        }

        if let Some(r) = renderer {
            self.progress_mcp_captures(r);
        }
    }

    fn progress_mcp_captures(&mut self, renderer: &mut Renderer) {
        if renderer.capture_request
            || renderer.capture_scene
            || renderer.captured.is_some()
        {
            return;
        }

        if let Some(front) = self.mcp_bridge.pending_captures.front_mut()
            && !front.dispatched_to_renderer
        {
            front.dispatched_to_renderer = true;
            if front.include_ui {
                renderer.capture_request = true;
            } else {
                renderer.capture_scene = true;
            }
        }
    }

    fn progress_mcp_state_machines(&mut self, _renderer: Option<&mut Renderer>) {
        let now = Instant::now();

        // 1. Progress pending PrimRezNew transactions
        if !self.mcp_bridge.build_tx_mgr.pending_rez.is_empty() {
            let candidates: Vec<(u32, Uuid, [f32; 3], crate::world::objects::ObjKey)> = self
                .world
                .objects
                .iter()
                .filter(|(_, o)| !o.is_avatar() && o.parent_id == 0)
                .map(|(_, o)| {
                    (
                        o.key.local_id,
                        o.full_id,
                        [o.position.x, o.position.y, o.position.z],
                        o.key,
                    )
                })
                .collect();

            for (local_id, object_uuid, pos, key) in candidates {
                if let Some((name, reply_tx)) = self
                    .mcp_bridge
                    .build_tx_mgr
                    .try_match_candidate(local_id, object_uuid, pos)
                {
                    let assigned_name = name
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("Object")
                        .to_string();
                    if assigned_name != "Object" {
                        self.send(NetCommand::Build(BuildCmd::SetName {
                            handle: key.region,
                            local_ids: vec![local_id],
                            name: assigned_name.clone(),
                        }));
                    }
                    let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                        "status": "rezzed",
                        "local_id": local_id,
                        "uuid": object_uuid.to_string(),
                        "name": assigned_name,
                        "position": pos,
                    }))));
                }
            }

            // Expire timed-out rez transactions (5s)
            while let Some(front) = self.mcp_bridge.build_tx_mgr.pending_rez.front() {
                if now.duration_since(front.started_at) > Duration::from_secs(5) {
                    if let Some(expired) = self.mcp_bridge.build_tx_mgr.pending_rez.pop_front() {
                        let _ = expired.reply_tx.send(Err(McpError::Timeout(5000)));
                    }
                } else {
                    break;
                }
            }
        }

        // 2. Progress pending task inventory script inspections
        let mut remaining_inspects = VecDeque::new();
        while let Some(inspect) = self.mcp_bridge.pending_script_inspects.pop_front() {
            let loaded = self
                .build
                .contents
                .objects
                .get(&inspect.object_uuid)
                .is_some_and(|inv| !inv.loading);
            if loaded || now.duration_since(inspect.started_at) > Duration::from_secs(3) {
                let resp = self.build_script_inspect_json(inspect.local_id, inspect.object_uuid);
                let _ = inspect.reply_tx.send(resp);
            } else {
                remaining_inspects.push_back(inspect);
            }
        }
        self.mcp_bridge.pending_script_inspects = remaining_inspects;

        // 2b. Progress pending async RezScript creations awaiting task inventory
        let mut remaining_creations = VecDeque::new();
        while let Some(mut creation) = self.mcp_bridge.pending_script_creations.pop_front() {
            let new_item_id = self
                .build
                .contents
                .objects
                .get(&creation.object_uuid)
                .filter(|inv| !inv.loading)
                .and_then(|inv| {
                    inv.items
                        .iter()
                        .rfind(|it| {
                            (it.asset_type == 10 || it.inv_type == 10)
                                && !creation.known_script_ids.contains(&it.item_id)
                        })
                        .map(|it| it.item_id)
                });
            if let Some(item_id) = new_item_id {
                let request = Uuid::new_v4();
                let target = ScriptTarget {
                    object: creation.object_uuid,
                    item: item_id,
                };
                self.mcp_bridge.pending_script_compiles.insert(
                    request,
                    PendingScriptCompile {
                        local_id: creation.local_id,
                        object_uuid: creation.object_uuid,
                        item_id,
                        script_name: creation.script_name,
                        started_at: now,
                        reply_tx: creation.reply_tx,
                    },
                );
                self.send(NetCommand::SaveScript {
                    request,
                    target,
                    handle: creation.handle,
                    options: ScriptOptions {
                        mono: creation.mono,
                        luau: aurora_net::scripts::Luau::Off,
                        running: creation.running,
                        experience: Uuid::nil(),
                    },
                    source: creation.source_code,
                });
            } else if now.duration_since(creation.started_at) > Duration::from_secs(12) {
                let _ = creation.reply_tx.send(Err(McpError::Timeout(12000)));
            } else {
                if now.duration_since(creation.last_requested) >= Duration::from_millis(500) {
                    creation.last_requested = now;
                    if let Some(idx) = self.world.objects.index_of_uuid(&creation.object_uuid) {
                        self.build.request_contents(&self.world, idx, true);
                        self.flush_build();
                    }
                }
                remaining_creations.push_back(creation);
            }
        }
        self.mcp_bridge.pending_script_creations = remaining_creations;

        // 3. Expire timed-out script reads / compiles
        self.mcp_bridge.pending_script_reads.retain(|_, pending| {
            now.duration_since(pending.started_at) <= Duration::from_secs(5)
        });
        self.mcp_bridge
            .pending_script_compiles
            .retain(|_, pending| {
                now.duration_since(pending.started_at) <= Duration::from_secs(5)
            });

        // 4. Progress serialized COF mutation queue
        let cof_ready = self.world.outfit.is_done()
            && self.appearance_ui.pending.is_none()
            && !self.world.rlva_detach_in_flight(now);

        if let Some(active) = &self.mcp_bridge.cof_queue.active {
            if cof_ready || now.duration_since(active.started_at) > Duration::from_secs(5) {
                let completed = self.mcp_bridge.cof_queue.active.take().unwrap();
                let worn = self.collect_worn_appearance();
                let _ = completed.reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "status": "settled",
                    "operation": format!("{:?}", completed.operation),
                    "message": self.appearance_ui.message,
                    "worn": worn,
                }))));
            }
        }

        if self.mcp_bridge.cof_queue.active.is_none()
            && cof_ready
            && let Some(next) = self.mcp_bridge.cof_queue.queue.pop_front()
        {
            match self.dispatch_cof_operation(&next.operation) {
                Ok(immediate_done) => {
                    if immediate_done || self.appearance_ui.pending.is_none() {
                        let worn = self.collect_worn_appearance();
                        let _ = next.reply_tx.send(Ok(ViewerResponse::Json(json!({
                            "status": "settled",
                            "message": self.appearance_ui.message,
                            "worn": worn,
                        }))));
                    } else {
                        self.mcp_bridge.cof_queue.active = Some(ActiveCofMutation {
                            operation: next.operation,
                            started_at: now,
                            reply_tx: next.reply_tx,
                        });
                    }
                }
                Err(err) => {
                    let _ = next.reply_tx.send(Err(err));
                }
            }
        }

        // 4. Progress voice speaker activity monitoring for realtime SSE
        if self.mcp_bridge.sse_tx.is_some() && self.voice.connected {
            let levels = self.voice.levels(self.world.agent_id);
            for (id, &(level, speaking)) in &levels {
                let was_speaking = self
                    .mcp_bridge
                    .last_voice_speaking
                    .get(id)
                    .copied()
                    .unwrap_or(false);
                if was_speaking != speaking {
                    self.mcp_bridge.last_voice_speaking.insert(*id, speaking);
                    self.mcp_bridge.push_event(
                        "voice",
                        "SpeakerChanged",
                        json!({
                            "avatar_uuid": id.to_string(),
                            "speaking": speaking,
                            "level": level,
                        }),
                    );
                }
            }
        }
    }

    fn handle_mcp_envelope(&mut self, envelope: McpRequestEnvelope, mut renderer: Option<&mut Renderer>) {
        let McpRequestEnvelope {
            lease_id: _,
            request,
            reply_tx,
        } = envelope;
        match request {
            ViewerRequest::ConnectionStatus | ViewerRequest::GetAgentStatus => {
                let region_name = if self.in_world() {
                    self.world.region_name()
                } else {
                    "Offline".to_string()
                };
                let region_info = self
                    .world
                    .main_region
                    .and_then(|h| self.world.regions.get(&h))
                    .and_then(|r| r.info.as_ref());
                let is_estate_manager = region_info
                    .is_some_and(|i| i.is_estate_manager || i.owner == self.world.agent_id)
                    || self.world.god_level > 0;
                let is_region_owner = region_info
                    .is_some_and(|i| i.owner == self.world.agent_id);
                let region_owner_id = region_info.map(|i| i.owner.to_string());
                let pos = self.world.agent.position;
                let rot = self.world.agent.body_rotation();
                let vel = self.world.agent.velocity;
                let (w, h) = renderer.as_deref().map_or((0, 0), |r| r.size());
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "connected": self.in_world(),
                    "headless": crate::cli::get().headless,
                    "offscreen_gpu": crate::cli::get().offscreen_gpu,
                    "screen": format!("{:?}", self.screen),
                    "login_state": {
                        "error": self.login_form.error.as_deref(),
                        "busy": self.login_form.busy,
                        "show_mfa": self.login_form.show_mfa,
                        "progress": self.login_progress.as_ref().map(|(msg, frac)| json!({ "stage": msg, "fraction": frac })),
                    },
                    "demo_mode": self.demo,
                    "agent_id": self.world.agent_id.to_string(),
                    "display_name": self.world.social.display_name_of(&self.world.agent_id),
                    "region_name": region_name,
                    "is_estate_manager": is_estate_manager,
                    "is_region_owner": is_region_owner,
                    "region_owner_id": region_owner_id,
                    "can_manage_estate": is_estate_manager,
                    "parcel_name": self.world.parcel_name,
                    "position": [pos.x, pos.y, pos.z],
                    "rotation_quat": [rot.x, rot.y, rot.z, rot.w],
                    "velocity": [vel.x, vel.y, vel.z],
                    "flying": self.world.agent.flying,
                    "sitting": self.world.agent.is_sitting(),
                    "ground_sit": self.world.agent.ground_sit,
                    "health": self.world.health,
                    "balance": self.world.balance,
                    "fps": self.perf.fps,
                    "frame_ms": self.perf.frame_ms,
                    "viewport": [w, h],
                    "draw_distance": self.settings.draw_distance,
                    "rlva_enabled": self.settings.rlva_enabled,
                }))));
            }
            ViewerRequest::CapabilitiesRefresh => {
                let region_name = self.in_world().then(|| self.world.region_name());
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "refreshed": true,
                    "connected": self.in_world(),
                    "region": region_name,
                }))));
            }
            ViewerRequest::ViewerCall {
                api,
                operation,
                arguments,
            } => {
                let resp = self.handle_legacy_viewer_call(&api, &operation, &arguments);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EventsSubscribe { source } => {
                self.mcp_bridge.subscribed_sources.insert(source);
                let active: Vec<String> = self
                    .mcp_bridge
                    .subscribed_sources
                    .iter()
                    .cloned()
                    .collect();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "subscribed": active,
                }))));
            }
            ViewerRequest::EventsUnsubscribe { source } => {
                if source == "*" || source.eq_ignore_ascii_case("all") {
                    self.mcp_bridge.subscribed_sources.clear();
                } else {
                    self.mcp_bridge.subscribed_sources.remove(&source);
                }
                let active: Vec<String> = self
                    .mcp_bridge
                    .subscribed_sources
                    .iter()
                    .cloned()
                    .collect();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "subscribed": active,
                }))));
            }
            ViewerRequest::EventsRead { after } => {
                let events: Vec<Value> = self
                    .mcp_bridge
                    .events_buffer
                    .iter()
                    .filter(|ev| ev["seq"].as_u64().unwrap_or(0) > after)
                    .cloned()
                    .collect();
                let last_seq = self.mcp_bridge.next_event_seq;
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "count": events.len(),
                    "last_seq": last_seq,
                    "events": events,
                }))));
            }
            ViewerRequest::RadarScanAvatars {
                max_radius_m,
                limit,
            } => {
                let me = self.world.agent.position;
                let mut rows: Vec<(f32, Value)> = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| o.is_avatar() && o.parent_id == 0)
                    .filter_map(|(_, o)| {
                        let dist = o.position.distance(me);
                        if dist > max_radius_m {
                            return None;
                        }
                        let anims: Vec<String> = self
                            .world
                            .animations
                            .get(&o.full_id)
                            .map(|list| list.iter().map(|a| a.id.to_string()).collect())
                            .unwrap_or_default();
                        let name = self.world.social.display_name_of(&o.full_id);
                        Some((
                            dist,
                            json!({
                                "uuid": o.full_id.to_string(),
                                "local_id": o.key.local_id,
                                "name": name,
                                "position": [o.position.x, o.position.y, o.position.z],
                                "distance_m": (dist * 100.0).round() / 100.0,
                                "sitting_on": o.parent_id,
                                "animations": anims,
                                "is_self": o.full_id == self.world.agent_id,
                            }),
                        ))
                    })
                    .collect();
                rows.sort_by(|a, b| a.0.total_cmp(&b.0));
                let avatars: Vec<Value> = rows.into_iter().take(limit).map(|(_, v)| v).collect();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "count": avatars.len(),
                    "avatars": avatars,
                }))));
            }
            ViewerRequest::RadarScanObjects {
                max_radius_m,
                root_only,
                name_filter,
                limit,
            } => {
                let me = self.world.agent.position;
                let needle = name_filter
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let mut rows: Vec<(f32, Value)> = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| !o.is_avatar())
                    .filter(|(_, o)| !root_only || o.parent_id == 0)
                    .filter_map(|(idx, o)| {
                        let world_pos = crate::build::replace::region_transform(&self.world, idx)
                            .map(|(p, _)| p)
                            .unwrap_or(o.position);
                        let dist = world_pos.distance(me);
                        if dist > max_radius_m {
                            return None;
                        }
                        let props = self.build.props.get(&o.full_id);
                        let obj_name = props
                            .map(|p| p.name.clone())
                            .filter(|s| !s.is_empty())
                            .or_else(|| {
                                let t = o.text.lines().next().unwrap_or("").trim();
                                (!t.is_empty()).then(|| t.to_owned())
                            })
                            .unwrap_or_else(|| format!("Prim {}", o.key.local_id));
                        if !needle.is_empty() && !obj_name.to_ascii_lowercase().contains(&needle) {
                            return None;
                        }
                        let desc = props
                            .map(|p| p.description.clone())
                            .unwrap_or_default();
                        let owner_id = if !o.owner_id.is_nil() {
                            o.owner_id
                        } else {
                            props.map(|p| p.owner_id).unwrap_or(Uuid::nil())
                        };
                        Some((
                            dist,
                            json!({
                                "local_id": o.key.local_id,
                                "uuid": o.full_id.to_string(),
                                "parent_id": o.parent_id,
                                "name": obj_name,
                                "description": desc,
                                "position": [world_pos.x, world_pos.y, world_pos.z],
                                "local_position": [o.position.x, o.position.y, o.position.z],
                                "rotation_quat": [o.rotation.x, o.rotation.y, o.rotation.z, o.rotation.w],
                                "scale": [o.scale.x, o.scale.y, o.scale.z],
                                "distance_m": (dist * 100.0).round() / 100.0,
                                "attachment_point": o.attachment_point(),
                                "is_mesh": o.volume.is_mesh(),
                                "owner_id": owner_id.to_string(),
                            }),
                        ))
                    })
                    .collect();
                rows.sort_by(|a, b| a.0.total_cmp(&b.0));
                let objects: Vec<Value> = rows.into_iter().take(limit).map(|(_, v)| v).collect();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "count": objects.len(),
                    "objects": objects,
                }))));
            }
            ViewerRequest::CommsReadHistory {
                channel_filter,
                limit,
            } => {
                let filter = channel_filter
                    .unwrap_or_else(|| "all".to_string())
                    .to_ascii_lowercase();
                let mut entries = Vec::new();
                if filter == "all" || filter == "local" || filter == "script" || filter == "nearby" {
                    for line in self.world.chat.iter().rev() {
                        let kind_str = match line.kind {
                            crate::world::ChatKind::Local(_) | crate::world::ChatKind::Own(_) => "local",
                            crate::world::ChatKind::Object(_) | crate::world::ChatKind::ObjectIm => "script",
                            crate::world::ChatKind::Im => "im",
                            crate::world::ChatKind::System => "system",
                            _ => "other",
                        };
                        if filter != "all"
                            && !(filter == "nearby" && kind_str == "local")
                            && filter != kind_str
                        {
                            continue;
                        }
                        entries.push(json!({
                            "kind": kind_str,
                            "from": line.from,
                            "source_id": line.source.to_string(),
                            "text": line.text,
                        }));
                        if entries.len() >= limit {
                            break;
                        }
                    }
                }
                if (filter == "all" || filter == "script") && entries.len() < limit {
                    for line in self.world.script_debug.lines.iter().rev() {
                        entries.push(json!({
                            "kind": "script_debug",
                            "from": line.from,
                            "source_id": line.source.to_string(),
                            "text": line.text,
                        }));
                        if entries.len() >= limit {
                            break;
                        }
                    }
                }
                if (filter == "all" || filter == "im") && entries.len() < limit {
                    for session in &self.world.social.ims {
                        for line in session.lines.iter().rev() {
                            entries.push(json!({
                                "kind": "im",
                                "session_with": session.other.to_string(),
                                "from": line.from,
                                "text": line.text,
                            }));
                            if entries.len() >= limit {
                                break;
                            }
                        }
                        if entries.len() >= limit {
                            break;
                        }
                    }
                }
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "count": entries.len(),
                    "messages": entries,
                }))));
            }
            ViewerRequest::CommsSend {
                message,
                channel,
                chat_type,
                target_uuid,
            } => {
                if let Some(target) = target_uuid {
                    self.world.own_im(target, &message);
                    self.send(NetCommand::SendIm {
                        to: target,
                        message: message.clone(),
                    });
                    let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                        "sent": true,
                        "mode": "im",
                        "target_uuid": target.to_string(),
                        "message": message,
                    }))));
                } else {
                    let ct = match chat_type {
                        ChatKind::Whisper => aurora_net::ChatType::Whisper,
                        ChatKind::Normal => aurora_net::ChatType::Normal,
                        ChatKind::Shout => aurora_net::ChatType::Shout,
                    };
                    self.send(NetCommand::Chat {
                        message: message.clone(),
                        channel,
                        chat_type: ct,
                    });
                    let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                        "sent": true,
                        "mode": "chat",
                        "channel": channel,
                        "chat_type": format!("{ct:?}"),
                        "message": message,
                    }))));
                }
            }
            ViewerRequest::NavWalkTo {
                position,
                allow_flying,
                stop_distance: _,
                timeout_ms: _,
            } => {
                if allow_flying {
                    self.world.agent.flying = true;
                }
                let target = Vec3::from_array(position);
                self.autopilot = Some(crate::autopilot::AutoPilot::start(
                    self.world.agent.position,
                    target,
                    false,
                    Instant::now(),
                ));
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "status": "moving",
                    "target": position,
                    "flying": self.world.agent.flying,
                }))));
            }
            ViewerRequest::NavMovementStatus => {
                let pos = self.world.agent.position;
                let (moving, target, dist) = match &self.autopilot {
                    Some(ap) => {
                        let d = Vec3::new(ap.target.x - pos.x, ap.target.y - pos.y, 0.0).length();
                        (
                            !ap.reached(pos),
                            Some([ap.target.x, ap.target.y, ap.target.z]),
                            Some(d),
                        )
                    }
                    None => (false, None, None),
                };
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "moving": moving,
                    "target": target,
                    "current_position": [pos.x, pos.y, pos.z],
                    "distance_remaining": dist,
                    "flying": self.world.agent.flying,
                    "sitting": self.world.agent.is_sitting(),
                    "ground_sit": self.world.agent.ground_sit,
                }))));
            }
            ViewerRequest::NavStop => {
                self.autopilot = None;
                let pos = self.world.agent.position;
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "status": "stopped",
                    "position": [pos.x, pos.y, pos.z],
                }))));
            }
            ViewerRequest::NavSit { target } => {
                if let Some(target_str) = target.filter(|s| !s.trim().is_empty()) {
                    let Some(obj_idx) = self.resolve_prim_index(&target_str) else {
                        let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                            "Sit target object '{target_str}' not found in region"
                        ))));
                        return;
                    };
                    let obj = self.world.objects.get(obj_idx).unwrap();
                    let target_uuid = obj.full_id;
                    let handle = obj.key.region;
                    self.send(NetCommand::RequestSit {
                        handle,
                        target: target_uuid,
                        offset: Vec3::ZERO,
                    });
                    let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                        "status": "sit_requested",
                        "target_uuid": target_uuid.to_string(),
                    }))));
                } else {
                    self.world.agent.ground_sit = true;
                    self.send(NetCommand::OneShotControl(
                        aurora_net::control::SIT_ON_GROUND,
                    ));
                    let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                        "status": "ground_sitting",
                    }))));
                }
            }
            ViewerRequest::NavStand => {
                self.world.agent.ground_sit = false;
                self.send(NetCommand::OneShotControl(aurora_net::control::STAND_UP));
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "status": "standing",
                }))));
            }
            ViewerRequest::NavTeleport {
                region_name,
                position,
            } => {
                let dest = Vec3::from_array(position);
                let current_name = self.world.region_name();
                let target_region = region_name.unwrap_or_else(|| current_name.clone());
                if (target_region.is_empty() || target_region.eq_ignore_ascii_case(&current_name))
                    && let Some(handle) = self.world.main_region
                {
                    self.send(NetCommand::TeleportTo {
                        handle,
                        position: dest,
                        look_at: self.world.agent.forward(),
                    });
                } else {
                    self.send(NetCommand::TeleportToRegion {
                        request: Uuid::new_v4(),
                        name: target_region.clone(),
                        position: dest,
                        look_at: self.world.agent.forward(),
                    });
                }
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "status": "teleport_initiated",
                    "region_name": target_region,
                    "position": position,
                }))));
            }
            ViewerRequest::RlvaExecute { commands } => {
                let resp = self.handle_mcp_rlva_execute(commands);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::SettingsQuery {
                search,
                modified_only,
                limit,
            } => {
                let defaults = crate::settings::registry::defaults();
                let entries = crate::settings::registry::entries(&self.settings, &defaults);
                let needle = search.to_ascii_lowercase();
                let matched: Vec<Value> = entries
                    .into_iter()
                    .filter(|e| !modified_only || e.changed())
                    .filter(|e| needle.is_empty() || e.key.to_ascii_lowercase().contains(&needle))
                    .take(limit)
                    .map(|e| {
                        let modified = e.changed();
                        let group = e.key.split('.').next().unwrap_or("general").to_string();
                        json!({
                            "key": e.key,
                            "group": group,
                            "kind": format!("{:?}", e.kind),
                            "value": e.value,
                            "default": e.default,
                            "modified": modified,
                            "locked": false,
                        })
                    })
                    .collect();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "count": matched.len(),
                    "settings": matched,
                }))));
            }
            ViewerRequest::SettingGet { group: _, key } => {
                let defaults = crate::settings::registry::defaults();
                let entries = crate::settings::registry::entries(&self.settings, &defaults);
                let found = entries.into_iter().find(|e| {
                    e.key.eq_ignore_ascii_case(&key)
                        || e.key
                            .rsplit('.')
                            .next()
                            .is_some_and(|leaf| leaf.eq_ignore_ascii_case(&key))
                });
                match found {
                    Some(e) => {
                        let modified = e.changed();
                        let group = e.key.split('.').next().unwrap_or("general").to_string();
                        let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                            "key": e.key,
                            "group": group,
                            "kind": format!("{:?}", e.kind),
                            "value": e.value,
                            "default": e.default,
                            "modified": modified,
                            "locked": false,
                        }))));
                    }
                    None => {
                        let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                            "Unknown viewer setting '{key}'"
                        ))));
                    }
                }
            }
            ViewerRequest::SettingSet {
                group: _,
                key,
                value,
            } => {
                let defaults = crate::settings::registry::defaults();
                let previous = crate::settings::registry::entries(&self.settings, &defaults)
                    .into_iter()
                    .find(|e| {
                        e.key.eq_ignore_ascii_case(&key)
                            || e.key
                                .rsplit('.')
                                .next()
                                .is_some_and(|leaf| leaf.eq_ignore_ascii_case(&key))
                    })
                    .map(|e| (e.key, e.value));
                let Some((canonical_key, prev_val)) = previous else {
                    let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                        "Unknown viewer setting '{key}'"
                    ))));
                    return;
                };
                match crate::settings::registry::set(
                    &mut self.settings,
                    &canonical_key,
                    value.clone(),
                ) {
                    Ok(()) => {
                        let _ = reply_tx.send(Ok(ViewerResponse::SettingMutated {
                            key: canonical_key,
                            previous_value: prev_val,
                            current_value: value,
                        }));
                    }
                    Err(err) => {
                        let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                            "Failed to set '{canonical_key}': {err}"
                        ))));
                    }
                }
            }
            ViewerRequest::CameraControl { command } => {
                let prev_action = RollbackAction::RestoreCamera {
                    mode: format!("{:?}", self.camera.mode),
                    position: [
                        self.camera.position.x,
                        self.camera.position.y,
                        self.camera.position.z,
                    ],
                    focus: [
                        self.camera.target.x,
                        self.camera.target.y,
                        self.camera.target.z,
                    ],
                    fov: self.settings.camera.fov,
                };
                match command {
                    CameraCommand::FollowAvatar | CameraCommand::Release => {
                        self.camera
                            .handle_reset_view(&mut self.world.agent, &self.settings.camera);
                    }
                    CameraCommand::FocusObject {
                        object_id,
                        distance,
                        pitch_deg,
                        yaw_deg,
                    } => {
                        let focus = self
                            .resolve_prim_index(&object_id)
                            .and_then(|idx| {
                                crate::build::replace::region_transform(&self.world, idx)
                                    .map(|(p, _)| p)
                            })
                            .unwrap_or(self.world.agent.position);
                        let pitch = pitch_deg.to_radians();
                        let yaw = yaw_deg.to_radians();
                        let dist = distance.max(0.5);
                        let horiz = dist * pitch.cos();
                        let dz = dist * pitch.sin();
                        let eye = focus + Vec3::new(horiz * yaw.cos(), horiz * yaw.sin(), dz);
                        self.camera.set_view(
                            eye,
                            focus,
                            &mut self.world.agent,
                            &self.settings.camera,
                        );
                    }
                    CameraCommand::ManualTransform {
                        position,
                        focus,
                        fov_deg,
                    } => {
                        self.camera.set_view(
                            Vec3::from_array(position),
                            Vec3::from_array(focus),
                            &mut self.world.agent,
                            &self.settings.camera,
                        );
                        if let Some(fov) = fov_deg {
                            let rad = fov.clamp(10.0, 140.0).to_radians();
                            self.settings.camera.fov = rad;
                            self.camera.fov_y = rad;
                        }
                    }
                    CameraCommand::Orbit {
                        delta_yaw_deg,
                        delta_pitch_deg,
                        zoom_delta,
                    } => {
                        let current_dist =
                            (self.camera.position - self.camera.target).length().max(0.5);
                        let new_dist = (current_dist + zoom_delta).max(0.5);
                        self.camera.demo_orbit(
                            delta_yaw_deg.to_radians(),
                            delta_pitch_deg.to_radians(),
                            Some(new_dist),
                            &self.settings.camera,
                        );
                    }
                    CameraCommand::MeshPreview {
                        yaw_deg,
                        pitch_deg,
                        distance,
                    } => {
                        let focus = self.world.agent.position + Vec3::new(0.0, 0.0, 0.5);
                        let yaw = yaw_deg.to_radians();
                        let pitch = pitch_deg.to_radians();
                        let dist = distance.max(0.5);
                        let horiz = dist * pitch.cos();
                        let dz = dist * pitch.sin();
                        let eye = focus + Vec3::new(horiz * yaw.cos(), horiz * yaw.sin(), dz);
                        self.camera.set_view(
                            eye,
                            focus,
                            &mut self.world.agent,
                            &self.settings.camera,
                        );
                    }
                }
                let state = json!({
                    "mode": format!("{:?}", self.camera.mode),
                    "position": [self.camera.position.x, self.camera.position.y, self.camera.position.z],
                    "focus": [self.camera.target.x, self.camera.target.y, self.camera.target.z],
                    "fov_deg": self.settings.camera.fov.to_degrees(),
                });
                let _ = reply_tx.send(Ok(ViewerResponse::CameraMutated {
                    previous_camera: Some(prev_action),
                    state,
                }));
            }
            ViewerRequest::PoserInspect { subject } => {
                let subject_id = subject.unwrap_or(self.world.agent_id);
                let resp = self.handle_mcp_poser_inspect(subject_id);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PoserSetBones {
                subject,
                transforms,
            } => {
                let subject_id = subject.unwrap_or(self.world.agent_id);
                let resp = self.handle_mcp_poser_set_bones(subject_id, transforms);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PoserReset { subject } => {
                if let Some(subject_id) = subject {
                    self.mcp_bridge.mcp_poses.remove(&subject_id);
                } else {
                    self.mcp_bridge.mcp_poses.clear();
                }
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "reset": true,
                    "subject": subject.map(|id| id.to_string()),
                }))));
            }
            ViewerRequest::FloaterList => {
                let floaters = self.collect_floater_list();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "floaters": floaters,
                }))));
            }
            ViewerRequest::FloaterOpen { name, open } => {
                let resp = self.set_floater_open(&name, open);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::ViewportCapture {
                buffer_type,
                include_ui,
                include_hud: _,
                max_dimensions,
            } => match buffer_type {
                CaptureBufferType::DepthNormalized => {
                    let frame = self.compute_normalized_depth_frame(
                        max_dimensions.0,
                        max_dimensions.1,
                        renderer.as_deref(),
                    );
                    let _ = reply_tx.send(Ok(ViewerResponse::FrameCaptured(frame)));
                }
                CaptureBufferType::ColorRgb => {
                    if renderer.is_none() {
                        let _ = reply_tx.send(Err(McpError::Internal(
                            "viewport_capture ColorRgb is unavailable in CPU --headless mode; restart with --offscreen-gpu".into(),
                        )));
                    } else {
                        self.mcp_bridge.pending_captures.push_back(PendingCapture {
                            include_ui,
                            dispatched_to_renderer: false,
                            reply_tx,
                        });
                    }
                }
            },
            ViewerRequest::InventoryBrowse {
                folder_id_or_path,
                depth: _,
                limit,
            } => {
                let folder_uuid = Uuid::parse_str(folder_id_or_path.trim()).ok();
                let resp = self.handle_mcp_inventory_browse(folder_uuid, None, None, false, limit);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::InventorySearch {
                folder_id,
                query,
                asset_type,
                worn_only,
                limit,
            } => {
                let folder_uuid = folder_id.as_deref().and_then(|s| Uuid::parse_str(s.trim()).ok());
                let resp = self.handle_mcp_inventory_browse(
                    folder_uuid,
                    Some(query),
                    asset_type,
                    worn_only,
                    limit,
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::AppearanceGetWorn => {
                let worn = self.collect_worn_appearance();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "cof_ready": self.world.outfit.is_done() && self.appearance_ui.pending.is_none(),
                    "cof_busy": self.mcp_bridge.cof_queue.is_busy(),
                    "cof_pending": self.mcp_bridge.cof_queue.pending_len(),
                    "worn": worn,
                }))));
            }
            ViewerRequest::AppearanceMutate { operation } => {
                self.mcp_bridge.cof_queue.enqueue(operation, reply_tx);
            }
            ViewerRequest::PrimRezNew {
                position,
                scale,
                rotation_quat,
                pcode,
                name,
            } => {
                let Some(handle) = self.world.main_region else {
                    let _ = reply_tx.send(Err(McpError::NotInWorld));
                    return;
                };
                let known_local_ids: HashSet<u32> = self
                    .world
                    .objects
                    .iter()
                    .map(|(_, o)| o.key.local_id)
                    .collect();
                let pos_vec = Vec3::from_array(position);
                let rot_quat = Quat::from_array(rotation_quat);
                let scale_vec = Vec3::from_array(scale);
                let shape = crate::build::shapes::shapes()[0].raw();

                self.mcp_bridge
                    .build_tx_mgr
                    .register_rez(position, name, known_local_ids, reply_tx);

                let prim = NewPrim {
                    handle,
                    group_id: self
                        .world
                        .group_for_rez(self.settings.build.rez_under_land_group),
                    pcode: if pcode == 0 {
                        aurora_prim::params::LL_PCODE_VOLUME
                    } else {
                        pcode
                    },
                    material: 3, // Wood default
                    add_flags: 0x02,
                    shape,
                    ray_start: pos_vec,
                    ray_end: pos_vec,
                    ray_target: Uuid::nil(),
                    ray_end_is_intersection: false,
                    bypass_raycast: true,
                    scale: scale_vec,
                    rotation: rot_quat,
                    state: 0,
                };
                self.send(NetCommand::Build(BuildCmd::Add(Box::new(prim))));
                // Immediately check if synchronous DemoSim already added the prim
                self.progress_mcp_state_machines(renderer.as_deref_mut());
            }
            ViewerRequest::PrimTransform { transforms } => {
                let resp = self.handle_mcp_prim_transform(transforms);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PrimSetShape {
                local_id,
                path_cut,
                profile_cut,
                hollow,
                twist,
                color,
                texture_id,
            } => {
                let resp = self.handle_mcp_prim_set_shape(
                    local_id,
                    path_cut,
                    profile_cut,
                    hollow,
                    twist,
                    color,
                    texture_id,
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PrimLink { local_ids, delink } => {
                let resp = self.handle_mcp_prim_link(local_ids, delink);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PrimDelete { local_ids } => {
                let resp = self.handle_mcp_prim_delete(local_ids);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::LslInspectObjectScripts { object_local_id } => {
                self.handle_mcp_lsl_inspect(object_local_id, reply_tx);
            }
            ViewerRequest::LslReadScript {
                object_local_id,
                script_item_id,
            } => {
                self.handle_mcp_lsl_read(object_local_id, script_item_id, reply_tx);
            }
            ViewerRequest::LslWriteAndCompile {
                object_local_id,
                script_item_id,
                script_name,
                source_code,
                mono,
                running,
            } => {
                self.handle_mcp_lsl_write(
                    object_local_id,
                    script_item_id,
                    script_name,
                    source_code,
                    mono,
                    running,
                    reply_tx,
                );
            }
            ViewerRequest::LslListenScriptOutput {
                object_id,
                local_id,
                duration_ms: _,
            } => {
                let resp = self.handle_mcp_lsl_listen_output(object_id, local_id);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::LsdInspect {
                object_local_id,
                filter_regex,
                include_protected,
                passphrase,
                limit,
            } => {
                let resp = self.handle_mcp_lsd_inspect(
                    object_local_id,
                    filter_regex,
                    include_protected,
                    passphrase,
                    limit,
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::LsdMutate {
                object_local_id,
                operations,
            } => {
                let resp = self.handle_mcp_lsd_mutate(object_local_id, operations);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::LocalMeshOpen { open } => {
                self.imports.open = open;
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "open": self.imports.open,
                    "entries_count": self.imports.entries.len(),
                }))));
            }
            ViewerRequest::LocalMeshStatus => {
                let entries: Vec<Value> = self
                    .imports
                    .entries
                    .iter()
                    .map(|e| {
                        json!({
                            "id": e.id.to_string(),
                            "name": e.name,
                            "local": e.local,
                            "local_published": e.local_published,
                        })
                    })
                    .collect();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "open": self.imports.open,
                    "busy": self.imports.busy(),
                    "auto_reload": self.mcp_bridge.local_mesh_auto_reload,
                    "entries_count": entries.len(),
                    "entries": entries,
                }))));
            }
            ViewerRequest::LocalMeshAutoReload { enabled } => {
                self.mcp_bridge.local_mesh_auto_reload = enabled;
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "auto_reload": self.mcp_bridge.local_mesh_auto_reload,
                }))));
            }
            ViewerRequest::ApplyRollbacks { actions } => {
                let mut restored = 0usize;
                for action in actions {
                    match action {
                        RollbackAction::RestoreSetting { key, value } => {
                            if crate::settings::registry::set(
                                &mut self.settings,
                                &key,
                                value,
                            )
                            .is_ok()
                            {
                                restored += 1;
                            }
                        }
                        RollbackAction::RestoreCamera {
                            mode: _,
                            position,
                            focus,
                            fov,
                        } => {
                            self.camera.set_view(
                                Vec3::from_array(position),
                                Vec3::from_array(focus),
                                &mut self.world.agent,
                                &self.settings.camera,
                            );
                            self.settings.camera.fov = fov;
                            self.camera.fov_y = fov;
                            restored += 1;
                        }
                        RollbackAction::ResetPoser => {
                            self.mcp_bridge.mcp_poses.clear();
                            restored += 1;
                        }
                        RollbackAction::RestoreEnvironment { time_of_day } => {
                            if time_of_day == 0 {
                                self.world.eep.clear_local();
                            } else {
                                self.world.eep.set_time_of_day(time_of_day);
                            }
                            self.panels.time_of_day = time_of_day;
                            self.settings.time_of_day = time_of_day;
                            restored += 1;
                        }
                    }
                }
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "restored_count": restored,
                }))));
            }
            // ---- Phase 6: Suite 1 — Direct Asset Upload, LiveMesh, Inventory Rez & Materials ----
            ViewerRequest::LiveMeshControl { action, source_id } => {
                let resp = self.handle_mcp_livemesh_control(&action, source_id);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::AssetUpload {
                action,
                paths,
                choice,
                include_textures,
            } => {
                let resp =
                    self.handle_mcp_asset_upload(&action, paths, choice.as_deref(), include_textures);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::InventoryRezObject {
                item_id,
                position,
                select,
            } => {
                self.handle_mcp_inventory_rez_object(item_id, position, select, reply_tx, renderer);
            }
            ViewerRequest::ObjectTakeOrDerez {
                local_id,
                action,
                folder_id,
            } => {
                let resp = self.handle_mcp_object_take_or_derez(local_id, &action, folder_id);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PrimSetMaterial {
                local_id,
                face,
                texture_id,
                material_id,
                color,
                repeats,
                offsets,
                rotation_rad,
                glow,
                fullbright,
                alpha_mode,
                bump_shiny,
            } => {
                let resp = self.handle_mcp_prim_set_material(
                    local_id,
                    face,
                    texture_id,
                    material_id,
                    color,
                    repeats,
                    offsets,
                    rotation_rad,
                    glow,
                    fullbright,
                    alpha_mode,
                    bump_shiny,
                );
                let _ = reply_tx.send(resp);
            }
            // ---- Phase 6: Suite 2 — Remaining 13 Legacy Viewer MCP Tools ----
            ViewerRequest::MeshUploadOpen => {
                self.imports.open(crate::imports::Choice::Mesh);
                self.imports.open = true;
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "ok": true,
                    "open": true,
                    "choice": "mesh",
                }))));
            }
            ViewerRequest::MeshUploadStatus => {
                let resp = self.handle_mcp_mesh_upload_status();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::NativeFileDialogs => {
                let dialogs = self.handle_mcp_native_file_dialogs();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(dialogs)));
            }
            ViewerRequest::NativeFileChoose {
                dialog_id,
                filename,
            } => {
                let resp = self.handle_mcp_native_file_choose(dialog_id, &filename);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiInspect { path } => {
                let resp = self.handle_mcp_ui_inspect(&path);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiFind {
                query,
                under,
                search_in,
                match_mode,
                max_depth,
                include_info,
                offset,
                limit,
            } => {
                let resp = self.handle_mcp_ui_find(
                    &query,
                    &under,
                    &search_in,
                    &match_mode,
                    max_depth,
                    include_info,
                    offset,
                    limit,
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiClick {
                path,
                button,
                floater,
                observe_path,
            } => {
                let resp = self.handle_mcp_ui_click(
                    &path,
                    &button,
                    floater.as_deref(),
                    observe_path.as_deref(),
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiGetValue { path } => {
                let resp = self.handle_mcp_ui_get_value(&path);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiSetText { path, text } => {
                let resp = self.handle_mcp_ui_set_text(&path, &text);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiSelect { path, value } => {
                let resp = self.handle_mcp_ui_select(&path, value);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiPressKey {
                path,
                keysym,
                modifiers,
                observe_path,
            } => {
                let resp = self.handle_mcp_ui_press_key(
                    &path,
                    &keysym,
                    &modifiers,
                    observe_path.as_deref(),
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiListMenus { query } => {
                let resp = self.handle_mcp_ui_list_menus(&query);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::UiInvokeMenu { name } => {
                let resp = self.handle_mcp_ui_invoke_menu(&name);
                let _ = reply_tx.send(resp);
            }
            // ---- Phase 6: Suite 3 — In-World Object Interaction, Notecards & Particles ----
            ViewerRequest::ObjectTouch {
                local_id,
                face,
                uv,
                st,
                hold_ms,
            } => {
                let resp = self.handle_mcp_object_touch(local_id, face, uv, st, hold_ms);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::AvatarSit {
                action,
                target_local_id,
                target_uuid,
                offset,
            } => {
                let resp =
                    self.handle_mcp_avatar_sit(&action, target_local_id, target_uuid, offset);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::NotecardRead {
                item_id,
                object_local_id,
            } => {
                let resp = self.handle_mcp_notecard_read(item_id, object_local_id);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::NotecardWrite {
                item_id,
                object_local_id,
                name,
                body,
            } => {
                let resp = self.handle_mcp_notecard_write(
                    item_id,
                    object_local_id,
                    name.as_deref(),
                    &body,
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PrimSetParticles {
                local_id,
                clear,
                pattern,
                texture_id,
                start_color,
                end_color,
                start_scale,
                end_scale,
                particle_max_age,
                burst_rate,
                burst_part_count,
                burst_radius,
                burst_speed_min,
                burst_speed_max,
                accel,
                omega,
                follow_src,
                follow_velocity,
                interp_color,
                interp_scale,
                emissive,
                ribbon,
            } => {
                self.handle_mcp_prim_set_particles(
                    local_id,
                    clear,
                    pattern.as_deref(),
                    texture_id,
                    start_color,
                    end_color,
                    start_scale,
                    end_scale,
                    particle_max_age,
                    burst_rate,
                    burst_part_count,
                    burst_radius,
                    burst_speed_min,
                    burst_speed_max,
                    accel,
                    omega,
                    follow_src,
                    follow_velocity,
                    interp_color,
                    interp_scale,
                    emissive,
                    ribbon,
                    reply_tx,
                );
            }
            ViewerRequest::ViewerLogin {
                username,
                password,
                location,
                mfa_token,
            } => {
                let resp = self.handle_mcp_viewer_login(
                    username.as_deref(),
                    password.as_deref(),
                    location.as_deref(),
                    mfa_token.as_deref(),
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::ViewerShutdown => {
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "status": "shutting_down",
                    "message": "Viewer clean shutdown initiated."
                }))));
                if crate::cli::get().headless {
                    self.shutdown_headless();
                } else {
                    self.exit_prompt.request();
                    self.confirm_exit(crate::ui::exit::Action::Quit);
                }
            }
            // ---- Phase 7: Physics Flags, Metadata, Animations & Economy ----
            ViewerRequest::PrimSetPhysicsProperties {
                local_id,
                physics,
                temporary,
                phantom,
                shape_type,
                density,
                friction,
                restitution,
                gravity_multiplier,
            } => {
                let resp = self.handle_mcp_prim_set_physics_properties(
                    local_id,
                    physics,
                    temporary,
                    phantom,
                    shape_type.as_deref(),
                    density,
                    friction,
                    restitution,
                    gravity_multiplier,
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::PrimSetMetadata {
                local_id,
                name,
                description,
            } => {
                let resp = self.handle_mcp_prim_set_metadata(local_id, name.as_deref(), description.as_deref());
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::AvatarAnimationPlay {
                animation,
                loop_anim,
            } => {
                let resp = self.handle_mcp_avatar_animation_play(&animation, loop_anim.unwrap_or(true));
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::AvatarAnimationStop { animation } => {
                let resp = self.handle_mcp_avatar_animation_stop(&animation);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::AvatarAnimationsList => {
                let resp = self.handle_mcp_avatar_animations_list();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EconomyGetBalance => {
                let resp = self.handle_mcp_economy_get_balance();
                let _ = reply_tx.send(resp);
            }
            // ---- Phase 8: Land, Parcel & Estate Suite ----
            ViewerRequest::ParcelInfoQuery => {
                let resp = self.handle_mcp_parcel_info();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::ParcelSetMusic { music_url } => {
                let resp = self.handle_mcp_parcel_set_music(&music_url);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::ParcelSetProperties {
                name,
                description,
                flags,
            } => {
                let resp = self.handle_mcp_parcel_set_properties(name.as_deref(), description.as_deref(), flags);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::ParcelAccessListModify {
                action,
                list,
                avatar_uuid,
            } => {
                let resp = self.handle_mcp_parcel_access_list_modify(&action, &list, &avatar_uuid);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::ParcelEject { target_uuid, ban } => {
                let resp = self.handle_mcp_parcel_eject(&target_uuid, ban);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EconomyPayAvatar {
                target_uuid,
                amount,
                message,
            } => {
                let resp = self.handle_mcp_economy_pay_avatar(&target_uuid, amount, message);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EconomyPayObject {
                target_local_id,
                amount,
                description,
            } => {
                let resp = self.handle_mcp_economy_pay_object(target_local_id, amount, description);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::GroupsList => {
                let resp = self.handle_mcp_groups_list();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::GroupsActivate { group_uuid } => {
                let resp = self.handle_mcp_groups_activate(&group_uuid);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::FriendsList { online_only } => {
                let resp = self.handle_mcp_friends_list(online_only);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EnvironmentGet => {
                let resp = self.handle_mcp_environment_get();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EnvironmentSetTime { preset, time_of_day } => {
                let resp = self.handle_mcp_environment_set_time(preset.as_deref(), time_of_day);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::EnvironmentApplyPreset { preset, sky_uuid, water_uuid } => {
                let resp = self.handle_mcp_environment_apply_preset(
                    preset.as_deref(),
                    sky_uuid.as_deref(),
                    water_uuid.as_deref(),
                );
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::VoiceStatus => {
                let resp = self.handle_mcp_voice_status();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::VoiceParticipantsList => {
                let resp = self.handle_mcp_voice_participants_list();
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::VoiceSetUserVolume { avatar_uuid, volume } => {
                let resp = self.handle_mcp_voice_set_user_volume(&avatar_uuid, volume);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::VoiceSetUserMute { avatar_uuid, mute } => {
                let resp = self.handle_mcp_voice_set_user_mute(&avatar_uuid, mute);
                let _ = reply_tx.send(resp);
            }
            ViewerRequest::VoiceTransmit { transmitting } => {
                let resp = self.handle_mcp_voice_transmit(transmitting);
                let _ = reply_tx.send(resp);
            }
        }
    }

    fn handle_legacy_viewer_call(
        &mut self,
        api: &str,
        operation: &str,
        arguments: &Value,
    ) -> Result<ViewerResponse, McpError> {
        let op = operation.to_ascii_lowercase();
        match (api, op.as_str()) {
            (_, "get_agent_status" | "connection_status" | "getstatus" | "getposition") => {
                let pos = self.world.agent.position;
                Ok(ViewerResponse::Json(json!({
                    "connected": self.in_world(),
                    "agent_id": self.world.agent_id.to_string(),
                    "position": [pos.x, pos.y, pos.z],
                })))
            }
            (_, "floater_list" | "list") => Ok(ViewerResponse::Json(json!({
                "floaters": self.collect_floater_list(),
            }))),
            (_, "floater_open" | "open") => {
                let name = arguments
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("inventory");
                let open = arguments
                    .get("open")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                self.set_floater_open(name, open)
            }
            _ => Err(McpError::InvalidParams(format!(
                "Unknown viewer_call '{api}.{operation}'. Call viewer_api_inspect to list supported operations."
            ))),
        }
    }

    fn handle_mcp_rlva_execute(
        &mut self,
        commands: Vec<String>,
    ) -> Result<ViewerResponse, McpError> {
        if !self.settings.rlva_enabled {
            self.settings.rlva_enabled = true;
            self.world.rlva_configure(true);
        }
        let source_id = Uuid::from_u128(0x726c7661);
        let mut executed = Vec::new();
        for cmd_str in commands {
            let normalized = if cmd_str.starts_with('@') {
                cmd_str
            } else {
                format!("@{cmd_str}")
            };
            self.world.apply(NetEvent::Chat(aurora_net::ChatMessage {
                from_name: "MCP RLVa Controller".to_owned(),
                source_id,
                owner_id: self.world.agent_id,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: self.world.agent.position,
                message: normalized.clone(),
            }));
            executed.push(normalized);
        }
        self.rlva_camera_commands();

        let mut replies = Vec::new();
        for cmd in &self.world.rlva.out {
            match cmd {
                NetCommand::ChatReply { channel, message } => {
                    replies.push(json!({
                        "channel": channel,
                        "reply": message,
                    }));
                }
                NetCommand::ImReply { to, message, .. } => {
                    replies.push(json!({
                        "to": to.to_string(),
                        "reply": message,
                    }));
                }
                _ => {}
            }
        }

        let active_restrictions: Vec<Value> = self
            .world
            .rlva
            .restrictions()
            .into_iter()
            .map(|(obj, rules)| {
                json!({
                    "source_id": obj.to_string(),
                    "rules": rules,
                })
            })
            .collect();

        Ok(ViewerResponse::Json(json!({
            "executed": executed,
            "source_id": source_id.to_string(),
            "replies": replies,
            "active_restrictions": active_restrictions,
        })))
    }

    fn handle_mcp_poser_inspect(&self, subject_id: Uuid) -> Result<ViewerResponse, McpError> {
        let rig = &self.world.avatar_lib.rig;
        let pose = if let Some(override_pose) = self.mcp_bridge.mcp_poses.get(&subject_id) {
            (**override_pose).clone()
        } else if let Some(scene_pose) = self.scene.poser.get(&subject_id) {
            (**scene_pose).clone()
        } else {
            crate::ik_edit::Frame::subject(&self.world, subject_id)
                .and_then(|frame| crate::ik_edit::poser::SubjectPose::capture(rig, &frame))
                .ok_or_else(|| {
                    McpError::InvalidParams("Subject skeleton not available".into())
                })?
        };

        let mut pairs: Vec<(&String, &usize)> = rig.names.iter().collect();
        pairs.sort_by_key(|(_, idx)| **idx);
        let mut joints = Vec::new();
        for (name, &idx) in pairs {
            let rot = pose.rotations.get(idx).copied().unwrap_or(Quat::IDENTITY);
            let (rx, ry, rz) = rot.to_euler(EulerRot::XYZ);
            let pos = pose.positions.get(idx).copied().unwrap_or(Vec3::ZERO);
            let scale = pose.scales.get(idx).copied().unwrap_or(Vec3::ONE);
            joints.push(json!({
                "index": idx,
                "name": name,
                "rotation_euler_deg": [rx.to_degrees(), ry.to_degrees(), rz.to_degrees()],
                "rotation_quat": [rot.x, rot.y, rot.z, rot.w],
                "position_offset": [pos.x, pos.y, pos.z],
                "scale": [scale.x, scale.y, scale.z],
            }));
        }

        Ok(ViewerResponse::Json(json!({
            "subject_id": subject_id.to_string(),
            "joint_count": joints.len(),
            "joints": joints,
        })))
    }

    fn handle_mcp_poser_set_bones(
        &mut self,
        subject_id: Uuid,
        transforms: Vec<BoneTransform>,
    ) -> Result<ViewerResponse, McpError> {
        let rig = &self.world.avatar_lib.rig;
        let mut pose = if let Some(existing) = self.mcp_bridge.mcp_poses.get(&subject_id) {
            (**existing).clone()
        } else if let Some(scene_pose) = self.scene.poser.get(&subject_id) {
            (**scene_pose).clone()
        } else {
            crate::ik_edit::Frame::subject(&self.world, subject_id)
                .and_then(|frame| crate::ik_edit::poser::SubjectPose::capture(rig, &frame))
                .ok_or_else(|| {
                    McpError::InvalidParams("Subject skeleton not available".into())
                })?
        };

        let mut modified_bones = Vec::new();

        for transform in transforms {
            let Some(&joint_idx) = rig.names.get(&transform.bone_name) else {
                return Err(McpError::InvalidParams(format!(
                    "Unknown avatar skeleton joint '{}'",
                    transform.bone_name
                )));
            };
            if let Some(quat) = transform.rotation_quat
                && let Some(slot) = pose.rotations.get_mut(joint_idx)
            {
                *slot = Quat::from_array(quat).normalize();
            } else if let Some(euler_deg) = transform.rotation_euler_deg
                && let Some(slot) = pose.rotations.get_mut(joint_idx)
            {
                *slot = Quat::from_euler(
                    EulerRot::XYZ,
                    euler_deg[0].to_radians(),
                    euler_deg[1].to_radians(),
                    euler_deg[2].to_radians(),
                );
            }
            if let Some(pos) = transform.position_offset
                && let Some(slot) = pose.positions.get_mut(joint_idx)
            {
                *slot = Vec3::from_array(pos);
            }
            if let Some(scale) = transform.scale
                && let Some(slot) = pose.scales.get_mut(joint_idx)
            {
                *slot = Vec3::from_array(scale);
            }
            pose.locks.remove(&joint_idx);
            modified_bones.push(transform.bone_name);
        }

        self.mcp_bridge
            .mcp_poses
            .insert(subject_id, Arc::new(pose));

        Ok(ViewerResponse::PoserMutated {
            state: json!({
                "subject_id": subject_id.to_string(),
                "modified_bones": modified_bones,
            }),
        })
    }

    fn compute_normalized_depth_frame(
        &self,
        width: u32,
        height: u32,
        renderer: Option<&Renderer>,
    ) -> RawCapturedFrame {
        let w = width.clamp(32, 1024);
        let h = height.clamp(32, 1024);
        let (win_w, win_h) = renderer.map_or((1280, 720), |r| r.size());
        let aspect = win_w.max(1) as f32 / win_h.max(1) as f32;
        let view_proj = self.camera.proj(aspect) * self.camera.view();
        let inv_vp = view_proj.inverse();
        let cam_pos = self.camera.position;
        let near = self.camera.near.max(0.1);
        let far = self.settings.draw_distance.max(near + 1.0);

        let spheres: Vec<(Vec3, f32)> = self
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.attachment_point() == 0)
            .filter_map(|(idx, o)| {
                let pos = crate::build::replace::region_transform(&self.world, idx)
                    .map(|(p, _)| p)
                    .unwrap_or(o.position);
                let radius = (o.scale.length() * 0.5).max(0.25);
                if pos.distance(cam_pos) <= far + radius {
                    Some((pos, radius))
                } else {
                    None
                }
            })
            .collect();

        let mut rgba = vec![255u8; (w * h * 4) as usize];
        let ground_z = self.world.ground_height(cam_pos).unwrap_or(20.0);
        for py in 0..h {
            let ndc_y = 1.0 - 2.0 * ((py as f32 + 0.5) / h as f32);
            for px in 0..w {
                let ndc_x = 2.0 * ((px as f32 + 0.5) / w as f32) - 1.0;
                let far_clip = inv_vp * glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
                let far_world = far_clip.truncate() / far_clip.w.max(1e-6);
                let ray_dir = (far_world - cam_pos).normalize_or_zero();

                let mut best_t = far;
                for &(center, radius) in &spheres {
                    let oc = cam_pos - center;
                    let b = oc.dot(ray_dir);
                    let c = oc.dot(oc) - radius * radius;
                    let disc = b * b - c;
                    if disc >= 0.0 {
                        let t = -b - disc.sqrt();
                        if t >= near && t < best_t {
                            best_t = t;
                        }
                    }
                }

                if ray_dir.z.abs() > 1e-4 {
                    let t_ground = (ground_z - cam_pos.z) / ray_dir.z;
                    if t_ground >= near && t_ground < best_t {
                        best_t = t_ground;
                    }
                }

                let norm = ((best_t - near) / (far - near)).clamp(0.0, 1.0);
                let val = ((1.0 - norm) * 255.0).round() as u8;
                let idx = ((py * w + px) * 4) as usize;
                rgba[idx] = val;
                rgba[idx + 1] = val;
                rgba[idx + 2] = val;
                rgba[idx + 3] = 255;
            }
        }

        RawCapturedFrame {
            width: w,
            height: h,
            buffer_type: CaptureBufferType::DepthNormalized,
            rgba_bytes: rgba,
            near_clip: near,
            far_clip: far,
            camera_position: [cam_pos.x, cam_pos.y, cam_pos.z],
            camera_focus: [
                self.camera.target.x,
                self.camera.target.y,
                self.camera.target.z,
            ],
            in_demo_mode: self.demo,
        }
    }

fn inventory_item_type_str(asset_type: i32, inv_type: i32) -> &'static str {
    match inv_type {
        10 => "script",
        6 => "object",
        7 => "notecard",
        0 => "texture",
        1 => "sound",
        2 => "calling_card",
        3 => "landmark",
        18 => "wearable",
        19 => "animation",
        20 => "gesture",
        22 => "mesh",
        25 => "settings",
        26 => "material",
        _ => match asset_type {
            0 | 12 => "texture",
            1 | 17 => "sound",
            2 => "calling_card",
            3 => "landmark",
            4 | 10 | 11 => "script",
            5 => "clothing",
            6 => "object",
            7 => "notecard",
            13 => "bodypart",
            20 => "animation",
            21 => "gesture",
            24 => "link",
            25 => "link_folder",
            49 => "mesh",
            56 => "settings",
            57 => "material",
            _ => "unknown",
        },
    }
}

    fn handle_mcp_inventory_browse(
        &self,
        folder_uuid: Option<Uuid>,
        search_query: Option<String>,
        asset_type_filter: Option<String>,
        worn_only: bool,
        limit: usize,
    ) -> Result<ViewerResponse, McpError> {
        let inv = &self.world.inventory;
        let query = search_query.unwrap_or_default().to_ascii_lowercase();
        let type_filter = asset_type_filter.unwrap_or_default().to_ascii_lowercase();
        let worn_set = self.world.outfit_attachment_items();

        let matches_type = |asset_type: i32| -> bool {
            if type_filter.is_empty() || type_filter == "all" {
                return true;
            }
            match type_filter.as_str() {
                "texture" => asset_type == 0,
                "sound" => asset_type == 1,
                "landmark" => asset_type == 3,
                "clothing" => asset_type == 5,
                "object" => asset_type == 6,
                "notecard" => asset_type == 7,
                "lsl" | "script" => asset_type == 10,
                "bodypart" => asset_type == 13,
                "animation" => asset_type == 20,
                "gesture" => asset_type == 21,
                "mesh" => asset_type == 49,
                "settings" => asset_type == 56,
                "material" => asset_type == 57,
                _ => true,
            }
        };

        let mut folders_out = Vec::new();
        let mut items_out = Vec::new();

        if !query.is_empty() || worn_only {
            for folder in inv.folders.values() {
                if !query.is_empty() && folder.info.name.to_ascii_lowercase().contains(&query) {
                    folders_out.push(json!({
                        "uuid": folder.info.id.to_string(),
                        "parent_id": folder.info.parent.to_string(),
                        "name": folder.info.name,
                        "type_default": folder.info.type_default,
                        "library": folder.library,
                    }));
                    if folders_out.len() >= limit {
                        break;
                    }
                }
            }
            for item in inv.items.values() {
                if worn_only && !worn_set.contains(&item.id) {
                    continue;
                }
                if !matches_type(item.asset_type) {
                    continue;
                }
                if query.is_empty()
                    || item.name.to_ascii_lowercase().contains(&query)
                    || item.desc.to_ascii_lowercase().contains(&query)
                {
                    items_out.push(self.serialize_inventory_item(item));
                    if items_out.len() >= limit {
                        break;
                    }
                }
            }
        } else {
            let root_id = folder_uuid.unwrap_or(inv.root);
            if let Some(folder) = inv.folders.get(&root_id) {
                for child_id in &folder.children {
                    if let Some(child) = inv.folders.get(child_id) {
                        folders_out.push(json!({
                            "uuid": child.info.id.to_string(),
                            "parent_id": child.info.parent.to_string(),
                            "name": child.info.name,
                            "type_default": child.info.type_default,
                            "library": child.library,
                        }));
                        if folders_out.len() >= limit {
                            break;
                        }
                    }
                }
                for item_id in &folder.items {
                    if let Some(item) = inv.items.get(item_id)
                        && matches_type(item.asset_type)
                    {
                        items_out.push(self.serialize_inventory_item(item));
                        if items_out.len() >= limit {
                            break;
                        }
                    }
                }
            }
        }

        Ok(ViewerResponse::Json(json!({
            "folder_count": folders_out.len(),
            "item_count": items_out.len(),
            "folders": folders_out,
            "items": items_out,
        })))
    }

    fn serialize_inventory_item(&self, item: &aurora_net::inventory::InvItem) -> Value {
        // Enforce AGENTS.md §2 Creator Permissions before exposing asset_id
        let show_asset_uuid =
            crate::world::inventory::actions::asset_uuid_shown(&self.world.inventory, item.asset_id);
        let mask = crate::world::inventory::actions::agent_mask(item, self.world.agent_id);
        let can_copy = (mask & perm::COPY) != 0;
        let can_modify = (mask & perm::MODIFY) != 0;
        let can_transfer = (mask & perm::TRANSFER) != 0;
        let type_name = Self::inventory_item_type_str(item.asset_type, item.inv_type);
        let is_object = item.asset_type == 6 || item.inv_type == 6;
        let is_script = item.asset_type == 10 || item.asset_type == 4 || item.inv_type == 10;
        let is_notecard = item.asset_type == 7 || item.inv_type == 7;

        json!({
            "item_id": item.id.to_string(),
            "parent_id": item.parent.to_string(),
            "name": item.name,
            "description": item.desc,
            "type": type_name,
            "asset_type": item.asset_type,
            "inv_type": item.inv_type,
            "is_object": is_object,
            "is_script": is_script,
            "is_notecard": is_notecard,
            "creator_id": item.creator.to_string(),
            "owner_id": item.owner.to_string(),
            "is_link": item.asset_type == 24 || item.asset_type == 25,
            "asset_id": if show_asset_uuid { Some(item.asset_id.to_string()) } else { None },
            "is_no_copy": !can_copy,
            "is_no_modify": !can_modify,
            "is_no_transfer": !can_transfer,
            "permissions": {
                "copy": can_copy,
                "modify": can_modify,
                "transfer": can_transfer,
            }
        })
    }

    fn collect_worn_appearance(&self) -> Value {
        let inv = &self.world.inventory;
        let cof_id = crate::world::inventory::actions::system(
            inv,
            crate::world::outfit::FT_CURRENT_OUTFIT,
        );
        let mut cof_links = Vec::new();
        if let Some(cof) = cof_id.and_then(|id| inv.folders.get(&id)) {
            for item_id in &cof.items {
                if let Some(link) = inv.items.get(item_id) {
                    let target = inv.items.get(&link.asset_id).unwrap_or(link);
                    cof_links.push(json!({
                        "link_id": link.id.to_string(),
                        "target_item_id": target.id.to_string(),
                        "name": target.name,
                        "asset_type": target.asset_type,
                    }));
                }
            }
        }

        let mut attachments = Vec::new();
        for (_, obj) in self.world.objects.iter() {
            let pt = obj.attachment_point();
            if pt != 0 && obj.parent_id != 0 {
                let point_name = self
                    .world
                    .avatar_lib
                    .attach_names
                    .get(&pt)
                    .cloned()
                    .unwrap_or_else(|| format!("Point {pt}"));
                let obj_name = self
                    .build
                    .props
                    .get(&obj.full_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                attachments.push(json!({
                    "object_uuid": obj.full_id.to_string(),
                    "local_id": obj.key.local_id,
                    "item_id": obj.attachment_item_id().map(|id| id.to_string()),
                    "point_id": pt,
                    "point_name": point_name,
                    "name": obj_name,
                }));
            }
        }

        json!({
            "cof_folder_id": cof_id.map(|id| id.to_string()),
            "cof_items": cof_links,
            "attachments": attachments,
        })
    }

    fn dispatch_cof_operation(&mut self, operation: &AppearanceOperation) -> Result<bool, McpError> {
        match operation {
            AppearanceOperation::Attach {
                item_id,
                attach_point,
                replace_existing,
            } => {
                if !self.world.inventory.items.contains_key(item_id) {
                    return Err(McpError::InvalidParams(format!(
                        "Inventory item '{item_id}' not found"
                    )));
                }
                let action = crate::world::appearance::Action::WearItem {
                    item: *item_id,
                    replace: *replace_existing,
                    point: attach_point.unwrap_or(0),
                };
                self.apply_appearance_action(action);
                Ok(false)
            }
            AppearanceOperation::Detach { item_id } => {
                self.apply_appearance_action(crate::world::appearance::Action::Remove(*item_id));
                Ok(false)
            }
            AppearanceOperation::WearFolder { folder_id, append } => {
                if !self.world.inventory.folders.contains_key(folder_id) {
                    return Err(McpError::InvalidParams(format!(
                        "Outfit folder '{folder_id}' not found"
                    )));
                }
                let action = crate::world::appearance::Action::WearOutfits {
                    folders: vec![*folder_id],
                    append: *append,
                };
                self.apply_appearance_action(action);
                Ok(false)
            }
            AppearanceOperation::RemoveFolder { folder_id } => {
                let Some(folder) = self.world.inventory.folders.get(folder_id) else {
                    return Err(McpError::InvalidParams(format!(
                        "Folder '{folder_id}' not found"
                    )));
                };
                let items: Vec<Uuid> = folder.items.clone();
                self.apply_appearance_action(crate::world::appearance::Action::RemoveItems(items));
                Ok(false)
            }
        }
    }

    fn resolve_prim_index(&self, id_str: &str) -> Option<usize> {
        if let Ok(local_id) = id_str.trim().parse::<u32>()
            && let Some(handle) = self.world.main_region
        {
            return self
                .world
                .objects
                .index_of(&crate::world::objects::ObjKey {
                    region: handle,
                    local_id,
                });
        }
        if let Ok(uuid) = Uuid::parse_str(id_str.trim()) {
            return self.world.objects.index_of_uuid(&uuid);
        }
        None
    }

    fn resolve_prim_by_local_id(&self, local_id: u32) -> Result<(usize, Uuid), McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let Some(idx) = self
            .world
            .objects
            .index_of(&crate::world::objects::ObjKey {
                region: handle,
                local_id,
            })
        else {
            return Err(McpError::InvalidParams(format!(
                "Object with local_id {local_id} not found in region"
            )));
        };
        let obj = self.world.objects.get(idx).unwrap();
        let root_idx = if obj.parent_id == 0 {
            idx
        } else {
            self.world
                .objects
                .index_of(&crate::world::objects::ObjKey {
                    region: handle,
                    local_id: obj.parent_id,
                })
                .unwrap_or(idx)
        };
        let root_uuid = self
            .world
            .objects
            .get(root_idx)
            .map(|r| r.full_id)
            .unwrap_or(obj.full_id);
        Ok((idx, root_uuid))
    }

    fn check_object_modify_permission(&self, obj_idx: usize) -> Result<(), McpError> {
        let Some(obj) = self.world.objects.get(obj_idx) else {
            return Err(McpError::InvalidParams("Object not found".into()));
        };
        if !obj.owner_id.is_nil() && obj.owner_id != self.world.agent_id && self.world.god_level == 0
        {
            return Err(McpError::PermissionDenied(
                "you do not have Modify rights on this object".into(),
            ));
        }
        if let Some(props) = self.build.props.get(&obj.full_id)
            && (props.owner_mask & perm::MODIFY) == 0
            && !props.owner_id.is_nil()
            && self.world.god_level == 0
        {
            return Err(McpError::PermissionDenied(
                "object lacks Modify permission".into(),
            ));
        }
        Ok(())
    }

    fn handle_mcp_prim_transform(
        &mut self,
        transforms: Vec<PrimTransformSpec>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let mut updates = Vec::new();
        for spec in &transforms {
            let (idx, _) = self.resolve_prim_by_local_id(spec.local_id)?;
            self.check_object_modify_permission(idx)?;
            let new_pos = spec.position.map(Vec3::from_array);
            let new_rot = if let Some(q) = spec.rotation_quat {
                Some(Quat::from_array(q).normalize())
            } else {
                spec.rotation_euler_deg.map(|r| {
                    Quat::from_euler(
                        EulerRot::XYZ,
                        r[0].to_radians(),
                        r[1].to_radians(),
                        r[2].to_radians(),
                    )
                })
            };
            let new_scale = spec.scale.map(Vec3::from_array);
            if new_pos.is_some() || new_rot.is_some() || new_scale.is_some() {
                updates.push(TransformUpdate {
                    local_id: spec.local_id,
                    position: new_pos,
                    rotation: new_rot,
                    scale: new_scale,
                    linked: false,
                    uniform: false,
                });
            }
            if let Some(name) = &spec.name {
                self.send(NetCommand::Build(BuildCmd::SetName {
                    handle,
                    local_ids: vec![spec.local_id],
                    name: name.clone(),
                }));
            }
            if let Some(desc) = &spec.description {
                self.send(NetCommand::Build(BuildCmd::SetDescription {
                    handle,
                    local_ids: vec![spec.local_id],
                    description: desc.clone(),
                }));
            }
        }
        if !updates.is_empty() {
            self.send(NetCommand::Build(BuildCmd::Transform { handle, updates }));
        }
        Ok(ViewerResponse::Json(json!({
            "transformed_count": transforms.len(),
        })))
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mcp_prim_set_shape(
        &mut self,
        local_id: u32,
        path_cut: Option<[f32; 2]>,
        profile_cut: Option<[f32; 2]>,
        hollow: Option<f32>,
        twist: Option<[f32; 2]>,
        color: Option<[f32; 4]>,
        texture_id: Option<Uuid>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
        self.check_object_modify_permission(idx)?;

        if path_cut.is_some() || profile_cut.is_some() || hollow.is_some() || twist.is_some() {
            let mut vol = self.world.objects.get(idx).unwrap().volume.clone();
            if let Some([begin, end]) = path_cut {
                vol.path.begin = begin.clamp(0.0, 0.95);
                vol.path.end = end.clamp(vol.path.begin + 0.05, 1.0);
            }
            if let Some([begin, end]) = profile_cut {
                vol.profile.begin = begin.clamp(0.0, 0.95);
                vol.profile.end = end.clamp(vol.profile.begin + 0.05, 1.0);
            }
            if let Some(h) = hollow {
                vol.profile.hollow = h.clamp(0.0, 0.95);
            }
            if let Some([tb, te]) = twist {
                vol.path.twist_begin = tb.clamp(-1.0, 1.0);
                vol.path.twist_end = te.clamp(-1.0, 1.0);
            }
            self.build.set_shape(&mut self.world, idx, vol);
            self.flush_build();
        }

        if color.is_some() || texture_id.is_some() {
            let num_faces = self.build.num_faces(&self.world, idx);
            let obj = self.world.objects.get_mut(idx).unwrap();
            let mut te = obj.te.as_deref().cloned().unwrap_or_default();
            if let Some(rgba) = color {
                for face in &mut te.faces {
                    face.color = rgba;
                }
            }
            if let Some(tex) = texture_id {
                for face in &mut te.faces {
                    face.texture = tex;
                }
            }
            let packed = aurora_prim::te::pack_texture_entry(&te, num_faces);
            let media_url = obj.media_url.clone();
            obj.te = Some(Arc::new(te));
            obj.material_dirty = true;
            self.send(NetCommand::Build(BuildCmd::SetTextures {
                handle,
                local_id,
                media_url,
                texture_entry: packed,
            }));
        }

        Ok(ViewerResponse::Json(json!({
            "updated": true,
            "local_id": local_id,
        })))
    }

    fn handle_mcp_prim_link(
        &mut self,
        local_ids: Vec<u32>,
        delink: bool,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        for &lid in &local_ids {
            let (idx, _) = self.resolve_prim_by_local_id(lid)?;
            self.check_object_modify_permission(idx)?;
        }
        if delink {
            self.send(NetCommand::Build(BuildCmd::Delink {
                handle,
                local_ids: local_ids.clone(),
            }));
        } else {
            self.send(NetCommand::Build(BuildCmd::Link {
                handle,
                local_ids: local_ids.clone(),
            }));
        }
        Ok(ViewerResponse::Json(json!({
            "action": if delink { "delinked" } else { "linked" },
            "local_ids": local_ids,
        })))
    }

    fn handle_mcp_prim_delete(&mut self, local_ids: Vec<u32>) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        for &lid in &local_ids {
            let (idx, _) = self.resolve_prim_by_local_id(lid)?;
            self.check_object_modify_permission(idx)?;
        }
        let trash_id = crate::world::inventory::actions::system(
            &self.world.inventory,
            14, // FT_TRASH
        )
        .unwrap_or_default();
        self.send(NetCommand::Build(BuildCmd::DeRez {
            handle,
            local_ids: local_ids.clone(),
            destination: aurora_net::build::derez::TRASH,
            destination_id: trash_id,
            group_id: self.world.groups.active,
        }));
        Ok(ViewerResponse::Json(json!({
            "deleted_local_ids": local_ids,
        })))
    }

    fn handle_mcp_lsl_inspect(
        &mut self,
        object_local_id: u32,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let (idx, _) = match self.resolve_prim_by_local_id(object_local_id) {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_tx.send(Err(e));
                return;
            }
        };
        let object_uuid = self.world.objects.get(idx).unwrap().full_id;
        self.build.request_contents(&self.world, idx, true);
        self.flush_build();

        let loaded = self
            .build
            .contents
            .objects
            .get(&object_uuid)
            .is_some_and(|inv| !inv.loading);
        if loaded {
            let _ = reply_tx.send(self.build_script_inspect_json(object_local_id, object_uuid));
        } else {
            self.mcp_bridge
                .pending_script_inspects
                .push_back(PendingScriptInspect {
                    local_id: object_local_id,
                    object_uuid,
                    started_at: Instant::now(),
                    reply_tx,
                });
        }
    }

    fn build_script_inspect_json(
        &self,
        local_id: u32,
        object_uuid: Uuid,
    ) -> Result<ViewerResponse, McpError> {
        let scripts: Vec<Value> = self
            .build
            .contents
            .objects
            .get(&object_uuid)
            .map(|inv| {
                inv.items
                    .iter()
                    .filter(|it| it.asset_type == 10 || it.inv_type == 10)
                    .map(|it| {
                        let mask = crate::world::inventory::actions::task_agent_mask(
                            it,
                            self.world.agent_id,
                        );
                        let (running, mono) = self
                            .build
                            .contents
                            .running
                            .get(&(object_uuid, it.item_id))
                            .copied()
                            .unwrap_or((true, Some(true)));
                        json!({
                            "item_id": it.item_id.to_string(),
                            "name": it.name,
                            "description": it.description,
                            "running": running,
                            "mono": mono,
                            "permissions": {
                                "copy": (mask & perm::COPY) != 0,
                                "modify": (mask & perm::MODIFY) != 0,
                                "transfer": (mask & perm::TRANSFER) != 0,
                            }
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(ViewerResponse::Json(json!({
            "local_id": local_id,
            "object_uuid": object_uuid.to_string(),
            "script_count": scripts.len(),
            "scripts": scripts,
        })))
    }

    fn handle_mcp_lsl_read(
        &mut self,
        object_local_id: u32,
        script_item_id: Uuid,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        if self.world.rlva.has("viewscript") {
            let _ = reply_tx.send(Err(McpError::PermissionDenied(
                "RLVa @viewscript restriction forbids reading LSL scripts".into(),
            )));
            return;
        }
        let Some(handle) = self.world.main_region else {
            let _ = reply_tx.send(Err(McpError::NotInWorld));
            return;
        };
        let (idx, _) = match self.resolve_prim_by_local_id(object_local_id) {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_tx.send(Err(e));
                return;
            }
        };
        let object_uuid = self.world.objects.get(idx).unwrap().full_id;
        self.build.request_contents(&self.world, idx, false);
        self.flush_build();

        let item = self
            .build
            .contents
            .objects
            .get(&object_uuid)
            .and_then(|inv| inv.items.iter().find(|it| it.item_id == script_item_id))
            .cloned();
        let Some(item) = item else {
            let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                "Script item '{script_item_id}' not found inside object {object_local_id}"
            ))));
            return;
        };

        // Enforce AGENTS.md §2 Creator Permissions (requires both MODIFY and COPY to read source)
        let mask = crate::world::inventory::actions::task_agent_mask(&item, self.world.agent_id);
        if (mask & (perm::MODIFY | perm::COPY)) != (perm::MODIFY | perm::COPY)
            && self.world.god_level == 0
        {
            let _ = reply_tx.send(Err(McpError::PermissionDenied(format!(
                "reading LSL script '{}' requires both Copy and Modify permissions",
                item.name
            ))));
            return;
        }

        let request = Uuid::new_v4();
        let target = ScriptTarget {
            object: object_uuid,
            item: item.item_id,
        };
        self.mcp_bridge.pending_script_reads.insert(
            request,
            PendingScriptRead {
                local_id: object_local_id,
                object_uuid,
                item_id: item.item_id,
                script_name: item.name,
                started_at: Instant::now(),
                reply_tx,
            },
        );
        self.send(NetCommand::LoadScript {
            request,
            target,
            handle,
            owner: item.owner_id,
            asset: item.asset_id,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mcp_lsl_write(
        &mut self,
        object_local_id: u32,
        script_item_id: Option<Uuid>,
        script_name: String,
        source_code: String,
        mono: bool,
        running: bool,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        if self.world.rlva.has("viewscript") {
            let _ = reply_tx.send(Err(McpError::PermissionDenied(
                "RLVa @viewscript restriction forbids editing LSL scripts".into(),
            )));
            return;
        }
        let Some(handle) = self.world.main_region else {
            let _ = reply_tx.send(Err(McpError::NotInWorld));
            return;
        };
        let (idx, _) = match self.resolve_prim_by_local_id(object_local_id) {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_tx.send(Err(e));
                return;
            }
        };
        if let Err(e) = self.check_object_modify_permission(idx) {
            let _ = reply_tx.send(Err(e));
            return;
        }
        let object_uuid = self.world.objects.get(idx).unwrap().full_id;
        self.build.request_contents(&self.world, idx, false);
        self.flush_build();

        let existing = self
            .build
            .contents
            .objects
            .get(&object_uuid)
            .and_then(|inv| {
                inv.items
                    .iter()
                    .find(|it| {
                        script_item_id.is_some_and(|id| it.item_id == id)
                            || it.name.eq_ignore_ascii_case(&script_name)
                    })
                    .cloned()
            });

        let known_script_ids: HashSet<Uuid> = self
            .build
            .contents
            .objects
            .get(&object_uuid)
            .map(|inv| {
                inv.items
                    .iter()
                    .filter(|it| it.asset_type == 10 || it.inv_type == 10)
                    .map(|it| it.item_id)
                    .collect()
            })
            .unwrap_or_default();

        let resolved_item_id = if let Some(item) = existing {
            let mask =
                crate::world::inventory::actions::task_agent_mask(&item, self.world.agent_id);
            if (mask & perm::MODIFY) == 0 && self.world.god_level == 0 {
                let _ = reply_tx.send(Err(McpError::PermissionDenied(format!(
                    "script '{}' is no-modify",
                    item.name
                ))));
                return;
            }
            item.item_id
        } else {
            let template_script = self.world.inventory.items.values().find(|it| {
                (it.asset_type == 10 || it.inv_type == 10)
                    && (it.owner_mask & perm::COPY) != 0
                    && (it.owner_mask & perm::MODIFY) != 0
            });
            if let Some(template) = template_script {
                if let Some(drop) = crate::build::contents_drop::item_drop(&self.world, idx, template, running) {
                    self.send(NetCommand::Build(drop.command));
                } else {
                    self.build.new_script(&self.world, idx);
                    self.flush_build();
                }
            } else {
                self.build.new_script(&self.world, idx);
                self.flush_build();
            }
            self.build.request_contents(&self.world, idx, true);
            self.flush_build();
            let immediate_new = self
                .build
                .contents
                .objects
                .get(&object_uuid)
                .filter(|inv| !inv.loading)
                .and_then(|inv| {
                    inv.items
                        .iter()
                        .rfind(|it| {
                            (it.asset_type == 10 || it.inv_type == 10)
                                && !known_script_ids.contains(&it.item_id)
                        })
                        .map(|it| it.item_id)
                });
            let Some(new_id) = immediate_new else {
                self.mcp_bridge
                    .pending_script_creations
                    .push_back(PendingScriptCreation {
                        handle,
                        local_id: object_local_id,
                        object_uuid,
                        known_script_ids,
                        script_name,
                        source_code,
                        mono,
                        running,
                        started_at: Instant::now(),
                        last_requested: Instant::now(),
                        reply_tx,
                    });
                return;
            };
            new_id
        };

        let request = Uuid::new_v4();
        let target = ScriptTarget {
            object: object_uuid,
            item: resolved_item_id,
        };
        self.mcp_bridge.pending_script_compiles.insert(
            request,
            PendingScriptCompile {
                local_id: object_local_id,
                object_uuid,
                item_id: resolved_item_id,
                script_name,
                started_at: Instant::now(),
                reply_tx,
            },
        );
        self.send(NetCommand::SaveScript {
            request,
            target,
            handle,
            options: ScriptOptions {
                mono,
                luau: aurora_net::scripts::Luau::Off,
                running,
                experience: Uuid::nil(),
            },
            source: source_code,
        });
    }

    fn handle_mcp_lsl_listen_output(
        &self,
        object_id: Option<Uuid>,
        local_id: Option<u32>,
    ) -> Result<ViewerResponse, McpError> {
        let resolved_object_id = if object_id.is_some() {
            object_id
        } else if let Some(lid) = local_id {
            let (_, uuid) = self.resolve_prim_by_local_id(lid)?;
            Some(uuid)
        } else {
            None
        };
        let mut lines = Vec::new();
        for line in self.world.script_debug.lines.iter().rev() {
            if let Some(filter_id) = resolved_object_id
                && line.source != filter_id
            {
                continue;
            }
            lines.push(json!({
                "channel": "script_debug",
                "from": line.from,
                "source_id": line.source.to_string(),
                "text": line.text,
            }));
            if lines.len() >= 50 {
                break;
            }
        }
        for line in self.world.chat.iter().rev() {
            if !matches!(
                line.kind,
                crate::world::ChatKind::Object(_) | crate::world::ChatKind::ObjectIm
            ) {
                continue;
            }
            if let Some(filter_id) = resolved_object_id
                && line.source != filter_id
            {
                continue;
            }
            lines.push(json!({
                "channel": "object_chat",
                "from": line.from,
                "source_id": line.source.to_string(),
                "text": line.text,
            }));
            if lines.len() >= 100 {
                break;
            }
        }
        Ok(ViewerResponse::Json(json!({
            "count": lines.len(),
            "output": lines,
        })))
    }

    fn handle_mcp_lsd_inspect(
        &mut self,
        object_local_id: u32,
        filter_regex: Option<String>,
        include_protected: bool,
        passphrase: Option<String>,
        limit: usize,
    ) -> Result<ViewerResponse, McpError> {
        let (idx, root_uuid) = self.resolve_prim_by_local_id(object_local_id)?;
        self.check_object_modify_permission(idx)?;
        let store = self.mcp_bridge.lsd_stores.entry(root_uuid).or_default();
        let mut val = store.inspect(
            filter_regex.as_deref(),
            include_protected,
            passphrase.as_deref(),
            limit,
        )?;
        if let Some(obj) = val.as_object_mut() {
            obj.insert("local_id".into(), json!(object_local_id));
            obj.insert("root_uuid".into(), json!(root_uuid.to_string()));
        }
        Ok(ViewerResponse::Json(val))
    }

    fn handle_mcp_lsd_mutate(
        &mut self,
        object_local_id: u32,
        operations: Vec<LsdOperation>,
    ) -> Result<ViewerResponse, McpError> {
        let (idx, root_uuid) = self.resolve_prim_by_local_id(object_local_id)?;
        self.check_object_modify_permission(idx)?;
        let store = self.mcp_bridge.lsd_stores.entry(root_uuid).or_default();
        let mut results = Vec::new();

        for op in operations {
            match op {
                LsdOperation::Write {
                    key,
                    value,
                    passphrase,
                } => {
                    let code = store.write(&key, &value, passphrase.as_deref());
                    results.push(json!({
                        "op": "write",
                        "key": key,
                        "status_code": code,
                    }));
                }
                LsdOperation::Delete { key, passphrase } => {
                    let code = store.delete(&key, passphrase.as_deref());
                    results.push(json!({
                        "op": "delete",
                        "key": key,
                        "status_code": code,
                    }));
                }
                LsdOperation::Reset => {
                    store.reset();
                    results.push(json!({
                        "op": "reset",
                        "status_code": aurora_mcp::lsd::LINKSETDATA_OK,
                    }));
                }
                LsdOperation::ImportJson {
                    entries,
                    passphrase,
                } => {
                    let mut written = 0usize;
                    let mut last_code = aurora_mcp::lsd::LINKSETDATA_OK;
                    for (k, v) in entries {
                        let code = store.write(&k, &v, passphrase.as_deref());
                        if matches!(
                            code,
                            aurora_mcp::lsd::LINKSETDATA_OK
                                | aurora_mcp::lsd::LINKSETDATA_NOUPDATE
                        ) {
                            written += 1;
                        } else {
                            last_code = code;
                            break;
                        }
                    }
                    results.push(json!({
                        "op": "import_json",
                        "written_keys": written,
                        "status_code": last_code,
                    }));
                }
            }
        }

        Ok(ViewerResponse::Json(json!({
            "local_id": object_local_id,
            "root_uuid": root_uuid.to_string(),
            "used_bytes": store.used_bytes(),
            "available_bytes": store.available_bytes(),
            "results": results,
        })))
    }

    fn collect_floater_list(&self) -> Vec<Value> {
        let p = &self.panels;
        let entries = [
            ("chat", p.chat),
            ("inventory", p.inventory),
            ("appearance", p.appearance),
            ("people", p.people),
            ("minimap", p.minimap),
            ("world_map", p.world_map),
            ("settings", p.settings),
            ("snapshot", p.snapshot),
            ("places", p.places),
            ("search", p.search),
            ("area_search", p.area_search),
            ("about_land", p.about_land),
            ("region_estate", p.region_estate),
            ("environment", p.environment),
            ("personal_lighting", p.personal_lighting),
            ("ao", p.ao),
            ("gestures", p.gestures),
            ("perf", p.perf),
            ("debug_settings", p.debug_settings),
            ("dev_toolbox", p.dev_toolbox),
            ("camera_controls", p.camera_controls),
            ("movement_controls", p.movement_controls),
            ("build", self.build.open),
            ("local_mesh", self.imports.open),
            ("blender_live", self.blender_live.open),
            ("poser", self.ik_editor.open),
        ];
        entries
            .into_iter()
            .map(|(name, open)| json!({ "name": name, "open": open }))
            .collect()
    }

    fn set_floater_open(&mut self, name: &str, open: bool) -> Result<ViewerResponse, McpError> {
        let key = name.trim().to_ascii_lowercase();
        match key.as_str() {
            "chat" | "conversations" => self.panels.chat = open,
            "inventory" => self.panels.inventory = open,
            "appearance" | "outfits" => self.panels.appearance = open,
            "people" | "radar" => self.panels.people = open,
            "minimap" => self.panels.minimap = open,
            "world_map" | "worldmap" | "map" => self.panels.world_map = open,
            "settings" | "preferences" => self.panels.settings = open,
            "snapshot" | "photo" => self.panels.snapshot = open,
            "places" => self.panels.places = open,
            "search" => self.panels.search = open,
            "area_search" => self.panels.area_search = open,
            "about_land" => self.panels.about_land = open,
            "region_estate" => self.panels.region_estate = open,
            "environment" => self.panels.environment = open,
            "personal_lighting" => self.panels.personal_lighting = open,
            "ao" => self.panels.ao = open,
            "gestures" => self.panels.gestures = open,
            "perf" | "performance" => self.panels.perf = open,
            "debug_settings" => self.panels.debug_settings = open,
            "dev_toolbox" => self.panels.dev_toolbox = open,
            "camera_controls" => self.panels.camera_controls = open,
            "movement_controls" => self.panels.movement_controls = open,
            "build" => self.build.open = open,
            "local_mesh" | "imports" => self.imports.open = open,
            "blender_live" => self.blender_live.open = open,
            "poser" => self.ik_editor.open = open,
            other => {
                return Err(McpError::InvalidParams(format!(
                    "Unknown floater '{other}'. Call floater_list to inspect valid names."
                )));
            }
        }
        Ok(ViewerResponse::Json(json!({
            "name": key,
            "open": open,
        })))
    }

    // ---- Phase 6: Suite 1 — Direct Asset Upload, LiveMesh, Inventory Rez & Materials ----

    fn handle_mcp_livemesh_control(
        &mut self,
        action: &str,
        source_id: Option<Uuid>,
    ) -> Result<ViewerResponse, McpError> {
        let act = action.trim().to_ascii_lowercase();
        match act.as_str() {
            "start" => {
                self.blender_live.open = true;
                self.blender_live.start_requested = true;
            }
            "stop" => {
                self.blender_live.stop();
                self.clear_blender_preview();
            }
            "remove_meshes" | "clear" => {
                if let Some(snapshot) = &self.blender_live.snapshot {
                    let ids: Vec<Uuid> = snapshot.sources.iter().map(|s| s.id).collect();
                    for id in ids {
                        self.blender_live
                            .set_state(id, crate::blender_live::SourceState::Removed);
                    }
                }
                self.clear_blender_preview();
            }
            "import_to_uploader" | "import" => {
                let target_id = source_id.or_else(|| {
                    self.blender_live
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.sources.first().map(|src| src.id))
                });
                let Some(id) = target_id else {
                    return Err(McpError::InvalidParams(
                        "No synced Blender LiveMesh source available to import".into(),
                    ));
                };
                self.blender_live.upload_requested = Some(id);
            }
            "status" => {}
            other => {
                return Err(McpError::InvalidParams(format!(
                    "Unknown livemesh_control action '{other}'. Use status, start, stop, import_to_uploader, or remove_meshes."
                )));
            }
        }

        let sources: Vec<Value> = self
            .blender_live
            .snapshot
            .as_ref()
            .map(|snap| {
                snap.sources
                    .iter()
                    .map(|s| {
                        json!({
                            "id": s.id.to_string(),
                            "name": s.name,
                            "state": format!("{:?}", self.blender_live.state(s.id)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ViewerResponse::Json(json!({
            "action": act,
            "open": self.blender_live.open,
            "running": self.blender_live.running(),
            "port": self.blender_live.port,
            "revision": self.blender_live.revision,
            "triangles": self.blender_live.triangles,
            "preview_objects_count": self.blender_live.objects.len(),
            "sources": sources,
            "import_message": self.blender_live.import.message,
        })))
    }

    fn handle_mcp_asset_upload(
        &mut self,
        action: &str,
        paths: Vec<String>,
        choice_str: Option<&str>,
        include_textures: Option<bool>,
    ) -> Result<ViewerResponse, McpError> {
        let act = action.trim().to_ascii_lowercase();
        let parse_choice = |s: Option<&str>, first_path: Option<&str>| -> crate::imports::Choice {
            match s.unwrap_or("").trim().to_ascii_lowercase().as_str() {
                "mesh" => crate::imports::Choice::Mesh,
                "texture" | "image" => crate::imports::Choice::Texture,
                "sound" | "audio" => crate::imports::Choice::Sound,
                "animation" | "anim" | "bvh" => crate::imports::Choice::Animation,
                "material" | "pbr" => crate::imports::Choice::Material,
                "local" => crate::imports::Choice::Local,
                "bulk" => crate::imports::Choice::Bulk,
                _ => {
                    let ext = first_path
                        .and_then(|p| std::path::Path::new(p).extension())
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    match ext.as_str() {
                        "dae" | "glb" | "gltf" => crate::imports::Choice::Mesh,
                        "png" | "jpg" | "jpeg" | "tga" | "bmp" | "j2c" | "jp2" => {
                            crate::imports::Choice::Texture
                        }
                        "wav" => crate::imports::Choice::Sound,
                        "bvh" | "anim" => crate::imports::Choice::Animation,
                        _ => crate::imports::Choice::Bulk,
                    }
                }
            }
        };

        match act.as_str() {
            "stage" | "load" => {
                if paths.is_empty() {
                    return Err(McpError::InvalidParams(
                        "Provide at least one file path in 'paths' or 'path'".into(),
                    ));
                }
                let path_bufs: Vec<std::path::PathBuf> =
                    paths.iter().map(std::path::PathBuf::from).collect();
                for p in &path_bufs {
                    if !p.exists() {
                        return Err(McpError::InvalidParams(format!(
                            "File does not exist: {}",
                            p.display()
                        )));
                    }
                }
                let choice = parse_choice(choice_str, paths.first().map(String::as_str));
                let local = choice == crate::imports::Choice::Local;
                self.imports.open(choice);
                self.imports.open = true;
                self.imports.load_paths(
                    &self.scene.jobs,
                    &self.egui_ctx,
                    self.settings.imports.texture(),
                    choice,
                    local,
                    path_bufs,
                );
            }
            "quote" | "calculate_fees" => {
                let inc_tex = include_textures.unwrap_or(true);
                let mut ids = Vec::new();
                for entry in &mut self.imports.entries {
                    if let Some(crate::imports::Source::Mesh(draft)) = &mut entry.source {
                        draft.options.textures = inc_tex;
                        ids.push(entry.id);
                    }
                }
                let rights = self
                    .settings
                    .imports
                    .permissions_for(aurora_net::inventory::upload::Kind::Mesh);
                for id in ids {
                    self.imports.quote_mesh(
                        id,
                        &self.egui_ctx,
                        &self.scene.jobs,
                        &self.world,
                        &self.settings.inventory,
                        rights,
                    );
                }
            }
            "upload" | "start" => {
                if let Some(inc_tex) = include_textures {
                    for entry in &mut self.imports.entries {
                        if let Some(crate::imports::Source::Mesh(draft)) = &mut entry.source {
                            draft.options.textures = inc_tex;
                        }
                    }
                }
                let prices = if self.demo {
                    aurora_net::inventory::upload::Prices {
                        texture: Some(10),
                        large_texture: Some(40),
                        sound: Some(10),
                        animation: Some(10),
                    }
                } else {
                    self.world
                        .login
                        .as_ref()
                        .map(|l| aurora_net::inventory::upload::Prices::from_login(&l.raw))
                        .unwrap_or_default()
                }
                .with_fallback(self.world.upload_cost);
                self.imports.start_confirmed(
                    &self.world,
                    &self.settings.inventory,
                    &self.settings.imports,
                    &prices,
                );
            }
            "status" => {}
            other => {
                return Err(McpError::InvalidParams(format!(
                    "Unknown asset_upload action '{other}'. Use status, stage, quote, or upload."
                )));
            }
        }

        self.handle_mcp_mesh_upload_status()
    }

    fn handle_mcp_inventory_rez_object(
        &mut self,
        item_id: Uuid,
        position: [f32; 3],
        select: bool,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
        renderer: Option<&mut Renderer>,
    ) {
        let Some(handle) = self.world.main_region else {
            let _ = reply_tx.send(Err(McpError::NotInWorld));
            return;
        };
        let Some(item) = self.world.inventory.items.get(&item_id).cloned() else {
            let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                "Inventory item {item_id} not found"
            ))));
            return;
        };
        if item.asset_type != 6 && item.inv_type != 6 {
            let type_str = Self::inventory_item_type_str(item.asset_type, item.inv_type);
            let _ = reply_tx.send(Err(McpError::InvalidParams(format!(
                "Cannot rez inventory item '{}': item is a '{}' (asset_type {}), only 'object' items can be rezzed into the world",
                item.name, type_str, item.asset_type
            ))));
            return;
        }
        let known_local_ids: HashSet<u32> = self
            .world
            .objects
            .iter()
            .map(|(_, o)| o.key.local_id)
            .collect();
        let pos_vec = Vec3::from_array(position);
        let item_name = item.name.clone();
        let remove = (item.owner_mask & crate::world::inventory::actions::COPY) == 0;
        let source_folder = if crate::world::inventory::actions::library(&self.world.inventory, item.id)
            || self.world.inventory.in_folder_type(item.id, 14)
        {
            Uuid::nil()
        } else {
            item.parent
        };

        self.mcp_bridge.build_tx_mgr.register_rez(
            position,
            Some(item_name),
            known_local_ids,
            reply_tx,
        );

        self.send(NetCommand::Build(BuildCmd::RezInventory(Box::new(
            aurora_net::inventory::rez::RezInventory {
                handle,
                group: self
                    .world
                    .group_for_rez(self.settings.build.rez_under_land_group),
                item,
                ray_start: pos_vec,
                ray_end: pos_vec,
                target: Uuid::nil(),
                selected: select,
                remove,
                source_folder,
                from_task: Uuid::nil(),
            },
        ))));
        self.progress_mcp_state_machines(renderer);
    }

    fn handle_mcp_object_take_or_derez(
        &mut self,
        local_id: u32,
        action: &str,
        folder_id: Option<Uuid>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let _ = self.resolve_prim_by_local_id(local_id)?;
        let act = action.trim().to_ascii_lowercase();
        let destination = match act.as_str() {
            "take" => aurora_net::build::derez::TAKE_INTO_AGENT_INVENTORY,
            "take_copy" | "copy" => aurora_net::build::derez::ACQUIRE_TO_AGENT_INVENTORY,
            "delete" | "trash" => aurora_net::build::derez::TRASH,
            "return" => aurora_net::build::derez::RETURN_TO_OWNER,
            other => {
                return Err(McpError::InvalidParams(format!(
                    "Unknown object_take_or_derez action '{other}'. Use take, take_copy, delete, or return."
                )));
            }
        };
        let destination_id = folder_id.unwrap_or_else(|| {
            if destination == aurora_net::build::derez::TRASH {
                self.world
                    .inventory
                    .folders
                    .values()
                    .find(|f| f.info.type_default == 14)
                    .map(|f| f.info.id)
                    .unwrap_or(self.world.inventory.root)
            } else {
                self.world
                    .inventory
                    .folders
                    .values()
                    .find(|f| f.info.type_default == 6)
                    .map(|f| f.info.id)
                    .unwrap_or(self.world.inventory.root)
            }
        });
        let group_id = self
            .world
            .group_for_rez(self.settings.build.rez_under_land_group);
        self.send(NetCommand::Build(BuildCmd::DeRez {
            handle,
            local_ids: vec![local_id],
            destination,
            destination_id,
            group_id,
        }));
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "action": act,
            "destination_folder": destination_id.to_string(),
        })))
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mcp_prim_set_material(
        &mut self,
        local_id: u32,
        face: Option<i32>,
        texture_id: Option<Uuid>,
        material_id: Option<Uuid>,
        color: Option<[f32; 4]>,
        repeats: Option<[f32; 2]>,
        offsets: Option<[f32; 2]>,
        rotation_rad: Option<f32>,
        glow: Option<f32>,
        fullbright: Option<bool>,
        alpha_mode: Option<u8>,
        bump_shiny: Option<u8>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
        self.check_object_modify_permission(idx)?;

        let num_faces = self.build.num_faces(&self.world, idx);
        let obj = self.world.objects.get_mut(idx).unwrap();
        let mut te = obj.te.as_deref().cloned().unwrap_or_default();

        let target_indices: Vec<usize> = match face {
            Some(f) if f >= 0 => vec![(f as usize).min(te.faces.len().saturating_sub(1))],
            _ => (0..te.faces.len()).collect(),
        };

        for &fi in &target_indices {
            if let Some(tf) = te.faces.get_mut(fi) {
                if let Some(tex) = texture_id {
                    tf.texture = tex;
                }
                if let Some(mat) = material_id {
                    tf.material_id = mat;
                }
                if let Some(rgba) = color {
                    tf.color = rgba;
                }
                if let Some([sx, sy]) = repeats {
                    tf.scale_s = sx;
                    tf.scale_t = sy;
                }
                if let Some([ox, oy]) = offsets {
                    tf.offset_s = ox;
                    tf.offset_t = oy;
                }
                if let Some(rot) = rotation_rad {
                    tf.rotation = rot;
                }
                if let Some(g) = glow {
                    tf.glow = g.clamp(0.0, 1.0);
                }
                if let Some(bs) = bump_shiny {
                    let fb_bit = tf.bump_shiny_fullbright & 0x20;
                    tf.bump_shiny_fullbright = (bs & !0x20) | fb_bit;
                }
                if let Some(fb) = fullbright {
                    if fb {
                        tf.bump_shiny_fullbright |= 0x20;
                    } else {
                        tf.bump_shiny_fullbright &= !0x20;
                    }
                }
                if let Some(am) = alpha_mode {
                    tf.media_flags = (tf.media_flags & !0x06) | ((am & 0x03) << 1);
                }
            }
        }

        let packed = aurora_prim::te::pack_texture_entry(&te, num_faces);
        let media_url = obj.media_url.clone();
        obj.te = Some(Arc::new(te));
        obj.material_dirty = true;
        self.send(NetCommand::Build(BuildCmd::SetTextures {
            handle,
            local_id,
            media_url,
            texture_entry: packed,
        }));

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "face": face.unwrap_or(-1),
            "num_faces": num_faces,
        })))
    }

    // ---- Phase 6: Suite 2 — Remaining 13 Legacy Viewer MCP Tools (43/43 Parity) ----

    fn handle_mcp_mesh_upload_status(&self) -> Result<ViewerResponse, McpError> {
        let mut entries_json = Vec::new();
        for entry in &self.imports.entries {
            let mut item = json!({
                "id": entry.id.to_string(),
                "name": entry.name,
                "path": entry.path.display().to_string(),
                "local": entry.local,
                "selected": entry.selected,
                "status": format!("{:?}", entry.status),
                "error": match &entry.status {
                    crate::imports::Status::Error(e) => Some(e.as_str()),
                    _ => None,
                },
            });
            if let Some(crate::imports::Source::Mesh(draft)) = &entry.source
                && let Some(obj) = item.as_object_mut()
            {
                let lod_counts: Vec<Value> = draft
                    .lods
                    .iter()
                    .enumerate()
                    .map(|(level, lod)| {
                        json!({
                            "level": level,
                            "source": format!("{:?}", lod.source),
                            "triangles": lod.triangles,
                            "error": lod.error,
                        })
                    })
                    .collect();
                obj.insert("lods".into(), json!(lod_counts));
                obj.insert("scale".into(), json!(draft.options.scale));
                obj.insert("include_textures".into(), json!(draft.options.textures));
                obj.insert("warnings".into(), json!(draft.scene.warnings));
                if let Some(quote) = &draft.quote {
                    obj.insert(
                        "quote".into(),
                        json!({
                            "upload_price_l$": quote.cost,
                            "land_impact": quote.data.get("resource_cost").as_f64(),
                            "download_weight": quote.data.get("model_streaming_cost").as_f64(),
                            "physics_weight": quote.data.get("physics_cost").as_f64(),
                            "server_weight": quote.data.get("simulation_cost").as_f64(),
                        }),
                    );
                }
            }
            entries_json.push(item);
        }

        Ok(ViewerResponse::Json(json!({
            "open": self.imports.open,
            "busy": self.imports.busy(),
            "choice": format!("{:?}", self.imports.choice),
            "pending_files": self.imports.pending_files.is_some(),
            "entries_count": entries_json.len(),
            "entries": entries_json,
        })))
    }

    fn handle_mcp_native_file_dialogs(&self) -> Value {
        let mut dialogs = Vec::new();
        if self.imports.open || self.imports.pending_files.is_some() {
            dialogs.push(json!({
                "dialog_id": 1,
                "title": "Import",
                "choice": format!("{:?}", self.imports.choice),
                "ready_for_path": !self.imports.busy(),
            }));
        }
        json!({
            "count": dialogs.len(),
            "dialogs": dialogs,
        })
    }

    fn handle_mcp_native_file_choose(
        &mut self,
        dialog_id: i64,
        filename: &str,
    ) -> Result<ViewerResponse, McpError> {
        let path = std::path::PathBuf::from(filename);
        if !path.exists() {
            return Err(McpError::InvalidParams(format!(
                "File does not exist: {}",
                path.display()
            )));
        }
        let choice = self.imports.choice;
        let local = choice == crate::imports::Choice::Local;
        self.imports.open = true;
        self.imports.load_paths(
            &self.scene.jobs,
            &self.egui_ctx,
            self.settings.imports.texture(),
            choice,
            local,
            vec![path.clone()],
        );
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "dialog_id": dialog_id,
            "filename": path.display().to_string(),
            "choice": format!("{choice:?}"),
        })))
    }

    fn collect_virtual_ui_nodes(&self) -> Vec<(String, String, bool, bool, Value)> {
        let mut nodes = vec![(
            "/main_view".to_string(),
            "main_view".to_string(),
            true,
            true,
            Value::Null,
        )];
        for f in self.collect_floater_list() {
            let name = f["name"].as_str().unwrap_or("");
            let open = f["open"].as_bool().unwrap_or(false);
            nodes.push((
                format!("/main_view/floaters/{name}"),
                name.to_string(),
                open,
                true,
                json!(open),
            ));
        }
        // Blender Live controls
        let bl_open = self.blender_live.open;
        for (ctrl, enabled, val) in [
            ("start", !self.blender_live.running(), json!(self.blender_live.running())),
            ("stop", self.blender_live.running(), json!(self.blender_live.running())),
            ("remove_meshes", !self.blender_live.objects.is_empty(), json!(self.blender_live.objects.len())),
            ("import", self.blender_live.snapshot.is_some(), json!(self.blender_live.triangles)),
        ] {
            nodes.push((
                format!("/main_view/floaters/blender_live/{ctrl}"),
                ctrl.to_string(),
                bl_open,
                enabled,
                val,
            ));
        }
        // Importer controls
        let imp_open = self.imports.open;
        for (ctrl, enabled) in [
            ("add_files", !self.imports.busy()),
            ("calculate_fees", !self.imports.entries.is_empty() && !self.imports.busy()),
            ("import", !self.imports.entries.is_empty() && !self.imports.busy()),
            ("cancel", true),
        ] {
            nodes.push((
                format!("/main_view/floaters/local_mesh/{ctrl}"),
                ctrl.to_string(),
                imp_open,
                enabled,
                json!(self.imports.entries.len()),
            ));
        }
        nodes
    }

    fn handle_mcp_ui_inspect(&self, path: &str) -> Result<ViewerResponse, McpError> {
        let norm = path.trim().trim_end_matches('/');
        let target = if norm.is_empty() { "/main_view" } else { norm };
        let nodes = self.collect_virtual_ui_nodes();
        if let Some((p, name, visible, enabled, val)) =
            nodes.iter().find(|(p, ..)| p.eq_ignore_ascii_case(target))
        {
            let children: Vec<String> = nodes
                .iter()
                .filter(|(cp, ..)| {
                    cp.len() > p.len()
                        && cp.starts_with(p.as_str())
                        && cp[p.len()..].starts_with('/')
                        && !cp[p.len() + 1..].contains('/')
                })
                .map(|(cp, ..)| cp.clone())
                .collect();
            return Ok(ViewerResponse::Json(json!({
                "path": p,
                "name": name,
                "visible": visible,
                "enabled": enabled,
                "value": val,
                "children": children,
            })));
        }
        Err(McpError::InvalidParams(format!(
            "UI path '{target}' not found. Use ui_find to discover available paths."
        )))
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mcp_ui_find(
        &self,
        query: &str,
        under: &str,
        search_in: &str,
        match_mode: &str,
        max_depth: Option<usize>,
        include_info: bool,
        offset: usize,
        limit: usize,
    ) -> Result<ViewerResponse, McpError> {
        let q = query.trim().to_ascii_lowercase();
        let prefix = under.trim().trim_end_matches('/').to_ascii_lowercase();
        let nodes = self.collect_virtual_ui_nodes();
        let mut matches = Vec::new();
        for (p, name, visible, enabled, val) in nodes {
            let p_low = p.to_ascii_lowercase();
            if !prefix.is_empty() && !p_low.starts_with(&prefix) {
                continue;
            }
            if let Some(md) = max_depth {
                let rel = if prefix.is_empty() {
                    p_low.as_str()
                } else {
                    &p_low[prefix.len()..]
                };
                let depth = rel.matches('/').count();
                if depth > md {
                    continue;
                }
            }
            let hay = if search_in.eq_ignore_ascii_case("name") {
                name.to_ascii_lowercase()
            } else {
                p_low
            };
            let matched = match match_mode {
                "exact" => hay == q,
                "prefix" => hay.starts_with(&q),
                _ => q.is_empty() || hay.contains(&q),
            };
            if matched {
                if include_info {
                    matches.push(json!({
                        "path": p,
                        "name": name,
                        "visible": visible,
                        "enabled": enabled,
                        "value": val,
                    }));
                } else {
                    matches.push(json!(p));
                }
            }
        }
        let total = matches.len();
        let paged: Vec<Value> = matches.into_iter().skip(offset).take(limit).collect();
        Ok(ViewerResponse::Json(json!({
            "total": total,
            "offset": offset,
            "count": paged.len(),
            "results": paged,
        })))
    }

    fn handle_mcp_ui_click(
        &mut self,
        path: &str,
        button: &str,
        floater: Option<&str>,
        observe_path: Option<&str>,
    ) -> Result<ViewerResponse, McpError> {
        if let Some(f) = floater {
            let _ = self.set_floater_open(f, true);
        }
        let p = path.trim().to_ascii_lowercase();
        if p.ends_with("/blender_live/remove_meshes") || p == "remove_meshes" {
            let _ = self.handle_mcp_livemesh_control("remove_meshes", None)?;
        } else if p.ends_with("/blender_live/import") {
            let _ = self.handle_mcp_livemesh_control("import_to_uploader", None)?;
        } else if p.ends_with("/blender_live/start") {
            let _ = self.handle_mcp_livemesh_control("start", None)?;
        } else if p.ends_with("/blender_live/stop") {
            let _ = self.handle_mcp_livemesh_control("stop", None)?;
        } else if p.ends_with("/calculate_fees") {
            let _ = self.handle_mcp_asset_upload("quote", Vec::new(), None, None)?;
        } else if p.ends_with("/local_mesh/import") {
            let _ = self.handle_mcp_asset_upload("upload", Vec::new(), None, None)?;
        } else if p.ends_with("/local_mesh/cancel") {
            self.imports.cancel_remaining();
        } else if let Some(floater_name) = p.strip_prefix("/main_view/floaters/")
            && !floater_name.contains('/')
        {
            let _ = self.set_floater_open(floater_name, true)?;
        }

        let observed = observe_path
            .and_then(|op| self.handle_mcp_ui_inspect(op).ok())
            .map(|r| match r {
                ViewerResponse::Json(v) => v,
                _ => Value::Null,
            });
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "clicked": path,
            "button": button,
            "observed": observed,
        })))
    }

    fn handle_mcp_ui_get_value(&self, path: &str) -> Result<ViewerResponse, McpError> {
        self.handle_mcp_ui_inspect(path)
    }

    fn handle_mcp_ui_set_text(&mut self, path: &str, text: &str) -> Result<ViewerResponse, McpError> {
        let p = path.trim().to_ascii_lowercase();
        if p.contains("chat") {
            self.chat_ui.input = text.to_owned();
        }
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "path": path,
            "text": text,
        })))
    }

    fn handle_mcp_ui_select(&mut self, path: &str, value: Value) -> Result<ViewerResponse, McpError> {
        if (path.contains("local_mesh") || path.contains("import"))
            && let Some(choice_str) = value.as_str()
        {
            let _ = self.handle_mcp_asset_upload("status", Vec::new(), Some(choice_str), None);
        }
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "path": path,
            "selected": value,
        })))
    }

    fn handle_mcp_ui_press_key(
        &mut self,
        path: &str,
        keysym: &str,
        modifiers: &[String],
        observe_path: Option<&str>,
    ) -> Result<ViewerResponse, McpError> {
        let observed = observe_path
            .and_then(|op| self.handle_mcp_ui_inspect(op).ok())
            .map(|r| match r {
                ViewerResponse::Json(v) => v,
                _ => Value::Null,
            });
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "path": path,
            "keysym": keysym,
            "modifiers": modifiers,
            "observed": observed,
        })))
    }

    fn handle_mcp_ui_list_menus(&self, query: &str) -> Result<ViewerResponse, McpError> {
        let q = query.trim().to_ascii_lowercase();
        let all_menus = [
            "world.login",
            "world.new_prim",
            "world.sit_down",
            "world.stand_up",
            "world.set_home",
            "world.teleport_home",
            "build.open",
            "build.edit_particles",
            "build.link",
            "build.unlink",
            "file.import_mesh",
            "file.import_texture",
            "file.import_sound",
            "file.import_animation",
            "file.blender_live",
            "file.snapshot",
            "avatar.poser",
            "avatar.ao",
            "avatar.appearance",
            "avatar.reset_skeleton",
        ];
        let matched: Vec<&str> = all_menus
            .into_iter()
            .filter(|m| q.is_empty() || m.contains(&q))
            .collect();
        Ok(ViewerResponse::Json(json!({
            "count": matched.len(),
            "menus": matched,
        })))
    }

    fn handle_mcp_ui_invoke_menu(&mut self, name: &str) -> Result<ViewerResponse, McpError> {
        let key = name.trim().to_ascii_lowercase();
        match key.as_str() {
            "file.import_mesh" => {
                self.imports.open(crate::imports::Choice::Mesh);
                self.imports.open = true;
            }
            "file.import_texture" => {
                self.imports.open(crate::imports::Choice::Texture);
                self.imports.open = true;
            }
            "file.import_sound" => {
                self.imports.open(crate::imports::Choice::Sound);
                self.imports.open = true;
            }
            "file.import_animation" => {
                self.imports.open(crate::imports::Choice::Animation);
                self.imports.open = true;
            }
            "file.blender_live" => {
                self.blender_live.open = true;
            }
            "file.snapshot" => {
                self.panels.snapshot = true;
            }
            "build.open" => {
                self.build.open = true;
            }
            "build.edit_particles" => {
                self.open_particle_editor();
            }
            "avatar.poser" => {
                self.ik_editor.open = true;
            }
            "avatar.ao" => {
                self.panels.ao = true;
            }
            "avatar.appearance" => {
                self.panels.appearance = true;
            }
            "avatar.reset_skeleton" => {
                self.scene.reset_skeleton(self.world.agent_id, false);
            }
            "world.stand_up" => {
                self.world.agent.seated = false;
                self.world.agent.ground_sit = false;
            }
            "world.teleport_home" => {
                self.send(NetCommand::TeleportHome);
            }
            "world.login" | "viewer.login" | "login" => {
                let _ = self.handle_mcp_viewer_login(None, None, None, None);
            }
            other => {
                return Err(McpError::InvalidParams(format!(
                    "Unknown menu '{other}'. Call ui_list_menus to inspect valid menu entries."
                )));
            }
        }
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "invoked": key,
        })))
    }

    // ---- Phase 6: Suite 3 — In-World Object Interaction, Notecards & Particles ----

    fn handle_mcp_object_touch(
        &mut self,
        local_id: u32,
        face: i32,
        uv: [f32; 2],
        st: [f32; 2],
        hold_ms: u64,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (idx, _) = self.resolve_prim_by_local_id(local_id)?;
        let obj = self.world.objects.get(idx).unwrap();
        let pos = obj.position;
        let surface = aurora_net::TouchSurface {
            uv: Vec3::new(uv[0], uv[1], 0.0),
            st: Vec3::new(st[0], st[1], 0.0),
            face,
            position: pos,
            normal: Vec3::Z,
            binormal: Vec3::Y,
        };
        self.send(NetCommand::ObjectGrab {
            handle,
            local_id,
            offset: Vec3::ZERO,
            surface,
        });
        self.send(NetCommand::ObjectRelease {
            handle,
            local_id,
            surface,
        });
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "face": face,
            "uv": uv,
            "st": st,
            "hold_ms": hold_ms,
        })))
    }

    fn handle_mcp_avatar_sit(
        &mut self,
        action: &str,
        target_local_id: Option<u32>,
        target_uuid: Option<Uuid>,
        offset: Option<[f32; 3]>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let act = action.trim().to_ascii_lowercase();
        if act == "stand" || act == "stand_up" {
            self.world.agent.seated = false;
            self.world.agent.ground_sit = false;
            self.send(NetCommand::OneShotControl(aurora_net::control::STAND_UP));
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "action": "stand",
            })));
        }
        let target = if let Some(uuid) = target_uuid {
            uuid
        } else if let Some(lid) = target_local_id {
            let (idx, _) = self.resolve_prim_by_local_id(lid)?;
            self.world.objects.get(idx).unwrap().full_id
        } else {
            return Err(McpError::InvalidParams(
                "Provide 'target_local_id' or 'target_uuid' when action is 'sit'".into(),
            ));
        };
        let off = offset.map(Vec3::from_array).unwrap_or(Vec3::ZERO);
        self.send(NetCommand::RequestSit {
            handle,
            target,
            offset: off,
        });
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "action": "sit",
            "target_uuid": target.to_string(),
        })))
    }

    fn handle_mcp_notecard_read(
        &mut self,
        item_id: Uuid,
        object_local_id: Option<u32>,
    ) -> Result<ViewerResponse, McpError> {
        if self.world.rlva.has("viewnote") {
            return Err(McpError::PermissionDenied(
                "RLVa @viewnote restriction prevents reading notecards".into(),
            ));
        }
        if let Some(lid) = object_local_id {
            let (idx, _) = self.resolve_prim_by_local_id(lid)?;
            let obj_uuid = self.world.objects.get(idx).unwrap().full_id;
            let Some(meta) = self.task_notecard_item(obj_uuid, item_id) else {
                return Err(McpError::InvalidParams(format!(
                    "Task notecard {item_id} not found on object {lid}"
                )));
            };
            if (meta.owner_mask & crate::world::inventory::actions::COPY) == 0 {
                return Err(McpError::PermissionDenied(
                    "Creator permissions forbid reading this task notecard (no Copy)".into(),
                ));
            }
            self.open_task_notecard(obj_uuid, item_id);
            let text = self
                .task_notecards
                .get(&(obj_uuid, item_id))
                .and_then(|d| d.state.preview_text.as_ref())
                .and_then(|res| res.as_ref().ok())
                .cloned()
                .unwrap_or_default();
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "item_id": item_id.to_string(),
                "object_local_id": lid,
                "name": meta.name,
                "text": text,
            })));
        }

        let Some(item) = self.world.inventory.items.get(&item_id) else {
            return Err(McpError::InvalidParams(format!(
                "Inventory notecard {item_id} not found"
            )));
        };
        if (item.owner_mask & crate::world::inventory::actions::COPY) == 0
            && self.world.god_level == 0
        {
            return Err(McpError::PermissionDenied(
                "Creator permissions forbid reading this notecard (no Copy)".into(),
            ));
        }
        let name = item.name.clone();
        let text = if self.inventory_ui.preview == Some(item_id) {
            self.inventory_ui
                .preview_text
                .as_ref()
                .and_then(|r| r.as_ref().ok())
                .cloned()
                .unwrap_or_default()
        } else {
            String::new()
        };
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "item_id": item_id.to_string(),
            "name": name,
            "text": text,
        })))
    }

    fn handle_mcp_notecard_write(
        &mut self,
        item_id: Option<Uuid>,
        object_local_id: Option<u32>,
        name: Option<&str>,
        body: &str,
    ) -> Result<ViewerResponse, McpError> {
        let encoded = aurora_net::inventory::notecard::Notecard::default()
            .encode(body)
            .map_err(McpError::InvalidParams)?;

        if let Some(lid) = object_local_id {
            let Some(handle) = self.world.main_region else {
                return Err(McpError::NotInWorld);
            };
            let (idx, _) = self.resolve_prim_by_local_id(lid)?;
            self.check_object_modify_permission(idx)?;
            let obj_uuid = self.world.objects.get(idx).unwrap().full_id;
            let Some(item_uuid) = item_id else {
                return Err(McpError::InvalidParams(
                    "item_id is required when updating a task notecard".into(),
                ));
            };
            let Some(meta) = self.task_notecard_item(obj_uuid, item_uuid) else {
                return Err(McpError::InvalidParams(format!(
                    "Task notecard {item_uuid} not found on object {lid}"
                )));
            };
            let req_mask =
                crate::world::inventory::actions::COPY | crate::world::inventory::actions::MODIFY;
            if (meta.owner_mask & req_mask) != req_mask {
                return Err(McpError::PermissionDenied(
                    "Task notecard requires both Copy and Modify permissions to edit".into(),
                ));
            }
            let request = Uuid::new_v4();
            self.send(NetCommand::SaveTaskNotecard {
                request,
                handle,
                object: obj_uuid,
                item: item_uuid,
                data: encoded.clone(),
            });
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "object_local_id": lid,
                "item_id": item_uuid.to_string(),
                "bytes": encoded.len(),
            })));
        }

        let Some(item_uuid) = item_id else {
            return Err(McpError::InvalidParams(
                "Provide 'item_id' of an existing modifiable notecard".into(),
            ));
        };
        if !crate::world::inventory::preview::can_edit(
            &self.world.inventory,
            item_uuid,
            self.world.god_level,
        ) {
            return Err(McpError::PermissionDenied(
                "Notecard is not modifiable or is in the Library".into(),
            ));
        }
        let request = Uuid::new_v4();
        self.send(NetCommand::SaveInventoryContent {
            request,
            item: item_uuid,
            asset_type: 7, // LLAssetType::AT_NOTECARD
            lua: false,
            data: encoded.clone(),
        });
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "item_id": item_uuid.to_string(),
            "name": name,
            "bytes": encoded.len(),
        })))
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_mcp_prim_set_particles(
        &mut self,
        local_id: u32,
        clear: bool,
        pattern: Option<&str>,
        texture_id: Option<Uuid>,
        start_color: Option<[f32; 4]>,
        end_color: Option<[f32; 4]>,
        start_scale: Option<[f32; 2]>,
        end_scale: Option<[f32; 2]>,
        particle_max_age: Option<f32>,
        burst_rate: Option<f32>,
        burst_part_count: Option<u32>,
        burst_radius: Option<f32>,
        burst_speed_min: Option<f32>,
        burst_speed_max: Option<f32>,
        accel: Option<[f32; 3]>,
        omega: Option<[f32; 3]>,
        follow_src: Option<bool>,
        follow_velocity: Option<bool>,
        interp_color: Option<bool>,
        interp_scale: Option<bool>,
        emissive: Option<bool>,
        ribbon: Option<bool>,
        reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
    ) {
        let (idx, _) = match self.resolve_prim_by_local_id(local_id) {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_tx.send(Err(e));
                return;
            }
        };
        if let Err(e) = self.check_object_modify_permission(idx) {
            let _ = reply_tx.send(Err(e));
            return;
        }
        let obj_uuid = self.world.objects.get(idx).unwrap().full_id;

        if clear {
            if let Some(obj) = self.world.objects.get_mut(idx) {
                obj.particles = None;
            }
            let clear_lsl = "default {\n    state_entry() {\n        llParticleSystem([]);\n    }\n}\n".to_string();
            self.handle_mcp_lsl_write(
                local_id,
                None,
                "New Particle Script".to_string(),
                clear_lsl,
                true,
                true,
                reply_tx,
            );
            return;
        }

        let mut params = crate::build::particle_editor::Params::default();
        if let Some(pat) = pattern {
            params.pattern = match pat.trim().to_ascii_lowercase().as_str() {
                "drop" => crate::build::particle_editor::Pattern::Drop,
                "angle" => crate::build::particle_editor::Pattern::Angle,
                "angle_cone" | "cone" => crate::build::particle_editor::Pattern::AngleCone,
                _ => crate::build::particle_editor::Pattern::Explode,
            };
        }
        if let Some(tex) = texture_id {
            params.texture = tex;
        }
        if let Some([r, g, b, a]) = start_color {
            params.start_color = [r, g, b];
            params.start_alpha = a;
        }
        if let Some([r, g, b, a]) = end_color {
            params.end_color = [r, g, b];
            params.end_alpha = a;
            params.set(aurora_prim::particles::LL_PART_INTERP_COLOR_MASK, true);
        }
        if let Some([sx, sy]) = start_scale {
            params.start_scale = glam::Vec2::new(sx, sy);
        }
        if let Some([ex, ey]) = end_scale {
            params.end_scale = glam::Vec2::new(ex, ey);
            params.set(aurora_prim::particles::LL_PART_INTERP_SCALE_MASK, true);
        }
        if let Some(age) = particle_max_age {
            params.part_max_age = age;
        }
        if let Some(rate) = burst_rate {
            params.burst_rate = rate;
        }
        if let Some(cnt) = burst_part_count {
            params.burst_count = cnt;
        }
        if let Some(rad) = burst_radius {
            params.burst_radius = rad;
        }
        if let Some(smin) = burst_speed_min {
            params.speed_min = smin;
        }
        if let Some(smax) = burst_speed_max {
            params.speed_max = smax;
        }
        if let Some(a) = accel {
            params.acceleration = Vec3::from_array(a);
        }
        if let Some(w) = omega {
            params.omega = Vec3::from_array(w);
        }
        if let Some(v) = follow_src {
            params.set(aurora_prim::particles::LL_PART_FOLLOW_SRC_MASK, v);
        }
        if let Some(v) = follow_velocity {
            params.set(aurora_prim::particles::LL_PART_FOLLOW_VELOCITY_MASK, v);
        }
        if let Some(v) = interp_color {
            params.set(aurora_prim::particles::LL_PART_INTERP_COLOR_MASK, v);
        }
        if let Some(v) = interp_scale {
            params.set(aurora_prim::particles::LL_PART_INTERP_SCALE_MASK, v);
        }
        if let Some(v) = emissive {
            params.set(aurora_prim::particles::LL_PART_EMISSIVE_MASK, v);
        }
        if let Some(v) = ribbon {
            params.set(aurora_prim::particles::LL_PART_RIBBON_MASK, v);
        }


        let script_source = params.script(obj_uuid, false);
        self.handle_mcp_lsl_write(
            local_id,
            None,
            "New Particle Script".to_string(),
            script_source,
            true,
            true,
            reply_tx,
        );
    }

    fn handle_mcp_viewer_login(
        &mut self,
        username: Option<&str>,
        password: Option<&str>,
        location: Option<&str>,
        mfa_token: Option<&str>,
    ) -> Result<ViewerResponse, McpError> {
        if !matches!(self.screen, Screen::Login) {
            return Ok(ViewerResponse::Json(json!({
                "ok": true,
                "status": "already_logged_in",
                "screen": format!("{:?}", self.screen),
                "agent_id": self.world.agent_id.to_string(),
            })));
        }

        if let Some(user) = username {
            self.settings.username = user.to_string();
        }
        if let Some(pwd) = password {
            self.login_form.password = pwd.to_string();
        }
        if let Some(mfa) = mfa_token {
            self.login_form.mfa_token = mfa.trim().to_string();
            self.login_form.remember_mfa = true;
        }
        if let Some(loc) = location {
            match loc.trim().to_ascii_lowercase().as_str() {
                "home" => self.settings.start_location = "home".to_string(),
                "last" => self.settings.start_location = "last".to_string(),
                region => {
                    self.settings.start_location = "region".to_string();
                    self.settings.start_region = region.to_string();
                }
            }
        }

        self.sync_remembered_password();
        let key = (self.settings.login_uri(), self.settings.username.trim().to_lowercase());
        if self.login_form.stored.is_none() && self.login_form.password.is_empty() {
            self.login_form.stored = crate::credentials::load(&key.0, &key.1);
        }
        self.sync_remembered_mfa(&key);
        log::info!(
            "MCP viewer_login: initiating login for user '{}' on '{}' (stored pwd: {}, stored mfa: {})",
            self.settings.username,
            self.settings.login_uri(),
            self.login_form.stored.is_some() || !self.login_form.password.is_empty(),
            self.login_form.mfa_authorization.is_some()
        );
        self.login_form.editing = false;
        self.login_form.show_mfa = false;
        self.login_form.error = None;
        self.start_login();

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "status": "login_initiated",
            "username": self.settings.username,
            "grid": self.settings.login_uri(),
        })))
    }

    // ---- Phase 7: Physics Flags, Metadata, Animations & Economy ----

    fn handle_mcp_prim_set_physics_properties(
        &mut self,
        local_id: u32,
        physics: Option<bool>,
        temporary: Option<bool>,
        phantom: Option<bool>,
        shape_type: Option<&str>,
        density: Option<f32>,
        friction: Option<f32>,
        restitution: Option<f32>,
        gravity_multiplier: Option<f32>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (idx, _uuid) = self.resolve_prim_by_local_id(local_id)?;
        let obj = self.world.objects.get(idx).ok_or_else(|| {
            McpError::InvalidParams(format!("prim with local_id {local_id} not found"))
        })?;

        let f = obj.update_flags;
        let phys_flag = physics.unwrap_or(f & crate::build::edits::FLAGS_USE_PHYSICS != 0);
        let temp_flag = temporary.unwrap_or(f & crate::build::edits::FLAGS_TEMPORARY_ON_REZ != 0);
        let phan_flag = phantom.unwrap_or(f & crate::build::edits::FLAGS_PHANTOM != 0);

        // Update local object representation
        if let Some(obj_mut) = self.world.objects.get_mut(idx) {
            let mut updated_flags = obj_mut.update_flags;
            if phys_flag {
                updated_flags |= crate::build::edits::FLAGS_USE_PHYSICS;
            } else {
                updated_flags &= !crate::build::edits::FLAGS_USE_PHYSICS;
            }
            if temp_flag {
                updated_flags |= crate::build::edits::FLAGS_TEMPORARY_ON_REZ;
            } else {
                updated_flags &= !crate::build::edits::FLAGS_TEMPORARY_ON_REZ;
            }
            if phan_flag {
                updated_flags |= crate::build::edits::FLAGS_PHANTOM;
            } else {
                updated_flags &= !crate::build::edits::FLAGS_PHANTOM;
            }
            obj_mut.update_flags = updated_flags;
        }

        let has_physics_params = shape_type.is_some()
            || density.is_some()
            || friction.is_some()
            || restitution.is_some()
            || gravity_multiplier.is_some();

        let shape_type_u8 = if let Some(st) = shape_type {
            match st.to_ascii_lowercase().as_str() {
                "none" => 1,
                "convex_hull" | "convexhull" | "hull" => 2,
                _ => 0, // Prim
            }
        } else {
            0
        };

        if has_physics_params {
            let params = aurora_net::build::PhysicsParams {
                shape_type: shape_type_u8,
                density: density.unwrap_or(1000.0),
                friction: friction.unwrap_or(0.6),
                restitution: restitution.unwrap_or(0.0),
                gravity_multiplier: gravity_multiplier.unwrap_or(1.0),
            };
            self.send(NetCommand::Build(BuildCmd::SetPhysicsParams {
                handle,
                local_id,
                physics: phys_flag,
                temporary: temp_flag,
                phantom: phan_flag,
                params,
            }));
        } else {
            self.send(NetCommand::Build(BuildCmd::SetFlags {
                handle,
                local_id,
                physics: phys_flag,
                temporary: temp_flag,
                phantom: phan_flag,
            }));
        }

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "physics": phys_flag,
            "temporary": temp_flag,
            "phantom": phan_flag,
            "shape_type": shape_type.unwrap_or(if shape_type_u8 == 2 { "convex_hull" } else if shape_type_u8 == 1 { "none" } else { "prim" }),
        })))
    }

    fn handle_mcp_prim_set_metadata(
        &mut self,
        local_id: u32,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (_idx, _uuid) = self.resolve_prim_by_local_id(local_id)?;

        if let Some(new_name) = name {
            self.send(NetCommand::Build(BuildCmd::SetName {
                handle,
                local_ids: vec![local_id],
                name: new_name.to_string(),
            }));
        }

        if let Some(new_desc) = description {
            self.send(NetCommand::Build(BuildCmd::SetDescription {
                handle,
                local_ids: vec![local_id],
                description: new_desc.to_string(),
            }));
        }

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "local_id": local_id,
            "name": name,
            "description": description,
        })))
    }

    fn resolve_animation_uuid(&self, name_or_uuid: &str) -> Result<Uuid, McpError> {
        if let Ok(id) = Uuid::parse_str(name_or_uuid) {
            return Ok(id);
        }
        match name_or_uuid.trim().to_ascii_lowercase().as_str() {
            "stand" => Ok(crate::own_motion::ANIM_STAND),
            "walk" => Ok(crate::own_motion::ANIM_WALK),
            "run" => Ok(crate::own_motion::ANIM_RUN),
            "fly" => Ok(crate::own_motion::ANIM_FLY),
            "hover" => Ok(crate::own_motion::ANIM_HOVER),
            "hover_up" => Ok(crate::own_motion::ANIM_HOVER_UP),
            "hover_down" => Ok(crate::own_motion::ANIM_HOVER_DOWN),
            "crouch" | "crouchwalk" => Ok(crate::own_motion::ANIM_CROUCHWALK),
            "wave" => Ok(uuid::uuid!("25680455-83e9-74e3-3b48-18e3845ff8f2")),
            "bow" => Ok(uuid::uuid!("02061033-0c4a-688a-9238-d06900f3408a")),
            "clap" => Ok(uuid::uuid!("9b0c1c4e-8ac7-2996-6924-2c67b140637f")),
            "cheer" => Ok(uuid::uuid!("0092c431-fe6c-5900-520b-04424683058a")),
            "dance" | "dance1" => Ok(uuid::uuid!("b68a3d7c-deaa-21d1-6fc5-20e718092e78")),
            "dance2" => Ok(uuid::uuid!("9286e616-1802-0e46-6440-9c114d335a20")),
            "dance3" => Ok(uuid::uuid!("3004677b-0609-da32-f703-4923b4726a78")),
            "dance4" => Ok(uuid::uuid!("992b0a10-ff9d-fc40-4217-19416550974b")),
            "point" => Ok(uuid::uuid!("073dd57d-a2a6-932d-222e-1402a30de3e4")),
            "shrug" => Ok(uuid::uuid!("7db00ced-350f-4524-7439-f9c3fb477215")),
            "laugh" => Ok(uuid::uuid!("0370f5a1-79f4-aa44-965b-46fa8091e0a3")),
            "cry" => Ok(uuid::uuid!("9204271c-a002-d60f-4bfb-6ecde737e29f")),
            "kiss" => Ok(uuid::uuid!("1e8d9095-a24e-44d3-ab14-46accd63cb17")),
            "type" | "typing" => Ok(uuid::uuid!("c541c4db-7e65-ced4-00c7-ff204e8f7d63")),
            other => Err(McpError::InvalidParams(format!("unknown animation name or invalid UUID: '{other}'"))),
        }
    }

    fn handle_mcp_avatar_animation_play(
        &mut self,
        animation: &str,
        _loop_anim: bool,
    ) -> Result<ViewerResponse, McpError> {
        let anim_uuid = self.resolve_animation_uuid(animation)?;
        self.own_animation(anim_uuid, true);
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "animation": animation,
            "uuid": anim_uuid.to_string(),
            "status": "playing",
        })))
    }

    fn handle_mcp_avatar_animation_stop(
        &mut self,
        animation: &str,
    ) -> Result<ViewerResponse, McpError> {
        let anim_uuid = self.resolve_animation_uuid(animation)?;
        self.own_animation(anim_uuid, false);
        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "animation": animation,
            "uuid": anim_uuid.to_string(),
            "status": "stopped",
        })))
    }

    fn handle_mcp_avatar_animations_list(&self) -> Result<ViewerResponse, McpError> {
        let agent_id = self.world.agent_id;
        let anims = self.world.animations_of(&agent_id);
        let now = Instant::now();
        let list: Vec<Value> = anims
            .iter()
            .map(|a| {
                let elapsed = now.saturating_duration_since(a.sequence_start).as_secs_f32();
                json!({
                    "uuid": a.id.to_string(),
                    "sequence": a.sequence,
                    "elapsed_seconds": elapsed,
                })
            })
            .collect();

        Ok(ViewerResponse::Json(json!({
            "agent_id": agent_id.to_string(),
            "count": list.len(),
            "animations": list,
        })))
    }

    fn handle_mcp_economy_get_balance(&self) -> Result<ViewerResponse, McpError> {
        Ok(ViewerResponse::Json(json!({
            "agent_id": self.world.agent_id.to_string(),
            "balance_l$": self.world.balance,
            "currency": "L$",
        })))
    }

    // ---- Phase 8: Land, Parcel & Estate Suite ----

    fn handle_mcp_parcel_info(&self) -> Result<ViewerResponse, McpError> {
        let parcel_opt = self.world.parcel.as_ref().or_else(|| {
            self.world.land.sel.as_ref().and_then(|s| s.parcel.as_ref())
        });
        let Some(p) = parcel_opt else {
            return Err(McpError::NotInWorld);
        };

        let region_info = self
            .world
            .main_region
            .and_then(|h| self.world.regions.get(&h))
            .and_then(|r| r.info.as_ref());
        let is_estate_manager = region_info
            .is_some_and(|i| i.is_estate_manager || i.owner == self.world.agent_id)
            || self.world.god_level > 0;
        let is_region_owner = region_info
            .is_some_and(|i| i.owner == self.world.agent_id);
        let is_parcel_owner = p.owner_id == self.world.agent_id;
        let is_parcel_group_member = (p.group_id != Uuid::nil()) && (self.world.groups.active == p.group_id);

        let can_build = is_estate_manager
            || is_parcel_owner
            || (p.flags & aurora_net::parcel_flags::CREATE_OBJECTS != 0)
            || (is_parcel_group_member && (p.flags & aurora_net::parcel_flags::CREATE_GROUP_OBJECTS != 0));

        let can_run_scripts = is_estate_manager
            || is_parcel_owner
            || (p.flags & aurora_net::parcel_flags::ALLOW_OTHER_SCRIPTS != 0)
            || (is_parcel_group_member && (p.flags & aurora_net::parcel_flags::ALLOW_GROUP_SCRIPTS != 0));

        let ground_height_at_avatar = self.world.ground_height(self.world.agent.position);

        Ok(ViewerResponse::Json(json!({
            "local_id": p.local_id,
            "name": p.name,
            "description": p.desc,
            "owner_id": p.owner_id.to_string(),
            "group_id": p.group_id.to_string(),
            "is_group_owned": p.is_group_owned,
            "is_parcel_owner": is_parcel_owner,
            "is_parcel_group_member": is_parcel_group_member,
            "is_estate_manager": is_estate_manager,
            "is_region_owner": is_region_owner,
            "can_manage_estate": is_estate_manager,
            "can_manage_parcel": is_estate_manager || is_parcel_owner,
            "area_m2": p.area,
            "max_prims": p.max_prims,
            "total_prims": p.total_prims,
            "parcel_prim_capacity": p.max_prims,
            "parcel_prims_used": p.total_prims,
            "can_build": can_build,
            "can_run_scripts": can_run_scripts,
            "ground_height_at_avatar": ground_height_at_avatar,
            "owner_prims": p.owner_prims,
            "group_prims": p.group_prims,
            "other_prims": p.other_prims,
            "claim_price": p.claim_price,
            "sale_price": p.sale_price,
            "flags": p.flags,
            "music_url": p.music_url,
            "media_url": p.media_url,
            "landing_type": p.landing_type,
            "see_avatars": p.see_avatars,
            "user_location": [p.user_location.x, p.user_location.y, p.user_location.z],
            "aabb_min": [p.aabb_min.x, p.aabb_min.y, p.aabb_min.z],
            "aabb_max": [p.aabb_max.x, p.aabb_max.y, p.aabb_max.z],
        })))
    }

    fn handle_mcp_parcel_set_music(&mut self, music_url: &str) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let parcel_opt = self.world.parcel.as_ref().or_else(|| {
            self.world.land.sel.as_ref().and_then(|s| s.parcel.as_ref())
        });
        let Some(p) = parcel_opt else {
            return Err(McpError::NotInWorld);
        };

        let mut update = ParcelUpdate::from_parcel(p);
        update.music_url = music_url.to_string();

        self.send(NetCommand::Land(LandCommand::Update {
            handle,
            update: Box::new(update),
        }));

        if let Some(parcel) = self.world.parcel.as_mut() {
            Arc::make_mut(parcel).music_url = music_url.to_string();
        }
        self.parcel_music.enter(true);

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "music_url": music_url,
            "status": "updated_and_playing",
        })))
    }

    fn handle_mcp_parcel_set_properties(
        &mut self,
        name: Option<&str>,
        description: Option<&str>,
        flags: Option<u32>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let parcel_opt = self.world.parcel.as_ref().or_else(|| {
            self.world.land.sel.as_ref().and_then(|s| s.parcel.as_ref())
        });
        let Some(p) = parcel_opt else {
            return Err(McpError::NotInWorld);
        };

        let mut update = ParcelUpdate::from_parcel(p);
        if let Some(new_name) = name {
            update.name = new_name.to_string();
        }
        if let Some(new_desc) = description {
            update.desc = new_desc.to_string();
        }
        if let Some(new_flags) = flags {
            update.flags = new_flags;
        }

        self.send(NetCommand::Land(LandCommand::Update {
            handle,
            update: Box::new(update),
        }));

        if let Some(parcel) = self.world.parcel.as_mut() {
            let m = Arc::make_mut(parcel);
            if let Some(new_name) = name {
                m.name = new_name.to_string();
            }
            if let Some(new_desc) = description {
                m.desc = new_desc.to_string();
            }
            if let Some(new_flags) = flags {
                m.flags = new_flags;
            }
        }

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "name": name,
            "description": description,
            "flags": flags,
        })))
    }

    fn handle_mcp_parcel_access_list_modify(
        &mut self,
        action: &str,
        list: &str,
        avatar_uuid_str: &str,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let target = Uuid::parse_str(avatar_uuid_str.trim())
            .map_err(|e| McpError::InvalidParams(format!("invalid avatar UUID: {e}")))?;

        let local_id = self.world.parcel.as_ref().map(|p| p.local_id).unwrap_or(1);
        let flags = if list.eq_ignore_ascii_case("ban") || list.eq_ignore_ascii_case("blacklist") {
            AL_BAN
        } else {
            AL_ACCESS
        };

        let entries = if action.eq_ignore_ascii_case("add") {
            vec![AccessEntry { id: target, time: 0, flags: 0 }]
        } else {
            Vec::new()
        };

        self.send(NetCommand::Land(LandCommand::AccessListUpdate {
            handle,
            local_id,
            flags,
            entries,
        }));

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "action": action,
            "list": if flags == AL_BAN { "ban" } else { "allow" },
            "avatar_uuid": target.to_string(),
        })))
    }

    fn handle_mcp_parcel_eject(
        &mut self,
        target_uuid_str: &str,
        ban: Option<bool>,
    ) -> Result<ViewerResponse, McpError> {
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let target = Uuid::parse_str(target_uuid_str.trim())
            .map_err(|e| McpError::InvalidParams(format!("invalid resident UUID: {e}")))?;

        let ban_flag = ban.unwrap_or(false);
        self.send(NetCommand::Land(LandCommand::EjectUser {
            handle,
            target,
            ban: ban_flag,
        }));

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "target_uuid": target.to_string(),
            "banned": ban_flag,
            "status": "eject_command_dispatched",
        })))
    }

    fn handle_mcp_economy_pay_avatar(
        &mut self,
        target_uuid_str: &str,
        amount: i32,
        message: Option<String>,
    ) -> Result<ViewerResponse, McpError> {
        if amount <= 0 {
            return Err(McpError::InvalidParams("amount must be greater than 0".into()));
        }
        let balance = self.world.balance.unwrap_or(0);
        if balance < amount {
            return Err(McpError::InvalidParams(format!(
                "insufficient funds: balance is L${balance}, required L${amount}"
            )));
        }
        let target = Uuid::parse_str(target_uuid_str.trim())
            .map_err(|e| McpError::InvalidParams(format!("invalid avatar UUID: {e}")))?;

        let msg = message.unwrap_or_default();
        self.send(NetCommand::PayResident {
            to: target,
            group: false,
            amount,
            message: msg.clone(),
        });

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "target_uuid": target.to_string(),
            "amount": amount,
            "message": msg,
            "balance_before": balance,
            "status": "payment_dispatched",
        })))
    }

    fn handle_mcp_economy_pay_object(
        &mut self,
        target_local_id: u32,
        amount: i32,
        description: Option<String>,
    ) -> Result<ViewerResponse, McpError> {
        if amount <= 0 {
            return Err(McpError::InvalidParams("amount must be greater than 0".into()));
        }
        let balance = self.world.balance.unwrap_or(0);
        if balance < amount {
            return Err(McpError::InvalidParams(format!(
                "insufficient funds: balance is L${balance}, required L${amount}"
            )));
        }
        let Some(handle) = self.world.main_region else {
            return Err(McpError::NotInWorld);
        };
        let (_idx, target_uuid) = self.resolve_prim_by_local_id(target_local_id)?;

        let desc = description.unwrap_or_else(|| format!("Object {target_local_id}"));
        self.send(NetCommand::PayObject {
            handle,
            object: target_uuid,
            amount,
            description: desc.clone(),
        });

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "target_local_id": target_local_id,
            "target_uuid": target_uuid.to_string(),
            "amount": amount,
            "description": desc,
            "balance_before": balance,
            "status": "payment_dispatched",
        })))
    }

    fn handle_mcp_groups_list(&self) -> Result<ViewerResponse, McpError> {
        let active_id = self.world.groups.active;
        let active_title = self.world.groups.active_title.clone();
        let groups: Vec<Value> = self
            .world
            .groups
            .groups
            .iter()
            .map(|g| {
                json!({
                    "group_id": g.id.to_string(),
                    "name": g.name.clone(),
                    "insignia": g.insignia.to_string(),
                    "powers": g.powers,
                    "accept_notices": g.accept_notices,
                    "list_in_profile": g.list_in_profile,
                    "contribution": g.contribution,
                    "is_active": g.id == active_id,
                })
            })
            .collect();

        Ok(ViewerResponse::Json(json!({
            "active_group_id": active_id.to_string(),
            "active_group_title": active_title,
            "count": groups.len(),
            "groups": groups,
        })))
    }

    fn handle_mcp_groups_activate(&mut self, group_uuid_str: &str) -> Result<ViewerResponse, McpError> {
        let group_id = Uuid::parse_str(group_uuid_str.trim())
            .map_err(|e| McpError::InvalidParams(format!("invalid group UUID: {e}")))?;

        if group_id != Uuid::nil() && !self.world.groups.groups.iter().any(|g| g.id == group_id) {
            return Err(McpError::InvalidParams(format!(
                "avatar is not a member of group '{group_id}'"
            )));
        }

        self.send(NetCommand::ActivateGroup(group_id));
        self.world.groups.active = group_id;

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "active_group_id": group_id.to_string(),
            "status": "group_activated",
        })))
    }

    fn handle_mcp_friends_list(&self, online_only: Option<bool>) -> Result<ViewerResponse, McpError> {
        use crate::world::social::rights;

        let filter_online = online_only.unwrap_or(false);
        let mut friends_vec = Vec::new();

        for friend in &self.world.social.friends {
            if filter_online && !friend.online {
                continue;
            }
            let name = self
                .world
                .social
                .names
                .get(&friend.id)
                .cloned()
                .or_else(|| self.world.social.avatar_names.complete(&friend.id))
                .unwrap_or_else(|| friend.id.to_string());

            friends_vec.push(json!({
                "avatar_uuid": friend.id.to_string(),
                "name": name,
                "online": friend.online,
                "rights_given": friend.rights_given,
                "rights_has": friend.rights_has,
                "can_see_online": (friend.rights_has & rights::ONLINE_STATUS) != 0,
                "can_see_on_map": (friend.rights_has & rights::MAP_LOCATION) != 0,
                "can_modify_objects": (friend.rights_has & rights::MODIFY_OBJECTS) != 0,
            }));
        }

        Ok(ViewerResponse::Json(json!({
            "count": friends_vec.len(),
            "online_only": filter_online,
            "friends": friends_vec,
        })))
    }

    // ---- Phase 10: Environment / EEP Suite ----

    fn handle_mcp_environment_get(&mut self) -> Result<ViewerResponse, McpError> {
        let (sky, water) = self.world.environment_frames(&mut self.panels.time_of_day);
        let state = environment_state_json(
            self.panels.time_of_day,
            self.world.eep.has_local(),
            self.world.eep.rlva_locked,
            &sky,
            &water,
        );
        Ok(ViewerResponse::Json(state))
    }

    fn handle_mcp_environment_set_time(
        &mut self,
        preset_str: Option<&str>,
        time_of_day_val: Option<f32>,
    ) -> Result<ViewerResponse, McpError> {
        if self.world.eep.rlva_locked {
            return Err(McpError::InvalidParams(
                "Environment is locked by RLVa @setenv".into(),
            ));
        }

        let target_preset: u8 = if let Some(p) = preset_str {
            match p.trim().to_ascii_lowercase().as_str() {
                "shared" | "default" | "region" | "clear" | "reset" | "0" => 0,
                "sunrise" | "dawn" | "1" => 1,
                "midday" | "noon" | "2" => 2,
                "sunset" | "dusk" | "3" => 3,
                "midnight" | "night" | "4" => 4,
                "midday_legacy" | "legacy_midday" | "5" => 5,
                other => {
                    return Err(McpError::InvalidParams(format!(
                        "unknown time_of_day preset '{other}'. Valid: shared, sunrise, midday, sunset, midnight, midday_legacy"
                    )));
                }
            }
        } else if let Some(t) = time_of_day_val {
            if t <= 0.0 {
                0
            } else if t <= 5.0 && t.fract() == 0.0 {
                t as u8
            } else if t <= 24.0 {
                if (5.0..=8.5).contains(&t) {
                    1
                } else if (8.5..=15.5).contains(&t) {
                    2
                } else if (15.5..=20.5).contains(&t) {
                    3
                } else {
                    4
                }
            } else {
                return Err(McpError::InvalidParams(format!(
                    "invalid time_of_day value: {t}. Expected preset index (0-5) or 24-hour time (0.0-24.0)"
                )));
            }
        } else {
            return Err(McpError::InvalidParams(
                "either 'preset' string or 'time_of_day' numeric value must be provided".into(),
            ));
        };

        let previous_time_of_day = self.panels.time_of_day;

        if target_preset == 0 {
            self.world.eep.clear_local();
        } else {
            self.world.eep.set_time_of_day(target_preset);
        }
        self.panels.time_of_day = target_preset;
        self.settings.time_of_day = target_preset;

        let (sky, water) = self.world.environment_frames(&mut self.panels.time_of_day);
        let state = environment_state_json(
            self.panels.time_of_day,
            self.world.eep.has_local(),
            self.world.eep.rlva_locked,
            &sky,
            &water,
        );

        Ok(ViewerResponse::EnvironmentMutated {
            previous_time_of_day,
            state,
        })
    }

    fn handle_mcp_environment_apply_preset(
        &mut self,
        preset_str: Option<&str>,
        sky_uuid_str: Option<&str>,
        water_uuid_str: Option<&str>,
    ) -> Result<ViewerResponse, McpError> {
        use crate::world::env_select::SettingsKind;

        if self.world.eep.rlva_locked {
            return Err(McpError::InvalidParams(
                "Environment is locked by RLVa @setenv".into(),
            ));
        }

        let previous_time_of_day = self.panels.time_of_day;

        if let Some(preset) = preset_str {
            match preset.trim().to_ascii_lowercase().as_str() {
                "shared" | "default" | "clear" | "reset" | "0" => {
                    self.world.eep.clear_local();
                    self.panels.time_of_day = 0;
                    self.settings.time_of_day = 0;
                }
                "sunrise" | "dawn" | "1" => {
                    self.world.eep.set_time_of_day(1);
                    self.panels.time_of_day = 1;
                    self.settings.time_of_day = 1;
                }
                "midday" | "noon" | "2" => {
                    self.world.eep.set_time_of_day(2);
                    self.panels.time_of_day = 2;
                    self.settings.time_of_day = 2;
                }
                "sunset" | "dusk" | "3" => {
                    self.world.eep.set_time_of_day(3);
                    self.panels.time_of_day = 3;
                    self.settings.time_of_day = 3;
                }
                "midnight" | "night" | "4" => {
                    self.world.eep.set_time_of_day(4);
                    self.panels.time_of_day = 4;
                    self.settings.time_of_day = 4;
                }
                "midday_legacy" | "legacy_midday" | "5" => {
                    self.world.eep.set_time_of_day(5);
                    self.panels.time_of_day = 5;
                    self.settings.time_of_day = 5;
                }
                other => {
                    return Err(McpError::InvalidParams(format!(
                        "unknown environment preset '{other}'"
                    )));
                }
            }
        }

        if let Some(sky_uuid) = sky_uuid_str {
            let sky_id = Uuid::parse_str(sky_uuid.trim())
                .map_err(|e| McpError::InvalidParams(format!("invalid sky UUID: {e}")))?;
            self.world.eep.request_local(sky_id, SettingsKind::Sky);
        }

        if let Some(water_uuid) = water_uuid_str {
            let water_id = Uuid::parse_str(water_uuid.trim())
                .map_err(|e| McpError::InvalidParams(format!("invalid water UUID: {e}")))?;
            self.world.eep.request_local(water_id, SettingsKind::Water);
        }

        let (sky, water) = self.world.environment_frames(&mut self.panels.time_of_day);
        let state = environment_state_json(
            self.panels.time_of_day,
            self.world.eep.has_local(),
            self.world.eep.rlva_locked,
            &sky,
            &water,
        );

        Ok(ViewerResponse::EnvironmentMutated {
            previous_time_of_day,
            state,
        })
    }

    // ---- Phase 11: Spatial Voice Suite ----

    fn handle_mcp_voice_status(&self) -> Result<ViewerResponse, McpError> {
        let light_str = match self.voice.light {
            crate::voice::VoiceLight::Off => "Off",
            crate::voice::VoiceLight::Connecting => "Connecting",
            crate::voice::VoiceLight::Connected => "Connected",
            crate::voice::VoiceLight::Down => "Down",
        };
        let ear_str = match self.settings.audio.voice_ear {
            crate::settings::VoiceEar::Avatar => "Avatar",
            crate::settings::VoiceEar::Camera => "Camera",
        };
        let is_transmitting = self.talking();
        let in_call = self.world.calls.active.is_some();
        let channel_type = if in_call { "Call" } else { "Spatial" };

        Ok(ViewerResponse::Json(json!({
            "voice_enabled": self.settings.audio.voice_enabled,
            "connected": self.voice.connected,
            "status": self.voice.status,
            "light": light_str,
            "channel_type": channel_type,
            "ear": ear_str,
            "own_speaking": self.voice.own_level > aurora_voice::SPEAKING_AUDIO_LEVEL,
            "own_level": self.voice.own_level,
            "mic_level": self.voice.mic_level,
            "mic_meter": self.voice.mic_meter,
            "is_transmitting": is_transmitting,
            "in_call": in_call,
        })))
    }

    fn handle_mcp_voice_participants_list(&self) -> Result<ViewerResponse, McpError> {
        let participants = self.voice.participants();
        let mut list = Vec::new();

        for p in participants {
            let name = self
                .world
                .social
                .names
                .get(&p.agent_id)
                .cloned()
                .or_else(|| self.world.social.avatar_names.complete(&p.agent_id))
                .unwrap_or_else(|| p.agent_id.to_string());

            let user_volume = self.voice.user_volume(&p.agent_id);
            let user_muted = self.voice.is_muted(&p.agent_id);

            list.push(json!({
                "avatar_uuid": p.agent_id.to_string(),
                "name": name,
                "speaking": p.speaking,
                "level": p.level,
                "moderator_muted": p.moderator_muted,
                "user_muted": user_muted,
                "user_volume": user_volume,
            }));
        }

        Ok(ViewerResponse::Json(json!({
            "count": list.len(),
            "connected": self.voice.connected,
            "participants": list,
        })))
    }

    fn handle_mcp_voice_set_user_volume(
        &mut self,
        avatar_uuid_str: &str,
        volume: f32,
    ) -> Result<ViewerResponse, McpError> {
        let id = Uuid::parse_str(avatar_uuid_str.trim())
            .map_err(|e| McpError::InvalidParams(format!("invalid avatar UUID: {e}")))?;

        self.voice.set_user_volume(id, volume);

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "avatar_uuid": id.to_string(),
            "volume": volume,
            "status": "volume_updated",
        })))
    }

    fn handle_mcp_voice_set_user_mute(
        &mut self,
        avatar_uuid_str: &str,
        mute: bool,
    ) -> Result<ViewerResponse, McpError> {
        let id = Uuid::parse_str(avatar_uuid_str.trim())
            .map_err(|e| McpError::InvalidParams(format!("invalid avatar UUID: {e}")))?;

        self.voice.set_user_mute(id, mute);

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "avatar_uuid": id.to_string(),
            "muted": mute,
            "status": if mute { "user_muted" } else { "user_unmuted" },
        })))
    }

    fn handle_mcp_voice_transmit(&mut self, transmitting: bool) -> Result<ViewerResponse, McpError> {
        self.voice.set_transmit(transmitting);

        Ok(ViewerResponse::Json(json!({
            "ok": true,
            "transmitting": transmitting,
            "status": if transmitting { "transmitting_started" } else { "transmitting_stopped" },
        })))
    }
}

fn environment_state_json(
    time_of_day: u8,
    has_local: bool,
    rlva_locked: bool,
    sky: &crate::world::eep::SkyFrame,
    water: &crate::world::eep::WaterFrame,
) -> Value {
    let preset_name = match time_of_day {
        0 => "shared",
        1 => "sunrise",
        2 => "midday",
        3 => "sunset",
        4 => "midnight",
        5 => "midday_legacy",
        _ => "custom",
    };
    json!({
        "time_of_day_preset": time_of_day,
        "preset_name": preset_name,
        "has_local_override": has_local,
        "rlva_locked": rlva_locked,
        "sky": {
            "sun_direction": [sky.sun_direction().x, sky.sun_direction().y, sky.sun_direction().z],
            "moon_direction": [sky.moon_direction().x, sky.moon_direction().y, sky.moon_direction().z],
            "sunlight": [sky.sunlight.x, sky.sunlight.y, sky.sunlight.z],
            "ambient": [sky.ambient.x, sky.ambient.y, sky.ambient.z],
            "blue_horizon": [sky.blue_horizon.x, sky.blue_horizon.y, sky.blue_horizon.z],
            "blue_density": [sky.blue_density.x, sky.blue_density.y, sky.blue_density.z],
            "haze_density": sky.haze_density,
            "haze_horizon": sky.haze_horizon,
            "cloud_color": [sky.cloud_color.x, sky.cloud_color.y, sky.cloud_color.z],
            "cloud_scale": sky.cloud_scale,
            "cloud_shadow": sky.cloud_shadow,
            "gamma": sky.gamma,
        },
        "water": {
            "fog_color": [water.fog_color.x, water.fog_color.y, water.fog_color.z],
            "fog_density": water.fog_density,
            "underwater_fog_mod": water.underwater_fog_mod,
            "fresnel_scale": water.fresnel_scale,
            "fresnel_offset": water.fresnel_offset,
            "blur_multiplier": water.blur_multiplier,
            "normal_scale": [water.normal_scale.x, water.normal_scale.y, water.normal_scale.z],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cof_mutation_queue_serializes_requests() {
        let mut q = CofMutationQueue::default();
        let (tx1, _rx1) = oneshot::channel();
        let (tx2, _rx2) = oneshot::channel();
        q.enqueue(
            AppearanceOperation::Attach {
                item_id: Uuid::new_v4(),
                attach_point: None,
                replace_existing: false,
            },
            tx1,
        );
        q.enqueue(
            AppearanceOperation::WearFolder {
                folder_id: Uuid::new_v4(),
                append: false,
            },
            tx2,
        );
        assert_eq!(q.pending_len(), 2);
        assert!(!q.is_busy());
    }

    #[test]
    fn build_transaction_manager_matches_spatial_candidate() {
        let mut mgr = BuildTransactionManager::default();
        let (tx, _rx) = oneshot::channel();
        let mut known = HashSet::new();
        known.insert(10);

        mgr.register_rez(
            [128.0, 128.0, 25.0],
            Some("MCP Cube".into()),
            known,
            tx,
        );

        // Known local_id 10 should not match even at the exact position
        assert!(
            mgr.try_match_candidate(10, Uuid::new_v4(), [128.0, 128.0, 25.0])
                .is_none()
        );
        // Far away new prim should not match
        assert!(
            mgr.try_match_candidate(11, Uuid::new_v4(), [135.0, 128.0, 25.0])
                .is_none()
        );
        // Nearby new prim within 0.5m should match
        let matched = mgr.try_match_candidate(12, Uuid::new_v4(), [128.1, 127.9, 25.0]);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().0.as_deref(), Some("MCP Cube"));
    }

    #[test]
    fn animation_uuid_resolution_and_defaults() {
        let wave = uuid::uuid!("25680455-83e9-74e3-3b48-18e3845ff8f2");
        let stand = crate::own_motion::ANIM_STAND;
        let walk = crate::own_motion::ANIM_WALK;
        assert_eq!(walk, uuid::uuid!("6ed24bd8-91aa-4b12-ccc7-c97c857ab4e0"));
        assert_eq!(stand, uuid::uuid!("2408fe9e-df1d-1d7d-f4ff-1384fa7b350f"));
        assert_ne!(wave, stand);
    }
}
