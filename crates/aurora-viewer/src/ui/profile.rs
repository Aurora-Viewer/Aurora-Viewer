//! Avatar profile windows, one per avatar, laid out like Firestorm's
//! LLFloaterProfile (`newview/llfloaterprofile.cpp`, `llpanelprofile.cpp`,
//! `llpanelprofilepicks.cpp`, `llpanelprofileclassifieds.cpp`,
//! `floater_profile.xml`, `panel_profile_*.xml`, originally LGPL 2.1):
//! "Second Life" (name, key, status, picture, birth date, account, partner,
//! about, groups, actions), "Feed" (the my.secondlife.com feed in a
//! browser), "Favorites" (picks), "Classifieds", "Real life" and
//! "Notes". Our own profile can be edited (texts, pictures, picks, show in
//! search, full birth date); notes can be written on anyone.
//!
//! This is the classic window (Preferences › Aurora › Profiles); the
//! default one is drawn by `aurora`, on the same state, data and actions.

mod aurora;

pub use aurora::banner_image;

use super::menu;
use super::texture_library::texture_view::{self, ImageView};
use super::texture_picker::TexturePicker;
use super::web_view::WebView;
use super::widgets::{self, Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use crate::world::profiles::{self, Online};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::profile::location_text;
use aurora_net::{AvatarProfile, NetCommand, PickInfo};
use egui::{Color32, RichText, Vec2};
use glam::DVec3;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Ask to open a profile from anywhere in the interface (names and agent
/// links in text); the app reads it once per frame (LLAgentHandler).
pub fn request_open(ctx: &egui::Context, id: Uuid) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("aurora_open_profile"), id));
}

pub fn take_open_request(ctx: &egui::Context) -> Option<Uuid> {
    ctx.data_mut(|d| d.remove_temp::<Uuid>(egui::Id::unique("aurora_open_profile")))
}

pub enum ProfileAction {
    Im(Uuid),
    OfferTeleport(Uuid),
    ToggleBlock(Uuid),
    /// Save fields (`World::save_profile`).
    Save {
        target: Uuid,
        data: Llsd,
    },
    /// Requests and changes of picks, classifieds, parcels, friend rights
    /// (`World::profile_command` keeps the local copy in step).
    Net(NetCommand),
    /// Our display name (the button next to our name).
    DisplayName,
    /// The right to modify our objects, granted or revoked once confirmed
    /// (the rights bar of the Aurora window).
    ModifyRights {
        friend: Uuid,
        rights: i32,
        grant: bool,
    },
    /// Chat of one of our groups (double-click in the group list).
    GroupChat(Uuid),
    /// Picks and classifieds: "Show on map" / "Teleport"
    /// (LLFloaterWorldMap::trackLocation, gAgent.teleportViaLocation).
    ShowOnMap(DVec3),
    Teleport(DVec3),
    /// Inventory dropped on the window: the Share confirmation, then the
    /// give (`give_drop`).
    Give {
        to: Uuid,
        items: Vec<Uuid>,
    },
    /// "Share": the inventory in front, to drag from onto the window
    /// (LLAvatarActions::share, which also opens a conversation to drop
    /// on; the profile takes the drop itself).
    Share(Uuid),
}

/// What the "Feed" browser needs from the app.
pub struct WebContext<'a> {
    pub paths: Option<&'a aurora_media::PluginPaths>,
    pub browser: &'a aurora_media::BrowserSettings,
    pub cookie: Option<&'a aurora_media::Cookie>,
    /// Grid's web profile address ("https://my.secondlife.com/"), None
    /// when the grid has none.
    pub profile_base: Option<&'a str>,
}

const TAB_SL: usize = 0;
const TAB_FEED: usize = 1;
const TAB_PICKS: usize = 2;
const TAB_CLASSIFIEDS: usize = 3;
const TAB_RL: usize = 4;
const TAB_NOTES: usize = 5;

/// Left column of the labels ("Name:", "Key:", "Info:"...), right-aligned.
const LABEL_W: f32 = 60.0;

/// A text being edited against the value the server has.
#[derive(Default)]
struct Edit {
    text: String,
    base: String,
}

impl Edit {
    fn dirty(&self) -> bool {
        self.text != self.base
    }

    /// Follow the server value while the user has not changed it.
    fn sync(&mut self, server: &str) {
        if !self.dirty() && self.base != server {
            self.base = server.to_owned();
            self.text = server.to_owned();
        }
    }
}

/// The pick shown on our own profile, as edited (LLPanelProfilePick).
#[derive(Clone, Default, PartialEq)]
struct PickEdit {
    /// Nil for a pick being created.
    id: Uuid,
    name: String,
    desc: String,
    snapshot: Uuid,
    pos: DVec3,
    /// Location text while it is not saved (location_notice).
    location: Option<String>,
}

/// One of our classifieds as edited (LLPanelProfileClassified's edit panel).
#[derive(Clone, Default, PartialEq)]
struct ClassifiedEdit {
    /// Nil for a classified being created.
    id: Uuid,
    name: String,
    desc: String,
    snapshot: Uuid,
    category: u32,
    mature: bool,
    auto_renew: bool,
    pos: DVec3,
    parcel: Uuid,
    sim_name: String,
    parcel_name: String,
    /// Location text while it is not saved (location_notice).
    location: Option<String>,
    /// What it cost (an edit sends it back unchanged).
    price: i32,
}

/// A new classified waiting for the price to pay (LLPublishClassifiedFloater).
#[derive(Clone, Default, PartialEq)]
struct Publish {
    price: String,
    /// MinClassifiedPrice / ClassifiedInsufficientFunds, shown in the strip.
    error: Option<String>,
}

struct Window {
    id: Uuid,
    open: bool,
    tab: usize,
    about: Edit,
    fl_about: Edit,
    notes: Edit,
    /// "Info" / "Real life" shown with their links (the eye toggle) on our profile.
    about_preview: bool,
    fl_preview: bool,
    /// Closing with unsaved changes (ProfileUnsavedChanges).
    confirm_close: bool,
    /// Bring to front on the next frame.
    raise: bool,
    web: WebView,
    pick_sel: Option<Uuid>,
    /// Our pick being edited and its saved state.
    pick_edit: Option<(PickEdit, PickEdit)>,
    classified_sel: Option<Uuid>,
    classifieds_asked: bool,
    /// Pick / classified / parcel details already asked for.
    asked: HashSet<Uuid>,
    /// "Delete..." waiting for a yes.
    confirm_delete: bool,
    /// Rights we grant this friend, being edited (LLFloaterProfilePermissions).
    rights_edit: Option<i32>,
    /// Height under "Info" in the "Second Life" tab (previous frame).
    bottom_h: f32,
    /// Our own profile edited in place (the Aurora window).
    editing: bool,
    /// The selected profile picture shown at full resolution.
    photo: Option<PhotoPreview>,
    /// Our classified being edited and its saved state (the Aurora window).
    classified_edit: Option<(ClassifiedEdit, ClassifiedEdit)>,
    /// The price asked before a new classified is published.
    publish: Option<Publish>,
    /// Classifieds whose details, statistics and profile click were asked
    /// (LLPanelProfileClassified::onOpen).
    classifieds_opened: HashSet<Uuid>,
    /// The notes were just saved ("Saved" until the next change).
    notes_saved: bool,
    /// Demo captures: our first classified in its form (false) or a new
    /// one at its price (true), once the tab shows.
    demo_classified: Option<bool>,
    /// Open a photo once the demo profile reply arrives.
    demo_photo: Option<(PictureSlot, bool)>,
}

impl Window {
    fn new(id: Uuid) -> Window {
        Window {
            id,
            open: true,
            tab: TAB_SL,
            about: Edit::default(),
            fl_about: Edit::default(),
            notes: Edit::default(),
            about_preview: false,
            fl_preview: false,
            confirm_close: false,
            raise: true,
            web: WebView::default(),
            pick_sel: None,
            pick_edit: None,
            classified_sel: None,
            classifieds_asked: false,
            asked: HashSet::new(),
            confirm_delete: false,
            rights_edit: None,
            bottom_h: 200.0,
            editing: false,
            photo: None,
            classified_edit: None,
            publish: None,
            classifieds_opened: HashSet::new(),
            notes_saved: false,
            demo_classified: None,
            demo_photo: None,
        }
    }

    fn ask(&mut self, id: Uuid, cmd: NetCommand, actions: &mut Vec<ProfileAction>) {
        if !id.is_nil() && self.asked.insert(id) {
            actions.push(ProfileAction::Net(cmd));
        }
    }
}

/// Which picture the texture picker changes.
#[derive(Clone, Copy)]
enum PictureSlot {
    SecondLife,
    FirstLife,
    /// The snapshot of our pick or classified being edited (in the window
    /// of this id).
    Pick(Uuid),
    Classified(Uuid),
}

struct PhotoPreview {
    slot: PictureSlot,
    asset: Uuid,
    view: ImageView,
}

impl Window {
    fn open_photo(&mut self, slot: PictureSlot, asset: Uuid) {
        if !asset.is_nil() {
            self.photo = Some(PhotoPreview {
                slot,
                asset,
                view: ImageView::default(),
            });
        }
    }
}

#[derive(Default)]
pub struct ProfileUi {
    windows: Vec<Window>,
    /// Profile pictures wanted (by avatar id, like the conversations).
    pub wanted_pics: HashSet<Uuid>,
    /// Other images wanted by asset id (pictures, snapshots).
    pub wanted_images: HashSet<Uuid>,
    picker: TexturePicker,
    picker_slot: Option<PictureSlot>,
    /// Banners of the Aurora window by profile picture (`banner_image`),
    /// made by the viewer when it decodes the picture.
    pub banners: HashMap<Uuid, egui::TextureHandle>,
}

