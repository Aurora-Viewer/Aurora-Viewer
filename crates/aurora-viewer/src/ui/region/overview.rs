//! The header and the "Overview" of the Region / Estate window: the region
//! and its estate as anyone may see them. The facts are LLFloaterRegionInfo's
//! (the General tab's name, type, estate ID and grid position, the server
//! version of gLastVersionChannel, the Covenant tab's estate, owner and
//! covenant with its resale and join / subdivide clauses), the estate's
//! access, voice and managers for who manages it, the restart schedule of
//! LLFloaterRegionRestartSchedule, the region environment of
//! LLPanelRegionEnvironment and the simulator's statistics of the Statistics
//! floater (floater_stats.xml): agents, objects, time dilation, frames,
//! script and spare time. The header's numbers follow SimStats every second.

use super::{RegionAction, RegionUi, View, action, role_name};
use crate::theme::Palette;
use crate::ui::sheet::{self, PICTURE, Tone, card_title, cards, fact, kv, pill, semibold};
use crate::ui::{menu, presence, widgets};
use crate::world::World;
use crate::world::estate::Role;
use aurora_net::estate::{RestartSchedule, SimStats, estate_flags as ef, sim_stat};
use aurora_net::region_flags as rf;
use egui::{RichText, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// Height of an action button.
pub(super) const ACTION_H: f32 = 28.0;
/// Width of the header's gauges (the mockup's 170 px; 130 compact).
const REGION_GAUGE_W: f32 = 170.0;
const COMPACT_GAUGE_W: f32 = 130.0;
/// Height of the covenant excerpt before "Read all".
const COVENANT_EXCERPT: f32 = 92.0;

/// What the window draws with.
pub(super) struct Pictures<'a> {
    pub avatars: &'a HashMap<Uuid, egui::TextureHandle>,
    /// The mini-map's image of the region.
    pub map: Option<&'a egui::TextureHandle>,
}

/// How the simulator keeps up, from its time dilation: smooth from 0.95,
/// slowed from 0.75, struggling below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Health {
    Smooth,
    Slowed,
    Struggling,
}

pub(super) fn health(dilation: f32) -> Health {
    if dilation >= 0.95 {
        Health::Smooth
    } else if dilation >= 0.75 {
        Health::Slowed
    } else {
        Health::Struggling
    }
}

