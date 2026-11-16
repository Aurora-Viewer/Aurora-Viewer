//! Experience profile: what the experience is and the agent's permission
//! for it, with the Legacy Viewer's Allow / Forget / Block buttons
//! (LLFloaterExperienceProfile, indra/newview/llfloaterexperienceprofile.cpp,
//! originally LGPL 2.1, refreshExperience): the banner (the logo), the name
//! with a button that copies the experience's SLURL, the description, the
//! rating with its badge, the scope, the location, owner, group and
//! Marketplace links with a conversation's menus, and "Report Abuse". An
//! administrator also edits it (the Edit tab, experienceIsAdmin): name,
//! description, rating, location, group (the owner only), marketplace
//! address, logo, "Enable Experience" and "Hide In Search", saved with
//! UpdateExperience; closing or going back with changes asks first.
//!
//! The banner spans the window edge to edge, where the Legacy Viewer frames
//! it in a margin, with the shape of the Legacy Viewer's box (creators make
//! their banners for it); the window grows no wider than `MAX_WIDTH`, so the
//! banner never takes the screen. The permission and its actions stay in a
//! footer under the scrolled details.

use super::menu;
use super::texture_library::texture_view::ImageView;
use super::texture_picker::TexturePicker;
use super::widgets::{self, ActionKind, Floater, action_button_if, flat_button};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::experience::{
    ExperienceDetails, ExperienceEdit, ExperiencePermission, ExperiencePermissionChange, MATURITIES, update_body,
};
use egui::{Rect, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const WINDOW: &str = "experience_profile";
/// Tall enough for a banner and every row: the window opens before the
/// details arrive, so its size cannot follow them.
const VIEW_SIZE: Vec2 = Vec2::new(400.0, 600.0);
const EDIT_SIZE: Vec2 = Vec2::new(420.0, 600.0);
const LABEL_WIDTH: f32 = 110.0;
const LOGO_SIZE: Vec2 = Vec2::new(180.0, 122.0);
/// floater_experienceprofile.xml: the name's and the marketplace address's
/// byte limits, the description's character limit.
const NAME_BYTES: usize = 63;
const MARKETPLACE_BYTES: usize = 255;
const DESCRIPTION_CHARS: usize = 2048;

/// The Legacy Viewer's banner: the `logo` texture picker of
/// floater_experienceprofile.xml, 290 × 197 with its 23-point caption
/// (LLTextureCtrl::mCaptionHeight), so a 290 × 174 picture.
pub const BANNER_ASPECT: f32 = 290.0 / 174.0;
const MAX_WIDTH: f32 = 560.0;
/// The least size of the window: a band of banner, the name and the first
/// rows, and the actions.
const MIN_SIZE: Vec2 = Vec2::new(320.0, 470.0);
/// The height kept under the banner for the details and the actions, and
/// the least band of banner shown.
const DETAILS_ROOM: f32 = 250.0;
const MIN_BANNER: f32 = 80.0;
const TITLE_SIZE: f32 = 15.0;
// the least size leaves the details and the actions their room, and the
// window can still be made that narrow
const _: () = assert!(MIN_SIZE.y >= DETAILS_ROOM + MIN_BANNER && MIN_SIZE.x <= MAX_WIDTH);
/// The side of the owner's picture and of the group's insignia.
const PICTURE: f32 = 18.0;

/// `secondlife:///app/experience/<id>/profile`, the experience's SLURL
/// (LLSLURL("experience", id, "profile")).
pub fn slurl(id: Uuid) -> String {
    format!("secondlife:///app/experience/{id}/profile")
}

/// The right-click menu of an experience's name: "Copy SLurl", as the
/// Legacy Viewer's link of the name (LLUrlEntryExperienceProfile).
pub fn name_menu(response: &egui::Response, p: &Palette, id: Uuid) {
    menu::context_menu(response, p, |ui| {
        if menu::item(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl")) {
            ui.ctx().copy_text(slurl(id));
        }
    });
}

/// The pictures the window shows: the decoded images by texture, the
/// residents' profile pictures by resident.
pub struct Pictures<'a> {
    pub images: &'a HashMap<Uuid, egui::TextureHandle>,
    pub avatars: &'a HashMap<Uuid, egui::TextureHandle>,
}

/// What follows a save or the "Save changes?" answer (PostSaveAction).
#[derive(Clone, Copy, PartialEq, Debug)]
enum After {
    /// Save: the Edit tab stays, with what was saved.
    Stay,
    /// Back: the profile's view.
    View,
    /// The window closes.
    Close,
}

#[derive(Default)]
pub struct ExperienceProfileUi {
    /// The Edit tab: the experience and its fields.
    edit: Option<(Uuid, ExperienceEdit)>,
    /// "Save changes?" asked, and what follows the answer.
    confirm: Option<After>,
    /// A save sent, and what follows once it is saved.
    after_save: Option<After>,
    picker: TexturePicker,
    /// The logo chosen was not a full-permission texture.
    logo_refused: bool,
    pub wanted_images: HashSet<Uuid>,
    /// Residents whose profile picture the window shows (the owner).
    pub wanted_pics: HashSet<Uuid>,
    /// Groups whose profile was asked, for their insignia (once each).
    asked_groups: HashSet<Uuid>,
    /// The banner at full resolution, decoded on the scene's workers like a
    /// profile's photo (`prepare_banner`): the interface copies stop at
    /// 256 pixels, blurred across a window-wide banner.
    banner: Option<(Uuid, ImageView)>,
    /// How many physical pixels wide the banner was last drawn.
    banner_px: f32,
}

