//! "Manage" of the Region / Estate window: the settings of Firestorm's
//! Estate, Region and Terrain tabs grouped by intent, a sentence each
//! (LLPanelEstateInfo, LLPanelRegionGeneralInfo, LLPanelRegionTerrainInfo in
//! newview/llfloaterregioninfo.cpp, originally LGPL 2.1). Each section keeps
//! its changes until its bar applies them, as those tabs send everything
//! with "Apply" (`draft.rs`); the rules of what may be changed are theirs:
//! the estate owner and its managers change everything but the terrain's
//! bake, the owner's alone. The other sections of the navigation come with
//! the next steps of the redesign.

use super::confirm::Ask;
use super::draft::{Access, Draft};
use super::{ManageDemo, RegionUi, View};
use crate::theme::Palette;
use crate::ui::sheet::{self, RowText, editable, line, srow, srow_ext, why};
use crate::ui::widgets;
use crate::world::World;
use crate::world::estate::Role;
use aurora_net::estate::{ESTATE_LAST_LINDEN, RegionRules, TerrainSettings, estate_flags as ef};
use egui::{RichText, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// The sections of this step, in the navigation's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Section {
    #[default]
    Access,
    Rules,
    Terrain,
}

impl Section {
    pub(super) const ALL: [Section; 3] = [Section::Access, Section::Rules, Section::Terrain];

    fn icon(self) -> &'static str {
        match self {
            Section::Access => "door-enter",
            Section::Rules => "shield-filled",
            Section::Terrain => "mountain-filled",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Section::Access => aurora_i18n::tr!("ui-land-manage-access"),
            Section::Rules => aurora_i18n::tr!("region-manage-rules"),
            Section::Terrain => aurora_i18n::tr!("region-manage-terrain"),
        }
    }
}

/// The mockup's other sections, greyed in the navigation until their step.
fn later() -> [(&'static str, &'static str); 5] {
    [
        ("code", aurora_i18n::tr!("region-manage-scripts")),
        ("sun-high-filled", aurora_i18n::tr!("common-environment")),
        ("flask-filled", aurora_i18n::tr!("ui-land-manage-experiences")),
        ("gavel", aurora_i18n::tr!("region-manage-covenant")),
        ("bolt-filled", aurora_i18n::tr!("region-manage-interventions")),
    ]
}

/// What the page keeps between frames.
#[derive(Default)]
pub(super) struct ManageState {
    pub nav: sheet::SectionScroll<Section>,
    pub access: Draft<Access>,
    pub rules: Draft<RegionRules>,
    pub terrain: Draft<TerrainSettings>,
    /// The terrain texture the texture picker is choosing.
    pub picking: Option<usize>,
    /// AURORA_DEMO_REGION's section, changes and question, set up once the
    /// simulator answered.
    pub demo: Option<ManageDemo>,
}

/// AURORA_DEMO_REGION on Manage: scroll to the section, make the mockup's
/// changes there and ask the question, once the demo simulator answered.
pub(super) fn demo(s: &mut RegionUi, v: &View, world: &World) {
    let Some(d) = s.manage.demo else {
        return;
    };
    let (Some(rules), Some(terrain)) = (server_rules(world), server_terrain(world, v)) else {
        return;
    };
    let access = server_access(world);
    if v.manages() && access.is_none() {
        return;
    }
    s.manage.demo = None;
    let section = match d.section {
        Some(1) => Section::Rules,
        Some(2) => Section::Terrain,
        _ => Section::Access,
    };
    if section != Section::Access || d.pending {
        s.manage.nav.go(section);
        // the changes' bar ends the section
        s.manage.nav.jump_to_end = d.pending;
    }
    let heights = d.ask == Some(Ask::TextureHeights);
    let override_off = d.ask == Some(Ask::AccessOverride);
    let bonus = d.ask == Some(Ask::ObjectBonus { dont_ask: false });
    if let Some(server) = access
        && (d.pending && section == Section::Access || override_off)
    {
        let mut a = server;
        if override_off {
            a.set(ef::ALLOW_ACCESS_OVERRIDE, false);
        } else {
            a.set(ef::DENY_ANONYMOUS, true);
            a.set(ef::ALLOW_VOICE, false);
        }
        s.manage.access.set(&server, a);
    }
    if d.pending && section == Section::Rules || bonus {
        // the mockup's "2 changes not sent yet"
        let mut r = rules;
        r.agent_limit = 50.0;
        r.object_bonus = 1.5;
        s.manage.rules.set(&rules, r);
    }
    if d.pending && section == Section::Terrain || heights {
        let mut t = terrain;
        if heights {
            t.start_height[1] = t.height_range[1] + 10.0;
        } else {
            t.water_height += 0.5;
            t.raise_limit = 6.0;
        }
        s.manage.terrain.set(&terrain, t);
    }
    s.ask = d.ask;
}

