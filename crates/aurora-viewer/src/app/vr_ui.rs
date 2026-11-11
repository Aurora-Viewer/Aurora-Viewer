//! The viewer's interface in VR: a floating panel opened with the left
//! controller's X button (or its menu button where the runtime passes it on), used with the pointer beam (the trigger is the
//! left mouse button, the right thumbstick scrolls), and an on-screen
//! keyboard while a text field has the focus.
//!
//! The render worker draws the whole interface on the panel
//! (`aurora_render::VrPanel`); here the beam's hit on it becomes ordinary
//! egui pointer events, given to the interface with the next frame's input,
//! so every window works as it does with the mouse.

use super::*;

/// The panel's width, metres, and where it opens: this far ahead of the
/// headset and this far below the eyes.
const PANEL_WIDTH: f32 = 1.6;
const PANEL_AHEAD: f32 = 1.0;
const PANEL_BELOW: f32 = 0.15;
/// Points scrolled per frame at full right-stick deflection.
const SCROLL: f32 = 10.0;

/// The wrist's chat: lines shown, the panel's width (metres) and how far
/// above the hand it floats.
const WRIST_LINES: usize = 6;
const WRIST_WIDTH: f32 = 0.26;
const WRIST_ABOVE: f32 = 0.08;
/// The panel glides back in front once the head is this far from it
/// (cosine: 60 degrees), this much of the way each frame.
const PANEL_AWAY: f32 = 0.5;
/// The pie's radius in the interface (points, as `ui::context::pie`), with
/// room for its labels, and its size in the world per metre away.
const PIE_RADIUS: f32 = 180.0;
const PIE_MARGIN: f32 = 24.0;
const PIE_METRES_PER_POINT: f32 = 0.0009;
const PANEL_GLIDE: f32 = 0.12;

const ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm,.?"];

/// Where the panel opens, in front of the headset and facing it, level,
/// with the interface's aspect (`aspect`: height / width).
pub(super) fn place_panel(head: Vec3, head_forward: Vec3, aspect: f32) -> aurora_render::VrPanel {
    let forward = head_forward.with_z(0.0).normalize_or(Vec3::X);
    let half = PANEL_WIDTH * 0.5;
    aurora_render::VrPanel {
        center: head + forward * PANEL_AHEAD - Vec3::Z * PANEL_BELOW,
        // (Z up: the facing crossed with up is the viewer's right)
        right: forward.cross(Vec3::Z).normalize_or(Vec3::Y) * half,
        up: Vec3::Z * half * aspect.clamp(0.2, 3.0),
        uv: aurora_render::VrPanel::WHOLE,
    }
}

/// The VR camera: held in the other hand (following it) or set down where
/// it was let go.
pub(super) struct VrPhotoCamera {
    held: bool,
    position: Vec3,
    rotation: glam::Quat,
}

/// The camera's view, the viewfinder's half size (metres) and how near the
/// hand must be to pick the camera up again.
const CAMERA_FOV: f32 = 60.0 * std::f32::consts::PI / 180.0;
const VIEWFINDER_HALF: (f32, f32) = (0.16, 0.09);

/// An object carried by the hand in VR, with the build tools open: its
/// root's place and turn in the hand's frame when it was taken.
pub(super) struct VrGrab {
    key: crate::world::objects::ObjKey,
    id: uuid::Uuid,
    offset: Vec3,
    turn: glam::Quat,
    sent: Instant,
}

/// Where the wrist's chat shows: above the left hand while it is raised
/// (pointing up) and looked at, within reach. None otherwise.
pub(super) fn wrist_point(head: Vec3, head_forward: Vec3, hand: &aurora_render::VrControllerState) -> Option<Vec3> {
    if !hand.active || !hand.position.is_finite() {
        return None;
    }
    let to_hand = hand.position - head;
    let distance = to_hand.length();
    let looked_at = head_forward.normalize_or_zero().dot(to_hand / distance.max(1e-4)) > 0.85;
    let raised = (hand.orientation * Vec3::NEG_Z).z > 0.35;
    (distance > 0.15 && distance < 0.8 && looked_at && raised).then(|| hand.position + Vec3::Z * WRIST_ABOVE)
}

/// The part of the interface image `rect` covers, 0..1 (left, top, right, bottom).
fn image_part(rect: egui::Rect, screen: egui::Rect) -> [f32; 4] {
    [
        (rect.min.x - screen.min.x) / screen.width().max(1.0),
        (rect.min.y - screen.min.y) / screen.height().max(1.0),
        (rect.max.x - screen.min.x) / screen.width().max(1.0),
        (rect.max.y - screen.min.y) / screen.height().max(1.0),
    ]
    .map(|c| c.clamp(0.0, 1.0))
}

