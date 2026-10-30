//! "About Land", laid out like Firestorm's LLFloaterLand
//! (newview/llfloaterland.cpp, llpanellandaudio.cpp, llpanellandmedia.cpp,
//! llpanelenvironment.cpp, floater_about_land.xml, originally LGPL 2.1):
//! nine tabs on the selected parcel (LLViewerParcelMgr's selection, see
//! world/land.rs), each control enabled by the same rights as Firestorm
//! (isParcelModifiableByAgent with the matching group power). Changes are
//! sent at once like the panels' onCommitAny (sendParcelPropertiesUpdate,
//! access list updates...). By default the same panes are laid out by
//! Aurora (`aurora.rs`): an overview first, then the settings and the lists;
//! the nine tabs stay in Preferences › Aurora.

mod access;
mod aurora;
mod covenant;
mod dialogs;
mod environment;
mod experiences;
pub(crate) mod files;
/// The FS:PP import of a CSV list, also the estate lists'.
pub(crate) use access::import_csv;
mod general;
mod manage;
mod media;
mod objects;
mod options;
mod overview;
mod people;
mod scripts;
mod sound;

use aurora::Page;
pub(crate) use dialogs::{pick_row, settings_flags};
pub(crate) use environment::day_hours;
pub(crate) use overview::day;

use super::avatar_picker::AvatarPicker;
use super::texture_picker::TexturePicker;
use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{AgentRights, PrimRoom, PrimUsage, modifiable_by_agent, owned_by_agent};
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
    const ALL: [Tab; 9] = [
        Tab::General,
        Tab::Covenant,
        Tab::Objects,
        Tab::Options,
        Tab::Media,
        Tab::Sound,
        Tab::Access,
        Tab::Experiences,
        Tab::Environment,
    ];

    fn icon(self) -> &'static str {
        match self {
            Tab::General => "home-filled",
            Tab::Covenant => "file-description-filled",
            Tab::Objects => "box",
            Tab::Options => "settings-filled",
            Tab::Media => "device-tv-filled",
            Tab::Sound => "device-speaker-filled",
            Tab::Access => "key-filled",
            Tab::Experiences => "flask-filled",
            Tab::Environment => "leaf-filled",
        }
    }

    /// Name of the tab for AURORA_DEMO_LAND, whatever the interface language.
    fn key(self) -> &'static str {
        match self {
            Tab::General => "general",
            Tab::Covenant => "covenant",
            Tab::Objects => "objects",
            Tab::Options => "options",
            Tab::Media => "media",
            Tab::Sound => "sound",
            Tab::Access => "access",
            Tab::Experiences => "experiences",
            Tab::Environment => "environment",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::General => aurora_i18n::tr!("common-general"),
            Tab::Covenant => aurora_i18n::tr!("ui-land-mod-rules"),
            Tab::Objects => aurora_i18n::tr!("common-objects"),
            Tab::Options => aurora_i18n::tr!("ui-land-mod-options"),
            Tab::Media => aurora_i18n::tr!("ui-land-mod-media"),
            Tab::Sound => aurora_i18n::tr!("common-sound"),
            Tab::Access => aurora_i18n::tr!("ui-land-mod-access"),
            Tab::Experiences => aurora_i18n::tr!("ui-land-mod-experiences"),
            Tab::Environment => aurora_i18n::tr!("common-environment"),
        }
    }
}

/// Width of the left column of labels (left="10" width="100" in the XML).
const KEY_W: f32 = 140.0;

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
#[derive(Clone)]
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

