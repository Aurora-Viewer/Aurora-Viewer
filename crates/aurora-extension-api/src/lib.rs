//! Versioned, serialization-stable messages exchanged between Aurora and extensions.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Current serialized extension API version.
pub const API_VERSION: u32 = 1;
/// Maximum JSON envelope size accepted by the host.
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024;
/// Maximum actions accepted in a single extension response.
pub const MAX_RESPONSE_ACTIONS: usize = 16;
/// Maximum UTF-8 bytes in a guest-provided UI string.
pub const MAX_UI_TEXT_BYTES: usize = 2 * 1024;
/// Maximum UTF-8 bytes in a sanitized nearby-chat event.
pub const MAX_CHAT_TEXT_BYTES: usize = 4 * 1024;
/// Maximum controls a plugin can declare for one panel.
pub const MAX_PANEL_CONTROLS: usize = 16;
/// Maximum extension-owned values restored with one request.
pub const MAX_STORAGE_ENTRIES: usize = 32;
/// Maximum key size in extension-owned storage.
pub const MAX_STORAGE_KEY_BYTES: usize = 64;
/// Maximum value size in extension-owned storage.
pub const MAX_STORAGE_VALUE_BYTES: usize = 1024;

/// A v1 capability name used in a package manifest and an explicit user grant.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read only accepted, sanitized nearby resident chat events.
    ChatNearbyRead,
    /// Add bounded, separate annotations to a matching nearby-chat event.
    UiChatAnnotate,
    /// Show bounded host-rendered Aurora notification cards.
    UiNotificationShow,
    /// Register declarative, host-rendered panels and controls.
    UiPanelRegister,
    /// Read and write bounded, account/grid-scoped extension-owned values.
    ExtensionStorage,
    /// High-risk grant for every host-mediated capability currently defined by API v1.
    /// This fixed snapshot does not grant future permission variants automatically.
    FullAccess,
}

impl Permission {
    /// Return the stable manifest key for this permission.
    pub const fn manifest_key(self) -> &'static str {
        match self {
            Self::ChatNearbyRead => "chat.nearby.read",
            Self::UiChatAnnotate => "ui.chat.annotate",
            Self::UiNotificationShow => "ui.notification.show",
            Self::UiPanelRegister => "ui.panel.register",
            Self::ExtensionStorage => "extension.storage",
            Self::FullAccess => "extension.full_access",
        }
    }

    /// Parse one exact, case-sensitive v1 manifest permission key.
    pub fn from_manifest_key(key: &str) -> Option<Self> {
        match key {
            "chat.nearby.read" => Some(Self::ChatNearbyRead),
            "ui.chat.annotate" => Some(Self::UiChatAnnotate),
            "ui.notification.show" => Some(Self::UiNotificationShow),
            "ui.panel.register" => Some(Self::UiPanelRegister),
            "extension.storage" => Some(Self::ExtensionStorage),
            "extension.full_access" => Some(Self::FullAccess),
            _ => None,
        }
    }

    /// Whether this grant covers a required capability.
    ///
    /// `FullAccess` is intentionally a fixed API v1 snapshot. New permission
    /// variants require explicit consent and are not covered by it by default.
    pub fn covers(self, required: Self) -> bool {
        self == required
            || matches!(
                (self, required),
                (Self::FullAccess, Self::ChatNearbyRead)
                    | (Self::FullAccess, Self::UiChatAnnotate)
                    | (Self::FullAccess, Self::UiNotificationShow)
                    | (Self::FullAccess, Self::UiPanelRegister)
            )
    }
}

/// One versioned request delivered to an extension.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionRequest {
    /// Must equal [`API_VERSION`].
    pub api_version: u32,
    /// Host-generated correlation ID; the response must echo it.
    pub request_id: u64,
    /// Lifecycle or sanitized viewer event.
    pub event: ExtensionEvent,
    /// Present only when the host granted `extension.storage`; omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<Vec<ExtensionStorageEntry>>,
}

/// One plain-text value in this extension's isolated persistent storage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionStorageEntry {
    /// Stable key chosen by the extension.
    pub key: String,
    /// Plain-text value owned only by this extension.
    pub value: String,
}

