//! AURORA_DEMO_WORN: the scripted right click, « Modifier », arrow drag and
//! « Inspecter » on the object our demo avatar wears (demo_worn.rs).
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
        let (edit, inspect) = (matches!(mode.as_str(), "edit" | "move"), mode == "inspect");
        if !(edit || inspect || mode == "menu") {
            return;
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
}
