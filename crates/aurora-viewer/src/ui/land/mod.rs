//! "À propos du terrain", laid out like Firestorm's LLFloaterLand
//! (newview/llfloaterland.cpp, llpanellandaudio.cpp, llpanellandmedia.cpp,
//! llpanelenvironment.cpp, floater_about_land.xml, originally LGPL 2.1):
//! nine tabs on the selected parcel (LLViewerParcelMgr's selection, see
//! world/land.rs), each control enabled by the same rights as Firestorm
//! (isParcelModifiableByAgent with the matching group power). Changes are
//! sent at once like the panels' onCommitAny (sendParcelPropertiesUpdate,
//! access list updates...).

mod access;
mod covenant;
mod dialogs;
mod environment;
mod experiences;
mod files;
mod general;
mod media;
mod objects;
mod options;
mod sound;

use super::avatar_picker::AvatarPicker;
use super::texture_picker::TexturePicker;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{AgentRights, modifiable_by_agent, owned_by_agent};
use aurora_net::{GroupMembership, ParcelInfo, RegionHandle};
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    General,
    Covenant,
    Objects,
    Options,
    Media,
    Sound,
    Access,
    Experiences,
    Environment,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::General => "Général",
            Tab::Covenant => "Règlement",
            Tab::Objects => "Objets",
            Tab::Options => "Options",
            Tab::Media => "Médias",
            Tab::Sound => "Son",
            Tab::Access => "Accès",
            Tab::Experiences => "Expériences",
            Tab::Environment => "Environnement",
        }
    }
}

/// Width of the left column of labels (left="10" width="100" in the XML).
const KEY_W: f32 = 112.0;

/// A text field being edited against the server's value: it follows the
/// parcel until it has the focus, and commits when it loses it (or on
/// Enter), like LLLineEditor's commit on focus lost.
#[derive(Default)]
struct Draft {
    text: String,
    base: String,
    editing: bool,
}

impl Draft {
    fn sync(&mut self, server: &str) {
        if !self.editing && self.base != server {
            self.base = server.to_owned();
            self.text = server.to_owned();
        }
    }

    /// Track the widget's focus; Some(text) when a change is committed.
    fn after(&mut self, r: &egui::Response) -> Option<String> {
        self.editing = r.has_focus();
        if r.lost_focus() && self.text != self.base {
            self.base = self.text.clone();
            return Some(self.text.clone());
        }
        None
    }
}

/// What the tabs read about the selection, its region and the agent.
struct View {
    parcel: Option<Arc<ParcelInfo>>,
    handle: RegionHandle,
    region: Option<RegionView>,
    agent: Uuid,
    groups: Vec<GroupMembership>,
    /// The selection is in the agent's region.
    same_region: bool,
    /// Unix time (s).
    now: i64,
}

struct RegionView {
    name: String,
    owner: Uuid,
    estate_manager: bool,
    product: String,
    sim_access: u8,
    flags: u32,
    max_tasks: u32,
    has_land_resources: bool,
}

impl View {
    fn rights(&self) -> AgentRights<'_> {
        AgentRights {
            id: self.agent,
            groups: &self.groups,
        }
    }

    /// isParcelModifiableByAgent(parcel, power).
    fn can(&self, power: u64) -> bool {
        self.parcel.as_ref().is_some_and(|p| modifiable_by_agent(p, &self.rights(), power))
    }

    /// isParcelOwnedByAgent(parcel, power).
    fn owns(&self, power: u64) -> bool {
        self.parcel.as_ref().is_some_and(|p| owned_by_agent(p, &self.rights(), power))
    }

    /// Region flag of the selection region.
    fn region_flag(&self, flag: u32) -> bool {
        self.region.as_ref().is_some_and(|r| r.flags & flag != 0)
    }
}

/// Modal questions of the floater (LLNotificationsUtil confirmations and
/// the small floaters it opens).
enum Dialog {
    Confirm(dialogs::Confirm),
    /// LLFloaterGroupPicker for "Choisir".
    GroupPicker,
    /// LLFloaterSellLand.
    Sell(dialogs::SellForm),
    /// LLFloaterBanDuration for the residents picked.
    BanDuration { ids: Vec<Uuid>, temporary: bool, hours: u32 },
    /// A message with only "OK" (alerts of the panels).
    Message(String),
    /// LLFloaterURLEntry for the media home page.
    MediaUrl(String),
}

/// Who the avatar picker is choosing for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PickFor {
    Access,
    Ban,
    SellTo,
}