/// "Sunday at 06:00 UTC", "Tuesday, Thursday at 02:00 UTC", "Every day at
/// 06:00 UTC" (the schedule floater's days and its UTC time).
pub(super) fn schedule_text(s: &RestartSchedule) -> String {
    let time = format!("{:02}:{:02}", s.seconds / 3600, (s.seconds % 3600) / 60);
    let days = if s.days.iter().all(|d| *d) {
        aurora_i18n::tr!("region-schedule-every-day").to_owned()
    } else {
        let names: Vec<String> = s
            .days
            .iter()
            .enumerate()
            .filter(|(_, on)| **on)
            .map(|(i, _)| aurora_i18n::tr!("region-weekday", day = i as i64))
            .collect();
        capitalized(&names.join(", "))
    };
    aurora_i18n::tr!("region-schedule-at", days = days, time = time)
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// The grid position of a region (getOriginGlobal / 256).
pub(super) fn grid_position(handle: aurora_net::RegionHandle) -> (u32, u32) {
    let (x, y) = aurora_net::handle_to_origin(handle);
    (x / 256, y / 256)
}

/// "12 / 40": agents in the region against its limit.
fn count_of(used: f32, max: u32) -> String {
    format!(
        "{} / {}",
        aurora_i18n::integer(used.round() as i64),
        aurora_i18n::integer(i64::from(max))
    )
}

fn fraction(used: f32, max: u32) -> f32 {
    if max == 0 { 0.0 } else { (used / max as f32).clamp(0.0, 1.0) }
}

/// The estate's name: its own answer, else the covenant's.
fn estate_name(world: &World, v: &View) -> Option<String> {
    world
        .estate
        .estate
        .as_ref()
        .map(|e| e.name.clone())
        .filter(|n| !n.is_empty())
        .or_else(|| {
            world
                .land
                .covenant
                .as_ref()
                .filter(|c| c.handle == v.handle)
                .map(|c| c.estate_name.clone())
        })
}

/// The picture of the header: the region's map with the agent's place and
/// the region's name.
fn tile(ui: &mut egui::Ui, p: &Palette, v: &View, world: &World, pics: &Pictures) {
    let (rect, _) = ui.allocate_exact_size(PICTURE, Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    let uv = match pics.map {
        Some(t) => {
            let [w, h] = t.size();
            let uv = sheet::cover_uv(w, h, rect.size());
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                .uv(uv)
                .tint(widgets::PICTURE_TINT)
                .corner_radius(radius)
                .paint_at(ui, rect);
            Some(uv)
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.bar);
            sheet::icon_at(ui, "map", rect.center(), 48.0, p.muted_dim);
            None
        }
    };
    // where the agent stands (north up)
    if let Some(uv) = uv
        && v.same_region
    {
        let size = world.regions.get(&v.handle).map_or(256.0, |r| r.heightmap.size_x.max(256) as f32);
        let pos = world.agent.position;
        let (u, t) = (pos.x / size, 1.0 - pos.y / size);
        if uv.x_range().contains(u) && uv.y_range().contains(t) {
            let at = egui::pos2(
                rect.left() + (u - uv.min.x) / uv.width() * rect.width(),
                rect.top() + (t - uv.min.y) / uv.height() * rect.height(),
            );
            ui.painter().circle_filled(at, 6.0, p.navy.gamma_multiply(0.35));
            ui.painter().circle_filled(at, 4.0, p.teal);
        }
    }
    // the region's name, shadowed to read on any map
    let galley = ui.painter().layout_job(semibold(ui, &v.name, 10.5, p.ink));
    let at = egui::pos2(rect.left() + 7.0, rect.bottom() - 6.0 - galley.size().y);
    ui.painter().galley(at + Vec2::splat(1.0), galley.clone(), p.navy);
    ui.painter().galley(at, galley, p.ink);
}

/// The region at a glance, over the pages; `slim` (a compact window's
/// settings) keeps only its name, rating, type and estate.
#[allow(clippy::too_many_arguments)]
pub(super) fn header(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    pics: &Pictures,
    compact: bool,
    slim: bool,
) {
    sheet::hero(ui, p, |ui| {
        if !compact {
            tile(ui, p, v, world, pics);
        }
        sheet::hero_column(ui, |ui| main_column(ui, p, s, v, world, pics, compact, slim));
    });
}

#[allow(clippy::too_many_arguments)]
fn main_column(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, pics: &Pictures, compact: bool, slim: bool) {
    let details = world.estate.region.clone();
    let stats = world.estate.stats_of(v.handle).cloned();
    // name, rating and type
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let size = if slim { 15.0 } else { 19.0 };
        ui.add(egui::Label::new(semibold(ui, &v.name, size, p.ink)).truncate());
        widgets::maturity_badge_of(ui, p, v.sim_access, 15.0);
        if !v.product.is_empty() {
            egui::Frame::new()
                .fill(p.teal.gamma_multiply(0.14))
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::symmetric(6, 1))
                .show(ui, |ui| {
                    ui.label(semibold(ui, &crate::ui::land::product_name(&v.product), 10.5, p.teal));
                });
        }
    });
    // the estate, the grid position and the server
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        widgets::glyph(ui, "building-castle", p.muted, 13.0);
        ui.label(RichText::new(aurora_i18n::tr!("ui-land-overview-estate")).size(12.0).color(p.muted));
        let name = estate_name(world, v).unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
        let r = ui.label(RichText::new(name).size(12.0).color(p.teal));
        if let Some(d) = &details {
            r.on_hover_text(aurora_i18n::tr!(
                "region-estate-id-tip",
                id = aurora_i18n::integer(i64::from(d.estate_id))
            ));
        }
        if !compact {
            let (x, y) = grid_position(v.handle);
            let version = world.estate.versions.get(&v.handle).cloned().unwrap_or_else(|| "—".to_owned());
            ui.label(
                RichText::new(format!(
                    "· {}",
                    aurora_i18n::tr!(
                        "region-position-server",
                        x = x.to_string(),
                        y = y.to_string(),
                        version = short_version(&version)
                    )
                ))
                .size(12.0)
                .color(p.muted),
            );
        }
    });
    if slim {
        return;
    }
    owner(ui, p, s, v, world, pics);
    // agents, objects, simulator, schedule
    let gauge_w = if compact { COMPACT_GAUGE_W } else { REGION_GAUGE_W };
    let list = fact_list(p, world, details.as_deref(), stats.as_ref());
    facts(ui, p, &list, gauge_w, if compact { 12.0 } else { 18.0 });
    // the rules of the region, after a restart under way
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        restart_pill(ui, p, v, world);
        for permit in permits(v, world.estate.estate.as_ref().map(|e| e.flags)) {
            pill(ui, p, permit.tone, permit.icon, &permit.label).on_hover_text(permit.tip);
        }
    });
}

