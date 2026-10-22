//! "Overview" of the Aurora About Land (`aurora.rs`): what anyone wants to
//! know of a parcel before its settings. The actions of Firestorm's floater
//! that are not settings (teleport to the landing point, FIRE-10043; buy a
//! pass, LLPanelLandGeneral::onClickBuyPass; buy land, script info), the
//! covenant of LLPanelLandCovenant as a card that opens in full, and the
//! facts of the General tab (estate, region, search, traffic, claim date,
//! sale, land ID) and of the media tabs, read only.
//!
//! The permission pills of the header follow the rules of the navigation
//! bar's parcel icons (`ui/parcel_icons.rs`, LLViewerParcelMgr::allowAgent*)
//! for what the region overrides, with the parcel's own three states
//! (everyone, group, nobody) of LLPanelLandOptions where it has them.

use super::aurora::{Tone, card_title, cards, kv, pill};
use super::dialogs::Confirm;
use super::{Dialog, LandUi, View};
use crate::theme::Palette;
use crate::ui::{menu, widgets};
use crate::world::World;
use crate::world::land::{can_agent_buy, powers};
use aurora_net::land::OS_LEASE_PENDING;
use aurora_net::{ParcelInfo, parcel_flags as pf, region_flags as rf};
use egui::{RichText, Vec2};
use glam::Vec3;
use uuid::Uuid;

/// One pill of the header: a permission of visitors and how far it goes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Permit {
    pub icon: &'static str,
    pub tone: Tone,
    pub label: &'static str,
    pub tip: &'static str,
}

/// Everyone (`all`), the parcel's group (`group`) or nobody: the two
/// columns of the Options tab.
fn reach(all: bool, group: bool) -> Tone {
    if all {
        Tone::Allow
    } else if group {
        Tone::Group
    } else {
        Tone::Deny
    }
}

