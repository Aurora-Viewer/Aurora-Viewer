//! Avatar profile windows, one per avatar, laid out like Firestorm's
//! LLFloaterProfile (`newview/llfloaterprofile.cpp`, `llpanelprofile.cpp`,
//! `llpanelprofilepicks.cpp`, `llpanelprofileclassifieds.cpp`,
//! `floater_profile.xml`, `panel_profile_*.xml`, originally LGPL 2.1):
//! "Vie SL" (name, key, status, picture, birth date, account, partner,
//! about, groups, actions), "Flux" (the my.secondlife.com feed in a
//! browser), "Favoris" (picks), "Annonces" (classifieds), "Vie RL" and
//! "Notes". Our own profile can be edited (texts, pictures, picks, show in
//! search, full birth date); notes can be written on anyone.

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
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("aurora_open_profile"), id));
}

pub fn take_open_request(ctx: &egui::Context) -> Option<Uuid> {
    ctx.data_mut(|d| d.remove_temp::<Uuid>(egui::Id::new("aurora_open_profile")))
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
    /// Chat of one of our groups (double-click in the group list).
    GroupChat(Uuid),
    /// Picks and classifieds: "Montrer sur carte" / "Téléportation"
    /// (LLFloaterWorldMap::trackLocation, gAgent.teleportViaLocation).
    ShowOnMap(DVec3),
    Teleport(DVec3),
}

/// What the "Flux" browser needs from the app.
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

/// Left column of the labels ("Nom :", "Clé :", "Info :"...), right-aligned.
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

struct Window {
    id: Uuid,
    open: bool,
    tab: usize,
    about: Edit,
    fl_about: Edit,
    notes: Edit,
    /// "Info" / "Vie RL" shown with their links (the eye toggle) on our profile.
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
    /// "Supprimer..." waiting for a yes.
    confirm_delete: bool,
    /// Rights we grant this friend, being edited (LLFloaterProfilePermissions).
    rights_edit: Option<i32>,
    /// Height under "Info" in the "Vie SL" tab (previous frame).
    bottom_h: f32,
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
}

impl ProfileUi {
    /// LLAvatarActions::showProfile: one window per avatar, raised if open;
    /// each opening reloads the data (LLPanelProfile::updateData).
    pub fn open(&mut self, world: &mut World, id: Uuid) {
        if id.is_nil() {
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

    /// Demo captures: show one tab (0 Vie SL ... 5 Notes).
    pub fn set_tab(&mut self, id: Uuid, tab: usize) {
        if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
            w.tab = tab.min(TAB_NOTES);
        }
    }

    /// The "Flux" page that has the keyboard, if any.
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
        world: &World,
        want_names: &mut HashSet<Uuid>,
        web: &WebContext,
    ) -> Vec<ProfileAction> {
        let mut actions = Vec::new();
        let screen = ctx.content_rect();
        let mut pick_picture: Option<(PictureSlot, Uuid)> = None;
        for (i, w) in self.windows.iter_mut().enumerate() {
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
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(&win_id)));
            }
            let name = world.social.name_of(&w.id);
            world.social.avatar_names.want(&w.id);
            // floater_profile.xml: 485 x 510 (480 x 510 at least), centered
            let size = Vec2::new(485.0, 620.0);
            let pos = screen.center() - size * 0.5 + Vec2::splat(24.0 * (i % 6) as f32);
            let mut open = true;
            let mut floater = Floater::new(&win_id, name.clone(), pos, size);
            floater.min_size = Vec2::new(480.0, 620.0);
            floater.show(ctx, p, &mut open, |ui| {
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
                        ("Vie SL", true),
                        ("Flux", web.profile_base.is_some()),
                        ("Favoris", true),
                        ("Annonces", true),
                        ("Vie RL", true),
                        ("Notes", true),
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
            if let Some(rights) = w.rights_edit {
                rights_dialog(ctx, p, w, &name, rights, &mut actions);
            }
            if !open {
                // LLFloaterProfile::onClickCloseBtn: ask before losing edits
                let pick_dirty = w.pick_edit.as_ref().is_some_and(|(e, base)| e != base);
                if w.about.dirty() || w.fl_about.dirty() || w.notes.dirty() || pick_dirty {
                    w.confirm_close = true;
                } else {
                    w.open = false;
                }
            }
            if !w.open {
                w.web.close();
            }
        }
        self.windows.retain(|w| w.open);
        if let Some((slot, current)) = pick_picture {
            let title = match slot {
                PictureSlot::SecondLife => "Photo du profil",
                PictureSlot::FirstLife => "Photo RL",
            };
            self.picker.open(title, current);
            self.picker_slot = Some(slot);
        }
        if let Some(asset) = self.picker.show(ctx, p, world, images) {
            let key = match self.picker_slot {
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
        self.wanted_images.extend(self.picker.wanted_images.drain());
        actions
    }
}

/// What every tab reads.
struct TabCtx<'a> {
    p: &'a Palette,
    world: &'a World,
    images: &'a HashMap<Uuid, egui::TextureHandle>,
    own: bool,
}