/// A restart of this region under way: its count, and for who manages the
/// estate a click that cancels it (`restart` -1, as the alert's button).
fn restart_pill(ui: &mut egui::Ui, p: &Palette, v: &View, world: &mut World) {
    use crate::world::region_restart::Phase;
    let Some(a) = world.region_restart.alert.as_ref() else {
        return;
    };
    let ours = a.handle.map_or(a.region == v.name, |h| h == v.handle);
    if !ours || a.phase != Phase::Live {
        return;
    }
    let text = aurora_i18n::tr!("region-int-restart-pill", time = crate::ui::region_restart::time_text(a.seconds()));
    let r = pill(ui, p, Tone::Deny, "refresh", &text);
    if v.manages() {
        let r = r
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(aurora_i18n::tr!("region-int-restart-pill-tip"));
        if r.clicked() {
            world.estate.intervene(aurora_net::estate::EstateCommand::Restart {
                handle: v.handle,
                seconds: -1,
            });
        }
    }
}

/// "2026.10.02.4893" of "Second Life Server 2026.10.02.4893".
pub(super) fn short_version(v: &str) -> &str {
    v.rsplit(' ').next().unwrap_or(v)
}

/// The owner: their round picture with their status, their name, and for
/// who manages the estate its managers.
fn owner(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, pics: &Pictures) {
    const SIDE: f32 = 30.0;
    let managers: Vec<Uuid> = world
        .estate
        .access
        .clone()
        .flatten()
        .map(|a| a.managers.clone())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(SIDE), Sense::hover());
        let name = if v.owner.is_nil() {
            aurora_i18n::tr!("common-loading").to_owned()
        } else {
            sheet::names(world, &v.owner).0
        };
        let pic = pics.avatars.get(&v.owner);
        if pic.is_none() && !v.owner.is_nil() {
            s.wanted_pics.insert(v.owner);
        }
        widgets::avatar_disc(ui, p, rect, pic, &name, false);
        let status = if v.owner == world.agent_id {
            presence::Status::Online
        } else {
            presence::of(world, &v.owner)
        };
        if status != presence::Status::Unknown {
            presence::paint_dot(ui.painter(), p, rect, status, p.field);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let r = ui
                .add(egui::Label::new(semibold(ui, &name, 13.0, p.ink)).sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(aurora_i18n::tr!("common-view-profile"));
            if r.clicked() && !v.owner.is_nil() {
                crate::ui::profile::request_open(ui.ctx(), v.owner);
            }
            let mut line = role_name(Role::Owner).to_owned();
            if v.manages() && !managers.is_empty() {
                let names: Vec<String> = managers.iter().map(|id| sheet::names(world, id).0).collect();
                line.push_str(&format!(
                    " · {}",
                    aurora_i18n::tr!("region-managers", n = names.len() as i64, names = names.join(", "))
                ));
            }
            ui.label(RichText::new(line).size(12.0).color(p.muted));
        });
    });
}

