//! Asynchronous bridge protocol between the `aurora-mcp` background server and
//! the main viewer frame loop (`crates/aurora-viewer/src/mcp_bridge.rs`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Default local port for the Streamable HTTP / SSE MCP server.
pub const DEFAULT_MCP_PORT: u16 = 3847;

/// Bounded command channel capacity to prevent memory growth if the main
/// thread is paused or loading.
pub const BRIDGE_CHANNEL_CAPACITY: usize = 64;

/// Maximum per-frame time budget spent polling MCP commands on the main loop.
pub const FRAME_BUDGET_MICROS: u64 = 1500;

/// Maximum number of items returned by any bounded scan or search tool.
pub const MAX_BOUNDED_ITEMS: usize = 100;

#[derive(Debug, Clone, Error, Serialize, Deserialize, PartialEq, Eq)]
pub enum McpError {
    #[error("control lease required: call control_acquire or viewer_control_acquire first")]
    NoActiveLease,
    #[error("control lease held by '{holder}' (expires in {remaining_secs}s)")]
    LeaseConflict {
        holder: String,
        remaining_secs: u64,
    },
    #[error("viewer main loop is disconnected or shutting down")]
    ViewerDisconnected,
    #[error("operation timed out after {0}ms")]
    Timeout(u64),
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("creator permissions forbid this operation: {0}")]
    PermissionDenied(String),
    #[error("viewer is not currently logged into a region or demo world")]
    NotInWorld,
    #[error("{0}")]
    Internal(String),
}

/// Envelope sent from the async Tokio MCP server to the viewer's main loop.
pub struct McpRequestEnvelope {
    pub lease_id: Option<String>,
    pub request: ViewerRequest,
    pub reply_tx: oneshot::Sender<Result<ViewerResponse, McpError>>,
}