/// "Enregistrer les modifications ?" shown when closing with edits.
fn unsaved_bar(ui: &mut egui::Ui, p: &Palette, w: &mut Window, actions: &mut Vec<ProfileAction>) {
    egui::Frame::new()
        .fill(p.warn.gamma_multiply(0.18))
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Des modifications ne sont pas enregistrées.").size(12.0).color(p.ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, "Annuler").clicked() {
                        w.confirm_close = false;
                    }
                    if flat_button(ui, p, "Ignorer").clicked() {
                        w.open = false;
                    }
                    if flat_button(ui, p, "Enregistrer").clicked() {
                        save_all(w, actions);
                        w.open = false;
                    }
                });
            });
        });
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
    if let Some((e, base)) = &w.pick_edit
        && e != base
    {
        actions.push(ProfileAction::Net(NetCommand::PickUpdate(Box::new(pick_of(e)))));
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
        .fill(p.field)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(egui::CornerRadius::same(2))
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
    ui.label(RichText::new("(en cours de chargement...)").size(12.0).color(p.muted));
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
            .max_height(size.y)
            .min_scrolled_height(size.y)
            .show(ui, |ui| {
                ui.add_sized(
                    [size.x, size.y],
                    egui::TextEdit::multiline(&mut edit.text)
                        .desired_width(size.x)
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

/// "Enregistrer" / "Annuler" at the bottom right (save_description_changes).
fn save_buttons(ui: &mut egui::Ui, p: &Palette, edit: &mut Edit) -> bool {
    let mut save = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_enabled_ui(edit.dirty(), |ui| {
            if flat_button(ui, p, "Annuler").clicked() {
                edit.text = edit.base.clone();
            }
            save = flat_button(ui, p, "Enregistrer").clicked();
        });
    });
    if save {
        edit.base = edit.text.clone();
    }
    save
}

/// A Phosphor icon `size` px wide that can be clicked.
fn icon(ui: &mut egui::Ui, p: &Palette, name: &str, size: f32, tint: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
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
    let icon = if *on { "eye" } else { "eye-slash" };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
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
        .on_hover_text(if *on { "Modifier le texte" } else { "Aperçu avec les liens" })
        .clicked()
    {
        *on = !*on;
    }
}

/// Friend rights icon (can_see_online, can_see_on_map, can_edit_objects).
fn rights_icon(ui: &mut egui::Ui, p: &Palette, icon: &str, granted: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
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
    let state = if granted { "autorisé" } else { "non autorisé" };
    resp.on_hover_text(format!("{tip} : {state}\nClic : modifier les droits")).clicked()
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
        label(ui, p, "Nom :");
        let rights_w = if friend.is_some() { 3.0 * 26.0 } else { 0.0 };
        let field_w = (ui.available_width() - rights_w - if own { 36.0 } else { 8.0 }).max(80.0);
        inset(ui, p, field_w, 22.0, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(name).size(12.5).color(p.ink)).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| copy_menu(ui, p, world, id));
            });
        });
        if own
            && icon(ui, p, "sliders", 20.0, p.muted)
                .on_hover_text("Définir le nom d'affichage")
                .clicked()
        {
            actions.push(ProfileAction::DisplayName);
        }
        if let Some(f) = friend {
            ui.spacing_mut().item_spacing.x = 4.0;
            let mut clicked = false;
            clicked |= rights_icon(ui, p, "eye", f.rights_given & 1 != 0, "Voir quand je suis en ligne");
            clicked |= rights_icon(ui, p, "map-pin", f.rights_given & 2 != 0, "Me trouver sur la carte");
            clicked |= rights_icon(ui, p, "cube", f.rights_given & 4 != 0, "Modifier, supprimer ou prendre mes objets");
            if clicked {
                w.rights_edit = Some(f.rights_given);
            }
        }
    });
    // key row: the UUID, then the online status of the others
    ui.horizontal(|ui| {
        label(ui, p, "Clé :");
        let key_w = (ui.available_width() - if own { 8.0 } else { 100.0 }).max(80.0);
        inset(ui, p, key_w, 20.0, |ui| {
            ui.add(egui::Label::new(RichText::new(id.to_string()).size(11.5).monospace().color(p.ink)).selectable(true));
        });
        if !own {
            let status = profiles::online_status(profile, own, friend.map(|f| (f.online, f.rights_has & 1 != 0)));
            let (text, col) = match status {
                Online::Yes => ("Connecté", p.success),
                Online::No => ("Déconnecté", p.muted),
                Online::Unknown => ("Inconnu", p.warn),
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
                widgets::icon_menu(ui, p, "gear-six", "Photo du profil", |ui| {
                    ui.add_enabled(false, egui::Button::new("Charger une photo"))
                        .on_disabled_hover_text("À venir");
                    if ui.button("Changer la photo").clicked() {
                        *pick_picture = Some((PictureSlot::SecondLife, pr.sl_image));
                        ui.close();
                    }
                    if ui
                        .add_enabled(!pr.sl_image.is_nil(), egui::Button::new("Supprimer la photo"))
                        .clicked()
                    {
                        actions.push(ProfileAction::Save {
                            target: id,
                            data: llsd_map! { "sl_image_id" => Uuid::nil() },
                        });
                        ui.close();
                    }
                });
            }
        });
        let r = widgets::picture(ui, p, pics.get(&id), Vec2::splat(162.0), "user-circle", "Photo du profil");
        if own
            && let Some(pr) = profile
            && r.on_hover_text("Clic : changer la photo").clicked()
        {
            *pick_picture = Some((PictureSlot::SecondLife, pr.sl_image));
        }
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let col_w = ui.available_width() - 4.0;
            ui.label(RichText::new("Date de naissance:").size(12.0).color(p.muted));
            inset(ui, p, col_w, 38.0, |ui| {
                match profile.and_then(|pr| profiles::birth_text(pr, own, now)) {
                    Some(t) => {
                        ui.label(RichText::new(t).size(12.0).color(p.ink));
                    }
                    None => loading(ui, p),
                }
            });
            ui.add_space(3.0);
            ui.label(RichText::new("Compte :").size(12.0).color(p.muted));
            inset(ui, p, col_w, 52.0, |ui| match profile {
                Some(pr) => {
                    ui.horizontal_top(|ui| {
                        if let Some(b) = profiles::badge(pr) {
                            icon(ui, p, "star", 18.0, p.amber).on_hover_text(b);
                        }
                        ui.label(RichText::new(profiles::account_text(pr)).size(12.0).color(p.ink));
                    });
                }
                None => loading(ui, p),
            });
            ui.add_space(3.0);
            ui.label(RichText::new("Partenaire :").size(12.0).color(p.muted));
            inset(ui, p, col_w, 22.0, |ui| match profile.map(|pr| pr.partner) {
                Some(partner) if !partner.is_nil() => {
                    world.social.avatar_names.want(&partner);
                    let r = ui.add(
                        egui::Label::new(
                            RichText::new(world.social.name_of(&partner))
                                .size(12.0)
                                .color(super::colors::c(super::colors::get().chat_slurl)),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if r.on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Voir le profil")
                        .clicked()
                    {
                        request_open(ui.ctx(), partner);
                    }
                }
                Some(_) => {
                    ui.label(RichText::new("Aucun").size(12.0).color(p.ink));
                }
                None => loading(ui, p),
            });
        });
    });
    // "Info" (103 high at least, follows="all") takes the space left by the
    // groups (80), "Donner" and the buttons, measured on the previous frame
    let groups_h = 80.0;
    let field_w = full_w - LABEL_W - 20.0;
    ui.add_space(6.0);
    let info_h = (ui.available_height() - w.bottom_h - 8.0).max(103.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            label(ui, p, "Info :");
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
                egui::Id::new(("sl_about", id)),
            );
        } else {
            inset(ui, p, field_w, info_h, |ui| loading(ui, p));
        }
    });
    let bottom = ui.scope(|ui| {
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                label(ui, p, "Groupes :");
                if !own {
                    ui.horizontal(|ui| {
                        ui.add_space(LABEL_W - 22.0);
                        ui.add_enabled(false, egui::Button::new("+").min_size(Vec2::splat(20.0)))
                            .on_disabled_hover_text("Inviter dans un groupe (à venir)");
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
            label(ui, p, "Donner :");
            inset(ui, p, field_w, 20.0, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Déposez l'objet ici.").size(11.5).color(p.muted_dim))
                        .on_hover_text("Donner un objet de l'inventaire (à venir)");
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
            ui.label(RichText::new("Aucun").size(12.0).color(p.ink));
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
            "Double-clic : chat du groupe"
        } else {
            "Profil du groupe (à venir)"
        };
        if r.on_hover_text(tip).double_clicked() && member {
            actions.push(ProfileAction::GroupChat(g.id));
        }
    }
}

/// Own profile bottom: "Afficher dans la recherche", "Afficher la date de
/// naissance complète", Enregistrer / Annuler of "Info".
fn own_bottom(ui: &mut egui::Ui, p: &Palette, w: &mut Window, profile: Option<&AvatarProfile>, now: f64, actions: &mut Vec<ProfileAction>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        if let Some(pr) = profile {
            let mut publish = pr.allow_publish();
            if ui.checkbox(&mut publish, "Afficher dans la recherche").changed() {
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
                if ui.checkbox(&mut full, "Afficher la date de naissance complète").changed() {
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
            ui.add_enabled(false, egui::Button::new("Situer sur la carte").min_size(size))
                .on_disabled_hover_text("À venir");
            ui.add_enabled(false, egui::Button::new("Payer").min_size(size))
                .on_disabled_hover_text("À venir");
        });
        ui.vertical(|ui| {
            if ui.add(egui::Button::new("Proposer de téléporter").min_size(size)).clicked() {
                actions.push(ProfileAction::OfferTeleport(id));
            }
            if ui.add(egui::Button::new("Message instantané").min_size(size)).clicked() {
                actions.push(ProfileAction::Im(id));
            }
        });
        ui.vertical(|ui| {
            let friend_label = if is_friend { "Supprimer l'ami" } else { "Ajouter un ami" };
            ui.add_enabled(false, egui::Button::new(friend_label).min_size(size))
                .on_disabled_hover_text("À venir");
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let bw = Vec2::new(col_w - 34.0, 20.0);
                if ui
                    .add(egui::Button::new(if blocked { "Débloquer" } else { "Bloquer" }).min_size(bw))
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
    widgets::icon_menu(ui, p, "list", "Plus", |ui| {
        for label in ["Carte", "Payer", "Partager", "Appeler"] {
            ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text("À venir");
        }
        if ui.button(if blocked { "Débloquer" } else { "Bloquer" }).clicked() {
            actions.push(ProfileAction::ToggleBlock(id));
            ui.close();
        }
        ui.separator();
        if ui.button("Copier le nom").clicked() {
            ui.ctx().copy_text(world.social.name_of(&id));
            ui.close();
        }
        if ui.button("Copier l'URI").clicked() {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
            ui.close();
        }
        if ui.button("Copier l'UUID / clé de l'agent").clicked() {
            ui.ctx().copy_text(id.to_string());
            ui.close();
        }
        ui.separator();
        ui.add_enabled(false, egui::Button::new("Signaler"))
            .on_disabled_hover_text("À venir");
    });
}

/// menu_fs_profile_name_field.xml (copy menu inside the name field).
fn copy_menu(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid) {
    let names = &world.social.avatar_names;
    let display = names.get(&id).map(|n| n.display(&names.options));
    let legacy = world.legacy_name(&id);
    widgets::icon_menu(ui, p, "link", "Copier", |ui| {
        if ui
            .add_enabled(display.is_some(), egui::Button::new("Copier le nom d'affichage"))
            .clicked()
        {
            ui.ctx().copy_text(display.clone().unwrap_or_default());
            ui.close();
        }
        if ui.add_enabled(legacy.is_some(), egui::Button::new("Copier le nom")).clicked() {
            ui.ctx().copy_text(legacy.clone().unwrap_or_default());
            ui.close();
        }
        if ui.button("Copier l'UUID / clé de l'agent").clicked() {
            ui.ctx().copy_text(id.to_string());
            ui.close();
        }
        if ui.button("Copier l'URI").clicked() {
            ui.ctx().copy_text(format!("secondlife:///app/agent/{id}/about"));
            ui.close();
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
    Floater::new(&win_id, "Droits", screen.center() - Vec2::new(150.0, 70.0), Vec2::new(300.0, 140.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(RichText::new(format!("Autoriser {name} à :")).size(12.5).color(p.ink));
            for (bit, label) in [
                (1, "Voir quand je suis en ligne"),
                (2, "Me trouver sur la carte"),
                (4, "Modifier, supprimer ou prendre mes objets"),
            ] {
                let mut on = r & bit != 0;
                if ui.checkbox(&mut on, label).changed() {
                    r = if on { r | bit } else { r & !bit };
                }
            }
            ui.add_space(4.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if flat_button(ui, p, "Annuler").clicked() {
                    done = true;
                }
                if flat_button(ui, p, "OK").clicked() {
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
        ui.label(RichText::new("Pas de profil web sur cette grille.").size(12.0).color(c.p.muted));
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
            Some(s) => format!("Heure de chargement : {s:.2} secondes"),
            None => "Chargement…".to_owned(),
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
            ui.set_width(150.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                for (id, name) in items {
                    let sel = *selected == Some(*id);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(150.0, 22.0), egui::Sense::click());
                    let fill = if sel {
                        p.violet.gamma_multiply(0.6)
                    } else if resp.hovered() {
                        p.raised
                    } else {
                        p.field
                    };
                    ui.painter().rect_filled(rect, 0.0, fill);
                    let label = if name.is_empty() { "(sans nom)" } else { name.as_str() };
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
                RichText::new("Faites connaître aux autres résidents vos endroits favoris dans Second Life.")
                    .size(12.0)
                    .color(p.muted),
            );
        });
        ui.horizontal(|ui| {
            let creating = w.pick_edit.as_ref().is_some_and(|(e, _)| e.id.is_nil());
            let can_add = pr.picks.len() < picks_limit(world) && !creating;
            if ui.add_enabled(can_add, egui::Button::new("Nouveau...")).clicked() {
                w.pick_edit = Some((new_pick(world), PickEdit::default()));
                w.pick_sel = Some(Uuid::nil());
            }
            if ui.add_enabled(w.pick_sel.is_some(), egui::Button::new("Supprimer...")).clicked() {
                w.confirm_delete = true;
            }
            if w.confirm_delete {
                ui.label(RichText::new("Supprimer ce favori ?").size(12.0).color(p.warn));
                if flat_button(ui, p, "Oui").clicked() {
                    if let Some(id) = w.pick_sel.filter(|id| !id.is_nil()) {
                        actions.push(ProfileAction::Net(NetCommand::PickDelete(id)));
                    }
                    w.pick_edit = None;
                    w.pick_sel = None;
                    w.confirm_delete = false;
                }
                if flat_button(ui, p, "Non").clicked() {
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
                "Vous n'avez pas créé de favoris.\nCliquez sur le bouton Nouveau pour créer un favori."
            } else {
                "L'utilisateur n'a pas de favoris"
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
        location: Some(location_text(&["(mise à jour après enregistrement)", &parcel_name, &region], pos)),
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
        "user-circle",
        "",
    );
    ui.label(RichText::new("Nom :").size(12.0).strong().color(p.ink));
    if let (true, Some((e, _))) = (own, w.pick_edit.as_mut()) {
        ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(width).char_limit(63));
    } else {
        inset(ui, p, width, 22.0, |ui| {
            ui.label(RichText::new(&name).size(12.5).color(p.ink));
        });
    }
    ui.label(RichText::new("Description :").size(12.0).strong().color(p.ink));
    // a fixed height: sizes taken from the space left would grow the window
    let desc_h = 80.0;
    if let (true, Some((e, _))) = (own, w.pick_edit.as_mut()) {
        egui::ScrollArea::vertical()
            .id_salt(("pick_desc", sel))
            .max_height(desc_h)
            .show(ui, |ui| {
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
    ui.label(RichText::new("Lieu :").size(12.0).strong().color(p.ink));
    let location = match (&w.pick_edit, info) {
        (Some((e, _)), _) if own && e.location.is_some() => e.location.clone().unwrap_or_default(),
        (_, Some(i)) if !i.parcel.is_nil() => match world.profiles.parcels.get(&i.parcel) {
            Some(pa) => location_text(&["", &pa.name, &pa.sim_name], pos),
            None => {
                w.ask(i.parcel, NetCommand::ParcelInfoRequest(i.parcel), actions);
                "Chargement en cours...".to_owned()
            }
        },
        (_, Some(i)) => location_text(&["", &i.sim_name], pos),
        _ => String::new(),
    };
    inset(ui, p, width, 22.0, |ui| {
        ui.add(egui::Label::new(RichText::new(location).size(12.0).color(p.muted)).truncate());
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let has_pos = pos != DVec3::ZERO;
        if ui.add_enabled(has_pos, egui::Button::new("Montrer sur carte")).clicked() {
            actions.push(ProfileAction::ShowOnMap(pos));
        }
        if ui.add_enabled(has_pos, egui::Button::new("Téléportation")).clicked() {
            actions.push(ProfileAction::Teleport(pos));
        }
        if own && ui.button("Définir le lieu").clicked() {
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
        ui.separator();
        ui.horizontal(|ui| {
            if e.id.is_nil() {
                if ui.button("Créer un lieu").clicked() {
                    created = Some(pick_of(e));
                }
            } else {
                let dirty = e != base;
                if ui.add_enabled(dirty, egui::Button::new("Enregistrer le lieu")).clicked() {
                    *base = e.clone();
                    actions.push(ProfileAction::Net(NetCommand::PickUpdate(Box::new(pick_of(e)))));
                }
                if ui.add_enabled(dirty, egui::Button::new("Annuler")).clicked() {
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
            ui.add_enabled(false, egui::Button::new("Nouvelle..."))
                .on_disabled_hover_text("Publier une petite annonce (payante) : à venir");
            if ui
                .add_enabled(w.classified_sel.is_some(), egui::Button::new("Supprimer..."))
                .clicked()
            {
                w.confirm_delete = true;
            }
            if w.confirm_delete {
                ui.label(RichText::new("Supprimer cette annonce ?").size(12.0).color(p.warn));
                if flat_button(ui, p, "Oui").clicked() {
                    if let Some(id) = w.classified_sel.take() {
                        actions.push(ProfileAction::Net(NetCommand::ClassifiedDelete(id)));
                    }
                    w.confirm_delete = false;
                }
                if flat_button(ui, p, "Non").clicked() {
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
                "Vous n'avez pas créé de petites annonces.\nCliquez sur le bouton Nouvelle pour créer une petite annonce."
            } else {
                "L'utilisateur n'a pas de petites annonces"
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
        "user-circle",
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
        "Endroit :",
        &location_text(&[&info.parcel_name, &info.sim_name], info.pos_global),
    );
    row(ui, "Type de contenu :", profiles::classified_maturity(info.flags));
    row(ui, "Catégorie :", &world.classified_category(info.category));
    row(ui, "Date de création :", &profiles::slt_date(info.creation_date));
    row(ui, "Coût de l'annonce :", &format!("L${}", info.price));
    if c.own {
        let renew = info.flags & aurora_net::profile::classified_flags::AUTO_RENEW != 0;
        row(ui, "Renouv. auto :", if renew { "Activé" } else { "Désactivé" });
    }
    ui.label(RichText::new("Description :").size(12.0).color(p.muted));
    rich(ui, p, emoji, world, want_names, &info.desc);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Téléportation").clicked() {
            actions.push(ProfileAction::Teleport(info.pos_global));
        }
        if ui.button("Carte").clicked() {
            actions.push(ProfileAction::ShowOnMap(info.pos_global));
        }
        if c.own {
            ui.add_enabled(false, egui::Button::new("Modifier"))
                .on_disabled_hover_text("À venir");
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
        let r = widgets::picture(ui, p, c.images.get(&pr.fl_image), Vec2::splat(200.0), "user-circle", "Photo RL");
        if c.own && r.on_hover_text("Clic : changer l'image").clicked() {
            *pick_picture = Some((PictureSlot::FirstLife, pr.fl_image));
        }
        if c.own {
            ui.add_space(70.0);
            ui.vertical(|ui| {
                ui.add_space(45.0);
                let size = Vec2::new(120.0, 20.0);
                ui.add_enabled(false, egui::Button::new("Charger une image").min_size(size))
                    .on_disabled_hover_text("À venir");
                if ui.add(egui::Button::new("Changer l'image").min_size(size)).clicked() {
                    *pick_picture = Some((PictureSlot::FirstLife, pr.fl_image));
                }
                if ui
                    .add_enabled(!pr.fl_image.is_nil(), egui::Button::new("Supprimer l'image").min_size(size))
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
        egui::Id::new(("fl_about", w.id)),
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
        RichText::new("Prenez des notes sur cette personne. Personne ne peut les voir.")
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
        .max_height(size.y)
        .min_scrolled_height(size.y)
        .show(ui, |ui| {
            ui.add_sized(
                [size.x, size.y],
                egui::TextEdit::multiline(&mut w.notes.text)
                    .desired_width(size.x)
                    .char_limit(65_530),
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
