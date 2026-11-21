//! "Region settings" of Aurora-Sim based grids: the limits their
//! simulators let the estate set (draw distance, prim sizes, link counts,
//! name tags, minimap…), shown only when the region offers the
//! DispatchOpenRegionSettings capability. Port of
//! LLPanelRegionOpenSettingsInfo (indra/newview/llfloaterregioninfo.cpp,
//! panel_region_open_region_settings.xml, originally LGPL 2.1; the
//! OpenRegionInfo side is Copyright (C) 2011 Patrick Sapinski, Matthew
//! Beardmore). Its values are OpenRegionInfo's (`aurora_net::open_region`);
//! "Apply" sends the whole tab, as onClickOrs does.

use super::manage::{loading, number, switch};
use super::{RegionUi, View};
use crate::theme::Palette;
use crate::ui::sheet::{self, editable, line, srow, why};
use crate::ui::widgets;
use crate::world::World;
use aurora_net::open_region::{CAP, OpenRegionSettings};

pub(super) const ICON: &str = "dimensions";

pub(super) fn label() -> &'static str {
    aurora_i18n::tr!("region-manage-open-settings")
}

/// LLFloaterRegionInfo::postBuild: the tab exists only when the region
/// has the capability.
pub(super) fn available(world: &World, v: &View) -> bool {
    world.regions.get(&v.handle).is_some_and(|r| r.caps.contains_key(CAP))
}

/// What the simulator said, the group limit filled in from the account
/// when the region names none (refreshFromRegion shows gMaxAgentGroups).
pub(super) fn server(world: &World, v: &View) -> Option<OpenRegionSettings> {
    let mut s = world.regions.get(&v.handle)?.open_settings?;
    if s.max_groups.is_none() {
        let limit = crate::ui::contacts::group_limit(world);
        s.max_groups = Some(if limit > 0 { limit as i32 } else { -1 });
    }
    Some(s)
}

/// refreshFromRegion: editable by a god or whoever may manage the estate.
fn can_edit(world: &World, v: &View) -> bool {
    v.manages() || (v.shown == v.role && world.god_level > 0)
}

/// The tab, under its title.
pub(super) fn section(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    sheet::section_title(ui, p, ICON, label(), |_| {});
    why(ui, p, aurora_i18n::tr!("region-open-why"));
    let Some(server) = server(world, v) else {
        loading(ui, p);
        return;
    };
    let can = can_edit(world, v);
    let mut now = s.manage.open_settings.shown(&server);
    let before = now;
    rows(ui, p, &mut now, can);
    if now != before {
        s.manage.open_settings.set(&server, now);
    }
    let pending = s.manage.open_settings.pending(&server);
    if can && pending > 0 {
        match sheet::pending_bar(ui, p, pending) {
            Some(true) => {
                if let Some(settings) = s.manage.open_settings.apply(&server) {
                    world.estate.set_open_region_settings(v.handle, settings);
                }
            }
            Some(false) => s.manage.open_settings.cancel(),
            None => {}
        }
    }
}

/// A whole number in one of the spinners.
fn whole(ui: &mut egui::Ui, value: &mut i32, range: std::ops::RangeInclusive<i32>, can: bool) {
    let mut f = *value as f32;
    if number(ui, &mut f, *range.start() as f32..=*range.end() as f32, 1.0, 0, "", 80.0, can) {
        *value = f.round() as i32;
    }
}

