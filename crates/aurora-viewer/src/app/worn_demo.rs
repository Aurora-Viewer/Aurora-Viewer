//! AURORA_DEMO_WORN: the scripted right click, "Edit", arrow drag and
//! "Inspect" on the object our demo avatar wears (demo_worn.rs).
use super::*;
use crate::ui::context::CtxAction;
use crate::world::objects::ObjKey;

impl App {
    pub(super) fn demo_worn_steps(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_WORN") else {
            return;
        };
        if matches!(mode.as_str(), "temporary-detach" | "temporary-drop") {
            self.demo_temporary_detach_steps(mode == "temporary-detach");
            return;
        }
        if mode == "temporary" {
            self.demo_temporary_steps();
            return;
        }
        let freeze = mode == "freeze";
        let (edit, inspect) = (matches!(mode.as_str(), "edit" | "move" | "freeze"), mode == "inspect");
        if !(edit || inspect || mode == "menu") {
            return;
        }
        if freeze {
            self.demo_worn_freeze_steps();
        }
        let key = ObjKey {
            region: self.world.main_region.unwrap_or_default(),
            local_id: crate::demo::worn::ROOT,
        };
        let Some(idx) = self.world.objects.index_of(&key) else {
            return;
        };
        let now = Instant::now();
        match self.frame_count {
            // right click on the root prim, where it is drawn
            700 => {
                if let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, now, 0)
                    && let Some(cursor) = self.build.cam.project_px(pos)
                {
                    self.cursor_pos = cursor;
                    self.open_context_menu();
                }
            }
            740 if edit => {
                self.on_ctx_action(CtxAction::EditAttachment(key));
                self.context_menu = None;
            }
            740 if inspect => {
                if let Some(id) = self.world.objects.get(idx).map(|o| o.full_id) {
                    self.on_ctx_action(CtxAction::Inspect(id));
                }
                self.context_menu = None;
            }
            // the arrow along the attachment point's Z, 45 px from the pivot
            780 if mode == "move" => {
                if let Some(b) = self.build.bounds(&self.world, now)
                    && let Some((_, frame)) = crate::build::edit_parent(&self.world, idx, now)
                {
                    let handle = b.center + frame * Vec3::Z * self.build.cam.meters_for_pixels(b.center, 45.0);
                    if let Some(cursor) = self.build.cam.project_px(handle) {
                        self.cursor_pos = cursor;
                        self.click(click_pick::Click::Build);
                    }
                }
            }
            781..=820 if mode == "move" => self.cursor_pos.1 -= 2.0,
            830 if mode == "move" => {
                self.on_left_release();
                if let Some(o) = self.world.objects.get(idx) {
                    log::info!(
                        "demo worn edit: local position={:?}, attachment={}",
                        o.position,
                        o.attachment_point()
                    );
                }
            }
            _ => {}
        }
    }

    /// A temporary attachment's linked script starts a dance, then removes
    /// the attachment without sending an animation stop of its own.
    /// AURORA_DEMO_WORN=temporary: at frame 600 an experience attaches a
    /// temporary object to the left hand, and Appearance opens on Worn, its
    /// "Temporary attachments" section named by AttachmentResources.
    fn demo_temporary_steps(&mut self) {
        if self.frame_count != 600 {
            return;
        }
        let Some(handle) = self.world.main_region else {
            return;
        };
        self.world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: crate::demo::worn::temporary(),
        });
        self.panels.appearance = true;
        self.appearance_ui.open(2, false);
        log::info!("demo temporary attachments: {:?}", self.world.temp_attachments());
    }

    fn demo_temporary_detach_steps(&mut self, killed: bool) {
        let agent = self.world.agent_id;
        let anim = crate::demo::CROWD_DANCE_ANIM;
        let Some(handle) = self.world.main_region else {
            return;
        };
        match self.frame_count {
            600 => {
                let mut root = crate::demo::worn::root();
                root.name_values = format!("AttachItemID STRING RW SV {}", uuid::Uuid::nil());
                self.world.apply(NetEvent::ObjectUpdates {
                    handle,
                    objects: vec![root],
                });
                let child = ObjKey {
                    region: handle,
                    local_id: crate::demo::worn::CHILD,
                };
                let Some(source) = self
                    .world
                    .objects
                    .index_of(&child)
                    .and_then(|i| self.world.objects.get(i))
                    .map(|o| o.full_id)
                else {
                    return;
                };
                let rig = self.scene.avatar_lib.rig.clone();
                self.scene.anims.insert(
                    anim,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::crowd_dance_animation(), &rig)),
                );
                self.world.apply(NetEvent::AvatarAnimations {
                    avatar: agent,
                    anims: vec![(anim, 1)],
                    sources: vec![source],
                });
            }
            900 => {
                let detach = if killed {
                    NetEvent::ObjectsKilled {
                        handle,
                        local_ids: vec![crate::demo::worn::ROOT],
                    }
                } else {
                    let mut root = crate::demo::worn::root();
                    root.parent_id = 0;
                    root.name_values = format!("AttachItemID STRING RW SV {}", uuid::Uuid::nil());
                    NetEvent::ObjectUpdates {
                        handle,
                        objects: vec![root],
                    }
                };
                self.world.apply(detach);
            }
            750 | 1050 => log::info!(
                "demo temporary detach: frame {} killed={killed} dancing={}",
                self.frame_count,
                self.world.animations_of(&agent).iter().any(|a| a.id == anim)
            ),
            _ => {}
        }
    }

    /// AURORA_DEMO_WORN=freeze: our avatar dances, the worn object is
    /// right-clicked (700) and edited (740), and the build window closes at
    /// 1100: the dance stands still in between (anim_pause.rs).
    fn demo_worn_freeze_steps(&mut self) {
        let agent = self.world.agent_id;
        match self.frame_count {
            600 => {
                let rig = self.scene.avatar_lib.rig.clone();
                self.scene.anims.insert(
                    crate::demo::CROWD_DANCE_ANIM,
                    Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::crowd_dance_animation(), &rig)),
                );
                self.world.apply(NetEvent::AvatarAnimations {
                    sources: Vec::new(),
                    avatar: agent,
                    anims: vec![(crate::demo::CROWD_DANCE_ANIM, 1)],
                });
            }
            1100 => self.build.close(&mut self.world, &mut self.settings.build),
            _ => {}
        }
        if matches!(self.frame_count, 690 | 720 | 900 | 1099 | 1130 | 1200) {
            log::info!(
                "demo worn freeze: frame {} paused={} selection={} head={:.3?}",
                self.frame_count,
                self.scene.paused_skeletons.contains(&agent),
                self.build.selection.len(),
                self.scene.head_world(&self.world, &agent, Instant::now())
            );
        }
    }
}