#[derive(Default)]
pub struct LandUi {
    tab: Tab,
    was_open: bool,
    name: Draft,
    desc: Draft,
    clean_time: Draft,
    media_desc: Draft,
    music_url: Draft,
    dialog: Option<Dialog>,
    picker: AvatarPicker,
    pick_for: Option<PickFor>,
    texture: TexturePicker,
    /// Which texture the picker is choosing: true = snapshot, false = media.
    texture_snapshot: bool,
    /// Object owners list: selected row, sort column and order.
    owner_sel: Option<Uuid>,
    owner_sort: Option<(usize, bool)>,
    access_sel: HashSet<Uuid>,
    ban_sel: HashSet<Uuid>,
    allowed_exp_sel: Option<Uuid>,
    blocked_exp_sel: Option<Uuid>,
    /// Day length / offset being dragged (hours), committed on release.
    env_drag: Option<(f32, f32)>,
    /// Sound tab: the music URL last sent (parcel, URL) and the selection
    /// revision then (FIRE-29157).
    last_music_url: Option<(i32, String)>,
    last_music_rev: u64,
    /// Access tab: passes sold to the group (the combo, not kept by the
    /// parcel), pass price / hours being edited.
    pass_to_group: bool,
    pass_edit: Option<(i32, f32)>,
    /// Export / import file dialog running.
    file_job: Option<files::Job>,
    /// Images to stream (snapshot, media texture).
    pub wanted_images: HashSet<Uuid>,
}

impl LandUi {
    /// Open on the parcel at a global position (right-click on land, mini-map).
    pub fn select_at_global(world: &mut World, gx: f64, gy: f64) {
        let hit = world.regions.iter().find_map(|(h, r)| {
            let (ox, oy) = aurora_net::handle_to_origin(*h);
            let (sx, sy) = (r.heightmap.size_x.max(256) as f64, r.heightmap.size_y.max(256) as f64);
            let (lx, ly) = (gx - ox as f64, gy - oy as f64);
            ((0.0..sx).contains(&lx) && (0.0..sy).contains(&ly)).then_some((*h, glam::Vec3::new(lx as f32, ly as f32, 0.0), (sx as f32, sy as f32)))
        });
        if let Some((h, pos, size)) = hit {
            world.land.select_at(h, pos, size);
        }
    }

    /// LLFloaterLand::onOpen: no selection, the agent's parcel.
    fn select_agent_parcel(world: &mut World) {
        if let Some(h) = world.main_region {
            let size = world
                .regions
                .get(&h)
                .map_or((256.0, 256.0), |r| (r.heightmap.size_x.max(256) as f32, r.heightmap.size_y.max(256) as f32));
            let pos = world.agent.position;
            world.land.select_at(h, pos, size);
        }
    }

