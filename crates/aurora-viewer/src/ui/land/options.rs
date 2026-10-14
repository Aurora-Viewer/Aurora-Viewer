//! "Options" (LLPanelLandOptions::refresh / refreshSearch / onCommitAny /
//! onClickSet / onClickClear, FIRE-10043 teleport to the landing point,
//! FIRE-6604 terraform checkbox).

use super::general::set;
use super::{Dialog, LandUi, View, check, dim, text};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{MIN_PARCEL_AREA, powers};
use aurora_net::{ParcelInfo, parcel_flags as pf, region_flags as rf};
use egui::{RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// PARCEL_DIRECTORY_FEE (L$ per week).
pub(super) const DIRECTORY_FEE: i32 = 30;

/// Categories of the combo, in its order (LLParcel::ECategory, French skin).
pub(super) fn categories() -> [(i32, &'static str); 13] {
    [
        (0, aurora_i18n::tr!("ui-land-options-all-categories")),
        (1, aurora_i18n::tr!("ui-land-options-linden-owned")),
        (3, aurora_i18n::tr!("ui-land-options-arts-and-culture")),
        (4, aurora_i18n::tr!("ui-land-options-business")),
        (5, aurora_i18n::tr!("ui-land-options-education")),
        (6, aurora_i18n::tr!("ui-land-options-games")),
        (7, aurora_i18n::tr!("common-favorites")),
        (8, aurora_i18n::tr!("ui-land-options-newcomer-friendly")),
        (9, aurora_i18n::tr!("ui-land-options-parks-and-nature")),
        (10, aurora_i18n::tr!("ui-land-options-residential")),
        (11, aurora_i18n::tr!("common-shopping-2")),
        (14, aurora_i18n::tr!("extra-location")),
        (13, aurora_i18n::tr!("ui-land-options-other")),
    ]
}

/// The name of a category, none for "any category" (C_NONE).
pub(super) fn category_name(category: i32) -> Option<&'static str> {
    categories().into_iter().find(|c| c.0 == category && c.0 != 0).map(|c| c.1)
}

/// Teleport routing (LLParcel::ELandingType).
fn landing_types() -> [&'static str; 3] {
    [
        aurora_i18n::tr!("common-blocked"),
        aurora_i18n::tr!("ui-land-options-fixed-landing-point"),
        aurora_i18n::tr!("ui-land-options-unrestricted-landing"),
    ]
}

/// The options as the panel shows them: group boxes are checked when
/// "everyone" is (refresh), and onCommitAny derives the flags back.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Shown {
    pub edit_objects: bool,
    pub edit_group: bool,
    pub all_entry: bool,
    pub group_entry: bool,
    pub terraform: bool,
    pub safe: bool,
    pub fly: bool,
    pub group_scripts: bool,
    pub other_scripts: bool,
    pub push: bool,
    pub see_avs: bool,
    pub directory: bool,
    pub mature: bool,
}

/// What the maturity box shows for the region's rating: never for General,
/// by choice for Moderate, always for Adult.
pub(super) fn mature_value(parcel: &ParcelInfo, sim_access: u8) -> bool {
    match sim_access {
        21 => parcel.flags & pf::MATURE_PUBLISH != 0,
        42 => true,
        _ => false,
    }
}

/// refreshSearch: small parcels stay out of search unless already listed;
/// whether the search box is enabled and its tooltip.
pub(super) fn search_state(v: &View, parcel: &ParcelInfo, listed: bool) -> (bool, &'static str) {
    let can_search = v.can(powers::LAND_FIND_PLACES) && !v.region_flag(rf::BLOCK_PARCEL_SEARCH);
    let large = parcel.area >= MIN_PARCEL_AREA;
    match (large, can_search) {
        (true, true) => (
            true,
            aurora_i18n::tr!("ui-land-options-let-other-residents-find-this-parcel-in-search-results"),
        ),
        (false, true) if listed => (
            true,
            aurora_i18n::tr!("ui-land-options-let-other-residents-find-this-parcel-in-search-results"),
        ),
        (false, true) => (
            false,
            aurora_i18n::tr!("ui-land-options-this-option-is-disabled-because-the-parcel-is-128"),
        ),
        _ => (
            false,
            aurora_i18n::tr!("ui-land-options-this-option-is-disabled-because-you-cannot-change-this"),
        ),
    }
}