/// The pills, in the mockup's order: build, scripts, fly, push, damage,
/// voice, object entry.
pub(super) fn permits(parcel: &ParcelInfo, region_flags: u32) -> Vec<Permit> {
    let f = |b: u32| parcel.flags & b != 0;
    let region = |b: u32| region_flags & b != 0;
    let mut out = Vec::with_capacity(7);

    let build = reach(f(pf::CREATE_OBJECTS), f(pf::CREATE_GROUP_OBJECTS));
    out.push(Permit {
        icon: "box",
        tone: build,
        label: match build {
            Tone::Allow => aurora_i18n::tr!("ui-land-overview-build"),
            Tone::Group => aurora_i18n::tr!("ui-land-overview-build-group"),
            _ => aurora_i18n::tr!("ui-land-overview-no-build"),
        },
        tip: match build {
            Tone::Allow => aurora_i18n::tr!("ui-parcel-icons-building-allowed-for-everyone-you-can-create-and-rez"),
            Tone::Group => aurora_i18n::tr!("ui-parcel-icons-building-restricted-to-the-parcel-group-you-cannot-create"),
            _ => aurora_i18n::tr!("ui-parcel-icons-building-prohibited-you-cannot-create-or-rez-objects-here"),
        },
    });

    // the region can stop every script whatever the parcel says
    let stopped = region(rf::SKIP_SCRIPTS) || region(rf::ESTATE_SKIP_SCRIPTS);
    let scripts = if stopped {
        Tone::Deny
    } else {
        reach(f(pf::ALLOW_OTHER_SCRIPTS), f(pf::ALLOW_GROUP_SCRIPTS))
    };
    out.push(Permit {
        icon: "code",
        tone: scripts,
        label: match scripts {
            Tone::Allow => aurora_i18n::tr!("ui-land-overview-scripts"),
            Tone::Group => aurora_i18n::tr!("ui-land-overview-scripts-group"),
            _ => aurora_i18n::tr!("ui-land-overview-no-scripts"),
        },
        tip: if region(rf::ESTATE_SKIP_SCRIPTS) {
            aurora_i18n::tr!("ui-parcel-icons-an-administrator-has-temporarily-stopped-scripts-in-this-region")
        } else if region(rf::SKIP_SCRIPTS) {
            aurora_i18n::tr!("ui-parcel-icons-no-scripts-are-running-in-this-region")
        } else if scripts == Tone::Allow {
            aurora_i18n::tr!("ui-parcel-icons-scripts-allowed")
        } else if scripts == Tone::Group {
            aurora_i18n::tr!("ui-parcel-icons-visitor-scripts-disabled-only-the-land-owner-s-and")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-visitor-scripts-disabled-only-the-land-owner-s-scripts")
        },
    });

    let fly = !region(rf::BLOCK_FLY) && f(pf::ALLOW_FLY);
    out.push(Permit {
        icon: "plane-tilt-filled",
        tone: if fly { Tone::Allow } else { Tone::Deny },
        label: if fly {
            aurora_i18n::tr!("ui-land-overview-fly")
        } else {
            aurora_i18n::tr!("ui-land-overview-no-fly")
        },
        tip: if fly {
            aurora_i18n::tr!("ui-parcel-icons-flight-allowed")
        } else if region(rf::BLOCK_FLY) {
            aurora_i18n::tr!("ui-parcel-icons-flight-disabled-in-this-region-you-cannot-fly-here")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-flight-disabled-on-this-parcel-you-cannot-fly-here")
        },
    });

    let push = !region(rf::RESTRICT_PUSHOBJECT) && !f(pf::RESTRICT_PUSHOBJECT) && !parcel.region_push_override;
    out.push(Permit {
        icon: "hand-stop",
        tone: if push { Tone::Allow } else { Tone::Deny },
        label: if push {
            aurora_i18n::tr!("ui-land-overview-push")
        } else {
            aurora_i18n::tr!("ui-land-overview-no-push")
        },
        tip: if push {
            aurora_i18n::tr!("ui-parcel-icons-pushing-allowed-other-people-s-scripts-can-push-you")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-pushing-restricted-only-the-landowner-can-push-others")
        },
    });

    let damage = region(rf::ALLOW_DAMAGE) || f(pf::ALLOW_DAMAGE);
    out.push(Permit {
        icon: "heart-filled",
        tone: if damage { Tone::Deny } else { Tone::Allow },
        label: if damage {
            aurora_i18n::tr!("ui-land-overview-damage")
        } else {
            aurora_i18n::tr!("ui-land-overview-safe")
        },
        tip: if damage {
            aurora_i18n::tr!("ui-land-overview-damage-tip")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-safe-zone-no-damage")
        },
    });

    let voice = region(rf::ALLOW_VOICE) && f(pf::ALLOW_VOICE_CHAT);
    out.push(Permit {
        icon: "microphone-filled",
        tone: if voice { Tone::Allow } else { Tone::Deny },
        label: if voice {
            aurora_i18n::tr!("ui-land-overview-voice")
        } else {
            aurora_i18n::tr!("ui-land-overview-no-voice")
        },
        tip: if voice {
            aurora_i18n::tr!("ui-parcel-icons-voice-allowed")
        } else if !region(rf::ALLOW_VOICE) {
            aurora_i18n::tr!("ui-parcel-icons-voice-disabled-in-this-region-you-will-not-hear")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-voice-disabled-on-this-parcel-you-will-not-hear")
        },
    });

    let entry = reach(f(pf::ALLOW_ALL_OBJECT_ENTRY), f(pf::ALLOW_GROUP_OBJECT_ENTRY));
    out.push(Permit {
        icon: "package-import",
        tone: entry,
        label: match entry {
            Tone::Allow => aurora_i18n::tr!("ui-land-overview-object-entry"),
            Tone::Group => aurora_i18n::tr!("ui-land-overview-object-entry-group"),
            _ => aurora_i18n::tr!("ui-land-overview-no-object-entry"),
        },
        tip: match entry {
            Tone::Allow => aurora_i18n::tr!("ui-land-options-when-checked-residents-can-move-their-objects-from-other"),
            Tone::Group => aurora_i18n::tr!("ui-land-options-when-checked-parcel-group-members-can-move-their-objects"),
            _ => aurora_i18n::tr!("ui-land-overview-no-object-entry-tip"),
        },
    });
    out
}

/// Who may come, as the Access tab's checkboxes say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Access {
    Everyone,
    GroupAndList,
    ListOnly,
}

/// LLPanelLandAccess::refresh: public unless the estate lets the parcel
/// restrict it and the parcel uses its access list.
pub(super) fn access(parcel: &ParcelInfo) -> Access {
    let override_ok = parcel.region_allow_access_override;
    if !override_ok || parcel.flags & pf::USE_ACCESS_LIST == 0 {
        Access::Everyone
    } else if parcel.flags & pf::USE_ACCESS_GROUP != 0 {
        Access::GroupAndList
    } else {
        Access::ListOnly
    }
}

fn access_pill(a: Access) -> (Tone, &'static str) {
    match a {
        Access::Everyone => (Tone::Allow, aurora_i18n::tr!("ui-land-overview-open-to-all")),
        Access::GroupAndList => (Tone::Group, aurora_i18n::tr!("ui-land-overview-group-and-list")),
        Access::ListOnly => (Tone::Deny, aurora_i18n::tr!("ui-land-overview-list-only")),
    }
}

