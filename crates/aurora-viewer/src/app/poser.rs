//! Consent transport and UI remain independent of whether the editor is open.
use super::*;
use crate::{theme::Palette, ui::widgets, world::bridge::QueryState};

impl App {
    /// AURORA_DEMO_COUPLE: Nova stands beside you and accepts an Ask from
    /// the animation editor's couple section 1.5 s later, as a resident
    /// would. `leave`: 10 s after she becomes Person B she teleports away
    /// (her avatar is killed), and stays drawn and posable. `clip`,
    /// `hover` and `mirror` script the screenshots (below).
    pub(super) fn couple_demo(&mut self, mode: &str) {
        let peer = crate::demo::poser::PEER;
        if self.frame_count == 400
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![crate::demo::poser::peer()],
            });
        }
        if self.world.objects.index_of_uuid(&peer).is_some() {
            let mut consent = std::mem::take(&mut self.world.poser_consent);
            consent.demo_accept(&self.world, peer, std::time::Duration::from_millis(1500));
            self.world.poser_consent = consent;
        }
        // the scripted modes, for screenshots (with
        // AURORA_DEMO_AVATAR_STUDIO=animation-editor, which opens the
        // editor at frame 800): Nova is asked and becomes Person B
        // (`mirror`: a mirror of you is Person B instead)
        let scripted = matches!(mode, "clip" | "hover" | "mirror");
        if scripted && self.frame_count >= 860 && self.frame_count.is_multiple_of(30) && self.world.couple_partner.is_none() {
            if mode == "mirror" {
                self.ik_editor.demo_mirror_step();
            } else if self.ik_editor.demo_couple_step(&self.world, peer) {
                log::info!("demo couple: Nova is Person B (frame {})", self.frame_count);
            }
        }
        // once the world has B (the Studio drops B's hover until then): a
        // side view, so that face to face B is not hidden behind you, and
        // in `hover` B's own hover chosen in the menu
        let b = if mode == "mirror" { self.world.mirror() } else { Some(peer) };
        if scripted && b.is_some() && self.world.couple_partner == b && self.couple_demo_since.is_none() {
            self.couple_demo_since = Some(Instant::now());
            self.avatar_studio.yaw = (self.avatar_studio.facing + std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU);
            if mode == "hover" {
                self.avatar_studio.hover_height = 0.1;
                self.avatar_studio.partner_hover_height = Some(0.3);
                self.avatar_studio.hover_partner = true;
            }
        }
        if mode == "leave" && self.world.couple_partner == Some(peer) {
            let since = *self.couple_demo_since.get_or_insert(Instant::now());
            if since.elapsed() >= std::time::Duration::from_secs(10)
                && let Some(key) = self
                    .world
                    .objects
                    .index_of_uuid(&peer)
                    .and_then(|i| self.world.objects.get(i))
                    .map(|o| o.key)
            {
                log::info!("demo couple: Person B teleports away");
                self.world.apply(aurora_net::NetEvent::ObjectsKilled {
                    handle: key.region,
                    local_ids: vec![key.local_id],
                });
                self.couple_demo_since = Some(Instant::now() + std::time::Duration::from_secs(3600));
            }
        }
    }
    pub(super) fn poser_demo(&mut self, mode: &str) {
        if self.frame_count == 300
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![crate::demo::action_avatar(false)],
            });
        }
        // square-head: our shape's Head Shape slider at 100, which turns the
        // forehead joints' scale negative on one axis (#1249)
        if mode == "square-head" && self.frame_count == 300 {
            let values = self.world.avatar_lib.shape_params.values_with(&[(193, 1.0)]);
            self.world.preview_shape(Some(values));
        }
        // shape-change: our Height slider at 100 after the head edit, with the
        // pose active, which must follow the new shape at once (#2415)
        if mode == "shape-change" && self.frame_count == 600 {
            let values = self.world.avatar_lib.shape_params.values_with(&[(33, 1.0)]);
            self.world.preview_shape(Some(values));
        }
        // bridge-* modes show the subject menu's bridge hints for the same peer.
        // compact*: the same peer, a friend nearby to ask from the subject
        // menu; compact-accept has her accept at once
        let fixture = mode.starts_with("consent-") || mode.starts_with("bridge-") || mode.starts_with("compact");
        if fixture
            && self.frame_count == 400
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![crate::demo::poser::peer()],
            });
        }
        if mode == "compact-crowd" {
            self.poser_crowd_demo();
        }
        self.ik_editor.demo_tick(&self.world, self.frame_count, &format!("poser-{mode}"));
        // compact-save: Save's form opens (`poser::compact`)
        if mode == "compact-save" && self.frame_count == 490 {
            self.egui_ctx.data_mut(|d| d.insert_temp(egui::Id::unique("poser-save-open"), true));
        }
        if self.frame_count == 420 {
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.contacts = false;
            self.panels.minimap = false;
            self.panels.perf = false;
            if mode.starts_with("compact") {
                widgets::place_floater(
                    &self.egui_ctx,
                    "poser-compact",
                    egui::pos2(1000.0, 90.0),
                    crate::ik_edit::poser::COMPACT_SIZE,
                );
            } else if mode == "frame" {
                // the framed joint sits at the centre of the view: the
                // window stays on the left, clear of it
                widgets::place_floater(&self.egui_ctx, "poser", egui::pos2(10.0, 70.0), egui::vec2(700.0, 760.0));
            } else {
                widgets::place_floater(
                    &self.egui_ctx,
                    "poser",
                    egui::pos2(if mode == "connected" { 780.0 } else { 650.0 }, 70.0),
                    egui::vec2(880.0, 760.0),
                );
            }
        }
        if mode == "animesh" && self.frame_count == 440 {
            let target = self
                .world
                .objects
                .iter()
                .find(|(_, o)| o.key.local_id == 9200)
                .map(|(i, o)| (i, o.full_id));
            if let Some((index, id)) = target {
                if let Some(object) = self.world.objects.get_mut(index) {
                    object.owner_id = self.world.agent_id;
                }
                self.ik_editor.select_pose_subject(&self.world, id);
            }
        }
        if fixture {
            let peer = self
                .world
                .objects
                .index_of_uuid(&crate::demo::poser::PEER)
                .map(|_| crate::demo::poser::PEER);
            if let Some(peer) = peer {
                if self.frame_count == 440 && !self.world.social.friends.iter().any(|f| f.id == peer) {
                    self.world.social.friends.push(crate::world::social::Friend {
                        id: peer,
                        online: true,
                        rights_given: 1,
                        rights_has: 1,
                    });
                }
                // compact-long accepts too: its head and menu show the long names
                if mode.starts_with("consent-") || mode == "compact-accept" || mode == "compact-long" {
                    let consent_mode = if mode.starts_with("compact") { "consent-accept" } else { mode };
                    let mut consent = std::mem::take(&mut self.world.poser_consent);
                    consent.demo(&self.world, peer, consent_mode, self.frame_count);
                    self.world.poser_consent = consent;
                }
                // compact-long: the longest names Second Life allows (a
                // 31-letter display name, a long legacy account), kept over
                // the demo's own names reply
                if mode == "compact-long" {
                    self.world.social.avatar_names.on_display_names(
                        &[aurora_net::AvatarNameData {
                            id: peer,
                            username: "wolfgangmaximilianwanderwood.wynterbourne".into(),
                            display_name: "Wolfgang Maximilian Wanderwood".into(),
                            legacy_first: "wolfgangmaximilianwanderwood".into(),
                            legacy_last: "Wynterbourne".into(),
                            ..Default::default()
                        }],
                        &[],
                        Some(3600),
                    );
                }
                // compact*: Nova answers yes to an Ask of yours, the demo
                // having no one behind her
                if mode.starts_with("compact") {
                    let mut consent = std::mem::take(&mut self.world.poser_consent);
                    consent.demo_accept(&self.world, peer, std::time::Duration::from_millis(1500));
                    self.world.poser_consent = consent;
                }
                if self.frame_count == 440 && (!mode.starts_with("compact") || mode == "compact-invite" || mode == "compact-long") {
                    self.egui_ctx
                        .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
                }
                if self.frame_count == 490
                    && (mode == "consent-accept" || mode == "consent-withdraw" || mode == "compact-accept" || mode == "compact-long")
                {
                    self.ik_editor.select_pose_subject(&self.world, peer);
                }
            }
        }
        if self.frame_count == 500 {
            let connected = mode == "connected";
            let actor = self
                .ik_editor
                .poser_focus(&self.world, if connected { "mWristLeft" } else { "mPelvis" })
                .unwrap_or(self.world.agent.position);
            let eye = actor
                + if connected {
                    Vec3::new(-0.9, -1.3, 0.3)
                } else if mode == "camera" {
                    Vec3::new(4.0, -4.0, 1.5)
                } else {
                    Vec3::new(-4.0, -4.0, 1.5)
                };
            let focus = actor
                + if connected {
                    Vec3::new(0.4, -0.6, 0.0)
                } else if mode == "camera" {
                    Vec3::new(2.0, 2.0, 0.0)
                } else {
                    Vec3::new(2.0, -2.0, 0.0)
                };
            log::info!("demo poser {mode}: rendered focus {actor:?}, eye {eye:?}, target {focus:?}");
            self.camera.set_view(eye, focus, &mut self.world.agent, &self.settings.camera);
        }
    }
    /// AURORA_DEMO_POSER=compact-crowd: five residents around you (400),
    /// their names (kept over the demo's own reply), asked at 450, all
    /// accepting at 480, and the subject menu open at 500.
    fn poser_crowd_demo(&mut self) {
        use crate::demo::poser::{CROWD, crowd, crowd_id};
        if self.frame_count == 400
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates { handle, objects: crowd() });
        }
        if self.frame_count < 400 {
            return;
        }
        let names: Vec<_> = CROWD
            .iter()
            .enumerate()
            .map(|(i, (display, username, last))| aurora_net::AvatarNameData {
                id: crowd_id(i),
                username: (*username).into(),
                display_name: (*display).into(),
                legacy_first: display.split(' ').next().unwrap_or(display).into(),
                legacy_last: (*last).into(),
                ..Default::default()
            })
            .collect();
        self.world.social.avatar_names.on_display_names(&names, &[], Some(3600));
        let mut consent = std::mem::take(&mut self.world.poser_consent);
        for i in 0..CROWD.len() {
            if self.frame_count == 450 {
                consent.request(&self.world, crowd_id(i), Instant::now());
            }
            if self.frame_count == 480 {
                consent.demo_accept(&self.world, crowd_id(i), std::time::Duration::ZERO);
            }
        }
        self.world.poser_consent = consent;
        if self.frame_count == 500 {
            self.egui_ctx
                .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
        }
    }
    pub(super) fn tick_poser_consent(&mut self) {
        let mut consent = std::mem::take(&mut self.world.poser_consent);
        consent.sync(&self.world, Instant::now());
        let bridge = match self.demo.then(|| std::env::var("AURORA_DEMO_POSER").ok()).flatten() {
            // The demo has no worn bridge: its poser fixtures stand for a
            // ready one, unless a bridge-* mode shows a waiting state.
            Some(mode) => match mode.as_str() {
                "bridge-starting" => QueryState::Starting,
                "bridge-silent" => QueryState::Silent,
                "bridge-failed" => QueryState::Failed,
                "bridge-off" => QueryState::Detached,
                _ => QueryState::Ready,
            },
            None => self.bridge_runtime.query_state(Instant::now()),
        };
        self.ik_editor.set_poser_bridge(bridge);
        for command in consent.commands(self.bridge_runtime.query_endpoint()) {
            self.net.send(command);
        }
        self.world.poser_consent = consent;
    }
    /// The poser's consent requests, and the notice that one ended; the
    /// question itself is our bridge's dialog on the resident's screen.
    /// `over_photo` in the full-frame photo mode, where the poser window is
    /// raised to the foreground (`over_viewfinder`) and the notice must stay
    /// above it.
    pub(super) fn poser_consent_ui(&mut self, ctx: &egui::Context, p: &Palette, over_photo: bool) {
        let mut consent = std::mem::take(&mut self.world.poser_consent);
        // The requester sees every nearby avatar with Ask in the poser's
        // subject menu; there is no capability probe.
        for request in self.ik_editor.take_poser_requests() {
            match request {
                crate::ik_edit::poser::Request::Ask(peer) => {
                    consent.request(&self.world, peer, Instant::now());
                }
                crate::ik_edit::poser::Request::Cancel(peer) => consent.cancel(peer),
                // The Movelock window's Repair: recompile and restart both scripts.
                crate::ik_edit::poser::Request::RepairBridge => self.bridge_runtime.retry(Instant::now()),
            }
        }
        // in the photo mode, the notice stays over the poser window it
        // raises to the same layer (elsewhere the windows keep their order)
        let mut on_top = None;
        if let Some(notice) = consent.notice {
            let area = egui::Area::new(egui::Id::unique("poser-consent-notice"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 52.0))
                .show(ctx, |ui| {
                    widgets::overlay(ui, p, egui::Margin::same(10), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(aurora_i18n::text(notice));
                            if ui.button(aurora_i18n::tr!("common-close")).clicked() {
                                consent.notice = None;
                            }
                        });
                    });
                });
            on_top = Some(area.response.layer_id);
        }
        // never over another dialog (the exit's question): raised each frame,
        // it would take its place as the top modal and leave it unclickable
        if over_photo
            && ctx.memory(|m| m.top_modal_layer()).is_none()
            && let Some(layer) = on_top
        {
            ctx.move_to_top(layer);
        }
        self.world.poser_consent = consent;
    }
}