#[derive(Clone)]
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
    /// LLFloaterGroupPicker for "Choose".
    GroupPicker,
    /// LLFloaterSellLand.
    Sell(dialogs::SellForm),
    /// LLFloaterBanDuration for the residents picked.
    BanDuration {
        ids: Vec<Uuid>,
        temporary: bool,
        hours: u32,
        /// Residents already banned whose duration changes.
        replace: bool,
    },
    /// A message with only "OK" (alerts of the panels).
    Message(String),
    /// LLFloaterURLEntry for the media home page.
    MediaUrl(String),
    /// LLFloaterExperiencePicker for a list of the parcel
    /// (AL_ALLOW_EXPERIENCE / AL_BLOCK_EXPERIENCE).
    ExperiencePicker(dialogs::ExperiencePicker),
    /// LLFloaterSettingsPicker for the parcel's environment.
    EnvironmentPicker(dialogs::EnvironmentPicker),
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
    /// The classic window's tab.
    tab: Tab,
    /// The Aurora window's page; on "Manage" the section in view, the one
    /// to scroll to and the folded parts that are open.
    page: Page,
    manage_nav: crate::ui::sheet::SectionScroll<manage::Section>,
    manage_open: HashSet<manage::Section>,
    /// The owner (or an officer) looks at the parcel as a visitor would.
    view_as_visitor: bool,
    /// "People and objects": the search fields of the two lists and the
    /// object owners ticked.
    access_filter: String,
    ban_filter: String,
    owners_sel: HashSet<Uuid>,
    /// Demo captures of "People and objects": ask for the object owners
    /// once, as the Refresh button does.
    refresh_owners_once: bool,
    /// Demo captures: scroll "People and objects" to the object owners.
    scroll_to_owners: bool,
    /// Demo captures: "Show" the owner's objects once the parcel is known.
    show_once: bool,
    /// The overview's covenant shown in full.
    covenant_open: bool,
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
    /// "Custom" was chosen: the app opens the day cycle editor on the
    /// parcel's day (taken once).
    pub custom_day: bool,
    /// Sound tab: the music URL last sent (parcel, URL) and the selection
    /// revision then (FIRE-29157).
    last_music_url: Option<(i32, String)>,
    last_music_rev: u64,
    /// Access tab: passes sold to the group (the combo, not kept by the
    /// parcel), pass price / hours being edited.
    pass_to_group: bool,
    pass_edit: Option<(i32, f32)>,
    /// "Buy access" of the ground menu: when it was asked, while the
    /// properties of the parcel it selected are awaited.
    pass_wanted: Option<std::time::Instant>,
    /// Export / import file dialog running.
    file_job: Option<files::Job>,
    /// Offline demo: the region answers LandResources.
    demo: bool,
    /// "Script info" (LLFloaterScriptLimits).
    scripts: scripts::ScriptsUi,
    /// Images to stream (snapshot, media texture).
    pub wanted_images: HashSet<Uuid>,
    /// Residents whose profile picture the header waits for.
    pub wanted_pics: HashSet<Uuid>,
}

impl LandUi {
    /// Open on the parcel at a global position (right-click on land, mini-map).
    pub fn select_at_global(world: &mut World, gx: f64, gy: f64) {
        if let Some((h, pos, size)) = Self::region_at_global(world, gx, gy) {
            world.land.select_at(h, pos, size);
        }
    }

    /// Land.BuyPass of the ground menu (LLLandBuyPass): the parcel at a
    /// global position is selected, and the question follows once its
    /// properties arrive, with or without the floater.
    pub fn buy_pass_at(&mut self, world: &mut World, gx: f64, gy: f64) {
        if let Some((h, pos, size)) = Self::region_at_global(world, gx, gy) {
            world.land.select_at(h, pos, size);
            self.pass_wanted = Some(std::time::Instant::now());
        }
    }

    /// The pass question is shown.
    fn asks_pass(&self) -> bool {
        matches!(self.dialog, Some(Dialog::Confirm(dialogs::Confirm::BuyPass)))
    }