/// Teleport routing (LLParcel::ELandingType) and the landing point.
pub(super) fn arrival(parcel: &ParcelInfo) -> String {
    let lp = parcel.user_location;
    match parcel.landing_type {
        0 => aurora_i18n::tr!("ui-land-overview-arrival-blocked").to_owned(),
        1 if lp != Vec3::ZERO => aurora_i18n::tr!("ui-land-overview-arrival-fixed-at", position = position(lp)),
        1 => aurora_i18n::tr!("ui-land-overview-arrival-fixed").to_owned(),
        _ => aurora_i18n::tr!("ui-land-overview-arrival-anywhere").to_owned(),
    }
}

/// "x, y, z" rounded, as Firestorm writes a position in a region.
fn position(v: Vec3) -> String {
    format!("{}, {}, {}", v.x.round() as i64, v.y.round() as i64, v.z.round() as i64)
}

/// The place the parcel stands for: its landing point, else the middle of
/// its bounding box on the ground (`ground` gives the terrain height).
pub(super) fn place_point(parcel: &ParcelInfo, ground: impl Fn(f32, f32) -> f32) -> Vec3 {
    if parcel.user_location != Vec3::ZERO {
        return parcel.user_location;
    }
    let c = (parcel.aabb_min + parcel.aabb_max) * 0.5;
    Vec3::new(c.x, c.y, ground(c.x, c.y))
}

/// "1214b5b2-…-b18243908089": the land ID's first and last groups.
pub(super) fn short_id(id: &Uuid) -> String {
    let s = id.hyphenated().to_string();
    match (s.split('-').next(), s.rsplit('-').next()) {
        (Some(a), Some(b)) => format!("{a}-…-{b}"),
        _ => s,
    }
}

/// A stream's address without its scheme nor the closing slash.
pub(super) fn stream_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.strip_suffix('/').unwrap_or(rest)
}

/// "Mar 12, 2025" in SLT, the date of a time stamp without its hour.
pub(crate) fn day(secs: i64) -> String {
    let (y, m, d, ..) = super::slt_parts(secs);
    aurora_i18n::tr!(
        "ui-land-overview-date",
        day = d.to_string(),
        month = super::months()[m - 1].to_owned(),
        year = y.to_string()
    )
}

/// The parcel's position link, its type and area: the line under the name.
pub(super) fn region_line(v: &View, parcel: &ParcelInfo, point: Vec3) -> (String, String) {
    let region = v.region.as_ref().map_or("", |r| r.name.as_str());
    let link = format!("{region} ({})", position(point));
    let product = super::product_name(v.region.as_ref().map_or("", |r| r.product.as_str()));
    let area = format!("{} m²", aurora_i18n::integer(i64::from(parcel.area)));
    let rest = if product.is_empty() { area } else { format!("{product} · {area}") };
    (link, rest)
}

/// Why no pass can be bought on a parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoPass {
    Own,
    NotSold,
    Banned,
}

/// LLPanelLandGeneral::refresh and LLMenuParcelObserver::changed: a pass
/// for someone else's land that sells them, unless the last ban line the
/// agent ran into was a ban (`banned`, isCollisionBanned).
pub(super) fn pass_refusal(parcel: &ParcelInfo, agent: Uuid, banned: bool) -> Option<NoPass> {
    if parcel.owner_id == agent {
        Some(NoPass::Own)
    } else if parcel.flags & pf::USE_PASS_LIST == 0 {
        Some(NoPass::NotSold)
    } else if banned {
        Some(NoPass::Banned)
    } else {
        None
    }
}

pub(super) fn can_buy_pass(parcel: &ParcelInfo, agent: Uuid, banned: bool) -> bool {
    pass_refusal(parcel, agent, banned).is_none()
}

/// LLViewerParcelMgr::isCollisionBanned: only BA_BANNED, not "not in the
/// group" nor "not on the list" (which a pass is for).
pub(super) fn collision_banned(world: &World) -> bool {
    world.map.collision.as_ref().is_some_and(|c| c.kind == 1)
}

/// The point of the region the floater talks about, on the ground.
fn point_of(v: &View, parcel: &ParcelInfo, world: &World) -> Vec3 {
    let ground = |x: f32, y: f32| world.regions.get(&v.handle).map_or(0.0, |r| r.heightmap.sample(x, y));
    place_point(parcel, ground)
}

/// The actions under the header, then the cards.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    let Some(parcel) = v.parcel.clone() else {
        return;
    };
    ui.add_space(10.0);
    actions(ui, p, s, v, world, &parcel);
    ui.add_space(12.0);
    let left = |ui: &mut egui::Ui, world: &mut World, s: &mut LandUi| description_card(ui, p, s, v, world, &parcel);
    let right = |ui: &mut egui::Ui, world: &mut World, _: &mut LandUi| info_card(ui, p, v, world, &parcel);
    cards(ui, p, world, s, 1.3, left, right);
}