/// A fact of the header: its key, its value with what follows it, and its
/// gauge (id, fraction, tint).
struct Fact {
    key: &'static str,
    value: String,
    rest: String,
    gauge: Option<(&'static str, f32, Option<egui::Color32>)>,
    /// A plain line of 12.5 px (the schedule) rather than a 14 px figure.
    small: bool,
}

/// The header's facts: agents, objects, the simulator's health, and the
/// restart schedule when the region sends it (RegionSchedule).
fn fact_list(p: &Palette, world: &World, details: Option<&aurora_net::estate::RegionDetails>, stats: Option<&SimStats>) -> Vec<Fact> {
    let loading = || aurora_i18n::tr!("common-loading").to_owned();
    let mut out = Vec::with_capacity(4);
    // agents against the region's limit (and its type's)
    out.push(match (stats.and_then(|s| s.get(sim_stat::NUMAGENTMAIN)), details) {
        (Some(n), Some(d)) => Fact {
            key: aurora_i18n::tr!("region-avatars"),
            value: count_of(n, d.max_agents),
            rest: aurora_i18n::tr!("region-avatars-most", max = aurora_i18n::integer(i64::from(d.hard_max_agents))),
            gauge: Some(("region_agents", fraction(n, d.max_agents), None)),
            small: false,
        },
        _ => Fact {
            key: aurora_i18n::tr!("region-avatars"),
            value: loading(),
            rest: String::new(),
            gauge: None,
            small: false,
        },
    });
    // objects against the region's capacity
    out.push(match stats {
        Some(s) if s.object_capacity > 0 => {
            let used = s.get(sim_stat::NUMTASKS).unwrap_or(0.0);
            let left = (i64::from(s.object_capacity) - used.round() as i64).max(0);
            let frac = fraction(used, s.object_capacity);
            let tint = (frac >= 0.9).then_some(p.danger).or((frac >= 0.8).then_some(p.warn));
            Fact {
                key: aurora_i18n::tr!("common-objects"),
                value: count_of(used, s.object_capacity),
                rest: aurora_i18n::tr!("ui-land-overview-objects-free", n = left, count = aurora_i18n::integer(left)),
                gauge: Some(("region_objects", frac, tint)),
                small: false,
            }
        }
        _ => Fact {
            key: aurora_i18n::tr!("common-objects"),
            value: loading(),
            rest: String::new(),
            gauge: None,
            small: false,
        },
    });
    // how the simulator keeps up
    out.push(match stats.and_then(|s| s.get(sim_stat::TIME_DILATION)) {
        Some(d) => {
            let (label, tint) = match health(d) {
                Health::Smooth => (aurora_i18n::tr!("region-health-smooth"), p.success),
                Health::Slowed => (aurora_i18n::tr!("region-health-slowed"), p.warn),
                Health::Struggling => (aurora_i18n::tr!("region-health-struggling"), p.danger),
            };
            Fact {
                key: aurora_i18n::tr!("region-simulator"),
                value: label.to_owned(),
                rest: aurora_i18n::tr!("region-dilation", value = aurora_i18n::number(f64::from(d), 2)),
                gauge: Some(("region_health", d.clamp(0.0, 1.0), Some(tint))),
                small: false,
            }
        }
        None => Fact {
            key: aurora_i18n::tr!("region-simulator"),
            value: loading(),
            rest: String::new(),
            gauge: None,
            small: false,
        },
    });
    // the restart schedule, shown as the region sends it
    if let Some(Ok(schedule)) = &world.estate.schedule {
        out.push(Fact {
            key: aurora_i18n::tr!("region-restart-schedule"),
            value: schedule
                .as_ref()
                .map_or_else(|| aurora_i18n::tr!("region-schedule-none").to_owned(), schedule_text),
            rest: String::new(),
            gauge: None,
            small: true,
        });
    }
    out
}

/// The facts flowed into rows like the mockup's wrapping flex box: each as
/// wide as its text or its gauge, `gap` apart.
fn facts(ui: &mut egui::Ui, p: &Palette, list: &[Fact], gauge_w: f32, gap: f32) {
    let width_of = |ui: &egui::Ui, f: &Fact| {
        let size = if f.small { 12.5 } else { 14.0 };
        let mut w = ui.painter().layout_job(semibold(ui, &f.value, size, p.ink)).size().x;
        if !f.rest.is_empty() {
            w += ui
                .painter()
                .layout_no_wrap(format!(" · {}", f.rest), egui::FontId::proportional(12.0), p.muted)
                .size()
                .x;
        }
        let key = ui
            .painter()
            .layout_no_wrap(f.key.to_owned(), egui::FontId::proportional(11.0), p.muted)
            .size()
            .x;
        let gauge = if f.gauge.is_some() { gauge_w } else { 0.0 };
        w.max(key).max(gauge).ceil()
    };
    let room = ui.available_width();
    let mut rows: Vec<Vec<(&Fact, f32)>> = vec![Vec::new()];
    let mut used = 0.0;
    for f in list {
        let w = width_of(ui, f);
        let row = rows.last_mut().expect("one row at least");
        if !row.is_empty() && used + gap + w > room {
            rows.push(vec![(f, w)]);
            used = w;
        } else {
            used += if row.is_empty() { w } else { gap + w };
            row.push((f, w));
        }
    }
    for (i, row) in rows.into_iter().enumerate() {
        // the rows as far apart as the facts (the column puts 8 px)
        if i > 0 {
            ui.add_space(gap - 8.0);
        }
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (f, w) in row {
                ui.allocate_ui_with_layout(Vec2::new(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(w);
                    fact(ui, p, f.key, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.label(semibold(ui, &f.value, if f.small { 12.5 } else { 14.0 }, p.ink));
                            if !f.rest.is_empty() {
                                ui.label(RichText::new(format!(" · {}", f.rest)).size(12.0).color(p.muted));
                            }
                        });
                        if let Some((id, frac, tint)) = f.gauge {
                            sheet::gauge(ui, p, id, frac, gauge_w, tint);
                        }
                    });
                });
            }
        });
    }
}

/// One pill of the header: a rule of the region or of its estate.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Permit {
    pub icon: &'static str,
    pub tone: Tone,
    pub label: String,
    pub tip: &'static str,
}