/// A change of the parcel besides its flags (category, routing).
pub(super) type Extra = Option<Box<dyn FnOnce(&mut aurora_net::land::ParcelUpdate)>>;

/// onCommitAny: send what changed, unless scripts would be turned off in a
/// region that allows damage (the panel refuses it with a message).
pub(super) fn commit(s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, before: Shown, now: Shown, extra: Extra) {
    let changed = before.flags(parcel.flags) != now.flags(parcel.flags) || before.see_avs != now.see_avs;
    if !changed && extra.is_none() {
        return;
    }
    let region_damage = v.region_flag(rf::ALLOW_DAMAGE);
    let scripts_off = (!now.other_scripts && parcel.flags & pf::ALLOW_OTHER_SCRIPTS != 0)
        || (!(now.group_scripts || now.other_scripts) && parcel.flags & pf::ALLOW_GROUP_SCRIPTS != 0);
    if region_damage && scripts_off {
        s.dialog = Some(Dialog::Message(
            aurora_i18n::tr!("ui-land-options-cannot-disable-scripts-damage-is-enabled-throughout-the-region").into(),
        ));
        return;
    }
    let flags = now.flags(parcel.flags);
    let see = now.see_avs;
    world.land.update(|u| {
        u.flags = flags;
        u.see_avs = see;
        if let Some(f) = extra {
            f(u);
        }
    });
}

impl Shown {
    pub(super) fn of(p: &ParcelInfo, region_push: bool, mature: bool) -> Shown {
        let f = |b| p.flags & b != 0;
        Shown {
            edit_objects: f(pf::CREATE_OBJECTS),
            edit_group: f(pf::CREATE_GROUP_OBJECTS) || f(pf::CREATE_OBJECTS),
            all_entry: f(pf::ALLOW_ALL_OBJECT_ENTRY),
            group_entry: f(pf::ALLOW_GROUP_OBJECT_ENTRY) || f(pf::ALLOW_ALL_OBJECT_ENTRY),
            terraform: f(pf::ALLOW_TERRAFORM),
            safe: !f(pf::ALLOW_DAMAGE),
            fly: f(pf::ALLOW_FLY),
            group_scripts: f(pf::ALLOW_GROUP_SCRIPTS) || f(pf::ALLOW_OTHER_SCRIPTS),
            other_scripts: f(pf::ALLOW_OTHER_SCRIPTS),
            // a region override shows (and sends) the restriction
            push: f(pf::RESTRICT_PUSHOBJECT) || region_push,
            see_avs: p.see_avatars,
            directory: f(pf::SHOW_DIRECTORY),
            mature,
        }
    }

