//! Local terrain editing: swatches, live picker choices and reversible previews.
//! Unlike LLPanelRegionTerrainInfo's Apply-only cover, this preview is local
//! until Apply (requested Aurora behaviour); closing/cancelling sends nothing.

use super::{RegionUi, View, advanced, manage};
use crate::scene::Scene;
use crate::world::{World, terrain::SurfaceOverride};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct SurfaceUi {
    pub ids: [Uuid; 4],
    pub models: [Option<(Uuid, Arc<aurora_render::preview::Model>)>; 4],
    previews: crate::ui::texture_library::previews::Previews,
    pub pick_before: Option<Uuid>,
    pub demo_mode: u8,
    pub demo_paint_scroll: bool,
    demo_tick: u8,
}

impl SurfaceUi {
    pub fn prepare(&mut self, ctx: &egui::Context, scene: &mut Scene, wanted: &mut HashSet<Uuid>) {
        self.previews.poll();
        for (slot, asset) in self.ids.into_iter().enumerate() {
            self.models[slot] = if asset.is_nil() {
                None
            } else {
                self.previews.model(asset, scene, wanted, ctx).map(|model| (asset, model))
            };
        }
    }
}

pub(super) fn clear_preview(s: &RegionUi, world: &mut World) {
    if let Some(region) = s.manage_region.and_then(|handle| world.regions.get_mut(&handle)) {
        region.terrain_surface.preview = None;
    }
}

pub(super) fn value(s: &mut RegionUi, v: &View, world: &World) -> Option<SurfaceOverride> {
    let server = manage::server_terrain(world, v)?;
    let ids = s.manage.terrain.shown(&server).textures;
    let transforms = advanced::server_transforms(world)
        .map(|server| s.advanced.transforms.shown(&server).0)
        .unwrap_or_else(|| {
            world
                .regions
                .get(&v.handle)
                .map_or(Default::default(), |r| r.terrain_surface.transforms)
        });
    Some(SurfaceOverride {
        ids,
        materials: s.advanced.kind.or(s.advanced.shown_kind) == Some(advanced::TerrainKind::Materials),
        transforms,
    })
}

pub(super) fn update_preview(s: &mut RegionUi, v: &View, world: &mut World) {
    let preview = v.manages().then(|| value(s, v, world)).flatten().filter(|surface| {
        let Some(region) = world.regions.get(&v.handle) else { return false };
        let committed = region.terrain_surface.applied;
        let ids = committed.map(|v| v.ids).or_else(|| region.info.as_ref().map(|v| v.terrain_detail));
        ids != Some(surface.ids) || s.advanced.kind.is_some() || s.advanced.transforms.editing().is_some()
    });
    if let Some(region) = world.regions.get_mut(&v.handle) {
        region.terrain_surface.preview = preview;
    }
}

pub(super) fn pending(s: &RegionUi, v: &View, world: &World, ids: [Uuid; 4]) -> bool {
    world.regions.get(&v.handle).is_some_and(|region| {
        let committed = region
            .terrain_surface
            .applied
            .map(|v| v.ids)
            .or_else(|| region.info.as_ref().map(|v| v.terrain_detail));
        committed != Some(ids) || s.advanced.transforms.editing().is_some()
    })
}

pub(super) fn cancel_cover(s: &mut RegionUi, v: &View, world: &mut World) {
    let Some(server) = manage::server_terrain(world, v) else { return };
    let applied = world.regions.get(&v.handle).and_then(|r| r.terrain_surface.applied);
    let mut now = s.manage.terrain.shown(&server);
    now.textures = applied.map_or(server.textures, |v| v.ids);
    s.manage.terrain.set(&server, now);
    s.advanced.kind = applied.map(|v| {
        if v.materials {
            advanced::TerrainKind::Materials
        } else {
            advanced::TerrainKind::Textures
        }
    });
    s.advanced.transforms.cancel();
    s.texture.open = false;
    s.manage.picking = None;
    s.surface.pick_before = None;
    clear_preview(s, world);
}