    /// LLPanelLandGeneral::onClickBuyPass, once the parcel asked by the
    /// ground menu is known. The menu cannot grey the entry as
    /// LLMenuParcelObserver does (it does not know the parcel before it is
    /// selected), so a parcel without a pass for us says why instead.
    fn pass_question(&mut self, world: &mut World) {
        /// No answer from the simulator: the question is dropped.
        const WAIT: std::time::Duration = std::time::Duration::from_secs(15);
        let Some(asked) = self.pass_wanted else {
            return;
        };
        let Some(sel) = world.land.sel.as_ref() else {
            self.pass_wanted = None;
            return;
        };
        let Some(parcel) = sel.parcel.clone() else {
            if asked.elapsed() > WAIT {
                log::info!("Buy access: no parcel properties after {} s", WAIT.as_secs());
                self.pass_wanted = None;
            }
            return;
        };
        self.pass_wanted = None;
        let why = match overview::pass_refusal(&parcel, world.agent_id, overview::collision_banned(world)) {
            None => {
                self.dialog = Some(Dialog::Confirm(dialogs::Confirm::BuyPass));
                return;
            }
            Some(overview::NoPass::Own) => aurora_i18n::tr!("ui-land-pass-own-land"),
            Some(overview::NoPass::NotSold) => aurora_i18n::tr!("ui-land-pass-not-sold"),
            Some(overview::NoPass::Banned) => aurora_i18n::tr!("ui-land-pass-banned"),
        };
        world.notifications.push(
            crate::world::notifications::Kind::Info,
            aurora_i18n::tr!("ui-context-buy-access"),
            why,
            crate::world::notifications::Data::None,
        );
    }

    /// The region we are connected to that holds a global position: its
    /// handle, the position in it and its size. "About Land" only reaches
    /// the parcels of these regions.
    pub fn region_at_global(world: &World, gx: f64, gy: f64) -> Option<(aurora_net::RegionHandle, glam::Vec3, (f32, f32))> {
        world.regions.iter().find_map(|(h, r)| {
            let (ox, oy) = aurora_net::handle_to_origin(*h);
            let (sx, sy) = (r.heightmap.size_x.max(256) as f64, r.heightmap.size_y.max(256) as f64);
            let (lx, ly) = (gx - ox as f64, gy - oy as f64);
            ((0.0..sx).contains(&lx) && (0.0..sy).contains(&ly)).then_some((
                *h,
                glam::Vec3::new(lx as f32, ly as f32, 0.0),
                (sx as f32, sy as f32),
            ))
        })
    }

    /// Show a tab by its key (`Tab::key`), its label in the interface language
    /// or its index (demo captures). The Aurora window opens the page that
    /// holds it, or a page by its key (`Page::key`).
    pub fn set_tab(&mut self, name: &str) {
        let key = name.trim().to_lowercase();
        // the pickers of the environment and of the experiences, open
        if key == "environment-picker" || key == "experience-picker" {
            let env = key == "environment-picker";
            let section = if env {
                manage::Section::Environment
            } else {
                manage::Section::Experiences
            };
            self.page = Page::Manage;
            self.manage_nav.go(section);
            self.manage_open.insert(section);
            self.dialog = Some(if env {
                Dialog::EnvironmentPicker(dialogs::EnvironmentPicker {
                    track: None,
                    filter: String::new(),
                    selected: None,
                })
            } else {
                let mut picker = dialogs::ExperiencePicker::new(aurora_net::land::AL_ALLOW_EXPERIENCE, 42);
                picker.search_now = true;
                Dialog::ExperiencePicker(picker)
            });
            return;
        }
        if matches!(
            key.as_str(),
            "scripts" | "scripts-descending" | "scripts-name" | "scripts-owner" | "scripts-empty" | "scripts-attachments"
        ) {
            self.page = Page::Manage;
            self.manage_nav.go(manage::Section::Sale);
            self.manage_open.insert(manage::Section::Sale);
            self.scripts.demo(&key);
            return;
        }
        if key == "show-objects" {
            self.page = Page::Manage;
            self.manage_nav.go(manage::Section::Objects);
            self.show_once = true;
            return;
        }
        if key == "owners" {
            self.page = Page::People;
            self.refresh_owners_once = true;
            self.scroll_to_owners = true;
            return;
        }
        if let Some(page) = Page::ALL.into_iter().find(|p| p.key() == key) {
            self.page = page;
            self.refresh_owners_once = page == Page::People;
            return;
        }
        let by_name = |t: &&Tab| {
            let l = t.label().to_lowercase();
            t.key() == key || l == key || l.replace(['é', 'è'], "e") == key
        };
        if let Some(t) = Tab::ALL
            .iter()
            .find(by_name)
            .or_else(|| key.parse::<usize>().ok().and_then(|i| Tab::ALL.get(i)))
        {
            self.tab = *t;
            let (page, section) = Page::of(*t);
            self.page = page;
            if let Some(section) = section {
                self.manage_nav.go(section);
                self.manage_open.insert(section);
            }
        }
    }