/// The estate's access flags, once `estateupdateinfo` told them.
fn server_access(world: &World) -> Option<Access> {
    world.estate.estate.as_deref().map(Access::of)
}

/// The General panel's controls as processRegionInfo fills them.
pub(super) fn server_rules(world: &World) -> Option<RegionRules> {
    world.estate.region.as_deref().map(RegionRules::of)
}

/// The Terrain panel's: water and limits of RegionInfo, textures and
/// elevations of the region's composition (RegionHandshake).
pub(super) fn server_terrain(world: &World, v: &View) -> Option<TerrainSettings> {
    let d = world.estate.region.as_deref()?;
    let info = world.regions.get(&v.handle)?.info.as_ref()?;
    Some(TerrainSettings {
        water_height: d.water_height,
        raise_limit: d.terrain_raise_limit,
        lower_limit: d.terrain_lower_limit,
        textures: info.terrain_detail,
        start_height: info.terrain_start_height,
        height_range: info.terrain_height_range,
    })
}

/// The page under the header.
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    images: &HashMap<Uuid, egui::TextureHandle>,
    compact: bool,
) {
    if !v.manages() {
        let text = if v.role == Role::Resident {
            aurora_i18n::tr!("region-manage-lock-resident")
        } else {
            aurora_i18n::tr!("region-manage-lock-preview")
        };
        sheet::lock_bar(ui, p, text);
    }
    let xp = world
        .estate
        .experiences
        .clone()
        .flatten()
        .map(|x| x.trusted.len() + x.allowed.len() + x.blocked.len())
        .filter(|n| *n > 0)
        .map(|n| n.to_string());
    let nav = |ui: &mut egui::Ui, sec: Section, lit: bool| {
        if compact {
            sheet::nav_compact(ui, p, sec.icon(), sec.label(), lit)
        } else {
            widgets::navigation_wrapped(ui, p, sec.icon(), sec.label(), None, lit)
        }
    };
    let tail = |ui: &mut egui::Ui| {
        let later_tip = aurora_i18n::tr!("common-not-available-in-aurora-yet");
        for (i, (icon, label)) in later().into_iter().enumerate() {
            // the experiences' count, as the mockup shows it
            let count = (i == 2).then_some(xp.as_deref()).flatten();
            editable(ui, false, |ui| {
                if compact {
                    sheet::nav_compact(ui, p, icon, label, false)
                } else {
                    widgets::navigation_wrapped(ui, p, icon, label, count, false)
                }
            })
            .on_disabled_hover_text(later_tip);
        }
    };
    let mut state = std::mem::take(&mut s.manage.nav);
    sheet::sections_with(
        ui,
        p,
        "region_estate_manage",
        &Section::ALL,
        &mut state,
        compact,
        nav,
        tail,
        |ui, sec| match sec {
            Section::Access => access(ui, p, s, v, world),
            Section::Rules => rules(ui, p, s, v, world),
            Section::Terrain => terrain(ui, p, s, v, world, images),
        },
    );
    s.manage.nav = state;
}

fn title(ui: &mut egui::Ui, p: &Palette, sec: Section, right: impl FnOnce(&mut egui::Ui)) {
    sheet::section_title(ui, p, sec.icon(), sec.label(), right);
}

/// A switch at the right end of a row.
fn switch(ui: &mut egui::Ui, p: &Palette, on: &mut bool, can: bool) -> bool {
    editable(ui, can, |ui| widgets::switch(ui, p, on).changed())
}

/// A number field of the mockup: right aligned, `decimals` after the
/// point, a unit.
#[allow(clippy::too_many_arguments)]
fn number(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    speed: f64,
    decimals: usize,
    unit: &'static str,
    width: f32,
    can: bool,
) -> bool {
    editable(ui, can, |ui| {
        let mut v = f64::from(*value);
        let r = ui.add_sized(
            [width, 24.0],
            egui::DragValue::new(&mut v)
                .range(f64::from(*range.start())..=f64::from(*range.end()))
                .speed(speed)
                .custom_parser(aurora_i18n::parse_number)
                .custom_formatter(move |v, _| format!("{}{unit}", aurora_i18n::number(v, decimals))),
        );
        if r.changed() {
            *value = v as f32;
        }
        r.changed()
    })
}

