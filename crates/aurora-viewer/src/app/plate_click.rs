//! What the pointer does with a name plate (the human's requests of
//! 2026-10-14): the plate under it shows full ("Expand a plate on hover"),
//! and a left click on another avatar's plate runs the action chosen in
//! Preferences › Aurora › "Click on a plate", one of the entries of the
//! avatar's menu, with the hand cursor of a clickable object. Firestorm has
//! neither: a click on a name tag is a click on its avatar.
use super::*;
use crate::settings::PlateClick;
use crate::ui::context::{CtxAction, Target};
use uuid::Uuid;

/// Frame at which `AURORA_DEMO_PLATE_HOVER=x,y,click` clicks the plate.
const DEMO_CLICK_FRAME: u64 = 700;

/// `AURORA_DEMO_PLATE_HOVER=x,y[,click]`: the pointer held at x, y (points)
/// for the plates, and a click there at frame 700.
fn demo_pointer() -> Option<(egui::Pos2, bool)> {
    static POINTER: std::sync::OnceLock<Option<(egui::Pos2, bool)>> = std::sync::OnceLock::new();
    *POINTER.get_or_init(|| {
        let value = std::env::var("AURORA_DEMO_PLATE_HOVER").ok()?;
        let mut parts = value.split(',').map(str::trim);
        let x = parts.next()?.parse().ok()?;
        let y = parts.next()?.parse().ok()?;
        Some((egui::pos2(x, y), parts.next() == Some("click")))
    })
}

/// `AURORA_DEMO_PLATE_CLICK=profile|im|tp|menu`: the action for this run,
/// without saving it.
fn demo_action() -> Option<PlateClick> {
    static ACTION: std::sync::OnceLock<Option<PlateClick>> = std::sync::OnceLock::new();
    *ACTION.get_or_init(|| {
        Some(match std::env::var("AURORA_DEMO_PLATE_CLICK").ok()?.trim() {
            "profile" => PlateClick::Profile,
            "im" => PlateClick::InstantMessage,
            "tp" => PlateClick::OfferTeleport,
            "menu" => PlateClick::Menu,
            _ => PlateClick::Nothing,
        })
    })
}

/// The plate under `pointer`: the last drawn (the nearest) wins.
pub(super) fn plate_at(tags: &[ui::hud::NameTag], pointer: egui::Pos2) -> Option<Uuid> {
    tags.iter().rev().find(|t| t.rect.contains(pointer)).map(|t| t.id)
}

/// Whether a plate click acts, and so shows the hand: an action chosen, on
/// someone else's plate.
pub(super) fn plate_click_acts(action: PlateClick, plate: Option<Uuid>, me: Uuid) -> bool {
    action != PlateClick::Nothing && plate.is_some_and(|id| id != me)
}

/// The entry of the avatar's menu a plate click runs; the menu itself is
/// opened apart.
pub(super) fn plate_ctx_action(action: PlateClick, id: Uuid) -> Option<CtxAction> {
    match action {
        PlateClick::Nothing | PlateClick::Menu => None,
        PlateClick::Profile => Some(CtxAction::Profile(id)),
        PlateClick::InstantMessage => Some(CtxAction::Im(id)),
        PlateClick::OfferTeleport => Some(CtxAction::OfferTeleport(id)),
    }
}

impl App {
    /// The action of a click on a plate: the setting, or the demo's.
    pub(super) fn plate_click_action(&self) -> PlateClick {
        match demo_action() {
            Some(action) if self.demo => action,
            _ => self.settings.plate_click,
        }
    }