impl App {
    /// Left X (or the left menu button, where the runtime passes it on:
    /// Virtual Desktop keeps it for its own menu) opens and closes the
    /// panel; it is closed outside VR. Kept in the render worker's hands
    /// every frame.
    pub(super) fn vr_panel_button(&mut self, renderer: &Renderer) {
        let input = renderer.vr_input();
        if !renderer.vr_engaged() {
            self.vr_panel = None;
            self.vr_menu_click = false;
            self.vr_keyboard_target = None;
            self.vr_panel_drag = None;
            self.vr_wrist_at = None;
            self.vr_dialog_seen = self.world.notifications.last_id();
            renderer.set_vr_panel(None);
            renderer.set_vr_wrist(None);
            renderer.set_vr_menu(None);
            renderer.set_vr_map(None);
            self.vr_map_panel = None;
            self.vr_pie = None;
            self.vr_menu_panel = None;
            return;
        }
        // the right-click pie, floating at its object while it is open
        if self.context_menu.is_none() {
            self.vr_pie = None;
        }
        self.vr_menu_panel = self.vr_pie.and_then(|point| {
            let screen = self.egui_ctx.content_rect();
            let radius = PIE_RADIUS.min((screen.width().min(screen.height()) * 0.5 - 8.0).max(0.0)) + PIE_MARGIN;
            let part = egui::Rect::from_center_size(screen.center(), egui::vec2(radius * 2.0, radius * 2.0)).intersect(screen);
            if part.width() < 1.0 {
                return None;
            }
            let to_head = input.head_position - point;
            let distance = to_head.length().clamp(0.6, 12.0);
            let forward = (-to_head).with_z(0.0).normalize_or(Vec3::X);
            let half = egui::vec2(part.width(), part.height()) * 0.5 * PIE_METRES_PER_POINT * distance;
            // in front of the object, towards the head
            let toward = to_head.normalize_or_zero() * (0.2 * distance).min(0.8);
            Some(aurora_render::VrPanel {
                center: point + toward,
                right: forward.cross(Vec3::Z).normalize_or(Vec3::Y) * half.x,
                up: Vec3::Z * half.y,
                uv: image_part(part, screen),
            })
        });
        renderer.set_vr_menu(self.vr_menu_panel);
        // the microphone heard: one sound as it comes on, another as it
        // goes off (its badge in the view shows which)
        let sending = self.vr_mic_light();
        if let (Some(before), Some(now)) = (self.vr_mic_state, sending)
            && before != now
        {
            self.play_ui_sound_forced(if now {
                crate::ui_sound::UiSound::MicToggle
            } else {
                crate::ui_sound::UiSound::PieMenuHide
            });
        }
        self.vr_mic_state = sending;
        // a script's menu or text box just came: the panel opens on it
        let seen = self.vr_dialog_seen;
        if self.world.notifications.list.iter().any(|n| {
            n.id > seen
                && matches!(
                    n.data,
                    crate::world::notifications::Data::Dialog { .. } | crate::world::notifications::Data::TextBox { .. }
                )
        }) {
            self.vr_open_panel = true;
        }
        self.vr_dialog_seen = self.world.notifications.last_id();
        let menu = input.left.active && (input.left.primary || input.left.menu);
        if menu && !self.vr_menu_click {
            self.vr_panel = if self.vr_panel.is_some() {
                None
            } else {
                let rect = self.egui_ctx.content_rect();
                let aspect = rect.height() / rect.width().max(1.0);
                Some(place_panel(input.head_position, input.head_forward, aspect))
            };
            if self.settings.camera.vr.haptics {
                renderer.vr_haptic(aurora_render::VrSide::Left, 0.35, 0.04);
            }
        }
        self.vr_menu_click = menu;
        if std::mem::take(&mut self.vr_close_panel) {
            self.vr_panel = None;
        }
        if std::mem::take(&mut self.vr_open_panel) && self.vr_panel.is_none() {
            let rect = self.egui_ctx.content_rect();
            let aspect = rect.height() / rect.width().max(1.0);
            self.vr_panel = Some(place_panel(input.head_position, input.head_forward, aspect));
        }
        if std::mem::take(&mut self.vr_recenter_request) {
            renderer.vr_recenter();
        }
        let rect = self.egui_ctx.content_rect();
        let aspect = rect.height() / rect.width().max(1.0);
        // turned well away from the panel: it glides back in front
        if let Some(panel) = &mut self.vr_panel
            && self.settings.camera.vr.panel_follows
            && self.vr_panel_drag.is_none()
        {
            let to = (panel.center - input.head_position).with_z(0.0).normalize_or_zero();
            let facing = input.head_forward.with_z(0.0).normalize_or_zero();
            if to != Vec3::ZERO && facing != Vec3::ZERO && to.dot(facing) < PANEL_AWAY {
                self.vr_panel_return = true;
            }
            if self.vr_panel_return {
                let target = place_panel(input.head_position, input.head_forward, aspect);
                panel.center = panel.center.lerp(target.center, PANEL_GLIDE);
                panel.right = panel.right.lerp(target.right, PANEL_GLIDE).normalize_or(Vec3::Y) * target.right.length();
                panel.up = target.up;
                if panel.center.distance(target.center) < 0.02 {
                    *panel = target;
                    self.vr_panel_return = false;
                }
            }
        } else {
            self.vr_panel_return = false;
        }
        renderer.set_vr_panel(self.vr_panel);
        // the wrist's chat (not with the panel open: the chat is there)
        let camera_in_left = self.vr_photo_camera.as_ref().is_some_and(|c| c.held)
            && self.settings.camera.vr.dominant_hand == crate::camera::VrHand::Right;
        self.vr_wrist_at = (self.settings.camera.vr.wrist_chat && self.vr_panel.is_none() && !camera_in_left)
            .then(|| wrist_point(input.head_position, input.head_forward, &input.left))
            .flatten();
        let wrist = self.vr_wrist_at.zip(self.vr_wrist_rect).and_then(|(at, part)| {
            if part.width() < 1.0 || rect.width() < 1.0 {
                return None;
            }
            let half = WRIST_WIDTH * 0.5;
            let half_height = half * part.height() / part.width();
            let forward = (at - input.head_position).with_z(0.0).normalize_or(Vec3::X);
            Some(aurora_render::VrPanel {
                center: at + Vec3::Z * half_height,
                right: forward.cross(Vec3::Z).normalize_or(Vec3::Y) * half,
                up: Vec3::Z * half_height,
                uv: image_part(part, rect),
            })
        });
        self.vr_wrist_panel = wrist;
        renderer.set_vr_wrist(wrist);
        // the minimap (the toolbar's Map) on the right wrist: above the right
        // hand while it is raised and looked at (the left hand's beam works it)
        let camera_in_right = self.vr_photo_camera.as_ref().is_some_and(|c| c.held)
            && self.settings.camera.vr.dominant_hand == crate::camera::VrHand::Left;
        let raised = (self.vr_minimap && self.vr_panel.is_none() && !camera_in_right)
            .then(|| wrist_point(input.head_position, input.head_forward, &input.right))
            .flatten();
        let map = raised.and_then(|at| {
            let part = self.egui_ctx.memory(|m| m.area_rect(egui::Id::unique("minimap")))?.intersect(rect);
            if part.width() < 1.0 || part.height() < 1.0 {
                return None;
            }
            let half = WRIST_WIDTH * 0.5;
            let half_height = half * part.height() / part.width();
            let forward = (at - input.head_position).with_z(0.0).normalize_or(Vec3::X);
            let side = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
            Some(aurora_render::VrPanel {
                center: at + Vec3::Z * half_height,
                right: side * half,
                up: Vec3::Z * half_height,
                uv: image_part(part, rect),
            })
        });
        self.vr_map_panel = map;
        renderer.set_vr_map(map);
    }