/// Height of an action button.
const ACTION_H: f32 = 28.0;

fn actions(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo) {
    let point = point_of(v, parcel, world);
    let region = v.region.as_ref().map_or(String::new(), |r| r.name.clone());
    let size = Vec2::new(0.0, ACTION_H);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let has_lp = parcel.user_location != Vec3::ZERO;
        let go = widgets::action_button(
            ui,
            p,
            "send-filled",
            if has_lp {
                aurora_i18n::tr!("ui-land-overview-go-to-landing-point")
            } else {
                aurora_i18n::tr!("ui-land-overview-go-to-parcel")
            },
            widgets::ActionKind::Primary,
            size,
        );
        if go
            .on_hover_text(aurora_i18n::tr!("ui-land-options-teleport-to-landing-point"))
            .clicked()
        {
            let (ox, oy) = aurora_net::handle_to_origin(v.handle);
            world
                .map
                .track_location(ox as f64 + point.x as f64, oy as f64 + point.y as f64, point.z, true);
        }
        // "Create landmark" is for where the agent stands (STORM-411)
        let here = landmark_here(v, parcel, world.parcel.as_ref().map(|a| a.local_id));
        if widgets::action_button_if(
            ui,
            p,
            "bookmark-filled",
            aurora_i18n::tr!("common-landmark"),
            widgets::ActionKind::Flat,
            size,
            here,
        )
        .on_disabled_hover_text(aurora_i18n::tr!("ui-land-overview-landmark-here"))
        .clicked()
        {
            crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::CreateLandmark);
        }
        let slurl = (!region.is_empty()).then(|| crate::slurl::make(&region, point));
        if widgets::action_button_if(
            ui,
            p,
            "copy-filled",
            aurora_i18n::tr!("ui-land-overview-slurl"),
            widgets::ActionKind::Flat,
            size,
            slurl.is_some(),
        )
        .on_hover_text(aurora_i18n::tr!("common-copy-slurl"))
        .clicked()
            && let Some(url) = slurl
        {
            ui.ctx().copy_text(url);
        }
        // onClickBuyLand: startBuyLand on the selection (LLFloaterBuyLand)
        let buy = can_agent_buy(parcel, &v.rights(), None, true);
        if widgets::action_button_if(
            ui,
            p,
            "shopping-cart-filled",
            aurora_i18n::tr!("ui-land-overview-buy"),
            widgets::ActionKind::Flat,
            size,
            buy,
        )
        .on_disabled_hover_text(aurora_i18n::tr!("ui-land-general-this-land-is-not-for-sale-to-you"))
        .clicked()
        {
            world.land.request_buy(false);
        }
        if widgets::action_button_if(
            ui,
            p,
            "ticket-filled",
            aurora_i18n::tr!("ui-land-general-buy-a-pass"),
            widgets::ActionKind::Flat,
            size,
            can_buy_pass(parcel, v.agent, collision_banned(world)),
        )
        .on_hover_text(aurora_i18n::tr!("ui-land-general-a-pass-grants-temporary-access-to-this-land"))
        .clicked()
        {
            s.dialog = Some(Dialog::Confirm(Confirm::BuyPass));
        }
        // LLFloaterReporter, about where the agent is
        if widgets::action_button(
            ui,
            p,
            "flag-filled",
            aurora_i18n::tr!("ui-land-overview-report"),
            widgets::ActionKind::Flat,
            size,
        )
        .clicked()
        {
            crate::ui::context::request(
                ui.ctx(),
                crate::ui::context::CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Nothing),
            );
        }
        let mut more = More::default();
        more_menu(ui, p, v, world, &parcel_for_group(parcel, v, world), &mut more);
        if more.scripts {
            s.scripts.open();
        }
        if more.buy_for_group {
            world.land.request_buy(true);
        }
    });
}

/// "Landmark" only for the parcel the agent stands on.
pub(super) fn landmark_here(v: &View, parcel: &ParcelInfo, agent_parcel: Option<i32>) -> bool {
    v.same_region && agent_parcel == Some(parcel.local_id)
}

/// LLPanelLandGeneral::refresh: buying for the active group, with its power.
fn parcel_for_group(parcel: &ParcelInfo, v: &View, world: &World) -> bool {
    let group = world.groups.active;
    !group.is_nil() && can_agent_buy(parcel, &v.rights(), Some(group), true)
}

/// What the "…" menu asked for.
#[derive(Default)]
struct More {
    scripts: bool,
    buy_for_group: bool,
}

