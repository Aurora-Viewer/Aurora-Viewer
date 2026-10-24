//! Consent transport and UI remain independent of whether the editor is open.
use super::*;
use crate::{
    theme::Palette,
    ui::widgets,
    world::poser_consent::{self, Status},
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
        if mode.starts_with("consent-")
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
                egui::pos2(if mode == "connected" { 800.0 } else { 650.0 }, 70.0),
                egui::vec2(800.0, 736.0),
            );
        }
        if mode == "library" && self.frame_count == 451 {
            widgets::size_floater(&self.egui_ctx, "poser", egui::vec2(1030.0, 736.0));
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
        if mode.starts_with("consent-") {
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
                let mut consent = std::mem::take(&mut self.world.poser_consent);
                consent.demo(&self.world, peer, mode, self.frame_count);
                self.world.poser_consent = consent;
                if self.frame_count == 440 {
                    self.egui_ctx
                        .data_mut(|d| d.insert_temp(egui::Id::unique("poser-people-open"), true));
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
        for command in consent.commands(self.bridge_runtime.query_endpoint()) {
            self.net.send(command);
        }
        self.world.poser_consent = consent;
    }
    pub(super) fn poser_consent_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        let mut consent = std::mem::take(&mut self.world.poser_consent);
        let mut choose = None;
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
        // The requester sees every nearby avatar with Ask; there is no capability probe.
        if self.ik_editor.poser_open() {
            let id = egui::Id::unique("poser-people-open");
            let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
            if open {
                let floater = widgets::Floater::new(
                    "poser-people",
                    aurora_i18n::tr!("poser-other-avatars"),
                    egui::pos2(600.0, 100.0),
                    egui::vec2(410.0, 400.0),
                );
                floater.show(ctx, p, &mut open, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (_, object) in self.world.objects.avatars() {
                            let peer = object.full_id;
                            if !poser_consent::nearby(&self.world, peer) {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.label(poser_consent::name(&self.world, peer));
                                let status = consent.status(peer);
                                let key = match status {
                                    Status::Ask => "poser-consent-ask",
                                    Status::Pending => "poser-consent-pending",
                                    Status::Accepted => "poser-consent-ready",
                                    Status::Refused => "poser-consent-refused",
                                    Status::NoAnswer => "poser-consent-no-answer",
                                };
                                if ui
                                    .add_enabled(
                                        status != Status::Refused && status != Status::NoAnswer,
                                        egui::Button::new(aurora_i18n::text(key)),
                                    )
                                    .clicked()
                                {
                                    match status {
                                        Status::Ask => {
                                            consent.request(&self.world, peer, Instant::now());
                                        }
                                        Status::Pending => consent.cancel(peer),
                                        Status::Accepted => choose = Some(peer),
                                        _ => {}
                                    }
                                }
                            });
                        }
                    });
                });
            }
            ctx.data_mut(|d| d.insert_temp(id, open));
        }
        self.world.poser_consent = consent;
        if let Some(peer) = choose {
            self.ik_editor.select_pose_subject(&self.world, peer);
        }
    }
}