impl ExperienceProfileUi {
    /// Shows the profile of `world.experiences.profile`; returns the
    /// permission change asked for.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        pics: Pictures,
    ) -> Option<(Uuid, ExperiencePermissionChange)> {
        let images = pics.images;
        let Some(id) = world.experiences.profile else {
            self.leave_edit(ctx);
            return None;
        };
        if self.edit.as_ref().is_some_and(|(e, _)| *e != id) {
            self.leave_edit(ctx);
        }
        if std::mem::take(&mut world.experiences.profile_raise) {
            widgets::expand_floater(ctx, WINDOW);
        }
        let details = world.experiences.details.get(&id).cloned();
        let admin = world.experiences.admin.contains(&id) && details.as_ref().is_some_and(|d| !d.invalid());
        if !admin {
            self.leave_edit(ctx);
        }
        // LLFloaterExperienceProfile::mPostEdit: opened to be edited
        if admin
            && world.experiences.profile_edit
            && let Some(d) = &details
        {
            world.experiences.profile_edit = false;
            self.enter_edit(ctx, id, d);
        }
        if self.finish_save(ctx, world, id) {
            return None;
        }
        let screen = ctx.content_rect();
        let mut open = true;
        let mut change = None;
        let mut pick_logo = false;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("ui-experience-profile-title"),
            egui::pos2(screen.center().x - VIEW_SIZE.x * 0.5, screen.center().y - VIEW_SIZE.y * 0.5),
            VIEW_SIZE,
        );
        // room for a band of banner, the first rows and the actions
        floater.min_size = MIN_SIZE;
        floater.max_width(MAX_WIDTH).flush_body().show(ctx, p, &mut open, |ui| {
            let Some(d) = details.as_ref() else {
                body(ui, |ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-experience-profile-loading"))
                            .size(12.0)
                            .color(p.muted),
                    );
                });
                return;
            };
            if self.edit.is_some() {
                body(ui, |ui| pick_logo = self.edit_tab(ui, p, world, images, d));
            } else {
                change = self.view(ui, p, world, &pics, d, admin);
            }
        });
        if pick_logo && let Some((_, e)) = &self.edit {
            self.picker
                .open_with_defaults(aurora_i18n::tr!("ui-experience-profile-logo"), e.logo, None, true);
            // stricter than the Legacy Viewer: full-permission textures
            // only (experience_logo_allowed); the others stay listed, greyed
            self.picker.required_permissions = crate::world::inventory::actions::FULL;
            self.logo_refused = false;
        }
        if self.edit.is_some() {
            if let Some(asset) = self.picker.show(ctx, p, world, images)
                && let Some((_, e)) = &mut self.edit
            {
                // the UUID field and the pipette reach textures the list
                // greys out: the shared rule decides for every way in
                if crate::world::inventory::actions::experience_logo_allowed(&world.inventory, asset) {
                    e.logo = asset;
                    self.logo_refused = false;
                } else {
                    self.logo_refused = true;
                }
            }
            self.wanted_images.extend(self.picker.wanted_images.drain());
        }
        if !open {
            // canClose: changes not saved ask first
            if self.dirty(world) {
                self.confirm = Some(After::Close);
            } else {
                self.close(ctx, world);
            }
        }
        change.map(|c| (id, c))
    }

    fn enter_edit(&mut self, ctx: &egui::Context, id: Uuid, d: &ExperienceDetails) {
        self.edit = Some((id, ExperienceEdit::of(d)));
        self.confirm = None;
        self.after_save = None;
        self.logo_refused = false;
        widgets::size_floater(ctx, WINDOW, EDIT_SIZE);
    }

    /// The banner of the profile on screen, at the size it is drawn
    /// (`ProfileUi::prepare_photos` does the same for a photo); dropped
    /// with the window.
    pub(crate) fn prepare_banner(&mut self, ctx: &egui::Context, scene: &mut crate::scene::Scene, world: &World) {
        let logo = world
            .experiences
            .profile
            .and_then(|id| world.experiences.details.get(&id))
            .map_or(Uuid::nil(), |d| d.logo);
        if logo.is_nil() {
            self.banner = None;
            return;
        }
        if self.banner.as_ref().is_none_or(|(asset, _)| *asset != logo) {
            self.banner = Some((logo, ImageView::default()));
        }
        if let Some((_, view)) = &mut self.banner {
            // the size it is drawn, before the first frame a usual window's
            let px = if self.banner_px > 0.0 {
                self.banner_px
            } else {
                VIEW_SIZE.x * ctx.pixels_per_point()
            };
            scene.textures.note_usage_boosted(&logo, px);
            view.prepare(ctx, logo, scene);
        }
    }

    fn leave_edit(&mut self, ctx: &egui::Context) {
        if self.edit.take().is_some() {
            widgets::size_floater(ctx, WINDOW, VIEW_SIZE);
        }
        self.confirm = None;
        self.after_save = None;
        self.picker.open = false;
        self.logo_refused = false;
    }

    fn close(&mut self, ctx: &egui::Context, world: &mut World) {
        self.leave_edit(ctx);
        world.experiences.profile = None;
    }

    /// onFieldChanged: a field differs from the experience as saved.
    fn dirty(&self, world: &World) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|(id, e)| world.experiences.details.get(id).is_some_and(|d| e.dirty(d)))
    }

    fn save(&mut self, world: &mut World, after: After) {
        let Some((id, e)) = &self.edit else {
            return;
        };
        let Some(d) = world.experiences.details.get(id) else {
            return;
        };
        let body = update_body(d, e);
        world.experiences.save(*id, body);
        self.after_save = Some(after);
        self.confirm = None;
    }

    /// onSaveComplete: once saved, what the save was for; true when the
    /// window closed.
    fn finish_save(&mut self, ctx: &egui::Context, world: &mut World, id: Uuid) -> bool {
        let Some(after) = self.after_save else {
            return false;
        };
        if world.experiences.saving.contains(&id) {
            return false;
        }
        self.after_save = None;
        let Some(Ok(notes)) = world.experiences.saved.get(&id) else {
            return false;
        };
        // ignored fields are shown in the view rather than closed away
        // (the Legacy Viewer alerts them on their own)
        let after = if after == After::Close && !notes.is_empty() {
            After::View
        } else {
            after
        };
        match after {
            After::Stay => {
                if let (Some((_, e)), Some(d)) = (self.edit.as_mut(), world.experiences.details.get(&id)) {
                    *e = ExperienceEdit::of(d);
                }
                false
            }
            After::View => {
                self.leave_edit(ctx);
                false
            }
            After::Close => {
                self.close(ctx, world);
                true
            }
        }
    }

    /// The profile: the banner across the top, the details scrolled under
    /// it, and the permission and its actions in a footer that never
    /// scrolls away.
    fn view(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        pics: &Pictures,
        d: &ExperienceDetails,
        admin: bool,
    ) -> Option<ExperiencePermissionChange> {
        // the banner stays above the scrolled details: the scroll bar's
        // strip would otherwise cut it short of the window's right edge
        if !d.logo.is_nil() {
            self.wanted_images.insert(d.logo);
            // never so tall that the details have no room left: past that,
            // the whole banner smaller (`banner`)
            let most = (ui.available_height() - DETAILS_ROOM).max(MIN_BANNER);
            // the interface copy, and over it the full resolution once
            // decoded
            let full = self
                .banner
                .as_ref()
                .filter(|(asset, _)| *asset == d.logo)
                .and_then(|(_, view)| view.image.clone());
            let drawn = banner(ui, p, pics.images.get(&d.logo), full.as_ref(), most);
            self.banner_px = drawn.width() * ui.ctx().pixels_per_point();
        }
        let (mut list, footer) = widgets::above_footer(ui);
        egui::ScrollArea::vertical()
            .id_salt("experience_profile_body")
            .auto_shrink([false, false])
            .show(&mut list, |ui| {
                body(ui, |ui| self.details(ui, p, world, pics, d));
            });
        let mut change = None;
        let mut edit = false;
        footer.show(ui, |ui| {
            footer_body(ui, |ui| (change, edit) = actions(ui, p, world, d.id, admin));
        });
        if edit {
            self.enter_edit(ui.ctx(), d.id, d);
        }
        change
    }

    /// Everything under the banner.
    fn details(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World, pics: &Pictures, d: &ExperienceDetails) {
        let id = d.id;
        let name = if d.name.trim().is_empty() { id.to_string() } else { d.name.clone() };
        // the name, and right after it the button that copies the
        // experience's SLURL (the Legacy Viewer draws the name as that
        // SLURL's link; the avatar profile's place has the same button);
        // a right click on the name copies it too
        let copy = Vec2::splat(24.0);
        let gap = ui.spacing().item_spacing.x;
        let room = (ui.available_width() - copy.x - gap).max(0.0);
        ui.horizontal_top(|ui| {
            let title = ui
                .scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(
                        egui::Label::new(RichText::new(name).size(TITLE_SIZE).strong().color(p.ink))
                            .wrap()
                            .sense(egui::Sense::click()),
                    )
                })
                .inner;
            name_menu(&title, p, id);
            // centred on the name's first line, however many it wraps to
            let line = ui.ctx().fonts_mut(|f| f.row_height(&egui::FontId::proportional(TITLE_SIZE)));
            let at = Rect::from_center_size(
                egui::pos2(title.rect.right() + gap + copy.x / 2.0, title.rect.top() + line / 2.0),
                copy,
            );
            let button = ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| {
                super::widgets::action_button(ui, p, "copy-filled", "", ActionKind::Flat, copy)
            });
            if button.inner.on_hover_text(aurora_i18n::tr!("common-copy-slurl")).clicked() {
                ui.ctx().copy_text(slurl(id));
            }
        });
        if d.invalid() {
            ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-experience-profile-unknown"))
                        .size(12.0)
                        .color(p.muted),
                )
                .wrap(),
            );
        }
        if !d.description.trim().is_empty() {
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(RichText::new(&d.description).size(12.0).color(p.ink))
                    .wrap()
                    .selectable(true),
            );
        }
        ui.add_space(8.0);
        if !d.invalid() {
            labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-rating"), |ui| {
                if d.maturity != 0 {
                    super::widgets::maturity_badge_of(ui, p, d.maturity, 14.0);
                }
                value(ui, p, crate::world::worldmap::access_label(d.maturity));
            });
            let scope = if d.grid_wide() {
                aurora_i18n::tr!("world-experiences-grid-scope")
            } else {
                aurora_i18n::tr!("world-experiences-land-scope")
            };
            row(ui, p, aurora_i18n::tr!("ui-experience-profile-scope"), scope);
        }
        if !d.slurl.trim().is_empty() {
            labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-location"), |ui| {
                location(ui, p, &d.slurl)
            });
        }
        if !d.owner.is_nil() {
            labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-owner"), |ui| {
                self.owner(ui, p, world, pics, d.owner);
            });
        }
        if !d.group.is_nil() {
            labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-group"), |ui| {
                self.group(ui, p, world, pics, d.group);
            });
        }
        if !d.marketplace.is_empty() {
            labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-marketplace"), |ui| {
                web_link(ui, p, &d.marketplace);
            });
        }
        save_notes(ui, p, world, id);
    }

    /// The owner's picture and name, which opens the profile (LLSLURL
    /// "agent ... inspect"); hidden as everywhere while RLVa hides names.
    fn owner(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World, pics: &Pictures, owner: Uuid) {
        if !world.rlva_can_name(owner) {
            value(ui, p, aurora_i18n::tr!("scene-object-hidden-person"));
            return;
        }
        world.social.want_name(owner);
        let name = world.social.name_of(&owner);
        self.wanted_pics.insert(owner);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(PICTURE), egui::Sense::hover());
        super::widgets::avatar_picture(ui, p, rect, pics.avatars.get(&owner), &name, false);
        let r = link(ui, p, &name).on_hover_text(aurora_i18n::tr!("common-view-profile"));
        // the menu of a resident's name in a conversation (menu_url_agent.xml)
        super::chat::resident_name_menu(&r, p, world, owner);
        if r.clicked() {
            super::profile::request_open(ui.ctx(), owner);
        }
    }

    /// The group's insignia and name, which opens its profile (LLSLURL
    /// "group ... inspect"); its profile is asked once for the insignia of
    /// a group the agent is not in.
    fn group(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World, pics: &Pictures, group: Uuid) {
        let ours = world.groups.group(&group).map(|g| (g.name.clone(), g.insignia));
        let searched = world.search.groups.get(&group).map(|g| (g.name.clone(), g.insignia));
        if ours.is_none() && searched.is_none() && self.asked_groups.insert(group) {
            world.search.request_group(group);
        }
        let insignia = [&ours, &searched].into_iter().flatten().map(|(_, i)| *i).find(|i| !i.is_nil());
        let groups = world.groups.groups.clone();
        let name = world
            .land
            .group_name(&group, &groups)
            .or_else(|| searched.map(|(n, _)| n).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into());
        if let Some(i) = insignia {
            self.wanted_images.insert(i);
        }
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(PICTURE), egui::Sense::hover());
        super::widgets::avatar_picture(ui, p, rect, insignia.and_then(|i| pics.images.get(&i)), &name, false);
        let r = link(ui, p, &name).on_hover_text(aurora_i18n::tr!("ui-group-profile-view"));
        // a conversation's group link menu (menu_url_group.xml)
        super::chat::app_link_menu(&r, p, world, &format!("secondlife:///app/group/{group}/inspect"), &name);
        if r.clicked() {
            super::group_profile::request_open(ui.ctx(), group);
        }
    }

    /// The Edit tab (edit_panel_experience_info); true when the logo is
    /// clicked (its picker opens).
    fn edit_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
        d: &ExperienceDetails,
    ) -> bool {
        let id = d.id;
        let saving = world.experiences.saving.contains(&id);
        let here = world.back_slurl();
        let agent = world.agent_id;
        let mut groups: Vec<(Uuid, String)> = world.groups.groups.iter().map(|g| (g.id, g.name.clone())).collect();
        groups.sort_by_key(|(_, name)| name.to_lowercase());
        let Some((_, e)) = self.edit.as_mut() else {
            return false;
        };
        if !e.group.is_nil() && !groups.iter().any(|(g, _)| *g == e.group) {
            let name = world
                .land
                .group_name(&e.group, &world.groups.groups)
                .unwrap_or_else(|| e.group.to_string());
            groups.insert(0, (e.group, name));
        }
        let mut pick_logo = false;
        let mut save = None;
        let mut discard = None;
        let mut back = false;
        let mut cancel = false;
        let (mut body, foot) = widgets::above_footer(ui);
        egui::ScrollArea::vertical()
            .id_salt("experience_profile_edit")
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                // the logo: full-permission textures only
                if !e.logo.is_nil() && !images.contains_key(&e.logo) {
                    self.wanted_images.insert(e.logo);
                }
                let tex = images.get(&e.logo).filter(|_| !e.logo.is_nil());
                if widgets::picture(
                    ui,
                    p,
                    tex,
                    LOGO_SIZE,
                    "photo-filled",
                    aurora_i18n::tr!("ui-experience-profile-no-logo"),
                )
                .on_hover_text(aurora_i18n::tr!("ui-experience-profile-logo-tip"))
                .clicked()
                {
                    pick_logo = true;
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("ui-experience-profile-logo-rule"))
                            .size(11.5)
                            .color(p.muted),
                    )
                    .wrap(),
                );
                if self.logo_refused {
                    ui.add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("ui-experience-profile-logo-refused"))
                                .size(12.0)
                                .color(p.danger),
                        )
                        .wrap(),
                    );
                }
                ui.add_space(4.0);
                labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-name"), |ui| {
                    if ui
                        .add(egui::TextEdit::singleline(&mut e.name).desired_width(f32::INFINITY))
                        .changed()
                    {
                        crate::report_abuse::clamp_bytes(&mut e.name, NAME_BYTES);
                    }
                });
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-experience-profile-description"))
                        .size(12.0)
                        .color(p.muted),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut e.description)
                        .char_limit(DESCRIPTION_CHARS)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-rating"), |ui| {
                    egui::ComboBox::from_id_salt("experience_profile_rating")
                        .selected_text(crate::world::worldmap::access_label(e.maturity))
                        .show_ui(ui, |ui| {
                            for m in MATURITIES {
                                ui.selectable_value(&mut e.maturity, m, crate::world::worldmap::access_label(m));
                            }
                        })
                        .response
                        .on_hover_text(aurora_i18n::tr!("ui-experience-profile-rating-tip"));
                });
                labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-location"), |ui| {
                    let shown = if e.slurl.trim().is_empty() {
                        aurora_i18n::tr!("status-none").to_owned()
                    } else {
                        place_label(&e.slurl)
                    };
                    ui.add(egui::Label::new(RichText::new(shown).size(12.0).color(p.ink)).wrap());
                });
                ui.horizontal(|ui| {
                    ui.add_space(LABEL_WIDTH + ui.spacing().item_spacing.x);
                    let set = ui
                        .add_enabled_ui(here.is_some(), |ui| {
                            flat_button(ui, p, aurora_i18n::tr!("ui-experience-profile-set-location"))
                        })
                        .inner;
                    if set.clicked()
                        && let Some(here) = &here
                    {
                        e.slurl = here.clone();
                    }
                    let clear = ui
                        .add_enabled_ui(!e.slurl.is_empty(), |ui| flat_button(ui, p, aurora_i18n::tr!("common-clear")))
                        .inner;
                    if clear.clicked() {
                        e.slurl.clear();
                    }
                });
                // the Group button is the owner's only (refreshExperience)
                let owner = d.owner == agent;
                labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-group"), |ui| {
                    let current = groups
                        .iter()
                        .find(|(g, _)| *g == e.group)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| aurora_i18n::tr!("status-none").to_owned());
                    let combo = ui.add_enabled_ui(owner, |ui| {
                        egui::ComboBox::from_id_salt("experience_profile_group")
                            .selected_text(current)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut e.group, Uuid::nil(), aurora_i18n::tr!("status-none"));
                                for (g, name) in &groups {
                                    ui.selectable_value(&mut e.group, *g, name.as_str());
                                }
                            })
                    });
                    if !owner {
                        combo
                            .response
                            .on_disabled_hover_text(aurora_i18n::tr!("ui-experience-profile-group-owner-only"));
                    }
                });
                labelled(ui, p, aurora_i18n::tr!("ui-experience-profile-marketplace"), |ui| {
                    if ui
                        .add(egui::TextEdit::singleline(&mut e.marketplace).desired_width(f32::INFINITY))
                        .changed()
                    {
                        crate::report_abuse::clamp_bytes(&mut e.marketplace, MARKETPLACE_BYTES);
                    }
                });
                widgets::checkbox(ui, p, &mut e.enabled, aurora_i18n::tr!("ui-experience-profile-enabled"));
                widgets::checkbox(ui, p, &mut e.private, aurora_i18n::tr!("ui-experience-profile-private"));
                ui.add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("ui-experience-profile-changes-delay"))
                            .size(11.5)
                            .color(p.muted),
                    )
                    .wrap(),
                );
                save_notes(ui, p, world, id);
            });
        let dirty = e.dirty(d);
        let confirm = self.confirm;
        foot.show(ui, |ui| {
            if let Some(after) = confirm {
                match unsaved_bar(ui, p, saving) {
                    Some(Answer::Save) => save = Some(after),
                    Some(Answer::DontSave) => discard = Some(after),
                    Some(Answer::Cancel) => cancel = true,
                    None => {}
                }
            }
            ui.horizontal(|ui| {
                if action_button_if(
                    ui,
                    p,
                    "arrow-back-up",
                    aurora_i18n::tr!("common-back"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                    !saving,
                )
                .clicked()
                {
                    back = true;
                }
                let label = if saving {
                    aurora_i18n::tr!("ui-experience-profile-saving")
                } else {
                    aurora_i18n::tr!("common-save")
                };
                if action_button_if(
                    ui,
                    p,
                    "device-floppy-filled",
                    label,
                    ActionKind::Primary,
                    Vec2::new(0.0, 26.0),
                    dirty && !saving,
                )
                .clicked()
                {
                    save = Some(After::Stay);
                }
            });
        });
        // changeToView: Back with changes asks first
        if back {
            if dirty {
                self.confirm = Some(After::View);
            } else {
                discard = Some(After::View);
            }
        }
        if cancel {
            self.confirm = None;
        }
        if let Some(after) = save {
            self.save(world, after);
        } else if let Some(after) = discard {
            if after == After::Close {
                self.close(ui.ctx(), world);
            } else {
                self.leave_edit(ui.ctx());
            }
        }
        pick_logo
    }
}