/// "Who can come": the estate's access (LLPanelEstateInfo). The simulator
/// sends it to the estate's owner and managers only: a resident gets a
/// sentence instead.
fn access(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let server = server_access(world);
    let can = v.manages() && server.is_some();
    let mut now = server.map(|a| s.manage.access.shown(&a)).unwrap_or_default();
    let before = now;
    let options = [
        (aurora_i18n::tr!("common-everyone"), true),
        (aurora_i18n::tr!("region-lists-only"), true),
    ];
    title(ui, p, Section::Access, |ui| {
        if v.manages() {
            editable(ui, can, |ui| {
                let mut choice = usize::from(!now.has(ef::EXTERNALLY_VISIBLE));
                if widgets::segmented(ui, p, &mut choice, &options) {
                    now.set(ef::EXTERNALLY_VISIBLE, choice == 0);
                }
            });
        }
    });
    why(ui, p, aurora_i18n::tr!("region-access-why"));
    if !v.manages() {
        empty(ui, p, "eye-off", aurora_i18n::tr!("region-access-not-sent"));
        return;
    }
    let public = now.has(ef::EXTERNALLY_VISIBLE);
    let rows: [(u64, &str, &str, bool); 6] = [
        (ef::DENY_AGEUNVERIFIED, aurora_i18n::tr!("region-access-age"), "", public),
        (ef::DENY_ANONYMOUS, aurora_i18n::tr!("region-access-payment"), "", public),
        (
            ef::DENY_BOTS,
            aurora_i18n::tr!("region-access-bots"),
            aurora_i18n::tr!("region-access-bots-desc"),
            public,
        ),
        (
            ef::ALLOW_ACCESS_OVERRIDE,
            aurora_i18n::tr!("region-access-override"),
            aurora_i18n::tr!("region-access-override-desc"),
            true,
        ),
        (
            ef::ALLOW_DIRECT_TELEPORT,
            aurora_i18n::tr!("region-access-direct-teleport"),
            aurora_i18n::tr!("region-access-direct-teleport-desc"),
            true,
        ),
        (ef::ALLOW_VOICE, aurora_i18n::tr!("region-access-voice"), "", true),
    ];
    for (i, (flag, name, desc, enabled)) in rows.into_iter().enumerate() {
        if i > 0 {
            line(ui, p);
        }
        srow(ui, p, name, desc, 40.0, |ui| {
            let mut on = now.has(flag);
            if switch(ui, p, &mut on, can && enabled) {
                now.set(flag, on);
                // onChangeAccessOverride: what turning it off undoes
                if flag == ef::ALLOW_ACCESS_OVERRIDE && !on {
                    s.ask = Some(Ask::AccessOverride);
                }
            }
        });
    }
    let Some(server) = server else {
        return;
    };
    if now != before {
        s.manage.access.set(&server, now);
    }
    let pending = s.manage.access.pending(&server);
    if pending > 0 {
        match sheet::pending_bar(ui, p, pending) {
            Some(true) => apply_access(s, v, world),
            Some(false) => s.manage.access.cancel(),
            None => {}
        }
    }
}

/// The Estate panel's "Apply": a Linden estate is confirmed first
/// (LLPanelEstateInfo::sendUpdate, ChangeLindenEstate).
fn apply_access(s: &mut RegionUi, v: &View, world: &mut World) {
    if world.estate.estate.as_ref().is_some_and(|e| e.id <= ESTATE_LAST_LINDEN) {
        s.ask = Some(Ask::LindenEstate);
    } else {
        send_access(s, v, world);
    }
}

pub(super) fn send_access(s: &mut RegionUi, v: &View, world: &mut World) {
    let (Some(server), Some(estate)) = (server_access(world), world.estate.estate.clone()) else {
        return;
    };
    if let Some(a) = s.manage.access.apply(&server) {
        world.estate.set_estate(v.handle, a.applied_to(&estate));
    }
}

/// A sentence in the middle of a section, under a large faint icon.
fn empty(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(14.0);
        widgets::glyph(ui, icon, p.muted_dim, 30.0);
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.muted)).wrap());
        ui.add_space(14.0);
    });
}

