//! Read-only, bounded rez-provenance lookups for the opt-in audio filter.
use super::*;
use crate::world::rez_sound_blocks::Scope;
use aurora_net::bridge::Endpoint;

impl App {
    pub(super) fn rez_sound_scope(&self, endpoint: Option<&Endpoint>) -> Scope {
        Scope {
            agent: self.world.agent_id,
            region: self.world.main_region,
            session: endpoint.map(|e| e.token),
        }
    }

    pub(super) fn update_rez_sound_blocks(&mut self) {
        let endpoint = self.sync_bridge_queries();
        let scope = self.rez_sound_scope(endpoint.as_ref());
        let now = Instant::now();
        self.world
            .rez_sound_blocks
            .configure(self.settings.audio.mute_rez_descendants, scope, &self.world.mutes);
        if self.world.rez_sound_blocks.scan_due(now) {
            let sources = self.scene.sounds.object_sources(&self.world, self.camera.position);
            let mut live = Vec::new();
            for (id, region) in sources.into_iter().filter(|(_, region)| Some(*region) == scope.region) {
                let mut object = self.world.objects.index_of_uuid(&id).and_then(|i| self.world.objects.get(i));
                let owner = object.map(|o| o.owner_id);
                let mut root = object.map(|o| o.full_id);
                for _ in 0..64 {
                    let Some(o) = object else {
                        break;
                    };
                    if o.is_avatar() {
                        break;
                    }
                    root = Some(o.full_id);
                    object = self.world.objects.parent_of(o).and_then(|i| self.world.objects.get(i));
                }
                self.world.rez_sound_blocks.watch(id, region, root.zip(owner), now);
                live.push(id);
            }
            self.world.rez_sound_blocks.set_live(&live, now);
        }
        if let Some(endpoint) = endpoint
            && let Some((request, query)) = self.world.rez_sound_blocks.request(now)
        {
            self.send(NetCommand::BridgeQuery { request, endpoint, query });
        }
    }
}