    /// Where the pointer stands for the plates, in points: nowhere over a
    /// window, while the camera or a drag holds the mouse, in mouselook or
    /// with the classic name tags.
    fn plate_pointer(&self, ctx: &egui::Context) -> Option<egui::Pos2> {
        if self.demo
            && let Some((pointer, _)) = demo_pointer()
        {
            return Some(pointer);
        }
        let free = matches!(self.screen, Screen::World)
            && self.mouse_mode == MouseMode::None
            && !self.camera.mouselook()
            && !self.cursor_hold.held()
            && self.hud_drag.is_none()
            && !ctx.is_pointer_over_egui()
            && !ctx.egui_wants_pointer_input();
        let ppp = ctx.pixels_per_point().max(0.1);
        free.then(|| egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp))
    }

    /// Before the plates are drawn: the one under the pointer, from the
    /// rectangles of the last frame, and the settings they follow.
    pub(super) fn update_plate_hover(&mut self, ctx: &egui::Context) {
        self.plate_hover = self.plate_pointer(ctx).and_then(|pointer| plate_at(&self.name_tags, pointer));
        self.plates.hovered = self.plate_hover;
        self.plates.expand_on_hover = self.settings.plates_hover_expand;
        if self.demo
            && self.frame_count == DEMO_CLICK_FRAME
            && demo_pointer().is_some_and(|(_, click)| click)
            && let Some(id) = self.plate_hover
        {
            log::info!("demo plate click: {id} ({:?})", self.plate_click_action());
            if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                self.run_plate_click(idx);
            }
        }
    }

    /// The hand of a clickable object over a plate that acts.
    pub(super) fn plate_cursor(&self) -> bool {
        !self.classic_name_tags() && plate_click_acts(self.plate_click_action(), self.plate_hover, self.world.agent_id)
    }

    /// A left press on the world: on another avatar's plate with an action
    /// chosen, the action runs and the world does not get the click. Alt,
    /// Ctrl and Shift keep their meaning (camera, grab).
    pub(super) fn plate_press(&mut self, hit: Option<Vec3>, ray: Option<(Vec3, Vec3)>) -> bool {
        if self.alt || self.ctrl || self.shift || self.classic_name_tags() {
            return false;
        }
        let action = self.plate_click_action();
        if action == PlateClick::Nothing {
            return false;
        }
        let Some((idx, _)) = self.pick_name_tag(hit, ray) else {
            return false;
        };
        let Some(id) = self.world.objects.get(idx).map(|o| o.full_id) else {
            return false;
        };
        if !plate_click_acts(action, Some(id), self.world.agent_id) {
            return false;
        }
        self.run_plate_click(idx);
        true
    }

    fn run_plate_click(&mut self, idx: usize) {
        let Some(id) = self.world.objects.get(idx).map(|o| o.full_id) else {
            return;
        };
        match self.plate_click_action() {
            PlateClick::Nothing => {}
            PlateClick::Menu => self.open_avatar_menu(idx),
            action => {
                if let Some(act) = plate_ctx_action(action, id) {
                    self.on_ctx_action(act);
                }
            }
        }
    }

    /// The avatar's menu at the pointer, as a right click on it opens it
    /// (`world_menu`).
    fn open_avatar_menu(&mut self, idx: usize) {
        let Some(o) = self.world.objects.get(idx) else {
            return;
        };
        let id = o.full_id;
        let point = Scene::object_transform(&self.world, idx, Instant::now(), 0).map_or(Vec3::ZERO, |(pos, _, _)| pos);
        self.build.release_menu_selection(&self.world);
        let ppp = self.egui_ctx.pixels_per_point().max(0.1);
        let pos = match demo_pointer() {
            Some((pointer, _)) if self.demo => pointer,
            _ => egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp),
        };
        self.world.ui_sounds.push(UiSound::PieMenuAppear);
        self.context_menu = Some(ui::context::ContextMenu {
            pos,
            point,
            target: Target::Avatar {
                id,
                own: id == self.world.agent_id,
                attachment: None,
            },
            pie_path: Vec::new(),
            pie_input_frame: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(n: u128, rect: egui::Rect) -> ui::hud::NameTag {
        ui::hud::NameTag {
            id: Uuid::from_u128(n),
            rect,
        }
    }

    #[test]
    fn the_nearest_plate_under_the_pointer_is_the_one() {
        let far = tag(1, egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 60.0)));
        let near = tag(2, egui::Rect::from_min_size(egui::pos2(150.0, 120.0), egui::vec2(200.0, 60.0)));
        let tags = [far, near];
        assert_eq!(plate_at(&tags, egui::pos2(160.0, 130.0)), Some(Uuid::from_u128(2)));
        assert_eq!(plate_at(&tags, egui::pos2(110.0, 105.0)), Some(Uuid::from_u128(1)));
        assert_eq!(plate_at(&tags, egui::pos2(10.0, 10.0)), None);
    }

    #[test]
    fn a_click_acts_only_with_an_action_and_on_someone_else() {
        let (me, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert!(!plate_click_acts(PlateClick::Nothing, Some(other), me));
        for action in PlateClick::ALL.into_iter().filter(|a| *a != PlateClick::Nothing) {
            assert!(plate_click_acts(action, Some(other), me), "{action:?}");
            // our own plate and no plate: the click is the world's
            assert!(!plate_click_acts(action, Some(me), me));
            assert!(!plate_click_acts(action, None, me));
        }
    }

    #[test]
    fn each_action_is_an_entry_of_the_avatar_menu() {
        let id = Uuid::from_u128(7);
        assert!(matches!(plate_ctx_action(PlateClick::Profile, id), Some(CtxAction::Profile(i)) if i == id));
        assert!(matches!(plate_ctx_action(PlateClick::InstantMessage, id), Some(CtxAction::Im(i)) if i == id));
        assert!(matches!(
            plate_ctx_action(PlateClick::OfferTeleport, id),
            Some(CtxAction::OfferTeleport(i)) if i == id
        ));
        // nothing runs nothing; the menu is opened apart
        assert!(plate_ctx_action(PlateClick::Nothing, id).is_none());
        assert!(plate_ctx_action(PlateClick::Menu, id).is_none());
    }
}