    /// Where the beam meets the interface in VR: the panel or the wrist's
    /// chat, the nearer; its distance and the point of the interface image
    /// it is on (0..1).
    pub(super) fn vr_ui_hit(&self, ray: (Vec3, Vec3)) -> Option<(f32, f32, f32)> {
        let mut nearest: Option<(f32, f32, f32)> = None;
        for panel in [self.vr_menu_panel, self.vr_panel, self.vr_wrist_panel, self.vr_map_panel]
            .into_iter()
            .flatten()
        {
            if let Some((t, u, v)) = panel.hit(ray.0, ray.1)
                && nearest.is_none_or(|(near, _, _)| t < near)
            {
                let [u0, v0, u1, v1] = panel.uv;
                nearest = Some((t, u0 + u * (u1 - u0), v0 + v * (v1 - v0)));
            }
        }
        nearest
    }

    /// The arms' calibration, from the VR toolbar: for three seconds the
    /// widest span between the controllers is taken (both arms held out to
    /// the sides), then the avatar's arm over ours becomes the arm scale.
    pub(super) fn vr_calibrate_start(&mut self) {
        self.vr_calibration = Some((Instant::now(), 0.0));
        self.world.system_message(aurora_i18n::tr!("vr-arms-calibrate-start"));
    }

    pub(super) fn vr_calibrate_step(&mut self, renderer: &Renderer, input: &aurora_render::VrInputState) {
        let Some((since, widest)) = self.vr_calibration else { return };
        let span = (input.left.active && input.right.active)
            .then(|| input.left.position.distance(input.right.position))
            .filter(|s| s.is_finite())
            .unwrap_or(0.0);
        let widest = widest.max(span);
        if since.elapsed() < std::time::Duration::from_secs(3) {
            self.vr_calibration = Some((since, widest));
            return;
        }
        self.vr_calibration = None;
        let Some(avatar) = self.scene.own_arm_length(&self.world).filter(|_| widest > 0.6) else {
            self.world.system_message(aurora_i18n::tr!("vr-arms-calibrate-failed"));
            return;
        };
        let scale = (avatar / crate::scene::vr_arms::arm_from_span(widest)).clamp(0.5, 2.0);
        self.settings.camera.vr.arm_scale = scale;
        self.settings.save();
        self.world
            .system_message(format!("{} {:.0}%", aurora_i18n::tr!("vr-arms-calibrated"), scale * 100.0));
        if self.settings.camera.vr.haptics {
            renderer.vr_haptic(aurora_render::VrSide::Left, 0.6, 0.08);
            renderer.vr_haptic(aurora_render::VrSide::Right, 0.6, 0.08);
        }
    }