    /// LLFloaterLand::onOpen: no selection, the agent's parcel.
    fn select_agent_parcel(world: &mut World) {
        if let Some(h) = world.main_region {
            let size = world.regions.get(&h).map_or((256.0, 256.0), |r| {
                (r.heightmap.size_x.max(256) as f32, r.heightmap.size_y.max(256) as f32)
            });
            let pos = world.agent.position;
            world.land.select_at(h, pos, size);
        }
    }

    /// Show the floater; `open` is the panel flag (the top bar ⓘ, menus),
    /// `region_map` the mini-map's image of the selection's region (the
    /// classic header), `avatars` the profile pictures, `streams` the saved
    /// music streams. `classic`: Firestorm's nine tabs rather than Aurora's
    /// pages. True when the streams changed.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
        avatars: &HashMap<Uuid, egui::TextureHandle>,
        region_map: Option<&egui::TextureHandle>,
        streams: &mut Vec<String>,
        demo: bool,
        classic: bool,
        open: &mut bool,
    ) -> bool {
        let mut streams_changed = false;
        if !*open {
            if self.was_open {
                // onVisibilityChanged: the selection is dropped with the
                // floater, and the objects "Show" highlighted (unhighlightAll);
                // the pass question keeps it until answered (EXT-6464)
                world.land.unhighlight();
                if !self.asks_pass() {
                    self.dialog = None;
                }
                if self.pass_wanted.is_none() && !self.asks_pass() {
                    world.land.deselect();
                }
                self.scripts.open = false;
                self.picker.open = false;
                self.texture.open = false;
                self.was_open = false;
            }
            // the ground menu's question, without the floater; a selection
            // only the question used goes with it
            let asking = self.pass_wanted.is_some() || self.asks_pass();
            self.pass_question(world);
            if self.asks_pass() {
                let view = self.view(world);
                dialogs::show(ctx, p, self, &view, world);
            }
            if asking && self.pass_wanted.is_none() && !self.asks_pass() {
                world.land.deselect();
            }
            return false;
        }
        self.pass_question(world);
        if !self.was_open {
            self.was_open = true;
            if world.land.sel.is_none() {
                Self::select_agent_parcel(world);
            }
        }
        if self.show_once && world.land.sel.as_ref().is_some_and(|s| s.parcel.is_some()) {
            self.show_once = false;
            world.land.show_objects(aurora_net::land::RT_OWNER, Vec::new());
        }
        self.demo = demo;
        let view = self.view(world);
        let screen = ctx.content_rect();
        // FIRE-17280: no experiences tab where the region has none
        let has_xp = demo
            || world
                .regions
                .get(&view.handle)
                .is_some_and(|r| r.caps.contains_key("RegionExperiences"));
        // each layout keeps its own size
        let (id, size, min_size) = if classic {
            ("about_land", Vec2::new(790.0, 620.0), Vec2::new(740.0, 470.0))
        } else {
            ("about_land_aurora", aurora::SIZE, aurora::MIN_SIZE)
        };
        let mut floater = Floater::new(
            id,
            aurora_i18n::tr!("common-about-land"),
            egui::pos2(screen.center().x - size.x / 2.0, 80.0),
            size,
        )
        .help(aurora_i18n::tr!("ui-land-mod-information-and-settings-for-the-selected-parcel"));
        floater.min_size = min_size;
        floater.show(ctx, p, open, |ui| {
            if !classic {
                let pics = aurora::Pictures { images, avatars };
                streams_changed = aurora::show(ui, p, self, &view, world, &pics, streams, has_xp);
                return;
            }
            let tabs: Vec<Tab> = Tab::ALL.into_iter().filter(|t| *t != Tab::Experiences || has_xp).collect();
            if !tabs.contains(&self.tab) {
                self.tab = Tab::General;
            }
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(140.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(140.0);
                    egui::ScrollArea::vertical().id_salt("land_navigation").show(ui, |ui| {
                        for tab in tabs {
                            if widgets::navigation(ui, p, tab.icon(), tab.label(), self.tab == tab).clicked() {
                                self.tab = tab;
                            }
                        }
                    });
                });
                widgets::divider(ui, p);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    if let Some(parcel) = &view.parcel {
                        header(ui, p, &view, parcel, region_map);
                    }
                    ui.add_space(6.0);
                    widgets::section_header(ui, p, "adjustments-filled", self.tab.label());
                    let tab = self.tab;
                    streams_changed = pane(ui, p, self, &view, world, images, streams, tab);
                });
            });
        });
        dialogs::show(ctx, p, self, &view, world);
        scripts::show(ctx, p, &mut self.scripts, &view, world);
        if let Some(ids) = self.picker.show(ctx, p, world) {
            match self.pick_for.take() {
                Some(PickFor::Access) => access::add_allowed(world, &ids),
                Some(PickFor::Ban) => {
                    // LLFloaterBanDuration comes next
                    self.dialog = Some(Dialog::BanDuration {
                        ids,
                        temporary: false,
                        hours: 1,
                        replace: false,
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
                has_land_resources: r.caps.contains_key("LandResources") || self.demo,
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

/// One of the nine panes, over the room left below the cursor. True when
/// the saved music streams changed.
#[allow(clippy::too_many_arguments)]
fn pane(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    view: &View,
    world: &mut World,
    images: &HashMap<Uuid, egui::TextureHandle>,
    streams: &mut Vec<String>,
    tab: Tab,
) -> bool {
    let mut streams_changed = false;
    let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
    ui.allocate_rect(body, egui::Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(body.intersect(ui.clip_rect()));
    let ui = &mut child;
    ui.spacing_mut().item_spacing.y = 4.0;
    // the tabs made of rows scroll when the window is small; the
    // others size their lists to the height left
    let scroll = |ui: &mut egui::Ui, body: &mut dyn FnMut(&mut egui::Ui)| {
        egui::ScrollArea::vertical()
            .id_salt("about_land_body")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width() - 6.0);
                body(ui)
            });
    };
    match tab {
        Tab::General => scroll(ui, &mut |ui| general::show(ui, p, s, view, world)),
        Tab::Covenant => covenant::show(ui, p, view, world),
        Tab::Objects => objects::show(ui, p, s, view, world),
        Tab::Options => scroll(ui, &mut |ui| options::show(ui, p, s, view, world, images)),
        Tab::Media => scroll(ui, &mut |ui| media::show(ui, p, s, view, world, images)),
        Tab::Sound => streams_changed = sound::show(ui, p, s, view, world, streams),
        Tab::Access => access::show(ui, p, s, view, world),
        Tab::Experiences => experiences::show(ui, p, s, view, world),
        Tab::Environment => environment::show(ui, p, s, view, world),
    }
    streams_changed
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The region's map in the header stands beside the texts from the top of
/// the name's letters to the bottom of the gauge: its side, and how far
/// under the top of the name's line its letters start.
const HEADER_MAP: f32 = 87.0;
const HEADER_MAP_TOP: f32 = 4.0;

/// The parcel at a glance, over the tabs: the mini-map's image of its
/// region, its name, its area, and a gauge of the objects on the owner's
/// land in the region against what it may hold (the "Region
/// capacity" of the objects tab), which follows the simulator while the
/// floater is open (`Land::tick`). Not in Firestorm's floater.
fn header(ui: &mut egui::Ui, p: &Palette, v: &View, parcel: &ParcelInfo, map: Option<&egui::TextureHandle>) {
    let usage = PrimUsage::of(parcel, v.region.as_ref().map_or(0, |r| r.max_tasks));
    let region = v.region.as_ref().map_or("", |r| r.name.as_str());
    widgets::surface(ui, p, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.add_space(HEADER_MAP_TOP);
                let picture = widgets::picture(ui, p, map, Vec2::splat(HEADER_MAP), "map", "");
                if !region.is_empty() {
                    picture.on_hover_text(region);
                }
            });
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(&parcel.name).size(18.0).strong().color(p.ink)).truncate());
                ui.label(RichText::new(region).size(11.0).color(p.muted));
                ui.horizontal(|ui| {
                    widgets::stat(
                        ui,
                        p,
                        aurora_i18n::tr!("common-area"),
                        &format!("{} m²", aurora_i18n::integer(parcel.area as i64)),
                    );
                    // public land holds nothing and has no capacity
                    if usage.max > 0 || usage.used > 0 {
                        widgets::divider(ui, p);
                        widgets::stat(
                            ui,
                            p,
                            aurora_i18n::tr!("common-objects"),
                            &format!(
                                "{} / {}",
                                aurora_i18n::integer(usage.used as i64),
                                aurora_i18n::integer(usage.max as i64)
                            ),
                        );
                        widgets::divider(ui, p);
                        widgets::stat(
                            ui,
                            p,
                            aurora_i18n::tr!("ui-land-mod-objects-available"),
                            &aurora_i18n::integer(usage.available().max(0) as i64),
                        );
                    }
                });
                if usage.max > 0 || usage.used > 0 {
                    ui.add_space(2.0);
                    let over = -usage.available();
                    // the accent while there is room, the caution color with
                    // a fifth left, the danger color with a tenth left
                    let tint = match usage.room() {
                        PrimRoom::Plenty => None,
                        PrimRoom::Low => Some(p.warn),
                        PrimRoom::Critical => Some(p.danger),
                    };
                    let width = ui.available_width();
                    let gauge = ui
                        .scope(|ui| widgets::progress(ui, p, "land_objects", usage.fraction(), width, "", tint))
                        .response;
                    let percent = aurora_i18n::number_text(&format!("{:.0} %", usage.fraction() * 100.0));
                    gauge.on_hover_text(if over > 0 {
                        aurora_i18n::tr!("ui-land-mod-objects-over-capacity", count = over as i64)
                    } else {
                        aurora_i18n::tr!("ui-land-mod-objects-capacity-used", percent = percent)
                    });
                }
            });
        });
    });
}

