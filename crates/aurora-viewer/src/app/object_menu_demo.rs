//! AURORA_DEMO_OBJECT_MENU: the object menu entries on offline objects
//! (demo_object_menu.rs): the bell's menu, a script action and its window,
//! Return, Put on, owner particle filtering, Link and Take multiple on a kept
//! selection, Drop of the worn backpack (with AURORA_DEMO_WORN=1).
use super::*;
use crate::demo::object_menu::{self as fixture, BELL, CRATE, SPARKLES};
use crate::ui::context::CtxAction;
use crate::world::objects::ObjKey;

impl App {
    pub(super) fn object_menu_demo_steps(&mut self) {
        let Some(mode) = fixture::mode().filter(|_| self.demo) else {
            return;
        };
        let region = self.world.main_region.unwrap_or_default();
        let key = |local_id| ObjKey { region, local_id };
        let id_of = |app: &App, local| {
            app.world
                .objects
                .index_of(&key(local))
                .and_then(|i| app.world.objects.get(i))
                .map(|o| o.full_id)
        };
        match self.frame_count {
            // saved panels must not cover the objects
            580 => {
                self.panels.chat = false;
                self.panels.people = false;
                self.panels.inventory = false;
                self.panels.minimap = false;
                self.panels.settings = false;
            }
            600 => self.object_menu_fixture(),
            // a selection of both objects in the build tools (Link, Take multiple)
            650 if mode == "link" => {
                self.build.open_build(crate::build::Tool::Edit);
                for (local, extend) in [(CRATE, false), (BELL, true)] {
                    let idx = self.world.objects.index_of(&key(local));
                    self.build.click_select(&mut self.world, &self.settings.build, idx, extend);
                }
                self.flush_build();
            }
            700 => {
                let now = Instant::now();
                if matches!(mode, "profile-no-mod" | "profile-other")
                    && let Some(index) = self.world.objects.index_of(&key(BELL))
                    && let Some(object) = self.world.objects.get_mut(index)
                {
                    object.update_flags &= !ui::context::flags::OBJECT_MODIFY;
                    if mode == "profile-other" {
                        object.owner_id = crate::demo::DEMO_NOVA;
                        object.update_flags &= !ui::context::flags::OBJECT_YOU_OWNER;
                    }
                }
                // the particle cloud above its emitter, or the bell / the backpack
                let (local, lift) = match mode {
                    "particle" | "particle-block" => (SPARKLES, 0.45),
                    "drop" => (crate::demo::worn::ROOT, 0.0),
                    _ => (BELL, 0.0),
                };
                if let Some(idx) = self.world.objects.index_of(&key(local))
                    && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, now, 0)
                    && let Some(cursor) = self.build.cam.project_px(pos + Vec3::Z * lift)
                {
                    self.cursor_pos = cursor;
                    if !mode.starts_with("customize-") {
                        self.open_context_menu();
                    }
                }
            }
            740 => {
                if mode.starts_with("customize-") || matches!(mode, "avatar-menu" | "attachment-menu") {
                    let resident = crate::demo::DEMO_NOVA;
                    let target = match mode {
                        "customize-land" => Some(ui::context::Target::Ground),
                        "customize-self" => Some(ui::context::Target::Avatar {
                            id: self.world.agent_id,
                            own: true,
                            attachment: None,
                        }),
                        "customize-avatar" | "avatar-menu" => Some(ui::context::Target::Avatar {
                            id: resident,
                            own: false,
                            attachment: None,
                        }),
                        "customize-other-attachment" | "attachment-menu" => id_of(self, BELL).map(|object| ui::context::Target::Avatar {
                            id: resident,
                            own: false,
                            attachment: Some(object),
                        }),
                        "customize-hud" => {
                            self.world
                                .objects
                                .index_of(&key(crate::demo::hud::FIRST))
                                .map(|_| ui::context::Target::Attachment {
                                    key: key(crate::demo::hud::FIRST),
                                })
                        }
                        _ => None,
                    };
                    if let Some(target) = target {
                        self.context_menu = Some(ui::context::ContextMenu {
                            pos: egui::pos2(400.0, 140.0),
                            point: self.world.agent.position,
                            target,
                            pie_path: Vec::new(),
                            pie_input_frame: None,
                        });
                    }
                }
                let bell = id_of(self, BELL);
                let action = match mode {
                    mode if mode.starts_with("customize") => self.context_menu.clone().map(|m| CtxAction::CustomizeMenu(Box::new(m))),
                    "pbr" => bell.map(CtxAction::EditPbr),
                    "profile-hud" => id_of(self, crate::demo::hud::FIRST).map(CtxAction::ObjectProfile),
                    mode if mode.starts_with("profile") => bell.map(CtxAction::ObjectProfile),
                    "region-objects" | "characters" => bell.map(|id| CtxAction::Pathfinding {
                        id,
                        characters: mode == "characters",
                    }),
                    "reset-lod" => Some(CtxAction::ResetMeshLod(self.world.agent_id)),
                    "textures" => Some(CtxAction::ShowAvatarTextures(self.world.agent_id)),
                    "scripts" | "scripts-run" => bell.map(|id| CtxAction::Scripts {
                        id,
                        action: crate::build::script_queue::QueueKind::Reset,
                    }),
                    "return" | "return-yes" => bell.map(CtxAction::Return),
                    "wear" => bell.map(|id| CtxAction::Wear { id, point: 6, add: true }),
                    "particle-block" => self.object_menu.particle_owner.map(|id| CtxAction::SetParticleBlock {
                        id,
                        owner: true,
                        blocked: !self.settings.particle_filters.blocked(id, true),
                    }),
                    "drop" => Some(CtxAction::Drop(key(crate::demo::worn::ROOT))),
                    _ => None,
                };
                if let Some(action) = action {
                    log::info!("demo object menu: {action:?}");
                    self.context_menu = None;
                    self.on_ctx_action(action);
                }
                if mode == "profile-sale" {
                    self.send(aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::SetSaleInfo {
                        handle: region,
                        local_ids: vec![BELL],
                        sale_type: 2,
                        price: 25,
                    }));
                }
                if matches!(mode, "menu-export" | "menu-load") {
                    self.context_menu = None;
                    crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
                    self.context_customization.demo_notecard(mode == "menu-load");
                }
                // the question answered yes, without touching the saved
                // "ignore" choices
                if matches!(mode, "scripts-run" | "return-yes") {
                    self.object_menu_answer_yes();
                }
            }
            900 => {
                let lines = self.object_menu_queue_lines();
                log::info!(
                    "demo object menu result: mode={mode}, bell={}, particle owner={:?}, blocked={}, muted={}, live owner particles={}, queue={lines:?}",
                    if id_of(self, BELL).is_some() { "in the world" } else { "gone" },
                    self.object_menu.particle_owner,
                    self.object_menu
                        .particle_owner
                        .is_some_and(|id| self.settings.particle_filters.blocked(id, true)),
                    self.object_menu.particle_owner.is_some_and(|id| self.world.mutes.is_muted_id(&id)),
                    self.object_menu
                        .particle_owner
                        .map_or(0, |id| self.scene.particles.owner_particle_count(id)),
                );
            }
            _ => {}
        }
    }

    /// The bell, the crate and the particles in a row across the view,
    /// 5 m ahead of the avatar where the camera looks.
    fn object_menu_fixture(&mut self) {
        if matches!(fixture::mode(), Some("particle" | "particle-block")) {
            self.settings.particle_filters.set(crate::demo::DEMO_NOVA, true, false);
            let name = self.world.social.name_of(&crate::demo::DEMO_NOVA);
            self.world.unblock(crate::demo::DEMO_NOVA, &name, 0);
        }
        let forward = self.camera.forward().with_z(0.0).normalize_or(Vec3::X);
        let side = Vec3::new(forward.y, -forward.x, 0.0);
        let center = self.world.agent.position + forward * 5.0;
        for ev in fixture::events(center, side) {
            self.world.apply(ev);
        }
        log::info!("demo object menu: objects around {center:?}");
    }
}