impl ProfileUi {
    /// Decode the selected photo on the scene workers, just like an inventory
    /// texture preview. UI thumbnails alone cannot supply native pixels.
    pub(crate) fn prepare_photos(&mut self, ctx: &egui::Context, scene: &mut crate::scene::Scene, world: &World) {
        for window in &mut self.windows {
            if let Some((slot, actual)) = window.demo_photo
                && let Some(profile) = world.profiles.get(&window.id)
            {
                let asset = if matches!(slot, PictureSlot::FirstLife) {
                    profile.fl_image
                } else {
                    profile.sl_image
                };
                window.open_photo(slot, asset);
                if let Some(photo) = &mut window.photo {
                    photo.view.display.actual_size = actual;
                }
                window.demo_photo = None;
            }
            let Some(photo) = window.photo.as_mut().filter(|_| window.open) else {
                continue;
            };
            let asset = world.profiles.get(&window.id).map_or(Uuid::nil(), |profile| match photo.slot {
                PictureSlot::FirstLife => profile.fl_image,
                _ => profile.sl_image,
            });
            if asset.is_nil() {
                window.photo = None;
                continue;
            }
            if asset != photo.asset {
                photo.asset = asset;
                let display = photo.view.display;
                photo.view = ImageView::default();
                photo.view.display = display;
            }
            self.wanted_images.insert(asset);
            let pixels = if photo.view.display.actual_size {
                scene.textures.note_usage_fixed(&asset, 16384.0);
                16384.0
            } else {
                ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("profile_photo_preview", window.id))))
                    .map_or(520.0, |rect| rect.size().max_elem())
                    * ctx.pixels_per_point()
            };
            scene.textures.note_usage(&asset, pixels);
            photo.view.prepare(ctx, asset, scene);
            if let Some(size) = scene.textures.full_size(&asset) {
                photo.view.dimensions = Some(size);
            }
        }
    }

    /// The browser processes of the profiles' web pages that run.
    pub fn web_process_ids(&self) -> Vec<u32> {
        self.windows.iter().filter_map(|w| w.web.process_id()).collect()
    }

    /// LLAvatarActions::showProfile: one window per avatar, raised if open;
    /// each opening reloads the data (LLPanelProfile::updateData).
    pub fn open(&mut self, world: &mut World, id: Uuid) {
        if id.is_nil() || !world.rlva_can_name(id) {
            return;
        }
        world.profiles.refresh(id);
        world.social.want_name(id);
        match self.windows.iter_mut().find(|w| w.id == id) {
            Some(w) => {
                w.open = true;
                w.raise = true;
                w.asked.clear();
                w.classifieds_asked = false;
            }
            None => self.windows.push(Window::new(id)),
        }
    }

    /// Demo captures of the Aurora window (AURORA_DEMO_PROFILE): `edit` puts
    /// our profile in edit mode, `compact` gives the window the width of
    /// the compact layout, a number shows that tab.
    pub fn demo(&mut self, ctx: &egui::Context, id: Uuid, option: &str) {
        let Some(w) = self.windows.iter_mut().find(|w| w.id == id) else {
            return;
        };
        // several at once: "edit:3", "compact:2"
        for option in option.split(':') {
            match option {
                "edit" => w.editing = true,
                "form" => w.demo_classified = Some(false),
                "publish" => w.demo_classified = Some(true),
                "photo" => w.demo_photo = Some((PictureSlot::SecondLife, false)),
                "first-photo" => {
                    w.tab = TAB_RL;
                    w.demo_photo = Some((PictureSlot::FirstLife, false));
                }
                "actual" => {
                    if let Some((_, actual)) = &mut w.demo_photo {
                        *actual = true;
                    }
                }
                "compact" => {
                    let size = Vec2::new(380.0, 640.0);
                    let pos = ctx.content_rect().center() - size * 0.5;
                    widgets::place_floater(ctx, &aurora::window_id(id), pos, size);
                }
                tab => w.tab = tab.parse::<usize>().unwrap_or(TAB_SL).min(TAB_NOTES),
            }
        }
    }

    /// Where the first open profile window stands, either style
    /// (`AURORA_DEMO_GIVE=profile`).
    pub fn demo_window_rect(&self, ctx: &egui::Context) -> Option<egui::Rect> {
        let w = self.windows.iter().find(|w| w.open)?;
        [aurora::window_id(w.id), format!("profile-{}", w.id)]
            .iter()
            .find_map(|id| ctx.memory(|m| m.area_rect(egui::Id::unique(id.as_str()))))
    }

    /// The "Feed" page that has the keyboard, if any.
    pub fn web_has_focus(&self) -> bool {
        self.windows
            .iter()
            .find(|w| w.open && w.tab == TAB_FEED && w.web.focused)
            .is_some_and(|w| w.web.running())
    }

    /// The "Feed" page that has the keyboard, if any.
    pub fn focused_web(&mut self) -> Option<&mut aurora_media::MediaPlugin> {
        self.windows
            .iter_mut()
            .find(|w| w.open && w.tab == TAB_FEED && w.web.focused)
            .and_then(|w| w.web.plugin_mut())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        emoji: &mut super::emoji::Emoji,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        images: &HashMap<Uuid, egui::TextureHandle>,
        world: &mut World,
        want_names: &mut HashSet<Uuid>,
        web: &WebContext,
        classic: bool,
        friend_rights: bool,
    ) -> Vec<ProfileAction> {
        let mut actions = Vec::new();
        let screen = ctx.content_rect();
        let mut pick_picture: Option<(PictureSlot, Uuid)> = None;
        for (i, w) in self.windows.iter_mut().enumerate() {
            if !world.rlva_can_name(w.id) {
                continue;
            }
            if !classic {
                let profile = world.profiles.get(&w.id);
                if let Some(pr) = profile {
                    w.about.sync(&pr.sl_about);
                    w.fl_about.sync(&pr.fl_about);
                    if let Some(n) = &pr.notes {
                        w.notes.sync(n);
                    }
                }
                if !w.open {
                    continue;
                }
                if std::mem::take(&mut w.raise) {
                    let win_id = aurora::window_id(w.id);
                    widgets::expand_floater(ctx, &win_id);
                }
                world.social.avatar_names.want(&w.id);
                let mut env = aurora::Env {
                    p,
                    emoji,
                    pics,
                    images,
                    banners: &self.banners,
                    world,
                    want_names,
                    web,
                    wanted_pics: &mut self.wanted_pics,
                    wanted_images: &mut self.wanted_images,
                    actions: &mut actions,
                    pick_picture: &mut pick_picture,
                    friend_rights,
                };
                let open = aurora::window(ctx, &mut env, w, i);
                photo_window(ctx, p, world, pics, images, w);
                close_or_confirm(w, open);
                continue;
            }
            let profile = world.profiles.get(&w.id);
            if let Some(pr) = profile {
                w.about.sync(&pr.sl_about);
                w.fl_about.sync(&pr.fl_about);
                if let Some(n) = &pr.notes {
                    w.notes.sync(n);
                }
            }
            if !w.open {
                continue;
            }
            let win_id = format!("profile-{}", w.id);
            if std::mem::take(&mut w.raise) {
                widgets::expand_floater(ctx, &win_id);
            }
            let name = world.social.name_of(&w.id);
            world.social.avatar_names.want(&w.id);
            // floater_profile.xml: 485 x 510 (480 x 510 at least), centered
            let size = Vec2::new(540.0, 650.0);
            let pos = screen.center() - size * 0.5 + Vec2::splat(24.0 * (i % 6) as f32);
            let mut open = true;
            let mut floater = Floater::new(&win_id, name.clone(), pos, size);
            floater.min_size = Vec2::new(500.0, 580.0);
            let own = w.id == world.agent_id;
            let shown = floater.show(ctx, p, &mut open, |ui| {
                if w.confirm_close {
                    unsaved_bar(ui, p, w, &mut actions);
                    ui.add_space(4.0);
                }
                let before = w.tab;
                widgets::tabs(
                    ui,
                    p,
                    &mut w.tab,
                    &[
                        (aurora_i18n::tr!("ui-profile-second-life"), true),
                        (aurora_i18n::tr!("ui-profile-feed"), web.profile_base.is_some()),
                        (aurora_i18n::tr!("common-favorites"), true),
                        (aurora_i18n::tr!("ui-profile-classifieds"), true),
                        (aurora_i18n::tr!("ui-profile-real-life"), true),
                        (aurora_i18n::tr!("common-notes"), true),
                    ],
                );
                if before == TAB_FEED && w.tab != TAB_FEED {
                    w.web.focused = false;
                }
                ui.add_space(6.0);
                let c = TabCtx {
                    p,
                    world,
                    images,
                    own: w.id == world.agent_id,
                };
                // every tab draws in the same rectangle, the window's: its
                // content never changes the window's size
                let body = egui::Rect::from_min_max(ui.cursor().min, ui.max_rect().max);
                ui.allocate_rect(body, egui::Sense::hover());
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(body)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                child.set_clip_rect(body.intersect(ui.clip_rect()));
                let ui = &mut child;
                match w.tab {
                    TAB_FEED => feed_tab(ui, &c, w, web),
                    TAB_PICKS => picks_tab(ui, &c, emoji, w, want_names, &mut self.wanted_images, &mut actions),
                    TAB_CLASSIFIEDS => classifieds_tab(ui, &c, emoji, w, want_names, &mut self.wanted_images, &mut actions),
                    TAB_RL => first_life_tab(
                        ui,
                        &c,
                        emoji,
                        w,
                        profile,
                        want_names,
                        &mut self.wanted_images,
                        &mut actions,
                        &mut pick_picture,
                    ),
                    TAB_NOTES => notes_tab(ui, &c, w, profile, &mut actions),
                    _ => second_life_tab(
                        ui,
                        &c,
                        emoji,
                        pics,
                        w,
                        &name,
                        want_names,
                        &mut self.wanted_pics,
                        &mut actions,
                        &mut pick_picture,
                    ),
                }
            });
            if shown.is_some() && !own {
                give_drop(ctx, p, &win_id, &name, w.id, &mut actions);
            }
            if let Some(rights) = w.rights_edit {
                rights_dialog(ctx, p, w, &name, rights, &mut actions);
            }
            photo_window(ctx, p, world, pics, images, w);
            close_or_confirm(w, open);
        }
        self.windows.retain(|w| w.open);
        if let Some((slot, current)) = pick_picture {
            let title = match slot {
                PictureSlot::SecondLife => aurora_i18n::tr!("ui-profile-profile-picture"),
                PictureSlot::FirstLife => aurora_i18n::tr!("ui-profile-rl-picture"),
                PictureSlot::Pick(_) | PictureSlot::Classified(_) => aurora_i18n::tr!("ui-profile-aurora-snapshot"),
            };
            self.picker.open(title, current);
            self.picker_slot = Some(slot);
        }
        if let Some(asset) = self.picker.show_live(ctx, p, world, images) {
            // a snapshot goes into the edit, saved with it
            match self.picker_slot {
                Some(PictureSlot::Pick(id)) => {
                    if let Some((e, _)) = self.windows.iter_mut().find(|w| w.id == id).and_then(|w| w.pick_edit.as_mut()) {
                        e.snapshot = asset;
                    }
                }
                Some(PictureSlot::Classified(id)) => {
                    if let Some((e, _)) = self
                        .windows
                        .iter_mut()
                        .find(|w| w.id == id)
                        .and_then(|w| w.classified_edit.as_mut())
                    {
                        e.snapshot = asset;
                    }
                }
                slot => {
                    let key = match slot {
                        Some(PictureSlot::FirstLife) => "fl_image_id",
                        _ => "sl_image_id",
                    };
                    let mut data = Llsd::new_map();
                    data.insert(key, asset);
                    actions.push(ProfileAction::Save {
                        target: world.agent_id,
                        data,
                    });
                }
            }
        }
        self.wanted_images.extend(self.picker.wanted_images.drain());
        actions
    }
}