pub(super) fn picker_choice(s: &mut RegionUi, v: &View, world: &World, asset: Uuid, accept: bool) {
    if !v.manages() {
        return;
    }
    let Some(slot) = s.manage.picking else { return };
    let Some(server) = manage::server_terrain(world, v) else { return };
    let mut now = s.manage.terrain.shown(&server);
    now.textures[slot] = asset;
    s.manage.terrain.set(&server, now);
    if accept {
        s.manage.picking = None;
        s.surface.pick_before = None;
    }
}

pub(super) fn cancel_picker(s: &mut RegionUi, v: &View, world: &World) {
    if let Some(before) = s.surface.pick_before.take() {
        picker_choice(s, v, world, before, true);
    }
    s.manage.picking = None;
}

/// Exercise the production local preview and commit paths without a grid.
pub(super) fn demo_step(s: &mut RegionUi, v: &View, world: &mut World) {
    if s.surface.demo_mode == 0 {
        return;
    }
    s.surface.demo_tick = s.surface.demo_tick.saturating_add(1);
    if s.surface.demo_mode >= 4 {
        if let Some(region) = world.regions.get_mut(&v.handle) {
            if s.surface.demo_tick >= 5 && matches!(s.surface.demo_mode, 4 | 5) && region.heightmap.received_fraction() >= 1.0 {
                region.terrain_surface.request_paint();
                s.surface.demo_paint_scroll = true;
                s.surface.demo_mode = if s.surface.demo_mode == 4 { 0 } else { 7 };
                s.surface.demo_tick = 0;
            }
            if s.surface.demo_tick == 30 && s.surface.demo_mode == 7 {
                region.terrain_surface.clear_paint();
                s.surface.demo_paint_scroll = true;
                s.surface.demo_mode = 0;
            }
        }
        return;
    }
    let title = aurora_i18n::tr!("region-terrain-material-title", n = 1_i64);
    match s.surface.demo_tick {
        5 => {
            let Some(server) = manage::server_terrain(world, v) else { return };
            let before = s.manage.terrain.shown(&server).textures[0];
            s.manage.picking = Some(0);
            s.surface.pick_before = Some(before);
            s.texture.open_terrain_materials(&title, before);
        }
        10 => {
            let asset = crate::demo::region::terrain_replacement();
            picker_choice(s, v, world, asset, false);
            s.texture.open_terrain_materials(&title, asset);
        }
        30 => {
            match s.surface.demo_mode {
                2 => {
                    cancel_picker(s, v, world);
                    s.texture.open = false;
                }
                3 => {
                    picker_choice(s, v, world, crate::demo::region::terrain_replacement(), true);
                    s.texture.open = false;
                    manage::apply_terrain(s, v, world);
                }
                _ => {}
            }
            s.surface.demo_mode = 0;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{NetEvent, estate::TerrainSettings};

    fn fixture() -> (World, RegionUi, View, TerrainSettings) {
        let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(avatars);
        for event in crate::demo::events() {
            if matches!(
                event,
                NetEvent::LoggedIn(_) | NetEvent::MainRegionChanged { .. } | NetEvent::RegionHandshake(_)
            ) {
                let _ = world.apply(event);
            }
        }
        let handle = world.main_region.unwrap();
        let agent = world.agent_id;
        let info = Arc::make_mut(world.regions.get_mut(&handle).unwrap().info.as_mut().unwrap());
        info.owner = agent;
        info.terrain_detail = [Uuid::from_u128(100); 4];
        world.estate.region = Some(Arc::new(crate::demo::region::region_details()));
        let s = RegionUi {
            manage_region: Some(handle),
            ..Default::default()
        };
        let view = s.view(&world, handle);
        let server = manage::server_terrain(&world, &view).unwrap();
        (world, s, view, server)
    }

    #[test]
    fn terrain_live_picker_preview_cancel_and_apply_keep_simulator_state_separate() {
        for materials in [false, true] {
            let (mut world, mut s, v, server) = fixture();
            s.advanced.shown_kind = Some(if materials {
                advanced::TerrainKind::Materials
            } else {
                advanced::TerrainKind::Textures
            });
            let asset = Uuid::from_u128(200);
            s.manage.picking = Some(2);
            s.surface.pick_before = Some(server.textures[2]);
            picker_choice(&mut s, &v, &world, asset, false);
            update_preview(&mut s, &v, &mut world);
            let region = &world.regions[&v.handle];
            assert_eq!(region.terrain_surface.preview.unwrap().ids[2], asset);
            assert_eq!(region.terrain_surface.preview.unwrap().materials, materials);
            assert_eq!(region.info.as_ref().unwrap().terrain_detail, server.textures);
            assert!(world.estate.take_commands().is_empty(), "live preview never sends a simulator edit");
            cancel_picker(&mut s, &v, &world);
            update_preview(&mut s, &v, &mut world);
            assert!(world.regions[&v.handle].terrain_surface.effective().is_none());
            assert_eq!(s.manage.terrain.shown(&server).textures, server.textures);

            s.manage.picking = Some(1);
            s.surface.pick_before = Some(server.textures[1]);
            picker_choice(&mut s, &v, &world, asset, true);
            update_preview(&mut s, &v, &mut world);
            assert_eq!(world.regions[&v.handle].terrain_surface.preview.unwrap().ids[1], asset);
            let mut draft = s.manage.terrain.shown(&server);
            draft.water_height += 1.0;
            s.manage.terrain.set(&server, draft);
            cancel_cover(&mut s, &v, &mut world);
            assert_eq!(s.manage.terrain.shown(&server).textures, server.textures);
            assert_eq!(
                s.manage.terrain.shown(&server).water_height,
                server.water_height + 1.0,
                "cover cancel preserves other terrain edits"
            );
            assert!(world.regions[&v.handle].terrain_surface.preview.is_none());
            assert!(world.estate.take_commands().is_empty());

            s.manage.picking = Some(3);
            picker_choice(&mut s, &v, &world, asset, true);
            update_preview(&mut s, &v, &mut world);
            manage::send_terrain(&mut s, &v, &mut world);
            assert!(world.regions[&v.handle].terrain_surface.preview.is_none());
            assert_eq!(world.regions[&v.handle].terrain_surface.applied.unwrap().ids[3], asset);
            assert!(!world.estate.take_commands().is_empty());
            clear_preview(&s, &mut world);
            assert_eq!(
                world.regions[&v.handle].terrain_surface.effective().unwrap().ids[3],
                asset,
                "closing after Apply preserves the committed appearance"
            );
            let mut handshake = (**world.regions[&v.handle].info.as_ref().unwrap()).clone();
            handshake.terrain_detail[3] = asset;
            let _ = world.apply(NetEvent::RegionHandshake(Arc::new(handshake)));
            assert!(world.regions[&v.handle].terrain_surface.applied.is_none());
            assert_eq!(world.regions[&v.handle].info.as_ref().unwrap().terrain_detail[3], asset);
        }
    }

    #[test]
    fn terrain_preview_closes_and_cannot_be_used_after_management_permission_is_lost() {
        let (mut world, mut s, mut v, server) = fixture();
        s.advanced.shown_kind = Some(advanced::TerrainKind::Textures);
        s.manage.picking = Some(0);
        picker_choice(&mut s, &v, &world, Uuid::from_u128(500), false);
        update_preview(&mut s, &v, &mut world);
        assert!(world.regions[&v.handle].terrain_surface.preview.is_some());
        clear_preview(&s, &mut world);
        assert!(world.regions[&v.handle].terrain_surface.preview.is_none());
        v.shown = crate::world::estate::Role::Resident;
        picker_choice(&mut s, &v, &world, Uuid::from_u128(600), false);
        assert_ne!(s.manage.terrain.shown(&server).textures[0], Uuid::from_u128(600));
        update_preview(&mut s, &v, &mut world);
        assert!(world.regions[&v.handle].terrain_surface.preview.is_none());
        assert!(world.estate.take_commands().is_empty());
    }
}