/// The SaveChanges alert's buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Answer {
    Save,
    DontSave,
    Cancel,
}

/// "Save changes?" with Save, Don't Save and Cancel (the SaveChanges
/// alert, yesnocancelbuttons).
fn unsaved_bar(ui: &mut egui::Ui, p: &Palette, saving: bool) -> Option<Answer> {
    let mut answer = None;
    egui::Frame::new()
        .fill(p.warn.gamma_multiply(0.18))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-profile-there-are-unsaved-changes"))
                        .size(12.0)
                        .color(p.ink),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        answer = Some(Answer::Cancel);
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("ui-experience-profile-dont-save")).clicked() {
                        answer = Some(Answer::DontSave);
                    }
                    if ui
                        .add_enabled_ui(!saving, |ui| flat_button(ui, p, aurora_i18n::tr!("common-save")))
                        .inner
                        .clicked()
                    {
                        answer = Some(Answer::Save);
                    }
                });
            });
        });
    answer
}

/// onSaveComplete's alerts, under the profile: each field the server
/// ignored, or why the save failed.
fn save_notes(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid) {
    let Some(outcome) = world.experiences.saved.get(&id) else {
        return;
    };
    let lines: Vec<String> = match outcome {
        Ok(notes) => notes.iter().map(note_text).collect(),
        Err(e) => vec![aurora_i18n::tr!("ui-experience-profile-save-failed", error = e.clone())],
    };
    for line in lines {
        ui.add(egui::Label::new(RichText::new(line).size(12.0).color(p.danger)).wrap());
    }
}