/// "Region rules": the General panel's switches, limits and maturity.
fn rules(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    title(ui, p, Section::Rules, |_| {});
    why(ui, p, aurora_i18n::tr!("region-rules-why"));
    let (Some(server), Some(details)) = (server_rules(world), world.estate.region.clone()) else {
        loading(ui, p);
        return;
    };
    let can = v.manages();
    let mut now = s.manage.rules.shown(&server);
    let before = now;
    let switches: [(&str, &str, &mut bool); 8] = [
        (aurora_i18n::tr!("region-rules-no-fly"), "", &mut now.block_fly),
        (
            aurora_i18n::tr!("region-rules-no-fly-over"),
            aurora_i18n::tr!("region-rules-no-fly-over-desc"),
            &mut now.block_fly_over,
        ),
        (
            aurora_i18n::tr!("region-rules-no-terraform"),
            aurora_i18n::tr!("region-rules-no-terraform-desc"),
            &mut now.block_terraform,
        ),
        (aurora_i18n::tr!("region-rules-damage"), "", &mut now.allow_damage),
        (
            aurora_i18n::tr!("region-rules-push"),
            aurora_i18n::tr!("region-rules-push-desc"),
            &mut now.restrict_pushobject,
        ),
        (aurora_i18n::tr!("region-rules-resell"), "", &mut now.allow_land_resell),
        (aurora_i18n::tr!("region-rules-join"), "", &mut now.allow_parcel_changes),
        (aurora_i18n::tr!("region-rules-hide-search"), "", &mut now.block_parcel_search),
    ];
    for (i, (name, desc, value)) in switches.into_iter().enumerate() {
        if i > 0 {
            line(ui, p);
        }
        srow(ui, p, name, desc, 40.0, |ui| {
            switch(ui, p, value, can);
        });
    }
    line(ui, p);
    let hard = details.hard_max_agents.max(1) as f32;
    let agents_desc = aurora_i18n::tr!(
        "region-rules-agents-desc",
        max = aurora_i18n::integer(i64::from(details.hard_max_agents))
    );
    srow(ui, p, aurora_i18n::tr!("region-rules-agents"), &agents_desc, 80.0, |ui| {
        number(ui, &mut now.agent_limit, 1.0..=hard, 0.2, 0, "", 72.0, can);
    });
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-rules-bonus"),
        aurora_i18n::tr!("region-rules-bonus-desc"),
        80.0,
        |ui| {
            number(ui, &mut now.object_bonus, 1.0..=10.0, 0.05, 2, "", 72.0, can);
        },
    );
    line(ui, p);
    // processRegionInfo: the teen grid's estate keeps its rating
    let teen = details.parent_estate_id == 5;
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-rules-maturity"),
        aurora_i18n::tr!("region-rules-maturity-desc"),
        300.0,
        |ui| {
            const ACCESS: [u8; 3] = [13, 21, 42];
            editable(ui, can && !teen, |ui| {
                let options = [
                    (aurora_i18n::tr!("common-general"), true),
                    (aurora_i18n::tr!("common-moderate"), true),
                    (aurora_i18n::tr!("common-adult"), true),
                ];
                let mut choice = ACCESS.iter().position(|a| *a == now.sim_access).unwrap_or(0);
                if widgets::segmented(ui, p, &mut choice, &options) {
                    now.sim_access = ACCESS[choice];
                }
            });
            widgets::maturity_badge_of(ui, p, now.sim_access, 15.0);
        },
    );
    later_fold(ui, p, aurora_i18n::tr!("region-rules-advanced"));
    if now != before {
        s.manage.rules.set(&server, now);
    }
    let pending = s.manage.rules.pending(&server);
    if can && pending > 0 {
        match sheet::pending_bar(ui, p, pending) {
            Some(true) => apply_rules(s, v, world, &server),
            Some(false) => s.manage.rules.cancel(),
            None => {}
        }
    }
}

/// The General panel's "Apply": a new object bonus is confirmed first
/// (onBtnSet, ChangeObjectBonusFactor, unless its "ignore" box was ticked).
fn apply_rules(s: &mut RegionUi, v: &View, world: &mut World, server: &RegionRules) {
    let bonus_changed = s.manage.rules.editing().is_some_and(|r| r.object_bonus != server.object_bonus);
    if bonus_changed && s.confirm_object_bonus {
        s.ask = Some(Ask::ObjectBonus { dont_ask: false });
    } else {
        send_rules(s, v, world);
    }
}

