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
}

impl Permission {
    /// Return the stable manifest key for this permission.
    pub const fn manifest_key(self) -> &'static str {
        match self {
            Self::ChatNearbyRead => "chat.nearby.read",
            Self::UiChatAnnotate => "ui.chat.annotate",
            Self::UiNotificationShow => "ui.notification.show",
            Self::UiPanelRegister => "ui.panel.register",
        }
    }

    /// Parse one exact, case-sensitive v1 manifest permission key.
    pub fn from_manifest_key(key: &str) -> Option<Self> {
        match key {
            "chat.nearby.read" => Some(Self::ChatNearbyRead),
            "ui.chat.annotate" => Some(Self::UiChatAnnotate),
            "ui.notification.show" => Some(Self::UiNotificationShow),
            "ui.panel.register" => Some(Self::UiPanelRegister),
            _ => None,
        }
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
    /// Request a small panel rendered entirely by Aurora's native UI.
    RegisterPanel {
        /// Stable ASCII identifier unique to this extension.
        panel_id: String,
        /// Display title.
        title: String,
        /// Bounded declarative controls; no HTML, JavaScript or arbitrary layout.
        controls: Vec<PanelControl>,
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
                    if !matches!(request.event, ExtensionEvent::Initialize { .. })
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
            }
        }
        if serde_json::to_vec(self).map_err(|_| ProtocolError::InvalidResponse)?.len() > MAX_ENVELOPE_BYTES {
            return Err(ProtocolError::InvalidResponse);
        }
        Ok(())
    }
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
        assert_eq!(Permission::from_manifest_key("network.fetch"), None);
        let json = br#"{"api_version":1,"request_id":1,"event":{"type":"shutdown"},"extra":true}"#;
        assert!(serde_json::from_slice::<ExtensionRequest>(json).is_err());
    }

    #[test]
    fn response_cannot_annotate_an_unrelated_or_non_chat_event() {
        let request = ExtensionRequest {
            api_version: API_VERSION,
            request_id: 9,
            event: ExtensionEvent::Shutdown,
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
}
