//! Name plates expand on hover and offer independent plain, Shift and Ctrl
//! left-click actions (Preferences › Avatars). These are deliberate Aurora
//! additions; the Legacy Viewer treats a name-tag click as an avatar click.
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

/// `AURORA_DEMO_PLATE_JIGGLE=1`: jiggly plates for this run (not saved),
/// the pointer of `AURORA_DEMO_PLATE_HOVER` going in and out of its plate
/// every `DEMO_JIGGLE_FRAMES` frames from frame 600 to 700.
fn demo_jiggle() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_DEMO_PLATE_JIGGLE").is_ok_and(|v| v.trim() == "1"))
}
const DEMO_JIGGLE_FRAMES: u64 = 8;

/// `AURORA_DEMO_PLATE_COMBO=<clicks>`: the click combo for this run (not
/// saved), and that many clicks on the plate under the pointer of
/// `AURORA_DEMO_PLATE_HOVER`, one every `DEMO_COMBO_FRAMES` frames, the
/// last at frame 700.
fn demo_combo() -> Option<u64> {
    static CLICKS: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *CLICKS.get_or_init(|| std::env::var("AURORA_DEMO_PLATE_COMBO").ok()?.trim().parse().ok())
}
const DEMO_COMBO_FRAMES: u64 = 4;

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

/// Candidates captured at the press, frontmost first. Compare with the
/// displayed opaque depth, just as labels.wgsl does; invisible touch panels
/// and Blend faces do not hide a displayed plate (#420).
pub(super) fn visible_tag(tags: &[(Uuid, Vec3)], depth: Option<Vec3>, ray: Option<(Vec3, Vec3)>) -> Option<(Uuid, Vec3)> {
    let (origin, direction) = ray?;
    let limit = depth.map_or(f32::INFINITY, |p| (p - origin).dot(direction) + 0.05);
    tags.iter().copied().find(|(_, p)| (*p - origin).dot(direction) <= limit)
}

/// Whether a plate click acts, and so shows the hand: an action chosen, on
/// someone else's plate.
pub(super) fn plate_click_acts(action: PlateClick, plate: Option<Uuid>, me: Uuid) -> bool {
    action != PlateClick::Nothing && plate.is_some_and(|id| id != me)
}