/// Shorter than this (a minimized window), the window fades without the hint.
const GIVE_HINT_MIN_H: f32 = 120.0;

/// Inventory dropped on another resident's profile is given to them, after
/// the Share confirmation (LLPanelProfileSecondLife::handleDragAndDrop →
/// LLToolDragAndDrop::handleGiveDragAndDrop). The Legacy Viewer takes it on
/// the Second Life tab only, outside the about text and the group list, and
/// only changes the cursor; here the whole window takes it, on every tab,
/// and fades behind the conversations' hint from the start of the drag
/// (#991). Drawn over the window `win_id` once it is shown, both styles.
fn give_drop(ctx: &egui::Context, p: &Palette, win_id: &str, name: &str, to: Uuid, actions: &mut Vec<ProfileAction>) {
    let window = egui::Id::unique(win_id);
    let Some(rect) = ctx.memory(|m| m.area_rect(window)) else {
        return;
    };
    let layer = egui::LayerId::new(egui::Order::Middle, window);
    let id = window.with("give_drop");
    let fade = super::give_drop::fade(ctx, id, true);
    let target = super::inventory::drag::give_target_on(ctx, layer, rect);
    if fade > 0.0 {
        let painter = ctx.layer_painter(layer).with_clip_rect(rect);
        let radius = ctx.global_style().visuals.window_corner_radius;
        painter.rect_filled(rect, radius, p.panel.gamma_multiply(super::give_drop::DIM * fade));
        if target.is_some() {
            painter.rect_stroke(rect.shrink(1.0), radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
        }
        if rect.height() >= GIVE_HINT_MIN_H {
            super::give_drop::hint(&painter, p, id, rect, target.is_some(), name, fade);
        }
    }
    if let Some((items, true)) = target
        && !items.is_empty()
    {
        actions.push(ProfileAction::Give { to, items });
    }
}

/// LLFloaterProfileTexture (indra/newview/llfloaterprofiletexture.cpp), with
/// the inventory preview's dimensions, aspect ratios and native pixel display
/// added at the human's request for both profile pictures.
fn photo_window(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    images: &HashMap<Uuid, egui::TextureHandle>,
    window: &mut Window,
) {
    let Some(photo) = &mut window.photo else { return };
    let name = aurora::display_name(world, &window.id);
    let caption = match photo.slot {
        PictureSlot::FirstLife => aurora_i18n::tr!("ui-profile-rl-picture"),
        _ => aurora_i18n::tr!("ui-profile-profile-picture"),
    };
    let id = format!("profile-photo-{}", window.id);
    let size = Vec2::new(460.0, 520.0);
    let pos = ctx.content_rect().center() - size * 0.5;
    let mut open = true;
    let mut floater = Floater::new(&id, format!("{caption} · {name}"), pos, size);
    floater.min_size = Vec2::new(300.0, 260.0);
    floater.show(ctx, p, &mut open, |ui| {
        ui.horizontal_wrapped(|ui| texture_view::display_mode(ui, p, &mut photo.view.display));
        let image = photo.view.image.as_ref().or_else(|| images.get(&photo.asset)).or_else(|| {
            matches!(photo.slot, PictureSlot::SecondLife)
                .then(|| pics.get(&window.id))
                .flatten()
        });
        if let Some(rect) = texture_view::show_image(ui, p, image, photo.view.dimensions, &mut photo.view.display, None) {
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("profile_photo_preview", window.id)), rect));
        }
    });
    if !open {
        window.photo = None;
    }
}

/// What every tab reads.
struct TabCtx<'a> {
    p: &'a Palette,
    world: &'a World,
    images: &'a HashMap<Uuid, egui::TextureHandle>,
    own: bool,
}

/// "Save changes?" shown when closing with edits.
fn unsaved_bar(ui: &mut egui::Ui, p: &Palette, w: &mut Window, actions: &mut Vec<ProfileAction>) {
    egui::Frame::new()
        .fill(p.warn.gamma_multiply(0.18))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-there-are-unsaved-changes"))
                        .size(12.0)
                        .color(p.ink),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        w.confirm_close = false;
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-ignore")).clicked() {
                        w.open = false;
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-save")).clicked() {
                        save_all(w, actions);
                        w.open = false;
                    }
                });
            });
        });
}

/// Some text or pick of the window is edited and not saved.
fn has_unsaved(w: &Window) -> bool {
    let pick_dirty = w.pick_edit.as_ref().is_some_and(|(e, base)| e != base);
    let classified_dirty = w.classified_edit.as_ref().is_some_and(|(e, base)| e != base);
    w.about.dirty() || w.fl_about.dirty() || w.notes.dirty() || pick_dirty || classified_dirty
}

/// The close button was pressed when `open` is false:
/// LLFloaterProfile::onClickCloseBtn asks before losing edits.
fn close_or_confirm(w: &mut Window, open: bool) {
    if !open {
        if has_unsaved(w) {
            w.confirm_close = true;
        } else {
            w.open = false;
        }
    }
    if !w.open {
        w.web.close();
    }
}

fn save_all(w: &mut Window, actions: &mut Vec<ProfileAction>) {
    for (edit, key) in [
        (&mut w.about, "sl_about_text"),
        (&mut w.fl_about, "fl_about_text"),
        (&mut w.notes, "notes"),
    ] {
        if edit.dirty() {
            edit.base = edit.text.clone();
            let mut data = Llsd::new_map();
            data.insert(key, edit.text.clone());
            actions.push(ProfileAction::Save { target: w.id, data });
        }
    }
    if let Some((e, base)) = &mut w.pick_edit
        && e != base
    {
        // a new pick gets its id once: saved again, it is the same pick
        let pick = pick_of(e);
        e.id = pick.id;
        e.location = None;
        *base = e.clone();
        w.pick_sel = Some(pick.id);
        actions.push(ProfileAction::Net(NetCommand::PickUpdate(Box::new(pick))));
    }
    if let Some((e, base)) = &mut w.classified_edit
        && e != base
    {
        if e.id.is_nil() {
            // a new classified is paid: its price is asked first
            // (LLPanelProfileClassified::onSaveClick)
            w.publish.get_or_insert_with(Publish::default);
        } else {
            e.location = None;
            *base = e.clone();
            actions.push(ProfileAction::Net(NetCommand::ClassifiedUpdate(Box::new(classified_of(e)))));
        }
    }
}

/// The classified sent for an edit (LLPanelProfileClassified::sendUpdate): a
/// new one gets its id here.
fn classified_of(e: &ClassifiedEdit) -> aurora_net::ClassifiedInfo {
    aurora_net::ClassifiedInfo {
        id: if e.id.is_nil() { Uuid::new_v4() } else { e.id },
        category: e.category,
        name: e.name.clone(),
        desc: e.desc.clone(),
        parcel: e.parcel,
        snapshot: e.snapshot,
        sim_name: e.sim_name.clone(),
        parcel_name: e.parcel_name.clone(),
        pos_global: e.pos,
        flags: aurora_net::profile::classified_flags::request(e.auto_renew, e.mature),
        price: e.price,
        ..Default::default()
    }
}

fn pick_of(e: &PickEdit) -> PickInfo {
    PickInfo {
        id: if e.id.is_nil() { Uuid::new_v4() } else { e.id },
        name: e.name.clone(),
        desc: e.desc.clone(),
        snapshot: e.snapshot,
        pos_global: e.pos,
        enabled: true,
        ..Default::default()
    }
}

/// Right-aligned label of the left column.
fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.allocate_ui_with_layout(Vec2::new(LABEL_W, 18.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(RichText::new(text).size(12.0).color(p.muted));
    });
}

/// Inset box (view_border bevel="in") of a fixed size.
fn inset<R>(ui: &mut egui::Ui, p: &Palette, width: f32, height: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.field.gamma_multiply(0.82))
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_width((width - 10.0).max(10.0));
            ui.set_min_height((height - 6.0).max(8.0));
            ui.set_max_height((height - 6.0).max(8.0));
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), body).inner
        })
        .inner
}

fn loading(ui: &mut egui::Ui, p: &Palette) {
    ui.label(RichText::new(aurora_i18n::tr!("ui-profile-loading")).size(12.0).color(p.muted));
}

/// Text with emoji and clickable links, line by line.
fn rich(ui: &mut egui::Ui, p: &Palette, emoji: &mut super::emoji::Emoji, world: &World, want_names: &mut HashSet<Uuid>, text: &str) {
    for line in text.lines() {
        if line.is_empty() {
            ui.add_space(6.0);
        } else {
            super::chat::chat_text(ui, p, emoji, world, want_names, line, 12.5, p.ink, false);
        }
    }
}

