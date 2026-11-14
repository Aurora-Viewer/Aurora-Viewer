//! Native Rust Model Context Protocol (MCP) runtime server for Aurora Viewer.
//!
//! Exposes both Legacy Viewer MCP-compatible tools and Aurora-native capabilities
//! (Bento IK/Poser, RLVa internal command execution, non-blocking COLOR/DEPTH
//! viewport capture, COF state-machine mutation queue, async build correlation,
//! LSL compilation, and Linkset Data inspection/stress-testing).

pub mod asset_inspect;
pub mod bridge;
pub mod lease;
pub mod lsd;
pub mod router;
pub mod server;
pub mod vision;

pub use bridge::{
    AppearanceOperation, BRIDGE_CHANNEL_CAPACITY, BoneTransform, CameraCommand, CaptureBufferType, ChatKind, DEFAULT_MCP_PORT,
    FRAME_BUDGET_MICROS, LsdOperation, MAX_BOUNDED_ITEMS, McpError, McpRequestEnvelope, PrimTransformSpec, RawCapturedFrame,
    RollbackAction, ViewerRequest, ViewerResponse, create_bridge_channels,
};
pub use lease::{LeaseManager, LeaseSnapshot};
pub use lsd::LinksetDataStore;
pub use router::{McpProfile, McpRouter, ToolDefinition, all_tool_definitions, list_tools_for_profile};
pub use server::{MCP_PROTOCOL_VERSION, McpServerHandle, spawn_server};