/// The `<error_tag>ExperienceProfileMessage` notification of a field the
/// server ignored, else its English text (GenericAlert).
fn note_text(n: &aurora_net::experience::UpdateNote) -> String {
    match n.tag.as_str() {
        "NotInGroup" => aurora_i18n::tr!("ui-experience-profile-not-in-group").to_owned(),
        "Uneditable" => aurora_i18n::tr!("ui-experience-profile-uneditable", field = n.field.clone()),
        "RestrictedToOwner" => aurora_i18n::tr!("ui-experience-profile-owner-field", field = n.field.clone()),
        "MaturityRatingExceedsOwner" => aurora_i18n::tr!("ui-experience-profile-maturity-exceeds-owner").to_owned(),
        "RestrictedTerm" => aurora_i18n::tr!("ui-experience-profile-restricted-term", terms = n.extra.clone()),
        _ if !n.message.trim().is_empty() => n.message.clone(),
        _ => aurora_i18n::tr!("ui-experience-profile-ignored-field", field = n.field.clone()),
    }
}

/// "Region (x,y,z)" when the SLURL is a place (LLSLURL).
fn place_label(slurl: &str) -> String {
    crate::slurl::place_link(slurl).map(|l| l.label).unwrap_or_else(|| slurl.to_owned())
}