/// Events available to extensions in API v1.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtensionEvent {
    /// Called when the extension is enabled. Carries no account identifier or secret.
    Initialize {
        /// The host's supported API version, encoded as a short string.
        host_api_version: String,
    },
    /// Accepted nearby chat only; the host must check `chat.nearby.read` first.
    NearbyChat {
        /// Opaque host identifier used only to match this event's annotation.
        message_id: u64,
        /// Already-filtered display name; no account or object UUID is exposed.
        speaker: String,
        /// Sanitized text, bounded by [`MAX_CHAT_TEXT_BYTES`].
        text: String,
    },
    /// A user-operated event from a host-rendered extension panel.
    PanelAction {
        /// Stable panel identifier returned in a prior registration action.
        panel_id: String,
        /// Stable control identifier returned in a prior registration action.
        control_id: String,
        /// The control's value; button actions use `clicked`.
        value: PanelEventValue,
    },

    /// Called before the host disables an extension when the module is healthy.
    Shutdown,
}

/// A value from a host-rendered panel control.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelEventValue {
    /// A button was activated.
    Clicked,
    /// A toggle changed to this value.
    Toggled {
        /// Current toggle value.
        value: bool,
    },
}

/// One versioned response from an extension.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionResponse {
    /// Must equal [`API_VERSION`].
    pub api_version: u32,
    /// Must match the request's correlation ID.
    pub request_id: u64,
    /// Bounded host effects; all require the corresponding user-granted capability.
    pub actions: Vec<ExtensionAction>,
}

/// A declarative action that the host must authorize before applying.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExtensionAction {
    /// Annotate only the nearby-chat message that caused the current event.
    AnnotateChat {
        /// Must exactly match the incoming event's message ID.
        message_id: u64,
        /// Plain text rendered separately from the original line.
        text: String,
    },
    /// Ask the host to display a native in-viewer notification card.
    ShowNotification {
        /// Short title.
        title: String,
        /// Plain-text body.
        body: String,
    },
    /// Register a host-rendered panel during initialization or refresh it after one of its own actions.
    RegisterPanel {
        /// Stable ASCII identifier unique to this extension.
        panel_id: String,
        /// Display title.
        title: String,
        /// Bounded declarative controls; no HTML, JavaScript or arbitrary layout.
        controls: Vec<PanelControl>,
    },
    /// Set or remove one bounded extension-owned value (`None` removes the key).
    SetStorage {
        /// Stable key.
        key: String,
        /// New value, or `None` to remove it.
        value: Option<String>,
    },
}

/// One host-rendered, non-executable panel control.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelControl {
    /// Static text label.
    Label {
        /// Stable ASCII identifier.
        id: String,
        /// Plain text.
        text: String,
    },
    /// Boolean setting control.
    Toggle {
        /// Stable ASCII identifier.
        id: String,
        /// Plain-text label.
        label: String,
        /// Initial value for this panel registration.
        value: bool,
    },
    /// A button that produces a [`ExtensionEvent::PanelAction`].
    Button {
        /// Stable ASCII identifier.
        id: String,
        /// Plain-text label.
        label: String,
    },
}

/// A bounded protocol validation error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// API version is unsupported.
    UnsupportedVersion,
    /// Request fields exceed their bounds or contain display controls.
    InvalidRequest,
    /// Response correlation or action is invalid.
    InvalidResponse,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedVersion => "unsupported extension API version",
            Self::InvalidRequest => "invalid or oversized extension request",
            Self::InvalidResponse => "invalid or unauthorized extension response",
        })
    }
}

impl std::error::Error for ProtocolError {}

impl ExtensionRequest {
    /// Validate version, size limits and fields before serializing or dispatching.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.api_version != API_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        match &self.event {
            ExtensionEvent::Initialize { host_api_version } => validate_text(host_api_version, 32)?,
            ExtensionEvent::NearbyChat { speaker, text, .. } => {
                validate_text(speaker, 256)?;
                validate_text(text, MAX_CHAT_TEXT_BYTES)?;
            }
            ExtensionEvent::PanelAction { panel_id, control_id, .. } => {
                validate_id(panel_id)?;
                validate_id(control_id)?;
            }