/// The pills, in the mockup's order: the estate's access (for who manages
/// it), fly, damage, push, terraform, voice (for who manages it), resale,
/// join / subdivide. `estate`: the estate's flags once `estateupdateinfo`
/// answered.
pub(super) fn permits(v: &View, estate: Option<u64>) -> Vec<Permit> {
    let mut out = Vec::with_capacity(8);
    let region = |b: u32| v.region_flag(b);
    if v.manages()
        && let Some(e) = estate
    {
        let public = e & ef::EXTERNALLY_VISIBLE != 0;
        out.push(Permit {
            icon: "door-enter",
            tone: if public { Tone::Allow } else { Tone::Deny },
            label: if public {
                aurora_i18n::tr!("region-open-to-all")
            } else {
                aurora_i18n::tr!("region-lists-only")
            }
            .to_owned(),
            tip: if public {
                aurora_i18n::tr!("region-open-to-all-tip")
            } else {
                aurora_i18n::tr!("region-lists-only-tip")
            },
        });
    }
    let fly = !region(rf::BLOCK_FLY);
    out.push(Permit {
        icon: "plane-tilt-filled",
        tone: if fly { Tone::Allow } else { Tone::Deny },
        label: if fly {
            aurora_i18n::tr!("ui-land-overview-fly")
        } else {
            aurora_i18n::tr!("ui-land-overview-no-fly")
        }
        .to_owned(),
        tip: if fly {
            aurora_i18n::tr!("ui-parcel-icons-flight-allowed")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-flight-disabled-in-this-region-you-cannot-fly-here")
        },
    });
    let damage = region(rf::ALLOW_DAMAGE);
    out.push(Permit {
        icon: "heart-filled",
        tone: if damage { Tone::Deny } else { Tone::Allow },
        label: if damage {
            aurora_i18n::tr!("ui-land-overview-damage")
        } else {
            aurora_i18n::tr!("ui-land-overview-safe")
        }
        .to_owned(),
        tip: if damage {
            aurora_i18n::tr!("ui-land-overview-damage-tip")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-safe-zone-no-damage")
        },
    });
    let push = !region(rf::RESTRICT_PUSHOBJECT);
    out.push(Permit {
        icon: "hand-stop",
        tone: if push { Tone::Allow } else { Tone::Deny },
        label: if push {
            aurora_i18n::tr!("ui-land-overview-push")
        } else {
            aurora_i18n::tr!("ui-land-overview-no-push")
        }
        .to_owned(),
        tip: if push {
            aurora_i18n::tr!("ui-parcel-icons-pushing-allowed-other-people-s-scripts-can-push-you")
        } else {
            aurora_i18n::tr!("ui-parcel-icons-pushing-restricted-only-the-landowner-can-push-others")
        },
    });
    let terraform = !region(rf::BLOCK_TERRAFORM);
    out.push(Permit {
        icon: "mountain-filled",
        tone: if terraform { Tone::Allow } else { Tone::Deny },
        label: if terraform {
            aurora_i18n::tr!("region-terraform")
        } else {
            aurora_i18n::tr!("region-terraform-blocked")
        }
        .to_owned(),
        tip: if terraform {
            aurora_i18n::tr!("region-terraform-tip")
        } else {
            aurora_i18n::tr!("region-terraform-blocked-tip")
        },
    });
    if v.manages() {
        let voice = region(rf::ALLOW_VOICE);
        out.push(Permit {
            icon: "microphone-filled",
            tone: if voice { Tone::Allow } else { Tone::Deny },
            label: if voice {
                aurora_i18n::tr!("ui-land-overview-voice")
            } else {
                aurora_i18n::tr!("ui-land-overview-no-voice")
            }
            .to_owned(),
            tip: if voice {
                aurora_i18n::tr!("ui-parcel-icons-voice-allowed")
            } else {
                aurora_i18n::tr!("ui-parcel-icons-voice-disabled-in-this-region-you-will-not-hear")
            },
        });
    }
    let yes_no = |b: bool| if b { "yes" } else { "no" }.to_owned();
    let resell = !region(rf::BLOCK_LAND_RESELL);
    out.push(Permit {
        icon: "arrows-split",
        tone: if resell { Tone::Group } else { Tone::Deny },
        label: aurora_i18n::tr!("ui-land-overview-resell", allowed = yes_no(resell)),
        tip: if resell {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-can-be-resold")
        } else {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-cannot-be-resold")
        },
    });
    let changes = region(rf::ALLOW_PARCEL_CHANGES);
    out.push(Permit {
        icon: "layout-grid-filled",
        tone: if changes { Tone::Group } else { Tone::Deny },
        label: aurora_i18n::tr!("ui-land-overview-join-subdivide", allowed = yes_no(changes)),
        tip: if changes {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-can-be-joined-or")
        } else {
            aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-cannot-be-joined-or")
        },
    });
    out
}