/// sendUpdate, then RegionMaturityChange when the rating changed.
pub(super) fn send_rules(s: &mut RegionUi, v: &View, world: &mut World) {
    let Some(server) = server_rules(world) else {
        return;
    };
    if let Some(rules) = s.manage.rules.apply(&server) {
        world.estate.set_region_rules(v.handle, rules);
        if rules.sim_access != server.sim_access {
            s.ask = Some(Ask::MaturityChanged);
        }
    }
}

/// A row that says the simulator has not answered yet.
fn loading(ui: &mut egui::Ui, p: &Palette) {
    ui.label(RichText::new(aurora_i18n::tr!("common-loading")).size(12.0).color(p.muted));
}

/// An "Advanced" fold whose content comes with a later step: greyed.
fn later_fold(ui: &mut egui::Ui, p: &Palette, label: &str) {
    let mut open = false;
    editable(ui, false, |ui| {
        ui.scope(|ui| sheet::fold_row(ui, p, &mut open, label))
            .response
            .on_disabled_hover_text(aurora_i18n::tr!("common-not-available-in-aurora-yet"));
    });
}

/// Corners in the mockup's order (north-west, north-east, south-west,
/// south-east) and their index in the composition (00 south-west, 01
/// north-west, 10 south-east, 11 north-east).
fn corners() -> [(usize, &'static str); 4] {
    [
        (1, aurora_i18n::tr!("region-terrain-heights-nw")),
        (3, aurora_i18n::tr!("region-terrain-heights-ne")),
        (0, aurora_i18n::tr!("region-terrain-heights-sw")),
        (2, aurora_i18n::tr!("region-terrain-heights-se")),
    ]
}

/// "Terrain and water": the Terrain panel without its RAW file and PBR
/// materials.
fn terrain(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, images: &HashMap<Uuid, egui::TextureHandle>) {
    title(ui, p, Section::Terrain, |_| {});
    why(ui, p, aurora_i18n::tr!("region-terrain-why"));
    let Some(server) = server_terrain(world, v) else {
        loading(ui, p);
        return;
    };
    let can = v.manages();
    let owner = v.shown == Role::Owner;
    let mut now = s.manage.terrain.shown(&server);
    let before = now;
    srow(ui, p, aurora_i18n::tr!("region-terrain-water"), "", 90.0, |ui| {
        number(ui, &mut now.water_height, 0.0..=100.0, 0.1, 1, " m", 76.0, can);
    });
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-terrain-limits"),
        aurora_i18n::tr!("region-terrain-limits-desc"),
        170.0,
        |ui| {
            number(ui, &mut now.lower_limit, -100.0..=0.0, 0.2, 1, " m", 76.0, can);
            number(ui, &mut now.raise_limit, 0.0..=100.0, 0.2, 1, " m", 76.0, can);
        },
    );
    line(ui, p);
    // the bake is the estate owner's (refreshFromRegion: owner_or_god)
    let lock = (can && !owner).then_some(aurora_i18n::tr!("region-owner-only"));
    let text = RowText {
        name: aurora_i18n::tr!("region-terrain-bake"),
        lock,
        desc: aurora_i18n::tr!("region-terrain-bake-desc"),
        link: None,
    };
    srow_ext(ui, p, &text, 90.0, |ui| {
        let size = Vec2::new(0.0, 28.0);
        let label = aurora_i18n::tr!("region-terrain-bake-button");
        if widgets::action_button_if(ui, p, "", label, widgets::ActionKind::Flat, size, can && owner).clicked() {
            s.ask = Some(Ask::Bake);
        }
    });
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-terrain-cover"),
        aurora_i18n::tr!("region-terrain-cover-desc"),
        220.0,
        |ui| {
            // PBR materials come with the advanced step: the choice stays on
            // the textures (Firestorm hides it unless RenderTerrainPBREnabled)
            editable(ui, can, |ui| {
                let options = [
                    (aurora_i18n::tr!("common-textures"), true),
                    (aurora_i18n::tr!("region-terrain-materials"), true),
                ];
                let mut choice = 0;
                ui.scope(|ui| widgets::segmented(ui, p, &mut choice, &options))
                    .response
                    .on_hover_text(aurora_i18n::tr!("region-terrain-materials-later"));
            });
        },
    );
    swatches(ui, p, s, &now, images, can);
    for (n, (corner, name)) in corners().into_iter().enumerate() {
        line(ui, p);
        let desc = if n == 0 {
            aurora_i18n::tr!("region-terrain-heights-desc")
        } else {
            ""
        };
        srow(ui, p, name, desc, 170.0, |ui| {
            // right to left: high, then low
            number(ui, &mut now.height_range[corner], -500.0..=500.0, 0.5, 1, "", 76.0, can);
            number(ui, &mut now.start_height[corner], -500.0..=500.0, 0.5, 1, "", 76.0, can);
        });
    }
    later_fold(ui, p, aurora_i18n::tr!("region-terrain-advanced"));
    if now != before {
        s.manage.terrain.set(&server, now);
    }
    let pending = s.manage.terrain.pending(&server);
    if can && pending > 0 {
        match sheet::pending_bar(ui, p, pending) {
            Some(true) => apply_terrain(s, v, world),
            Some(false) => s.manage.terrain.cancel(),
            None => {}
        }
    }
}