            ExtensionEvent::Shutdown => {}
        }
        if let Some(entries) = &self.storage {
            if entries.len() > MAX_STORAGE_ENTRIES {
                return Err(ProtocolError::InvalidRequest);
            }
            let mut keys = HashSet::new();
            for entry in entries {
                validate_storage_key(&entry.key)?;
                validate_text(&entry.value, MAX_STORAGE_VALUE_BYTES)?;
                if !keys.insert(&entry.key) {
                    return Err(ProtocolError::InvalidRequest);
                }
            }
        }
        if serde_json::to_vec(self).map_err(|_| ProtocolError::InvalidRequest)?.len() > MAX_ENVELOPE_BYTES {
            return Err(ProtocolError::InvalidRequest);
        }
        Ok(())
    }
}

impl ExtensionResponse {
    /// Validate a guest response and bind chat annotations to the current event.
    pub fn validate_for(&self, request: &ExtensionRequest) -> Result<(), ProtocolError> {
        request.validate()?;
        if self.api_version != API_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        if self.request_id != request.request_id || self.actions.len() > MAX_RESPONSE_ACTIONS {
            return Err(ProtocolError::InvalidResponse);
        }
        let mut panel_ids = HashSet::new();
        for action in &self.actions {
            match action {
                ExtensionAction::AnnotateChat { message_id, text } => {
                    if !matches!(request.event, ExtensionEvent::NearbyChat { message_id: event_id, .. } if event_id == *message_id) {
                        return Err(ProtocolError::InvalidResponse);
                    }
                    validate_text(text, MAX_UI_TEXT_BYTES).map_err(|_| ProtocolError::InvalidResponse)?;
                }
                ExtensionAction::ShowNotification { title, body } => {
                    validate_text(title, 128).map_err(|_| ProtocolError::InvalidResponse)?;
                    validate_text(body, MAX_UI_TEXT_BYTES).map_err(|_| ProtocolError::InvalidResponse)?;
                }
                ExtensionAction::RegisterPanel { panel_id, title, controls } => {
                    if !panel_registration_allowed(&request.event, panel_id)
                        || !panel_ids.insert(panel_id)
                        || controls.len() > MAX_PANEL_CONTROLS
                    {
                        return Err(ProtocolError::InvalidResponse);
                    }
                    validate_id(panel_id).map_err(|_| ProtocolError::InvalidResponse)?;
                    validate_text(title, 128).map_err(|_| ProtocolError::InvalidResponse)?;
                    let mut control_ids = HashSet::new();
                    for control in controls {
                        let (id, text) = match control {
                            PanelControl::Label { id, text } => (id, text),
                            PanelControl::Toggle { id, label, .. } | PanelControl::Button { id, label } => (id, label),
                        };
                        validate_id(id).map_err(|_| ProtocolError::InvalidResponse)?;
                        validate_text(text, 256).map_err(|_| ProtocolError::InvalidResponse)?;
                        if !control_ids.insert(id) {
                            return Err(ProtocolError::InvalidResponse);
                        }
                    }
                }
                ExtensionAction::SetStorage { key, value } => {
                    validate_storage_key(key).map_err(|_| ProtocolError::InvalidResponse)?;
                    if let Some(value) = value {
                        validate_text(value, MAX_STORAGE_VALUE_BYTES).map_err(|_| ProtocolError::InvalidResponse)?;
                    }
                }
            }
        }
        if serde_json::to_vec(self).map_err(|_| ProtocolError::InvalidResponse)?.len() > MAX_ENVELOPE_BYTES {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
}

fn panel_registration_allowed(event: &ExtensionEvent, panel_id: &str) -> bool {
    match event {
        ExtensionEvent::Initialize { .. } => true,
        ExtensionEvent::PanelAction {
            panel_id: event_panel_id, ..
        } => event_panel_id == panel_id,
        _ => false,
    }
}

fn validate_storage_key(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > MAX_STORAGE_KEY_BYTES
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ProtocolError::InvalidRequest);
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), ProtocolError> {
    if value.is_empty()
        || value.len() > 64
        || !value.is_ascii()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_')
    {
        return Err(ProtocolError::InvalidRequest);
    }
    Ok(())
}

fn validate_text(value: &str, max_bytes: usize) -> Result<(), ProtocolError> {
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(ProtocolError::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_response_round_trip_versioned_envelopes() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 42,
            event: ExtensionEvent::NearbyChat {
                message_id: 7,
                speaker: "Nova Example".into(),
                text: "hello".into(),
            },
            storage: None,
        };
        let encoded = serde_json::to_vec(&request).expect("serialize request");
        let decoded: ExtensionRequest = serde_json::from_slice(&encoded).expect("deserialize request");
        assert_eq!(decoded, request);
        decoded.validate().expect("valid request");

        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 42,
            actions: vec![ExtensionAction::AnnotateChat {
                message_id: 7,
                text: "greeting".into(),
            }],
        };
        response.validate_for(&decoded).expect("authorized-to-apply response");
    }

    #[test]
    fn rejects_unknown_permissions_and_unknown_protocol_fields() {
        assert_eq!(Permission::from_manifest_key("ui.chat.annotate"), Some(Permission::UiChatAnnotate));
        assert_eq!(Permission::from_manifest_key("extension.full_access"), Some(Permission::FullAccess));
        assert_eq!(Permission::from_manifest_key("network.fetch"), None);
        let json = br#"{"api_version":1,"request_id":1,"event":{"type":"shutdown"},"extra":true}"#;
        assert!(serde_json::from_slice::<ExtensionRequest>(json).is_err());
    }

    #[test]
    fn full_access_covers_only_the_current_api_v1_permissions() {
        let full_access = Permission::FullAccess;
        assert_eq!(full_access.manifest_key(), "extension.full_access");
        for permission in [
            Permission::ChatNearbyRead,
            Permission::UiChatAnnotate,
            Permission::UiNotificationShow,
            Permission::UiPanelRegister,
        ] {
            assert!(full_access.covers(permission));
        }
        assert!(!Permission::ChatNearbyRead.covers(Permission::UiChatAnnotate));
        assert!(!Permission::UiPanelRegister.covers(Permission::FullAccess));
    }

    #[test]
    fn response_cannot_annotate_an_unrelated_or_non_chat_event() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 9,
            event: ExtensionEvent::Shutdown,
            storage: None,
        };
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 9,
            actions: vec![ExtensionAction::AnnotateChat {
                message_id: 1,
                text: "unrelated".into(),
            }],
        };
        assert_eq!(response.validate_for(&request), Err(ProtocolError::InvalidResponse));
    }

    #[test]
    fn rejects_display_controls_oversize_and_duplicate_panel_controls() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 1,
            event: ExtensionEvent::Shutdown,
            storage: None,
        };
        let control = PanelControl::Toggle {
            id: "enabled".into(),
            label: "Enabled".into(),
            value: true,
        };
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 1,
            actions: vec![ExtensionAction::ShowNotification {
                title: "bad\nheading".into(),
                body: "body".into(),
            }],
        };
        assert_eq!(response.validate_for(&request), Err(ProtocolError::InvalidResponse));

        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 2,
            event: ExtensionEvent::Initialize {
                host_api_version: "1.0.0".into(),
            },
            storage: None,
        };
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 2,
            actions: vec![ExtensionAction::RegisterPanel {
                panel_id: "main".into(),
                title: "Settings".into(),
                controls: vec![control.clone(), control],
            }],
        };
        assert_eq!(response.validate_for(&request), Err(ProtocolError::InvalidResponse));
    }

    #[test]
    fn panel_action_can_refresh_only_the_panel_that_emitted_it() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 17,
            event: ExtensionEvent::PanelAction {
                panel_id: "settings".into(),
                control_id: "enabled".into(),
                value: PanelEventValue::Toggled { value: true },
            },
            storage: None,
        };
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 17,
            actions: vec![ExtensionAction::RegisterPanel {
                panel_id: "settings".into(),
                title: "Settings".into(),
                controls: vec![PanelControl::Toggle {
                    id: "enabled".into(),
                    label: "Enabled".into(),
                    value: true,
                }],
            }],
        };
        response.validate_for(&request).expect("same-panel refresh is valid");

        let mismatched = ExtensionResponse {
            actions: vec![ExtensionAction::RegisterPanel {
                panel_id: "other".into(),
                title: "Other".into(),
                controls: vec![],
            }],
            ..response
        };
        assert_eq!(mismatched.validate_for(&request), Err(ProtocolError::InvalidResponse));
    }

    #[test]
    fn extension_storage_is_opt_in_bounded_and_not_in_full_access() {
        assert_eq!(
            Permission::from_manifest_key("extension.storage"),
            Some(Permission::ExtensionStorage)
        );
        assert!(!Permission::FullAccess.covers(Permission::ExtensionStorage));

        let request_without_storage = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 1,
            event: ExtensionEvent::Shutdown,
            storage: None,
        };
        let legacy_json = serde_json::to_string(&request_without_storage).expect("serialize legacy request");
        assert!(!legacy_json.contains("storage"));

        let request = ExtensionRequest {
            request_id: 2,
            storage: Some(vec![ExtensionStorageEntry {
                key: "ui.enabled".into(),
                value: "true".into(),
            }]),
            ..request_without_storage.clone()
        };
        request.validate().expect("bounded storage snapshot");
        let response = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 2,
            actions: vec![ExtensionAction::SetStorage {
                key: "ui.enabled".into(),
                value: Some("false".into()),
            }],
        };
        response.validate_for(&request).expect("bounded storage update");
    }

    #[test]
    fn storage_rejects_invalid_keys_values_duplicate_keys_and_excess_entries() {
        let base = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 1,
            event: ExtensionEvent::Shutdown,
            storage: None,
        };
        let invalid_key = ExtensionRequest {
            storage: Some(vec![ExtensionStorageEntry {
                key: "Bad Key".into(),
                value: "value".into(),
            }]),
            ..base.clone()
        };
        assert_eq!(invalid_key.validate(), Err(ProtocolError::InvalidRequest));

        let duplicate_key = ExtensionRequest {
            storage: Some(vec![
                ExtensionStorageEntry {
                    key: "key".into(),
                    value: "one".into(),
                },
                ExtensionStorageEntry {
                    key: "key".into(),
                    value: "two".into(),
                },
            ]),
            ..base.clone()
        };
        assert_eq!(duplicate_key.validate(), Err(ProtocolError::InvalidRequest));

        let too_many = ExtensionRequest {
            storage: Some(
                (0..=MAX_STORAGE_ENTRIES)
                    .map(|index| ExtensionStorageEntry {
                        key: format!("key-{index}"),
                        value: "value".into(),
                    })
                    .collect(),
            ),
            ..base.clone()
        };
        assert_eq!(too_many.validate(), Err(ProtocolError::InvalidRequest));

        let maximum_entry = ExtensionRequest {
            storage: Some(vec![ExtensionStorageEntry {
                key: "k".repeat(MAX_STORAGE_KEY_BYTES),
                value: "v".repeat(MAX_STORAGE_VALUE_BYTES),
            }]),
            ..base.clone()
        };
        maximum_entry.validate().expect("maximum key and value sizes are accepted");

        let oversized_key = ExtensionRequest {
            storage: Some(vec![ExtensionStorageEntry {
                key: "k".repeat(MAX_STORAGE_KEY_BYTES + 1),
                value: "value".into(),
            }]),
            ..base.clone()
        };
        assert_eq!(oversized_key.validate(), Err(ProtocolError::InvalidRequest));

        let oversized_value = ExtensionRequest {
            storage: Some(vec![ExtensionStorageEntry {
                key: "key".into(),
                value: "v".repeat(MAX_STORAGE_VALUE_BYTES + 1),
            }]),
            ..base.clone()
        };
        assert_eq!(oversized_value.validate(), Err(ProtocolError::InvalidRequest));

        let invalid_value = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 1,
            actions: vec![ExtensionAction::SetStorage {
                key: "key".into(),
                value: Some("bad\nvalue".into()),
            }],
        };
        assert_eq!(invalid_value.validate_for(&base), Err(ProtocolError::InvalidResponse));

        let oversized_action = ExtensionResponse {
            api_version: API_VERSION,
            request_id: 1,
            actions: vec![ExtensionAction::SetStorage {
                key: "key".into(),
                value: Some("v".repeat(MAX_STORAGE_VALUE_BYTES + 1)),
            }],
        };
        assert_eq!(oversized_action.validate_for(&base), Err(ProtocolError::InvalidResponse));
    }
}