/// The panel's controls, in its order, with its ranges.
fn rows(ui: &mut egui::Ui, p: &Palette, now: &mut OpenRegionSettings, can: bool) {
    use aurora_i18n::tr;
    const W: f32 = 90.0;
    srow(
        ui,
        p,
        tr!("region-open-draw-distance"),
        tr!("region-open-draw-distance-desc"),
        W,
        |ui| {
            number(ui, &mut now.draw_distance, -1.0..=10_000.0, 1.0, 0, " m", 80.0, can);
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        tr!("region-open-force-draw-distance"),
        tr!("region-open-force-draw-distance-desc"),
        40.0,
        |ui| {
            switch(ui, p, &mut now.force_draw_distance, can);
        },
    );
    let metres: [(&str, &str, &mut f32, usize); 4] = [
        (
            tr!("region-open-max-drag"),
            tr!("region-open-max-drag-desc"),
            &mut now.max_drag_distance,
            0,
        ),
        (
            tr!("region-open-max-prim"),
            tr!("region-open-max-prim-desc"),
            &mut now.max_prim_scale,
            3,
        ),
        (
            tr!("region-open-min-prim"),
            tr!("region-open-min-prim-desc"),
            &mut now.min_prim_scale,
            3,
        ),
        (
            tr!("region-open-max-phys-prim"),
            tr!("region-open-max-phys-prim-desc"),
            &mut now.max_phys_prim_scale,
            3,
        ),
    ];
    for (name, desc, value, decimals) in metres {
        line(ui, p);
        srow(ui, p, name, desc, W, |ui| {
            number(ui, value, 0.0..=10_000.0, 0.1, decimals, " m", 80.0, can);
        });
    }
    let percents: [(&str, &str, &mut f32); 2] = [
        (
            tr!("region-open-max-hollow"),
            tr!("region-open-max-hollow-desc"),
            &mut now.max_hollow_size,
        ),
        (
            tr!("region-open-min-hole"),
            tr!("region-open-min-hole-desc"),
            &mut now.min_hole_size,
        ),
    ];
    for (name, desc, value) in percents {
        line(ui, p);
        srow(ui, p, name, desc, W, |ui| {
            number(ui, value, 0.0..=100.0, 0.1, 2, "", 80.0, can);
        });
    }
    let counts: [(&str, &str, &mut i32, std::ops::RangeInclusive<i32>); 4] = [
        (
            tr!("region-open-max-link"),
            tr!("region-open-max-link-desc"),
            &mut now.max_link_count,
            0..=100_000,
        ),
        (
            tr!("region-open-max-link-phys"),
            tr!("region-open-max-link-phys-desc"),
            &mut now.max_link_count_phys,
            0..=100_000,
        ),
        (
            tr!("region-open-max-transfer"),
            tr!("region-open-max-transfer-desc"),
            &mut now.max_inventory_items_transfer,
            -1..=10_000,
        ),
        (
            tr!("region-open-max-groups"),
            tr!("region-open-max-groups-desc"),
            now.max_groups.get_or_insert(-1),
            -1..=1000,
        ),
    ];
    for (name, desc, value, range) in counts {
        line(ui, p);
        srow(ui, p, name, desc, W, |ui| whole(ui, value, range, can));
    }
    line(ui, p);
    srow(
        ui,
        p,
        tr!("region-open-terrain-scale"),
        tr!("region-open-terrain-scale-desc"),
        W,
        |ui| {
            number(ui, &mut now.terrain_detail_scale, -1.0..=10_000.0, 0.1, 2, "", 80.0, can);
        },
    );
    line(ui, p);
    // the spinner's 0 to 2, named
    srow(ui, p, tr!("region-open-show-tags"), "", 260.0, |ui| {
        editable(ui, can, |ui| {
            let options = [
                (tr!("region-open-tags-hide"), true),
                (tr!("region-open-tags-brief"), true),
                (tr!("region-open-tags-show"), true),
            ];
            let mut choice = now.show_tags.clamp(0, 2) as usize;
            if widgets::segmented(ui, p, &mut choice, &options) {
                now.show_tags = choice as i32;
            }
        });
    });
    let switches: [(&str, &mut bool); 5] = [
        (tr!("region-open-render-water"), &mut now.render_water),
        (tr!("region-open-allow-minimap"), &mut now.allow_minimap),
        (tr!("region-open-allow-physical"), &mut now.allow_physical_prims),
        (tr!("region-open-enforce-build"), &mut now.enforce_max_build),
        (tr!("region-open-parcel-windlight"), &mut now.allow_parcel_windlight),
    ];
    for (name, value) in switches {
        line(ui, p);
        srow(ui, p, name, "", 40.0, |ui| {
            switch(ui, p, value, can);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::estate::Role;
    use aurora_net::NetEvent;
    use std::sync::Arc;

    fn world() -> (World, aurora_net::RegionHandle) {
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
        let handle = world.main_region.expect("demo region");
        (world, handle)
    }

    fn view(world: &World, handle: aurora_net::RegionHandle, role: Role, shown: Role) -> View {
        let s = RegionUi {
            manage_region: Some(handle),
            ..Default::default()
        };
        let mut v = s.view(world, handle);
        v.role = role;
        v.shown = shown;
        v
    }

    #[test]
    fn the_tab_needs_the_capability_and_shows_the_account_group_limit() {
        let (mut world, handle) = world();
        let v = view(&world, handle, Role::Owner, Role::Owner);
        assert!(!available(&world, &v), "no capability, no tab (Second Life, OpenSim)");
        let region = world.regions.get_mut(&handle).unwrap();
        Arc::make_mut(&mut region.caps).insert(CAP.to_owned(), "demo://open".to_owned());
        assert!(available(&world, &v));
        assert!(server(&world, &v).is_none(), "loading until OpenRegionInfo");
        let _ = world.apply(NetEvent::OpenRegionSettings {
            handle,
            settings: Box::new(OpenRegionSettings {
                max_link_count: 255,
                ..OpenRegionSettings::default()
            }),
        });
        let s = server(&world, &v).expect("the message arrived");
        assert_eq!(s.max_link_count, 255);
        let limit = crate::ui::contacts::group_limit(&world) as i32;
        assert_eq!(s.max_groups, Some(if limit > 0 { limit } else { -1 }));
    }

    /// refreshFromRegion: a god or whoever may manage the estate edits it;
    /// "View as" Resident shows it read-only.
    #[test]
    fn who_may_change_the_settings() {
        let (mut world, handle) = world();
        assert!(can_edit(&world, &view(&world, handle, Role::Owner, Role::Owner)));
        assert!(can_edit(&world, &view(&world, handle, Role::Manager, Role::Manager)));
        assert!(!can_edit(&world, &view(&world, handle, Role::Owner, Role::Resident)));
        assert!(!can_edit(&world, &view(&world, handle, Role::Resident, Role::Resident)));
        world.god_level = 200;
        assert!(can_edit(&world, &view(&world, handle, Role::Resident, Role::Resident)));
    }

    #[test]
    fn apply_sends_the_whole_tab() {
        use super::super::draft::Draft;
        let (mut world, handle) = world();
        let server = OpenRegionSettings::default();
        let mut draft: Draft<OpenRegionSettings> = Draft::default();
        let mut now = draft.shown(&server);
        now.draw_distance = 512.0;
        now.allow_minimap = false;
        draft.set(&server, now);
        assert_eq!(draft.pending(&server), 2);
        let sent = draft.apply(&server).expect("changes");
        world.estate.set_open_region_settings(handle, sent);
        let cmds = world.estate.take_commands();
        assert!(cmds.iter().any(|c| matches!(
            c,
            aurora_net::NetCommand::Estate(aurora_net::estate::EstateCommand::SetOpenRegionSettings { handle: h, settings })
                if *h == handle && settings.draw_distance == 512.0 && !settings.allow_minimap
        )));
        assert_eq!(draft.shown(&server), sent, "kept shown until the region answers");
    }
}