/// Bordered text: read with links, or edited (own profile).
#[allow(clippy::too_many_arguments)]
fn text_area(
    ui: &mut egui::Ui,
    p: &Palette,
    emoji: &mut super::emoji::Emoji,
    world: &World,
    want_names: &mut HashSet<Uuid>,
    edit: &mut Edit,
    editable: bool,
    size: Vec2,
    max: usize,
    id: egui::Id,
) {
    if editable {
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_width(size.x)
            .max_height(size.y)
            .min_scrolled_height(size.y)
            .show(ui, |ui| {
                // beside the scroll bar's strip once the text is longer
                let width = ui.available_width();
                ui.add_sized(
                    [width, size.y],
                    egui::TextEdit::multiline(&mut edit.text)
                        .desired_width(width)
                        .char_limit(max)
                        .font(egui::FontId::proportional(12.5)),
                );
            });
        return;
    }
    inset(ui, p, size.x, size.y, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(id)
            .max_height(size.y - 8.0)
            .auto_shrink([false, true])
            .show(ui, |ui| rich(ui, p, emoji, world, want_names, &edit.text));
    });
}

/// "Save" / "Cancel" at the bottom right (save_description_changes).
fn save_buttons(ui: &mut egui::Ui, p: &Palette, edit: &mut Edit) -> bool {
    let mut save = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_enabled_ui(edit.dirty(), |ui| {
            if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                edit.text = edit.base.clone();
            }
            save = flat_button(ui, p, aurora_i18n::tr!("common-save")).clicked();
        });
    });
    if save {
        edit.base = edit.text.clone();
    }
    save
}

/// A Tabler icon `size` px wide that can be clicked.
fn icon(ui: &mut egui::Ui, p: &Palette, name: &str, size: f32, tint: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if let Some(t) = super::icons::global(name) {
        ui.painter().image(
            t.id(),
            rect.shrink(size * 0.12),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp
}

/// The eye toggle of the editable texts (btn_preview).
fn preview_toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool) {
    let icon = if *on { "eye-filled" } else { "eye-off" };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if let Some(t) = super::icons::global(icon) {
        ui.painter().image(
            t.id(),
            rect.shrink(2.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if *on { p.violet_light } else { p.muted },
        );
    }
    if resp
        .on_hover_text(if *on {
            aurora_i18n::tr!("ui-profile-edit-text")
        } else {
            aurora_i18n::tr!("ui-profile-preview-with-links")
        })
        .clicked()
    {
        *on = !*on;
    }
}

/// Friend rights icon (can_see_online, can_see_on_map, can_edit_objects).
fn rights_icon(ui: &mut egui::Ui, p: &Palette, icon: &str, granted: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if let Some(t) = super::icons::global(icon) {
        ui.painter().image(
            t.id(),
            rect.shrink(3.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if granted { p.violet_light } else { p.muted_dim },
        );
    }
    if !granted {
        ui.painter().line_segment(
            [rect.left_bottom() + Vec2::new(4.0, -4.0), rect.right_top() + Vec2::new(-4.0, 4.0)],
            egui::Stroke::new(1.5, p.danger),
        );
    }
    let state = if granted {
        aurora_i18n::tr!("ui-profile-allowed")
    } else {
        aurora_i18n::tr!("ui-profile-not-allowed")
    };
    resp.on_hover_text(aurora_i18n::tr!(
        "ui-profile-click-change-permissions",
        tip = tip.to_string(),
        state = state.to_string()
    ))
    .clicked()
}

#[allow(clippy::too_many_arguments)]
fn second_life_tab(
    ui: &mut egui::Ui,
    c: &TabCtx,
    emoji: &mut super::emoji::Emoji,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    w: &mut Window,
    name: &str,
    want_names: &mut HashSet<Uuid>,
    wanted_pics: &mut HashSet<Uuid>,
    actions: &mut Vec<ProfileAction>,
    pick_picture: &mut Option<(PictureSlot, Uuid)>,
) {
    let (p, world, own, id) = (c.p, c.world, c.own, w.id);
    let profile = world.profiles.get(&id);
    let friend = world.social.friends.iter().find(|f| f.id == id);
    if profile.is_some_and(|pr| !pr.sl_image.is_nil()) {
        wanted_pics.insert(id);
    }
    let full_w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 4.0;
    // name row: name field (copy menu inside), display name (own), friend rights
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("common-name-2"));
        let rights_w = if friend.is_some() { 3.0 * 26.0 } else { 0.0 };
        let field_w = (ui.available_width() - rights_w - if own { 36.0 } else { 8.0 }).max(80.0);
        inset(ui, p, field_w, 22.0, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(name)
                            .size(12.5)
                            .color(crate::ui::colors::resident_name(world, &id, p.ink)),
                    )
                    .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    copy_menu(ui, p, world, id);
                    crate::ui::staff::after_name(ui, p, crate::world::staff::mark(world, &id), 9.5);
                });
            });
        });
        if own
            && icon(ui, p, "pencil-filled", 20.0, p.muted)
                .on_hover_text(aurora_i18n::tr!("common-set-display-name"))
                .clicked()
        {
            actions.push(ProfileAction::DisplayName);
        }
        if let Some(f) = friend {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut clicked = false;
            clicked |= rights_icon(
                ui,
                p,
                "eye-filled",
                f.rights_given & 1 != 0,
                aurora_i18n::tr!("ui-profile-see-when-i-am-online"),
            );
            clicked |= rights_icon(
                ui,
                p,
                "map-pin-filled",
                f.rights_given & 2 != 0,
                aurora_i18n::tr!("ui-profile-find-me-on-map"),
            );
            clicked |= rights_icon(
                ui,
                p,
                "pencil-filled",
                f.rights_given & 4 != 0,
                aurora_i18n::tr!("ui-profile-modify-delete-or-take-my-objects"),
            );
            if clicked {
                w.rights_edit = Some(f.rights_given);
            }
        }
    });
    let appearance = world.contact_sets.appearance(&id);
    if !appearance.icons.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for badge in &appearance.icons {
                let color = crate::ui::colors::set_color(badge.color);
                if let Some(texture) = super::icons::global(crate::world::contact_sets::appearance::ICONS[badge.icon]) {
                    ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::splat(16.0)).tint(color));
                }
                ui.label(RichText::new(&badge.set).size(11.5).color(color));
            }
        });
    }
    // key row: the UUID, then the online status of the others
    ui.horizontal(|ui| {
        label(ui, p, aurora_i18n::tr!("ui-profile-key"));
        let key_w = (ui.available_width() - if own { 8.0 } else { 100.0 }).max(80.0);
        inset(ui, p, key_w, 20.0, |ui| {
            ui.add(egui::Label::new(RichText::new(id.to_string()).size(11.5).monospace().color(p.ink)).selectable(true));
        });
        if !own {
            let status = profiles::online_status(profile, own, friend.map(|f| (f.online, f.rights_has & 1 != 0)));
            let (text, col) = match status {
                Online::Yes => (aurora_i18n::tr!("ui-profile-connected"), p.success),
                Online::No => (aurora_i18n::tr!("ui-profile-disconnected"), p.muted),
                Online::Unknown => (aurora_i18n::tr!("common-unknown"), p.warn),
            };
            ui.add_sized([80.0, 18.0], egui::Label::new(RichText::new(text).size(12.0).color(col)));
        }
    });
    ui.add_space(2.0);
    // picture row: options (own), picture 162, birth / account / partner
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(52.0, 162.0), egui::Layout::top_down(egui::Align::Max), |ui| {
            if own && let Some(pr) = profile {
                // menu_fs_profile_image_actions.xml
                widgets::icon_menu(ui, p, "settings-filled", aurora_i18n::tr!("ui-profile-profile-picture"), |ui| {
                    let soon = aurora_i18n::tr!("ui-profile-coming-soon");
                    menu::item_tip(
                        ui,
                        p,
                        "file-upload-filled",
                        aurora_i18n::tr!("ui-profile-upload-picture"),
                        false,
                        soon,
                    );
                    if menu::item(ui, p, "photo-filled", aurora_i18n::tr!("ui-profile-change-picture")) {
                        *pick_picture = Some((PictureSlot::SecondLife, pr.sl_image));
                    }
                    if menu::item_if(
                        ui,
                        p,
                        "trash-filled",
                        aurora_i18n::tr!("ui-profile-delete-photo"),
                        !pr.sl_image.is_nil(),
                    ) {
                        actions.push(ProfileAction::Save {
                            target: id,
                            data: llsd_map! { "sl_image_id" => Uuid::nil() },
                        });
                    }
                });
            }
        });
        let r = widgets::picture(
            ui,
            p,
            pics.get(&id),
            Vec2::splat(162.0),
            "user-filled",
            aurora_i18n::tr!("ui-profile-profile-picture"),
        );
        if own
            && let Some(pr) = profile
            && r.clone()
                .on_hover_text(aurora_i18n::tr!("ui-profile-click-change-picture"))
                .clicked()
        {
            *pick_picture = Some((PictureSlot::SecondLife, pr.sl_image));
        } else if !own
            && let Some(pr) = profile.filter(|pr| !pr.sl_image.is_nil())
            && r.on_hover_text(aurora_i18n::tr!("ui-profile-aurora-big-photo")).clicked()
        {
            w.open_photo(PictureSlot::SecondLife, pr.sl_image);
        }
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let col_w = ui.available_width() - 4.0;
            ui.label(RichText::new(aurora_i18n::tr!("ui-profile-birth-date")).size(12.0).color(p.muted));
            inset(ui, p, col_w, 38.0, |ui| {
                match profile.and_then(|pr| profiles::birth_text(pr, own, now)) {
                    Some(t) => {
                        ui.label(RichText::new(t).size(12.0).color(p.ink));
                    }
                    None => loading(ui, p),
                }
            });
            ui.add_space(3.0);
            ui.label(RichText::new(aurora_i18n::tr!("ui-profile-account")).size(12.0).color(p.muted));
            inset(ui, p, col_w, 52.0, |ui| match profile {
                Some(pr) => {
                    ui.horizontal_top(|ui| {
                        if let Some(b) = profiles::badge(pr) {
                            icon(ui, p, "star-filled", 18.0, p.amber).on_hover_text(b);
                        }
                        ui.label(RichText::new(profiles::account_text(pr)).size(12.0).color(p.ink));
                    });
                }
                None => loading(ui, p),
            });
            ui.add_space(3.0);
            ui.label(RichText::new(aurora_i18n::tr!("ui-profile-partner")).size(12.0).color(p.muted));
            inset(ui, p, col_w, 22.0, |ui| match profile.map(|pr| pr.partner) {
                Some(partner) if !partner.is_nil() => {
                    world.social.avatar_names.want(&partner);
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(world.social.name_of(&partner))
                                .size(12.0)
                                .color(super::colors::text(super::colors::get().chat_slurl)),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(aurora_i18n::tr!("common-view-profile"))
                        .clicked()
                    {
                        request_open(ui.ctx(), partner);
                    }
                }
                Some(_) => {
                    ui.label(RichText::new(aurora_i18n::tr!("common-none")).size(12.0).color(p.ink));
                }
                None => loading(ui, p),
            });
        });
    });
    // "Info" (103 high at least, follows="all") takes the space left by the
    // groups (80), "Give" and the buttons, measured on the previous frame
    let groups_h = 80.0;
    let field_w = full_w - LABEL_W - 20.0;
    ui.add_space(6.0);
    let info_h = (ui.available_height() - w.bottom_h - 8.0).max(103.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            label(ui, p, aurora_i18n::tr!("ui-profile-info"));
            if own {
                ui.horizontal(|ui| {
                    ui.add_space(LABEL_W - 22.0);
                    preview_toggle(ui, p, &mut w.about_preview);
                });
            }
        });
        if profile.is_some() {
            text_area(
                ui,
                p,
                emoji,
                world,
                want_names,
                &mut w.about,
                own && !w.about_preview,
                Vec2::new(field_w, info_h),
                65_000,
                egui::Id::unique(("sl_about", id)),
            );
        } else {
            inset(ui, p, field_w, info_h, |ui| loading(ui, p));
        }
    });
    let bottom = ui.scope(|ui| {
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                label(ui, p, aurora_i18n::tr!("ui-profile-groups"));
                if !own {
                    ui.horizontal(|ui| {
                        ui.add_space(LABEL_W - 22.0);
                        widgets::icon_button(
                            ui,
                            p,
                            "users-plus",
                            aurora_i18n::tr!("ui-profile-invite-to-group-coming-soon"),
                            false,
                        );
                    });
                }
            });
            inset(ui, p, field_w, groups_h, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("groups", id))
                    .max_height(groups_h - 8.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| groups_list(ui, p, world, profile, own, actions));
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            label(ui, p, aurora_i18n::tr!("ui-profile-give"));
            inset(ui, p, field_w, 20.0, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-profile-drop-the-object-here"))
                            .size(11.5)
                            .color(p.muted_dim),
                    );
                });
            });
        });
        ui.add_space(4.0);
        if own {
            own_bottom(ui, p, w, profile, now, actions);
        } else {
            action_buttons(ui, p, world, w, friend.is_some(), actions);
        }
    });
    w.bottom_h = bottom.response.rect.height();
}