    /// Show the floater; `open` is the panel flag (the top bar ⓘ, menus),
    /// `streams` the saved music streams. True when they changed.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
        streams: &mut Vec<String>,
        demo: bool,
        open: &mut bool,
    ) -> bool {
        let mut streams_changed = false;
        if !*open {
            if self.was_open {
                // onVisibilityChanged: the selection is dropped with the floater
                world.land.deselect();
                self.dialog = None;
                self.picker.open = false;
                self.texture.open = false;
                self.was_open = false;
            }
            return false;
        }
        if !self.was_open {
            self.was_open = true;
            if world.land.sel.is_none() {
                Self::select_agent_parcel(world);
            }
        }
        let view = self.view(world);
        let screen = ctx.content_rect();
        let size = Vec2::new(600.0, 470.0);
        let mut floater = Floater::new(
            "about_land",
            "À propos du terrain",
            egui::pos2(screen.center().x - size.x / 2.0, 80.0),
            size,
        )
        .help("Informations et réglages de la parcelle sélectionnée");
        floater.min_size = Vec2::new(560.0, 440.0);
        floater.show(ctx, p, open, |ui| {
            // FIRE-17280: no experiences tab where the region has none
            let has_xp = demo
                || world
                    .regions
                    .get(&view.handle)
                    .is_some_and(|r| r.caps.contains_key("RegionExperiences"));
            let tabs: Vec<Tab> = [
                Tab::General,
                Tab::Covenant,
                Tab::Objects,
                Tab::Options,
                Tab::Media,
                Tab::Sound,
                Tab::Access,
                Tab::Experiences,
                Tab::Environment,
            ]
            .into_iter()
            .filter(|t| *t != Tab::Experiences || has_xp)
            .collect();
            let labels: Vec<(&str, bool)> = tabs.iter().map(|t| (t.label(), true)).collect();
            let mut idx = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
            widgets::tabs(ui, p, &mut idx, &labels);
            self.tab = tabs[idx];
            ui.add_space(8.0);
            let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
            ui.allocate_rect(body, egui::Sense::hover());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            let ui = &mut child;
            ui.spacing_mut().item_spacing.y = 5.0;
            match self.tab {
                Tab::General => general::show(ui, p, self, &view, world),
                Tab::Covenant => covenant::show(ui, p, &view, world),
                Tab::Objects => objects::show(ui, p, self, &view, world),
                Tab::Options => options::show(ui, p, self, &view, world, images),
                Tab::Media => media::show(ui, p, self, &view, world, images),
                Tab::Sound => streams_changed = sound::show(ui, p, self, &view, world, streams),
                Tab::Access => access::show(ui, p, self, &view, world),
                Tab::Experiences => experiences::show(ui, p, self, &view, world),
                Tab::Environment => environment::show(ui, p, self, &view, world),
            }
        });
        dialogs::show(ctx, p, self, &view, world);
        if let Some(ids) = self.picker.show(ctx, p, world) {
            match self.pick_for.take() {
                Some(PickFor::Access) => access::add_allowed(world, &ids),
                Some(PickFor::Ban) => {
                    // LLFloaterBanDuration comes next
                    self.dialog = Some(Dialog::BanDuration {
                        ids,
                        temporary: false,
                        hours: 1,
                    })
                }
                Some(PickFor::SellTo) => {
                    if let Some(Dialog::Sell(form)) = self.dialog.as_mut()
                        && let Some(id) = ids.first()
                    {
                        form.buyer = Some(*id);
                        world.social.want_name(*id);
                    }
                }
                None => {}
            }
        }
        if let Some(asset) = self.texture.show(ctx, p, world, images) {
            if self.texture_snapshot {
                world.land.update(|u| u.snapshot_id = asset);
            } else {
                world.land.update(|u| u.media_id = asset);
            }
        }
        self.wanted_images.extend(self.texture.wanted_images.drain());
        streams_changed
    }

    fn view(&self, world: &World) -> View {
        let sel = world.land.sel.as_ref();
        let handle = sel.map_or(world.main_region.unwrap_or(0), |s| s.handle);
        let region = world.regions.get(&handle).and_then(|r| {
            r.info.as_ref().map(|i| RegionView {
                name: i.name.clone(),
                owner: i.owner,
                estate_manager: i.is_estate_manager,
                product: i.product_name.clone(),
                sim_access: i.sim_access,
                flags: i.region_flags,
                max_tasks: r.max_tasks,
                has_land_resources: r.caps.contains_key("LandResources"),
            })
        });
        View {
            parcel: sel.and_then(|s| s.parcel.clone()),
            handle,
            region,
            agent: world.agent_id,
            groups: world.groups.groups.clone(),
            same_region: world.main_region == Some(handle),
            now: now_secs(),
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// "Aucune parcelle sélectionnée." (no_selection_text) while waiting.
fn no_selection(ui: &mut egui::Ui, p: &Palette) {
    ui.add_space(8.0);
    ui.label(RichText::new("Aucune parcelle sélectionnée.").size(12.0).color(p.muted));
}

/// The label of a row, in the left column.
fn key(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.allocate_ui_with_layout(Vec2::new(KEY_W, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).truncate());
    });
}

/// A row: label then content.
fn row<R>(ui: &mut egui::Ui, p: &Palette, label: &str, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        key(ui, p, label);
        body(ui)
    })
    .inner
}

fn text(ui: &mut egui::Ui, p: &Palette, s: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(s.into()).size(12.0).color(p.ink)).wrap())
}

fn dim(ui: &mut egui::Ui, p: &Palette, s: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(s.into()).size(12.0).color(p.muted)).wrap())
}

/// A resident's name that opens the profile (LLSLURL "agent ... inspect").
fn agent_link(ui: &mut egui::Ui, p: &Palette, world: &mut World, id: Uuid) {
    world.social.want_name(id);
    let name = world.social.name_of(&id);
    if ui
        .add(egui::Label::new(RichText::new(name).size(12.0).color(p.violet_light)).sense(egui::Sense::click()))
        .on_hover_text("Voir le profil")
        .clicked()
    {
        super::profile::request_open(ui.ctx(), id);
    }
}

/// A flat button, greyed when `enabled` is false.
fn button(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| widgets::flat_button(ui, p, label)).inner
}

