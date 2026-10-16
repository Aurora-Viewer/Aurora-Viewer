//! AURORA_DEMO_OBJECT_MENU: the object menu entries on offline objects
//! (demo_object_menu.rs): the bell's menu, a script action and its window,
//! Return, Put on, Block particle owner, Link and Take multiple on a kept
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
                    self.open_context_menu();
                }
            }
            740 => {
                let bell = id_of(self, BELL);
                let action = match mode {
                    "scripts" | "scripts-run" => bell.map(|id| CtxAction::Scripts {
                        id,
                        action: crate::build::script_queue::QueueKind::Reset,
                    }),
                    "return" | "return-yes" => bell.map(CtxAction::Return),
                    "wear" => bell.map(|id| CtxAction::Wear { id, point: 6, add: true }),
                    "particle-block" => self.object_menu.particle_owner.map(CtxAction::BlockParticleOwner),
                    "drop" => Some(CtxAction::Drop(key(crate::demo::worn::ROOT))),
                    _ => None,
                };
                if let Some(action) = action {
                    log::info!("demo object menu: {action:?}");
                    self.context_menu = None;
                    self.on_ctx_action(action);
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
                    "demo object menu result: mode={mode}, bell={}, particle owner={:?}, blocked={}, queue={lines:?}",
                    if id_of(self, BELL).is_some() { "in the world" } else { "gone" },
                    self.object_menu.particle_owner,
                    self.object_menu.particle_owner.is_some_and(|o| self.world.mutes.is_muted_id(&o)),
                );
            }
            _ => {}
        }
    }

    /// The bell, the crate and the particles in a row across the view,
    /// 5 m ahead of the avatar where the camera looks.
    fn object_menu_fixture(&mut self) {
        let forward = self.camera.forward().with_z(0.0).normalize_or(Vec3::X);
        let side = Vec3::new(forward.y, -forward.x, 0.0);
        let center = self.world.agent.position + forward * 5.0;
        for ev in fixture::events(center, side) {
            self.world.apply(ev);
        }
        log::info!("demo object menu: objects around {center:?}");
    }
}
