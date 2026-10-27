//! Animation explorer actions, following AnimationExplorer
//! (indra/newview/animationexplorer.cpp, originally LGPL 2.1): rows from the
//! world's animation log, names of the sources, the stand-in avatar preview,
//! and Stop / Stop and revoke / Blacklist.
//!
//! Deliberate difference: the Legacy Viewer asks an object's name with an
//! ObjectSelect immediately followed by an ObjectDeselect. Aurora asks it as
//! a hover does (RequestObjectPropertiesFamily), which selects nothing.
use super::*;
use crate::scene::anim::{AnimStreamer, Controller};
use crate::ui::animation_explorer::{Action, Row};
use crate::world::derender::{Entry, Kind};
use aurora_render::preview::{Model, Pose};
use std::collections::HashSet;
use std::time::Duration;
use uuid::Uuid;

/// LLPermissions bits RevokePermissions clears: all of them, as
/// LLVOAvatar::revokePermissionsOnObject sends.
const ALL_PERMISSIONS: u32 = u32::MAX;

/// The stand-in avatar's hand pose and emote (they change its geometry).
type ModelKey = (u32, String);

/// LLPreviewAnimation's dummy avatar: the stand, and the selected motion over
/// it, on a controller of its own, never on our avatar.
#[derive(Default)]
pub(super) struct Preview {
    controller: Controller,
    /// The motion started, and when (None: the stand alone).
    motion: Option<(Option<Uuid>, Instant)>,
    pose: Option<Pose>,
    model: Option<(ModelKey, Arc<Model>)>,
    building: Option<(ModelKey, crossbeam_channel::Receiver<Arc<Model>>)>,
}

impl Preview {
    /// AnimationExplorer::startMotion when the selection changes, then one
    /// pose for this frame.
    fn tick(&mut self, selected: Option<Uuid>, rig: &crate::scene::anim::Rig, streamer: &mut AnimStreamer, now: Instant) {
        if self.motion.is_none_or(|(motion, _)| motion != selected) {
            // deactivateAllMotions, then ANIM_AGENT_STAND and the motion
            self.controller = Controller::for_control_avatar();
            self.motion = Some((selected, now));
        }
        let started = self.motion.map_or(now, |(_, at)| at);
        let mut playing = vec![crate::world::PlayingAnimation::from_signal(
            crate::own_motion::ANIM_STAND,
            0,
            started,
            None,
        )];
        if let Some(id) = selected {
            playing.push(crate::world::PlayingAnimation::from_signal(id, 1, started, None));
        }
        self.controller.sync(&playing, now, |id| streamer.get(id));
        let mut pose = vec![glam::Mat4::IDENTITY.to_cols_array_2d(); rig.len() + 1];
        self.controller.evaluate(rig, now, None, &mut pose);
        self.pose = Some(pose.into());
    }

    /// The stand-in avatar with the hand pose and emote of the motion shown,
    /// built on a worker; the previous one stays drawn meanwhile.
    fn model(
        &mut self,
        key: ModelKey,
        jobs: &crate::scene::jobs::Jobs,
        avatars: &Arc<AvatarLibrary>,
        ctx: &egui::Context,
    ) -> Option<Arc<Model>> {
        if let Some((built, rx)) = &self.building
            && let Ok(model) = rx.try_recv()
        {
            self.model = Some((built.clone(), model));
            self.building = None;
        }
        let current = self.model.as_ref().is_some_and(|(k, _)| *k == key);
        let pending = self.building.as_ref().is_some_and(|(k, _)| *k == key);
        if !current && !pending {
            let (tx, rx) = crossbeam_channel::bounded(1);
            let (avatars, wake, (hand, emote)) = (avatars.clone(), ctx.clone(), key.clone());
            jobs.execute(move || {
                let _ = tx.send(Arc::new(crate::imports::preview::dummy_avatar(&avatars, hand, &emote)));
                wake.request_repaint();
            });
            self.building = Some((key, rx));
        }
        self.model.as_ref().map(|(_, model)| model.clone())
    }
}

