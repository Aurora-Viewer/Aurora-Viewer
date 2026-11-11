//! `AURORA_DEMO_CONTENTS_DROP=world|panel|tab[:ctrl][:drop]`: a selection of
//! four demo inventory entries (a notecard, a script, a sound and an object)
//! carried from frame 700, without a press. `world` puts the pointer on the
//! demo action object 976 (yours, modifiable; `AURORA_DEMO_ACTIONS`),
//! `panel` on the Contents list of the edit window
//! (`AURORA_DEMO_BUILD=move,object:976`, `AURORA_DEMO_BUILD_TAB=contents`).
//! `tab` rests on the Contents tab while General is showing
//! (`AURORA_DEMO_BUILD_TAB=general`) until the tab opens. `:ctrl` holds
//! Ctrl. `:drop` releases at frame 720, or at frame 780 for `tab` after the
//! pointer has moved onto the list.

use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Place {
    World,
    Panel,
    Tab,
}

#[derive(Clone, Copy)]
struct Mode {
    place: Place,
    ctrl: bool,
    drop: bool,
}

fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        if std::env::var("AURORA_DEMO").as_deref() != Ok("1") {
            return None;
        }
        let v = std::env::var("AURORA_DEMO_CONTENTS_DROP").ok()?;
        let mut parts = v.split(':');
        let place = match parts.next()? {
            "world" => Place::World,
            "panel" => Place::Panel,
            "tab" => Place::Tab,
            _ => return None,
        };
        let steps: Vec<_> = parts.collect();
        Some(Mode {
            place,
            ctrl: steps.contains(&"ctrl"),
            drop: steps.contains(&"drop"),
        })
    })
}

/// The demo inventory's instructions notecard, script, sound and object
/// (world/inventory/demo.rs).
const ITEMS: [u128; 4] = [8103, 8102, 8105, 8100];

impl App {
    pub(super) fn contents_drop_demo_input(&mut self, raw: &mut egui::RawInput, view_proj: glam::Mat4, size: (u32, u32)) {
        let Some(mode) = mode() else { return };
        let release = if mode.place == Place::Tab { 780 } else { 720 };
        if self.frame_count < 700 || mode.drop && self.frame_count > release {
            return;
        }
        let ctx = self.egui_ctx.clone();
        if self.frame_count == 700 {
            ui::inventory::drag::demo_carry(&ctx, ITEMS.iter().map(|&n| uuid::Uuid::from_u128(n)).collect());
        }
        let pos = if mode.place == Place::Panel || (mode.place == Place::Tab && mode.drop && self.frame_count >= 760) {
            crate::build::ui::contents::demo_zone(&ctx).map(|r| r.center())
        } else if mode.place == Place::Tab {
            crate::build::ui::demo_tab_rect(&ctx, 4).map(|r| r.center())
        } else {
            let projector = ui::hud::Projector {
                view_proj,
                width: size.0 as f32,
                height: size.1 as f32,
                ppp: ctx.pixels_per_point(),
            };
            self.world
                .objects
                .index_of_uuid(&crate::demo::action_id(976))
                .and_then(|idx| projector.project(self.scene.gpu.get(idx)?.center))
                .map(|(p, _)| p)
        };
        let Some(pos) = pos else { return };
        let modifiers = egui::Modifiers {
            ctrl: mode.ctrl,
            command: mode.ctrl,
            ..Default::default()
        };
        raw.focused = true;
        raw.events.push(egui::Event::ModifiersChanged(modifiers));
        raw.events.push(egui::Event::PointerMoved(pos));
        if mode.drop && self.frame_count == release {
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            });
        }
    }
}