    /// Nods, head shakes and waves: the built-in yes, no and hello
    /// animations, which everyone sees; stopped after a few seconds, with a
    /// pause before the next.
    pub(super) fn vr_gestures(&mut self, input: &aurora_render::VrInputState, settings: &crate::camera::VrSettings) {
        let now = Instant::now();
        if let Some((anim, until)) = self.vr_gesture
            && now >= until
        {
            self.send(NetCommand::AgentAnimation { anim, start: false });
            self.vr_gesture = None;
        }
        let forward = input.head_forward.normalize_or_zero();
        if !settings.gestures || forward == Vec3::ZERO || self.vr_panel.is_some() {
            self.vr_nod.reset();
            self.vr_shake.reset();
            self.vr_wave.reset();
            self.vr_head_yaw = None;
            return;
        }
        let window = std::time::Duration::from_millis(1200);
        let pitch = forward.z.clamp(-1.0, 1.0).asin();
        // the yaw unwrapped, so that turning through 180 degrees is no jump
        let yaw = forward.y.atan2(forward.x);
        let unwrapped = match self.vr_head_yaw {
            Some((last, total)) => {
                let step = (yaw - last + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                total + step
            }
            None => yaw,
        };
        self.vr_head_yaw = Some((yaw, unwrapped));
        let nod = self.vr_nod.step(pitch, 0.12, 3, window, now);
        let shake = self.vr_shake.step(unwrapped, 0.15, 3, window, now);
        // a hand raised above the head, moving side to side
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
        let raised = [input.left, input.right]
            .into_iter()
            .find(|h| h.active && h.position.z > input.head_position.z + 0.05);
        let wave = match raised {
            Some(hand) => self.vr_wave.step(
                (hand.position - input.head_position).dot(right),
                0.07,
                3,
                std::time::Duration::from_millis(1600),
                now,
            ),
            None => {
                self.vr_wave.reset();
                false
            }
        };
        if self.vr_gesture.is_some() {
            return;
        }
        let anim = if wave {
            Some(uuid::Uuid::from_u128(0x9b29cd61c45b5689ded291756b8d76a9))
        } else if nod {
            Some(uuid::Uuid::from_u128(0x15dd911dbe82285626db27659b142875))
        } else if shake {
            Some(uuid::Uuid::from_u128(0x5a977ed97f7244e94c4c6e913df8ae74))
        } else {
            None
        };
        if let Some(anim) = anim {
            self.send(NetCommand::AgentAnimation { anim, start: true });
            self.vr_gesture = Some((anim, now + std::time::Duration::from_millis(2500)));
            self.vr_nod.reset();
            self.vr_shake.reset();
            self.vr_wave.reset();
        }
    }

    /// The trigger on an object with the build tools open: its root is taken
    /// when we may edit it (not an avatar, not an attachment, not linked to
    /// another object).
    pub(super) fn vr_grab_start(
        &self,
        point: Vec3,
        ray: (Vec3, Vec3),
        hand: &aurora_render::VrControllerState,
    ) -> Option<VrGrab> {
        let idx = self.scene.left_interaction_at_ray(&self.world, point, ray, true)?;
        let root = crate::build::root_of(&self.world, idx);
        let o = self.world.objects.get(root)?;
        if o.is_avatar() || o.is_attachment() || o.parent_id != 0 || !self.build.can_modify(&self.world, root) {
            return None;
        }
        let at = self.world.region_offset(o.key.region)? + o.position;
        let hand_turn = hand.orientation.normalize();
        Some(VrGrab {
            key: o.key,
            id: o.full_id,
            offset: hand_turn.inverse() * (at - hand.position),
            turn: (hand_turn.inverse() * o.rotation).normalize(),
            sent: Instant::now(),
        })
    }

    /// The carried object follows the hand: shown there at once, sent to
    /// the simulator (MultipleObjectUpdate, the whole linkset) at most every
    /// 100 ms and when let go (`release`).
    pub(super) fn vr_grab_carry(&mut self, hand: &aurora_render::VrControllerState, release: bool) {
        let Some(mut grab) = self.vr_grab.take() else { return };
        let Some(idx) = self
            .world
            .objects
            .index_of(&grab.key)
            .filter(|&i| self.world.objects.get(i).is_some_and(|o| o.full_id == grab.id))
        else {
            return;
        };
        let Some(region) = self.world.region_offset(grab.key.region) else { return };
        let hand_turn = hand.orientation.normalize();
        let position = hand.position + hand_turn * grab.offset - region;
        let rotation = (hand_turn * grab.turn).normalize();
        if hand.active && position.is_finite() && rotation.is_finite() {
            if let Some(o) = self.world.objects.get_mut(idx) {
                o.position = position;
                o.rotation = rotation;
                o.velocity = Vec3::ZERO;
                o.acceleration = Vec3::ZERO;
                o.angular_velocity = Vec3::ZERO;
                o.render.needs_records = true;
            }
            if release || grab.sent.elapsed() >= std::time::Duration::from_millis(100) {
                grab.sent = Instant::now();
                self.send(NetCommand::Build(aurora_net::build::BuildCmd::Transform {
                    handle: grab.key.region,
                    updates: vec![aurora_net::build::TransformUpdate {
                        local_id: grab.key.local_id,
                        position: Some(position),
                        rotation: Some(rotation),
                        scale: None,
                        linked: true,
                        uniform: false,
                    }],
                }));
            }
        }
        if !release && hand.active {
            self.vr_grab = Some(grab);
        }
    }

    /// The VR toolbar, at the top of the panel while it is open: the
    /// camera, dictation, the arms' calibration, recentring, closing.
    pub(super) fn vr_toolbar(&mut self, ctx: &egui::Context) {
        if self.vr_panel.is_none() {
            return;
        }
        let camera_out = self.vr_photo_camera.is_some();
        let (mut camera, mut dictate, mut calibrate, mut recenter, mut close) = (false, false, false, false, false);
        let (mut place, mut mic) = (false, false);
        let mut map = false;
        let map_on = self.vr_minimap;
        let held = self.vr_photo_camera.as_ref().map(|c| c.held);
        let mic_on = self.mic_on;
        let mut person = false;
        let third = self.vr_third_person;
        egui::Area::new(egui::Id::unique("vr-toolbar"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 52.0))
            .constrain_to(ctx.content_rect())
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let size = egui::vec2(0.0, 34.0);
                        let view = if third {
                            aurora_i18n::tr!("ui-vr-toolbar-third-person")
                        } else {
                            aurora_i18n::tr!("ui-vr-toolbar-first-person")
                        };
                        person = ui.add(egui::Button::new(view).min_size(size)).clicked();
                        camera = ui
                            .add(egui::Button::new(aurora_i18n::tr!("ui-vr-toolbar-camera")).selected(camera_out).min_size(size))
                            .clicked();
                        if let Some(held) = held {
                            let text = if held {
                                aurora_i18n::tr!("ui-vr-toolbar-camera-set-down")
                            } else {
                                aurora_i18n::tr!("ui-vr-toolbar-camera-take")
                            };
                            place = ui.add(egui::Button::new(text).min_size(size)).clicked();
                        }
                        let mic_text = if mic_on {
                            aurora_i18n::tr!("ui-vr-toolbar-mic-on")
                        } else {
                            aurora_i18n::tr!("ui-vr-toolbar-mic-off")
                        };
                        mic = ui.add(egui::Button::new(mic_text).selected(mic_on).min_size(size)).clicked();
                        map = ui
                            .add(egui::Button::new(aurora_i18n::tr!("ui-vr-toolbar-map")).selected(map_on).min_size(size))
                            .clicked();
                        dictate = ui.add(egui::Button::new(aurora_i18n::tr!("ui-vr-toolbar-dictate")).min_size(size)).clicked();
                        calibrate = ui.add(egui::Button::new(aurora_i18n::tr!("ui-vr-toolbar-calibrate")).min_size(size)).clicked();
                        recenter = ui.add(egui::Button::new(aurora_i18n::tr!("ui-vr-toolbar-recenter")).min_size(size)).clicked();
                        close = ui.add(egui::Button::new(aurora_i18n::tr!("ui-vr-toolbar-close")).min_size(size)).clicked();
                    });
                });
            });
        if camera {
            // out in the other hand, or put away
            self.vr_photo_camera = if camera_out {
                None
            } else {
                Some(VrPhotoCamera {
                    held: true,
                    position: Vec3::ZERO,
                    rotation: glam::Quat::IDENTITY,
                })
            };
        }
        if place && let Some(camera) = &mut self.vr_photo_camera {
            camera.held = !camera.held;
        }
        if mic && self.in_world() {
            self.toggle_mic();
        }
        if map {
            self.vr_minimap = !map_on;
            // the minimap window itself, which the wrist shows
            if self.vr_minimap {
                self.panels.minimap = true;
            }
        }
        if dictate {
            self.vr_dictate(ctx);
        }
        if calibrate {
            self.vr_calibrate_start();
        }
        if person {
            self.vr_third_person = !third;
        }
        self.vr_recenter_request |= recenter;
        self.vr_close_panel |= close;
    }

    /// Dictation: Windows' voice typing (Win+H) into the text field that
    /// has the focus (a chat bar or a conversation); its own window shows
    /// on the desktop. Again to stop it.
    fn vr_dictate(&mut self, ctx: &egui::Context) {
        if !ctx.egui_wants_keyboard_input() {
            self.world.system_message(aurora_i18n::tr!("vr-dictate-no-field"));
            return;
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN,
            };
            let key = |vk: u16, up: bool| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: 0,
                        dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            };
            const VK_H: u16 = 0x48;
            let inputs = [key(VK_LWIN, false), key(VK_H, false), key(VK_H, true), key(VK_LWIN, true)];
            // SAFETY: a valid array of keyboard inputs and its element size
            unsafe {
                SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            }
        }
        #[cfg(not(windows))]
        self.world.system_message(aurora_i18n::tr!("vr-dictate-windows-only"));
    }

    /// The VR camera each frame: it follows the other hand while held, or
    /// stays where the toolbar set it down; that hand's trigger takes a
    /// photo (saved as the photo window's "to disk"). The camera's view and
    /// viewfinder go to the render worker.
    pub(super) fn vr_camera_step(
        &mut self,
        renderer: &Renderer,
        hand: &aurora_render::VrControllerState,
        side: aurora_render::VrSide,
        shutter: bool,
        settings: &crate::camera::VrSettings,
    ) {
        // (the grip stays free: a hand resting on it must not drop the camera)
        let Some(camera) = &mut self.vr_photo_camera else {
            renderer.set_vr_camera(None);
            return;
        };
        if camera.held && hand.active {
            camera.rotation = hand.orientation.normalize();
            camera.position = hand.position + camera.rotation * Vec3::new(0.0, 0.03, -0.04);
        }
        let (position, rotation) = (camera.position, camera.rotation);
        if position == Vec3::ZERO {
            renderer.set_vr_camera(None);
            return;
        }
        let forward = rotation * Vec3::NEG_Z;
        let up = rotation * Vec3::Y;
        let right = rotation * Vec3::X;
        renderer.set_vr_camera(Some(aurora_render::VrCameraView {
            position,
            forward,
            up,
            fov_y: CAMERA_FOV,
            desktop: settings.camera_on_desktop,
            // the screen above the camera's back, facing whoever holds it
            viewfinder: Some(aurora_render::VrPanel {
                center: position - forward * 0.02 + up * (VIEWFINDER_HALF.1 + 0.06),
                right: right * VIEWFINDER_HALF.0,
                up: up * VIEWFINDER_HALF.1,
                uv: aurora_render::VrPanel::WHOLE,
            }),
        }));
        if shutter {
            self.photo_to_disk();
            if settings.haptics {
                renderer.vr_haptic(side, 0.7, 0.05);
            }
        }
    }

    /// The desktop mouse on the panel: its pointer drawn there (the
    /// headset hides the system's), while no beam is on the panel.
    pub(super) fn vr_mouse_cursor(&self, ctx: &egui::Context) {
        if self.vr_panel.is_none() || self.vr_on_panel {
            return;
        }
        let Some(at) = ctx.pointer_latest_pos().filter(|p| ctx.content_rect().contains(*p)) else {
            return;
        };
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("vr-mouse-cursor")));
        // a pointer's head (convex: one shape)
        let arrow = vec![at, at + egui::vec2(0.0, 22.0), at + egui::vec2(15.0, 15.0)];
        painter.add(egui::Shape::convex_polygon(
            arrow,
            egui::Color32::WHITE,
            egui::Stroke::new(1.5, egui::Color32::BLACK),
        ));
    }

    /// The wrist's chat, laid out in a corner of the interface while the
    /// left hand is raised and looked at: the last lines of local chat. The
    /// render worker shows that part of the interface above the hand.
    pub(super) fn vr_wrist_chat(&mut self, ctx: &egui::Context) {
        if self.vr_wrist_at.is_none() {
            self.vr_wrist_rect = None;
            return;
        }
        // the tabs: local chat, then the three conversations spoken in last
        let mut recent: Vec<(uuid::Uuid, std::time::SystemTime)> = self
            .world
            .social
            .ims
            .iter()
            .filter_map(|s| s.lines.back().map(|l| (s.other, l.time)))
            .collect();
        recent.sort_by(|a, b| b.1.cmp(&a.1));
        recent.truncate(3);
        if self.vr_wrist_tab.is_some_and(|id| !recent.iter().any(|(r, _)| *r == id)) {
            self.vr_wrist_tab = None;
        }
        let tabs: Vec<(Option<uuid::Uuid>, String, usize)> = std::iter::once((None, aurora_i18n::tr!("ui-vr-wrist-nearby").to_string(), 0))
            .chain(recent.iter().map(|(id, _)| {
                let name = self.world.avatar_name(id).unwrap_or_else(|| id.to_string()[..8].to_string());
                let unread = self.world.social.ims.iter().find(|s| s.other == *id).map_or(0, |s| s.unread);
                (Some(*id), name, unread)
            }))
            .collect();
        let shown: Vec<&crate::world::ChatLine> = match self.vr_wrist_tab {
            None => self.world.chat.iter().rev().take(WRIST_LINES).collect(),
            Some(id) => self
                .world
                .social
                .ims
                .iter()
                .find(|s| s.other == id)
                .map(|s| s.lines.iter().rev().take(WRIST_LINES).collect())
                .unwrap_or_default(),
        };
        let lines: Vec<String> = shown
            .into_iter()
            .rev()
            .map(|l| if l.from.is_empty() { l.text.clone() } else { format!("{}: {}", l.from, l.text) })
            .collect();
        let mut picked: Option<Option<uuid::Uuid>> = None;
        let mut reply = false;
        let current = self.vr_wrist_tab;
        // the view's switches: first or third person, camera only
        let (mut person, mut camera_only) = (false, false);
        let third = self.vr_third_person;
        let only = self.settings.camera.vr.locomotion == crate::camera::VrLocomotion::CameraOnly;
        let shown = egui::Area::new(egui::Id::unique("vr-wrist-chat"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
            .constrain_to(ctx.content_rect())
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(380.0);
                    ui.horizontal_wrapped(|ui| {
                        for (id, name, unread) in &tabs {
                            let text = if *unread > 0 { format!("{name} ({unread})") } else { name.clone() };
                            if ui.selectable_label(current == *id, text).clicked() {
                                picked = Some(*id);
                            }
                        }
                    });
                    ui.separator();
                    if lines.is_empty() {
                        ui.weak(aurora_i18n::tr!("ui-vr-wrist-chat-empty"));
                    }
                    for line in &lines {
                        ui.add(egui::Label::new(line.as_str()).wrap());
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button(aurora_i18n::tr!("ui-vr-wrist-reply")).clicked() {
                            reply = true;
                        }
                        let view = if third {
                            aurora_i18n::tr!("ui-vr-toolbar-third-person")
                        } else {
                            aurora_i18n::tr!("ui-vr-toolbar-first-person")
                        };
                        person = ui.button(view).clicked();
                        camera_only = ui
                            .add(egui::Button::new(aurora_i18n::tr!("ui-vr-wrist-camera-only")).selected(only))
                            .clicked();
                    });
                });
            });
        self.vr_wrist_rect = Some(shown.response.rect);
        if let Some(tab) = picked {
            self.vr_wrist_tab = tab;
        }
        if person {
            self.vr_third_person = !third;
        }
        if camera_only {
            let vr = &mut self.settings.camera.vr;
            if only {
                vr.locomotion = self.vr_locomotion_before.take().unwrap_or_default();
            } else {
                self.vr_locomotion_before = Some(vr.locomotion);
                vr.locomotion = crate::camera::VrLocomotion::CameraOnly;
            }
            self.settings.save();
        }
        // reply: the panel opens, on the conversation for a message tab
        if reply {
            if let Some(id) = self.vr_wrist_tab {
                self.open_conversation_to_type(id);
            }
            self.vr_open_panel = true;
        }
    }

    /// The microphone's light on the left controller: sending or not; None
    /// while voice is off.
    pub(super) fn vr_mic_light(&self) -> Option<bool> {
        self.settings.audio.voice_enabled.then(|| self.talking())
    }

    /// The beam on the panel: pointer events for the interface. True when
    /// the ray meets the panel (the world under it is then not clicked).
    pub(super) fn vr_panel_pointer(
        &mut self,
        ray: (Vec3, Vec3),
        trigger: (bool, bool),
        scroll: f32,
    ) -> Option<f32> {
        let (pressed, released) = trigger;
        let hit = self.vr_ui_hit(ray);
        let Some((t, u, v)) = hit else {
            if self.vr_on_panel {
                // the beam left the panel: a button held there is let go
                if self.vr_ui_down {
                    self.vr_ui_events.push(egui::Event::PointerButton {
                        pos: self.vr_ui_pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                    self.vr_ui_down = false;
                }
                self.vr_ui_events.push(egui::Event::PointerGone);
            }
            self.vr_on_panel = false;
            return None;
        };
        let rect = self.egui_ctx.content_rect();
        let pos = rect.min + egui::vec2(u * rect.width(), v * rect.height());
        self.vr_on_panel = true;
        self.vr_ui_pos = pos;
        self.vr_ui_events.push(egui::Event::PointerMoved(pos));
        // a press where no window is: a HUD there takes it, as a click on
        // the desktop would (its touch or click action)
        if pressed && !self.egui_ctx.is_pointer_over_egui() && self.huds_shown() {
            let ppp = self.egui_ctx.pixels_per_point().max(0.1);
            if let Some((idx, point, hud_ray)) = self.scene.hud_click_pick(&self.world, (pos.x * ppp, pos.y * ppp))
                && let Some(target) = crate::interaction::target(&self.world, idx, &self.interactions.props)
            {
                self.activate_object_action(target, idx, point, Some(hud_ray));
            }
        }
        if pressed || (released && self.vr_ui_down) {
            self.vr_ui_down = pressed;
            self.vr_ui_events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        if scroll != 0.0 {
            self.vr_ui_events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, scroll * SCROLL),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Default::default(),
            });
        }
        Some(t)
    }

    /// The hover tip's label by the beam: the part of the interface image
    /// where the tip was laid out (at the middle, `draw_hover_tips`), a
    /// little in front of the hit and above it, facing the headset, larger
    /// with distance so it reads the same.
    pub(super) fn vr_tip_label(&self, renderer: &Renderer, at: Option<Vec3>, head: Vec3) {
        let label = at.zip(self.hover_tip_rect()).and_then(|(point, tip)| {
            let screen = self.egui_ctx.content_rect();
            if screen.width() < 1.0 || screen.height() < 1.0 || tip.width() < 1.0 {
                return None;
            }
            let to_head = head - point;
            let distance = to_head.length().clamp(0.6, 12.0);
            let forward = (-to_head).with_z(0.0).normalize_or(Vec3::X);
            let metres_per_point = 0.0008 * distance;
            let half = egui::vec2(tip.width(), tip.height()) * 0.5 * metres_per_point;
            let toward = to_head.normalize_or_zero() * (0.15 * distance).min(0.6);
            let uv = [
                (tip.min.x - screen.min.x) / screen.width(),
                (tip.min.y - screen.min.y) / screen.height(),
                (tip.max.x - screen.min.x) / screen.width(),
                (tip.max.y - screen.min.y) / screen.height(),
            ];
            Some(aurora_render::VrPanel {
                center: point + toward + Vec3::Z * (half.y + 0.04 * distance),
                right: forward.cross(Vec3::Z).normalize_or(Vec3::Y) * half.x,
                up: Vec3::Z * half.y,
                uv: uv.map(|c| c.clamp(0.0, 1.0)),
            })
        });
        renderer.set_vr_tip(label);
    }

    /// The on-screen keyboard, at the bottom of the interface while the
    /// panel is open and a text field has the focus. A key's click would
    /// take the focus from the field: it is given back, and the key goes to
    /// the field with the next frame's input.
    pub(super) fn vr_keyboard(&mut self, ctx: &egui::Context) {
        if self.vr_panel.is_none() {
            self.vr_keyboard_target = None;
            return;
        }
        if ctx.egui_wants_keyboard_input()
            && let Some(focused) = ctx.memory(|m| m.focused())
        {
            self.vr_keyboard_target = Some(focused);
        }
        let Some(target) = self.vr_keyboard_target else {
            return;
        };
        let mut typed: Vec<egui::Event> = Vec::new();
        let mut close = false;
        let shift = self.vr_keyboard_shift;
        let key = |key: egui::Key| {
            [true, false].map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            })
        };
        let rect = ctx.content_rect();
        egui::Area::new(egui::Id::unique("vr-keyboard"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
            .constrain_to(rect)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let size = egui::vec2(46.0, 42.0);
                    let font = egui::FontId::proportional(20.0);
                    for row in ROWS {
                        ui.horizontal(|ui| {
                            for c in row.chars() {
                                let text = if shift { c.to_uppercase().to_string() } else { c.to_string() };
                                let label = egui::RichText::new(&text).font(font.clone());
                                if ui.add_sized(size, egui::Button::new(label)).clicked() {
                                    typed.push(egui::Event::Text(text));
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let wide = egui::vec2(size.x * 2.0, size.y);
                        let shift_label = if shift { "⇧ ABC" } else { "⇧ abc" };
                        if ui
                            .add_sized(wide, egui::Button::new(egui::RichText::new(shift_label).font(font.clone())).selected(shift))
                            .clicked()
                        {
                            self.vr_keyboard_shift = !shift;
                        }
                        if ui
                            .add_sized(egui::vec2(size.x * 5.0, size.y), egui::Button::new(egui::RichText::new("␣").font(font.clone())))
                            .clicked()
                        {
                            typed.push(egui::Event::Text(" ".into()));
                        }
                        if ui.add_sized(wide, egui::Button::new(egui::RichText::new("⌫").font(font.clone()))).clicked() {
                            typed.extend(key(egui::Key::Backspace));
                        }
                        if ui.add_sized(wide, egui::Button::new(egui::RichText::new("⏎").font(font.clone()))).clicked() {
                            typed.extend(key(egui::Key::Enter));
                        }
                        if ui.add_sized(size, egui::Button::new(egui::RichText::new("✕").font(font.clone()))).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.vr_keyboard_target = None;
            ctx.memory_mut(|m| m.surrender_focus(target));
            return;
        }
        if !typed.is_empty() {
            ctx.memory_mut(|m| m.request_focus(target));
            self.vr_ui_events.extend(typed);
            // one capital, as on a phone
            if self.vr_keyboard_shift && shift {
                self.vr_keyboard_shift = false;
            }
        } else if !ctx.egui_wants_keyboard_input() && ctx.memory(|m| m.focused()).is_none() && !self.vr_on_panel_press() {
            // the field was left (clicked elsewhere, closed): no keyboard
            self.vr_keyboard_target = None;
        }
    }

    /// A press on the panel this frame that is not on the keyboard yet:
    /// the keyboard stays while the press decides.
    fn vr_on_panel_press(&self) -> bool {
        self.vr_ui_down
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_opens_level_in_front_facing_the_head() {
        let head = Vec3::new(10.0, 20.0, 30.0);
        let panel = place_panel(head, Vec3::new(0.0, 1.0, -0.6), 0.5);
        // straight ahead along +Y, a little below the eyes
        assert!((panel.center - Vec3::new(10.0, 20.0 + PANEL_AHEAD, 30.0 - PANEL_BELOW)).length() < 1e-5);
        assert!(panel.right.z.abs() < 1e-6 && (panel.right.length() - PANEL_WIDTH * 0.5).abs() < 1e-5);
        assert!((panel.up.length() - PANEL_WIDTH * 0.25).abs() < 1e-5);
        // looking along +Y, the right is +X
        assert!(panel.right.x > 0.0);
        // the head's ray straight ahead meets its middle
        let (_, u, v) = panel.hit(head - Vec3::Z * PANEL_BELOW, Vec3::Y).expect("hit");
        assert!((u - 0.5).abs() < 1e-4 && (v - 0.5).abs() < 1e-4);
    }
}
