//! "Refresh textures" and "Force appearance update (rebake)", after
//! Firestorm's Object.TexRefresh / Avatar.TexRefresh and
//! handle_rebake_textures (indra/newview/llviewermenu.cpp, originally LGPL
//! 2.1): the textures are forgotten and downloaded again
//! (`TextureStreamer::refresh`), and for a rebake the server is asked to
//! bake our outfit again (`World::rebake`).

use super::App;
use crate::scene::avatar;
use aurora_net::NetCommand;
use uuid::Uuid;

impl App {
    /// Object.TexRefresh (handle_object_tex_refresh): every texture of the
    /// linkset, those of its faces, of their materials (normal, specular,
    /// glTF maps) and its sculpt map, as the selection a right click makes.
    pub(super) fn refresh_object_textures(&mut self, id: Uuid) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let root = super::context_menu::linkset_root(&self.world, idx);
        let mut ids: Vec<Uuid> = Vec::new();
        for prim in crate::build::family(&self.world, root) {
            for t in self.scene.gpu.get(prim).map_or(&[][..], |g| &g.tex_ids[..]) {
                if !ids.contains(t) {
                    ids.push(*t);
                }
            }
        }
        let asked = ids.iter().filter(|t| self.scene.textures.refresh(t)).count();
        log::info!("refresh textures of object {id}: {asked} textures");
    }

    /// Avatar.TexRefresh (avatar_tex_refresh): its bakes are downloaded
    /// again, and the simulator is asked for its appearance anew
    /// (sendAvatarTexturesRequest).
    pub(super) fn refresh_avatar_textures(&mut self, id: Uuid) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let bakes = avatar::refreshable_bakes(|te| self.world.bake_texture(idx, te));
        let asked = bakes.iter().filter(|t| self.scene.textures.refresh(t)).count();
        log::info!("refresh textures of avatar {id}: {asked} bakes");
        self.send(NetCommand::AvatarTexturesRequest(id));
    }

    /// Advanced.RebakeTextures (handle_rebake_textures): the server bakes
    /// our outfit again (`World::rebake`) and our bakes are downloaded again
    /// (FIRE-11800). Its local parts have no counterpart here: Aurora only
    /// knows server bakes (forceBakeAllTextures), and the mesh LOD reset
    /// and the region-crossing flag are not ported.
    pub(super) fn rebake(&mut self) {
        if !self.in_world() {
            return;
        }
        log::info!("rebake: asking the server for a new appearance");
        self.world.rebake();
        for cmd in std::mem::take(&mut self.world.outfit.commands) {
            self.send(cmd);
        }
        let me = self.world.agent_id;
        self.refresh_avatar_textures(me);
    }
}
