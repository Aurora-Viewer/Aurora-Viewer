#![cfg_attr(target_arch = "wasm32", no_main)]

use aurora_extension_sdk::{ExtensionAction, ExtensionEvent, ExtensionHandler, PanelControl, PanelEventValue, export_extension};

#[derive(Default)]
struct SandboxDemo;

impl ExtensionHandler for SandboxDemo {
    fn on_event(&mut self, event: &ExtensionEvent) -> Vec<ExtensionAction> {
        match event {
            ExtensionEvent::Initialize { .. } => vec![ExtensionAction::RegisterPanel {
                panel_id: "sandbox-demo".into(),
                title: "Sandbox Demo".into(),
                controls: vec![
                    PanelControl::Label {
                        id: "offline-status".into(),
                        text: "Offline fixture: the WASM guest initialized.".into(),
                    },
                    PanelControl::Button {
                        id: "notify".into(),
                        label: "Show demo notification".into(),
                    },
                ],
            }],
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
}

export_extension!(SandboxDemo);

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
