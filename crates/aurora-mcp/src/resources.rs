//! MCP `resources/list`, `resources/templates/list`, and subscription routing (`C7`).

use serde_json::{Value, json};

pub const STATIC_RESOURCE_URIS: &[&str] = &[
    "aurora://status",
    "aurora://capabilities",
    "aurora://chat/recent",
    "aurora://events/recent",
    "aurora://audit/recent",
];

pub fn list_resources() -> Value {
    json!({
        "resources": [
            {
                "uri": "aurora://status",
                "name": "Viewer Connection & Agent Status",
                "description": "Live connection state, active lease owner, agent position, region, FPS, and login status.",
                "mimeType": "application/json"
            },
            {
                "uri": "aurora://capabilities",
                "name": "MCP Capabilities & Remaining Budgets",
                "description": "Active policy mode, remaining L$/upload budgets, rate-limit tokens, RLVa state, and parcel rights.",
                "mimeType": "application/json"
            },
            {
                "uri": "aurora://chat/recent",
                "name": "Recent Chat & Notices",
                "description": "Recent local chat, IMs, and simulator notices (tagged untrusted_world_content).",
                "mimeType": "application/json"
            },
            {
                "uri": "aurora://events/recent",
                "name": "Recent Session Events",
                "description": "Buffered viewer events matching this session's subscriptions.",
                "mimeType": "application/json"
            },
            {
                "uri": "aurora://audit/recent",
                "name": "Recent MCP Audit Log",
                "description": "Recent mutating/policy-gated tool invocations with SHA-256 argument hashes.",
                "mimeType": "application/json"
            }
        ]
    })
}

pub fn list_resource_templates() -> Value {
    json!({
        "resourceTemplates": [
            {
                "uriTemplate": "aurora://inventory/{folder}",
                "name": "Inventory Folder Listing",
                "description": "Browse an agent inventory folder by UUID or path (e.g. aurora://inventory/Objects).",
                "mimeType": "application/json"
            },
            {
                "uriTemplate": "aurora://capture/{id}",
                "name": "Capture Orbit Manifest",
                "description": "Read a capture_orbit manifest inside the capture directory by manifest ID.",
                "mimeType": "application/json"
            }
        ]
    })
}

pub fn is_subscribable_uri(uri: &str) -> bool {
    STATIC_RESOURCE_URIS.contains(&uri)
}

/// Maps an [`EventBus`] event source to matching subscribed resource URIs for
/// `notifications/resources/updated`.
pub fn uris_for_event_source(source: &str) -> Vec<&'static str> {
    match source.trim().to_ascii_lowercase().as_str() {
        "chat" | "im" => vec!["aurora://chat/recent", "aurora://events/recent"],
        "region" | "teleport" | "lease" => vec!["aurora://status", "aurora://capabilities", "aurora://events/recent"],
        "audit" => vec!["aurora://audit/recent", "aurora://events/recent"],
        _ => vec!["aurora://events/recent"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_and_templates_are_well_formed() {
        let res = list_resources();
        assert_eq!(res["resources"].as_array().unwrap().len(), STATIC_RESOURCE_URIS.len());
        assert_eq!(list_resource_templates()["resourceTemplates"].as_array().unwrap().len(), 2);
        assert!(is_subscribable_uri("aurora://status"));
        assert!(!is_subscribable_uri("aurora://unknown"));
        assert!(uris_for_event_source("chat").contains(&"aurora://chat/recent"));
        assert!(uris_for_event_source("audit").contains(&"aurora://audit/recent"));
    }
}