/// The actions under the header, then the cards.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, compact: bool) {
    ui.add_space(10.0);
    actions(ui, p, s, v, world, compact);
    ui.add_space(12.0);
    let left = |ui: &mut egui::Ui, world: &mut World, s: &mut RegionUi| covenant_card(ui, p, s, v, world);
    let right = |ui: &mut egui::Ui, world: &mut World, s: &mut RegionUi| health_card(ui, p, s, v, world);
    if compact {
        // one column: each card the whole width
        let mut c = sheet::card_begin(ui);
        c.child.spacing_mut().item_spacing.y = 6.0;
        left(&mut c.child, world, s);
        sheet::card_end(ui, p, c);
        let mut c = sheet::card_begin(ui);
        c.child.spacing_mut().item_spacing.y = 6.0;
        right(&mut c.child, world, s);
        sheet::card_end(ui, p, c);
    } else {
        cards(ui, p, world, s, 1.25, left, right);
    }
}

fn actions(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, compact: bool) {
    use widgets::ActionKind::{Flat, SoftDanger};
    // compact: the icons alone, their names on hover
    let label = |text: &'static str| if compact { "" } else { text };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let about = aurora_i18n::tr!("common-about-land");
        if action(ui, p, "map-pin-filled", label(about), Flat, true)
            .on_hover_text(about)
            .clicked()
        {
            s.action = Some(RegionAction::AboutLand);
        }
        let map = aurora_i18n::tr!("region-map");
        if action(ui, p, "map", label(map), Flat, true).on_hover_text(map).clicked() {
            s.action = Some(RegionAction::WorldMap);
        }
        let point = if v.same_region {
            world.agent.position
        } else {
            glam::Vec3::new(128.0, 128.0, 0.0)
        };
        let slurl = (!v.name.is_empty()).then(|| crate::slurl::make(&v.name, point));
        if action(
            ui,
            p,
            "copy-filled",
            label(aurora_i18n::tr!("ui-land-overview-slurl")),
            Flat,
            slurl.is_some(),
        )
        .on_hover_text(aurora_i18n::tr!("common-copy-slurl"))
        .clicked()
            && let Some(url) = slurl
        {
            ui.ctx().copy_text(url);
        }
        // the interventions' shortcuts (Manage › Interventions)
        if v.manages() {
            let message = aurora_i18n::tr!("region-message");
            if action(ui, p, "speakerphone", label(message), Flat, true)
                .on_hover_text(message)
                .clicked()
            {
                s.interventions.dialog = Some(super::interventions::Dialog::message());
            }
            let top = aurora_i18n::tr!("region-top-scripts");
            if action(ui, p, "gauge-filled", label(top), Flat, !world.estate.top_waiting)
                .on_hover_text(top)
                .clicked()
            {
                super::top_objects::open(s, v, world, aurora_net::estate::land_stat::TOP_SCRIPTS);
            }
            let restart = aurora_i18n::tr!("region-restart");
            if action(ui, p, "refresh", label(restart), SoftDanger, true)
                .on_hover_text(restart)
                .clicked()
            {
                s.interventions.dialog = Some(super::interventions::Dialog::restart());
            }
        }
        more_menu(ui, p, world, v.handle);
    });
}

/// "…": the region's identifier.
fn more_menu(ui: &mut egui::Ui, p: &Palette, world: &World, handle: aurora_net::RegionHandle) {
    let Some(t) = crate::ui::icons::global("dots-filled") else {
        return;
    };
    let img = egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.ink);
    let button = egui::Button::image(img).min_size(Vec2::new(30.0, ACTION_H));
    let region_id = world
        .regions
        .get(&handle)
        .and_then(|r| r.info.as_ref())
        .map(|i| i.region_id)
        .filter(|id| !id.is_nil());
    menu::button(ui, p, button, |ui| {
        if menu::item_if(ui, p, "id-filled", aurora_i18n::tr!("region-copy-region-id"), region_id.is_some())
            && let Some(id) = region_id
        {
            ui.ctx().copy_text(id.to_string());
        }
    })
    .on_hover_text(aurora_i18n::tr!("ui-land-overview-more"));
}