/// "No parcel selected." (no_selection_text) while waiting.
fn no_selection(ui: &mut egui::Ui, p: &Palette) {
    ui.add_space(8.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("common-no-parcel-selected"))
            .size(12.0)
            .color(p.muted),
    );
}

/// The label of a row, in the left column.
fn key(ui: &mut egui::Ui, p: &Palette, text: &str) {
    fixed(ui, KEY_W, |ui| {
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).truncate());
    });
}

/// A cell of a fixed width in a row (allocate_ui_with_layout alone
/// shrinks to its content).
fn fixed<R>(ui: &mut egui::Ui, width: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(Vec2::new(width, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        body(ui)
    })
    .inner
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
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("common-view-profile"))
        .clicked()
    {
        super::profile::request_open(ui.ctx(), id);
    }
}

/// A group, by its name or by `label`, that opens its profile (LLSLURL
/// "group ... inspect").
fn group_link(ui: &mut egui::Ui, p: &Palette, id: Uuid, label: &str) {
    if ui
        .add(egui::Label::new(RichText::new(label).size(12.0).color(p.violet_light)).sense(egui::Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("ui-group-profile-view"))
        .clicked()
    {
        super::group_profile::request_open(ui.ctx(), id);
    }
}

/// A flat button, greyed when `enabled` is false.
fn button(ui: &mut egui::Ui, p: &Palette, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| widgets::flat_button(ui, p, label)).inner
}