/// group_list: names as links (double-click: chat of one of our groups,
/// there is no group profile in Aurora yet).
fn groups_list(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &World,
    profile: Option<&AvatarProfile>,
    own: bool,
    actions: &mut Vec<ProfileAction>,
) {
    let Some(pr) = profile else {
        loading(ui, p);
        return;
    };
    if pr.groups.is_empty() {
        // our own list does not say "None"
        if !own {
            ui.label(RichText::new(aurora_i18n::tr!("common-none")).size(12.0).color(p.ink));
        }
        return;
    }
    let mut groups: Vec<_> = pr.groups.iter().collect();
    groups.sort_by_key(|g| g.name.to_lowercase());
    ui.spacing_mut().item_spacing.y = 1.0;
    for g in groups {
        let member = world.groups.is_member(&g.id);
        let r = ui.add(egui::Label::new(RichText::new(&g.name).size(12.5).color(p.indigo_light)).sense(egui::Sense::click()));
        let tip = if member {
            aurora_i18n::tr!("ui-profile-double-click-group-chat")
        } else {
            aurora_i18n::tr!("ui-profile-group-profile-coming-soon")
        };
        if r.on_hover_text(tip).double_clicked() && member {
            actions.push(ProfileAction::GroupChat(g.id));
        }
    }
}

/// Own profile bottom: "Show in search", "Show full birth
/// date", Save / Cancel of "Info".
fn own_bottom(ui: &mut egui::Ui, p: &Palette, w: &mut Window, profile: Option<&AvatarProfile>, now: f64, actions: &mut Vec<ProfileAction>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if let Some(pr) = profile {
            let mut publish = pr.allow_publish();
            if widgets::checkbox(ui, p, &mut publish, aurora_i18n::tr!("common-show-in-search")).changed() {
                actions.push(ProfileAction::Save {
                    target: w.id,
                    data: llsd_map! { "allow_publish" => publish },
                });
            }
            // only offered by servers that support it, to accounts older than a year
            if let (Some(hide), Some(born)) = (pr.hide_age, pr.born)
                && now - born > 365.0 * 86_400.0
            {
                let mut full = !hide;
                if widgets::checkbox(ui, p, &mut full, aurora_i18n::tr!("ui-profile-show-full-birth-date")).changed() {
                    actions.push(ProfileAction::Save {
                        target: w.id,
                        data: llsd_map! { "hide_age" => !full },
                    });
                }
            }
        }
    });
    ui.horizontal(|ui| {
        if save_buttons(ui, p, &mut w.about) {
            actions.push(ProfileAction::Save {
                target: w.id,
                data: llsd_map! { "sl_about_text" => w.about.text.clone() },
            });
        }
    });
}

/// buttonstack of panel_profile_secondlife.xml: three columns of two rows,
/// then the overflow menu.
fn action_buttons(ui: &mut egui::Ui, p: &Palette, world: &World, w: &Window, is_friend: bool, actions: &mut Vec<ProfileAction>) {
    let id = w.id;
    let blocked = world.is_avatar_blocked(&id);
    let col_w = ((ui.available_width() - 24.0) / 3.0).max(80.0);
    let size = Vec2::new(col_w, 20.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 2.0);
        ui.vertical(|ui| {
            if ui
                .add_enabled(
                    world.can_locate_avatar(id),
                    egui::Button::new(aurora_i18n::tr!("ui-profile-show-on-map-2")).min_size(size),
                )
                .clicked()
            {
                super::context::request(ui.ctx(), super::context::CtxAction::ShowOnMap(id));
            }
            if ui.add(egui::Button::new(aurora_i18n::tr!("common-pay")).min_size(size)).clicked() {
                crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::PayResident(id));
            }
        });
        ui.vertical(|ui| {
            if ui
                .add(egui::Button::new(aurora_i18n::tr!("ui-profile-offer-teleport")).min_size(size))
                .clicked()
            {
                actions.push(ProfileAction::OfferTeleport(id));
            }
            if ui
                .add(egui::Button::new(aurora_i18n::tr!("ui-profile-instant-message")).min_size(size))
                .clicked()
            {
                actions.push(ProfileAction::Im(id));
            }
        });
        ui.vertical(|ui| {
            let friend_label = if is_friend {
                aurora_i18n::tr!("ui-profile-remove-friend")
            } else {
                aurora_i18n::tr!("ui-profile-add-friend")
            };
            ui.add_enabled(false, egui::Button::new(friend_label).min_size(size))
                .on_disabled_hover_text(aurora_i18n::tr!("ui-profile-coming-soon"));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let bw = Vec2::new(col_w - 34.0, 20.0);
                if ui
                    .add(
                        egui::Button::new(if blocked {
                            aurora_i18n::tr!("common-unblock")
                        } else {
                            aurora_i18n::tr!("common-block")
                        })
                        .min_size(bw),
                    )
                    .clicked()
                {
                    actions.push(ProfileAction::ToggleBlock(id));
                }
                overflow_menu(ui, p, world, id, blocked, actions);
            });
        });
    });
}

/// menu_fs_profile_overflow.xml.
fn overflow_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid, blocked: bool, actions: &mut Vec<ProfileAction>) {
    widgets::icon_menu(ui, p, "menu-2-filled", aurora_i18n::tr!("ui-profile-more"), |ui| {
        if menu::item_if(ui, p, "map", aurora_i18n::tr!("common-map"), world.can_locate_avatar(id)) {
            super::context::request(ui.ctx(), super::context::CtxAction::ShowOnMap(id));
        }
        if menu::item(ui, p, "coin-filled", aurora_i18n::tr!("common-pay")) {
            crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::PayResident(id));
        }
        if menu::item(ui, p, "share", aurora_i18n::tr!("common-share")) {
            actions.push(ProfileAction::Share(id));
        }
        // LLAvatarActions::startCall (canCall: voice enabled)
        if menu::item_if(
            ui,
            p,
            "phone-filled",
            aurora_i18n::tr!("ui-profile-call"),
            world.calls.voice_enabled,
        ) {
            crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::Call(id));
        }
        let block = if blocked {
            aurora_i18n::tr!("common-unblock")
        } else {
            aurora_i18n::tr!("common-block")
        };
        if menu::item(ui, p, "ban", block) {
            actions.push(ProfileAction::ToggleBlock(id));
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-name")) {
            ui.ctx().copy_text(world.social.name_of(&id));
        }
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("ui-profile-copy-uri")) {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
        }
        if menu::item(ui, p, "key-filled", aurora_i18n::tr!("ui-profile-copy-agent-uuid-key")) {
            ui.ctx().copy_text(id.to_string());
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "flag-filled", aurora_i18n::tr!("common-report")) {
            super::context::request(
                ui.ctx(),
                super::context::CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Avatar(id)),
            );
        }
    });
}

/// menu_fs_profile_name_field.xml (copy menu inside the name field).
fn copy_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid) {
    let names = &world.social.avatar_names;
    let display = names.get(&id).map(|n| n.display(&names.options));
    let legacy = world.legacy_name(&id);
    widgets::icon_menu(ui, p, "link-filled", aurora_i18n::tr!("common-copy-2"), |ui| {
        let display_name = aurora_i18n::tr!("ui-profile-copy-display-name");
        if menu::item_if(ui, p, "id-filled", display_name, display.is_some()) {
            ui.ctx().copy_text(display.clone().unwrap_or_default());
        }
        if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-name"), legacy.is_some()) {
            ui.ctx().copy_text(legacy.clone().unwrap_or_default());
        }
        if menu::item(ui, p, "key-filled", aurora_i18n::tr!("ui-profile-copy-agent-uuid-key")) {
            ui.ctx().copy_text(id.to_string());
        }
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("ui-profile-copy-uri")) {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
        }
    });
}

