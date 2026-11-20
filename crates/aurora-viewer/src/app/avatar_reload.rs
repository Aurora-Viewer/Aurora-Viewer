//! "Reload avatar", in the right-click menu of an avatar, ours or another's:
//! every refresh the viewer has for an avatar, in one entry. The Legacy
//! Viewer has no such entry; each step is one of its own, in
//! indra/newview/llviewermenu.cpp unless named (originally LGPL 2.1):
//!
//! - its bakes downloaded again and its appearance asked anew
//!   (avatar_tex_refresh, Avatar.TexRefresh); ours is first baked again by
//!   the server (handle_rebake_textures, Advanced.RebakeTextures);
//! - ours only: every object we wear asked again on its point
//!   (LLAttachmentsMgr::refreshAttachments, indra/newview/llattachmentsmgr.cpp);
//! - the textures of every worn object downloaded again
//!   (handle_object_tex_refresh, Object.TexRefresh);
//! - every worn mesh through its highest level of detail (reset_mesh_lod),
//!   and a mesh whose download failed asked again at once
//!   (`MeshStreamer::retry_failed`);
//! - the avatar and every worn prim asked again from the simulator, as a
//!   cache miss is (RequestMultipleObjects, LLViewerRegion::requestCacheMisses
//!   in indra/newview/llviewerregion.cpp);
//! - its skeleton reset, its animations kept (LLAvatarResetSkeleton).
//!
//! Another resident's avatar is reloaded on our screen only: the protocol
//! lets no viewer have someone else's outfit baked again.
use super::App;
use crate::world::objects::ObjKey;
use aurora_net::{NetCommand, RegionHandle};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A reload downloads every bake and worn texture again: asked again this
/// soon, it would only redo what is still on its way. LLAttachmentsMgr's
/// MIN_RETRY_REQUEST_TIME, the guard of the attachments' step.
const RELOAD_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// When each avatar was last reloaded.
#[derive(Default)]
pub(super) struct Reloads(HashMap<Uuid, Instant>);

impl Reloads {
    /// Whether `id` may be reloaded at `now`; if so, the reload is counted.
    fn begin(&mut self, id: Uuid, now: Instant) -> bool {
        self.0.retain(|_, at| now.duration_since(*at) < RELOAD_AGAIN_AFTER);
        if self.0.contains_key(&id) {
            return false;
        }
        self.0.insert(id, now);
        true
    }
}

impl App {
    pub(super) fn reload_avatar(&mut self, id: Uuid) {
        if !self.in_world() || id.is_nil() {
            return;
        }
        let Some(avatar) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let me = id == self.world.agent_id;
        let name = self.world.social.display_name_of(&id);
        if !self.avatar_reloads.begin(id, Instant::now()) {
            self.world.system_message(aurora_i18n::tr!("avatar-reload-wait", name = name));
            return;
        }
        if me {
            // the server bakes our outfit again, then our bakes are
            // downloaded again (refresh_avatar_textures)
            self.rebake();
            self.refresh_attachments();
        } else {
            self.refresh_avatar_textures(id);
        }
        let roots = super::inspect::worn_roots(&self.world, &id);
        for root in &roots {
            self.refresh_object_textures(*root);
        }
        let prims: Vec<usize> = roots
            .iter()
            .filter_map(|root| self.world.objects.index_of_uuid(root))
            .flat_map(|root| crate::build::family(&self.world, root))
            .collect();
        let mut meshes = 0;
        for &prim in &prims {
            let mesh = self
                .world
                .objects
                .get(prim)
                .map(|o| o.visual_volume())
                .filter(|v| v.is_mesh())
                .and_then(|v| v.sculpt.map(|s| s.texture));
            if let Some(mesh) = mesh
                && self.scene.meshes.retry_failed(&mesh)
            {
                meshes += 1;
            }
        }
        self.scene.reset_mesh_lods(&mut self.world, &roots, Instant::now());
        let keys: Vec<ObjKey> = std::iter::once(avatar)
            .chain(prims.iter().copied())
            .filter_map(|i| self.world.objects.get(i).map(|o| o.key))
            .collect();
        for (handle, local_ids) in object_requests(&keys) {
            self.send(NetCommand::RequestObjects { handle, local_ids });
        }
        self.reset_skeleton(id, false);
        log::info!(
            "reload avatar {id}: {} worn objects, {} prims asked again, {meshes} failed meshes retried",
            roots.len(),
            keys.len()
        );
        let text = if me {
            aurora_i18n::tr!("avatar-reload-self").to_string()
        } else {
            aurora_i18n::tr!("avatar-reload-other", name = name).to_string()
        };
        self.world.system_message(text);
    }

    /// AURORA_DEMO_RELOAD_AVATAR=other|self: the right-click menu of the
    /// demo's other avatar (or ours) at frame 700, "Reload avatar" chosen at
    /// 740, then again at 760, too soon (docs/TESTING.md).
    pub(super) fn demo_reload_avatar_steps(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_RELOAD_AVATAR") else {
            return;
        };
        let own = mode.trim() == "self";
        let target = if own { self.world.agent_id } else { crate::demo::DEMO_LOUP };
        match self.frame_count {
            700 => {
                if let Some(idx) = self.world.objects.index_of_uuid(&target)
                    && let Some((pos, _, _)) = crate::scene::Scene::object_transform(&self.world, idx, Instant::now(), 0)
                    && let Some((x, y)) = self.build.cam.project_px(pos)
                {
                    let ppp = self.egui_ctx.pixels_per_point().max(0.1);
                    self.context_menu = Some(crate::ui::context::ContextMenu {
                        pos: egui::pos2(x / ppp, y / ppp),
                        point: pos,
                        target: crate::ui::context::Target::Avatar {
                            id: target,
                            own,
                            attachment: None,
                        },
                        pie_path: Vec::new(),
                        pie_input_frame: None,
                    });
                }
            }
            740 | 760 => {
                self.context_menu = None;
                self.build.release_menu_selection(&self.world);
                self.on_ctx_action(crate::ui::context::CtxAction::ReloadAvatar(target));
            }
            _ => {}
        }
    }
}

/// RequestMultipleObjects of these objects, one per region, each local id
/// once, in the order given (the avatar first).
fn object_requests(keys: &[ObjKey]) -> Vec<(RegionHandle, Vec<u32>)> {
    let mut out: Vec<(RegionHandle, Vec<u32>)> = Vec::new();
    for key in keys {
        let at = match out.iter().position(|(handle, _)| *handle == key.region) {
            Some(at) => at,
            None => {
                out.push((key.region, Vec::new()));
                out.len() - 1
            }
        };
        if !out[at].1.contains(&key.local_id) {
            out[at].1.push(key.local_id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(region: RegionHandle, local_id: u32) -> ObjKey {
        ObjKey { region, local_id }
    }

    #[test]
    fn requests_go_one_per_region_with_each_object_once() {
        let keys = [key(1, 10), key(1, 11), key(2, 20), key(1, 10), key(2, 21)];
        assert_eq!(object_requests(&keys), vec![(1, vec![10, 11]), (2, vec![20, 21])]);
        assert!(object_requests(&[]).is_empty());
    }

    #[test]
    fn an_avatar_is_reloaded_again_only_after_a_few_seconds() {
        let mut reloads = Reloads::default();
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let t = Instant::now();
        assert!(reloads.begin(a, t));
        assert!(!reloads.begin(a, t + Duration::from_secs(1)), "too soon");
        assert!(reloads.begin(b, t + Duration::from_secs(1)), "each avatar its own");
        assert!(reloads.begin(a, t + RELOAD_AGAIN_AFTER));
    }
}
