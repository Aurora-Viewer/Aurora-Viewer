#![cfg_attr(target_arch = "wasm32", no_main)]

use aurora_extension_sdk::{ExtensionAction, ExtensionEvent, ExtensionHandler, PanelControl, PanelEventValue, export_extension};

#[derive(Default)]
struct SandboxDemo;

impl ExtensionHandler for SandboxDemo {
    fn on_event(&mut self, event: &ExtensionEvent) -> Vec<ExtensionAction> {
        handle_event(event, false)
    }

    fn on_request(&mut self, request: &aurora_extension_sdk::ExtensionRequest) -> Vec<ExtensionAction> {
        let enabled = request
            .storage
            .as_ref()
            .and_then(|entries| entries.iter().find(|entry| entry.key == "ui.enabled"))
            .is_some_and(|entry| entry.value == "true");
        match &request.event {
            ExtensionEvent::PanelAction {
                panel_id,
                control_id,
                value: PanelEventValue::Toggled { value },
            } if panel_id == "sandbox-demo" && control_id == "enabled" => vec![
                ExtensionAction::SetStorage {
                    key: "ui.enabled".into(),
                    value: Some(value.to_string()),
                },
                register_panel(*value),
            ],
            event => handle_event(event, enabled),
        }
    }
}

fn handle_event(event: &ExtensionEvent, enabled: bool) -> Vec<ExtensionAction> {
    match event {
        ExtensionEvent::Initialize { .. } => vec![register_panel(enabled)],
        ExtensionEvent::PanelAction {
            panel_id,
            control_id,
            value: PanelEventValue::Clicked,
        } if panel_id == "sandbox-demo" && control_id == "notify" => {
            vec![ExtensionAction::ShowNotification {
                title: "Sandbox demo".into(),
                body: "The button event was handled by the WASM guest.".into(),
            }]
        }
        _ => Vec::new(),
    }
}

fn register_panel(enabled: bool) -> ExtensionAction {
    ExtensionAction::RegisterPanel {
        panel_id: "sandbox-demo".into(),
        title: "Sandbox Demo".into(),
        controls: vec![
            PanelControl::Label {
                id: "offline-status".into(),
                text: "Offline fixture: the WASM guest initialized.".into(),
            },
            PanelControl::Toggle {
                id: "enabled".into(),
                label: "Demo feature enabled".into(),
                value: enabled,
            },
            PanelControl::Button {
                id: "notify".into(),
                label: "Show demo notification".into(),
            },
        ],
    }
}

export_extension!(SandboxDemo);

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