/// LLFloaterProfilePermissions: the rights we grant a friend.
fn rights_dialog(ctx: &egui::Context, p: &Palette, w: &mut Window, name: &str, rights: i32, actions: &mut Vec<ProfileAction>) {
    let mut r = rights;
    let mut open = true;
    let mut done = false;
    let screen = ctx.content_rect();
    let win_id = format!("profile-rights-{}", w.id);
    Floater::new(
        &win_id,
        aurora_i18n::tr!("common-rights"),
        screen.center() - Vec2::new(150.0, 70.0),
        Vec2::new(300.0, 140.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-profile-allow-to", name = name.to_string()))
                .size(12.5)
                .color(p.ink),
        );
        for (bit, label) in [
            (1, aurora_i18n::tr!("ui-profile-see-when-i-am-online")),
            (2, aurora_i18n::tr!("ui-profile-find-me-on-map")),
            (4, aurora_i18n::tr!("ui-profile-modify-delete-or-take-my-objects")),
        ] {
            let mut on = r & bit != 0;
            if widgets::checkbox(ui, p, &mut on, label).changed() {
                r = if on { r | bit } else { r & !bit };
            }
        }
        ui.add_space(4.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                done = true;
            }
            if flat_button(ui, p, aurora_i18n::tr!("common-ok")).clicked() {
                if r != rights {
                    actions.push(ProfileAction::Net(NetCommand::GrantUserRights { friend: w.id, rights: r }));
                }
                done = true;
            }
        });
    });
    w.rights_edit = if done || !open { None } else { Some(r) };
}

/// LLPanelProfileWeb: [WEB_PROFILE_URL]<username>/?feed_only=true.
fn feed_tab(ui: &mut egui::Ui, c: &TabCtx, w: &mut Window, web: &WebContext) {
    let Some(base) = web.profile_base else {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-profile-no-web-profile-on-this-grid"))
                .size(12.0)
                .color(c.p.muted),
        );
        return;
    };
    let Some(username) = web_username(c.world, &w.id) else {
        loading(ui, c.p);
        return;
    };
    w.web.navigate(&format!("{base}{username}/?feed_only=true").to_lowercase());
    let size = Vec2::new(ui.available_width() - 4.0, (ui.available_height() - 34.0).max(100.0));
    w.web.show(ui, c.p, web.paths, web.browser, web.cookie, size);
    ui.vertical_centered(|ui| {
        let text = match w.web.load_secs {
            Some(s) => aurora_i18n::tr!("ui-profile-load-time-seconds", s = aurora_i18n::number_text(&format!("{:.2}", s))),
            None => aurora_i18n::tr!("common-loading-2").to_owned(),
        };
        ui.label(RichText::new(text).size(11.5).color(c.p.muted));
    });
}

/// LLAvatarName::getAccountName ("first.last"), else
/// LLCacheName::buildUsername from the legacy name.
fn web_username(world: &World, id: &Uuid) -> Option<String> {
    let names = &world.social.avatar_names;
    if let Some(u) = names.get(id).map(|n| n.username.clone()).filter(|u| !u.is_empty()) {
        return Some(u.replace(' ', "."));
    }
    world.legacy_name(id).map(|n| {
        let mut it = n.split_whitespace();
        let first = it.next().unwrap_or("");
        match it.next() {
            Some(last) if !last.eq_ignore_ascii_case("resident") => format!("{first}.{last}"),
            _ => first.to_owned(),
        }
    })
}

/// Left list of a picks / classifieds tab (tab_position="left", 150 wide).
fn side_list(ui: &mut egui::Ui, p: &Palette, items: &[(Uuid, String)], selected: &mut Option<Uuid>, height: f32, salt: &str) {
    egui::Frame::new().fill(p.field).show(ui, |ui| {
        ui.set_width(150.0);
        ui.set_min_height(height);
        egui::ScrollArea::vertical().id_salt(salt).max_height(height).show(ui, |ui| {
            // the 150 of the column, less the scroll bar's strip when it shows
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                for (id, name) in items {
                    let sel = *selected == Some(*id);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
                    let fill = if sel {
                        p.violet.gamma_multiply(0.6)
                    } else if resp.hovered() {
                        p.raised
                    } else {
                        p.field
                    };
                    ui.painter().rect_filled(rect, 0.0, fill);
                    let label = if name.is_empty() {
                        aurora_i18n::tr!("common-unnamed")
                    } else {
                        name.as_str()
                    };
                    // one line, cut with "…" (tab use_ellipses)
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        label.to_owned(),
                        egui::FontId::proportional(12.0),
                        if sel { p.ink } else { p.muted },
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width(138.0);
                    let galley = ui.painter().layout_job(job);
                    ui.painter()
                        .galley(rect.left_center() + Vec2::new(6.0, -galley.size().y / 2.0), galley, p.ink);
                    if resp.on_hover_text(label).clicked() {
                        *selected = Some(*id);
                    }
                }
            });
        });
    });
}

/// LLAgentBenefitsMgr "picks_limit" (login account_level_benefits).
fn picks_limit(world: &World) -> usize {
    world
        .login
        .as_ref()
        .map(|l| l.raw["account_level_benefits"]["picks_limit"].as_i32())
        .filter(|n| *n > 0)
        .unwrap_or(10) as usize
}

#[allow(clippy::too_many_arguments)]
fn picks_tab(
    ui: &mut egui::Ui,
    c: &TabCtx,
    emoji: &mut super::emoji::Emoji,
    w: &mut Window,
    want_names: &mut HashSet<Uuid>,
    wanted_images: &mut HashSet<Uuid>,
    actions: &mut Vec<ProfileAction>,
) {
    let (p, world, own) = (c.p, c.world, c.own);
    let Some(pr) = world.profiles.get(&w.id) else {
        loading(ui, p);
        return;
    };
    let mut items = pr.picks.clone();
    if let Some((e, _)) = &w.pick_edit
        && e.id.is_nil()
    {
        items.push((Uuid::nil(), e.name.clone()));
    }
    if own {
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-profile-let-other-residents-know-your-favorite-places-in-second"
                ))
                .size(12.0)
                .color(p.muted),
            );
        });
        ui.horizontal(|ui| {
            let creating = w.pick_edit.as_ref().is_some_and(|(e, _)| e.id.is_nil());
            let can_add = pr.picks.len() < picks_limit(world) && !creating && !world.rlva.has("showloc");
            if ui
                .add_enabled(can_add, egui::Button::new(aurora_i18n::tr!("ui-profile-new")))
                .clicked()
            {
                w.pick_edit = Some((new_pick(world), PickEdit::default()));
                w.pick_sel = Some(Uuid::nil());
            }
            if ui
                .add_enabled(w.pick_sel.is_some(), egui::Button::new(aurora_i18n::tr!("common-delete-2")))
                .clicked()
            {
                w.confirm_delete = true;
            }
            if w.confirm_delete {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-delete-this-favorite"))
                        .size(12.0)
                        .color(p.warn),
                );
                if flat_button(ui, p, aurora_i18n::tr!("common-yes")).clicked() {
                    if let Some(id) = w.pick_sel.filter(|id| !id.is_nil()) {
                        actions.push(ProfileAction::Net(NetCommand::PickDelete(id)));
                    }
                    w.pick_edit = None;
                    w.pick_sel = None;
                    w.confirm_delete = false;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-no")).clicked() {
                    w.confirm_delete = false;
                }
            }
        });
        ui.add_space(4.0);
    }
    if items.is_empty() {
        ui.add_space(120.0);
        ui.vertical_centered(|ui| {
            let t = if own {
                aurora_i18n::tr!("ui-profile-you-have-not-created-any-picks-click-new-to")
            } else {
                aurora_i18n::tr!("ui-profile-this-user-has-no-picks")
            };
            ui.label(RichText::new(t).size(12.0).color(p.muted));
        });
        return;
    }
    if w.pick_sel.is_none_or(|s| !items.iter().any(|(id, _)| *id == s)) {
        w.pick_sel = items.first().map(|(id, _)| *id);
    }
    let height = ui.available_height() - 12.0;
    ui.horizontal_top(|ui| {
        let before = w.pick_sel;
        side_list(ui, p, &items, &mut w.pick_sel, height, "picks");
        if before != w.pick_sel {
            // another pick: an unsaved new one is dropped
            w.pick_edit = None;
            w.confirm_delete = false;
        }
        ui.add_space(6.0);
        ui.vertical(|ui| {
            if let Some(sel) = w.pick_sel {
                egui::ScrollArea::vertical()
                    .id_salt(("pick", sel))
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| pick_detail(ui, c, emoji, w, sel, want_names, wanted_images, actions));
            }
        });
    });
}

/// A new pick at our position (LLPanelProfilePick::setAvatarId, null pick).
fn new_pick(world: &World) -> PickEdit {
    if world.rlva.has("showloc") {
        return PickEdit::default();
    }
    let region = world.region_name();
    let (parcel_name, desc, snapshot) = match &world.parcel {
        Some(pa) => (pa.name.clone(), pa.desc.clone(), pa.snapshot_id),
        None => (String::new(), String::new(), Uuid::nil()),
    };
    let pos = world.agent_global().unwrap_or(DVec3::ZERO);
    let name = if parcel_name.is_empty() {
        region.clone()
    } else {
        parcel_name.clone()
    };
    PickEdit {
        id: Uuid::nil(),
        name,
        desc,
        snapshot,
        pos,
        location: Some(location_text(
            &[aurora_i18n::tr!("ui-profile-updated-after-saving"), &parcel_name, &region],
            pos,
        )),
    }
}

