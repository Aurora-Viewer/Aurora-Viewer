//! Experience profile: what the experience is and the agent's permission
//! for it, with Firestorm's Allow / Forget / Block buttons
//! (LLFloaterExperienceProfile, indra/newview/llfloaterexperienceprofile.cpp,
//! originally LGPL 2.1): the logo, name and description, rating, scope,
//! location, owner, group and marketplace address, and "Report Abuse". An
//! administrator also edits it (the Edit tab, experienceIsAdmin): name,
//! description, rating, location, group (the owner only), marketplace
//! address, logo, "Enable Experience" and "Hide In Search", saved with
//! UpdateExperience; closing or going back with changes asks first.

use super::texture_picker::TexturePicker;
use super::widgets::{self, ActionKind, Floater, action_button_if, flat_button};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::experience::{
    ExperienceDetails, ExperienceEdit, ExperiencePermission, ExperiencePermissionChange, MATURITIES, update_body,
};
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const WINDOW: &str = "experience_profile";
const VIEW_SIZE: Vec2 = Vec2::new(380.0, 340.0);
const EDIT_SIZE: Vec2 = Vec2::new(420.0, 600.0);
const LABEL_WIDTH: f32 = 110.0;
const LOGO_SIZE: Vec2 = Vec2::new(180.0, 122.0);
/// floater_experienceprofile.xml: the name's and the marketplace address's
/// byte limits, the description's character limit.
const NAME_BYTES: usize = 63;
const MARKETPLACE_BYTES: usize = 255;
const DESCRIPTION_CHARS: usize = 2048;

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
}

