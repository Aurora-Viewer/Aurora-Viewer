//! MCP Tool Registry, Profile Router (`compact` vs `all`), and Request Dispatcher.

use crate::asset_inspect;
use crate::bridge::{
    AppearanceOperation, BoneTransform, CameraCommand, CaptureBufferType, ChatKind, LsdOperation,
    MAX_BOUNDED_ITEMS, McpError, McpRequestEnvelope, PrimTransformSpec, RollbackAction,
    ViewerRequest, ViewerResponse,
};
use crate::lease::{DEFAULT_LEASE_SECONDS, LeaseManager};
use crate::lsd;
use crate::vision;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpProfile {
    Compact,
    All,
}

impl McpProfile {
    pub fn from_env() -> Self {
        match std::env::var("AURORA_MCP_PROFILE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "compact" => McpProfile::Compact,
            _ => McpProfile::All,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(skip)]
    pub in_compact_profile: bool,
}

pub struct McpRouter {
    tx: mpsc::Sender<McpRequestEnvelope>,
    pub leases: Arc<LeaseManager>,
    pub profile: McpProfile,
    manifests: Mutex<HashMap<String, Value>>,
}

impl McpRouter {
    pub fn new(tx: mpsc::Sender<McpRequestEnvelope>, profile: McpProfile) -> Self {
        Self {
            tx,
            leases: Arc::new(LeaseManager::default()),
            profile,
            manifests: Mutex::new(HashMap::new()),
        }
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        let all = all_tool_definitions();
        match self.profile {
            McpProfile::All => all,
            McpProfile::Compact => all.into_iter().filter(|t| t.in_compact_profile).collect(),
        }
    }