/// A small Tabler icon button (refresh, +, −, copy).
fn icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, enabled: bool) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(22.0), sense);
    let fill = if enabled && r.hovered() {
        p.raised.gamma_multiply(1.3)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
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
fn check(ui: &mut egui::Ui, p: &Palette, enabled: bool, mut value: bool, label: &str) -> (egui::Response, bool) {
    let label = RichText::new(label).size(12.0);
    let r = ui
        .add_enabled_ui(enabled, |ui| super::widgets::checkbox(ui, p, &mut value, label))
        .inner;
    (r, value)
}

/// "Not available in Aurora yet" for the few Firestorm floaters not ported yet.
fn not_yet() -> &'static str {
    aurora_i18n::tr!("common-not-available-in-aurora-yet")
}

/// Region type in French (LLViewerRegion::getLocalizedSimProductName).
pub(crate) fn product_name(product: &str) -> String {
    match product {
        "Estate / Full Region" => aurora_i18n::tr!("ui-land-mod-estate-full-region"),
        "Estate / Homestead" => aurora_i18n::tr!("extra-estate-homestead"),
        "Mainland / Homestead" => aurora_i18n::tr!("extra-mainland-homestead"),
        "Mainland / Full Region" => aurora_i18n::tr!("ui-land-mod-mainland-full-region"),
        "Linden Homes / Full Region" => aurora_i18n::tr!("ui-land-mod-linden-homes-full-region"),
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

/// Seconds since the epoch in SLT (US Pacific time with its daylight saving
/// rule, like the bars' clock): (year, month, day, weekday 0 = Sunday, hour,
/// minute, second).
pub(crate) fn slt_parts(secs: i64) -> (i64, usize, i64, usize, i64, i64, i64) {
    let t = secs + crate::world::chat_log::format::pacific_offset_hours(secs) * 3600;
    let days = t.div_euclid(86400);
    let tod = t.rem_euclid(86400);
    let (y, m, d) = aurora_net::profile::civil_from_days(days);
    // 1970-01-01 was a Thursday
    let wday = (days + 4).rem_euclid(7) as usize;
    (y, m as usize, i64::from(d), wday, tod / 3600, (tod / 60) % 60, tod % 60)
}

pub(crate) fn weekdays() -> [&'static str; 7] {
    [
        aurora_i18n::tr!("extra-sun"),
        aurora_i18n::tr!("extra-mon"),
        aurora_i18n::tr!("extra-tue"),
        aurora_i18n::tr!("extra-wed"),
        aurora_i18n::tr!("extra-thu"),
        aurora_i18n::tr!("extra-fri"),
        aurora_i18n::tr!("extra-sat"),
    ]
}
pub(crate) fn months() -> [&'static str; 12] {
    [
        aurora_i18n::tr!("extra-jan"),
        aurora_i18n::tr!("ui-land-mod-feb"),
        aurora_i18n::tr!("extra-mar"),
        aurora_i18n::tr!("extra-apr"),
        aurora_i18n::tr!("extra-may"),
        aurora_i18n::tr!("extra-jun"),
        aurora_i18n::tr!("extra-jul"),
        aurora_i18n::tr!("ui-land-mod-aug"),
        aurora_i18n::tr!("extra-sep"),
        aurora_i18n::tr!("extra-oct"),
        aurora_i18n::tr!("extra-nov"),
        aurora_i18n::tr!("ui-land-mod-dec"),
    ]
}

/// A short date in the language's order: "14 Oct" (Japanese "10月14日"),
/// with the year when one is given. `month` counts from 1.
pub(crate) fn day_month(year: Option<i64>, month: usize, day: i64) -> String {
    let month = months()[month.clamp(1, 12) - 1];
    match year {
        Some(year) => aurora_i18n::tr!("date-day-month-year", day = day.to_string(), month = month, year = year.to_string()),
        None => aurora_i18n::tr!("date-day-month", day = day.to_string(), month = month),
    }
}

/// time_stamp_template of the French skin: "Sat 18 Jun 2022 11:19:10".
fn claim_date(secs: i64) -> String {
    let (y, m, d, w, h, mi, s) = slt_parts(secs);
    aurora_i18n::tr!(
        "land-claim-date",
        weekday = weekdays()[w],
        day = d.to_string(),
        month = months()[m - 1],
        year = y.to_string(),
        time = format!("{h:02}:{mi:02}:{s:02}")
    )
}

/// process_covenant_reply with the French LTime strings:
/// "Wed Jun 05 21:18:48 2024".
pub(crate) fn covenant_date(secs: i64) -> String {
    let (y, m, d, w, h, mi, s) = slt_parts(secs);
    aurora_i18n::tr!(
        "land-covenant-date",
        weekday = weekdays()[w],
        day = format!("{d:02}"),
        month = months()[m - 1],
        year = y.to_string(),
        time = format!("{h:02}:{mi:02}:{s:02}")
    )
}

/// LLResMgr::getMonetaryString with the selected thousands separator.
fn money(v: i32) -> String {
    aurora_i18n::integer(i64::from(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_like_the_french_skin() {
        // 2022-06-18 18:19:10 UTC = 11:19:10 PDT, a Saturday
        assert_eq!(
            claim_date(1_655_576_350),
            match aurora_i18n::language() {
                "fr" => "sam. 18 juin 2022 11:19:10",
                "ja" => "2022年6月18日（土）11:19:10",
                "pt-BR" => "sáb 18 jun 2022 11:19:10",
                _ => "Sat 18 Jun 2022 11:19:10",
            }
        );
        // 2024-06-06 04:18:48 UTC = Wed 5 June 21:18:48 PDT
        assert_eq!(
            covenant_date(1_717_647_528),
            match aurora_i18n::language() {
                "fr" => "mer. juin 05 21:18:48 2024",
                "ja" => "2024年6月05日（水）21:18:48",
                "pt-BR" => "qua, 05 de jun de 2024, 21:18:48",
                _ => "Wed Jun 05 21:18:48 2024",
            }
        );
    }

    #[test]
    fn slt_follows_the_2026_daylight_saving_switches() {
        // 2026-03-08 10:00 UTC: 2:00 PST becomes 3:00 PDT, a Sunday
        let spring = 1_772_964_000;
        assert_eq!(slt_parts(spring - 1), (2026, 3, 8, 0, 1, 59, 59));
        assert_eq!(slt_parts(spring), (2026, 3, 8, 0, 3, 0, 0));
        // 2026-11-01 09:00 UTC: 2:00 PDT becomes 1:00 PST
        let fall = 1_793_523_600;
        assert_eq!(slt_parts(fall - 1), (2026, 11, 1, 0, 1, 59, 59));
        assert_eq!(slt_parts(fall), (2026, 11, 1, 0, 1, 0, 0));
        // the weeks the day-of-year approximation got wrong: 2026-03-01
        // 20:00 UTC is still PST, 2026-10-26 19:00 UTC still PDT
        assert_eq!(slt_parts(1_772_395_200).4, 12);
        assert_eq!(slt_parts(1_793_041_200), (2026, 10, 26, 1, 12, 0, 0));
    }

    #[test]
    fn demo_tabs_are_found_by_key_whatever_the_language() {
        let mut ui = LandUi::default();
        for (name, tab) in [
            ("covenant", Tab::Covenant),
            ("Objects", Tab::Objects),
            ("environment", Tab::Environment),
            ("4", Tab::Media),
        ] {
            ui.set_tab(name);
            assert_eq!(ui.tab, tab, "{name}");
        }
        // an unknown name keeps the current tab
        ui.set_tab("nowhere");
        assert_eq!(ui.tab, Tab::Media);
    }

    #[test]
    fn money_groups_thousands() {
        assert_eq!(
            money(88000),
            match aurora_i18n::language() {
                "fr" => "88\u{a0}000",
                "pt-BR" => "88.000",
                _ => "88,000",
            }
        );
        assert_eq!(money(500), "500");
        assert_eq!(
            money(1_234_567),
            match aurora_i18n::language() {
                "fr" => "1\u{a0}234\u{a0}567",
                "pt-BR" => "1.234.567",
                _ => "1,234,567",
            }
        );
    }

    #[test]
    fn product_names_in_french() {
        assert_eq!(
            product_name("Mainland / Full Region"),
            aurora_i18n::tr!("ui-land-mod-mainland-full-region")
        );
        assert_eq!(product_name("Estate / Openspace"), "Estate / Openspace");
    }
}