impl ExperienceProfileUi {
    /// Shows the profile of `world.experiences.profile`; returns the
    /// permission change asked for.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> Option<(Uuid, ExperiencePermissionChange)> {
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
        Floater::new(
            WINDOW,
            aurora_i18n::tr!("ui-experience-profile-title"),
            egui::pos2(screen.center().x - VIEW_SIZE.x * 0.5, screen.center().y - VIEW_SIZE.y * 0.5),
            VIEW_SIZE,
        )
        .show(ctx, p, &mut open, |ui| {
            let Some(d) = details.as_ref() else {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-experience-profile-loading"))
                        .size(12.0)
                        .color(p.muted),
                );
                return;
            };
            if self.edit.is_some() {
                pick_logo = self.edit_tab(ui, p, world, images, d);
            } else {
                change = self.view_tab(ui, p, world, images, d, admin);
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

    fn view_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
        d: &ExperienceDetails,
        admin: bool,
    ) -> Option<ExperiencePermissionChange> {
        let id = d.id;
        let mut change = None;
        let permission = world.experiences.permissions.get(&id).copied();
        let busy = world.experiences.busy.contains(&id);
        let error = world.experiences.errors.get(&id).cloned();
        let owner = (!d.owner.is_nil()).then_some(d.owner);
        let owner_name = owner.map(|o| {
            let names = &world.social.avatar_names;
            names.want(&o);
            names.get(&o).map(|n| n.display_name.clone()).unwrap_or_else(|| o.to_string())
        });
        let group_name = (!d.group.is_nil()).then(|| {
            world
                .land
                .group_name(&d.group, &world.groups.groups)
                .unwrap_or_else(|| d.group.to_string())
        });
        egui::ScrollArea::vertical()
            .id_salt("experience_profile_view")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // the logo, when the experience has one
                if !d.logo.is_nil() {
                    let tex = images.get(&d.logo);
                    if tex.is_none() {
                        self.wanted_images.insert(d.logo);
                    }
                    widgets::picture(ui, p, tex, LOGO_SIZE, "photo-filled", "");
                    ui.add_space(6.0);
                }
                let name = if d.name.trim().is_empty() { id.to_string() } else { d.name.clone() };
                ui.label(RichText::new(name).size(15.0).strong().color(p.ink));
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
                    row(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-experience-profile-rating"),
                        crate::world::worldmap::access_label(d.maturity),
                    );
                    let scope = if d.grid_wide() {
                        aurora_i18n::tr!("world-experiences-grid-scope")
                    } else {
                        aurora_i18n::tr!("world-experiences-land-scope")
                    };
                    row(ui, p, aurora_i18n::tr!("ui-experience-profile-scope"), scope);
                }
                if !d.slurl.trim().is_empty() {
                    row(ui, p, aurora_i18n::tr!("ui-experience-profile-location"), &place_label(&d.slurl));
                }
                if let Some(owner) = &owner_name {
                    row(ui, p, aurora_i18n::tr!("ui-experience-profile-owner"), owner);
                }
                if let Some(group) = &group_name {
                    row(ui, p, aurora_i18n::tr!("ui-experience-profile-group"), group);
                }
                if !d.marketplace.trim().is_empty() {
                    row(ui, p, aurora_i18n::tr!("ui-experience-profile-marketplace"), &d.marketplace);
                }
                ui.add_space(10.0);
                let state = match (busy, permission) {
                    (true, _) | (false, None) => aurora_i18n::tr!("ui-experience-profile-checking"),
                    (false, Some(ExperiencePermission::Allowed)) => aurora_i18n::tr!("ui-experience-profile-allowed"),
                    (false, Some(ExperiencePermission::Blocked)) => aurora_i18n::tr!("ui-experience-profile-blocked"),
                    (false, Some(ExperiencePermission::Unset)) => aurora_i18n::tr!("ui-experience-profile-unset"),
                };
                row(ui, p, aurora_i18n::tr!("ui-experience-profile-permission"), state);
                if let Some(e) = &error {
                    ui.add(
                        egui::Label::new(
                            RichText::new(aurora_i18n::tr!("ui-experience-profile-error", error = e.clone()))
                                .size(12.0)
                                .color(p.danger),
                        )
                        .wrap(),
                    );
                }
                save_notes(ui, p, world, id);
                ui.add_space(8.0);
                // experiencePermissionResults: the button of the current
                // permission is the one that cannot be pressed
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    let ready = !busy && permission.is_some();
                    if action_button_if(
                        ui,
                        p,
                        "circle-check-filled",
                        aurora_i18n::tr!("ui-experience-profile-allow"),
                        ActionKind::Primary,
                        size,
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
                        size,
                        ready && permission != Some(ExperiencePermission::Unset),
                    )
                    .on_hover_text(aurora_i18n::tr!("ui-experience-profile-forget-tip"))
                    .clicked()
                    {
                        change = Some(ExperiencePermissionChange::Forget);
                    }
                    if action_button_if(
                        ui,
                        p,
                        "ban",
                        aurora_i18n::tr!("ui-experience-profile-block"),
                        ActionKind::FlatDanger,
                        size,
                        ready && permission != Some(ExperiencePermission::Blocked),
                    )
                    .clicked()
                    {
                        change = Some(ExperiencePermissionChange::Block);
                    }
                });
                // LLFloaterExperienceProfile::onReportExperience: Report
                // Abuse on the experience (LLFloaterReporter::
                // showFromExperience); on its own row with Edit, the window
                // is too narrow for a fourth button
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if admin
                        && action_button_if(
                            ui,
                            p,
                            "pencil-filled",
                            aurora_i18n::tr!("common-edit"),
                            ActionKind::Flat,
                            Vec2::new(0.0, 26.0),
                            true,
                        )
                        .clicked()
                    {
                        self.enter_edit(ui.ctx(), id, d);
                    }
                    if action_button_if(
                        ui,
                        p,
                        "flag-filled",
                        aurora_i18n::tr!("report-abuse-title"),
                        ActionKind::Flat,
                        Vec2::new(0.0, 26.0),
                        true,
                    )
                    .clicked()
                    {
                        crate::ui::context::request(
                            ui.ctx(),
                            crate::ui::context::CtxAction::ReportAbuse(crate::report_abuse::ReportTarget::Experience(id)),
                        );
                    }
                });
            });
        change
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