    async fn send_to_viewer(
        &self,
        lease_id: Option<String>,
        request: ViewerRequest,
        timeout_ms: u64,
    ) -> Result<ViewerResponse, McpError> {
        // Flush any expired lease rollbacks first.
        if let Some((_, expired_actions)) = self.leases.take_expired_rollbacks()
            && !expired_actions.is_empty()
        {
            let (rtx, _rrx) = oneshot::channel();
            let _ = self.tx.try_send(McpRequestEnvelope {
                lease_id: None,
                request: ViewerRequest::ApplyRollbacks {
                    actions: expired_actions,
                },
                reply_tx: rtx,
            });
        }

        let effective_lease = if request.requires_lease() {
            Some(self.leases.verify_for_mutation(lease_id.as_deref())?)
        } else {
            lease_id
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(McpRequestEnvelope {
                lease_id: effective_lease,
                request,
                reply_tx,
            })
            .await
            .map_err(|_| McpError::ViewerDisconnected)?;

        match tokio::time::timeout(Duration::from_millis(timeout_ms), reply_rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(McpError::ViewerDisconnected),
            Err(_) => Err(McpError::Timeout(timeout_ms)),
        }
    }

    /// Execute an MCP `tools/call` invocation and return the MCP CallToolResult JSON.
    pub async fn call_tool(&self, name: &str, args: Value) -> Value {
        match self.dispatch_tool(name, &args).await {
            Ok(content_blocks) => json!({
                "content": content_blocks,
                "isError": false,
            }),
            Err(err) => json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&json!({
                        "ok": false,
                        "error": err.to_string(),
                    })).unwrap_or_else(|_| err.to_string()),
                }],
                "isError": true,
            }),
        }
    }

    async fn dispatch_tool(&self, name: &str, args: &Value) -> Result<Vec<Value>, McpError> {
        let lease_id = args
            .get("lease_id")
            .and_then(Value::as_str)
            .map(str::to_owned);

        match name {
            // ---- Phase 0: Control Leases, Status & Dynamic API Inspection ----
            "control_acquire" | "viewer_control_acquire" => {
                let owner = args
                    .get("owner")
                    .or_else(|| args.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or("mcp-agent");
                let ttl = args
                    .get("ttl_seconds")
                    .or_else(|| args.get("seconds"))
                    .and_then(Value::as_u64)
                    .unwrap_or(DEFAULT_LEASE_SECONDS);
                let (snap, expired_rollbacks) =
                    self.leases.acquire(owner, ttl, lease_id.as_deref())?;
                if !expired_rollbacks.is_empty() {
                    let _ = self
                        .send_to_viewer(
                            None,
                            ViewerRequest::ApplyRollbacks {
                                actions: expired_rollbacks,
                            },
                            5000,
                        )
                        .await;
                }
                Ok(text_block(json!({
                    "ok": true,
                    "lease": snap,
                })))
            }

            "control_release" | "viewer_control_release" => {
                let (released, rollbacks) = self.leases.release(lease_id.as_deref())?;
                let restored_count = rollbacks.len();
                if !rollbacks.is_empty() {
                    let _ = self
                        .send_to_viewer(
                            None,
                            ViewerRequest::ApplyRollbacks { actions: rollbacks },
                            5000,
                        )
                        .await;
                }
                Ok(text_block(json!({
                    "ok": true,
                    "released": released,
                    "restored_rollbacks": restored_count,
                })))
            }

            "connection_status" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::ConnectionStatus, 5000)
                    .await?;
                let mut payload = into_json(resp);
                if let Value::Object(map) = &mut payload {
                    map.insert(
                        "active_lease".into(),
                        serde_json::to_value(self.leases.status()).unwrap_or(Value::Null),
                    );
                    map.insert(
                        "profile".into(),
                        json!(match self.profile {
                            McpProfile::Compact => "compact",
                            McpProfile::All => "all",
                        }),
                    );
                }
                Ok(text_block(payload))
            }

            "capabilities_refresh" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::CapabilitiesRefresh, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "viewer_api_inspect" => {
                let api = args
                    .get("api")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let operation = args.get("operation").and_then(Value::as_str);
                Ok(text_block(inspect_viewer_api(api, operation)))
            }

            "viewer_call" => {
                let api = req_str(args, "api")?;
                let operation = req_str(args, "operation")?;
                let call_args = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let timeout_s = args
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .unwrap_or(15)
                    .clamp(1, 60);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ViewerCall {
                            api,
                            operation,
                            arguments: call_args,
                        },
                        timeout_s * 1000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "events_subscribe" => {
                let source = req_str(args, "source")?;
                let resp = self
                    .send_to_viewer(None, ViewerRequest::EventsSubscribe { source }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "events_unsubscribe" => {
                let source = req_str(args, "source")?;
                let resp = self
                    .send_to_viewer(None, ViewerRequest::EventsUnsubscribe { source }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "events_read" => {
                let after = args.get("after").and_then(Value::as_u64).unwrap_or(0);
                let resp = self
                    .send_to_viewer(None, ViewerRequest::EventsRead { after }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 1: Agent Status, Radar, Comms, Navigation & RLVa ----
            "get_agent_status" | "avatar_position" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::GetAgentStatus, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "radar_scan_avatars" => {
                let max_radius_m = args
                    .get("max_radius_m")
                    .or_else(|| args.get("distance"))
                    .and_then(Value::as_f64)
                    .unwrap_or(96.0) as f32;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(32)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::RadarScanAvatars {
                            max_radius_m,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "radar_scan_objects" | "world_objects" => {
                let max_radius_m = args
                    .get("max_radius_m")
                    .or_else(|| args.get("distance"))
                    .and_then(Value::as_f64)
                    .unwrap_or(32.0) as f32;
                let root_only = args
                    .get("root_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let name_filter = args
                    .get("name_filter")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::RadarScanObjects {
                            max_radius_m,
                            root_only,
                            name_filter,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "comms_read_history" => {
                let channel_filter = args
                    .get("channel_filter")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, 200) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::CommsReadHistory {
                            channel_filter,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "comms_send" => {
                let message = req_str(args, "message")?;
                let channel = args.get("channel").and_then(Value::as_i64).unwrap_or(0) as i32;
                let chat_type = match args
                    .get("chat_type")
                    .and_then(Value::as_str)
                    .unwrap_or("normal")
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "whisper" => ChatKind::Whisper,
                    "shout" => ChatKind::Shout,
                    _ => ChatKind::Normal,
                };
                let target_uuid = parse_opt_uuid(args, "target_uuid")?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::CommsSend {
                            message,
                            channel,
                            chat_type,
                            target_uuid,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_walk_to" => {
                let pos = parse_vec3(args, "global_position")
                    .or_else(|_| parse_vec3(args, "position"))?;
                let allow_flying = args
                    .get("allow_flying")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let stop_distance = args
                    .get("stop_distance")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.5) as f32;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::NavWalkTo {
                            position: pos,
                            allow_flying,
                            stop_distance,
                            timeout_ms: 0,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_movement_status" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::NavMovementStatus, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_stop" => {
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::NavStop, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "nav_move_and_teleport" => {
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("walk_to");
                let req = match action {
                    "walk_to" => {
                        let position = parse_vec3_or_xyz(args)?;
                        let allow_flying = args
                            .get("allow_flying")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let stop_distance = args
                            .get("stop_distance")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.75) as f32;
                        let timeout_ms = args
                            .get("timeout_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        ViewerRequest::NavWalkTo {
                            position,
                            allow_flying,
                            stop_distance,
                            timeout_ms,
                        }
                    }
                    "sit" | "sit_on_object" => ViewerRequest::NavSit {
                        target: args
                            .get("target_id")
                            .or_else(|| args.get("target"))
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    },
                    "stand" | "stand_up" => ViewerRequest::NavStand,
                    "stop" => ViewerRequest::NavStop,
                    "teleport" | "teleport_to" => {
                        let position = parse_vec3_or_xyz(args).unwrap_or([128.0, 128.0, 25.0]);
                        let region_name = args
                            .get("region_name")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        ViewerRequest::NavTeleport {
                            region_name,
                            position,
                        }
                    }
                    other => {
                        return Err(McpError::InvalidParams(format!(
                            "unknown nav_move_and_teleport action '{other}'"
                        )));
                    }
                };
                let timeout = args
                    .get("timeout_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(15000)
                    .clamp(2000, 30000);
                let resp = self.send_to_viewer(lease_id, req, timeout).await?;
                Ok(text_block(into_json(resp)))
            }

            "rlva_execute" => {
                let commands: Vec<String> = args
                    .get("commands")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if commands.is_empty() {
                    return Err(McpError::InvalidParams(
                        "commands array must contain at least one @ command".into(),
                    ));
                }
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::RlvaExecute { commands }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 2: Settings, Camera, Poser & Floaters ----
            "settings_query" => {
                let search = args
                    .get("search")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let modified_only = args
                    .get("modified_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::SettingsQuery {
                            search,
                            modified_only,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "setting_get" => {
                let group = args
                    .get("group")
                    .and_then(Value::as_str)
                    .unwrap_or("Global")
                    .to_owned();
                let key = req_str(args, "key")?;
                let resp = self
                    .send_to_viewer(None, ViewerRequest::SettingGet { group, key }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "setting_set" | "settings_set" => {
                let group = args
                    .get("group")
                    .and_then(Value::as_str)
                    .unwrap_or("Global")
                    .to_owned();
                let key = args
                    .get("key")
                    .or_else(|| args.get("name"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'key' or 'name'".into()))?
                    .to_owned();
                let value = args
                    .get("value")
                    .cloned()
                    .ok_or_else(|| McpError::InvalidParams("missing 'value'".into()))?;
                let rollback_on_lease_end = args
                    .get("rollback_on_lease_end")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::SettingSet {
                            group,
                            key: key.clone(),
                            value,
                        },
                        5000,
                    )
                    .await?;
                match resp {
                    ViewerResponse::SettingMutated {
                        key,
                        previous_value,
                        current_value,
                    } => {
                        if rollback_on_lease_end {
                            self.leases.record_rollback(RollbackAction::RestoreSetting {
                                key: key.clone(),
                                value: previous_value.clone(),
                            });
                        }
                        Ok(text_block(json!({
                            "ok": true,
                            "key": key,
                            "before": previous_value,
                            "after": current_value,
                            "rollback_registered": rollback_on_lease_end,
                        })))
                    }
                    other => Ok(text_block(into_json(other))),
                }
            }

            "camera_set" => {
                let position = parse_vec3(args, "position")?;
                let focus = parse_vec3(args, "focus")?;
                let fov_deg = args.get("fov_deg").and_then(Value::as_f64).map(|v| v as f32);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::CameraControl {
                            command: CameraCommand::ManualTransform {
                                position,
                                focus,
                                fov_deg,
                            },
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(self.handle_camera_response(resp)))
            }

            "camera_release" => {
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::CameraControl {
                            command: CameraCommand::Release,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(self.handle_camera_response(resp)))
            }

            "mesh_preview_camera" => {
                let yaw_deg = args.get("yaw").and_then(Value::as_f64).unwrap_or(30.0) as f32;
                let pitch_deg = args.get("pitch").and_then(Value::as_f64).unwrap_or(15.0) as f32;
                let distance = args.get("distance").and_then(Value::as_f64).unwrap_or(3.0) as f32;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::CameraControl {
                            command: CameraCommand::MeshPreview {
                                yaw_deg,
                                pitch_deg,
                                distance,
                            },
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(self.handle_camera_response(resp)))
            }

            "camera_control" => {
                let mode = args
                    .get("mode")
                    .and_then(Value::as_str)
                    .unwrap_or("follow_avatar");
                let command = match mode {
                    "follow_avatar" => CameraCommand::FollowAvatar,
                    "release" => CameraCommand::Release,
                    "focus_object" => CameraCommand::FocusObject {
                        object_id: req_str(args, "object_id")?,
                        distance: args.get("distance").and_then(Value::as_f64).unwrap_or(4.0)
                            as f32,
                        pitch_deg: args
                            .get("pitch_deg")
                            .or_else(|| args.get("pitch"))
                            .and_then(Value::as_f64)
                            .unwrap_or(15.0) as f32,
                        yaw_deg: args
                            .get("yaw_deg")
                            .or_else(|| args.get("yaw"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32,
                    },
                    "manual_transform" => CameraCommand::ManualTransform {
                        position: parse_vec3(args, "position")?,
                        focus: parse_vec3(args, "look_at")
                            .or_else(|_| parse_vec3(args, "focus"))?,
                        fov_deg: args
                            .get("fov_deg")
                            .or_else(|| args.get("fov"))
                            .and_then(Value::as_f64)
                            .map(|v| v as f32),
                    },
                    "orbit" => CameraCommand::Orbit {
                        delta_yaw_deg: args
                            .get("delta_yaw")
                            .or_else(|| args.get("delta_yaw_deg"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32,
                        delta_pitch_deg: args
                            .get("delta_pitch")
                            .or_else(|| args.get("delta_pitch_deg"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32,
                        zoom_delta: args
                            .get("zoom")
                            .or_else(|| args.get("zoom_delta"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0) as f32,
                    },
                    other => {
                        return Err(McpError::InvalidParams(format!(
                            "unknown camera_control mode '{other}'"
                        )));
                    }
                };
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::CameraControl { command }, 5000)
                    .await?;
                Ok(text_block(self.handle_camera_response(resp)))
            }

            "poser_inspect" => {
                let subject = parse_opt_uuid(args, "subject")?;
                let resp = self
                    .send_to_viewer(None, ViewerRequest::PoserInspect { subject }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "poser_set_bones" => {
                let subject = parse_opt_uuid(args, "subject")?;
                let transforms_val = args
                    .get("transforms")
                    .cloned()
                    .ok_or_else(|| McpError::InvalidParams("missing 'transforms' array".into()))?;
                let transforms: Vec<BoneTransform> = serde_json::from_value(transforms_val)
                    .map_err(|e| McpError::InvalidParams(format!("invalid BoneTransform: {e}")))?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::PoserSetBones {
                            subject,
                            transforms,
                        },
                        5000,
                    )
                    .await?;
                self.leases.record_rollback(RollbackAction::ResetPoser);
                Ok(text_block(into_json(resp)))
            }

            "poser_reset" => {
                let subject = parse_opt_uuid(args, "subject")?;
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::PoserReset { subject }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "floater_list" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::FloaterList, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "floater_open" => {
                let name = req_str(args, "name")?;
                let open = args.get("open").and_then(Value::as_bool).unwrap_or(true);
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::FloaterOpen { name, open }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 3: Viewport Capture ("Vision"), Orbit & Image Diff ----
            "viewport_capture" | "snapshot" => {
                let layer_str = args
                    .get("buffer_type")
                    .or_else(|| args.get("layer"))
                    .and_then(Value::as_str)
                    .unwrap_or("ColorRgb")
                    .to_ascii_lowercase();
                let buffer_type = if layer_str.contains("depth") {
                    CaptureBufferType::DepthNormalized
                } else {
                    CaptureBufferType::ColorRgb
                };
                let include_ui = args
                    .get("include_ui")
                    .or_else(|| args.get("show_ui"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let include_hud = args
                    .get("include_hud")
                    .or_else(|| args.get("show_hud"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let max_w = args
                    .get("width")
                    .or_else(|| args.get("max_width"))
                    .and_then(Value::as_u64)
                    .unwrap_or(1024) as u32;
                let max_h = args
                    .get("height")
                    .or_else(|| args.get("max_height"))
                    .and_then(Value::as_u64)
                    .unwrap_or(1024) as u32;
                let label = args
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("viewport")
                    .to_owned();

                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::ViewportCapture {
                            buffer_type,
                            include_ui,
                            include_hud,
                            max_dimensions: (max_w, max_h),
                        },
                        15_000,
                    )
                    .await?;

                match resp {
                    ViewerResponse::FrameCaptured(raw) => {
                        let encoded = tokio::task::spawn_blocking(move || {
                            vision::encode_captured_frame(raw, (max_w, max_h), &label)
                        })
                        .await
                        .map_err(|e| McpError::Internal(format!("encode join error: {e}")))??;

                        Ok(vec![
                            json!({
                                "type": "image",
                                "data": encoded.base64_png,
                                "mimeType": "image/png",
                            }),
                            json!({
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "ok": true,
                                    "path": encoded.path,
                                    "sha256": encoded.sha256,
                                    "width": encoded.width,
                                    "height": encoded.height,
                                    "buffer_type": encoded.buffer_type,
                                    "near_clip": encoded.near_clip,
                                    "far_clip": encoded.far_clip,
                                })).unwrap_or_default(),
                            }),
                        ])
                    }
                    other => Ok(text_block(into_json(other))),
                }
            }

            "capture_orbit" => {
                let focus = parse_vec3(args, "focus")?;
                let radius = args
                    .get("radius")
                    .and_then(Value::as_f64)
                    .unwrap_or(3.0)
                    .max(0.5) as f32;
                let height_offset = args
                    .get("height_offset")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0) as f32;
                let views = args
                    .get("views")
                    .and_then(Value::as_u64)
                    .unwrap_or(4)
                    .clamp(1, 12) as usize;
                let label = args
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("orbit")
                    .to_owned();
                let evidence_kind = args
                    .get("evidence_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unspecified_viewer_scene")
                    .to_owned();

                let mut captured_views = Vec::with_capacity(views);
                for i in 0..views {
                    let angle = (i as f32) * std::f32::consts::TAU / (views as f32);
                    let pos = [
                        focus[0] + radius * angle.cos(),
                        focus[1] + radius * angle.sin(),
                        focus[2] + height_offset,
                    ];
                    let cam_resp = self
                        .send_to_viewer(
                            lease_id.clone(),
                            ViewerRequest::CameraControl {
                                command: CameraCommand::ManualTransform {
                                    position: pos,
                                    focus,
                                    fov_deg: None,
                                },
                            },
                            5000,
                        )
                        .await?;
                    let _ = self.handle_camera_response(cam_resp);

                    let frame_resp = self
                        .send_to_viewer(
                            None,
                            ViewerRequest::ViewportCapture {
                                buffer_type: CaptureBufferType::ColorRgb,
                                include_ui: false,
                                include_hud: false,
                                max_dimensions: (1024, 1024),
                            },
                            15_000,
                        )
                        .await?;
                    if let ViewerResponse::FrameCaptured(raw) = frame_resp {
                        let view_label = format!("{label}-view-{i}");
                        let encoded = tokio::task::spawn_blocking(move || {
                            vision::encode_captured_frame(raw, (1024, 1024), &view_label)
                        })
                        .await
                        .map_err(|e| McpError::Internal(format!("encode join error: {e}")))??;
                        captured_views.push(json!({
                            "view_index": i,
                            "angle_deg": (angle.to_degrees() * 10.0).round() / 10.0,
                            "camera_position": pos,
                            "focus": focus,
                            "path": encoded.path,
                            "sha256": encoded.sha256,
                            "width": encoded.width,
                            "height": encoded.height,
                        }));
                    }
                }

                // Release temporary orbit camera back to viewer camera.
                let _ = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::CameraControl {
                            command: CameraCommand::Release,
                        },
                        5000,
                    )
                    .await;

                let manifest_id = Uuid::new_v4().to_string();
                let manifest = json!({
                    "manifest_id": manifest_id,
                    "label": label,
                    "evidence_kind": evidence_kind,
                    "focus": focus,
                    "radius": radius,
                    "views": captured_views,
                });
                let manifest_path =
                    vision::capture_output_dir().join(format!("manifest-{manifest_id}.json"));
                let _ = std::fs::write(
                    &manifest_path,
                    serde_json::to_string_pretty(&manifest).unwrap_or_default(),
                );
                self.manifests
                    .lock()
                    .insert(manifest_path.display().to_string(), manifest.clone());
                self.manifests.lock().insert(manifest_id, manifest.clone());

                Ok(text_block(json!({
                    "ok": true,
                    "manifest_path": manifest_path.display().to_string(),
                    "manifest": manifest,
                })))
            }

            "capture_manifest_read" => {
                let key = args
                    .get("manifest_path")
                    .or_else(|| args.get("manifest_id"))
                    .or_else(|| args.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if let Some(m) = self.manifests.lock().get(key).cloned() {
                    return Ok(text_block(m));
                }
                let text = std::fs::read_to_string(key).map_err(|e| {
                    McpError::InvalidParams(format!("cannot read manifest '{key}': {e}"))
                })?;
                let val: Value = serde_json::from_str(&text)
                    .map_err(|e| McpError::InvalidParams(format!("invalid manifest JSON: {e}")))?;
                Ok(text_block(val))
            }

            "image_compare" => {
                let reference = req_str(args, "reference")
                    .or_else(|_| req_str(args, "image_a"))?;
                let observed = req_str(args, "observed")
                    .or_else(|_| req_str(args, "image_b"))?;
                let res = tokio::task::spawn_blocking(move || {
                    vision::compare_images(&reference, &observed)
                })
                .await
                .map_err(|e| McpError::Internal(format!("image_compare join error: {e}")))??;
                Ok(text_block(res))
            }

            // ---- Phase 4: Inventory & Appearance (COF State Machine) ----
            "inventory_browse" => {
                let folder_id_or_path = args
                    .get("folder_id_or_path")
                    .or_else(|| args.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .to_owned();
                let depth = args
                    .get("depth")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
                    .clamp(1, 5) as u8;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::InventoryBrowse {
                            folder_id_or_path,
                            depth,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "inventory_search" => {
                let query = args
                    .get("query")
                    .or_else(|| args.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let folder_id = args
                    .get("folder_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let asset_type = args
                    .get("asset_type")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let worn_only = args
                    .get("worn_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::InventorySearch {
                            folder_id,
                            query,
                            asset_type,
                            worn_only,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "appearance_get_worn" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::AppearanceGetWorn, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "appearance_mutate" => {
                let op_name = args
                    .get("op")
                    .or_else(|| args.get("action"))
                    .and_then(Value::as_str)
                    .unwrap_or("attach");
                let operation = match op_name {
                    "attach" | "wear" => AppearanceOperation::Attach {
                        item_id: parse_req_uuid(args, "item_id")?,
                        attach_point: args
                            .get("attach_point")
                            .and_then(Value::as_u64)
                            .map(|v| v as u8),
                        replace_existing: args
                            .get("replace_existing")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    },
                    "detach" | "take_off" => AppearanceOperation::Detach {
                        item_id: parse_req_uuid(args, "item_id")?,
                    },
                    "wear_folder" | "replace_outfit" => AppearanceOperation::WearFolder {
                        folder_id: parse_req_uuid(args, "folder_id")?,
                        append: args
                            .get("append")
                            .and_then(Value::as_bool)
                            .unwrap_or(op_name == "wear_folder"),
                    },
                    "remove_folder" => AppearanceOperation::RemoveFolder {
                        folder_id: parse_req_uuid(args, "folder_id")?,
                    },
                    other => {
                        return Err(McpError::InvalidParams(format!(
                            "unknown appearance_mutate op '{other}'"
                        )));
                    }
                };
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::AppearanceMutate { operation },
                        15_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 5: World Building, LSL, Linkset Data & Local Mesh ----
            "prim_rez_new" => {
                let position = parse_vec3(args, "position")?;
                let scale = parse_vec3(args, "scale").unwrap_or([0.5, 0.5, 0.5]);
                let rotation_quat = parse_vec4(args, "rotation_quat").unwrap_or([0.0, 0.0, 0.0, 1.0]);
                let pcode = args.get("pcode").and_then(Value::as_u64).unwrap_or(9) as u8;
                let name = args.get("name").and_then(Value::as_str).map(str::to_owned);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::PrimRezNew {
                            position,
                            scale,
                            rotation_quat,
                            pcode,
                            name,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_transform" => {
                let transforms: Vec<PrimTransformSpec> = if let Some(arr) = args.get("transforms") {
                    serde_json::from_value(arr.clone()).map_err(|e| {
                        McpError::InvalidParams(format!("invalid 'transforms' array: {e}"))
                    })?
                } else {
                    let local_id = args
                        .get("local_id")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| {
                            McpError::InvalidParams("missing 'local_id' or 'transforms'".into())
                        })? as u32;
                    vec![PrimTransformSpec {
                        local_id,
                        position: parse_vec3(args, "position").ok(),
                        rotation_quat: parse_vec4(args, "rotation_quat").ok(),
                        rotation_euler_deg: parse_vec3(args, "rotation_euler_deg").ok(),
                        scale: parse_vec3(args, "scale").ok(),
                        name: args.get("name").and_then(Value::as_str).map(str::to_owned),
                        description: args
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    }]
                };
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::PrimTransform { transforms }, 6000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_set_shape" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let path_cut = parse_vec2(args, "path_cut").ok();
                let profile_cut = parse_vec2(args, "profile_cut").ok();
                let hollow = args.get("hollow").and_then(Value::as_f64).map(|v| v as f32);
                let twist = parse_vec2(args, "twist").ok();
                let color = parse_vec4(args, "color").ok();
                let texture_id = parse_opt_uuid(args, "texture_id")?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::PrimSetShape {
                            local_id,
                            path_cut,
                            profile_cut,
                            hollow,
                            twist,
                            color,
                            texture_id,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_link" => {
                let local_ids: Vec<u32> = args
                    .get("local_ids")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_u64)
                            .map(|n| n as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                if local_ids.is_empty() {
                    return Err(McpError::InvalidParams("missing 'local_ids'".into()));
                }
                let delink = args.get("delink").and_then(Value::as_bool).unwrap_or(false);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::PrimLink { local_ids, delink },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_delete" => {
                let local_ids: Vec<u32> = args
                    .get("local_ids")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_u64)
                            .map(|n| n as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::PrimDelete { local_ids }, 6000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsl_inspect_object_scripts" => {
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'object_local_id'".into()))?
                    as u32;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::LslInspectObjectScripts { object_local_id },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsl_read_script" => {
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                let script_item_id = parse_req_uuid(args, "script_item_id")?;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::LslReadScript {
                            object_local_id,
                            script_item_id,
                        },
                        8000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsl_write_and_compile" => {
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32;
                let script_item_id = parse_opt_uuid(args, "script_item_id")?;
                let script_name = args
                    .get("script_name")
                    .and_then(Value::as_str)
                    .unwrap_or("New Script")
                    .to_owned();
                let source_code = req_str(args, "source_code")?;
                let mono = args.get("mono").and_then(Value::as_bool).unwrap_or(true);
                let running = args.get("running").and_then(Value::as_bool).unwrap_or(true);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::LslWriteAndCompile {
                            object_local_id,
                            script_item_id,
                            script_name,
                            source_code,
                            mono,
                            running,
                        },
                        15_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsl_listen_script_output" => {
                let object_id = parse_opt_uuid(args, "object_id")?;
                let local_id = args
                    .get("local_id")
                    .or_else(|| args.get("object_local_id"))
                    .and_then(Value::as_u64)
                    .map(|v| v as u32);
                let duration_ms = args
                    .get("duration_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(500)
                    .clamp(0, 10_000);
                if duration_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                }
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::LslListenScriptOutput {
                            object_id,
                            local_id,
                            duration_ms,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsd_inspect" => {
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'object_local_id'".into()))?
                    as u32;
                let filter_regex = args
                    .get("filter_regex")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let include_protected = args
                    .get("include_protected")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let passphrase = args
                    .get("passphrase")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::LsdInspect {
                            object_local_id,
                            filter_regex,
                            include_protected,
                            passphrase,
                            limit,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsd_mutate" => {
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'object_local_id'".into()))?
                    as u32;
                let operations: Vec<LsdOperation> = serde_json::from_value(
                    args.get("operations").cloned().ok_or_else(|| {
                        McpError::InvalidParams("missing 'operations' array".into())
                    })?,
                )
                .map_err(|e| McpError::InvalidParams(format!("invalid LsdOperation: {e}")))?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::LsdMutate {
                            object_local_id,
                            operations,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "lsd_stress_test" => {
                let payload = args
                    .get("payload")
                    .ok_or_else(|| McpError::InvalidParams("missing 'payload'".into()))?;
                let protected_keys = args
                    .get("protected_keys")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let filter_regex = args.get("filter_regex").and_then(Value::as_str);
                let report = lsd::stress_test_payload(payload, protected_keys, filter_regex)?;
                Ok(text_block(report))
            }

            "asset_inspect" | "asset_inspect_local" => {
                let file_path = req_str(args, "file_path")
                    .or_else(|_| req_str(args, "filename"))?;
                let res = tokio::task::spawn_blocking(move || {
                    asset_inspect::inspect_local_asset(&file_path)
                })
                .await
                .map_err(|e| McpError::Internal(format!("asset_inspect join error: {e}")))??;
                Ok(text_block(res))
            }

            "local_mesh_status" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::LocalMeshStatus, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "local_mesh_open" => {
                let open = args.get("open").and_then(Value::as_bool).unwrap_or(true);
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::LocalMeshOpen { open }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "local_mesh_auto_reload" => {
                let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::LocalMeshAutoReload { enabled },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 6: Suite 1 — Direct Asset Upload, LiveMesh, Inventory Rez & Materials ----
            "livemesh_control" => {
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("status")
                    .to_owned();
                let source_id = parse_opt_uuid(args, "source_id")?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::LiveMeshControl { action, source_id },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "asset_upload" => {
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("status")
                    .to_owned();
                let mut paths: Vec<String> = args
                    .get("paths")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(single) = args
                    .get("path")
                    .or_else(|| args.get("file_path"))
                    .and_then(Value::as_str)
                {
                    paths.push(single.to_owned());
                }
                let choice = args
                    .get("choice")
                    .or_else(|| args.get("asset_type"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let include_textures = args.get("include_textures").and_then(Value::as_bool);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::AssetUpload {
                            action,
                            paths,
                            choice,
                            include_textures,
                        },
                        15_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "inventory_rez_object" => {
                let item_id = parse_req_uuid(args, "item_id")?;
                let position = parse_vec3(args, "position")?;
                let select = args.get("select").and_then(Value::as_bool).unwrap_or(false);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::InventoryRezObject {
                            item_id,
                            position,
                            select,
                        },
                        8000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "object_take_or_derez" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("take")
                    .to_owned();
                let folder_id = parse_opt_uuid(args, "folder_id")?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ObjectTakeOrDerez {
                            local_id,
                            action,
                            folder_id,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_set_material" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let face = args.get("face").and_then(Value::as_i64).map(|v| v as i32);
                let texture_id = parse_opt_uuid(args, "texture_id")?;
                let material_id = parse_opt_uuid(args, "material_id")?;
                let color = parse_vec4(args, "color").ok();
                let repeats = parse_vec2(args, "repeats").ok();
                let offsets = parse_vec2(args, "offsets").ok();
                let rotation_rad = args
                    .get("rotation_rad")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32)
                    .or_else(|| {
                        args.get("rotation_deg")
                            .and_then(Value::as_f64)
                            .map(|d| (d as f32).to_radians())
                    });
                let glow = args.get("glow").and_then(Value::as_f64).map(|v| v as f32);
                let fullbright = args.get("fullbright").and_then(Value::as_bool);
                let alpha_mode = args
                    .get("alpha_mode")
                    .and_then(Value::as_u64)
                    .map(|v| v as u8);
                let bump_shiny = args
                    .get("bump_shiny")
                    .and_then(Value::as_u64)
                    .map(|v| v as u8);
                let resp = self
                    .send_to_viewer(
                        lease_id,
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
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 6: Suite 2 — Remaining 13 Legacy Viewer MCP Tools (43/43 Parity) ----
            "mesh_upload_open" => {
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::MeshUploadOpen, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "mesh_upload_status" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::MeshUploadStatus, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "native_file_dialogs" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::NativeFileDialogs, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "native_file_choose" => {
                let dialog_id = args.get("dialog_id").and_then(Value::as_i64).unwrap_or(1);
                let filename = req_str(args, "filename")?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::NativeFileChoose {
                            dialog_id,
                            filename,
                        },
                        8000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_inspect" => {
                let path = args
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("/main_view")
                    .to_owned();
                let resp = self
                    .send_to_viewer(None, ViewerRequest::UiInspect { path }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_find" => {
                let query = req_str(args, "query")?;
                let under = args
                    .get("under")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let search_in = args
                    .get("search_in")
                    .and_then(Value::as_str)
                    .unwrap_or("path")
                    .to_owned();
                let match_mode = args
                    .get("match")
                    .and_then(Value::as_str)
                    .unwrap_or("contains")
                    .to_owned();
                let max_depth = args
                    .get("max_depth")
                    .and_then(Value::as_u64)
                    .map(|v| v as usize);
                let include_info = args
                    .get("include_info")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(1, MAX_BOUNDED_ITEMS as u64) as usize;
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::UiFind {
                            query,
                            under,
                            search_in,
                            match_mode,
                            max_depth,
                            include_info,
                            offset,
                            limit,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_click" => {
                let path = req_str(args, "path")?;
                let button = args
                    .get("button")
                    .and_then(Value::as_str)
                    .unwrap_or("LEFT")
                    .to_owned();
                let floater = args
                    .get("floater")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let observe_path = args
                    .get("observe_path")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::UiClick {
                            path,
                            button,
                            floater,
                            observe_path,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_get_value" => {
                let path = req_str(args, "path")?;
                let resp = self
                    .send_to_viewer(None, ViewerRequest::UiGetValue { path }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_set_text" => {
                let path = req_str(args, "path")?;
                let text = req_str(args, "text")?;
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::UiSetText { path, text }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_select" => {
                let path = req_str(args, "path")?;
                let value = args
                    .get("value")
                    .cloned()
                    .ok_or_else(|| McpError::InvalidParams("missing 'value'".into()))?;
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::UiSelect { path, value }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_press_key" => {
                let path = req_str(args, "path")?;
                let keysym = req_str(args, "keysym")?;
                let modifiers: Vec<String> = args
                    .get("modifiers")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let observe_path = args
                    .get("observe_path")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::UiPressKey {
                            path,
                            keysym,
                            modifiers,
                            observe_path,
                        },
                        5000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_list_menus" => {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let resp = self
                    .send_to_viewer(None, ViewerRequest::UiListMenus { query }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "ui_invoke_menu" => {
                let name = req_str(args, "name")?;
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::UiInvokeMenu { name }, 5000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 6: Suite 3 — In-World Object Interaction, Notecards & Particles ----
            "object_touch" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let face = args.get("face").and_then(Value::as_i64).unwrap_or(0) as i32;
                let uv = parse_vec2(args, "uv").unwrap_or([0.5, 0.5]);
                let st = parse_vec2(args, "st").unwrap_or([0.5, 0.5]);
                let hold_ms = args
                    .get("hold_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .clamp(0, 5000);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ObjectTouch {
                            local_id,
                            face,
                            uv,
                            st,
                            hold_ms,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_sit" => {
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("sit")
                    .to_owned();
                let target_local_id = args
                    .get("target_local_id")
                    .or_else(|| args.get("local_id"))
                    .and_then(Value::as_u64)
                    .map(|v| v as u32);
                let target_uuid = match parse_opt_uuid(args, "target_uuid")? {
                    Some(u) => Some(u),
                    None => parse_opt_uuid(args, "object_id")?,
                };
                let offset = parse_vec3(args, "offset").ok();
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::AvatarSit {
                            action,
                            target_local_id,
                            target_uuid,
                            offset,
                        },
                        6000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "notecard_read" => {
                let item_id = parse_req_uuid(args, "item_id")?;
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .map(|v| v as u32);
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::NotecardRead {
                            item_id,
                            object_local_id,
                        },
                        8000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "notecard_write" => {
                let item_id = parse_opt_uuid(args, "item_id")?;
                let object_local_id = args
                    .get("object_local_id")
                    .and_then(Value::as_u64)
                    .map(|v| v as u32);
                let name = args.get("name").and_then(Value::as_str).map(str::to_owned);
                let body = req_str(args, "body")?;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::NotecardWrite {
                            item_id,
                            object_local_id,
                            name,
                            body,
                        },
                        8000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_set_particles" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let clear = args.get("clear").and_then(Value::as_bool).unwrap_or(false);
                let pattern = args
                    .get("pattern")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let texture_id = parse_opt_uuid(args, "texture_id")?;
                let start_color = parse_vec4(args, "start_color").ok();
                let end_color = parse_vec4(args, "end_color").ok();
                let start_scale = parse_vec2(args, "start_scale").ok();
                let end_scale = parse_vec2(args, "end_scale").ok();
                let particle_max_age = args
                    .get("particle_max_age")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32);
                let burst_rate = args
                    .get("burst_rate")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32);
                let burst_part_count = args
                    .get("burst_part_count")
                    .and_then(Value::as_u64)
                    .map(|v| v as u32);
                let burst_radius = args
                    .get("burst_radius")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32);
                let burst_speed_min = args
                    .get("burst_speed_min")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32);
                let burst_speed_max = args
                    .get("burst_speed_max")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32);
                let accel = parse_vec3(args, "accel").ok();
                let omega = parse_vec3(args, "omega").ok();
                let follow_src = args.get("follow_src").and_then(Value::as_bool);
                let follow_velocity = args.get("follow_velocity").and_then(Value::as_bool);
                let interp_color = args.get("interp_color").and_then(Value::as_bool);
                let interp_scale = args.get("interp_scale").and_then(Value::as_bool);
                let emissive = args.get("emissive").and_then(Value::as_bool);
                let ribbon = args.get("ribbon").and_then(Value::as_bool);
                let resp = self
                    .send_to_viewer(
                        lease_id,
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
                        },
                        15_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "viewer_login" => {
                let username = args.get("username").and_then(Value::as_str).map(str::to_owned);
                let password = args.get("password").and_then(Value::as_str).map(str::to_owned);
                let location = args.get("location").and_then(Value::as_str).map(str::to_owned);
                let mfa_token = args.get("mfa_token").and_then(Value::as_str).map(str::to_owned);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ViewerLogin {
                            username,
                            password,
                            location,
                            mfa_token,
                        },
                        15_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "viewer_shutdown" => {
                let lease_id = args.get("lease_id").and_then(Value::as_str).map(str::to_owned);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ViewerShutdown,
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 7: Physics Flags, Metadata, Animations & Economy ----
            "prim_set_physics_properties" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let physics = args.get("physics").and_then(Value::as_bool);
                let temporary = args.get("temporary").and_then(Value::as_bool);
                let phantom = args.get("phantom").and_then(Value::as_bool);
                let shape_type = args.get("shape_type").and_then(Value::as_str).map(str::to_owned);
                let density = args.get("density").and_then(Value::as_f64).map(|v| v as f32);
                let friction = args.get("friction").and_then(Value::as_f64).map(|v| v as f32);
                let restitution = args.get("restitution").and_then(Value::as_f64).map(|v| v as f32);
                let gravity_multiplier = args.get("gravity_multiplier").and_then(Value::as_f64).map(|v| v as f32);

                let resp = self
                    .send_to_viewer(
                        lease_id,
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
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "prim_set_metadata" => {
                let local_id = args
                    .get("local_id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'local_id'".into()))?
                    as u32;
                let name = args.get("name").and_then(Value::as_str).map(str::to_owned);
                let description = args.get("description").and_then(Value::as_str).map(str::to_owned);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::PrimSetMetadata {
                            local_id,
                            name,
                            description,
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_animation_play" => {
                let animation = args
                    .get("animation")
                    .or_else(|| args.get("name"))
                    .or_else(|| args.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'animation' (name or UUID)".into()))?
                    .to_string();
                let loop_anim = args.get("loop").and_then(Value::as_bool);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::AvatarAnimationPlay {
                            animation,
                            loop_anim,
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_animation_stop" => {
                let animation = args
                    .get("animation")
                    .or_else(|| args.get("name"))
                    .or_else(|| args.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'animation' (name or UUID)".into()))?
                    .to_string();

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::AvatarAnimationStop {
                            animation,
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "avatar_animations_list" => {
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::AvatarAnimationsList,
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "economy_get_balance" => {
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::EconomyGetBalance,
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 8: Land, Parcel & Estate Suite ----
            "parcel_info" => {
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::ParcelInfoQuery,
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "parcel_set_music" => {
                let music_url = args
                    .get("music_url")
                    .or_else(|| args.get("url"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'music_url'".into()))?
                    .to_string();

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ParcelSetMusic { music_url },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "parcel_set_properties" => {
                let name = args.get("name").and_then(Value::as_str).map(str::to_owned);
                let description = args.get("description").and_then(Value::as_str).map(str::to_owned);
                let flags = args.get("flags").and_then(Value::as_u64).map(|v| v as u32);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ParcelSetProperties {
                            name,
                            description,
                            flags,
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "parcel_access_list_modify" => {
                let action = args
                    .get("action")
                    .and_then(Value::as_str)
                    .unwrap_or("add")
                    .to_string();
                let list = args
                    .get("list")
                    .and_then(Value::as_str)
                    .unwrap_or("ban")
                    .to_string();
                let avatar_uuid = args
                    .get("avatar_uuid")
                    .or_else(|| args.get("target"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'avatar_uuid'".into()))?
                    .to_string();

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ParcelAccessListModify {
                            action,
                            list,
                            avatar_uuid,
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "parcel_eject" => {
                let target_uuid = args
                    .get("target_uuid")
                    .or_else(|| args.get("avatar_uuid"))
                    .or_else(|| args.get("target"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'target_uuid'".into()))?
                    .to_string();
                let ban = args.get("ban").and_then(Value::as_bool);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::ParcelEject { target_uuid, ban },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 9: Economy & Social Suite ----
            "economy_pay_avatar" => {
                let target_uuid = args
                    .get("target_uuid")
                    .or_else(|| args.get("recipient"))
                    .or_else(|| args.get("to"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'target_uuid'".into()))?
                    .to_string();
                let amount = args
                    .get("amount")
                    .or_else(|| args.get("amount_l$"))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'amount'".into()))?
                    as i32;
                if amount <= 0 {
                    return Err(McpError::InvalidParams("'amount' must be greater than 0".into()));
                }
                let message = args.get("message").and_then(Value::as_str).map(str::to_owned);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::EconomyPayAvatar {
                            target_uuid,
                            amount,
                            message,
                        },
                        10_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "economy_pay_object" => {
                let target_local_id = args
                    .get("target_local_id")
                    .or_else(|| args.get("local_id"))
                    .and_then(Value::as_u64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'target_local_id'".into()))?
                    as u32;
                let amount = args
                    .get("amount")
                    .or_else(|| args.get("amount_l$"))
                    .and_then(Value::as_i64)
                    .ok_or_else(|| McpError::InvalidParams("missing 'amount'".into()))?
                    as i32;
                if amount <= 0 {
                    return Err(McpError::InvalidParams("'amount' must be greater than 0".into()));
                }
                let description = args.get("description").and_then(Value::as_str).map(str::to_owned);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::EconomyPayObject {
                            target_local_id,
                            amount,
                            description,
                        },
                        10_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "groups_list" => {
                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::GroupsList,
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "groups_activate" => {
                let group_uuid = args
                    .get("group_uuid")
                    .or_else(|| args.get("group_id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpError::InvalidParams("missing 'group_uuid'".into()))?
                    .to_string();

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::GroupsActivate { group_uuid },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "friends_list" => {
                let online_only = args.get("online_only").and_then(Value::as_bool);

                let resp = self
                    .send_to_viewer(
                        None,
                        ViewerRequest::FriendsList { online_only },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            // ---- Phase 10: Environment & EEP Suite ----
            "environment_get" => {
                let resp = self
                    .send_to_viewer(None, ViewerRequest::EnvironmentGet, 5_000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "environment_set_time" => {
                let preset = args
                    .get("preset")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let time_of_day = args
                    .get("time_of_day")
                    .or_else(|| args.get("hour"))
                    .or_else(|| args.get("time"))
                    .and_then(Value::as_f64)
                    .map(|f| f as f32);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::EnvironmentSetTime {
                            preset,
                            time_of_day,
                        },
                        5_000,
                    )
                    .await?;

                if let ViewerResponse::EnvironmentMutated {
                    previous_time_of_day,
                    ref state,
                } = resp
                {
                    self.leases.record_rollback(RollbackAction::RestoreEnvironment {
                        time_of_day: previous_time_of_day,
                    });
                    return Ok(text_block(state.clone()));
                }
                Ok(text_block(into_json(resp)))
            }

            // Phase 11: Spatial Voice Suite
            "voice_status" => {
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::VoiceStatus, 5_000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "voice_participants_list" => {
                let resp = self
                    .send_to_viewer(lease_id, ViewerRequest::VoiceParticipantsList, 5_000)
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "voice_set_user_volume" => {
                let avatar_uuid = req_str(&args, "avatar_uuid")
                    .or_else(|_| req_str(&args, "target_uuid"))
                    .or_else(|_| req_str(&args, "agent_id"))?;
                let volume = args
                    .get("volume")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        McpError::InvalidParams(
                            "missing required number 'volume' [0.0..1.0]".into(),
                        )
                    })? as f32;
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::VoiceSetUserVolume {
                            avatar_uuid,
                            volume: volume.clamp(0.0, 1.0),
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "voice_set_user_mute" => {
                let avatar_uuid = req_str(&args, "avatar_uuid")
                    .or_else(|_| req_str(&args, "target_uuid"))
                    .or_else(|_| req_str(&args, "agent_id"))?;
                let mute = args
                    .get("mute")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::VoiceSetUserMute {
                            avatar_uuid,
                            mute,
                        },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "voice_transmit" => {
                let transmitting = args
                    .get("transmitting")
                    .or_else(|| args.get("transmit"))
                    .or_else(|| args.get("speaking"))
                    .or_else(|| args.get("on"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::VoiceTransmit { transmitting },
                        5_000,
                    )
                    .await?;
                Ok(text_block(into_json(resp)))
            }

            "environment_apply_preset" => {
                let preset = args
                    .get("preset")
                    .or_else(|| args.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let sky_uuid = args
                    .get("sky_uuid")
                    .or_else(|| args.get("sky_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let water_uuid = args
                    .get("water_uuid")
                    .or_else(|| args.get("water_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);

                let resp = self
                    .send_to_viewer(
                        lease_id,
                        ViewerRequest::EnvironmentApplyPreset {
                            preset,
                            sky_uuid,
                            water_uuid,
                        },
                        5_000,
                    )
                    .await?;

                if let ViewerResponse::EnvironmentMutated {
                    previous_time_of_day,
                    ref state,
                } = resp
                {
                    self.leases.record_rollback(RollbackAction::RestoreEnvironment {
                        time_of_day: previous_time_of_day,
                    });
                    return Ok(text_block(state.clone()));
                }
                Ok(text_block(into_json(resp)))
            }

            other => Err(McpError::InvalidParams(format!("unknown MCP tool '{other}'"))),
        }
    }

    fn handle_camera_response(&self, resp: ViewerResponse) -> Value {
        match resp {
            ViewerResponse::CameraMutated {
                previous_camera,
                state,
            } => {
                if let Some(rollback) = previous_camera {
                    self.leases.record_rollback(rollback);
                }
                state
            }
            other => into_json(other),
        }
    }
}

fn into_json(resp: ViewerResponse) -> Value {
    match resp {
        ViewerResponse::Json(v) => v,
        ViewerResponse::SettingMutated {
            key,
            previous_value,
            current_value,
        } => json!({
            "ok": true,
            "key": key,
            "before": previous_value,
            "after": current_value,
        }),
        ViewerResponse::CameraMutated { state, .. } => state,
        ViewerResponse::PoserMutated { state } => state,
        ViewerResponse::EnvironmentMutated { state, .. } => state,
        ViewerResponse::FrameCaptured(raw) => json!({
            "width": raw.width,
            "height": raw.height,
            "bytes": raw.rgba_bytes.len(),
        }),
    }
}

fn text_block(val: Value) -> Vec<Value> {
    vec![json!({
        "type": "text",
        "text": serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string()),
    })]
}

fn req_str(args: &Value, key: &str) -> Result<String, McpError> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| McpError::InvalidParams(format!("missing required string '{key}'")))
}

fn parse_req_uuid(args: &Value, key: &str) -> Result<Uuid, McpError> {
    let s = req_str(args, key)?;
    Uuid::parse_str(s.trim())
        .map_err(|e| McpError::InvalidParams(format!("invalid UUID for '{key}': {e}")))
}

fn parse_opt_uuid(args: &Value, key: &str) -> Result<Option<Uuid>, McpError> {
    let Some(s) = args.get(key).and_then(Value::as_str).map(str::trim) else {
        return Ok(None);
    };
    if s.is_empty() {
        return Ok(None);
    }
    Uuid::parse_str(s)
        .map(Some)
        .map_err(|e| McpError::InvalidParams(format!("invalid UUID for '{key}': {e}")))
}

fn parse_vec2(args: &Value, key: &str) -> Result<[f32; 2], McpError> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::InvalidParams(format!("missing 2D array '{key}'")))?;
    if arr.len() != 2 {
        return Err(McpError::InvalidParams(format!(
            "'{key}' must have 2 numbers"
        )));
    }
    Ok([
        arr[0].as_f64().unwrap_or(0.0) as f32,
        arr[1].as_f64().unwrap_or(0.0) as f32,
    ])
}

fn parse_vec3(args: &Value, key: &str) -> Result<[f32; 3], McpError> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::InvalidParams(format!("missing 3D vector '{key}'")))?;
    if arr.len() != 3 {
        return Err(McpError::InvalidParams(format!(
            "'{key}' must have 3 numbers"
        )));
    }
    Ok([
        arr[0].as_f64().unwrap_or(0.0) as f32,
        arr[1].as_f64().unwrap_or(0.0) as f32,
        arr[2].as_f64().unwrap_or(0.0) as f32,
    ])
}

fn parse_vec4(args: &Value, key: &str) -> Result<[f32; 4], McpError> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::InvalidParams(format!("missing 4D vector '{key}'")))?;
    if arr.len() != 4 {
        return Err(McpError::InvalidParams(format!(
            "'{key}' must have 4 numbers"
        )));
    }
    Ok([
        arr[0].as_f64().unwrap_or(0.0) as f32,
        arr[1].as_f64().unwrap_or(0.0) as f32,
        arr[2].as_f64().unwrap_or(0.0) as f32,
        arr[3].as_f64().unwrap_or(1.0) as f32,
    ])
}

fn parse_vec3_or_xyz(args: &Value) -> Result<[f32; 3], McpError> {
    if let Ok(v) = parse_vec3(args, "position") {
        return Ok(v);
    }
    if let Ok(v) = parse_vec3(args, "global_position") {
        return Ok(v);
    }
    let x = args
        .get("x")
        .and_then(Value::as_f64)
        .ok_or_else(|| McpError::InvalidParams("missing 'position' or 'x','y','z'".into()))?
        as f32;
    let y = args
        .get("y")
        .and_then(Value::as_f64)
        .ok_or_else(|| McpError::InvalidParams("missing 'y'".into()))? as f32;
    let z = args
        .get("z")
        .and_then(Value::as_f64)
        .ok_or_else(|| McpError::InvalidParams("missing 'z'".into()))? as f32;
    Ok([x, y, z])
}

fn inspect_viewer_api(api: &str, operation: Option<&str>) -> Value {
    let catalog = json!({
        "LLAppViewer": {
            "description": "Core viewer lifecycle and frame telemetry",
            "operations": {
                "getStatus": { "args": {}, "readonly": true },
                "getVersion": { "args": {}, "readonly": true }
            }
        },
        "LLViewerControl": {
            "description": "Unified viewer settings registry (with lease rollback)",
            "operations": {
                "get": { "args": { "group": "Global", "key": "camera.fov" }, "readonly": true },
                "set": { "args": { "group": "Global", "key": "camera.fov", "value": 1.05 }, "readonly": false },
                "list": { "args": { "search": "", "modified_only": false, "limit": 50 }, "readonly": true }
            }
        },
        "LLAgent": {
            "description": "Avatar status, movement, autopilot, sitting and teleportation",
            "operations": {
                "getPosition": { "args": {}, "readonly": true },
                "walkTo": { "args": { "position": [128.0, 128.0, 25.0] }, "readonly": false },
                "stop": { "args": {}, "readonly": false },
                "sit": { "args": { "target": "<uuid-or-local-id>" }, "readonly": false },
                "stand": { "args": {}, "readonly": false }
            }
        },
        "LLWorld": {
            "description": "Bounded spatial radar for avatars and objects",
            "operations": {
                "getObjects": { "args": { "max_radius_m": 32.0, "limit": 50 }, "readonly": true },
                "getAvatars": { "args": { "max_radius_m": 96.0, "limit": 32 }, "readonly": true }
            }
        },
        "LLCamera": {
            "description": "3D viewport camera control with automatic lease rollback",
            "operations": {
                "set": { "args": { "position": [128.0, 124.0, 26.0], "focus": [128.0, 128.0, 25.0] }, "readonly": false },
                "release": { "args": {}, "readonly": false }
            }
        },
        "Aurora.RLVa": {
            "description": "Direct internal RestrainedLove (@) command execution",
            "operations": {
                "execute": { "args": { "commands": ["@version=2222"] }, "readonly": false }
            }
        },
        "Aurora.Poser": {
            "description": "Native Bento IK/Poser skeleton inspection and joint rotation override",
            "operations": {
                "inspect": { "args": {}, "readonly": true },
                "setBones": { "args": { "transforms": [{ "bone_name": "mShoulderLeft", "rotation_euler_deg": [0.0, 0.0, -35.0] }] }, "readonly": false },
                "reset": { "args": {}, "readonly": false }
            }
        },
        "Aurora.Build": {
            "description": "Async-correlated in-world prim rezzing, transforms, linking, LSL and Linkset Data",
            "operations": {
                "rezNew": { "args": { "position": [128.0, 130.0, 25.0], "scale": [1.0, 1.0, 1.0] }, "readonly": false },
                "transform": { "args": { "local_id": 1, "scale": [2.0, 2.0, 0.5] }, "readonly": false },
                "lsdInspect": { "args": { "object_local_id": 1 }, "readonly": true },
                "lsdMutate": { "args": { "object_local_id": 1, "operations": [] }, "readonly": false }
            }
        }
    });

    if api.is_empty() {
        return json!({
            "available_apis": catalog,
        });
    }
    if let Some(entry) = catalog.get(api) {
        if let Some(op) = operation.filter(|s| !s.is_empty())
            && let Some(op_spec) = entry.get("operations").and_then(|o| o.get(op))
        {
            return json!({
                "api": api,
                "operation": op,
                "spec": op_spec,
            });
        }
        return json!({
            "api": api,
            "details": entry,
        });
    }
    json!({
        "api": api,
        "error": "unknown API name; pass empty string to list all APIs",
        "available_apis": catalog.as_object().map(|m| m.keys().collect::<Vec<_>>()),
    })
}

fn all_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // Phase 0: Session, Leases & Dynamic Dispatch
        ToolDefinition {
            name: "connection_status".into(),
            description: "Check Aurora Viewer runtime MCP status, region connection, FPS, and active control lease.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "capabilities_refresh".into(),
            description: "Refresh and report active simulator capabilities and runtime subsystems.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "control_acquire".into(),
            description: "Acquire a cooperative viewer control lease (Legacy Viewer MCP compatible). Automatically restores settings/camera on release.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "label": { "type": "string", "default": "Aurora MCP workflow" },
                    "seconds": { "type": "integer", "default": 300 },
                    "lease_id": { "type": "string" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "viewer_control_acquire".into(),
            description: "Acquire or renew a cooperative viewer control lease with automatic rollback stack.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "owner": { "type": "string", "default": "aurora-agent" },
                    "ttl_seconds": { "type": "integer", "default": 300 },
                    "lease_id": { "type": "string" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "control_release".into(),
            description: "Release the active control lease and restore any temporary camera/setting overrides (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "lease_id": { "type": "string" } }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "viewer_control_release".into(),
            description: "Release the active control lease and restore all rollback stack entries.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "lease_id": { "type": "string" } }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "viewer_api_inspect".into(),
            description: "Inspect available viewer APIs and operations (LLAppViewer, LLViewerControl, LLAgent, LLWorld, LLCamera, Aurora.RLVa, Aurora.Poser, Aurora.Build).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "api": { "type": "string" },
                    "operation": { "type": ["string", "null"] }
                },
                "required": ["api"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "viewer_call".into(),
            description: "Dynamically invoke a viewer API operation discovered via viewer_api_inspect.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "api": { "type": "string" },
                    "operation": { "type": "string" },
                    "arguments": { "type": "object", "default": {} },
                    "timeout": { "type": "integer", "default": 15 }
                },
                "required": ["api", "operation"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "events_subscribe".into(),
            description: "Subscribe to a bounded viewer event stream (e.g. Chat, StartupState, LLAutopilot, ObjectUpdates, ScriptDebug).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "source": { "type": "string" } },
                "required": ["source"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "events_unsubscribe".into(),
            description: "Unsubscribe from a viewer event stream.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "source": { "type": "string" } },
                "required": ["source"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "events_read".into(),
            description: "Read subscribed viewer events after a sequence cursor.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "after": { "type": "integer", "default": 0 } }
            }),
            in_compact_profile: false,
        },

        // Phase 1: Agent Telemetry, Radar, Comms, Navigation & RLVa
        ToolDefinition {
            name: "get_agent_status".into(),
            description: "Read agent UUID, display name, region name, coordinates (x,y,z), parcel, login state, FPS/VRAM, and sitting target.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "avatar_position".into(),
            description: "Read avatar position/orientation from the viewer (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "radar_scan_avatars".into(),
            description: "Spatially bounded scan of nearby avatars sorted by distance ascending (max 100).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "max_radius_m": { "type": "number", "default": 96.0 },
                    "limit": { "type": "integer", "default": 32, "maximum": 100 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "radar_scan_objects".into(),
            description: "Spatially bounded scan of nearby objects sorted by distance ascending (max 100).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "max_radius_m": { "type": "number", "default": 32.0 },
                    "root_only": { "type": "boolean", "default": true },
                    "name_filter": { "type": "string" },
                    "limit": { "type": "integer", "default": 50, "maximum": 100 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "world_objects".into(),
            description: "List nearby objects within distance in meters (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "distance": { "type": "number", "default": 32.0 } }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "comms_read_history".into(),
            description: "Read recent Local Chat, Group Chat, IM, and Simulator Notice/Dialog entries from the 200-entry ring buffer.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "channel_filter": { "type": "string" },
                    "limit": { "type": "integer", "default": 50, "maximum": 200 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "comms_send".into(),
            description: "Send Local Chat (channel 0), scripted object/HUD channel messages (positive/negative channels), or instant messages.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" },
                    "channel": { "type": "integer", "default": 0 },
                    "chat_type": { "type": "string", "enum": ["whisper", "normal", "shout"], "default": "normal" },
                    "target_uuid": { "type": "string" }
                },
                "required": ["message"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "nav_move_and_teleport".into(),
            description: "Walk to coordinates, sit on an object, stand up, or teleport to a region/coordinate.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["walk_to", "sit_on_object", "stand_up", "teleport_to", "stop"], "default": "walk_to" },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "x": { "type": "number" },
                    "y": { "type": "number" },
                    "z": { "type": "number" },
                    "target_id": { "type": "string" },
                    "region_name": { "type": "string" },
                    "allow_flying": { "type": "boolean", "default": false },
                    "stop_distance": { "type": "number", "default": 0.75 },
                    "timeout_ms": { "type": "integer", "default": 5000 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_walk_to".into(),
            description: "Start walking to coordinates via autopilot (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "global_position": { "type": "array", "items": { "type": "number" } },
                    "allow_flying": { "type": "boolean", "default": false },
                    "stop_distance": { "type": "number", "default": 0.5 }
                },
                "required": ["global_position"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_movement_status".into(),
            description: "Read autopilot progress and current avatar position (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_stop".into(),
            description: "Cancel automatic movement and read resulting avatar state (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "rlva_execute".into(),
            description: "Directly execute internal RestrainedLove (@) commands on Aurora's RLVa engine without needing an in-world LSL relay prim.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "commands": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["commands"]
            }),
            in_compact_profile: false,
        },

        // Phase 2: Settings, Camera, Poser & Floaters
        ToolDefinition {
            name: "settings_query".into(),
            description: "Search Aurora's unified settings registry by name/path.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "search": { "type": "string", "default": "" },
                    "modified_only": { "type": "boolean", "default": false },
                    "limit": { "type": "integer", "default": 50, "maximum": 100 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "settings_set".into(),
            description: "Modify a viewer setting with automatic lease rollback on disconnect/release.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "value": {},
                    "rollback_on_lease_end": { "type": "boolean", "default": true }
                },
                "required": ["name", "value"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "setting_get".into(),
            description: "Read a named viewer setting (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "default": "Global" },
                    "key": { "type": "string" }
                },
                "required": ["key"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "setting_set".into(),
            description: "Change a viewer setting and return before/after readback (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "group": { "type": "string", "default": "Global" },
                    "key": { "type": "string" },
                    "value": {}
                },
                "required": ["key", "value"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "camera_control".into(),
            description: "Control the 3D camera (follow_avatar, focus_object, manual_transform, orbit, release) with automatic lease rollback.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["follow_avatar", "focus_object", "manual_transform", "orbit", "release"] },
                    "object_id": { "type": "string" },
                    "distance": { "type": "number" },
                    "pitch_deg": { "type": "number" },
                    "yaw_deg": { "type": "number" },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "look_at": { "type": "array", "items": { "type": "number" } },
                    "fov_deg": { "type": "number" },
                    "delta_yaw": { "type": "number" },
                    "delta_pitch": { "type": "number" },
                    "zoom": { "type": "number" }
                },
                "required": ["mode"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "camera_set".into(),
            description: "Set a fixed camera position and focus in region coordinates (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "position": { "type": "array", "items": { "type": "number" } },
                    "focus": { "type": "array", "items": { "type": "number" } }
                },
                "required": ["position", "focus"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "camera_release".into(),
            description: "Release scripted camera control back to the viewer (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "mesh_preview_camera".into(),
            description: "Adjust mesh preview or avatar inspection camera orbit angles.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "yaw": { "type": "number", "default": 30.0 },
                    "pitch": { "type": "number", "default": 15.0 },
                    "distance": { "type": "number", "default": 3.0 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "poser_inspect".into(),
            description: "Inspect Aurora's native Bento IK/Poser state, controllable joints, and current local rotations.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "subject": { "type": "string" } }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "poser_set_bones".into(),
            description: "Batch-apply Euler (degrees) or Quaternion rotations to avatar Bento skeleton bones in the Poser.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "subject": { "type": "string" },
                    "transforms": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "bone_name": { "type": "string" },
                                "rotation_euler_deg": { "type": "array", "items": { "type": "number" } },
                                "rotation_quat": { "type": "array", "items": { "type": "number" } },
                                "position_offset": { "type": "array", "items": { "type": "number" } },
                                "scale": { "type": "array", "items": { "type": "number" } }
                            },
                            "required": ["bone_name"]
                        }
                    }
                },
                "required": ["transforms"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "poser_reset".into(),
            description: "Stop poser override and restore default avatar animation state.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "subject": { "type": "string" } }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "floater_list".into(),
            description: "List viewer panels/floaters and their open state (direct state read, no UI scraping).".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "floater_open".into(),
            description: "Open or close a named viewer panel/floater directly via internal state.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "open": { "type": "boolean", "default": true }
                },
                "required": ["name"]
            }),
            in_compact_profile: false,
        },

        // Phase 3: Vision / Viewport Capture
        ToolDefinition {
            name: "viewport_capture".into(),
            description: "Non-blocking GPU readback of the rendered viewport (ColorRgb or linearized DepthNormalized) returned as an MCP PNG image + metadata.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "buffer_type": { "type": "string", "enum": ["ColorRgb", "DepthNormalized"], "default": "ColorRgb" },
                    "include_ui": { "type": "boolean", "default": false },
                    "include_hud": { "type": "boolean", "default": false },
                    "max_width": { "type": "integer", "default": 1024 },
                    "max_height": { "type": "integer", "default": 1024 },
                    "label": { "type": "string", "default": "viewport" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "snapshot".into(),
            description: "Capture rendered viewport image (COLOR or DEPTH) and return MCP image + saved PNG path and SHA256 (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "default": 1280 },
                    "height": { "type": "integer", "default": 720 },
                    "layer": { "type": "string", "enum": ["COLOR", "DEPTH"], "default": "COLOR" },
                    "show_ui": { "type": "boolean", "default": false },
                    "show_hud": { "type": "boolean", "default": false },
                    "label": { "type": "string", "default": "snapshot" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "capture_orbit".into(),
            description: "Capture repeatable orbit views around a region-coordinate focus and write a JSON manifest.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "focus": { "type": "array", "items": { "type": "number" } },
                    "radius": { "type": "number", "default": 3.0 },
                    "height_offset": { "type": "number", "default": 1.0 },
                    "views": { "type": "integer", "default": 4 },
                    "label": { "type": "string", "default": "asset" },
                    "evidence_kind": { "type": "string", "default": "unspecified_viewer_scene" }
                },
                "required": ["focus"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "capture_manifest_read".into(),
            description: "Read a saved capture_orbit manifest by path or manifest_id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "manifest_path": { "type": "string" },
                    "manifest_id": { "type": "string" }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "image_compare".into(),
            description: "Compare two same-size PNG images on disk, computing RMSE/MAE/changed pixels and saving a visual diff PNG.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "reference": { "type": "string" },
                    "observed": { "type": "string" }
                },
                "required": ["reference", "observed"]
            }),
            in_compact_profile: true,
        },

        // Phase 4: Inventory & Appearance (COF Queue)
        ToolDefinition {
            name: "inventory_browse".into(),
            description: "Browse inventory folders by UUID or path (e.g. '/My Inventory/Objects') with bounded depth and item limit.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "folder_id_or_path": { "type": "string", "default": "/" },
                    "depth": { "type": "integer", "default": 1, "maximum": 5 },
                    "limit": { "type": "integer", "default": 50, "maximum": 100 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "inventory_search".into(),
            description: "Search inventory items with creator permission masks enforced (Legacy Viewer & Aurora compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "name": { "type": "string" },
                    "folder_id": { "type": "string" },
                    "asset_type": { "type": ["string", "null"] },
                    "worn_only": { "type": "boolean", "default": false },
                    "limit": { "type": "integer", "default": 50, "maximum": 100 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "appearance_get_worn".into(),
            description: "List all currently worn system wearables/body parts and attachments with attachment point names.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "appearance_mutate".into(),
            description: "Attach, detach, or wear an inventory item/folder serialized through the COF state-machine queue.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["attach", "detach", "wear_folder", "remove_folder"] },
                    "item_id": { "type": "string" },
                    "folder_id": { "type": "string" },
                    "attach_point": { "type": "integer" },
                    "replace_existing": { "type": "boolean", "default": false },
                    "append": { "type": "boolean", "default": true }
                },
                "required": ["op"]
            }),
            in_compact_profile: false,
        },

        // Phase 5: World Building, LSL, Linkset Data & Local Mesh
        ToolDefinition {
            name: "prim_rez_new".into(),
            description: "Rez a new prim at region coordinates and wait for the simulator ObjectUpdate ack to return the new local_id and full_uuid.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "position": { "type": "array", "items": { "type": "number" } },
                    "scale": { "type": "array", "items": { "type": "number" }, "default": [0.5, 0.5, 0.5] },
                    "rotation_quat": { "type": "array", "items": { "type": "number" }, "default": [0.0, 0.0, 0.0, 1.0] },
                    "pcode": { "type": "integer", "default": 9 },
                    "name": { "type": "string" }
                },
                "required": ["position"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_transform".into(),
            description: "Transform one or more prims (position, rotation, scale, name, description) and wait for simulator echo convergence.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "position": { "type": "array", "items": { "type": "number" } },
                    "rotation_quat": { "type": "array", "items": { "type": "number" } },
                    "rotation_euler_deg": { "type": "array", "items": { "type": "number" } },
                    "scale": { "type": "array", "items": { "type": "number" } },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "transforms": { "type": "array", "items": { "type": "object" } }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_set_shape".into(),
            description: "Modify prim volume parameters (path_cut, profile_cut, hollow, twist) or face color/texture.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "path_cut": { "type": "array", "items": { "type": "number" } },
                    "profile_cut": { "type": "array", "items": { "type": "number" } },
                    "hollow": { "type": "number" },
                    "twist": { "type": "array", "items": { "type": "number" } },
                    "color": { "type": "array", "items": { "type": "number" } },
                    "texture_id": { "type": "string" }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_link".into(),
            description: "Link or delink a list of prim local_ids (first local_id is root when linking).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_ids": { "type": "array", "items": { "type": "integer" } },
                    "delink": { "type": "boolean", "default": false }
                },
                "required": ["local_ids"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_delete".into(),
            description: "Delete (de-rez to trash) one or more prims by local_id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_ids": { "type": "array", "items": { "type": "integer" } }
                },
                "required": ["local_ids"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsl_inspect_object_scripts".into(),
            description: "Request and return task inventory items and scripts inside an in-world prim.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "object_local_id": { "type": "integer" } },
                "required": ["object_local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsl_read_script".into(),
            description: "Read the LSL source code of a script inside a prim (enforcing creator copy+modify permissions).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "object_local_id": { "type": "integer" },
                    "script_item_id": { "type": "string" }
                },
                "required": ["object_local_id", "script_item_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsl_write_and_compile".into(),
            description: "Write and compile LSL source into a prim task script and return compiler diagnostics or success.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "object_local_id": { "type": "integer" },
                    "script_item_id": { "type": "string" },
                    "script_name": { "type": "string", "default": "New Script" },
                    "source_code": { "type": "string" },
                    "mono": { "type": "boolean", "default": true },
                    "running": { "type": "boolean", "default": true }
                },
                "required": ["object_local_id", "source_code"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsl_listen_script_output".into(),
            description: "Capture llOwnerSay, llSay, and DEBUG_CHANNEL script output from a target prim (by local_id or object_id UUID).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "object_id": { "type": "string" },
                    "duration_ms": { "type": "integer", "default": 500 }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsd_inspect".into(),
            description: "Inspect Linkset Data (LSD) keys, values, protected status, and 128 KiB byte quota usage on a linkset.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "object_local_id": { "type": "integer" },
                    "filter_regex": { "type": "string" },
                    "include_protected": { "type": "boolean", "default": true },
                    "passphrase": { "type": "string" },
                    "limit": { "type": "integer", "default": 50, "maximum": 100 }
                },
                "required": ["object_local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsd_mutate".into(),
            description: "Batch write, delete, reset, or import JSON into a linkset's 128 KiB Linkset Data store.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "object_local_id": { "type": "integer" },
                    "operations": { "type": "array", "items": { "type": "object" } }
                },
                "required": ["object_local_id", "operations"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "lsd_stress_test".into(),
            description: "Offline pre-flight validator for LSL Linkset Data JSON payloads against the 128 KiB store limit and Mono 32 KiB string ceiling.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "payload": {},
                    "protected_keys": { "type": "boolean", "default": false },
                    "filter_regex": { "type": "string" }
                },
                "required": ["payload"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "asset_inspect".into(),
            description: "Inspect a local COLLADA/glTF/GLB/texture file and compute its SHA256, geometry counts, bounding box, and SL upload warnings (Legacy Viewer MCP compatible).".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "filename": { "type": "string" } },
                "required": ["filename"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "asset_inspect_local".into(),
            description: "Inspect a local .gltf, .glb, .dae, or image file before upload.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "file_path": { "type": "string" } },
                "required": ["file_path"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "local_mesh_status".into(),
            description: "Read Aurora's local mesh / Blender Live preview status.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "local_mesh_open".into(),
            description: "Open or close Aurora's local mesh / Blender Live panel.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "open": { "type": "boolean", "default": true } }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "local_mesh_auto_reload".into(),
            description: "Enable or disable automatic reload for local mesh previewing.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "enabled": { "type": "boolean", "default": true } }
            }),
            in_compact_profile: false,
        },
        // ---- Phase 6: Suite 1 — Direct Asset Upload, LiveMesh, Inventory Rez & Materials ----
        ToolDefinition {
            name: "livemesh_control".into(),
            description: "Programmatically inspect or control Aurora's LiveMesh (Blender Live) listener: status, start, stop, import_to_uploader, or remove_meshes.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "start", "stop", "import_to_uploader", "remove_meshes"],
                        "default": "status"
                    },
                    "source_id": { "type": "string" }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "asset_upload".into(),
            description: "Programmatically stage local files (.glb, .dae, .png, .wav, .bvh/.anim) into Aurora's Importer without OS dialogs, request simulator upload fee quotes, or start confirmed uploads.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["status", "stage", "quote", "upload"],
                        "default": "status"
                    },
                    "paths": { "type": "array", "items": { "type": "string" } },
                    "path": { "type": "string" },
                    "choice": {
                        "type": "string",
                        "enum": ["mesh", "texture", "sound", "animation", "material", "bulk", "local"]
                    },
                    "include_textures": { "type": "boolean", "default": true }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "inventory_rez_object".into(),
            description: "Rez an object from Agent Inventory at an exact [x, y, z] world position without mouse drag-and-drop.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "item_id": { "type": "string" },
                    "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                    "select": { "type": "boolean", "default": false }
                },
                "required": ["item_id", "position"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "object_take_or_derez".into(),
            description: "Take, take a copy, delete to trash, or return an in-world object by local_id.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "action": {
                        "type": "string",
                        "enum": ["take", "take_copy", "delete", "return"],
                        "default": "take"
                    },
                    "folder_id": { "type": "string" }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_set_material".into(),
            description: "Set per-face (or all-sides) TextureEntry parameters (texture UUID, RGBA color, UV repeats/offsets/rotation, glow, fullbright, alpha_mode, bump_shiny) and PBR glTF material asset UUID on a prim.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "face": { "type": "integer", "description": "0..N for a specific face, or -1 (or omitted) for all faces" },
                    "texture_id": { "type": "string" },
                    "material_id": { "type": "string" },
                    "color": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
                    "repeats": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "offsets": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 },
                    "rotation_rad": { "type": "number" },
                    "rotation_deg": { "type": "number" },
                    "glow": { "type": "number" },
                    "fullbright": { "type": "boolean" },
                    "alpha_mode": { "type": "integer" },
                    "bump_shiny": { "type": "integer" }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        // ---- Phase 6: Suite 2 — Remaining 13 Legacy Viewer MCP Tools (43/43 Parity) ----
        ToolDefinition {
            name: "mesh_upload_open".into(),
            description: "Open the standard mesh upload preview workflow. Does not submit an upload or authorize an upload fee.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "mesh_upload_status".into(),
            description: "Read importer LOD files/counts, physics, dimensions, warnings, weights, displayed fee and visibility.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "native_file_dialogs".into(),
            description: "List active or queued file selection dialogs in the viewer.".into(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "native_file_choose".into(),
            description: "Select a local file path for the viewer's importer without requiring manual OS dialog interaction.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "dialog_id": { "type": "integer", "default": 1 },
                    "filename": { "type": "string" }
                },
                "required": ["dialog_id", "filename"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_inspect".into(),
            description: "Read a known UI path's geometry and enabled/visible state without opening or focusing it.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "default": "/main_view" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_find".into(),
            description: "Search UI paths or basenames, case-insensitively, within a narrow under path.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "under": { "type": "string", "default": "" },
                    "search_in": { "type": "string", "enum": ["path", "name"], "default": "path" },
                    "match": { "type": "string", "enum": ["contains", "exact", "prefix", "glob"], "default": "contains" },
                    "max_depth": { "type": "integer" },
                    "include_info": { "type": "boolean", "default": false },
                    "offset": { "type": "integer", "default": 0 },
                    "limit": { "type": "integer", "default": 50 }
                },
                "required": ["query"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_click".into(),
            description: "Click a visible, enabled UI control by path and return before/after state.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "button": { "type": "string", "default": "LEFT" },
                    "floater": { "type": "string" },
                    "observe_path": { "type": "string" }
                },
                "required": ["path"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_get_value".into(),
            description: "Read the current value of a discovered UI control.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_set_text".into(),
            description: "Replace text in a discovered UI edit control and read back its value.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["path", "text"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_select".into(),
            description: "Select a visible enabled combobox or tab item by value and return readback.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "value": {}
                },
                "required": ["path", "value"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_press_key".into(),
            description: "Focus a UI path, press/release a key with optional modifiers, and read back state.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "keysym": { "type": "string" },
                    "modifiers": { "type": "array", "items": { "type": "string" }, "default": [] },
                    "observe_path": { "type": "string" }
                },
                "required": ["path", "keysym"]
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_list_menus".into(),
            description: "List viewer menu entries and bar actions, optionally filtered by query.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "default": "" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "ui_invoke_menu".into(),
            description: "Invoke a viewer menu entry or bar action by exact name.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }),
            in_compact_profile: true,
        },
        // ---- Phase 6: Suite 3 — In-World Object Interaction, Notecards & Particles ----
        ToolDefinition {
            name: "object_touch".into(),
            description: "Touch/grab an in-world prim at a specific face, UV, and ST coordinate (supports multi-button single-prim HUDs and panels).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "face": { "type": "integer", "default": 0 },
                    "uv": { "type": "array", "items": { "type": "number" }, "default": [0.5, 0.5] },
                    "st": { "type": "array", "items": { "type": "number" }, "default": [0.5, 0.5] },
                    "hold_ms": { "type": "integer", "default": 50 }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_sit".into(),
            description: "Sit on a target in-world object (by local_id / target_local_id or target_uuid / object_id) or stand up.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["sit", "stand"], "default": "sit" },
                    "local_id": { "type": "integer" },
                    "target_local_id": { "type": "integer" },
                    "target_uuid": { "type": "string" },
                    "object_id": { "type": "string" },
                    "offset": { "type": "array", "items": { "type": "number" } }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "notecard_read".into(),
            description: "Read the text of a notecard in Agent Inventory or inside an object's Task Inventory (enforces creator copy permissions and RLVa @viewnote).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "item_id": { "type": "string" },
                    "object_local_id": { "type": "integer" }
                },
                "required": ["item_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "notecard_write".into(),
            description: "Create or update a notecard in Agent Inventory or inside an object's Task Inventory (enforces creator modify permissions).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "item_id": { "type": "string" },
                    "object_local_id": { "type": "integer" },
                    "name": { "type": "string" },
                    "body": { "type": "string" }
                },
                "required": ["body"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_set_particles".into(),
            description: "Configure or clear a Second Life particle emitter on an owned prim using Aurora's ParticleEditor LSL generator.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer" },
                    "clear": { "type": "boolean", "default": false },
                    "pattern": { "type": "string", "enum": ["drop", "explode", "angle", "angle_cone"] },
                    "texture_id": { "type": "string" },
                    "start_color": { "type": "array", "items": { "type": "number" } },
                    "end_color": { "type": "array", "items": { "type": "number" } },
                    "start_scale": { "type": "array", "items": { "type": "number" } },
                    "end_scale": { "type": "array", "items": { "type": "number" } },
                    "particle_max_age": { "type": "number" },
                    "burst_rate": { "type": "number" },
                    "burst_part_count": { "type": "integer" },
                    "burst_radius": { "type": "number" },
                    "burst_speed_min": { "type": "number" },
                    "burst_speed_max": { "type": "number" },
                    "accel": { "type": "array", "items": { "type": "number" } },
                    "omega": { "type": "array", "items": { "type": "number" } },
                    "follow_src": { "type": "boolean" },
                    "follow_velocity": { "type": "boolean" },
                    "interp_color": { "type": "boolean" },
                    "interp_scale": { "type": "boolean" },
                    "emissive": { "type": "boolean" },
                    "ribbon": { "type": "boolean" }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "viewer_login".into(),
            description: "Initiate login into the grid when the viewer is on the login screen. If username and password are omitted, logs in automatically using saved account credentials from the viewer's secure credential store.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "username": { "type": "string", "description": "Optional account name (e.g. 'First Last' or 'first.last'). If omitted, uses active saved account." },
                    "password": { "type": "string", "description": "Optional account password. If omitted, uses remembered credentials from the credential store." },
                    "location": { "type": "string", "description": "Optional start location: 'home', 'last', or region name." },
                    "mfa_token": { "type": "string", "description": "Optional 6-digit multi-factor authentication code from authenticator app if MFA is required." }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "viewer_shutdown".into(),
            description: "Gracefully shut down and exit the viewer process. Flushes settings, inventories, layout caches, disconnects cleanly from the simulator, and signals the watchdog for a clean exit without triggering crash reports.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "lease_id": { "type": "string", "description": "Optional active lease ID" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "prim_set_physics_properties".into(),
            description: "Modify physics flags (physical, temporary, phantom) and simulator physics collision shape parameters (prim, convex hull, none, density, friction, restitution) for a primitive or linkset.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer", "description": "Local ID of target primitive" },
                    "physics": { "type": "boolean", "description": "Enable or disable rigid body dynamics (FLAGS_USE_PHYSICS)" },
                    "temporary": { "type": "boolean", "description": "Mark as temporary on rez (dies after ~1 minute)" },
                    "phantom": { "type": "boolean", "description": "Enable or disable phantom mode (avatar walks through)" },
                    "shape_type": { "type": "string", "enum": ["prim", "convex_hull", "none"], "description": "Collision shape type: 'prim' (accurate), 'convex_hull' (low-cost hull), or 'none' (no collision)" },
                    "density": { "type": "number", "description": "Material density in kg/m^3 (default 1000.0)" },
                    "friction": { "type": "number", "description": "Surface friction coefficient [0.0 .. 1.0] (default 0.6)" },
                    "restitution": { "type": "number", "description": "Restitution / bounciness [0.0 .. 1.0] (default 0.0)" },
                    "gravity_multiplier": { "type": "number", "description": "Gravity multiplier (default 1.0)" }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "prim_set_metadata".into(),
            description: "Update the in-world name and description of a primitive or linkset.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "local_id": { "type": "integer", "description": "Local ID of target primitive" },
                    "name": { "type": "string", "description": "New in-world object name" },
                    "description": { "type": "string", "description": "New in-world object description" }
                },
                "required": ["local_id"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_animation_play".into(),
            description: "Start playing an animation on the agent avatar. Accepts either standard names ('dance', 'dance1'..'dance8', 'wave', 'bow', 'clap', 'cheer', 'point', 'shrug', 'laugh', 'cry', 'kiss', 'sit', 'stand', 'walk', 'run', 'fly', 'hover', 'type') or an asset UUID.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "animation": { "type": "string", "description": "Animation name (e.g. 'dance', 'wave', 'bow', 'clap') or asset UUID" },
                    "loop": { "type": "boolean", "description": "Whether the animation loops (default true)" }
                },
                "required": ["animation"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_animation_stop".into(),
            description: "Stop playing an animation on the agent avatar by name or UUID.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "animation": { "type": "string", "description": "Animation name or asset UUID to halt" }
                },
                "required": ["animation"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "avatar_animations_list".into(),
            description: "List all currently active / playing animations on the agent avatar with elapsed duration and UUIDs.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "economy_get_balance".into(),
            description: "Retrieve the avatar's current Linden Dollar (L$) account balance.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "parcel_info".into(),
            description: "Query comprehensive metadata for the active or requested parcel, including name, description, owner UUID, group ownership, area in m², prim limits and usage (owner, group, other), audio stream URL, media URL, region bounding box, landing location, and flags.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "parcel_set_music".into(),
            description: "Update the streaming audio / internet radio URL of the parcel (e.g. Icecast, Shoutcast, or direct MP3/AAC audio stream) and automatically initiate audio stream playback.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "music_url": { "type": "string", "description": "Stream URL (e.g. 'http://stream.example.org:8000/live.mp3')" }
                },
                "required": ["music_url"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "parcel_set_properties".into(),
            description: "Modify parcel settings including name, description, and permission flags.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "New parcel name" },
                    "description": { "type": "string", "description": "New parcel description" },
                    "flags": { "type": "integer", "description": "Raw parcel flag bitmask" }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "parcel_access_list_modify".into(),
            description: "Add or remove an avatar from the parcel's access whitelist or ban blacklist.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "avatar_uuid": { "type": "string", "description": "Target resident UUID" },
                    "action": { "type": "string", "enum": ["add", "remove"], "description": "'add' to insert entry, 'remove' to remove entry" },
                    "list": { "type": "string", "enum": ["ban", "allow", "access"], "description": "Target list: 'ban' or 'allow'" }
                },
                "required": ["avatar_uuid"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "parcel_eject".into(),
            description: "Eject a resident from the parcel back to region entry, with optional immediate parcel ban.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target_uuid": { "type": "string", "description": "Resident UUID to eject" },
                    "ban": { "type": "boolean", "description": "Whether to also ban the avatar from the parcel (default false)" }
                },
                "required": ["target_uuid"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "economy_pay_avatar".into(),
            description: "Transfer Linden Dollars (L$) from the agent's account balance directly to another resident with an optional transaction message.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target_uuid": { "type": "string", "description": "Target resident avatar UUID" },
                    "amount": { "type": "integer", "description": "Amount in Linden Dollars (L$) to transfer (must be > 0)" },
                    "message": { "type": "string", "description": "Optional payment note or message" }
                },
                "required": ["target_uuid", "amount"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "economy_pay_object".into(),
            description: "Pay Linden Dollars (L$) to an in-world object (such as a tip jar, vendor, or event terminal).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "target_local_id": { "type": "integer", "description": "Local ID of target object in simulator" },
                    "amount": { "type": "integer", "description": "Amount in Linden Dollars (L$) to pay (must be > 0)" },
                    "description": { "type": "string", "description": "Optional description for payment receipt" }
                },
                "required": ["target_local_id", "amount"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "groups_list".into(),
            description: "List all groups the avatar is currently a member of, including group titles, powers bitmask, contribution, and currently active group.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "groups_activate".into(),
            description: "Activate a group by UUID to display its tag and title over the avatar in-world.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "group_uuid": { "type": "string", "description": "UUID of the group to activate (or '00000000-0000-0000-0000-000000000000' to clear active tag)" }
                },
                "required": ["group_uuid"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "friends_list".into(),
            description: "Retrieve the avatar's friends list, including display names, online status, and granted friendship permissions (can see online, see on map, modify objects).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "online_only": { "type": "boolean", "description": "Filter results to only friends currently online (default false)" }
                }
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "environment_get".into(),
            description: "Read the active environment enhancement project (EEP) lighting, sky parameters, water settings, time of day preset, sun/moon directions, and ambient light.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "environment_set_time".into(),
            description: "Override the local time of day or sun position for cinematic captures and daytime/nighttime inspection without altering simulator time. Automatically rolls back when control lease is released.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "enum": ["shared", "sunrise", "noon", "sunset", "midnight"], "description": "Named environment preset: 'shared' (region default), 'sunrise', 'noon' / 'midday', 'sunset', or 'midnight'" },
                    "time_of_day": { "type": "number", "description": "Optional fractional hour [0.0 .. 24.0] (e.g. 12.0 for midday, 18.5 for sunset, 0.0 for midnight)" }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "environment_apply_preset".into(),
            description: "Apply a local environment preset, custom Sky asset UUID, or custom Water asset UUID, or reset local overrides back to the simulator/region default ('reset' or 'clear').".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "preset": { "type": "string", "description": "Preset name or command: 'reset' / 'clear' to restore region default" },
                    "sky_uuid": { "type": "string", "description": "Optional custom Sky asset UUID" },
                    "water_uuid": { "type": "string", "description": "Optional custom Water asset UUID" }
                }
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "voice_status".into(),
            description: "Inspect the viewer's spatial WebRTC voice connection status, channel type (spatial parcel/region vs. multiagent call), ear listening mode, and microphone speaking level.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "voice_participants_list".into(),
            description: "List all participants currently in voice range (or in an active voice call), including their speaking state, audio volume energy, moderator mute status, and user volume settings.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            in_compact_profile: true,
        },
        ToolDefinition {
            name: "voice_set_user_volume".into(),
            description: "Adjust the volume level of a specific resident in the active spatial or call voice channel [0.0 = silent, 0.5 = normal, 1.0 = maximum boost]. Requires an active automation lease.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "avatar_uuid": { "type": "string", "description": "Target resident UUID" },
                    "volume": { "type": "number", "minimum": 0.0, "maximum": 1.0, "description": "Volume scalar between 0.0 (silent) and 1.0 (loud)" }
                },
                "required": ["avatar_uuid", "volume"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "voice_set_user_mute".into(),
            description: "Locally mute or unmute a resident in the active voice session. Requires an active automation lease.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "avatar_uuid": { "type": "string", "description": "Target resident UUID" },
                    "mute": { "type": "boolean", "description": "True to mute, false to unmute" }
                },
                "required": ["avatar_uuid", "mute"]
            }),
            in_compact_profile: false,
        },
        ToolDefinition {
            name: "voice_transmit".into(),
            description: "Set the avatar's in-world microphone transmission state (push-to-talk). When enabled with an active microphone, avatar transmits voice audio into the region. Requires an active automation lease.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "transmitting": { "type": "boolean", "description": "True to activate microphone transmission (talking), false to mute/stop" }
                },
                "required": ["transmitting"]
            }),
            in_compact_profile: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_router_exposes_legacy_and_aurora_tools() {
        let defs = all_tool_definitions();
        assert_eq!(defs.len(), 111, "expected 111 total tools (43 Legacy + 68 Aurora-native)");
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        // Verify all 43 Legacy Viewer compatibility tools
        for legacy in [
            "control_acquire",
            "control_release",
            "connection_status",
            "capabilities_refresh",
            "viewer_api_inspect",
            "viewer_call",
            "events_subscribe",
            "events_read",
            "events_unsubscribe",
            "avatar_position",
            "avatar_walk_to",
            "avatar_movement_status",
            "avatar_stop",
            "world_objects",
            "camera_set",
            "camera_release",
            "mesh_preview_camera",
            "setting_get",
            "setting_set",
            "snapshot",
            "capture_orbit",
            "capture_manifest_read",
            "image_compare",
            "inventory_search",
            "asset_inspect",
            "local_mesh_open",
            "local_mesh_status",
            "local_mesh_auto_reload",
            "floater_list",
            "floater_open",
            "mesh_upload_open",
            "mesh_upload_status",
            "native_file_dialogs",
            "native_file_choose",
            "ui_inspect",
            "ui_find",
            "ui_click",
            "ui_get_value",
            "ui_set_text",
            "ui_select",
            "ui_press_key",
            "ui_list_menus",
            "ui_invoke_menu",
        ] {
            assert!(
                names.contains(&legacy),
                "missing Legacy Viewer MCP tool: {legacy}"
            );
        }
        // Verify all 42 Aurora-native & LSD tools
        for native in [
            "viewer_control_acquire",
            "viewer_control_release",
            "get_agent_status",
            "radar_scan_avatars",
            "radar_scan_objects",
            "comms_read_history",
            "comms_send",
            "nav_move_and_teleport",
            "rlva_execute",
            "settings_query",
            "settings_set",
            "camera_control",
            "poser_inspect",
            "poser_set_bones",
            "poser_reset",
            "viewport_capture",
            "inventory_browse",
            "appearance_get_worn",
            "appearance_mutate",
            "prim_rez_new",
            "prim_transform",
            "prim_set_shape",
            "prim_link",
            "prim_delete",
            "lsl_inspect_object_scripts",
            "lsl_read_script",
            "lsl_write_and_compile",
            "lsl_listen_script_output",
            "lsd_inspect",
            "lsd_mutate",
            "lsd_stress_test",
            "asset_inspect_local",
            "livemesh_control",
            "asset_upload",
            "inventory_rez_object",
            "object_take_or_derez",
            "prim_set_material",
            "object_touch",
            "avatar_sit",
            "notecard_read",
            "notecard_write",
            "prim_set_particles",
            "viewer_login",
            "viewer_shutdown",
            "prim_set_physics_properties",
            "prim_set_metadata",
            "avatar_animation_play",
            "avatar_animation_stop",
            "avatar_animations_list",
            "economy_get_balance",
            "parcel_info",
            "parcel_set_music",
            "parcel_set_properties",
            "parcel_access_list_modify",
            "parcel_eject",
            "economy_pay_avatar",
            "economy_pay_object",
            "groups_list",
            "groups_activate",
            "friends_list",
            "environment_get",
            "environment_set_time",
            "environment_apply_preset",
            "voice_status",
            "voice_participants_list",
            "voice_set_user_volume",
            "voice_set_user_mute",
            "voice_transmit",
        ] {
            assert!(
                names.contains(&native),
                "missing Aurora-native MCP tool: {native}"
            );
        }
    }
}
