//! Native Rust Model Context Protocol (MCP) runtime server for Aurora Viewer.
//!
//! Exposes both Legacy Viewer MCP-compatible tools and Aurora-native capabilities
//! (Bento IK/Poser, RLVa internal command execution, non-blocking COLOR/DEPTH
//! viewport capture, COF state-machine mutation queue, async build correlation,
//! LSL compilation, and Linkset Data inspection/stress-testing).

pub mod asset_inspect;
pub mod audit;
pub mod auth;
pub mod bridge;
pub mod events;
pub mod lease;
pub mod lsd;
pub mod lsl_lint;
pub mod policy;
pub mod prompts;
pub mod rate_limit;
pub mod resources;
pub mod router;
pub mod server;
pub mod vision;

pub use audit::{AuditEntry, AuditLog, hash_args_sha256};
pub use bridge::{
    AppearanceOperation, BRIDGE_CHANNEL_CAPACITY, BoneTransform, CameraCommand, CaptureBufferType, ChatKind, DEFAULT_MCP_PORT,
    FRAME_BUDGET_MICROS, LsdOperation, MAX_BOUNDED_ITEMS, McpError, McpRequestEnvelope, PrimTransformSpec, RawCapturedFrame,
    RollbackAction, ViewerRequest, ViewerResponse, create_bridge_channels,
};
pub use events::{EventBus, EventFilter, KNOWN_EVENT_SOURCES, McpEvent};
pub use lease::{ControlView, HumanMode, LeaseManager, LeaseSnapshot, QueuedAgent};
pub use lsd::LinksetDataStore;
pub use policy::{ApprovalQueue, McpPolicy, PendingApprovalView, PolicyMode, destructive_hints_enabled};
pub use rate_limit::{RateBucket, RateLimiter};
pub use router::{McpProfile, McpRouter, ToolAnnotations, ToolDefinition, all_tool_definitions, list_tools_for_profile};
pub use server::{MCP_PROTOCOL_VERSION, McpServerHandle, SUPPORTED_PROTOCOL_VERSIONS, negotiate_protocol_version, spawn_server};