/// A small Phosphor icon button (refresh, +, −, copy).
fn icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(22.0), sense);
    let fill = if enabled && r.hovered() { p.raised.gamma_multiply(1.3) } else { p.raised };
    ui.painter().rect_filled(rect, 2.0, fill);
    if let Some(t) = super::icons::global(icon) {
        ui.painter().image(
            t.id(),
            rect.shrink(4.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if enabled { p.ink } else { p.muted_dim },
        );
    }
    r
}

/// A checkbox that reports a click (the value is the parcel's, not kept).
fn check(ui: &mut egui::Ui, enabled: bool, mut value: bool, label: &str) -> (egui::Response, bool) {
    let r = ui.add_enabled(enabled, egui::Checkbox::new(&mut value, RichText::new(label).size(12.0)));
    (r, value)
}

/// "Pas encore disponible dans Aurora" for the few Firestorm floaters not ported yet.
const NOT_YET: &str = "Pas encore disponible dans Aurora";

/// Region type in French (LLViewerRegion::getLocalizedSimProductName).
fn product_name(product: &str) -> String {
    match product {
        "Estate / Full Region" => "Domaine / Région entière",
        "Estate / Homestead" => "Domaine / Homestead",
        "Mainland / Homestead" => "Continent / Homestead",
        "Mainland / Full Region" => "Continent / Région entière",
        "Linden Homes / Full Region" => "Maisons Linden / Région entière",
        other => other,
    }
    .to_owned()
}

/// Maturity badge and name (insert_maturity_into_textbox).
fn maturity(ui: &mut egui::Ui, p: &Palette, v: &View) {
    let name = v.region.as_ref().map_or("", |r| super::bars::maturity_name(r.sim_access));
    widgets::maturity_badge(ui, p, name, 14.0);
    text(ui, p, name);
}

/// Seconds since the epoch in SLT (US Pacific, DST approximated by the
/// day of the year like the top bar): (year, month, day, weekday 0 = Sunday,
/// hour, minute, second).
fn slt_parts(secs: i64) -> (i64, usize, i64, usize, i64, i64, i64) {
    let year_day = (secs / 86400).rem_euclid(365);
    let off = if (68..=307).contains(&year_day) { -7 } else { -8 };
    let t = secs + off * 3600;
    let days = t.div_euclid(86400);
    let tod = t.rem_euclid(86400);
    // civil date from days since 1970-01-01 (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    // 1970-01-01 was a Thursday
    let wday = (days + 4).rem_euclid(7) as usize;
    (y, m as usize, d, wday, tod / 3600, (tod / 60) % 60, tod % 60)
}

const WEEKDAYS: [&str; 7] = ["dim.", "lun.", "mar.", "mer.", "jeu.", "ven.", "sam."];
const MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc.",
];

/// time_stamp_template of the French skin: "sam. 18 juin 2022 11:19:10".
fn claim_date(secs: i64) -> String {
    let (y, m, d, w, h, mi, s) = slt_parts(secs);
    format!("{} {d} {} {y} {h:02}:{mi:02}:{s:02}", WEEKDAYS[w], MONTHS[m - 1])
}

/// process_covenant_reply with the French LTime strings:
/// "mer. juin 05 21:18:48 2024".
fn covenant_date(secs: i64) -> String {
    let (y, m, d, w, h, mi, s) = slt_parts(secs);
    format!("{} {} {d:02} {h:02}:{mi:02}:{s:02} {y}", WEEKDAYS[w], MONTHS[m - 1])
}

/// LLResMgr::getMonetaryString for French: thousands with a space.
fn money(v: i32) -> String {
    let s = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{a0}');
        }
        out.push(c);
    }
    if v < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_like_the_french_skin() {
        // 2022-06-18 18:19:10 UTC = 11:19:10 PDT, a Saturday
        assert_eq!(claim_date(1_655_576_350), "sam. 18 juin 2022 11:19:10");
        // 2024-06-06 04:18:48 UTC = Wed 5 June 21:18:48 PDT
        assert_eq!(covenant_date(1_717_647_528), "mer. juin 05 21:18:48 2024");
    }

    #[test]
    fn money_groups_thousands() {
        assert_eq!(money(88000), "88\u{a0}000");
        assert_eq!(money(500), "500");
        assert_eq!(money(1_234_567), "1\u{a0}234\u{a0}567");
    }

    #[test]
    fn product_names_in_french() {
        assert_eq!(product_name("Mainland / Full Region"), "Continent / Région entière");
        assert_eq!(product_name("Estate / Openspace"), "Estate / Openspace");
    }
}
