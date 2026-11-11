//! Consent transport and UI remain independent of whether the editor is open.
use super::*;
use crate::{
    theme::Palette,
    ui::widgets,
    world::{bridge::QueryState, poser_consent},
};

impl App {
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
        if consent.dirty {
            self.settings.pose_requests = consent.preferences.clone();
            self.settings.save();
            consent.dirty = false;
        } else {
            consent.preferences.clone_from(&self.settings.pose_requests);
        }
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
    /// The posing consent's question, reminder and requests; `over_photo`
    /// in the full-frame photo mode, where the poser window is raised to
    /// the foreground (`over_viewfinder`) and they must stay above it.
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
                crate::ik_edit::poser::Request::OpenPrivacy => {
                    self.panels.settings = true;
                    self.options_ui.open(crate::ui::options::Page::Privacy, 0);
                }
                // The Movelock window's Repair: recompile and restart both scripts.
                crate::ik_edit::poser::Request::RepairBridge => self.bridge_runtime.retry(Instant::now()),
            }
        }
        // in the photo mode, both stay over the poser window it raises to
        // the same layer (elsewhere the windows keep their usual order)
        let mut on_top = Vec::new();
        let mut question = None;
        if let Some(request) = consent.incoming.front().copied() {
            let id = egui::Id::unique(("poser-consent-block", request.id));
            let mut block = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            let modal = widgets::Modal::new(egui::Id::unique(("poser-consent", request.id))).show(ctx, |ui| {
                // in the photo mode Enter is the shutter, pressed out of
                // habit: it must not grant a resident the right to pose us,
                // so there the answer takes a click (stricter than the
                // dialogs' Enter rule, `modal_keys`)
                if over_photo {
                    widgets::enter_pressed(ui);
                }
                ui.set_max_width(430.0);
                let distance = poser_consent::distance(&self.world, request.peer).unwrap_or(0.0);
                ui.heading(aurora_i18n::tr!(
                    "poser-consent-request",
                    name = poser_consent::name(&self.world, request.peer),
                    distance = format!("{distance:.0}")
                ));
                ui.label(aurora_i18n::tr!("poser-consent-explanation"));
                widgets::checkbox(ui, p, &mut block, aurora_i18n::tr!("poser-consent-block"));
                ctx.data_mut(|d| d.insert_temp(id, block));
                ui.horizontal(|ui| {
                    if ui.button(aurora_i18n::tr!("poser-consent-refuse")).clicked() {
                        consent.answer(request, false, block, Instant::now());
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "check",
                        aurora_i18n::text("poser-consent-accept"),
                        widgets::ActionKind::Primary,
                        egui::vec2(0.0, 28.0),
                    )
                    .clicked()
                    {
                        consent.answer(request, true, block, Instant::now());
                    }
                });
            });
            question = Some(modal.response.layer_id);
            on_top.push(modal.response.layer_id);
        }
        if !consent.granted_to.is_empty() || consent.notice.is_some() {
            let reminder = egui::Area::new(egui::Id::unique("poser-consent-reminder"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 52.0))
                .show(ctx, |ui| {
                    widgets::overlay(ui, p, egui::Margin::same(10), |ui| {
                        let peers: Vec<_> = consent.granted_to.keys().copied().collect();
                        for peer in peers {
                            ui.horizontal(|ui| {
                                ui.label(aurora_i18n::tr!(
                                    "poser-consent-reminder",
                                    name = poser_consent::name(&self.world, peer)
                                ));
                                if ui.button(aurora_i18n::tr!("poser-consent-withdraw")).clicked() {
                                    consent.withdraw(peer);
                                }
                            });
                        }
                        if let Some(notice) = consent.notice {
                            ui.horizontal(|ui| {
                                ui.label(aurora_i18n::text(notice));
                                if ui.button(aurora_i18n::tr!("common-close")).clicked() {
                                    consent.notice = None;
                                }
                            });
                        }
                    });
                });
            on_top.push(reminder.response.layer_id);
        }
        // never over another dialog (the exit's question): raised each frame,
        // they would take its place as the top modal and leave it unclickable
        let top = ctx.memory(|m| m.top_modal_layer());
        if over_photo && (top.is_none() || top == question) {
            for layer in on_top {
                ctx.move_to_top(layer);
            }
        }
        self.world.poser_consent = consent;
    }
}