/// "…": the rarer actions of Firestorm's General tab.
fn more_menu(ui: &mut egui::Ui, p: &Palette, v: &View, world: &World, buy_group: &bool, more: &mut More) {
    let Some(t) = crate::ui::icons::global("dots-filled") else {
        return;
    };
    let img = egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.ink);
    let button = egui::Button::image(img).min_size(Vec2::new(30.0, ACTION_H));
    let land_id = world.land.sel.as_ref().and_then(|s| s.parcel_uuid).flatten();
    menu::button(ui, p, button, |ui| {
        if menu::item_if(
            ui,
            p,
            "id-filled",
            aurora_i18n::tr!("ui-land-overview-copy-land-id"),
            land_id.is_some(),
        ) && let Some(id) = land_id
        {
            ui.ctx().copy_text(id.to_string());
        }
        if menu::item_if(ui, p, "users-group", aurora_i18n::tr!("ui-land-general-buy-for-group"), *buy_group) {
            more.buy_for_group = true;
        }
        if v.region.as_ref().is_some_and(|r| r.has_land_resources)
            && menu::item(ui, p, "code", aurora_i18n::tr!("ui-land-general-script-info"))
        {
            more.scripts = true;
        }
    })
    .on_hover_text(aurora_i18n::tr!("ui-land-overview-more"));
}

/// Height of the covenant excerpt before "Read all".
const COVENANT_EXCERPT: f32 = 92.0;
/// Height of its fade into the card.
const COVENANT_FADE: f32 = 30.0;

fn description_card(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo) {
    card_title(
        ui,
        p,
        "file-description-filled",
        aurora_i18n::tr!("ui-land-overview-description"),
        None,
    );
    let desc = parcel.desc.trim();
    if desc.is_empty() {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-overview-no-description"))
                .size(12.5)
                .color(p.muted_dim),
        );
    } else {
        ui.add(egui::Label::new(RichText::new(desc).size(12.5).color(p.muted)).wrap());
    }
    ui.add_space(10.0);

    let covenant = world.land.covenant.clone().filter(|c| c.handle == v.handle);
    let body = covenant
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
    let width = ui.available_width();
    let galley = ui.painter().layout(body, egui::FontId::proportional(12.5), p.muted, width);
    let long = galley.size().y > COVENANT_EXCERPT + 1.0;
    let link = long.then(|| {
        if s.covenant_open {
            aurora_i18n::tr!("ui-land-overview-read-less")
        } else {
            aurora_i18n::tr!("ui-land-overview-read-all")
        }
    });
    if card_title(ui, p, "gavel", aurora_i18n::tr!("ui-land-overview-covenant"), link).is_some_and(|r| r.clicked()) {
        s.covenant_open = !s.covenant_open;
    }
    let shown = if long && !s.covenant_open {
        COVENANT_EXCERPT
    } else {
        galley.size().y
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, shown), egui::Sense::hover());
    ui.painter_at(rect).galley(rect.min, galley, p.muted);
    if long && !s.covenant_open {
        fade(ui, p, rect);
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let yes_no = |b: bool| if b { "yes" } else { "no" }.to_owned();
        let resell = !v.region_flag(rf::BLOCK_LAND_RESELL);
        pill(
            ui,
            p,
            Tone::Neutral,
            "arrows-split",
            &aurora_i18n::tr!("ui-land-overview-resell", allowed = yes_no(resell)),
        )
        .on_hover_text(if resell {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-can-be-resold")
        } else {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-cannot-be-resold")
        });
        let changes = v.region_flag(rf::ALLOW_PARCEL_CHANGES);
        pill(
            ui,
            p,
            Tone::Neutral,
            "layout-grid-filled",
            &aurora_i18n::tr!("ui-land-overview-join-subdivide", allowed = yes_no(changes)),
        )
        .on_hover_text(if changes {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-can-be-joined-or")
        } else {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-cannot-be-joined-or")
        });
        if let Some(c) = &covenant {
            let when = if c.timestamp == 0 {
                aurora_i18n::tr!("ui-land-overview-never-modified").to_owned()
            } else {
                aurora_i18n::tr!("ui-land-overview-modified-on", date = day(c.timestamp as i64))
            };
            pill(ui, p, Tone::Neutral, "", &when);
        }
    });
}

/// The bottom of the covenant excerpt fading into the card.
fn fade(ui: &egui::Ui, p: &Palette, rect: egui::Rect) {
    crate::ui::sheet::fade(ui, p, rect, COVENANT_FADE);
}