fn label_cell(ui: &mut egui::Ui, p: &Palette, label: &str) {
    ui.allocate_ui_with_layout(
        Vec2::new(LABEL_WIDTH, 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(LABEL_WIDTH);
            ui.label(RichText::new(label).size(12.0).color(p.muted));
        },
    );
}

fn row(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str) {
    ui.horizontal(|ui| {
        label_cell(ui, p, label);
        ui.add(
            egui::Label::new(RichText::new(value).size(12.0).color(p.ink))
                .wrap()
                .selectable(true),
        );
    });
}

fn labelled(ui: &mut egui::Ui, p: &Palette, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        label_cell(ui, p, label);
        add(ui);
    });
}

/// The body's margins, which the banner above it does without.
fn body(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 8,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// The banner across the window, in the Legacy Viewer's shape; its room
/// is kept while it loads, so the profile does not jump. `copy` is the
/// interface's small copy, `sharp` the full resolution: drawn over the
/// copy rather than in its place, so the frame a new texture may still miss
/// on its way to the GPU shows the copy instead of a blink, and only once
/// it is finer than the copy (the first levels of a download are coarser).
///
/// When the window leaves it less than its full height (`most`), the banner
/// shrinks whole, centred, rather than losing its top and bottom (where
/// banners often carry their text); the strips beside it show the same
/// banner cut to the strip, blurred and darkened. Returns where it was
/// drawn.
fn banner(ui: &mut egui::Ui, p: &Palette, copy: Option<&egui::TextureHandle>, sharp: Option<&egui::TextureHandle>, most: f32) -> Rect {
    let width = ui.available_width();
    let full = width / BANNER_ASPECT;
    let height = full.min(most).round();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let sharp = sharp.filter(|t| finer(t.size(), copy.map(|c| c.size())));
    let Some(best) = sharp.or(copy) else {
        ui.painter().rect_filled(rect, 0.0, p.bar);
        return rect;
    };
    let whole = fitted(rect);
    if whole.width() < rect.width() - 0.5 {
        backdrop(ui, p, rect, best);
    }
    let all = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if let Some(t) = copy {
        ui.painter().image(t.id(), whole, all, super::widgets::PICTURE_TINT);
    }
    if let Some(t) = sharp {
        ui.painter().image(t.id(), whole, all, super::widgets::PICTURE_TINT);
    }
    rect
}

/// The whole banner in `rect`, in its shape, centred: as wide as `rect`
/// when it has the banner's full height, narrower when it is shorter.
fn fitted(rect: Rect) -> Rect {
    let width = (rect.height() * BANNER_ASPECT).min(rect.width());
    Rect::from_center_size(rect.center(), Vec2::new(width, width / BANNER_ASPECT))
}

/// Behind a banner narrower than the window: the band of it that fills the
/// strip, blurred (shifted copies averaged, as the group profile blurs its
/// insignia across its banner) and darkened toward the window's colour.
fn backdrop(ui: &egui::Ui, p: &Palette, rect: Rect, texture: &egui::TextureHandle) {
    const STEPS: i32 = 4;
    /// How far the copies spread, in the banner's width.
    const REACH: f32 = 0.03;
    let band = banner_band(rect.height() / (rect.width() / BANNER_ASPECT));
    let painter = ui.painter_at(rect);
    let mut k = 0.0;
    for y in 0..STEPS {
        for x in 0..STEPS {
            let d = Vec2::new(x as f32, y as f32) / (STEPS - 1) as f32 * 2.0 - Vec2::splat(1.0);
            k += 1.0;
            painter.image(
                texture.id(),
                rect,
                band.translate(d * REACH),
                super::widgets::PICTURE_TINT.gamma_multiply(1.0 / k),
            );
        }
    }
    painter.rect_filled(rect, 0.0, p.panel.gamma_multiply(0.55));
}

/// Whether a decode of `size` pixels is finer than the copy (wider, or any
/// decode when there is no copy yet).
fn finer(size: [usize; 2], copy: Option<[usize; 2]>) -> bool {
    copy.is_none_or(|c| size[0] > c[0])
}

/// The part of the banner shown when only `share` of its height fits: a
/// band around its middle, as wide as the banner.
fn banner_band(share: f32) -> Rect {
    let h = share.clamp(0.0, 1.0);
    Rect::from_min_max(egui::pos2(0.0, 0.5 - h / 2.0), egui::pos2(1.0, 0.5 + h / 2.0))
}

fn value(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add(
        egui::Label::new(RichText::new(text).size(12.0).color(p.ink))
            .wrap()
            .selectable(true),
    );
}

/// A text in the link colour that answers a click.
fn link(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.add(
        egui::Label::new(RichText::new(text).size(12.0).color(p.violet_light))
            .truncate()
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The experience's place: "Region (x,y,z)" (LLSLURL), which shows it on
/// the world map; a conversation's place link menu on a right click
/// (menu_url_slurl.xml: details, map, teleport, Copy SLurl).
fn location(ui: &mut egui::Ui, p: &Palette, url: &str) {
    let place = crate::slurl::place_link(url);
    let label = place.as_ref().map_or_else(|| url.to_owned(), |l| l.label.clone());
    let r = link(ui, p, &label).on_hover_text(aurora_i18n::tr!("common-show-on-map"));
    match &place {
        Some(l) => super::chat::place_link_menu(&r, p, url, l),
        None => {
            menu::context_menu(&r, p, |ui| {
                if menu::item(ui, p, "link-filled", aurora_i18n::tr!("common-copy-slurl")) {
                    ui.ctx().copy_text(url.to_owned());
                }
            });
        }
    }
    if r.clicked()
        && let Some(l) = place
    {
        super::context::request(ui.ctx(), super::context::CtxAction::ShowPlace(l.location.target(), l.location.pos));
    }
}

/// The Marketplace link: opened like a link of a text (trusted at once,
/// else after the external link warning). A right click offers what the
/// Legacy Viewer's web links do (menu_url_http.xml): open it, open it in
/// the system browser, copy it.
fn web_link(ui: &mut egui::Ui, p: &Palette, url: &str) {
    let r = link(ui, p, url).on_hover_text(url);
    menu::context_menu(&r, p, |ui| {
        if menu::item(ui, p, "world-filled", aurora_i18n::tr!("ui-notifications-open-link")) {
            super::chat::open_web_link(ui.ctx(), url);
        }
        if menu::item(ui, p, "external-link-filled", aurora_i18n::tr!("search-web-open-external")) {
            super::context::request(ui.ctx(), super::context::CtxAction::OpenUrlExternal(url.to_owned()));
        }
        menu::separator(ui, p);
        if menu::item(ui, p, "copy-filled", aurora_i18n::tr!("common-copy-url")) {
            ui.ctx().copy_text(url.to_owned());
        }
    });
    if r.clicked() {
        super::chat::open_web_link(ui.ctx(), url);
    }
}

/// The footer's margins: the body's across, a little air above.
fn footer_body(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 2,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// The height of the main actions, and of the secondary ones.
const HERO_H: f32 = 32.0;
const SECONDARY_H: f32 = 26.0;

/// The agent's permission and what changes it, always in view: Allow and
/// Forget, the two answers to an experience, side by side across the
/// window; Edit (for an administrator), Block and Report Abuse smaller
/// under them; true when Edit was pressed. As in
/// experiencePermissionResults, the button of the current permission is
/// the one that cannot be pressed.
fn actions(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid, admin: bool) -> (Option<ExperiencePermissionChange>, bool) {
    let permission = world.experiences.permissions.get(&id).copied();
    let busy = world.experiences.busy.contains(&id);
    let state = match (busy, permission) {
        (true, _) | (false, None) => aurora_i18n::tr!("ui-experience-profile-checking"),
        (false, Some(ExperiencePermission::Allowed)) => aurora_i18n::tr!("ui-experience-profile-allowed"),
        (false, Some(ExperiencePermission::Blocked)) => aurora_i18n::tr!("ui-experience-profile-blocked"),
        (false, Some(ExperiencePermission::Unset)) => aurora_i18n::tr!("ui-experience-profile-unset"),
    };
    super::widgets::divider(ui, p);
    row(ui, p, aurora_i18n::tr!("ui-experience-profile-permission"), state);
    if let Some(e) = world.experiences.errors.get(&id) {
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("ui-experience-profile-error", error = e.clone()))
                    .size(12.0)
                    .color(p.danger),
            )
            .wrap(),
        );
    }
    ui.add_space(6.0);
    let ready = !busy && permission.is_some();
    let mut change = None;
    ui.horizontal(|ui| {
        let half = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(0.0);
        if action_button_if(
            ui,
            p,
            "circle-check-filled",
            aurora_i18n::tr!("ui-experience-profile-allow"),
            ActionKind::Primary,
            Vec2::new(half, HERO_H),
            ready && permission != Some(ExperiencePermission::Allowed),
        )
        .clicked()
        {
            change = Some(ExperiencePermissionChange::Allow);
        }
        if action_button_if(
            ui,
            p,
            "restore",
            aurora_i18n::tr!("ui-experience-profile-forget"),
            ActionKind::Flat,
            Vec2::new(half, HERO_H),
            ready && permission != Some(ExperiencePermission::Unset),
        )
        .on_hover_text(aurora_i18n::tr!("ui-experience-profile-forget-tip"))
        .clicked()
        {
            change = Some(ExperiencePermissionChange::Forget);
        }
    });
    let mut edit = false;
    ui.horizontal(|ui| {
        if admin
            && action_button_if(
                ui,
                p,
                "pencil-filled",
                aurora_i18n::tr!("common-edit"),
                ActionKind::Flat,
                Vec2::new(0.0, SECONDARY_H),
                true,
            )
            .clicked()
        {
            edit = true;
        }
        if action_button_if(
            ui,
            p,
            "ban",
            aurora_i18n::tr!("ui-experience-profile-block"),
            ActionKind::FlatDanger,
            Vec2::new(0.0, SECONDARY_H),
            ready && permission != Some(ExperiencePermission::Blocked),
        )
        .clicked()
        {
            change = Some(ExperiencePermissionChange::Block);
        }
        // LLFloaterExperienceProfile::onReportExperience: Report Abuse on
        // the experience (LLFloaterReporter::showFromExperience)
        if action_button_if(
            ui,
            p,
            "flag-filled",
            aurora_i18n::tr!("report-abuse-title"),
            ActionKind::Flat,
            Vec2::new(0.0, SECONDARY_H),
            true,
        )
        .clicked()
        {
            super::context::request(
                ui.ctx(),
                super::context::CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Experience(id)),
            );
        }
    });
    (change, edit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slurl_opens_the_profile() {
        let id = Uuid::from_u128(0x1234);
        let url = slurl(id);
        assert_eq!(url, format!("secondlife:///app/experience/{id}/profile"));
        assert!(matches!(crate::slapp::parse(&url), Some(crate::slapp::AppLink::Experience(e)) if e == id));
    }

    #[test]
    fn a_short_window_shows_the_whole_banner_smaller() {
        // the full height: the banner spans the window
        let full = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(500.0, 500.0 / BANNER_ASPECT));
        let whole = fitted(full);
        assert!((whole.width() - 500.0).abs() < 1e-3 && (whole.height() - full.height()).abs() < 1e-3);
        // half the height: the whole banner, half as wide, centred
        let short = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(500.0, 150.0));
        let whole = fitted(short);
        assert!((whole.height() - 150.0).abs() < 1e-3);
        assert!((whole.width() - 150.0 * BANNER_ASPECT).abs() < 1e-3);
        assert!((whole.center().x - 250.0).abs() < 1e-3);
        // the strips behind show a band of the banner in their shape
        assert_eq!(banner_band(1.0), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)));
        let band = banner_band(0.5);
        assert_eq!((band.left(), band.right(), band.top(), band.bottom()), (0.0, 1.0, 0.25, 0.75));
    }

    #[test]
    fn the_sharp_banner_covers_the_copy_only_once_finer() {
        assert!(finer([512, 256], Some([256, 128])));
        assert!(!finer([128, 64], Some([256, 128])));
        assert!(!finer([256, 128], Some([256, 128])));
        assert!(finer([64, 32], None));
    }

    #[test]
    fn the_banner_has_the_legacy_shape() {
        assert!((BANNER_ASPECT - 1.6667).abs() < 0.001);
    }
}