/// What the explorer keeps between frames besides its window.
#[derive(Default)]
pub(super) struct ExplorerState {
    pub(super) preview: Preview,
    /// The log's generation the rows were made from, and when.
    rows_from: Option<(u64, Instant)>,
    /// Objects whose name was asked once already.
    names_asked: HashSet<Uuid>,
}

impl App {
    pub(super) fn animation_explorer_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        // RecentAnimationList::addAnimation keeps our own motions only while
        // the window is open.
        self.world.animation_log.record_own = self.panels.animation_explorer;
        if !self.panels.animation_explorer {
            if self.animation_explorer.preview.motion.is_some() {
                self.animation_explorer.preview = Preview {
                    model: self.animation_explorer.preview.model.take(),
                    ..Default::default()
                };
            }
            return;
        }
        let now = Instant::now();
        let generation = self.world.animation_log.generation;
        // The "Played" times and playing states every second, new rows at once.
        if self
            .animation_explorer
            .rows_from
            .is_none_or(|(g, at)| g != generation || now.duration_since(at) >= Duration::from_secs(1))
        {
            self.animation_explorer.rows_from = Some((generation, now));
            self.animation_explorer_ui.rows = self.animation_rows();
        }
        let selected = self.animation_explorer_ui.selected_row().map(|r| r.animation);
        let rig = self.scene.avatar_lib.rig.clone();
        let state = &mut self.animation_explorer;
        state.preview.tick(selected, &rig, &mut self.scene.anims, now);
        let key = selected
            .and_then(|id| self.scene.anims.peek(&id))
            .map_or((1, String::new()), |b| (b.anim.hand_pose, b.anim.emote_name.clone()));
        let model = state.preview.model(key, &self.scene.jobs, &self.scene.avatar_lib, ctx);
        let pose = state.preview.pose.clone();
        ctx.request_repaint();
        let action = self
            .animation_explorer_ui
            .show(ctx, p, &mut self.panels.animation_explorer, model, pose);
        let Some(row) = self.animation_explorer_ui.selected_row().cloned() else {
            return;
        };
        match action {
            Some(Action::Stop) => self.stop_explored_animation(&row),
            Some(Action::StopAndRevoke) => {
                self.stop_explored_animation(&row);
                // revokePermissionsOnObject: only an object still around
                if self.world.objects.index_of_uuid(&row.source).is_some() {
                    self.send(NetCommand::RevokePermissions {
                        target: row.source,
                        permissions: ALL_PERMISSIONS,
                    });
                }
            }
            Some(Action::Blacklist) => {
                self.stop_explored_animation(&row);
                // FSAssetBlacklist::addNewItemToBlacklist(animation, played by, region)
                let region = self
                    .world
                    .main_region
                    .and_then(|h| self.world.regions.get(&h))
                    .map(|r| r.name.clone())
                    .unwrap_or_default();
                self.world.derender.add(
                    row.animation,
                    Entry {
                        name: row.name,
                        region,
                        kind: Kind::Animation,
                        date: crate::world::derender::now_secs(),
                        permanent: true,
                    },
                );
            }
            None => {}
        }
    }

    /// AnimationExplorer::onStopPressed: the simulator stops it on our avatar.
    fn stop_explored_animation(&mut self, row: &Row) {
        self.send(NetCommand::AgentAnimation {
            anim: row.animation,
            start: false,
        });
    }

    /// AnimationExplorer::addAnimation / updateList, newest first. Names
    /// missing are asked once: a resident's from the name cache, an
    /// object's from its simulator.
    fn animation_rows(&mut self) -> Vec<Row> {
        let entries: Vec<_> = self.world.animation_log.entries().rev().copied().collect();
        let mut rows = Vec::with_capacity(entries.len());
        for entry in entries {
            let own = entry.source == self.world.agent_id;
            let object = self
                .world
                .objects
                .index_of_uuid(&entry.source)
                .and_then(|i| self.world.objects.get(i));
            let avatar = own || object.is_some_and(|o| o.is_avatar());
            let region = object.map(|o| o.key.region);
            let name = if avatar {
                self.world.social.want_name(entry.source);
                if self.world.rlva_can_name(entry.source) {
                    self.world.social.name_of(&entry.source)
                } else {
                    crate::world::rlva::privacy::anonymous(entry.source).display_name.clone()
                }
            } else if let Some(name) = self.known_object_name(&entry.source) {
                name
            } else {
                if let Some(handle) = region
                    && self.animation_explorer.names_asked.insert(entry.source)
                {
                    self.send(NetCommand::RequestObjectProperties {
                        handle,
                        object: entry.source,
                    });
                }
                entry.source.to_string()
            };
            rows.push(Row {
                id: entry.id,
                animation: entry.animation,
                source: entry.source,
                name,
                own,
                playing: self.world.animation_playing_from(&entry.source, &entry.animation),
                started: entry.started,
                priority: self.scene.anims.peek(&entry.animation).map(|b| b.anim.base_priority),
            });
        }
        rows
    }

    /// AURORA_DEMO_ANIMATION_EXPLORER=1|blacklist: a pose ball dances our
    /// avatar, a worn object's motion has stopped, our own stand is listed;
    /// `blacklist` also lists the stopped motion in the Asset blacklist.
    pub(super) fn demo_animation_explorer(&mut self, mode: &str) {
        self.panels.animation_explorer = true;
        self.panels.chat = false;
        self.panels.people = false;
        self.panels.appearance = false;
        self.world.animation_log.record_own = true;
        let rig = self.scene.avatar_lib.rig.clone();
        let (dance, wave) = (crate::demo::CROWD_DANCE_ANIM, crate::demo::EXPLORER_WAVE_ANIM);
        self.scene.anims.insert(
            dance,
            Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::crowd_dance_animation(), &rig)),
        );
        self.scene.anims.insert(
            wave,
            Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::loop_animation(), &rig)),
        );
        // Two objects of the plaza play them, named as the simulator would.
        let objects: Vec<_> = self
            .world
            .objects
            .iter()
            .filter(|(_, o)| !o.is_avatar() && o.parent_id == 0)
            .map(|(_, o)| o.full_id)
            .take(2)
            .collect();
        let [ball, hud] = objects[..] else {
            log::warn!("animation explorer demo: no plaza objects");
            return;
        };
        let names = [(ball, "Dance ball"), (hud, "Greeter HUD")];
        self.interactions
            .on_properties(&names.map(|(object_id, name)| aurora_net::build::ObjectProps {
                object_id,
                name: name.into(),
                ..Default::default()
            }));
        let agent = self.world.agent_id;
        for (anims, sources) in [
            (vec![(crate::demo::IDLE_ANIM, 2), (wave, 1)], vec![agent, hud]),
            (vec![(crate::demo::IDLE_ANIM, 2), (dance, 1)], vec![agent, ball]),
        ] {
            self.world.apply(NetEvent::AvatarAnimations {
                avatar: agent,
                anims,
                sources,
            });
        }
        self.animation_explorer_ui.selected = self.world.animation_log.entries().find(|e| e.animation == dance).map(|e| e.id);
        if mode == "blacklist" {
            // Isolate the fixture from the demo account's saved list.
            self.world.derender = Default::default();
            self.world.derender.add(
                wave,
                Entry {
                    name: "Greeter HUD".into(),
                    region: String::new(),
                    kind: Kind::Animation,
                    date: crate::world::derender::now_secs(),
                    permanent: true,
                },
            );
            self.panels.animation_explorer = false;
            self.panels.asset_blacklist = true;
        }
    }
}