/// panel_profile_pick.xml.
#[allow(clippy::too_many_arguments)]
fn pick_detail(
    ui: &mut egui::Ui,
    c: &TabCtx,
    emoji: &mut super::emoji::Emoji,
    w: &mut Window,
    sel: Uuid,
    want_names: &mut HashSet<Uuid>,
    wanted_images: &mut HashSet<Uuid>,
    actions: &mut Vec<ProfileAction>,
) {
    let (p, world, own) = (c.p, c.world, c.own);
    let info = world.profiles.picks.get(&sel);
    if !sel.is_nil() && info.is_none() {
        w.ask(sel, NetCommand::PickInfoRequest { creator: w.id, pick: sel }, actions);
    }
    // our own picks are edited in a copy of the server's
    if own && let Some(i) = info {
        let server = PickEdit {
            id: i.id,
            name: i.name.clone(),
            desc: i.desc.clone(),
            snapshot: i.snapshot,
            pos: i.pos_global,
            location: None,
        };
        let follow = match &w.pick_edit {
            Some((e, base)) => e.id != i.id || (e == base && *base != server),
            None => true,
        };
        if follow {
            w.pick_edit = Some((server.clone(), server));
        }
    }
    let width = (ui.available_width() - 8.0).min(300.0);
    ui.set_max_width(width);
    let (snapshot, pos, name, desc) = match (&w.pick_edit, info) {
        (Some((e, _)), _) if own => (e.snapshot, e.pos, e.name.clone(), e.desc.clone()),
        (_, Some(i)) => (i.snapshot, i.pos_global, i.name.clone(), i.desc.clone()),
        _ => {
            loading(ui, p);
            return;
        }
    };
    if !snapshot.is_nil() {
        wanted_images.insert(snapshot);
    }
    widgets::picture(
        ui,
        p,
        c.images.get(&snapshot),
        Vec2::new(width, width * 179.0 / 290.0),
        "user-filled",
        "",
    );
    ui.label(RichText::new(aurora_i18n::tr!("common-name-2")).size(12.0).strong().color(p.ink));
    if let (true, Some((e, _))) = (own, w.pick_edit.as_mut()) {
        ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(width).char_limit(63));
    } else {
        inset(ui, p, width, 22.0, |ui| {
            ui.label(RichText::new(&name).size(12.5).color(p.ink));
        });
    }
    ui.label(
        RichText::new(aurora_i18n::tr!("common-description"))
            .size(12.0)
            .strong()
            .color(p.ink),
    );
    // a fixed height: sizes taken from the space left would grow the window
    let desc_h = 80.0;
    if let (true, Some((e, _))) = (own, w.pick_edit.as_mut()) {
        egui::ScrollArea::vertical()
            .id_salt(("pick_desc", sel))
            .max_width(width)
            .max_height(desc_h)
            .show(ui, |ui| {
                // beside the scroll bar's strip once the text is longer
                let width = ui.available_width();
                ui.add_sized(
                    [width, desc_h],
                    egui::TextEdit::multiline(&mut e.desc).desired_width(width).char_limit(1023),
                );
            });
    } else {
        inset(ui, p, width, desc_h, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("pick_desc", sel))
                .max_height(desc_h - 8.0)
                .auto_shrink([false, true])
                .show(ui, |ui| rich(ui, p, emoji, world, want_names, &desc));
        });
    }
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-profile-location-2"))
            .size(12.0)
            .strong()
            .color(p.ink),
    );
    let location = match (&w.pick_edit, info) {
        (Some((e, _)), _) if own && e.location.is_some() => e.location.clone().unwrap_or_default(),
        (_, Some(i)) if !i.parcel.is_nil() => match world.profiles.parcels.get(&i.parcel) {
            Some(pa) => location_text(&["", &pa.name, &pa.sim_name], pos),
            None => {
                w.ask(i.parcel, NetCommand::ParcelInfoRequest(i.parcel), actions);
                aurora_i18n::tr!("ui-profile-loading-2").to_owned()
            }
        },
        (_, Some(i)) => location_text(&["", &i.sim_name], pos),
        _ => String::new(),
    };
    inset(ui, p, width, 22.0, |ui| {
        ui.add(egui::Label::new(RichText::new(world.rlva_filter_text(&location)).size(12.0).color(p.muted)).truncate());
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let has_pos = pos != DVec3::ZERO;
        if ui
            .add_enabled(has_pos, egui::Button::new(aurora_i18n::tr!("ui-profile-show-on-map")))
            .clicked()
        {
            actions.push(ProfileAction::ShowOnMap(pos));
        }
        if ui
            .add_enabled(has_pos, egui::Button::new(aurora_i18n::tr!("common-teleport-2")))
            .clicked()
        {
            actions.push(ProfileAction::Teleport(pos));
        }
        if own
            && ui
                .add_enabled(
                    !world.rlva.has("showloc"),
                    egui::Button::new(aurora_i18n::tr!("ui-profile-set-location")),
                )
                .clicked()
        {
            // onClickSetLocation: our position, parcel and region now
            let here = new_pick(world);
            if let Some((e, _)) = w.pick_edit.as_mut() {
                e.pos = here.pos;
                e.location = here.location;
            }
        }
    });
    let mut created = None;
    if own && let Some((e, base)) = w.pick_edit.as_mut() {
        widgets::divider(ui, p);
        ui.horizontal(|ui| {
            if e.id.is_nil() {
                if ui.button(aurora_i18n::tr!("ui-profile-create-place")).clicked() {
                    created = Some(pick_of(e));
                }
            } else {
                let dirty = e != base;
                if ui
                    .add_enabled(dirty, egui::Button::new(aurora_i18n::tr!("ui-profile-save-location")))
                    .clicked()
                {
                    *base = e.clone();
                    actions.push(ProfileAction::Net(NetCommand::PickUpdate(Box::new(pick_of(e)))));
                }
                if ui
                    .add_enabled(dirty, egui::Button::new(aurora_i18n::tr!("common-cancel")))
                    .clicked()
                {
                    *e = base.clone();
                }
            }
        });
    }
    if let Some(pick) = created {
        w.pick_sel = Some(pick.id);
        w.pick_edit = None;
        actions.push(ProfileAction::Net(NetCommand::PickUpdate(Box::new(pick))));
    }
}

#[allow(clippy::too_many_arguments)]
fn classifieds_tab(
    ui: &mut egui::Ui,
    c: &TabCtx,
    emoji: &mut super::emoji::Emoji,
    w: &mut Window,
    want_names: &mut HashSet<Uuid>,
    wanted_images: &mut HashSet<Uuid>,
    actions: &mut Vec<ProfileAction>,
) {
    let (p, world, own) = (c.p, c.world, c.own);
    if !w.classifieds_asked {
        w.classifieds_asked = true;
        actions.push(ProfileAction::Net(NetCommand::ClassifiedsRequest(w.id)));
    }
    if own {
        ui.horizontal(|ui| {
            ui.add_enabled(false, egui::Button::new(aurora_i18n::tr!("ui-profile-new-2")))
                .on_disabled_hover_text(aurora_i18n::tr!("ui-profile-publish-a-classified-listing-paid-coming-soon"));
            if ui
                .add_enabled(w.classified_sel.is_some(), egui::Button::new(aurora_i18n::tr!("common-delete-2")))
                .clicked()
            {
                w.confirm_delete = true;
            }
            if w.confirm_delete {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-delete-this-classified"))
                        .size(12.0)
                        .color(p.warn),
                );
                if flat_button(ui, p, aurora_i18n::tr!("common-yes")).clicked() {
                    if let Some(id) = w.classified_sel.take() {
                        actions.push(ProfileAction::Net(NetCommand::ClassifiedDelete(id)));
                    }
                    w.confirm_delete = false;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-no")).clicked() {
                    w.confirm_delete = false;
                }
            }
        });
        ui.add_space(4.0);
    }
    let items = world.profiles.classified_lists.get(&w.id).cloned().unwrap_or_default();
    if items.is_empty() {
        ui.add_space(160.0);
        ui.vertical_centered(|ui| {
            let t = if own {
                aurora_i18n::tr!("ui-profile-you-have-not-created-any-classifieds-click-new-to")
            } else {
                aurora_i18n::tr!("ui-profile-this-user-has-no-classified-listings")
            };
            ui.label(RichText::new(t).size(12.0).color(p.muted));
        });
        return;
    }
    if w.classified_sel.is_none_or(|s| !items.iter().any(|(id, _)| *id == s)) {
        w.classified_sel = items.first().map(|(id, _)| *id);
    }
    let height = ui.available_height() - 12.0;
    ui.horizontal_top(|ui| {
        side_list(ui, p, &items, &mut w.classified_sel, height, "classifieds");
        ui.add_space(6.0);
        ui.vertical(|ui| {
            let Some(sel) = w.classified_sel else {
                return;
            };
            let Some(info) = world.profiles.classifieds.get(&sel) else {
                w.ask(sel, NetCommand::ClassifiedInfoRequest(sel), actions);
                loading(ui, p);
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt(("classified", sel))
                .max_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| classified_detail(ui, c, emoji, info, want_names, wanted_images, actions));
        });
    });
}

/// panel_profile_classified.xml (read only).
fn classified_detail(
    ui: &mut egui::Ui,
    c: &TabCtx,
    emoji: &mut super::emoji::Emoji,
    info: &aurora_net::ClassifiedInfo,
    want_names: &mut HashSet<Uuid>,
    wanted_images: &mut HashSet<Uuid>,
    actions: &mut Vec<ProfileAction>,
) {
    let (p, world) = (c.p, c.world);
    let width = (ui.available_width() - 12.0).min(280.0);
    if !info.snapshot.is_nil() {
        wanted_images.insert(info.snapshot);
    }
    widgets::picture(
        ui,
        p,
        c.images.get(&info.snapshot),
        Vec2::new(width, width * 161.0 / 260.0),
        "user-filled",
        "",
    );
    ui.label(RichText::new(&info.name).size(14.0).strong().color(p.ink));
    let row = |ui: &mut egui::Ui, label: &str, value: &str| {
        ui.horizontal_top(|ui| {
            ui.add_sized([110.0, 16.0], egui::Label::new(RichText::new(label).size(12.0).color(p.muted)));
            ui.add(egui::Label::new(RichText::new(value).size(12.0).color(p.ink)).wrap());
        });
    };
    row(
        ui,
        aurora_i18n::tr!("ui-profile-location"),
        &world.rlva_filter_text(&location_text(&[&info.parcel_name, &info.sim_name], info.pos_global)),
    );
    row(
        ui,
        aurora_i18n::tr!("ui-profile-content-type"),
        profiles::classified_maturity(info.flags),
    );
    row(ui, aurora_i18n::tr!("common-category"), &world.classified_category(info.category));
    row(
        ui,
        aurora_i18n::tr!("ui-profile-creation-date"),
        &profiles::slt_date(info.creation_date),
    );
    row(ui, aurora_i18n::tr!("ui-profile-listing-cost"), &format!("L${}", info.price));
    if c.own {
        let renew = info.flags & aurora_net::profile::classified_flags::AUTO_RENEW != 0;
        row(
            ui,
            aurora_i18n::tr!("ui-profile-auto-renew"),
            if renew {
                aurora_i18n::tr!("ui-profile-enabled")
            } else {
                aurora_i18n::tr!("common-disabled")
            },
        );
    }
    ui.label(RichText::new(aurora_i18n::tr!("common-description")).size(12.0).color(p.muted));
    rich(ui, p, emoji, world, want_names, &info.desc);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button(aurora_i18n::tr!("common-teleport-2")).clicked() {
            actions.push(ProfileAction::Teleport(info.pos_global));
        }
        if ui.button(aurora_i18n::tr!("common-map")).clicked() {
            actions.push(ProfileAction::ShowOnMap(info.pos_global));
        }
        if c.own {
            ui.add_enabled(false, egui::Button::new(aurora_i18n::tr!("common-edit")))
                .on_disabled_hover_text(aurora_i18n::tr!("ui-profile-coming-soon"));
        }
    });
}