/// The covenant (LLPanelEstateCovenant) and the region environment.
fn covenant_card(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let covenant = world.land.covenant.clone().filter(|c| c.handle == v.handle);
    let body = covenant
        .as_ref()
        .and_then(|c| c.text.clone())
        .unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
    let width = ui.available_width();
    let long = ui
        .painter()
        .layout(body.clone(), egui::FontId::proportional(12.5), p.muted, width)
        .size()
        .y
        > COVENANT_EXCERPT + 1.0;
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
    sheet::excerpt(ui, p, &body, s.covenant_open, COVENANT_EXCERPT);
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
        );
        let changes = v.region_flag(rf::ALLOW_PARCEL_CHANGES);
        pill(
            ui,
            p,
            Tone::Neutral,
            "layout-grid-filled",
            &aurora_i18n::tr!("ui-land-overview-join-subdivide", allowed = yes_no(changes)),
        );
        if let Some(c) = &covenant {
            let when = if c.timestamp == 0 {
                aurora_i18n::tr!("ui-land-overview-never-modified").to_owned()
            } else {
                aurora_i18n::tr!("ui-land-overview-modified-on", date = crate::ui::land::day(i64::from(c.timestamp)))
            };
            pill(ui, p, Tone::Neutral, "", &when);
        }
    });
    ui.add_space(6.0);
    card_title(ui, p, "sun-high-filled", aurora_i18n::tr!("common-environment"), None);
    kv(ui, p, aurora_i18n::tr!("region-day-cycle"), |ui| {
        let text = match &world.estate.environment {
            None => aurora_i18n::tr!("common-loading").to_owned(),
            Some(Some(e)) if e.has_day => {
                let hours = aurora_i18n::number(e.day_length.max(0) as f64 / 3600.0, 0);
                if e.day_name.is_empty() {
                    aurora_i18n::tr!("region-day-length", hours = hours)
                } else {
                    aurora_i18n::tr!("region-day-cycle-value", name = e.day_name.clone(), hours = hours)
                }
            }
            Some(_) => aurora_i18n::tr!("region-default-environment").to_owned(),
        };
        value(ui, p, text);
    });
    kv(ui, p, aurora_i18n::tr!("region-parcels"), |ui| {
        // the estate's flag (LLEstateInfoModel::getAllowEnvironmentOverride),
        // else the region's
        let flags = world.estate.estate.as_ref().map_or(v.flags, |e| e.flags);
        value(
            ui,
            p,
            if flags & ef::ALLOW_ENVIRONMENT_OVERRIDE != 0 {
                aurora_i18n::tr!("region-parcels-own-environment")
            } else {
                aurora_i18n::tr!("region-parcels-region-environment")
            },
        );
    });
}

/// The simulator's statistics and the region's facts.
fn health_card(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let link = v.manages().then(|| aurora_i18n::tr!("region-top-scripts-link"));
    if let Some(r) = card_title(ui, p, "gauge-filled", aurora_i18n::tr!("region-health"), link)
        && r.clicked()
        && !world.estate.top_waiting
    {
        super::top_objects::open(s, v, world, aurora_net::estate::land_stat::TOP_SCRIPTS);
    }
    let stats = world.estate.stats_of(v.handle).cloned();
    let get = |id: u32| stats.as_ref().and_then(|s| s.get(id));
    let loading = aurora_i18n::tr!("common-loading");
    let num = |v: f32, d: usize| aurora_i18n::number(f64::from(v), d);
    kv(ui, p, aurora_i18n::tr!("region-time-dilation"), |ui| {
        value(ui, p, get(sim_stat::TIME_DILATION).map_or(loading.to_owned(), |d| num(d, 2)));
    });
    kv(ui, p, aurora_i18n::tr!("region-frames"), |ui| {
        let text = match (get(sim_stat::FPS), get(sim_stat::PHYSFPS)) {
            (Some(f), Some(ph)) => aurora_i18n::tr!("region-frames-value", fps = num(f, 0), physics = num(ph, 0)),
            _ => loading.to_owned(),
        };
        value(ui, p, text);
    });
    kv(ui, p, aurora_i18n::tr!("region-script-time"), |ui| {
        let text = match (get(sim_stat::SCRIPTMS), get(sim_stat::NUMSCRIPTSACTIVE)) {
            (Some(ms), Some(n)) => aurora_i18n::tr!(
                "region-script-time-value",
                ms = num(ms, 1),
                n = n.round() as i64,
                count = aurora_i18n::integer(n.round() as i64)
            ),
            _ => loading.to_owned(),
        };
        value(ui, p, text);
    });
    kv(ui, p, aurora_i18n::tr!("region-spare-time"), |ui| {
        match (get(sim_stat::SIMSPARETIME), get(sim_stat::FRAMEMS)) {
            (Some(spare), Some(frame)) => {
                let text = aurora_i18n::tr!("region-spare-time-value", spare = num(spare, 1), frame = num(frame, 0));
                // little room left is the simulator's limit
                let ink = if spare >= 2.0 { p.success } else { p.warn };
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(ink)).truncate());
            }
            _ => {
                value(ui, p, loading);
            }
        }
    });
    ui.add_space(6.0);
    card_title(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-land-overview-info"), None);
    let details = world.estate.region.clone();
    kv(ui, p, aurora_i18n::tr!("region-type"), |ui| {
        let product = if v.product.is_empty() {
            aurora_i18n::tr!("region-type-unknown").to_owned()
        } else {
            crate::ui::land::product_name(&v.product)
        };
        value(ui, p, product);
    });
    kv(ui, p, aurora_i18n::tr!("region-estate-id"), |ui| {
        value(
            ui,
            p,
            details
                .as_ref()
                .map_or(loading.to_owned(), |d| aurora_i18n::integer(i64::from(d.estate_id))),
        );
    });
    kv(ui, p, aurora_i18n::tr!("region-grid-position"), |ui| {
        let (x, y) = grid_position(v.handle);
        value(ui, p, format!("({x}, {y})"));
    });
    kv(ui, p, aurora_i18n::tr!("region-server"), |ui| {
        let version = world.estate.versions.get(&v.handle).cloned().unwrap_or_else(|| "—".to_owned());
        ui.add(egui::Label::new(RichText::new(version).monospace().size(11.0).color(p.muted)).truncate());
    });
    kv(ui, p, aurora_i18n::tr!("ui-land-mod-experiences"), |ui| {
        let text = match &world.estate.experiences {
            None => loading.to_owned(),
            Some(None) => "—".to_owned(),
            Some(Some(x)) => aurora_i18n::tr!(
                "region-experiences-value",
                trusted = x.trusted.len() as i64,
                allowed = x.allowed.len() as i64
            ),
        };
        value(ui, p, text);
    });
}