/// Alt retains LLToolCamera's gestures; Ctrl+Shift retains the grab tool
/// override of LLToolPie::getOverrideTool (indra/newview/lltoolpie.cpp).
/// A disabled modified action does not fall back to the plain click.
fn plate_click_choice(settings: &crate::settings::Settings, [alt, ctrl, shift]: [bool; 3]) -> PlateClick {
    match (alt, ctrl, shift) {
        (false, false, false) => settings.plate_click,
        (false, false, true) => settings.plate_shift_click,
        (false, true, false) => settings.plate_ctrl_click,
        _ => PlateClick::Nothing,
    }
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
    pub(super) fn name_tag_candidates(&self, ray: Option<(Vec3, Vec3)>) -> Vec<(Uuid, Vec3)> {
        let Some(ray) = ray else { return Vec::new() };
        let ppp = self.egui_ctx.pixels_per_point().max(0.1);
        let pointer = egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp);
        self.name_tags
            .iter()
            .rev()
            .filter(|t| t.rect.contains(pointer))
            .filter_map(|t| Some((t.id, t.point(ray)?)))
            .collect()
    }

    /// The action of a click on a plate: the setting, or the demo's.
    pub(super) fn plate_click_action(&self) -> PlateClick {
        let mods = [self.alt || self.object_view, self.ctrl, self.shift];
        match demo_action() {
            Some(action) if self.demo && mods == [false; 3] => action,
            _ => plate_click_choice(&self.settings, mods),
        }
    }

    /// Where the pointer stands for the plates, in points: nowhere over a
    /// window, while the camera or a drag holds the mouse, in mouselook or
    /// with the classic name tags.
    fn plate_pointer(&self, ctx: &egui::Context) -> Option<egui::Pos2> {
        if self.demo
            && let Some((pointer, _)) = demo_pointer()
        {
            let away = demo_jiggle() && (600..700).contains(&self.frame_count) && (self.frame_count / DEMO_JIGGLE_FRAMES).is_multiple_of(2);
            return (!away).then_some(pointer);
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
        self.plates.friend_tint = self.settings.plates_friend_tint;
        self.plates.still = self.settings.reduce_motion;
        self.plates.jiggle = self.settings.plates_jiggle || (self.demo && demo_jiggle());
        self.plates.click_combo = self.combo_on();
        if self.plates.click_combo {
            self.plates.tally.open(self.world.agent_id);
        }
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
        if self.demo
            && let Some(clicks) = demo_combo().filter(|n| *n > 0)
            && let Some(first) = DEMO_CLICK_FRAME.checked_sub((clicks - 1) * DEMO_COMBO_FRAMES)
            && (first..=DEMO_CLICK_FRAME).contains(&self.frame_count)
            && (self.frame_count - first).is_multiple_of(DEMO_COMBO_FRAMES)
        {
            self.plate_combo_press();
        }
    }

    /// The click combo is on: the setting, or the demo's.
    fn combo_on(&self) -> bool {
        self.settings.plates_click_combo || (self.demo && demo_combo().is_some())
    }

    /// A left press on a plate with the click combo on (Preferences › Just
    /// for fun): counted at once, from the plates of the last frame as the
    /// pointer's light is, without waiting for the depth. Without a click
    /// action, the click is the combo's alone: no touch, no double click
    /// walking or teleporting there. Not with a modifier (camera, grab and
    /// the modified actions keep it), nor while the build tools are open.
    pub(super) fn plate_combo_press(&mut self) -> bool {
        if !self.combo_on() || self.classic_name_tags() || self.build.open || self.alt || self.ctrl || self.shift || self.object_view {
            return false;
        }
        let Some(id) = self.plate_hover else {
            return false;
        };
        let now = self.egui_ctx.input(|i| i.time);
        let Some(hit) = self.plates.click(id, now) else {
            return false;
        };
        if self.demo {
            log::info!("demo plate combo: {id} {hit:?}, {} in all", self.plates.tally.get(id));
        }
        self.plate_click_action() == PlateClick::Nothing
    }

    /// The hand of a clickable object over a plate that acts.
    pub(super) fn plate_cursor(&self) -> bool {
        !self.classic_name_tags() && plate_click_acts(self.plate_click_action(), self.plate_hover, self.world.agent_id)
    }

    /// A left press on the world: on another avatar's plate with an action
    /// chosen for these modifiers, the action runs and takes the click.
    /// Alt and combined Ctrl+Shift retain their camera/grab meaning.
    pub(super) fn plate_press(&mut self, tag: Option<(Uuid, Vec3)>) -> bool {
        if self.classic_name_tags() {
            return false;
        }
        let action = self.plate_click_action();
        if action == PlateClick::Nothing {
            return false;
        }
        let Some((id, _)) = tag else {
            return false;
        };
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
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
        if self.demo {
            log::info!("demo plate action: {:?} on {id}", self.plate_click_action());
        }
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

    #[test]
    fn plate_click_modifiers_choose_independently_and_preserve_camera_gestures() {
        let settings = crate::settings::Settings {
            plate_click: PlateClick::Profile,
            plate_shift_click: PlateClick::InstantMessage,
            plate_ctrl_click: PlateClick::Menu,
            ..Default::default()
        };
        assert_eq!(plate_click_choice(&settings, [false, false, false]), PlateClick::Profile);
        assert_eq!(plate_click_choice(&settings, [false, false, true]), PlateClick::InstantMessage);
        assert_eq!(plate_click_choice(&settings, [false, true, false]), PlateClick::Menu);
        assert_eq!(plate_click_choice(&settings, [false, true, true]), PlateClick::Nothing);
        for ctrl in [false, true] {
            for shift in [false, true] {
                assert_eq!(plate_click_choice(&settings, [true, ctrl, shift]), PlateClick::Nothing);
            }
        }
        let disabled = crate::settings::Settings {
            plate_click: PlateClick::Profile,
            ..Default::default()
        };
        assert_eq!(plate_click_choice(&disabled, [false, false, true]), PlateClick::Nothing);
        assert_eq!(plate_click_choice(&disabled, [false, true, false]), PlateClick::Nothing);
    }

    fn tag(n: u128, rect: egui::Rect) -> ui::hud::NameTag {
        ui::hud::NameTag {
            id: Uuid::from_u128(n),
            rect,
            at: Vec3::X * 10.0,
            normal: Vec3::X,
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

    #[test]
    fn world_click_plate_uses_the_displayed_plane_at_every_side_and_pitch() {
        use glam::Mat4;
        let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 16.0 / 9.0, 0.1);
        for eye in [Vec3::new(0.0, -8.0, 2.0), Vec3::new(0.0, -8.0, 8.0)] {
            let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::Z, Vec3::Z);
            let vp = projection * view;
            let proj = ui::hud::Projector {
                view_proj: vp,
                width: 1920.0,
                height: 1080.0,
                ppp: 1.5,
            };
            for x in [-4.0, 0.0, 4.0] {
                let at = Vec3::new(x, 4.0, 3.0);
                let clip = vp * at.extend(1.0);
                let ndc = clip.truncate() / clip.w;
                let tag = ui::hud::NameTag::new(Uuid::from_u128(2), egui::Rect::EVERYTHING, at, &proj);
                for offset in [glam::Vec2::ZERO, glam::Vec2::new(0.10, 0.05), glam::Vec2::new(-0.10, -0.05)] {
                    let unproject = |z| {
                        let p = vp.inverse() * glam::Vec4::new(ndc.x + offset.x, ndc.y + offset.y, z, 1.0);
                        p.truncate() / p.w
                    };
                    let origin = unproject(1.0);
                    let direction = (unproject(0.001) - origin).normalize();
                    let point = tag.point((origin, direction)).expect("the tag's displayed plane");
                    let hit = vp * point.extend(1.0);
                    assert!((hit.z / hit.w - ndc.z).abs() < 1e-6);
                    let captured = [(tag.id, point)];
                    assert_eq!(
                        visible_tag(&captured, Some(point + direction), Some((origin, direction))),
                        Some((tag.id, point))
                    );
                    assert_eq!(visible_tag(&captured, Some(point - direction), Some((origin, direction))), None);
                }
            }
        }
        // A bad / behind-camera ray must not create a click target.
        let proj = ui::hud::Projector {
            view_proj: Mat4::IDENTITY,
            width: 100.0,
            height: 100.0,
            ppp: 1.0,
        };
        let tag = ui::hud::NameTag::new(Uuid::nil(), egui::Rect::EVERYTHING, Vec3::X, &proj);
        assert_eq!(tag.point((Vec3::ZERO, Vec3::X)), None);
    }

    #[test]
    fn world_click_plate_snapshot_keeps_the_pressed_target_and_respects_walls() {
        let ray = Some((Vec3::ZERO, Vec3::X));
        let near = (Uuid::from_u128(2), Vec3::X * 5.0);
        let far = (Uuid::from_u128(3), Vec3::X * 10.0);
        let pressed = vec![near, far];
        // The current frame's labels can move or change order while the GPU
        // answers: resolution uses the values owned by the pending press.
        let later = [far];
        assert_eq!(visible_tag(&pressed, None, ray), Some(near));
        assert_eq!(visible_tag(&later, None, ray), Some(far));
        assert_eq!(visible_tag(&pressed, Some(Vec3::X * 4.0), ray), None);
        assert_eq!(visible_tag(&pressed, Some(Vec3::X * 7.0), ray), Some(near));
        assert_eq!(visible_tag(&pressed, None, None), None);
    }
}