/// The four detail textures, lowest first; a click opens the texture
/// picker on one.
fn swatches(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    now: &TerrainSettings,
    images: &HashMap<Uuid, egui::TextureHandle>,
    can: bool,
) {
    const SIZE: Vec2 = Vec2::new(92.0, 56.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, id) in now.textures.iter().enumerate() {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let sense = if can { Sense::click() } else { Sense::hover() };
                let (rect, r) = ui.allocate_exact_size(SIZE, sense);
                let radius = ui.visuals().window_corner_radius;
                let tex = (!id.is_nil()).then(|| images.get(id)).flatten();
                match tex {
                    Some(t) => {
                        let [w, h] = t.size();
                        egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                            .uv(sheet::cover_uv(w, h, rect.size()))
                            .tint(widgets::PICTURE_TINT)
                            .corner_radius(radius)
                            .paint_at(ui, rect);
                    }
                    None => {
                        if !id.is_nil() {
                            s.wanted_images.insert(*id);
                        }
                        ui.painter().rect_filled(rect, radius, p.bar);
                        sheet::icon_at(ui, "photo-filled", rect.center(), 20.0, p.muted_dim);
                    }
                }
                if can && r.hovered() {
                    ui.painter()
                        .rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
                }
                let label = match i {
                    0 => aurora_i18n::tr!("region-terrain-texture-low").to_owned(),
                    3 => aurora_i18n::tr!("region-terrain-texture-high").to_owned(),
                    _ => (i + 1).to_string(),
                };
                ui.label(RichText::new(label).size(11.0).color(p.muted));
                if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    s.manage.picking = Some(i);
                    let title = aurora_i18n::tr!("region-terrain-texture-title", n = (i + 1) as i64);
                    s.texture.open(&title, *id);
                }
            });
        }
    });
    ui.add_space(4.0);
}

/// The Terrain panel's "Apply": elevation ranges whose low end is above
/// their high one are confirmed first (validateTextureHeights, unless
/// RegionCheckTextureHeights was turned off by "Don't ask").
fn apply_terrain(s: &mut RegionUi, v: &View, world: &mut World) {
    let inverted = s
        .manage
        .terrain
        .editing()
        .is_some_and(|t| (0..4).any(|i| t.start_height[i] > t.height_range[i]));
    if inverted && s.check_texture_heights {
        s.ask = Some(Ask::TextureHeights);
    } else {
        send_terrain(s, v, world);
    }
}

pub(super) fn send_terrain(s: &mut RegionUi, v: &View, world: &mut World) {
    let Some(server) = server_terrain(world, v) else {
        return;
    };
    if let Some(t) = s.manage.terrain.apply(&server) {
        world.estate.set_terrain(v.handle, t);
    }
}

/// The texture the picker chose for the swatch it was opened on.
pub(super) fn picked_texture(s: &mut RegionUi, v: &View, world: &World, asset: Uuid) {
    let Some(i) = s.manage.picking.take() else {
        return;
    };
    let Some(server) = server_terrain(world, v) else {
        return;
    };
    let mut now = s.manage.terrain.shown(&server);
    now.textures[i] = asset;
    s.manage.terrain.set(&server, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mockup's corners, in the composition's numbering.
    #[test]
    fn the_corners_follow_the_composition() {
        let order: Vec<usize> = corners().iter().map(|(i, _)| *i).collect();
        assert_eq!(order, [1, 3, 0, 2]);
    }
}