/// A value of a card.
fn value(ui: &mut egui::Ui, p: &Palette, text: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text.into()).size(12.5).color(p.ink)).truncate())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(flags: u32, shown: Role) -> View {
        View {
            handle: aurora_net::origin_to_handle(1012 * 256, 998 * 256),
            role: shown,
            shown,
            name: "Aurora".into(),
            sim_access: 13,
            flags: u64::from(flags),
            owner: Uuid::from_u128(1),
            product: String::new(),
            same_region: true,
        }
    }

    #[test]
    fn the_health_follows_the_time_dilation() {
        assert_eq!(health(0.99), Health::Smooth);
        assert_eq!(health(0.95), Health::Smooth);
        assert_eq!(health(0.8), Health::Slowed);
        assert_eq!(health(0.5), Health::Struggling);
    }

    #[test]
    fn the_grid_position_and_the_version() {
        assert_eq!(grid_position(view(0, Role::Owner).handle), (1012, 998));
        assert_eq!(short_version("Second Life Server 2026.10.02.4893"), "2026.10.02.4893");
        assert_eq!(short_version("2026.10.02"), "2026.10.02");
    }

    /// A resident sees the region's rules, not the estate's access nor its
    /// voice; the region's flags decide the tones.
    #[test]
    fn the_pills_follow_the_flags_and_the_role() {
        let flags = rf::BLOCK_TERRAFORM | rf::RESTRICT_PUSHOBJECT | rf::ALLOW_VOICE;
        let owner = permits(&view(flags, Role::Owner), Some(ef::EXTERNALLY_VISIBLE));
        let icons: Vec<_> = owner.iter().map(|p| (p.icon, p.tone)).collect();
        assert_eq!(
            icons,
            vec![
                ("door-enter", Tone::Allow),
                ("plane-tilt-filled", Tone::Allow),
                ("heart-filled", Tone::Allow),
                ("hand-stop", Tone::Deny),
                ("mountain-filled", Tone::Deny),
                ("microphone-filled", Tone::Allow),
                ("arrows-split", Tone::Group),
                ("layout-grid-filled", Tone::Deny),
            ]
        );
        let resident = permits(&view(flags, Role::Resident), Some(ef::EXTERNALLY_VISIBLE));
        assert_eq!(resident.len(), 6);
        assert!(resident.iter().all(|p| p.icon != "door-enter" && p.icon != "microphone-filled"));
        // the estate's flags not answered yet: no access pill
        assert_eq!(permits(&view(flags, Role::Owner), None).len(), 7);
    }

    #[test]
    fn the_schedule_says_its_days() {
        let mut s = RestartSchedule {
            days: [true, false, false, false, false, false, false],
            seconds: 6 * 3600,
        };
        let sunday = schedule_text(&s);
        assert!(sunday.contains("06:00"), "{sunday}");
        assert!(sunday.chars().next().is_some_and(char::is_uppercase), "{sunday}");
        s.days = [true; 7];
        assert!(schedule_text(&s).starts_with(aurora_i18n::tr!("region-schedule-every-day")));
    }
}
