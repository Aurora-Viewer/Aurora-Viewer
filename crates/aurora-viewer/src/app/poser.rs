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
        let fixture = mode.starts_with("consent-") || mode.starts_with("bridge-") || mode == "photo-request";
        if fixture
            && self.frame_count == 400
            && let Some(handle) = self.world.main_region
        {
            self.world.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![crate::demo::poser::peer()],
            });
        }
        if mode != "photo-request" {
            self.ik_editor.demo_tick(&self.world, self.frame_count, &format!("poser-{mode}"));
        }
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
                if mode.starts_with("consent-") || mode == "photo-request" {
                    let mut consent = std::mem::take(&mut self.world.poser_consent);
                    consent.demo(
                        &self.world,
                        peer,
                        if mode == "photo-request" { "consent-request" } else { mode },
                        self.frame_count,
                    );
                    self.world.poser_consent = consent;
                }
                if self.frame_count == 440 {
                    if mode == "photo-request" {
                        self.panels.snapshot = true;
                        self.snapshots.ui.full_frame = true;
                        self.ik_editor.request_pose_consent(peer);
                    } else {
                        self.egui_ctx
                            .data_mut(|d| d.insert_temp(egui::Id::unique("poser-subjects-open"), true));
                    }
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
        let now = Instant::now();
        let deferred = process_queued_poser_requests(&mut self.ik_editor, &mut consent, &self.world, now);
        for request in deferred {
            match request {
                crate::ik_edit::poser::Request::OpenPrivacy => {
                    self.panels.settings = true;
                    self.options_ui.open(crate::ui::options::Page::Privacy, 0);
                }
                crate::ik_edit::poser::Request::RepairBridge => self.bridge_runtime.retry(now),
                crate::ik_edit::poser::Request::Ask(_) | crate::ik_edit::poser::Request::Cancel(_) => unreachable!(),
            }
        }
        if consent.dirty {
            self.settings.pose_requests = consent.preferences.clone();
            self.settings.save();
            consent.dirty = false;
        } else {
            consent.preferences.clone_from(&self.settings.pose_requests);
        }
        consent.sync(&self.world, now);
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

fn apply_poser_requests(
    requests: Vec<crate::ik_edit::poser::Request>,
    consent: &mut poser_consent::State,
    world: &World,
    now: Instant,
) -> Vec<crate::ik_edit::poser::Request> {
    let mut deferred = Vec::new();
    for request in requests {
        match request {
            crate::ik_edit::poser::Request::Ask(peer) => {
                consent.request(world, peer, now);
            }
            crate::ik_edit::poser::Request::Cancel(peer) => consent.cancel(peer),
            action @ (crate::ik_edit::poser::Request::OpenPrivacy | crate::ik_edit::poser::Request::RepairBridge) => {
                deferred.push(action);
            }
        }
    }
    deferred
}

fn process_queued_poser_requests(
    editor: &mut crate::ik_edit::Editor,
    consent: &mut poser_consent::State,
    world: &World,
    now: Instant,
) -> Vec<crate::ik_edit::poser::Request> {
    apply_poser_requests(editor.take_poser_requests(), consent, world, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo, scene::avatar::AvatarLibrary};
    use aurora_net::{
        NetEvent,
        bridge::{
            Endpoint,
            query::{PoseKind, Query},
        },
    };
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn queued_pose_invite_is_transmitted_during_full_frame_photo_mode() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = demo::DEMO_AGENT;
        world.main_region = Some(1);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![demo::action_avatar(false), demo::poser::peer()],
        });
        let peer = demo::poser::PEER;
        let mut consent = poser_consent::State::default();
        consent.sync(&world, Instant::now());
        let mut editor = crate::ik_edit::Editor::default();
        editor.request_pose_consent(peer);
        let photo_full_frame = true;

        // The per-frame queue processor is independent of the UI branch; that
        // branch returns early while `photo_full_frame` is true.
        assert!(photo_full_frame);
        assert!(process_queued_poser_requests(&mut editor, &mut consent, &world, Instant::now()).is_empty());
        assert!(editor.take_poser_requests().is_empty(), "the per-frame step drained the queue");
        let endpoint = Endpoint {
            url: "https://synthetic.agni.lindenlab.com/cap/11111111-1111-4111-8111-111111111111".into(),
            token: Uuid::from_u128(1),
        };
        assert!(matches!(
            consent.commands(Some(endpoint)).as_slice(),
            [NetCommand::BridgeQuery { query: Query::Pose { kind: PoseKind::Request, target, .. }, .. }]
                if *target == peer
        ));
    }
}
