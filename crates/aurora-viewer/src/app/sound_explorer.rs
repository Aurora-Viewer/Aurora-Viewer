//! Sound Explorer actions, following NACLFloaterExploreSounds and
//! is_sound_blacklisted (indra/newview/NACLfloaterexploresounds.cpp,
//! indra/newview/llviewermessage.cpp). Preview never sends a network event.
use super::*;
use crate::ui::sound_explorer::{Action, Row};
use crate::world::derender::{Entry, Kind};
use std::time::Duration;
use uuid::Uuid;

impl App {
    pub(super) fn sound_explorer_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.panels.sound_explorer {
            return;
        }
        let now = Instant::now();
        if !self.sound_explorer_ui.paused && self.sound_explorer_ui.next_refresh.is_none_or(|at| now >= at) {
            self.sound_explorer_ui.next_refresh = Some(now + Duration::from_millis(250));
            self.sound_explorer_ui.rows = self
                .scene
                .sounds
                .history
                .entries()
                .map(|entry| {
                    if !entry.owner.is_nil() {
                        self.world.social.want_name(entry.owner);
                    }
                    let owner = sound_owner_name(&self.world, entry.owner);
                    Row {
                        entry: entry.clone(),
                        owner,
                    }
                })
                .collect();
        }
        // Refresh privacy-sensitive names even while the log itself is paused.
        for row in &mut self.sound_explorer_ui.rows {
            row.owner = sound_owner_name(&self.world, row.entry.owner);
        }
        let before = self.settings.audio.explorer;
        let action = self.sound_explorer_ui.show(
            ctx,
            p,
            &mut self.panels.sound_explorer,
            &mut self.settings.audio.explorer,
            self.scene.sounds.previewing(),
        );
        if before != self.settings.audio.explorer {
            self.settings.save();
        }
        let engine = self.audio_engine.as_ref();
        match action {
            Some(Action::Play) => {
                self.scene
                    .sounds
                    .preview(self.sound_explorer_ui.selected_rows().map(|r| r.entry.asset));
            }
            Some(Action::StopLocal) => self.scene.sounds.stop_previews(engine),
            Some(Action::Stop) => {
                let ids: Vec<_> = self.sound_explorer_ui.selected.iter().copied().collect();
                self.scene.sounds.stop_logged(engine, &ids);
            }
            Some(Action::LookAt(id)) => {
                if self.world.rlva.has("setcam_unlock") {
                    return;
                }
                if let Some(row) = self.sound_explorer_ui.rows.iter().find(|r| r.entry.id == id) {
                    let live = self
                        .world
                        .objects
                        .index_of_uuid(&row.entry.source)
                        .and_then(|i| Scene::object_transform(&self.world, i, now, 0))
                        .filter(|t| !t.2)
                        .map(|t| t.0);
                    let (x, y) = self.world.main_origin().unwrap_or_default();
                    let origin = glam::DVec3::new(f64::from(x), f64::from(y), 0.0);
                    let target = live.unwrap_or((row.entry.position - origin).as_vec3());
                    if !target.is_finite() {
                        return;
                    }
                    let eye = target + (self.world.agent.position - target).normalize_or(-Vec3::X) * 4.0 + Vec3::Z * 3.0;
                    self.camera.set_view(eye, target, &mut self.world.agent, &self.settings.camera);
                }
            }
            Some(Action::Blacklist(flag)) => {
                let rows: Vec<_> = self.sound_explorer_ui.selected_rows().cloned().collect();
                let entries = rows
                    .into_iter()
                    .map(|row| {
                        let id = if flag == 0 { row.entry.asset } else { row.entry.owner };
                        let region = self
                            .world
                            .main_region
                            .and_then(|h| self.world.regions.get(&h))
                            .map(|r| r.name.clone())
                            .unwrap_or_default();
                        (
                            id,
                            Entry {
                                name: row.owner,
                                region,
                                kind: Kind::Sound(flag),
                                date: crate::world::derender::now_secs(),
                                permanent: true,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                self.world.derender.add_sounds(entries);
            }
            Some(Action::BlacklistWindow) => ui::widgets::open_floater(ctx, ui::blacklist::WINDOW, &mut self.panels.asset_blacklist),
            None => {}
        }
    }

    pub(super) fn set_owner_sound_block(&mut self, owner: Uuid, flag: u8, blocked: bool) {
        let name = sound_owner_name(&self.world, owner);
        let region = self
            .world
            .main_region
            .and_then(|h| self.world.regions.get(&h))
            .map(|r| r.name.clone())
            .unwrap_or_default();
        self.world.derender.set_sound_category(
            owner,
            flag,
            blocked,
            Entry {
                name,
                region,
                kind: Kind::Sound(flag),
                date: crate::world::derender::now_secs(),
                permanent: true,
            },
        );
    }

    pub(super) fn demo_sound_explorer(&mut self, mode: &str) {
        self.panels.sound_explorer = true;
        self.panels.chat = false;
        self.panels.people = false;
        if mode.starts_with("blacklist") {
            self.panels.sound_explorer = false;
            self.panels.asset_blacklist = true;
            // Isolate the fixture from the demo account's saved preferences.
            self.world.derender = Default::default();
            let owner = crate::demo::DEMO_NOVA;
            self.world.derender.add_sounds([
                (
                    owner,
                    Entry {
                        name: aurora_i18n::tr!("common-resident").into(),
                        region: String::new(),
                        kind: Kind::Sound(7),
                        date: crate::world::derender::now_secs(),
                        permanent: true,
                    },
                ),
                (
                    Uuid::from_u128(0x101),
                    Entry {
                        name: aurora_i18n::tr!("sound-explorer-asset").into(),
                        region: String::new(),
                        kind: Kind::Sound(0),
                        date: crate::world::derender::now_secs(),
                        permanent: true,
                    },
                ),
            ]);
            // These lists have separate persistence. Keep both offline fixtures
            // in memory, including the mute cache saved at logout.
            self.world.cache_dir = None;
            self.world.mutes.reset(self.world.agent_id);
            self.world.mutes.state = crate::world::mutes::LoadState::Loaded;
            let _ = self.world.block(
                Uuid::from_u128(0x102),
                aurora_i18n::tr!("common-object"),
                crate::world::mutes::MuteType::Object,
            );
            if mode == "blacklist-ignored" {
                self.panels.asset_blacklist = false;
                self.open_ignored_list();
            }
            return;
        }
        let (asset, clip) = crate::demo::chime();
        self.scene.sounds.insert(asset, clip);
        let collision = Uuid::from_u128(crate::scene::sounds::COLLISION[30]);
        self.scene.sounds.insert(collision, crate::demo::ui_sound(collision));
        let owner = crate::demo::DEMO_NOVA;
        for mut event in crate::demo::sound_events(asset) {
            match &mut event {
                NetEvent::AttachedSound { owner: id, .. } | NetEvent::SoundTrigger { owner: id, .. } => *id = owner,
                _ => {}
            }
            if let Some(event) = self.world.apply(event) {
                self.on_app_event(event);
            }
        }
        self.scene.sounds.trigger_owned(
            asset,
            self.world.agent.position,
            None,
            0.5,
            crate::scene::sounds::Origin { object: owner, owner },
        );
        self.scene.sounds.trigger_owned(
            collision,
            self.world.agent.position,
            None,
            0.5,
            crate::scene::sounds::Origin {
                object: Uuid::from_u128(42),
                owner,
            },
        );
        self.settings.audio.explorer.repeats = mode == "all";
        self.settings.audio.explorer.avatars = mode == "all";
        self.settings.audio.explorer.collisions = mode == "all";
        if mode.starts_with("avatar-menu") {
            self.panels.sound_explorer = false;
            self.panels.inventory = false;
            self.world.derender = Default::default();
            if mode.ends_with("-mixed") {
                self.set_owner_sound_block(owner, 1, true);
                self.set_owner_sound_block(owner, 4, true);
            }
            self.settings.use_pie_menu = mode.ends_with("-pie");
            self.context_menu = Some(ui::context::ContextMenu {
                pos: egui::pos2(40.0, 60.0),
                point: self.world.agent.position,
                target: ui::context::Target::Avatar {
                    id: owner,
                    own: false,
                    attachment: None,
                },
                pie_path: Vec::new(),
                pie_input_frame: None,
            });
        }
    }
}

fn sound_owner_name(world: &World, owner: Uuid) -> String {
    if owner.is_nil() {
        return aurora_i18n::tr!("sound-explorer-unknown").to_owned();
    }
    if !world.rlva_can_name(owner) {
        return crate::world::rlva::privacy::anonymous(owner).display_name.clone();
    }
    world.social.name_of(&owner)
}
