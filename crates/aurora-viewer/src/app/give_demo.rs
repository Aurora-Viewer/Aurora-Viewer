//! `AURORA_DEMO_GIVE=profile|avatar[:over|:drop]` (#991): inventory carried
//! from frame 700 to a resident's profile window or to their avatar in the
//! world, without a press (the payload alone lasts until a release).
//! `profile` leaves the pointer beside the window (it fades behind the
//! hint), `:over` puts it on the window (outlined, the hint in ink) and
//! `:drop` releases it there at frame 720 (the Share confirmation). The
//! window is `AURORA_DEMO_PROFILE`'s. `avatar` puts the pointer on the
//! other avatar nearest the middle of the view (outlined); `:drop` releases
//! it there at frame 720. The entries are seven of `AURORA_DEMO_INVENTORY`'s.

use super::*;

/// The scenario: the target, and whether it is reached, released.
#[derive(Clone, Copy, PartialEq)]
struct Mode {
    avatar: bool,
    over: bool,
    drop: bool,
}

fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        if std::env::var("AURORA_DEMO").as_deref() != Ok("1") {
            return None;
        }
        let v = std::env::var("AURORA_DEMO_GIVE").ok()?;
        let (target, step) = v.split_once(':').unwrap_or((v.as_str(), ""));
        let avatar = match target {
            "profile" => false,
            "avatar" => true,
            _ => return None,
        };
        Some(Mode {
            avatar,
            over: avatar || matches!(step, "over" | "drop"),
            drop: step == "drop",
        })
    })
}

/// The entries carried: seven of the demo inventory's, its no-copy and
/// no-transfer objects among them (world/inventory/demo.rs).
const ITEMS: [u128; 7] = [8100, 8103, 8109, 8110, 8111, 8104, 8105];

impl App {
    pub(super) fn give_demo_input(&mut self, raw: &mut egui::RawInput, view_proj: glam::Mat4, size: (u32, u32)) {
        let Some(mode) = mode() else { return };
        if self.frame_count < 700 || mode.drop && self.frame_count > 720 {
            return;
        }
        let ctx = self.egui_ctx.clone();
        if self.frame_count == 700 {
            ui::inventory::drag::demo_carry(&ctx, ITEMS.iter().map(|&n| uuid::Uuid::from_u128(n)).collect());
        }
        let pos = if mode.avatar {
            let projector = ui::hud::Projector {
                view_proj,
                width: size.0 as f32,
                height: size.1 as f32,
                ppp: ctx.pixels_per_point(),
            };
            let middle = ctx.content_rect().center();
            self.world
                .objects
                .avatars()
                .filter(|(_, o)| o.full_id != self.world.agent_id)
                .filter_map(|(idx, _)| projector.project(self.scene.gpu.get(idx)?.center))
                .map(|(p, _)| p)
                .min_by(|a, b| a.distance(middle).total_cmp(&b.distance(middle)))
        } else {
            self.profile_ui.demo_window_rect(&ctx).map(|r| {
                if mode.over {
                    r.center()
                } else {
                    // beside the window, inside the screen
                    let screen = ctx.content_rect();
                    if r.left() - screen.left() > screen.right() - r.right() {
                        egui::pos2((screen.left() + r.left()) / 2.0, r.center().y)
                    } else {
                        egui::pos2((r.right() + screen.right()) / 2.0, r.center().y)
                    }
                }
            })
        };
        let Some(pos) = pos else { return };
        raw.focused = true;
        raw.events.push(egui::Event::PointerMoved(pos));
        if mode.drop && self.frame_count == 720 {
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            });
        }
    }
}