fn info_card(ui: &mut egui::Ui, p: &Palette, v: &View, world: &mut World, parcel: &ParcelInfo) {
    card_title(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-land-overview-info"), None);
    let covenant = world.land.covenant.clone().filter(|c| c.handle == v.handle);
    let region = v.region.as_ref();
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-estate"), |ui| match &covenant {
        Some(c) => {
            value(ui, p, &c.estate_name);
            if !c.estate_owner.is_nil() {
                value(ui, p, "·");
                super::agent_link(ui, p, world, c.estate_owner);
            }
        }
        None => {
            value(ui, p, aurora_i18n::tr!("common-loading"));
        }
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-region"), |ui| {
        value(ui, p, region.map_or(aurora_i18n::tr!("common-loading"), |r| r.name.as_str()));
        if let Some(r) = region {
            widgets::maturity_badge_of(ui, p, r.sim_access, 15.0);
        }
        // the agent's region: its Region / Estate window
        if v.same_region {
            ui.add_space(6.0);
            region_link(ui, p);
        }
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-in-search"), |ui| {
        value(
            ui,
            p,
            if parcel.flags & pf::SHOW_DIRECTORY != 0 {
                aurora_i18n::tr!("common-yes")
            } else {
                aurora_i18n::tr!("common-no")
            },
        );
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-traffic"), |ui| {
        let dwell = world.land.sel.as_ref().and_then(|s| s.dwell);
        value(
            ui,
            p,
            dwell.map_or(aurora_i18n::tr!("common-loading").to_owned(), |d| aurora_i18n::number(d as f64, 0)),
        );
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-acquired"), |ui| {
        if parcel.owner_id.is_nil() {
            value(ui, p, "—");
        } else {
            value(ui, p, day(parcel.claim_date as i64));
        }
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-sale"), |ui| {
        if parcel.flags & pf::FOR_SALE != 0 {
            value(
                ui,
                p,
                aurora_i18n::tr!("ui-land-overview-for-sale-price", price = super::money(parcel.sale_price)),
            );
            if !parcel.auth_buyer.is_nil() {
                value(ui, p, "·");
                super::agent_link(ui, p, world, parcel.auth_buyer);
            }
        } else if parcel.status == OS_LEASE_PENDING {
            value(ui, p, aurora_i18n::tr!("ui-land-general-sale-pending"));
        } else {
            value(ui, p, aurora_i18n::tr!("ui-land-general-not-for-sale"));
        }
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-land-id"), |ui| {
        match world.land.sel.as_ref().and_then(|s| s.parcel_uuid) {
            Some(Some(id)) => {
                let r = ui
                    .add(egui::Label::new(RichText::new(short_id(&id)).monospace().size(11.0).color(p.muted)).sense(egui::Sense::click()))
                    .on_hover_text(format!("{id}\n{}", aurora_i18n::tr!("ui-land-overview-click-to-copy")))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if r.clicked() {
                    ui.ctx().copy_text(id.to_string());
                }
            }
            Some(None) => {
                value(ui, p, aurora_i18n::tr!("ui-land-general-could-not-resolve-parcel-id"));
            }
            None => {
                value(ui, p, aurora_i18n::tr!("common-loading"));
            }
        }
    });
    ui.add_space(10.0);
    card_title(ui, p, "music", aurora_i18n::tr!("ui-land-overview-media-and-sound"), None);
    kv(ui, p, aurora_i18n::tr!("common-music"), |ui| {
        let url = parcel.music_url.trim();
        if url.is_empty() {
            value(ui, p, aurora_i18n::tr!("ui-land-overview-none"));
        } else {
            value(ui, p, stream_host(url)).on_hover_text(url);
        }
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-overview-media"), |ui| {
        let kind = super::media::widget(&parcel.media.mime).map_or(aurora_i18n::tr!("ui-land-media-no-contents"), |w| w.1);
        let url = parcel.media_url.trim();
        let page = if url.is_empty() {
            aurora_i18n::tr!("ui-land-overview-no-page").to_owned()
        } else {
            stream_host(url).to_owned()
        };
        value(ui, p, format!("{kind} · {page}"));
    });
}

/// "Region / Estate ›": the window of the region the parcel is in.
fn region_link(ui: &mut egui::Ui, p: &Palette) {
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            widgets::glyph(ui, "building-castle", p.violet_light, 13.0);
            let text = format!("{} ›", aurora_i18n::tr!("region-title"));
            ui.add(egui::Label::new(super::aurora::semibold(ui, &text, 12.0, p.violet_light)).sense(egui::Sense::click()))
        })
        .inner;
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        crate::ui::region::request_open(ui.ctx());
    }
}

/// A value of the information card.
fn value(ui: &mut egui::Ui, p: &Palette, text: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text.into()).size(12.5).color(p.ink)).truncate())
}

