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
        // bridge-* modes show the subject menu's bridge hints for the same peer.
        let fixture = mode.starts_with("consent-") || mode.starts_with("bridge-");
        if fixture
            && self.frame_count == 400
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![crate::demo::poser::peer()],
            });
        }
        self.ik_editor.demo_tick(&self.world, self.frame_count, &format!("poser-{mode}"));
        if self.frame_count == 420 {
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.contacts = false;
            self.panels.minimap = false;
            self.panels.perf = false;
            widgets::place_floater(
                &self.egui_ctx,
                "poser",
                egui::pos2(if mode == "connected" { 780.0 } else { 650.0 }, 70.0),
                egui::vec2(880.0, 760.0),
            );
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
                if mode.starts_with("consent-") {
                    let mut consent = std::mem::take(&mut self.world.poser_consent);
                    consent.demo(&self.world, peer, mode, self.frame_count);
                    self.world.poser_consent = consent;
                }
                if self.frame_count == 440 {
                    self.egui_ctx
                        .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
                }
                if self.frame_count == 490 && (mode == "consent-accept" || mode == "consent-withdraw") {
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
    pub(super) fn poser_consent_ui(&mut self, ctx: &egui::Context, p: &Palette) {
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
        if let Some(request) = consent.incoming.front().copied() {
            let id = egui::Id::unique(("poser-consent-block", request.id));
            let mut block = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            widgets::Modal::new(egui::Id::unique(("poser-consent", request.id))).show(ctx, |ui| {
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
        }
        if !consent.granted_to.is_empty() || consent.notice.is_some() {
            egui::Area::new(egui::Id::unique("poser-consent-reminder"))
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
        }
        self.world.poser_consent = consent;
    }
}