    /// onCommitAny: the parcel flags of what is shown.
    pub(super) fn flags(&self, old: u32) -> u32 {
        let mut f = old;
        f = set(f, pf::CREATE_OBJECTS, self.edit_objects);
        f = set(f, pf::CREATE_GROUP_OBJECTS, self.edit_group || self.edit_objects);
        f = set(f, pf::ALLOW_ALL_OBJECT_ENTRY, self.all_entry);
        f = set(f, pf::ALLOW_GROUP_OBJECT_ENTRY, self.group_entry || self.all_entry);
        f = set(f, pf::ALLOW_TERRAFORM, self.terraform);
        f = set(f, pf::ALLOW_DAMAGE, !self.safe);
        f = set(f, pf::ALLOW_FLY, self.fly);
        // landmarks cannot be restricted
        f = set(f, pf::ALLOW_LANDMARK, true);
        f = set(f, pf::ALLOW_GROUP_SCRIPTS, self.group_scripts || self.other_scripts);
        f = set(f, pf::ALLOW_OTHER_SCRIPTS, self.other_scripts);
        f = set(f, pf::SHOW_DIRECTORY, self.directory);
        f = set(f, pf::ALLOW_PUBLISH, false);
        f = set(f, pf::MATURE_PUBLISH, self.mature);
        set(f, pf::RESTRICT_PUSHOBJECT, self.push)
    }
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return;
    };
    let can_options = v.can(powers::LAND_OPTIONS);
    let can_terraform = v.can(powers::LAND_EDIT);
    let can_identity = v.can(powers::LAND_CHANGE_IDENTITY);
    let can_landing = v.can(powers::LAND_SET_LANDING_POINT);
    let sim_access = v.region.as_ref().map_or(0, |r| r.sim_access);
    let before = Shown::of(&parcel, parcel.region_push_override, mature_value(&parcel, sim_access));
    let mut now = before;
    let mut extra: Extra = None;

    ui.label(
        RichText::new(aurora_i18n::tr!("ui-land-options-allow-other-residents-to"))
            .size(12.0)
            .color(p.ink),
    );
    let line = |ui: &mut egui::Ui, label: &str, body: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            super::fixed(ui, 150.0, |ui| {
                ui.label(RichText::new(label).size(12.0).color(p.muted));
            });
            body(ui);
        });
    };
    let col = |ui: &mut egui::Ui, w: f32, body: &mut dyn FnMut(&mut egui::Ui)| {
        super::fixed(ui, w, body);
    };
    line(ui, aurora_i18n::tr!("ui-land-options-edit-land"), &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, p, can_terraform, now.terraform, aurora_i18n::tr!("common-all"));
            r.on_hover_text(aurora_i18n::tr!(
                "ui-land-options-when-checked-everyone-can-terraform-your-land-it-is"
            ));
            now.terraform = on;
        });
        dim(ui, p, aurora_i18n::tr!("ui-land-options-use-with-care"));
    });
    line(ui, aurora_i18n::tr!("ui-land-options-fly"), &mut |ui| {
        let (r, on) = check(ui, p, can_options, now.fly, aurora_i18n::tr!("common-all"));
        r.on_hover_text(aurora_i18n::tr!(
            "ui-land-options-when-checked-residents-can-fly-on-your-land-otherwise"
        ));
        now.fly = on;
    });
    line(ui, aurora_i18n::tr!("ui-land-options-build"), &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, p, can_options, now.edit_objects, aurora_i18n::tr!("common-all"));
            r.on_hover_text(aurora_i18n::tr!(
                "ui-land-options-when-checked-residents-can-create-and-rez-objects-on"
            ));
            now.edit_objects = on;
        });
        // explicitly on (and greyed) while everyone may build
        let (r, on) = check(
            ui,
            p,
            can_options && !before.edit_objects,
            now.edit_group,
            aurora_i18n::tr!("common-group"),
        );
        r.on_hover_text(aurora_i18n::tr!(
            "ui-land-options-when-checked-parcel-group-members-can-create-and-rez"
        ));
        now.edit_group = on;
    });
    line(ui, aurora_i18n::tr!("ui-land-options-allow-object-entry"), &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, p, can_options, now.all_entry, aurora_i18n::tr!("common-all"));
            r.on_hover_text(aurora_i18n::tr!(
                "ui-land-options-when-checked-residents-can-move-their-objects-from-other"
            ));
            now.all_entry = on;
        });
        let (r, on) = check(
            ui,
            p,
            can_options && !before.all_entry,
            now.group_entry,
            aurora_i18n::tr!("common-group"),
        );
        r.on_hover_text(aurora_i18n::tr!(
            "ui-land-options-when-checked-parcel-group-members-can-move-their-objects"
        ));
        now.group_entry = on;
    });
    line(ui, aurora_i18n::tr!("ui-land-options-run-scripts"), &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, p, can_options, now.other_scripts, aurora_i18n::tr!("common-all"));
            r.on_hover_text(aurora_i18n::tr!(
                "ui-land-options-when-checked-residents-can-run-scripts-on-your-parcel"
            ));
            now.other_scripts = on;
        });
        let (r, on) = check(
            ui,
            p,
            can_options && !before.other_scripts,
            now.group_scripts,
            aurora_i18n::tr!("common-group"),
        );
        r.on_hover_text(aurora_i18n::tr!(
            "ui-land-options-when-checked-parcel-group-members-can-run-scripts-on"
        ));
        now.group_scripts = on;
    });
    crate::ui::widgets::divider(ui, p);
    let (search_on, search_tip) = search_state(v, &parcel, before.directory);
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        let (r, on) = check(ui, p, can_options, now.safe, aurora_i18n::tr!("ui-land-options-safe-no-damage"));
        r.on_hover_text(aurora_i18n::tr!("ui-land-options-when-checked-the-land-is-safe-no-combat-damage"));
        now.safe = on;
        let label = if DIRECTORY_FEE == 0 {
            aurora_i18n::tr!("ui-land-options-place-in-search").to_owned()
        } else {
            aurora_i18n::tr!("extra-show-place-in-search-l-week", DIRECTORY_FEE = format!("{}", DIRECTORY_FEE))
        };
        let (r, on) = check(ui, p, search_on, now.directory, &label);
        r.on_hover_text(search_tip).on_disabled_hover_text(search_tip);
        now.directory = on;
        // maturity: PG never, mature by choice, adult always
        if sim_access == 21 || sim_access == 42 || sim_access == 13 {
            ui.horizontal(|ui| {
                let adult = sim_access == 42;
                let (r, on) = check(ui, p, sim_access == 21 && can_identity, now.mature, "");
                now.mature = on;
                super::widgets::maturity_badge(
                    ui,
                    p,
                    if adult {
                        aurora_i18n::tr!("common-adult")
                    } else {
                        aurora_i18n::tr!("common-moderate")
                    },
                    14.0,
                );
                text(
                    ui,
                    p,
                    if adult {
                        aurora_i18n::tr!("ui-land-options-adult-content")
                    } else {
                        aurora_i18n::tr!("ui-land-options-moderate-content")
                    },
                );
                r.on_hover_text(if adult {
                    aurora_i18n::tr!("ui-land-options-your-parcel-information-or-contents-are-rated-adult")
                } else {
                    aurora_i18n::tr!("ui-land-options-your-parcel-information-or-contents-are-rated-moderate")
                });
            });
        }
        let current = categories()
            .iter()
            .find(|c| c.0 == parcel.category)
            .map_or(aurora_i18n::tr!("status-other"), |c| c.1);
        ui.add_enabled_ui(search_on, |ui| {
            egui::ComboBox::from_id_salt("land_category")
                .selected_text(current)
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (id, label) in categories() {
                        if ui.selectable_label(id == parcel.category, label).clicked() && id != parcel.category {
                            extra = Some(Box::new(move |u| u.category = id as u8));
                        }
                    }
                })
                .response
                .on_hover_text(search_tip)
                .on_disabled_hover_text(search_tip);
        });

        let ui = &mut cols[1];
        let push_label = if parcel.region_push_override {
            aurora_i18n::tr!("ui-land-options-no-pushing-region-rules-take-precedence")
        } else {
            aurora_i18n::tr!("ui-land-options-no-pushing")
        };
        let (r, on) = check(ui, p, can_options && !parcel.region_push_override, now.push, push_label);
        r.on_hover_text(aurora_i18n::tr!(
            "ui-land-options-prevents-scripts-from-pushing-people-useful-for-preventing-abusive"
        ));
        now.push = on;
        ui.horizontal_top(|ui| {
            // the server must know SeeAVs (getHaveNewParcelLimitData)
            let enabled = can_options && parcel.have_new_parcel_limit_data;
            let (r, on) = check(ui, p, enabled, now.see_avs, "");
            r.on_hover_text(aurora_i18n::tr!("ui-land-options-allow-avatars-on-other-parcels-to-see-and-chat"));
            now.see_avs = on;
            // toggleSeeAvatars: the label toggles the box too
            let l = ui.add(
                egui::Label::new(
                    RichText::new(aurora_i18n::tr!("ui-land-options-avatars-on-other-parcels-can-see-and-chat-with"))
                        .size(12.0)
                        .color(if enabled { p.ink } else { p.muted }),
                )
                .wrap()
                .sense(egui::Sense::click()),
            );
            if enabled && l.clicked() {
                now.see_avs = !now.see_avs;
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("ui-land-options-picture")).size(12.0).color(p.muted));
            let size = Vec2::new(195.0, 150.0);
            let (rect, r) = ui.allocate_exact_size(size, if can_identity { egui::Sense::click() } else { egui::Sense::hover() });
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
            if parcel.snapshot_id.is_nil() {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    aurora_i18n::tr!("status-none-2"),
                    egui::FontId::proportional(12.0),
                    p.muted,
                );
            } else {
                s.wanted_images.insert(parcel.snapshot_id);
                match images.get(&parcel.snapshot_id) {
                    Some(t) => {
                        ui.painter().image(
                            t.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            super::super::widgets::PICTURE_TINT,
                        );
                    }
                    None => {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            aurora_i18n::tr!("common-loading-2"),
                            egui::FontId::proportional(12.0),
                            p.muted,
                        );
                    }
                }
            }
            if r.on_hover_text(aurora_i18n::tr!("common-click-to-select-an-image")).clicked() {
                s.texture_snapshot = true;
                s.texture.open(aurora_i18n::tr!("ui-land-options-land-picture"), parcel.snapshot_id);
            }
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.add_space(60.0);
            let lp = parcel.user_location;
            let has_lp = lp != glam::Vec3::ZERO;
            let landing = if has_lp {
                let look = parcel.user_look_at;
                let deg = ((look.y.atan2(-look.x) + std::f32::consts::TAU).to_degrees() + 0.5) as u32;
                format!("{}, {}, {} ({}°)", lp.x.round(), lp.y.round(), lp.z.round(), (deg - 90) % 360)
            } else {
                aurora_i18n::tr!("status-none").to_owned()
            };
            text(
                ui,
                p,
                aurora_i18n::tr!("ui-land-options-landing-point", landing = landing.to_string()),
            );
            ui.horizontal(|ui| {
                if super::button(ui, p, aurora_i18n::tr!("ui-land-options-set"), can_landing)
                    .on_hover_text(aurora_i18n::tr!(
                        "ui-land-options-sets-the-landing-point-for-visitors-to-your-avatar"
                    ))
                    .clicked()
                {
                    // onClickSet: only from inside the parcel
                    let inside = v.same_region && world.parcel.as_ref().is_some_and(|a| a.local_id == parcel.local_id);
                    if inside {
                        let (pos, at) = (world.agent.position, world.agent.forward());
                        world.land.update(|u| {
                            u.user_location = pos;
                            u.user_look_at = at;
                        });
                    } else {
                        s.dialog = Some(Dialog::Message(
                            aurora_i18n::tr!("ui-land-options-you-must-be-inside-the-parcel-to-set-its").into(),
                        ));
                    }
                }
                if super::button(ui, p, aurora_i18n::tr!("common-cancel"), can_landing)
                    .on_hover_text(aurora_i18n::tr!("ui-land-options-clear-landing-point"))
                    .clicked()
                {
                    world.land.update(|u| {
                        u.user_location = glam::Vec3::ZERO;
                        u.user_look_at = glam::Vec3::ZERO;
                    });
                }
                if super::button(ui, p, aurora_i18n::tr!("ui-land-options-teleport"), has_lp)
                    .on_hover_text(aurora_i18n::tr!("ui-land-options-teleport-to-landing-point"))
                    .clicked()
                {
                    let (ox, oy) = aurora_net::handle_to_origin(v.handle);
                    world
                        .map
                        .track_location(ox as f64 + lp.x as f64, oy as f64 + lp.y as f64, lp.z, true);
                }
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-land-options-teleport-routing"))
                    .size(12.0)
                    .color(p.muted),
            );
            let current = landing_types().get(parcel.landing_type as usize).copied().unwrap_or("");
            ui.add_enabled_ui(can_landing, |ui| {
                egui::ComboBox::from_id_salt("landing_type")
                    .selected_text(current)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (i, label) in landing_types().iter().enumerate() {
                            if ui.selectable_label(i as i32 == parcel.landing_type, *label).clicked() && i as i32 != parcel.landing_type {
                                extra = Some(Box::new(move |u| u.landing_type = i as u8));
                            }
                        }
                    })
                    .response
                    .on_hover_text(aurora_i18n::tr!(
                        "ui-land-options-teleport-routing-choose-how-teleports-to-your-land-are"
                    ));
            });
        });
    });

    commit(s, v, world, &parcel, before, now, extra);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyone_implies_group_like_on_commit_any() {
        let p = ParcelInfo {
            flags: pf::CREATE_OBJECTS | pf::ALLOW_OTHER_SCRIPTS,
            ..Default::default()
        };
        let shown = Shown::of(&p, false, false);
        assert!(shown.edit_group && shown.group_scripts);
        let f = shown.flags(p.flags);
        assert!(f & pf::CREATE_GROUP_OBJECTS != 0);
        assert!(f & pf::ALLOW_GROUP_SCRIPTS != 0);
        assert!(f & pf::ALLOW_LANDMARK != 0);
        // turning "everyone" off keeps group (it was shown checked)
        let mut off = shown;
        off.edit_objects = false;
        let f = off.flags(p.flags);
        assert!(f & pf::CREATE_OBJECTS == 0 && f & pf::CREATE_GROUP_OBJECTS != 0);
    }

    #[test]
    fn region_push_override_is_sent() {
        let p = ParcelInfo::default();
        let shown = Shown::of(&p, true, false);
        assert!(shown.flags(0) & pf::RESTRICT_PUSHOBJECT != 0);
        // damage: "safe" is the opposite flag
        assert!(shown.flags(0) & pf::ALLOW_DAMAGE == 0);
    }
}