/// What the header shows of the owner's capacity: "10 500 / 15 000" and
/// "4 500 free" (or how many are over).
pub(super) fn objects_fact(used: i32, max: i32) -> (String, String) {
    let main = format!(
        "{} / {}",
        aurora_i18n::integer(i64::from(used)),
        aurora_i18n::integer(i64::from(max))
    );
    let left = max - used;
    let rest = if left >= 0 {
        aurora_i18n::tr!(
            "ui-land-overview-objects-free",
            n = i64::from(left),
            count = aurora_i18n::integer(i64::from(left))
        )
    } else {
        aurora_i18n::tr!("ui-land-mod-objects-over-capacity", count = i64::from(-left))
    };
    (main, rest)
}

/// The header's access pill.
pub(super) fn access_fact(parcel: &ParcelInfo) -> (Tone, &'static str) {
    access_pill(access(parcel))
}

/// The camera of the header's picture: LLPanelLandOptions' snapshot, with
/// GP_LAND_CHANGE_IDENTITY.
pub(super) fn can_change_picture(v: &View) -> bool {
    v.can(powers::LAND_CHANGE_IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::land::powers;
    use aurora_net::GroupMembership;

    fn tones(parcel: &ParcelInfo, region: u32) -> Vec<(&'static str, Tone)> {
        permits(parcel, region).into_iter().map(|p| (p.icon, p.tone)).collect()
    }

    /// The demo parcel of the mockup: building and entry for the group,
    /// scripts and flying for all, no pushing, safe, voice.
    #[test]
    fn the_pills_say_who_may_do_what() {
        let parcel = ParcelInfo {
            flags: pf::ALLOW_FLY
                | pf::CREATE_GROUP_OBJECTS
                | pf::ALLOW_OTHER_SCRIPTS
                | pf::ALLOW_GROUP_SCRIPTS
                | pf::ALLOW_GROUP_OBJECT_ENTRY
                | pf::ALLOW_VOICE_CHAT
                | pf::RESTRICT_PUSHOBJECT,
            ..Default::default()
        };
        assert_eq!(
            tones(&parcel, rf::ALLOW_VOICE),
            vec![
                ("box", Tone::Group),
                ("code", Tone::Allow),
                ("plane-tilt-filled", Tone::Allow),
                ("hand-stop", Tone::Deny),
                ("heart-filled", Tone::Allow),
                ("microphone-filled", Tone::Allow),
                ("package-import", Tone::Group),
            ]
        );
        let labels: Vec<_> = permits(&parcel, rf::ALLOW_VOICE).into_iter().map(|p| p.label).collect();
        assert_eq!(labels[0], aurora_i18n::tr!("ui-land-overview-build-group"));
        assert_eq!(labels[3], aurora_i18n::tr!("ui-land-overview-no-push"));
    }

    #[test]
    fn the_region_overrides_the_parcel() {
        let parcel = ParcelInfo {
            flags: pf::ALLOW_FLY | pf::ALLOW_OTHER_SCRIPTS | pf::ALLOW_VOICE_CHAT | pf::CREATE_OBJECTS,
            ..Default::default()
        };
        let t = tones(
            &parcel,
            rf::BLOCK_FLY | rf::ESTATE_SKIP_SCRIPTS | rf::RESTRICT_PUSHOBJECT | rf::ALLOW_DAMAGE,
        );
        assert_eq!(t[0], ("box", Tone::Allow), "building is the parcel's alone");
        assert_eq!(t[1], ("code", Tone::Deny));
        assert_eq!(t[2], ("plane-tilt-filled", Tone::Deny));
        assert_eq!(t[3], ("hand-stop", Tone::Deny));
        assert_eq!(t[4], ("heart-filled", Tone::Deny), "damage is on");
        assert_eq!(t[5], ("microphone-filled", Tone::Deny), "no voice in the region");
        assert_eq!(t[6], ("package-import", Tone::Deny));
        let scripts = &permits(&parcel, rf::ESTATE_SKIP_SCRIPTS)[1];
        assert_eq!(
            scripts.tip,
            aurora_i18n::tr!("ui-parcel-icons-an-administrator-has-temporarily-stopped-scripts-in-this-region")
        );
        // a push restriction set by the region shows on the parcel too
        let overridden = ParcelInfo {
            region_push_override: true,
            ..Default::default()
        };
        assert_eq!(tones(&overridden, 0)[3], ("hand-stop", Tone::Deny));
    }

    #[test]
    fn access_like_the_access_tab() {
        let mut parcel = ParcelInfo {
            region_allow_access_override: true,
            ..Default::default()
        };
        assert_eq!(access(&parcel), Access::Everyone);
        parcel.flags = pf::USE_ACCESS_LIST;
        assert_eq!(access(&parcel), Access::ListOnly);
        parcel.flags |= pf::USE_ACCESS_GROUP;
        assert_eq!(access(&parcel), Access::GroupAndList);
        // the estate does not let the parcel restrict access
        parcel.region_allow_access_override = false;
        assert_eq!(access(&parcel), Access::Everyone);
        assert_eq!(access_fact(&parcel).0, Tone::Allow);
    }

    #[test]
    fn arrival_and_place() {
        let mut parcel = ParcelInfo {
            landing_type: 1,
            user_location: Vec3::new(140.2, 129.6, 25.4),
            aabb_min: Vec3::new(128.0, 128.0, 0.0),
            aabb_max: Vec3::new(256.0, 256.0, 0.0),
            ..Default::default()
        };
        assert_eq!(
            arrival(&parcel),
            aurora_i18n::tr!("ui-land-overview-arrival-fixed-at", position = "140, 130, 25".to_string())
        );
        assert_eq!(place_point(&parcel, |_, _| 0.0), parcel.user_location);
        parcel.user_location = Vec3::ZERO;
        assert_eq!(arrival(&parcel), aurora_i18n::tr!("ui-land-overview-arrival-fixed"));
        // no landing point: the middle of the parcel, on the ground
        assert_eq!(place_point(&parcel, |x, y| (x + y) / 100.0), Vec3::new(192.0, 192.0, 3.84));
        parcel.landing_type = 0;
        assert_eq!(arrival(&parcel), aurora_i18n::tr!("ui-land-overview-arrival-blocked"));
        parcel.landing_type = 2;
        assert_eq!(arrival(&parcel), aurora_i18n::tr!("ui-land-overview-arrival-anywhere"));
    }

    #[test]
    fn small_texts() {
        let id = Uuid::from_u128(0x1214_B5B2_7C42_D6C4_8741_B182_4390_8089);
        assert_eq!(short_id(&id), "1214b5b2-…-b18243908089");
        assert_eq!(stream_host("http://flux.exemple.org:8000/"), "flux.exemple.org:8000");
        assert_eq!(stream_host("flux.exemple.org"), "flux.exemple.org");
        // 2025-03-12 18:00 UTC = 11:00 PDT
        assert_eq!(
            day(1_741_802_400),
            aurora_i18n::tr!(
                "ui-land-overview-date",
                day = "12".to_string(),
                month = aurora_i18n::tr!("extra-mar").to_owned(),
                year = "2025".to_string()
            )
        );
        let (main, rest) = objects_fact(10_500, 15_000);
        assert_eq!(main, format!("{} / {}", aurora_i18n::integer(10_500), aurora_i18n::integer(15_000)));
        assert!(rest.contains(&aurora_i18n::integer(4500)), "{rest}");
        let (_, over) = objects_fact(16_200, 15_000);
        assert_eq!(over, aurora_i18n::tr!("ui-land-mod-objects-over-capacity", count = 1200_i64));
    }

    fn view(parcel: ParcelInfo, agent: Uuid, groups: Vec<GroupMembership>) -> View {
        View {
            parcel: Some(std::sync::Arc::new(parcel)),
            handle: 0,
            region: None,
            agent,
            groups,
            same_region: true,
            now: 0,
        }
    }

    /// The camera over the picture and the pass follow the rights: the
    /// owner, an officer with the identity power, not a visitor.
    #[test]
    fn rights_decide_the_picture_and_the_pass() {
        let (me, other, group) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(9));
        let member = |powers: u64| GroupMembership {
            id: group,
            name: String::new(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        };
        let leased = |owner: Uuid| ParcelInfo {
            owner_id: owner,
            group_id: group,
            is_group_owned: owner == group,
            status: aurora_net::land::OS_LEASED,
            flags: pf::USE_PASS_LIST,
            ..Default::default()
        };
        // owner
        let v = view(leased(me), me, Vec::new());
        assert!(can_change_picture(&v));
        assert_eq!(
            pass_refusal(v.parcel.as_deref().unwrap(), me, false),
            Some(NoPass::Own),
            "no pass for one's own land"
        );
        // officer of the owning group with GP_LAND_CHANGE_IDENTITY
        let v = view(leased(group), me, vec![member(powers::LAND_CHANGE_IDENTITY)]);
        assert!(can_change_picture(&v));
        // a member without the power, and a visitor
        assert!(!can_change_picture(&view(leased(group), me, vec![member(0)])));
        let v = view(leased(other), me, Vec::new());
        assert!(!can_change_picture(&v));
        assert!(can_buy_pass(v.parcel.as_deref().unwrap(), me, false));
        // isCollisionBanned refuses it; a parcel without passes says so
        assert_eq!(pass_refusal(v.parcel.as_deref().unwrap(), me, true), Some(NoPass::Banned));
        let mut closed = leased(other);
        closed.flags = 0;
        assert_eq!(pass_refusal(&closed, me, false), Some(NoPass::NotSold));
    }
}
