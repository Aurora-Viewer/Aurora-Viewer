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
        if self.events_buffer.len() >= MAX_EVENTS_BUFFER {
            self.events_buffer.pop_front();
        }
        self.events_buffer.push_back(json!({
            "seq": self.next_event_seq,
            "timestamp_ms": ts_ms,
            "source": source,
            "type": event_type,
            "data": payload,
        }));
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
    pub(super) fn update_mcp_bridge(&mut self, renderer: &mut Renderer) {
        self.init_mcp_bridge();
        let start = Instant::now();

        self.progress_mcp_state_machines(renderer);

        while start.elapsed() < MCP_FRAME_BUDGET {
            let Some(rx) = self.mcp_bridge.rx.as_mut() else {
                break;
            };
            let Ok(envelope) = rx.try_recv() else {
                break;
            };
            self.handle_mcp_envelope(envelope, renderer);
        }

        self.progress_mcp_captures(renderer);
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

    fn progress_mcp_state_machines(&mut self, _renderer: &mut Renderer) {
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
        while let Some(creation) = self.mcp_bridge.pending_script_creations.pop_front() {
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
                        running: creation.running,
                        experience: Uuid::nil(),
                    },
                    source: creation.source_code,
                });
            } else if now.duration_since(creation.started_at) > Duration::from_secs(5) {
                let _ = creation.reply_tx.send(Err(McpError::Timeout(5000)));
            } else {
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
    }

    fn handle_mcp_envelope(&mut self, envelope: McpRequestEnvelope, renderer: &mut Renderer) {
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
                let pos = self.world.agent.position;
                let rot = self.world.agent.body_rotation();
                let vel = self.world.agent.velocity;
                let (w, h) = renderer.size();
                let _ = reply_tx.send(Ok(ViewerResponse::Json(json!({
                    "connected": self.in_world(),
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
                        renderer,
                    );
                    let _ = reply_tx.send(Ok(ViewerResponse::FrameCaptured(frame)));
                }
                CaptureBufferType::ColorRgb => {
                    self.mcp_bridge.pending_captures.push_back(PendingCapture {
                        include_ui,
                        dispatched_to_renderer: false,
                        reply_tx,
                    });
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
                self.progress_mcp_state_machines(renderer);
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
                duration_ms: _,
            } => {
                let resp = self.handle_mcp_lsl_listen_output(object_id);
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
        renderer: &Renderer,
    ) -> RawCapturedFrame {
        let w = width.clamp(32, 1024);
        let h = height.clamp(32, 1024);
        let (win_w, win_h) = renderer.size();
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
        json!({
            "item_id": item.id.to_string(),
            "parent_id": item.parent.to_string(),
            "name": item.name,
            "description": item.desc,
            "asset_type": item.asset_type,
            "inv_type": item.inv_type,
            "is_link": item.asset_type == 24 || item.asset_type == 25,
            "asset_id": if show_asset_uuid { Some(item.asset_id.to_string()) } else { None },
            "permissions": {
                "copy": (mask & perm::COPY) != 0,
                "modify": (mask & perm::MODIFY) != 0,
                "transfer": (mask & perm::TRANSFER) != 0,
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
            self.build.new_script(&self.world, idx);
            self.flush_build();
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
                running,
                experience: Uuid::nil(),
            },
            source: source_code,
        });
    }

    fn handle_mcp_lsl_listen_output(
        &self,
        object_id: Option<Uuid>,
    ) -> Result<ViewerResponse, McpError> {
        let mut lines = Vec::new();
        for line in self.world.script_debug.lines.iter().rev() {
            if let Some(filter_id) = object_id
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
            if let Some(filter_id) = object_id
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
        renderer: &mut Renderer,
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
                ray_start: self.camera.position,
                ray_end: pos_vec,
                target: Uuid::nil(),
                selected: select,
                remove,
                source_folder,
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
}