/// panel_profile_firstlife.xml.
#[allow(clippy::too_many_arguments)]
fn first_life_tab(
    ui: &mut egui::Ui,
    c: &TabCtx,
    emoji: &mut super::emoji::Emoji,
    w: &mut Window,
    profile: Option<&AvatarProfile>,
    want_names: &mut HashSet<Uuid>,
    wanted_images: &mut HashSet<Uuid>,
    actions: &mut Vec<ProfileAction>,
    pick_picture: &mut Option<(PictureSlot, Uuid)>,
) {
    let p = c.p;
    let Some(pr) = profile else {
        loading(ui, p);
        return;
    };
    if !pr.fl_image.is_nil() {
        wanted_images.insert(pr.fl_image);
    }
    ui.horizontal_top(|ui| {
        let r = widgets::picture(
            ui,
            p,
            c.images.get(&pr.fl_image),
            Vec2::splat(200.0),
            "user-filled",
            aurora_i18n::tr!("ui-profile-rl-picture"),
        );
        if c.own {
            if r.on_hover_text(aurora_i18n::tr!("ui-profile-click-change-image")).clicked() {
                *pick_picture = Some((PictureSlot::FirstLife, pr.fl_image));
            }
        } else if !pr.fl_image.is_nil() && r.on_hover_text(aurora_i18n::tr!("ui-profile-aurora-big-photo")).clicked() {
            w.open_photo(PictureSlot::FirstLife, pr.fl_image);
        }
        if c.own {
            ui.add_space(70.0);
            ui.vertical(|ui| {
                ui.add_space(45.0);
                let size = Vec2::new(120.0, 20.0);
                ui.add_enabled(false, egui::Button::new(aurora_i18n::tr!("ui-profile-load-image")).min_size(size))
                    .on_disabled_hover_text(aurora_i18n::tr!("ui-profile-coming-soon"));
                if ui
                    .add(egui::Button::new(aurora_i18n::tr!("ui-profile-change-image")).min_size(size))
                    .clicked()
                {
                    *pick_picture = Some((PictureSlot::FirstLife, pr.fl_image));
                }
                if ui
                    .add_enabled(
                        !pr.fl_image.is_nil(),
                        egui::Button::new(aurora_i18n::tr!("ui-profile-remove-image")).min_size(size),
                    )
                    .clicked()
                {
                    actions.push(ProfileAction::Save {
                        target: w.id,
                        data: llsd_map! { "fl_image_id" => Uuid::nil() },
                    });
                }
                ui.horizontal(|ui| {
                    ui.add_space(50.0);
                    preview_toggle(ui, p, &mut w.fl_preview);
                });
            });
        }
    });
    ui.add_space(10.0);
    let h = (ui.available_height() - if c.own { 44.0 } else { 16.0 }).max(60.0);
    let width = ui.available_width() - 8.0;
    text_area(
        ui,
        p,
        emoji,
        c.world,
        want_names,
        &mut w.fl_about,
        c.own && !w.fl_preview,
        Vec2::new(width, h),
        65_000,
        egui::Id::unique(("fl_about", w.id)),
    );
    if c.own {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if save_buttons(ui, p, &mut w.fl_about) {
                actions.push(ProfileAction::Save {
                    target: w.id,
                    data: llsd_map! { "fl_about_text" => w.fl_about.text.clone() },
                });
            }
        });
    }
}

/// panel_profile_notes.xml: private notes, stored by the server.
fn notes_tab(ui: &mut egui::Ui, c: &TabCtx, w: &mut Window, profile: Option<&AvatarProfile>, actions: &mut Vec<ProfileAction>) {
    let p = c.p;
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-profile-take-notes-about-this-person-nobody-else-can-see"))
            .size(12.0)
            .strong()
            .color(p.ink),
    );
    ui.add_space(4.0);
    if profile.is_none() {
        loading(ui, p);
        return;
    }
    let size = Vec2::new(ui.available_width() - 8.0, (ui.available_height() - 44.0).max(60.0));
    egui::ScrollArea::vertical()
        .id_salt(("notes", w.id))
        .max_width(size.x)
        .max_height(size.y)
        .min_scrolled_height(size.y)
        .show(ui, |ui| {
            // beside the scroll bar's strip once the notes are longer
            let width = ui.available_width();
            ui.add_sized(
                [width, size.y],
                egui::TextEdit::multiline(&mut w.notes.text).desired_width(width).char_limit(65_530),
            );
        });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if save_buttons(ui, p, &mut w.notes) {
            actions.push(ProfileAction::Save {
                target: w.id,
                data: llsd_map! { "notes" => w.notes.text.clone() },
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inventory released on another resident's profile gives it to them, in
    /// both styles, wherever on the window; never on our own, nor beside it
    /// (#991).
    #[test]
    fn inventory_dropped_on_a_residents_profile_window_gives_it() {
        use crate::ui::inventory::InvDrag;
        let p = crate::theme::Theme::default().palette();
        let mut emoji = super::super::emoji::Emoji::load();
        let browser = crate::media::browser_settings(std::path::Path::new("."), "fr");
        let web = WebContext {
            paths: None,
            browser: &browser,
            cookie: None,
            profile_base: None,
        };
        let (pics, images) = (HashMap::new(), HashMap::new());
        let item = Uuid::from_u128(8100);
        let other = Uuid::from_u128(0xF00D);
        for classic in [false, true] {
            for (to, gives) in [(other, true), (Uuid::from_u128(0xA6E7), false)] {
                let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                world.agent_id = Uuid::from_u128(0xA6E7);
                let mut ui = ProfileUi::default();
                ui.open(&mut world, to);
                let ctx = egui::Context::default();
                // a frame's actions, and where the window then stands
                let mut frame = |events: Vec<egui::Event>| {
                    let mut want_names = HashSet::new();
                    let mut actions = Vec::new();
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                            events,
                            ..Default::default()
                        },
                        |ui_| {
                            actions = ui.show(
                                ui_.ctx(),
                                &p,
                                &mut emoji,
                                &pics,
                                &images,
                                &mut world,
                                &mut want_names,
                                &web,
                                classic,
                                true,
                            )
                        },
                    );
                    output.textures_delta.clear();
                    (actions, ui.demo_window_rect(&ctx))
                };
                let mut rect = None;
                for _ in 0..3 {
                    rect = frame(Vec::new()).1;
                }
                let rect = rect.expect("the profile window");
                let beside = egui::pos2(rect.left() / 2.0, rect.center().y);
                for (pos, on_window) in [(beside, false), (rect.center(), true), (rect.left_top() + Vec2::splat(4.0), true)] {
                    egui::DragAndDrop::set_payload(&ctx, InvDrag(item));
                    let mut events = vec![egui::Event::PointerMoved(pos)];
                    assert!(!frame(events.clone()).0.iter().any(|a| matches!(a, ProfileAction::Give { .. })));
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                    let given = frame(events)
                        .0
                        .into_iter()
                        .any(|a| matches!(a, ProfileAction::Give { to: t, items } if t == to && items == [item]));
                    assert_eq!(given, gives && on_window, "classic {classic}, to {to}, at {pos:?}");
                    egui::DragAndDrop::clear_payload(&ctx);
                }
            }
        }
    }

    #[test]
    fn classic_profile_map_button_honors_remote_friends_received_rights() {
        use crate::ui::context::{CtxAction, take_requests};
        use crate::ui::widgets::elide_tests::{Frames, painted};
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let id = Uuid::from_u128(7);
        world.social.friends.push(super::super::super::world::social::Friend {
            id,
            online: true,
            rights_given: 3,
            rights_has: 0,
        });
        let p = crate::theme::Theme::default().palette();
        let w = Window::new(id);
        let mut actions = Vec::new();
        let mut frames = Frames::new();
        for (received, online) in [(0, true), (3, true), (0, true), (3, false)] {
            world.social.friends[0].rights_has = received;
            world.social.friends[0].online = online;
            let mut point = egui::Pos2::ZERO;
            for _ in 0..2 {
                let texts = frames.run(Vec::new(), |ui| action_buttons(ui, &p, &world, &w, true, &mut actions));
                point = painted(&texts, aurora_i18n::tr!("ui-profile-show-on-map-2")).rect.center();
            }
            for pressed in [true, false] {
                frames.run(
                    vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ],
                    |ui| action_buttons(ui, &p, &world, &w, true, &mut actions),
                );
            }
            let requests = take_requests(&frames.ctx);
            assert_eq!(requests.len(), usize::from(received != 0 && online));
            assert!(requests.iter().all(|a| matches!(a, CtxAction::ShowOnMap(target) if *target == id)));
            assert!(actions.is_empty());
        }
    }

    #[test]
    fn edit_follows_the_server_until_changed() {
        let mut e = Edit::default();
        e.sync("a");
        assert_eq!((e.text.as_str(), e.dirty()), ("a", false));
        e.text = "ab".into();
        e.sync("c");
        assert_eq!((e.text.as_str(), e.base.as_str()), ("ab", "a"));
        e.text = "a".into();
        e.sync("c");
        assert_eq!(e.text, "c");
    }

    #[test]
    fn new_picks_get_an_id() {
        let e = PickEdit {
            name: "Plage".into(),
            ..Default::default()
        };
        let p = pick_of(&e);
        assert!(!p.id.is_nil());
        assert!(p.enabled);
        assert_eq!(p.name, "Plage");
    }
}