/// State snapshot restored automatically when a control lease expires or is released.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RollbackAction {
    RestoreSetting {
        key: String,
        value: Value,
    },
    RestoreCamera {
        mode: String,
        position: [f32; 3],
        focus: [f32; 3],
        fov: f32,
    },
    ResetPoser,
    RestoreEnvironment {
        time_of_day: u8,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    Whisper,
    Normal,
    Shout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoneTransform {
    pub bone_name: String,
    /// Optional Euler angles in degrees `[rx, ry, rz]`.
    pub rotation_euler_deg: Option<[f32; 3]>,
    /// Optional quaternion `[x, y, z, w]`.
    pub rotation_quat: Option<[f32; 4]>,
    /// Optional local translation offset `[x, y, z]`.
    pub position_offset: Option<[f32; 3]>,
    /// Optional local scale `[sx, sy, sz]`.
    pub scale: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum CameraCommand {
    FollowAvatar,
    Release,
    FocusObject {
        object_id: String,
        distance: f32,
        pitch_deg: f32,
        yaw_deg: f32,
    },
    ManualTransform {
        position: [f32; 3],
        focus: [f32; 3],
        fov_deg: Option<f32>,
    },
    Orbit {
        delta_yaw_deg: f32,
        delta_pitch_deg: f32,
        zoom_delta: f32,
    },
    MeshPreview {
        yaw_deg: f32,
        pitch_deg: f32,
        distance: f32,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CaptureBufferType {
    ColorRgb,
    DepthNormalized,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AppearanceOperation {
    Attach {
        item_id: Uuid,
        attach_point: Option<u8>,
        replace_existing: bool,
    },
    Detach {
        item_id: Uuid,
    },
    WearFolder {
        folder_id: Uuid,
        append: bool,
    },
    RemoveFolder {
        folder_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrimTransformSpec {
    pub local_id: u32,
    pub position: Option<[f32; 3]>,
    pub rotation_quat: Option<[f32; 4]>,
    pub rotation_euler_deg: Option<[f32; 3]>,
    pub scale: Option<[f32; 3]>,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LsdOperation {
    Write {
        key: String,
        value: String,
        passphrase: Option<String>,
    },
    Delete {
        key: String,
        passphrase: Option<String>,
    },
    Reset,
    ImportJson {
        entries: std::collections::BTreeMap<String, String>,
        passphrase: Option<String>,
    },
}

/// Requests sent from the MCP server to the viewer main thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ViewerRequest {
    // Phase 0: Core Status, Capabilities, Leases & Dynamic Calls
    ConnectionStatus,
    CapabilitiesRefresh,
    ApplyRollbacks {
        actions: Vec<RollbackAction>,
    },
    ViewerCall {
        api: String,
        operation: String,
        arguments: Value,
    },
    EventsSubscribe {
        source: String,
    },
    EventsUnsubscribe {
        source: String,
    },
    EventsRead {
        after: u64,
    },

    // Phase 1: Agent Telemetry, Radar, Comms, Navigation & RLVa
    GetAgentStatus,
    RadarScanAvatars {
        max_radius_m: f32,
        limit: usize,
    },
    RadarScanObjects {
        max_radius_m: f32,
        root_only: bool,
        name_filter: Option<String>,
        limit: usize,
    },
    CommsReadHistory {
        channel_filter: Option<String>,
        limit: usize,
    },
    CommsSend {
        message: String,
        channel: i32,
        chat_type: ChatKind,
        target_uuid: Option<Uuid>,
    },
    NavWalkTo {
        position: [f32; 3],
        allow_flying: bool,
        stop_distance: f32,
        timeout_ms: u64,
    },
    NavMovementStatus,
    NavStop,
    NavSit {
        target: Option<String>,
    },
    NavStand,
    NavTeleport {
        region_name: Option<String>,
        position: [f32; 3],
    },
    RlvaExecute {
        commands: Vec<String>,
    },

    // Phase 2: Settings, Camera, Poser & Floaters
    SettingsQuery {
        search: String,
        modified_only: bool,
        limit: usize,
    },
    SettingGet {
        group: String,
        key: String,
    },
    SettingSet {
        group: String,
        key: String,
        value: Value,
    },
    CameraControl {
        command: CameraCommand,
    },
    PoserInspect {
        subject: Option<Uuid>,
    },
    PoserSetBones {
        subject: Option<Uuid>,
        transforms: Vec<BoneTransform>,
    },
    PoserReset {
        subject: Option<Uuid>,
    },
    FloaterList,
    FloaterOpen {
        name: String,
        open: bool,
    },

    // Phase 3: Viewport Capture (Color & Depth)
    ViewportCapture {
        buffer_type: CaptureBufferType,
        include_ui: bool,
        include_hud: bool,
        max_dimensions: (u32, u32),
    },

    // Phase 4: Inventory & Appearance (COF Queue)
    InventoryBrowse {
        folder_id_or_path: String,
        depth: u8,
        limit: usize,
    },
    InventorySearch {
        folder_id: Option<String>,
        query: String,
        asset_type: Option<String>,
        worn_only: bool,
        limit: usize,
    },
    AppearanceGetWorn,
    AppearanceMutate {
        operation: AppearanceOperation,
    },

    // Phase 5: World Building, LSL, Linkset Data & Local Mesh
    PrimRezNew {
        position: [f32; 3],
        scale: [f32; 3],
        rotation_quat: [f32; 4],
        pcode: u8,
        name: Option<String>,
    },
    PrimTransform {
        transforms: Vec<PrimTransformSpec>,
    },
    PrimSetShape {
        local_id: u32,
        path_cut: Option<[f32; 2]>,
        profile_cut: Option<[f32; 2]>,
        hollow: Option<f32>,
        twist: Option<[f32; 2]>,
        color: Option<[f32; 4]>,
        texture_id: Option<Uuid>,
    },
    PrimLink {
        local_ids: Vec<u32>,
        delink: bool,
    },
    PrimDelete {
        local_ids: Vec<u32>,
    },
    LslInspectObjectScripts {
        object_local_id: u32,
    },
    LslReadScript {
        object_local_id: u32,
        script_item_id: Uuid,
    },
    LslWriteAndCompile {
        object_local_id: u32,
        script_item_id: Option<Uuid>,
        script_name: String,
        source_code: String,
        mono: bool,
        running: bool,
    },
    LslListenScriptOutput {
        object_id: Option<Uuid>,
        local_id: Option<u32>,
        duration_ms: u64,
    },
    LsdInspect {
        object_local_id: u32,
        filter_regex: Option<String>,
        include_protected: bool,
        passphrase: Option<String>,
        limit: usize,
    },
    LsdMutate {
        object_local_id: u32,
        operations: Vec<LsdOperation>,
    },
    LocalMeshStatus,
    LocalMeshOpen {
        open: bool,
    },
    LocalMeshAutoReload {
        enabled: bool,
    },

    // Phase 6: Suite 1 — Direct Asset Upload, LiveMesh, Inventory Rez & Materials
    LiveMeshControl {
        action: String,
        source_id: Option<Uuid>,
    },
    AssetUpload {
        action: String,
        paths: Vec<String>,
        choice: Option<String>,
        include_textures: Option<bool>,
    },
    InventoryRezObject {
        item_id: Uuid,
        position: [f32; 3],
        select: bool,
    },
    ObjectTakeOrDerez {
        local_id: u32,
        action: String,
        folder_id: Option<Uuid>,
    },
    PrimSetMaterial {
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
    },

    // Phase 6: Suite 2 — Remaining 13 Legacy Viewer MCP Tools (43/43 Parity)
    MeshUploadOpen,
    MeshUploadStatus,
    NativeFileDialogs,
    NativeFileChoose {
        dialog_id: i64,
        filename: String,
    },
    UiInspect {
        path: String,
    },
    UiFind {
        query: String,
        under: String,
        search_in: String,
        match_mode: String,
        max_depth: Option<usize>,
        include_info: bool,
        offset: usize,
        limit: usize,
    },
    UiClick {
        path: String,
        button: String,
        floater: Option<String>,
        observe_path: Option<String>,
    },
    UiGetValue {
        path: String,
    },
    UiSetText {
        path: String,
        text: String,
    },
    UiSelect {
        path: String,
        value: Value,
    },
    UiPressKey {
        path: String,
        keysym: String,
        modifiers: Vec<String>,
        observe_path: Option<String>,
    },
    UiListMenus {
        query: String,
    },
    UiInvokeMenu {
        name: String,
    },

    // Phase 6: Suite 3 — In-World Object Interaction, Notecards & Particle System
    ObjectTouch {
        local_id: u32,
        face: i32,
        uv: [f32; 2],
        st: [f32; 2],
        hold_ms: u64,
    },
    AvatarSit {
        action: String,
        target_local_id: Option<u32>,
        target_uuid: Option<Uuid>,
        offset: Option<[f32; 3]>,
    },
    NotecardRead {
        item_id: Uuid,
        object_local_id: Option<u32>,
    },
    NotecardWrite {
        item_id: Option<Uuid>,
        object_local_id: Option<u32>,
        name: Option<String>,
        body: String,
    },
    PrimSetParticles {
        local_id: u32,
        clear: bool,
        pattern: Option<String>,
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
    },

    // Autonomous Session & Login Management
    ViewerLogin {
        username: Option<String>,
        password: Option<String>,
        location: Option<String>,
        mfa_token: Option<String>,
    },
    ViewerShutdown,

    // Phase 7: Physics Flags, Metadata, Animations & Economy
    PrimSetPhysicsProperties {
        local_id: u32,
        physics: Option<bool>,
        temporary: Option<bool>,
        phantom: Option<bool>,
        shape_type: Option<String>,
        density: Option<f32>,
        friction: Option<f32>,
        restitution: Option<f32>,
        gravity_multiplier: Option<f32>,
    },
    PrimSetMetadata {
        local_id: u32,
        name: Option<String>,
        description: Option<String>,
    },
    AvatarAnimationPlay {
        animation: String,
        loop_anim: Option<bool>,
    },
    AvatarAnimationStop {
        animation: String,
    },
    AvatarAnimationsList,
    EconomyGetBalance,

    // Phase 8: Land, Parcel & Estate
    ParcelInfoQuery,
    ParcelSetMusic {
        music_url: String,
    },
    ParcelSetProperties {
        name: Option<String>,
        description: Option<String>,
        flags: Option<u32>,
    },
    ParcelAccessListModify {
        action: String,
        list: String,
        avatar_uuid: String,
    },
    ParcelEject {
        target_uuid: String,
        ban: Option<bool>,
    },

    // Phase 9: Economy & Social
    EconomyPayAvatar {
        target_uuid: String,
        amount: i32,
        message: Option<String>,
    },
    EconomyPayObject {
        target_local_id: u32,
        amount: i32,
        description: Option<String>,
    },
    GroupsList,
    GroupsActivate {
        group_uuid: String,
    },
    FriendsList {
        online_only: Option<bool>,
    },

    // Phase 10: Environment & EEP
    EnvironmentGet,
    EnvironmentSetTime {
        preset: Option<String>,
        time_of_day: Option<f32>,
    },
    EnvironmentApplyPreset {
        preset: Option<String>,
        sky_uuid: Option<String>,
        water_uuid: Option<String>,
    },

    // Phase 11: Spatial Voice Suite
    VoiceStatus,
    VoiceParticipantsList,
    VoiceSetUserVolume {
        avatar_uuid: String,
        volume: f32,
    },
    VoiceSetUserMute {
        avatar_uuid: String,
        mute: bool,
    },
    VoiceTransmit {
        transmitting: bool,
    },
}

/// Raw frame returned from the viewer/render pipeline before off-thread PNG encoding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCapturedFrame {
    pub width: u32,
    pub height: u32,
    pub buffer_type: CaptureBufferType,
    /// RGBA8 pixels for `ColorRgb`, or 8-bit linearized grayscale (or RGBA8) for `DepthNormalized`.
    pub rgba_bytes: Vec<u8>,
    pub near_clip: f32,
    pub far_clip: f32,
    pub camera_position: [f32; 3],
    pub camera_focus: [f32; 3],
    pub in_demo_mode: bool,
}

/// Responses sent back from the viewer main thread over the `oneshot` channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ViewerResponse {
    Json(Value),
    SettingMutated {
        key: String,
        previous_value: Value,
        current_value: Value,
    },
    CameraMutated {
        previous_camera: Option<RollbackAction>,
        state: Value,
    },
    PoserMutated {
        state: Value,
    },
    EnvironmentMutated {
        previous_time_of_day: u8,
        state: Value,
    },
    FrameCaptured(RawCapturedFrame),
}

impl ViewerRequest {
    /// Returns true if executing this request mutates viewer or world state and
    /// therefore requires an active control lease.
    pub fn requires_lease(&self) -> bool {
        match self {
            ViewerRequest::ConnectionStatus
            | ViewerRequest::CapabilitiesRefresh
            | ViewerRequest::ApplyRollbacks { .. }
            | ViewerRequest::EventsSubscribe { .. }
            | ViewerRequest::EventsUnsubscribe { .. }
            | ViewerRequest::EventsRead { .. }
            | ViewerRequest::GetAgentStatus
            | ViewerRequest::RadarScanAvatars { .. }
            | ViewerRequest::RadarScanObjects { .. }
            | ViewerRequest::CommsReadHistory { .. }
            | ViewerRequest::NavMovementStatus
            | ViewerRequest::SettingsQuery { .. }
            | ViewerRequest::SettingGet { .. }
            | ViewerRequest::PoserInspect { .. }
            | ViewerRequest::FloaterList
            | ViewerRequest::ViewportCapture { .. }
            | ViewerRequest::InventoryBrowse { .. }
            | ViewerRequest::InventorySearch { .. }
            | ViewerRequest::AppearanceGetWorn
            | ViewerRequest::LslInspectObjectScripts { .. }
            | ViewerRequest::LslReadScript { .. }
            | ViewerRequest::LslListenScriptOutput { .. }
            | ViewerRequest::LsdInspect { .. }
            | ViewerRequest::LocalMeshStatus
            | ViewerRequest::MeshUploadStatus
            | ViewerRequest::NativeFileDialogs
            | ViewerRequest::UiInspect { .. }
            | ViewerRequest::UiFind { .. }
            | ViewerRequest::UiGetValue { .. }
            | ViewerRequest::UiListMenus { .. }
            | ViewerRequest::NotecardRead { .. }
            | ViewerRequest::ViewerLogin { .. }
            | ViewerRequest::ViewerShutdown
            | ViewerRequest::AvatarAnimationsList
            | ViewerRequest::EconomyGetBalance
            | ViewerRequest::ParcelInfoQuery
            | ViewerRequest::GroupsList
            | ViewerRequest::FriendsList { .. }
            | ViewerRequest::EnvironmentGet
            | ViewerRequest::VoiceStatus
            | ViewerRequest::VoiceParticipantsList => false,

            ViewerRequest::LiveMeshControl { action, .. }
            | ViewerRequest::AssetUpload { action, .. } => {
                !action.eq_ignore_ascii_case("status")
            }

            ViewerRequest::ViewerCall { api, operation, .. } => {
                // Read-only LEAP / viewer queries do not require a lease.
                !is_readonly_viewer_call(api, operation)
            }

            ViewerRequest::CommsSend { .. }
            | ViewerRequest::NavWalkTo { .. }
            | ViewerRequest::NavStop
            | ViewerRequest::NavSit { .. }
            | ViewerRequest::NavStand
            | ViewerRequest::NavTeleport { .. }
            | ViewerRequest::RlvaExecute { .. }
            | ViewerRequest::SettingSet { .. }
            | ViewerRequest::CameraControl { .. }
            | ViewerRequest::PoserSetBones { .. }
            | ViewerRequest::PoserReset { .. }
            | ViewerRequest::FloaterOpen { .. }
            | ViewerRequest::AppearanceMutate { .. }
            | ViewerRequest::PrimRezNew { .. }
            | ViewerRequest::PrimTransform { .. }
            | ViewerRequest::PrimSetShape { .. }
            | ViewerRequest::PrimLink { .. }
            | ViewerRequest::PrimDelete { .. }
            | ViewerRequest::LslWriteAndCompile { .. }
            | ViewerRequest::LsdMutate { .. }
            | ViewerRequest::LocalMeshOpen { .. }
            | ViewerRequest::LocalMeshAutoReload { .. }
            | ViewerRequest::InventoryRezObject { .. }
            | ViewerRequest::ObjectTakeOrDerez { .. }
            | ViewerRequest::PrimSetMaterial { .. }
            | ViewerRequest::MeshUploadOpen
            | ViewerRequest::NativeFileChoose { .. }
            | ViewerRequest::UiClick { .. }
            | ViewerRequest::UiSetText { .. }
            | ViewerRequest::UiSelect { .. }
            | ViewerRequest::UiPressKey { .. }
            | ViewerRequest::UiInvokeMenu { .. }
            | ViewerRequest::ObjectTouch { .. }
            | ViewerRequest::AvatarSit { .. }
            | ViewerRequest::NotecardWrite { .. }
            | ViewerRequest::PrimSetParticles { .. }
            | ViewerRequest::PrimSetPhysicsProperties { .. }
            | ViewerRequest::PrimSetMetadata { .. }
            | ViewerRequest::AvatarAnimationPlay { .. }
            | ViewerRequest::AvatarAnimationStop { .. }
            | ViewerRequest::ParcelSetMusic { .. }
            | ViewerRequest::ParcelSetProperties { .. }
            | ViewerRequest::ParcelAccessListModify { .. }
            | ViewerRequest::ParcelEject { .. }
            | ViewerRequest::EconomyPayAvatar { .. }
            | ViewerRequest::EconomyPayObject { .. }
            | ViewerRequest::GroupsActivate { .. }
            | ViewerRequest::EnvironmentSetTime { .. }
            | ViewerRequest::EnvironmentApplyPreset { .. }
            | ViewerRequest::VoiceSetUserVolume { .. }
            | ViewerRequest::VoiceSetUserMute { .. }
            | ViewerRequest::VoiceTransmit { .. } => true,
        }
    }
}

fn is_readonly_viewer_call(api: &str, operation: &str) -> bool {
    let op = operation.to_ascii_lowercase();
    matches!(
        (api, op.as_str()),
        ("LLAppViewer", "getstatus" | "getversion")
            | ("LLViewerControl", "get" | "list")
            | ("LLAgent", "getposition" | "getstatus" | "getgroups")
            | ("LLWorld", "getobjects" | "getavatars" | "getregion")
            | ("LLInventory", "search" | "browse" | "getworn")
    ) || op.starts_with("get")
        || op.starts_with("list")
        || op.starts_with("read")
        || op.starts_with("inspect")
}

/// Creates a bounded channel pair connecting the Tokio MCP server to the viewer main thread.
pub fn create_bridge_channels() -> (
    mpsc::Sender<McpRequestEnvelope>,
    mpsc::Receiver<McpRequestEnvelope>,
) {
    mpsc::channel(BRIDGE_CHANNEL_CAPACITY)
}
