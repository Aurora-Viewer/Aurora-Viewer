//! "Manage" of the Aurora About Land (`aurora.rs`): every setting of
//! Firestorm's General, Options, Objects, Media, Sound, Access, Experiences
//! and Environment tabs, grouped by what one wants to do, each group with a
//! sentence that says what it is for and its rarer options folded. The rules
//! and the messages are the panels' (LLPanelLandGeneral, LLPanelLandOptions,
//! LLPanelLandObjects, LLPanelLandMedia, LLPanelLandAudio, LLPanelLandAccess,
//! LLPanelLandExperiences, LLPanelLandEnvironment): the same state and commit
//! code as the classic panes (`options::Shown`, `access::AccessState`…).
//! What the agent may not change is shown greyed, with the power it lacks.

use super::access::AccessState;
use super::aurora::semibold;
use super::dialogs::{Confirm, SellForm};
use super::options::{self, Shown};
use super::{Dialog, LandUi, View};
use crate::theme::Palette;
use crate::ui::sheet::{self, editable, fold_link, fold_row, line, lock_line, srow, why};
use crate::ui::{menu, widgets};

/// The cards and the banner the other pages share.
pub(super) use crate::ui::sheet::{card_begin, card_end, lock_bar};
use crate::world::World;
use crate::world::land::{EnvState, PrimUsage, powers};
use aurora_net::land::{AL_ALLOW_EXPERIENCE, AL_BLOCK_EXPERIENCE, LandCommand, OS_LEASED, PARCEL_MAX_EXPERIENCE_LIST};
use aurora_net::land::{RT_GROUP, RT_OTHER, RT_OWNER};
use aurora_net::{ParcelInfo, parcel_flags as pf, region_flags as rf};
use egui::{Pos2, Rect, RichText, Sense, Vec2};
use glam::Vec3;
use uuid::Uuid;

/// The groups of settings, in the navigation's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(super) enum Section {
    #[default]
    Access,
    Rules,
    Objects,
    Visibility,
    Media,
    Environment,
    Experiences,
    Sale,
}

impl Section {
    pub(super) const ALL: [Section; 8] = [
        Section::Access,
        Section::Rules,
        Section::Objects,
        Section::Visibility,
        Section::Media,
        Section::Environment,
        Section::Experiences,
        Section::Sale,
    ];

    fn icon(self) -> &'static str {
        match self {
            Section::Access => "door-enter",
            Section::Rules => "hand-finger",
            Section::Objects => "box",
            Section::Visibility => "search-filled",
            Section::Media => "music",
            Section::Environment => "sun-high-filled",
            Section::Experiences => "flask-filled",
            Section::Sale => "tag-filled",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Section::Access => aurora_i18n::tr!("ui-land-manage-access"),
            Section::Rules => aurora_i18n::tr!("ui-land-manage-rules"),
            Section::Objects => aurora_i18n::tr!("ui-land-manage-objects"),
            Section::Visibility => aurora_i18n::tr!("ui-land-manage-visibility"),
            Section::Media => aurora_i18n::tr!("ui-land-manage-media"),
            Section::Environment => aurora_i18n::tr!("ui-land-manage-environment"),
            Section::Experiences => aurora_i18n::tr!("ui-land-manage-experiences"),
            Section::Sale => aurora_i18n::tr!("ui-land-manage-sale"),
        }
    }

    /// The group power the section's settings need (the panels' main
    /// one; a few rows need another, named in their tooltip).
    pub(super) fn power(self) -> u64 {
        match self {
            Section::Access => powers::LAND_MANAGE_ALLOWED,
            Section::Rules | Section::Experiences => powers::LAND_OPTIONS,
            Section::Objects => powers::LAND_RETURN_NON_GROUP,
            Section::Visibility => powers::LAND_FIND_PLACES,
            Section::Media => powers::LAND_CHANGE_MEDIA,
            Section::Environment => powers::LAND_ALLOW_ENVIRONMENT,
            Section::Sale => powers::LAND_SET_SALE_INFO,
        }
    }
}

/// The name of a group power as roles_constants.h describes it.
pub(super) fn power_name(power: u64) -> &'static str {
    match power {
        powers::LAND_MANAGE_ALLOWED => aurora_i18n::tr!("ui-land-power-manage-allowed"),
        powers::LAND_MANAGE_BANNED => aurora_i18n::tr!("ui-land-power-manage-banned"),
        powers::LAND_SET_LANDING_POINT => aurora_i18n::tr!("ui-land-power-landing-point"),
        powers::LAND_EDIT => aurora_i18n::tr!("ui-land-power-edit"),
        powers::LAND_FIND_PLACES => aurora_i18n::tr!("ui-land-power-find-places"),
        powers::LAND_CHANGE_IDENTITY => aurora_i18n::tr!("ui-land-power-identity"),
        powers::LAND_CHANGE_MEDIA => aurora_i18n::tr!("ui-land-power-media"),
        powers::LAND_ALLOW_ENVIRONMENT => aurora_i18n::tr!("ui-land-power-environment"),
        powers::LAND_SET_SALE_INFO => aurora_i18n::tr!("ui-land-power-sale"),
        powers::LAND_DEED => aurora_i18n::tr!("ui-land-power-deed"),
        powers::LAND_RELEASE => aurora_i18n::tr!("ui-land-power-release"),
        powers::LAND_RETURN_GROUP_OWNED | powers::LAND_RETURN_GROUP_SET | powers::LAND_RETURN_NON_GROUP => {
            aurora_i18n::tr!("ui-land-power-return")
        }
        _ => aurora_i18n::tr!("ui-land-power-options"),
    }
}

/// Every power a setting of the page needs.
const EDIT_POWERS: [u64; 14] = [
    powers::LAND_OPTIONS,
    powers::LAND_MANAGE_ALLOWED,
    powers::LAND_MANAGE_BANNED,
    powers::LAND_SET_LANDING_POINT,
    powers::LAND_EDIT,
    powers::LAND_FIND_PLACES,
    powers::LAND_CHANGE_IDENTITY,
    powers::LAND_CHANGE_MEDIA,
    powers::LAND_ALLOW_ENVIRONMENT,
    powers::LAND_SET_SALE_INFO,
    powers::LAND_RETURN_GROUP_OWNED,
    powers::LAND_RETURN_GROUP_SET,
    powers::LAND_RETURN_NON_GROUP,
    powers::LAND_RELEASE,
];

/// The banner over the settings when none can be changed, saying why
/// (isParcelModifiableByAgent: the owner, or for a group's land the members
/// with the power; a purchase waiting for approval cannot be changed).
/// `preview`: the owner looks at the window as a visitor would.
pub(super) fn lock_banner(v: &View, preview: bool, group: &str) -> Option<String> {
    let parcel = v.parcel.as_ref()?;
    if preview {
        return Some(aurora_i18n::tr!("ui-land-manage-lock-preview").to_owned());
    }
    if parcel.owner_id == v.agent && parcel.status != OS_LEASED {
        return Some(aurora_i18n::tr!("ui-land-general-you-must-approve-your-purchase-before-editing-this-land").to_owned());
    }
    if EDIT_POWERS.iter().any(|p| v.can(*p)) {
        return None;
    }
    Some(if parcel.owner_id.is_nil() {
        aurora_i18n::tr!("ui-land-manage-lock-public").to_owned()
    } else if parcel.is_group_owned {
        aurora_i18n::tr!("ui-land-manage-lock-group", group = group.to_owned())
    } else {
        aurora_i18n::tr!("ui-land-manage-lock-owner").to_owned()
    })
}

/// A section the agent may partly manage: the power one of its settings
/// lacks, said once under its title.
pub(super) fn missing_power(v: &View, power: u64) -> Option<String> {
    (!v.can(power)).then(|| aurora_i18n::tr!("ui-land-manage-missing-power", power = power_name(power).to_owned()))
}

/// The title of a section: its icon and name, and `right` at the right end.
fn title(ui: &mut egui::Ui, p: &Palette, sec: Section, right: impl FnOnce(&mut egui::Ui)) {
    sheet::section_title(ui, p, sec.icon(), sec.label(), right);
}

fn button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, enabled: bool) -> egui::Response {
    widgets::action_button_if(ui, p, icon, label, widgets::ActionKind::Flat, Vec2::new(0.0, 26.0), enabled)
}

/// What the section's title row shows at the right of the navigation.
fn nav_count(v: &View, world: &World, sec: Section) -> Option<String> {
    let parcel = v.parcel.as_ref()?;
    match sec {
        Section::Objects => {
            let usage = PrimUsage::of(parcel, v.region.as_ref().map_or(0, |r| r.max_tasks));
            (usage.max > 0).then(|| aurora_i18n::number_text(&format!("{:.0} %", usage.fraction() * 100.0)))
        }
        Section::Experiences => {
            let n = world.land.sel.as_ref().map_or(0, |s| s.allowed_experiences.len());
            (n > 0).then(|| n.to_string())
        }
        _ => None,
    }
}

/// The page. True when the saved music streams changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    streams: &mut Vec<String>,
    preview: bool,
    has_xp: bool,
) -> bool {
    let Some(parcel) = v.parcel.clone() else {
        return false;
    };
    let groups = world.groups.groups.clone();
    let group = world.land.group_name(&parcel.group_id, &groups).unwrap_or_default();
    let banner = lock_banner(v, preview, &group);
    if let Some(text) = &banner {
        lock_bar(ui, p, text);
    }
    let sections: Vec<Section> = Section::ALL
        .into_iter()
        .filter(|sec| *sec != Section::Experiences || has_xp)
        .collect();
    let mut streams_changed = false;
    let lock = banner.is_some();
    let counts: Vec<Option<String>> = sections.iter().map(|sec| nav_count(v, world, *sec)).collect();
    let nav = |ui: &mut egui::Ui, sec: Section, lit: bool| {
        let count = sections.iter().position(|x| *x == sec).and_then(|i| counts[i].as_deref());
        let mut r = widgets::navigation_wrapped(ui, p, sec.icon(), sec.label(), count, lit);
        if banner.is_none()
            && let Some(m) = missing_power(v, sec.power())
        {
            r = r.on_hover_text(m);
        }
        r
    };
    let mut nav_state = std::mem::take(&mut s.manage_nav);
    sheet::sections(ui, p, "about_land_manage", &sections, &mut nav_state, nav, |ui2, sec| match sec {
        Section::Access => access(ui2, p, s, v, world, &parcel, lock, &group),
        Section::Rules => rules(ui2, p, s, v, world, &parcel, lock),
        Section::Objects => objects(ui2, p, s, v, world, &parcel, lock),
        Section::Visibility => visibility(ui2, p, s, v, world, &parcel, images, lock),
        Section::Media => streams_changed |= media(ui2, p, s, v, world, &parcel, images, streams, lock),
        Section::Environment => environment(ui2, p, s, v, world, &parcel, lock),
        Section::Experiences => experiences(ui2, p, s, v, world, lock),
        Section::Sale => sale(ui2, p, s, v, world, &parcel, lock),
    });
    s.manage_nav = nav_state;
    streams_changed
}

fn is_open(s: &LandUi, sec: Section) -> bool {
    s.manage_open.contains(&sec)
}

fn set_open(s: &mut LandUi, sec: Section, open: bool) {
    if open {
        s.manage_open.insert(sec);
    } else {
        s.manage_open.remove(&sec);
    }
}

/// The look-at of the landing point as the Options tab writes it.
fn heading(look: Vec3) -> u32 {
    let deg = ((look.y.atan2(-look.x) + std::f32::consts::TAU).to_degrees() + 0.5) as u32;
    (deg - 90) % 360
}

/// The access choice of the title: everyone, the group and the list, or
/// the list only.
pub(super) fn access_choice(a: &AccessState) -> usize {
    if a.public {
        0
    } else if a.group {
        1
    } else {
        2
    }
}

/// The access state for a choice of the title (the checkboxes behind it:
/// "Anyone can visit", and the group without restrictions).
pub(super) fn with_access_choice(mut a: AccessState, choice: usize, group_known: bool) -> AccessState {
    match choice {
        0 => a.public = true,
        1 => {
            a.public = false;
            a.group = group_known;
            // onCommitGroupCheck: no passes "for the group" with group access
            a.pass_group = false;
        }
        _ => {
            a.public = false;
            a.group = false;
        }
    }
    a
}

#[allow(clippy::too_many_arguments)]
fn access(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, lock: bool, group: &str) {
    let override_ok = parcel.region_allow_access_override;
    let can = override_ok && v.can(powers::LAND_MANAGE_ALLOWED);
    let group_known = !parcel.group_id.is_nil() && !group.is_empty();
    let mut now = AccessState::of(parcel, s.pass_to_group);
    let before = now;
    let options = [
        (aurora_i18n::tr!("common-everyone"), true),
        (aurora_i18n::tr!("ui-land-overview-group-and-list"), true),
        (aurora_i18n::tr!("ui-land-overview-list-only"), true),
    ];
    title(ui, p, Section::Access, |ui| {
        editable(ui, can, |ui| {
            let mut choice = access_choice(&now);
            if widgets::segmented(ui, p, &mut choice, &options) {
                now = with_access_choice(now, choice, group_known);
            }
        });
    });
    why(ui, p, aurora_i18n::tr!("ui-land-manage-access-why"));
    // what the estate imposes links to where it is decided
    let see = aurora_i18n::tr!("ui-land-manage-see-in-region");
    let mut see_region = false;
    if !override_ok {
        see_region |= sheet::lock_line_link(ui, p, aurora_i18n::tr!("ui-land-manage-access-estate"), see);
    } else if !lock && let Some(m) = missing_power(v, powers::LAND_MANAGE_ALLOWED) {
        lock_line(ui, p, &m);
    }
    let estate = aurora_i18n::tr!("ui-land-manage-set-by-estate");
    let age = sheet::RowText {
        name: aurora_i18n::tr!("ui-land-manage-age"),
        lock: None,
        desc: if parcel.region_deny_age_unverified {
            estate
        } else {
            aurora_i18n::tr!("ui-land-manage-age-desc")
        },
        link: parcel.region_deny_age_unverified.then_some(see),
    };
    see_region |= sheet::srow_ext(ui, p, &age, 40.0, |ui| {
        editable(ui, now.public && can && !parcel.region_deny_age_unverified, |ui| {
            widgets::switch(ui, p, &mut now.age)
                .on_hover_text(aurora_i18n::tr!("ui-land-access-residents-must-be-at-least-18-to-access-this"));
        });
    });
    line(ui, p);
    let payment = sheet::RowText {
        name: aurora_i18n::tr!("ui-land-manage-payment"),
        lock: None,
        desc: if parcel.region_deny_anonymous { estate } else { "" },
        link: parcel.region_deny_anonymous.then_some(see),
    };
    see_region |= sheet::srow_ext(ui, p, &payment, 40.0, |ui| {
        editable(ui, now.public && can && !parcel.region_deny_anonymous, |ui| {
            widgets::switch(ui, p, &mut now.payment).on_hover_text(aurora_i18n::tr!(
                "ui-land-access-residents-must-have-payment-info-on-file-to-access"
            ));
        });
    });
    if see_region {
        crate::ui::region::request_open(ui.ctx());
    }
    line(ui, p);
    let group_label = aurora_i18n::tr!(
        "ui-land-manage-group-enters",
        group = if group_known { group.to_owned() } else { "—".to_owned() }
    );
    srow(ui, p, &group_label, "", 40.0, |ui| {
        let can_allow_group = !now.public || (now.payment ^ now.age);
        editable(ui, group_known && can && can_allow_group, |ui| {
            let mut on = now.group;
            widgets::switch(ui, p, &mut on).on_hover_text(aurora_i18n::tr!("ui-land-access-set-the-group-on-the-general-tab"));
            if on != now.group {
                now.group = on;
                if on && !now.public {
                    now.pass_group = false;
                }
            }
        });
    });
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-passes"),
        aurora_i18n::tr!("ui-land-manage-passes-desc"),
        360.0,
        |ui| {
            editable(ui, !now.public && can, |ui| {
                widgets::switch(ui, p, &mut now.pass)
                    .on_hover_text(aurora_i18n::tr!("ui-land-access-allow-temporary-access-to-this-parcel"));
            });
            let pass_on = !now.public && now.pass && can;
            editable(ui, pass_on, |ui| {
                let mut to = usize::from(now.pass_group);
                let to_options = [
                    (aurora_i18n::tr!("common-everyone"), true),
                    (aurora_i18n::tr!("common-group"), true),
                ];
                if widgets::segmented(ui, p, &mut to, &to_options) {
                    now.pass_group = to == 1;
                }
                // the spinners send their value once released or typed
                let (mut price, mut hours) = s.pass_edit.unwrap_or((now.price, now.hours));
                let r2 = ui.add(
                    egui::DragValue::new(&mut hours)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0.0..=24.0)
                        .speed(0.1)
                        .fixed_decimals(2)
                        .suffix(" h"),
                );
                let r = ui.add(
                    egui::DragValue::new(&mut price)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0..=500)
                        .suffix(" L$"),
                );
                let active = r.dragged() || r.has_focus() || r2.dragged() || r2.has_focus();
                if active {
                    s.pass_edit = Some((price, hours));
                } else if s.pass_edit.take().is_some() || r.changed() || r2.changed() {
                    now.price = price;
                    now.hours = hours;
                }
            });
        },
    );
    s.pass_to_group = now.pass_group;
    if now != before {
        super::access::commit(world, parcel, &now, group_known);
    }
    line(ui, p);
    // teleport routing and landing point (Options tab)
    let can_landing = v.can(powers::LAND_SET_LANDING_POINT);
    let routing = [
        (aurora_i18n::tr!("ui-land-overview-arrival-fixed"), true),
        (aurora_i18n::tr!("ui-land-overview-arrival-anywhere"), true),
        (aurora_i18n::tr!("ui-land-overview-arrival-blocked"), true),
    ];
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-routing"),
        aurora_i18n::tr!("ui-land-manage-routing-desc"),
        260.0,
        |ui| {
            editable(ui, can_landing, |ui| {
                // LLParcel::ELandingType: 0 blocked, 1 landing point, 2 anywhere
                let mut choice = match parcel.landing_type {
                    1 => 0,
                    0 => 2,
                    _ => 1,
                };
                if widgets::segmented(ui, p, &mut choice, &routing) {
                    let t = [1u8, 2, 0][choice];
                    world.land.update(|u| u.landing_type = t);
                }
            });
        },
    );
    line(ui, p);
    let lp = parcel.user_location;
    let has_lp = lp != Vec3::ZERO;
    let lp_desc = if has_lp {
        aurora_i18n::tr!(
            "ui-land-manage-landing-point-at",
            position = format!("{}, {}, {}", lp.x.round(), lp.y.round(), lp.z.round()),
            heading = heading(parcel.user_look_at).to_string()
        )
    } else {
        aurora_i18n::tr!("ui-land-manage-landing-point-none").to_owned()
    };
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-landing-point"), &lp_desc, 220.0, |ui| {
        if button(ui, p, "", aurora_i18n::tr!("ui-land-manage-clear"), can_landing && has_lp)
            .on_hover_text(aurora_i18n::tr!("ui-land-options-clear-landing-point"))
            .clicked()
        {
            world.land.update(|u| {
                u.user_location = Vec3::ZERO;
                u.user_look_at = Vec3::ZERO;
            });
        }
        if button(ui, p, "map-pin-filled", aurora_i18n::tr!("ui-land-manage-set-here"), can_landing)
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
    });
    if !lock && !can_landing && can {
        lock_line(ui, p, &missing_power(v, powers::LAND_SET_LANDING_POINT).unwrap_or_default());
    }
    let mut go = false;
    fold_row(ui, p, &mut go, aurora_i18n::tr!("ui-land-manage-lists-elsewhere"));
    if go {
        s.page = super::Page::People;
    }
}

/// One line of the visitors' table: everyone's box, the group's (None:
/// the setting has no group column, an empty greyed box like the mockup).
#[allow(clippy::too_many_arguments)]
fn who_row(
    ui: &mut egui::Ui,
    p: &Palette,
    label: &str,
    tip: &str,
    all: &mut bool,
    group: Option<&mut bool>,
    can_all: bool,
    can_group: bool,
    warn: Option<&str>,
) {
    const ALL_W: f32 = 110.0;
    const GROUP_W: f32 = 90.0;
    ui.horizontal(|ui| {
        ui.set_min_height(30.0);
        ui.spacing_mut().item_spacing.x = 0.0;
        let label_w = ui.available_width() - ALL_W - GROUP_W;
        ui.allocate_ui_with_layout(Vec2::new(label_w, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(label_w);
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(label).size(12.5).color(p.ink)).on_hover_text(tip);
            if let Some(w) = warn {
                widgets::glyph(ui, "alert-triangle-filled", p.warn, 12.0);
                ui.label(RichText::new(w).size(11.0).color(p.warn));
            }
        });
        let cell = |ui: &mut egui::Ui, w: f32, body: &mut dyn FnMut(&mut egui::Ui)| {
            ui.allocate_ui_with_layout(Vec2::new(w, 30.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_min_width(w);
                ui.add_space(6.0);
                body(ui);
            });
        };
        cell(ui, ALL_W, &mut |ui| {
            editable(ui, can_all, |ui| {
                widgets::checkbox(ui, p, all, "").on_hover_text(tip);
            });
        });
        let mut none = false;
        let group = group.unwrap_or(&mut none);
        cell(ui, GROUP_W, &mut |ui| {
            editable(ui, can_group, |ui| {
                widgets::checkbox(ui, p, group, "");
            });
        });
    });
}

fn rules(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, lock: bool) {
    let can = v.can(powers::LAND_OPTIONS);
    let can_terraform = v.can(powers::LAND_EDIT);
    let sim_access = v.region.as_ref().map_or(0, |r| r.sim_access);
    let before = Shown::of(parcel, parcel.region_push_override, options::mature_value(parcel, sim_access));
    let mut now = before;
    title(ui, p, Section::Rules, |_| {});
    why(ui, p, aurora_i18n::tr!("ui-land-manage-rules-why"));
    if !lock && let Some(m) = missing_power(v, powers::LAND_OPTIONS) {
        lock_line(ui, p, &m);
    }
    // the table's head
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let label_w = ui.available_width() - 200.0;
        ui.add_space(label_w);
        for (text, w) in [
            (aurora_i18n::tr!("ui-land-manage-everyone-head"), 110.0),
            (aurora_i18n::tr!("ui-land-manage-group-head"), 90.0),
        ] {
            ui.allocate_ui_with_layout(Vec2::new(w, 16.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_min_width(w);
                ui.label(semibold(ui, text, 11.0, p.muted_dim));
            });
        }
    });
    // explicitly on (and greyed) while everyone may
    let (b_obj, b_entry, b_scripts) = (before.edit_objects, before.all_entry, before.other_scripts);
    who_row(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-build"),
        aurora_i18n::tr!("ui-land-options-when-checked-residents-can-create-and-rez-objects-on"),
        &mut now.edit_objects,
        Some(&mut now.edit_group),
        can,
        can && !b_obj,
        None,
    );
    line(ui, p);
    who_row(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-object-entry"),
        aurora_i18n::tr!("ui-land-options-when-checked-residents-can-move-their-objects-from-other"),
        &mut now.all_entry,
        Some(&mut now.group_entry),
        can,
        can && !b_entry,
        None,
    );
    line(ui, p);
    who_row(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-run-scripts"),
        aurora_i18n::tr!("ui-land-options-when-checked-residents-can-run-scripts-on-your-parcel"),
        &mut now.other_scripts,
        Some(&mut now.group_scripts),
        can,
        can && !b_scripts,
        None,
    );
    line(ui, p);
    who_row(
        ui,
        p,
        aurora_i18n::tr!("ui-land-overview-fly"),
        aurora_i18n::tr!("ui-land-options-when-checked-residents-can-fly-on-your-land-otherwise"),
        &mut now.fly,
        None,
        can,
        false,
        None,
    );
    line(ui, p);
    let terraform_tip = if can_terraform {
        aurora_i18n::tr!("ui-land-options-when-checked-everyone-can-terraform-your-land-it-is").to_owned()
    } else {
        missing_power(v, powers::LAND_EDIT).unwrap_or_default()
    };
    who_row(
        ui,
        p,
        aurora_i18n::tr!("ui-land-power-edit"),
        &terraform_tip,
        &mut now.terraform,
        None,
        can_terraform,
        false,
        Some(aurora_i18n::tr!("ui-land-manage-use-with-care")),
    );
    ui.add_space(4.0);
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-safe"),
        aurora_i18n::tr!("ui-land-manage-safe-desc"),
        40.0,
        |ui| {
            editable(ui, can, |ui| {
                widgets::switch(ui, p, &mut now.safe)
                    .on_hover_text(aurora_i18n::tr!("ui-land-options-when-checked-the-land-is-safe-no-combat-damage"));
            });
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-overview-no-push"),
        if parcel.region_push_override {
            aurora_i18n::tr!("ui-land-manage-push-region")
        } else {
            aurora_i18n::tr!("ui-land-manage-push-desc")
        },
        40.0,
        |ui| {
            editable(ui, can && !parcel.region_push_override, |ui| {
                widgets::switch(ui, p, &mut now.push).on_hover_text(aurora_i18n::tr!(
                    "ui-land-options-prevents-scripts-from-pushing-people-useful-for-preventing-abusive"
                ));
            });
        },
    );
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-see-avatars"), "", 40.0, |ui| {
        // the server must know SeeAVs (getHaveNewParcelLimitData)
        editable(ui, can && parcel.have_new_parcel_limit_data, |ui| {
            widgets::switch(ui, p, &mut now.see_avs)
                .on_hover_text(aurora_i18n::tr!("ui-land-options-allow-avatars-on-other-parcels-to-see-and-chat"));
        });
    });
    options::commit(s, v, world, parcel, before, now, None);
}

/// The parcel's own capacity (LLPanelLandObjects::refresh): its prims with
/// the bonus, never more than the region holds.
pub(super) fn parcel_capacity(parcel: &ParcelInfo, region_max: u32) -> i32 {
    let mut max = (parcel.max_prims as f32 * parcel.prim_bonus).round() as i32;
    if region_max > 0 {
        max = max.min(region_max as i32);
    }
    max
}

/// The widths of the split bar's parts: owner, group, others and what is
/// free, each part with objects at least `min` wide.
pub(super) fn split_widths(counts: [i32; 3], capacity: i32, width: f32, min: f32) -> [f32; 4] {
    let used: i32 = counts.iter().map(|c| (*c).max(0)).sum();
    let total = capacity.max(used).max(1) as f32;
    let mut w = [0.0; 4];
    for (i, c) in counts.iter().enumerate() {
        if *c > 0 {
            w[i] = (*c as f32 / total * width).max(min);
        }
    }
    w[3] = (width - w[0] - w[1] - w[2]).max(0.0);
    w
}

fn objects(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, lock: bool) {
    let region_max = v.region.as_ref().map_or(0, |r| r.max_tasks);
    let usage = PrimUsage::of(parcel, region_max);
    let capacity = parcel_capacity(parcel, region_max);
    let can_owned = v.can(powers::LAND_RETURN_GROUP_OWNED);
    let can_group = v.can(powers::LAND_RETURN_GROUP_SET);
    let can_other = v.can(powers::LAND_RETURN_NON_GROUP);
    let any = can_owned || can_group || can_other;
    title(ui, p, Section::Objects, |_| {});
    if !lock && !any {
        lock_line(ui, p, &missing_power(v, powers::LAND_RETURN_NON_GROUP).unwrap_or_default());
    }
    ui.add_space(6.0);
    // the split of the parcel's capacity
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 14.0), Sense::hover());
    let colors = [p.violet, p.teal, p.amber, p.bar];
    let widths = split_widths([parcel.owner_prims, parcel.group_prims, parcel.other_prims], capacity, width, 4.0);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, widgets::TRACK_RADIUS, p.bar);
    let mut x = rect.left();
    for (w, c) in widths.iter().zip(colors).take(3) {
        if *w > 0.0 {
            painter.rect_filled(Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(*w, rect.height())), 0.0, c);
            x += w;
        }
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        for (label, count, color) in [
            (aurora_i18n::tr!("common-owner"), parcel.owner_prims, p.violet),
            (aurora_i18n::tr!("common-group"), parcel.group_prims, p.teal),
            (aurora_i18n::tr!("ui-land-manage-others"), parcel.other_prims, p.amber),
        ] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (sq, _) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                ui.painter().rect_filled(sq, widgets::TRACK_RADIUS, color);
                ui.label(RichText::new(label).size(12.0).color(p.muted));
                ui.label(semibold(ui, &aurora_i18n::integer(i64::from(count)), 12.0, p.ink));
            });
        }
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "ui-land-manage-impact",
                impact = aurora_i18n::integer(i64::from(parcel.total_prims)),
                capacity = aurora_i18n::integer(i64::from(capacity)),
                used = aurora_i18n::integer(i64::from(usage.used)),
                max = aurora_i18n::integer(i64::from(usage.max))
            ))
            .size(12.0)
            .color(p.muted),
        );
    });
    if parcel.prim_bonus != 1.0 || parcel.selected_prims > 0 {
        let mut notes = Vec::new();
        if parcel.prim_bonus != 1.0 {
            notes.push(aurora_i18n::tr!(
                "ui-land-objects-object-bonus-factor",
                value0 = aurora_i18n::number_text(&format!("{:.2}", parcel.prim_bonus))
            ));
        }
        if parcel.selected_prims > 0 {
            notes.push(aurora_i18n::tr!(
                "ui-land-manage-selected-sat",
                count = aurora_i18n::integer(i64::from(parcel.selected_prims))
            ));
        }
        ui.label(RichText::new(notes.join(" · ")).size(11.5).color(p.muted));
    }
    ui.add_space(4.0);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-auto-return"),
        aurora_i18n::tr!("ui-land-manage-auto-return-desc"),
        90.0,
        |ui| {
            ui.label(RichText::new("min").size(12.0).color(p.muted));
            s.clean_time.sync(&parcel.other_clean_time.to_string());
            let r = editable(ui, any, |ui| {
                ui.add(egui::TextEdit::singleline(&mut s.clean_time.text).desired_width(50.0))
            });
            // validateNonNegativeS32
            s.clean_time.text.retain(|c| c.is_ascii_digit());
            if let Some(t) = s.clean_time.after(&r) {
                // onCommitClean: only a changed value is sent
                let minutes: i32 = t.parse().unwrap_or(0);
                if minutes != parcel.other_clean_time {
                    world
                        .land
                        .parcel_command(|handle, local_id| LandCommand::SetOtherCleanTime { handle, local_id, minutes });
                    world.land.update(|_| {});
                }
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-return"),
        aurora_i18n::tr!("ui-land-manage-return-desc"),
        440.0,
        |ui| {
            // onClickShow*Objects: the simulator highlights them (ForceObjectSelect)
            let shown = [
                (
                    aurora_i18n::tr!("ui-land-manage-return-owner"),
                    parcel.owner_prims,
                    can_owned,
                    RT_OWNER,
                ),
                (
                    aurora_i18n::tr!("ui-land-manage-return-group"),
                    parcel.group_prims,
                    can_group,
                    RT_GROUP,
                ),
                (
                    aurora_i18n::tr!("ui-land-manage-return-others"),
                    parcel.other_prims,
                    can_other,
                    RT_OTHER,
                ),
            ];
            let any_shown = shown.iter().any(|(_, count, can, _)| *can && *count > 0);
            editable(ui, any_shown, |ui| {
                let mut button = egui::Button::new(RichText::new(aurora_i18n::tr!("common-show")).size(12.5));
                if let Some(caret) = crate::ui::icons::global(widgets::disclosure_caret(true)) {
                    button = button.right_text(egui::Image::new((caret.id(), Vec2::splat(10.0))).tint(p.muted));
                }
                menu::button(ui, p, button.min_size(Vec2::new(0.0, 26.0)), |ui| {
                    for (label, count, can, kind) in shown {
                        if menu::item_if(ui, p, "eye-filled", label, can && count > 0) {
                            world.land.show_objects(kind, Vec::new());
                        }
                    }
                })
                .on_hover_text(aurora_i18n::tr!("ui-land-manage-show-tip"));
            });
            for (label, count, can, confirm) in [
                (
                    aurora_i18n::tr!("ui-land-manage-return-others"),
                    parcel.other_prims,
                    can_other,
                    Confirm::ReturnOther,
                ),
                (
                    aurora_i18n::tr!("ui-land-manage-return-group"),
                    parcel.group_prims,
                    can_group,
                    Confirm::ReturnGroup,
                ),
                (
                    aurora_i18n::tr!("ui-land-manage-return-owner"),
                    parcel.owner_prims,
                    can_owned,
                    Confirm::ReturnOwner,
                ),
            ] {
                if button(ui, p, "", label, can && count > 0)
                    .on_hover_text(aurora_i18n::tr!("ui-land-objects-return-objects-to-their-owners"))
                    .clicked()
                {
                    s.dialog = Some(Dialog::Confirm(confirm));
                }
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn visibility(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    parcel: &ParcelInfo,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    lock: bool,
) {
    let can_identity = v.can(powers::LAND_CHANGE_IDENTITY);
    let sim_access = v.region.as_ref().map_or(0, |r| r.sim_access);
    let before = Shown::of(parcel, parcel.region_push_override, options::mature_value(parcel, sim_access));
    let mut now = before;
    let mut extra: options::Extra = None;
    let (search_on, search_tip) = options::search_state(v, parcel, before.directory);
    title(ui, p, Section::Visibility, |_| {});
    ui.add_space(4.0);
    if !lock && let Some(m) = missing_power(v, powers::LAND_FIND_PLACES) {
        lock_line(ui, p, &m);
    }
    let fee = aurora_i18n::tr!("ui-land-manage-search-desc", fee = options::DIRECTORY_FEE.to_string());
    srow(ui, p, aurora_i18n::tr!("ui-land-options-place-in-search"), &fee, 40.0, |ui| {
        editable(ui, search_on, |ui| {
            widgets::switch(ui, p, &mut now.directory)
                .on_hover_text(search_tip)
                .on_disabled_hover_text(search_tip);
        });
    });
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-category-rating"), "", 400.0, |ui| {
        // maturity: General never, Moderate by choice, Adult always
        match sim_access {
            21 => {
                editable(ui, can_identity, |ui| {
                    let mut choice = usize::from(now.mature);
                    let opts = [
                        (aurora_i18n::tr!("common-general"), true),
                        (aurora_i18n::tr!("common-moderate"), true),
                    ];
                    if widgets::segmented(ui, p, &mut choice, &opts) {
                        now.mature = choice == 1;
                    }
                });
                widgets::maturity_badge(
                    ui,
                    p,
                    if now.mature {
                        aurora_i18n::tr!("common-moderate")
                    } else {
                        aurora_i18n::tr!("common-general")
                    },
                    15.0,
                );
            }
            _ => {
                ui.label(RichText::new(crate::ui::bars::maturity_name(sim_access)).size(12.0).color(p.muted));
                widgets::maturity_badge_of(ui, p, sim_access, 15.0);
            }
        }
        let current = options::categories()
            .iter()
            .find(|c| c.0 == parcel.category)
            .map_or(aurora_i18n::tr!("status-other"), |c| c.1);
        editable(ui, search_on, |ui| {
            egui::ComboBox::from_id_salt("land_category_aurora")
                .selected_text(current)
                .width(170.0)
                .show_ui(ui, |ui| {
                    for (id, label) in options::categories() {
                        if ui.selectable_label(id == parcel.category, label).clicked() && id != parcel.category {
                            extra = Some(Box::new(move |u| u.category = id as u8));
                        }
                    }
                });
        });
    });
    options::commit(s, v, world, parcel, before, now, extra);
    let mut open = is_open(s, Section::Visibility);
    fold_row(ui, p, &mut open, aurora_i18n::tr!("ui-land-manage-visibility-advanced"));
    set_open(s, Section::Visibility, open);
    if !open {
        return;
    }
    if !lock && let Some(m) = missing_power(v, powers::LAND_CHANGE_IDENTITY) {
        lock_line(ui, p, &m);
    }
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-name"), "", 340.0, |ui| {
        s.name.sync(&parcel.name);
        let r = editable(ui, can_identity, |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.name.text).desired_width(330.0))
        });
        // validateASCIIPrintableNoPipe, on what is typed
        if r.changed() {
            let base = s.name.base.clone();
            s.name.text.retain(|c| ((' '..='~').contains(&c) && c != '|') || base.contains(c));
        }
        if let Some(name) = s.name.after(&r) {
            world.land.update(|u| u.name = name);
        }
    });
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-overview-description"), "", 340.0, |ui| {
        s.desc.sync(&parcel.desc);
        let r = editable(ui, can_identity, |ui| {
            ui.add(egui::TextEdit::multiline(&mut s.desc.text).desired_rows(3).desired_width(330.0))
        });
        if let Some(desc) = s.desc.after(&r) {
            world.land.update(|u| u.desc = desc);
        }
    });
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-options-land-picture"),
        aurora_i18n::tr!("ui-land-manage-picture-desc"),
        110.0,
        |ui| {
            let size = Vec2::new(100.0, 66.0);
            let sense = if can_identity { Sense::click() } else { Sense::hover() };
            let (rect, r) = ui.allocate_exact_size(size, sense);
            let tex = (!parcel.snapshot_id.is_nil())
                .then(|| {
                    s.wanted_images.insert(parcel.snapshot_id);
                    images.get(&parcel.snapshot_id)
                })
                .flatten();
            match tex {
                Some(t) => {
                    let [w, h] = t.size();
                    egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), size))
                        .uv(super::aurora::cover_uv(w, h, size))
                        .corner_radius(ui.visuals().window_corner_radius)
                        .paint_at(ui, rect);
                }
                None => {
                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.bar);
                }
            }
            if r.on_hover_text(aurora_i18n::tr!("common-click-to-select-an-image")).clicked() {
                s.texture_snapshot = true;
                s.texture.open(aurora_i18n::tr!("ui-land-options-land-picture"), parcel.snapshot_id);
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn media(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    parcel: &ParcelInfo,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
    streams: &mut Vec<String>,
    lock: bool,
) -> bool {
    let mut open = is_open(s, Section::Media);
    title(ui, p, Section::Media, |ui| {
        let label = if open {
            aurora_i18n::tr!("ui-land-manage-hide")
        } else {
            aurora_i18n::tr!("ui-land-manage-show")
        };
        fold_link(ui, p, &mut open, label);
    });
    set_open(s, Section::Media, open);
    if !open {
        why(ui, p, aurora_i18n::tr!("ui-land-manage-media-why"));
        return false;
    }
    ui.add_space(4.0);
    if !lock && let Some(m) = missing_power(v, powers::LAND_CHANGE_MEDIA) {
        lock_line(ui, p, &m);
    }
    let streams_changed = sound(ui, p, s, v, world, parcel, streams);
    line(ui, p);
    ui.add_space(6.0);
    ui.label(semibold(ui, aurora_i18n::tr!("ui-land-manage-parcel-media"), 12.5, p.violet_light));
    parcel_media(ui, p, s, v, world, parcel, images);
    streams_changed
}

/// The Sound tab (LLPanelLandAudio, with FIRE-593 / FIRE-29157 for the
/// saved streams).
fn sound(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    parcel: &ParcelInfo,
    streams: &mut Vec<String>,
) -> bool {
    let mut streams_changed = false;
    let can_media = v.can(powers::LAND_CHANGE_MEDIA);
    // FIRE-29157: the region answered with another URL than the one we set
    if let Some((local_id, url)) = s.last_music_url.clone()
        && local_id == parcel.local_id
        && world.land.sel.as_ref().is_some_and(|sel| sel.revision > s.last_music_rev)
    {
        if parcel.music_url != url && !url.is_empty() {
            log::warn!("removing stream from saved streams list because it was rejected by the region: {url}");
            streams.retain(|x| *x != url);
            streams_changed = true;
        }
        s.last_music_url = None;
    }
    let mut commit: Option<String> = None;
    srow(ui, p, aurora_i18n::tr!("common-music"), "", 420.0, |ui| {
        let current = s.music_url.text.trim().to_owned();
        if widgets::icon_button(
            ui,
            p,
            "copy-filled",
            aurora_i18n::tr!("ui-land-sound-copy-music-stream-url-to-clipboard"),
            true,
        ) && !current.is_empty()
        {
            ui.ctx().copy_text(current.clone());
        }
        if widgets::icon_button(
            ui,
            p,
            "minus",
            aurora_i18n::tr!("ui-land-sound-remove-the-music-stream-url-from-the-saved-stream"),
            can_media,
        ) {
            let before = streams.len();
            let no_http = current.strip_prefix("http://").unwrap_or("").to_owned();
            streams.retain(|x| *x != current && !(!x.contains("://") && *x == no_http));
            streams_changed |= streams.len() != before;
        }
        if widgets::icon_button(
            ui,
            p,
            "plus",
            aurora_i18n::tr!("ui-land-sound-add-the-music-stream-url-to-the-saved-streams"),
            can_media,
        ) && !current.is_empty()
        {
            if streams.contains(&current) {
                log::info!("could not add stream to saved streams list because it is already in the list: {current}");
            } else {
                streams.push(current.clone());
                streams_changed = true;
            }
        }
        editable(ui, can_media, |ui| {
            // the combo's drop-down: the parcel's URL, then the saved streams
            let img = crate::ui::icons::global("caret-down-filled")
                .map(|t| egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.muted));
            let b = match img {
                Some(img) => egui::Button::image(img),
                None => egui::Button::new("v"),
            };
            egui::containers::menu::MenuButton::from_button(b).ui(ui, |ui| {
                ui.set_min_width(320.0);
                if ui.button(parcel.music_url.as_str()).clicked() {
                    ui.close();
                }
                menu::separator(ui, p);
                for url in streams.iter() {
                    if ui.button(url.as_str()).clicked() {
                        commit = Some(url.clone());
                        ui.close();
                    }
                }
            });
            s.music_url.sync(&parcel.music_url);
            let r = ui.add(egui::TextEdit::singleline(&mut s.music_url.text).desired_width(280.0));
            if let Some(u) = s.music_url.after(&r) {
                commit = Some(u);
            }
        });
    });
    let mut flags = parcel.flags;
    let mut any_av = parcel.any_av_sounds;
    let mut group_av = parcel.group_av_sounds || parcel.any_av_sounds;
    let mut obscure = parcel.media.obscure_moap;
    let before = (flags, any_av, group_av, obscure);
    let set = |flags: &mut u32, flag: u32, on: bool| *flags = super::general::set(*flags, flag, on);
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-sound-restrict-gesture-and-object-sounds-to-this-parcel"),
        "",
        40.0,
        |ui| {
            editable(ui, can_media, |ui| {
                let mut on = flags & pf::SOUND_LOCAL != 0;
                widgets::switch(ui, p, &mut on);
                set(&mut flags, pf::SOUND_LOCAL, on);
            });
        },
    );
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-avatar-sounds"), "", 220.0, |ui| {
        let can_av = v.can(powers::LAND_OPTIONS) && parcel.have_new_parcel_limit_data;
        // on (and greyed) while "everyone" is
        editable(ui, can_av && !parcel.any_av_sounds, |ui| {
            widgets::checkbox(ui, p, &mut group_av, RichText::new(aurora_i18n::tr!("common-group")).size(12.0));
        });
        editable(ui, can_av, |ui| {
            widgets::checkbox(ui, p, &mut any_av, RichText::new(aurora_i18n::tr!("common-everyone")).size(12.0));
        });
    });
    line(ui, p);
    let voice_region = v.region_flag(rf::ALLOW_VOICE);
    let allow_voice = flags & pf::ALLOW_VOICE_CHAT != 0;
    let voice = sheet::RowText {
        name: aurora_i18n::tr!("ui-land-sound-enable-voice-chat"),
        lock: None,
        desc: if voice_region {
            ""
        } else {
            aurora_i18n::tr!("ui-land-sound-enable-voice-controlled-by-estate")
        },
        link: (!voice_region).then_some(aurora_i18n::tr!("ui-land-manage-see-in-region")),
    };
    let see_region = sheet::srow_ext(ui, p, &voice, 40.0, |ui| {
        editable(ui, can_media && voice_region, |ui| {
            let mut on = voice_region && allow_voice;
            widgets::switch(ui, p, &mut on);
            if voice_region {
                set(&mut flags, pf::ALLOW_VOICE_CHAT, on);
            }
        });
    });
    if see_region {
        crate::ui::region::request_open(ui.ctx());
    }
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-sound-restrict-voice-chat-to-this-parcel"),
        "",
        40.0,
        |ui| {
            editable(ui, voice_region && can_media && allow_voice, |ui| {
                let mut on = parcel.flags & pf::USE_ESTATE_VOICE_CHAN == 0;
                widgets::switch(ui, p, &mut on);
                set(&mut flags, pf::USE_ESTATE_VOICE_CHAN, !on);
            });
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-sound-hide-moap"),
        aurora_i18n::tr!("ui-land-sound-media-on-prims-outside-a-parcel-must-not-auto"),
        40.0,
        |ui| {
            editable(ui, can_media, |ui| {
                widgets::switch(ui, p, &mut obscure);
            });
        },
    );
    if commit.is_some() || before != (flags, any_av, group_av, obscure) {
        let music = super::sound::normalize_url(&commit.unwrap_or_else(|| parcel.music_url.clone()));
        s.music_url.base = music.clone();
        s.music_url.text = music.clone();
        // "everyone" implies the group
        let group = any_av || group_av;
        world.land.update(|u| {
            u.flags = flags;
            u.music_url = music.clone();
            u.any_av_sounds = any_av;
            u.group_av_sounds = group;
            u.obscure_moap = obscure;
        });
        s.last_music_url = Some((parcel.local_id, music));
        s.last_music_rev = world.land.sel.as_ref().map_or(0, |sel| sel.revision);
    }
    streams_changed
}

/// The Media tab (LLPanelLandMedia).
fn parcel_media(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    parcel: &ParcelInfo,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
) {
    let can = v.can(powers::LAND_CHANGE_MEDIA);
    let w = super::media::widget(&parcel.media.mime);
    let allow_resize = w.is_some_and(|w| w.3);
    let allow_loop = w.is_some_and(|w| w.4);
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-type"), "", 320.0, |ui| {
        let mime = if parcel.media.mime.is_empty() || parcel.media.mime == "none/none" {
            "—".to_owned()
        } else {
            parcel.media.mime.clone()
        };
        ui.label(RichText::new(mime).size(11.5).color(p.muted));
        editable(ui, can, |ui| {
            egui::ComboBox::from_id_salt("land_media_type_aurora")
                .selected_text(w.map_or("", |w| w.1))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (key, label, default_type, _, _) in super::media::widgets() {
                        if ui.selectable_label(w.is_some_and(|w| w.0 == key), label).clicked() && w.is_none_or(|w| w.0 != key) {
                            // onCommitType: the widget's default type
                            world.land.update(|u| u.media_type = default_type.to_owned());
                        }
                    }
                })
                .response
                .on_hover_text(aurora_i18n::tr!("ui-land-media-specify-whether-the-url-points-to-a-movie-web"));
        });
    });
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-home-page"), "", 360.0, |ui| {
        if button(ui, p, "", aurora_i18n::tr!("common-choose"), can).clicked() {
            world.land.media_type = None;
            s.dialog = Some(Dialog::MediaUrl(parcel.media_url.clone()));
        }
        let mut url = parcel.media_url.clone();
        crate::ui::widgets::text_field(ui, false, &mut url, |b, _| egui::TextEdit::singleline(b).desired_width(260.0));
    });
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-overview-description"),
        aurora_i18n::tr!("ui-land-media-text-displayed-beside-the-play-load-button"),
        340.0,
        |ui| {
            s.media_desc.sync(&parcel.media.desc);
            let r = editable(ui, can, |ui| {
                ui.add(egui::TextEdit::singleline(&mut s.media_desc.text).desired_width(330.0))
            });
            if let Some(d) = s.media_desc.after(&r) {
                world.land.update(|u| u.media_desc = d);
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-replace-texture"),
        aurora_i18n::tr!("ui-land-media-objects-using-this-texture-show-the-movie-or-web"),
        60.0,
        |ui| {
            let size = Vec2::splat(48.0);
            let (rect, r) = ui.allocate_exact_size(size, if can { Sense::click() } else { Sense::hover() });
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.bar);
            let id = parcel.media.media_id;
            if !id.is_nil() {
                s.wanted_images.insert(id);
                if let Some(t) = images.get(&id) {
                    ui.painter().image(
                        t.id(),
                        rect,
                        Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        widgets::PICTURE_TINT,
                    );
                }
            }
            if r.on_hover_text(aurora_i18n::tr!("common-click-to-select-an-image")).clicked() {
                s.texture_snapshot = false;
                s.texture.open(aurora_i18n::tr!("ui-land-media-media-texture"), id);
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-media-automatic-scaling"),
        aurora_i18n::tr!("ui-land-media-selecting-this-option-automatically-scales-the-content-of-this"),
        40.0,
        |ui| {
            editable(ui, can, |ui| {
                let mut on = parcel.media.auto_scale;
                if widgets::switch(ui, p, &mut on).changed() {
                    world.land.update(|u| u.media_auto_scale = on);
                }
            });
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-size"),
        aurora_i18n::tr!("ui-land-media-web-media-size-leave-0-for-the-default"),
        220.0,
        |ui| {
            // disallowed sizes show 0 (findAllowResize)
            let (mut width, mut height) = if allow_resize {
                (parcel.media.width, parcel.media.height)
            } else {
                (0, 0)
            };
            ui.label(RichText::new("pixels").size(12.0).color(p.muted));
            let (rh, rw) = editable(ui, can && allow_resize, |ui| {
                let rh = ui.add(
                    egui::DragValue::new(&mut height)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0..=1024),
                );
                ui.label(RichText::new("×").size(12.0).color(p.muted));
                let rw = ui.add(
                    egui::DragValue::new(&mut width)
                        .custom_parser(aurora_i18n::parse_number)
                        .range(0..=1024),
                );
                (rh, rw)
            });
            let commit = |r: &egui::Response| (r.changed() && !r.dragged()) || r.drag_stopped();
            if commit(&rw) || commit(&rh) {
                world.land.update(|u| {
                    u.media_width = width;
                    u.media_height = height;
                });
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-media-looping"),
        aurora_i18n::tr!("ui-land-media-loop-media-when-playback-ends-it-starts-again"),
        40.0,
        |ui| {
            // DEV-10042: no looping for static types
            editable(ui, can && allow_loop, |ui| {
                let mut on = allow_loop && parcel.media.looping;
                if widgets::switch(ui, p, &mut on).changed() {
                    world.land.update(|u| u.media_loop = on);
                }
            });
        },
    );
}

/// The environment choice of the title: the region's when the parcel has
/// no day cycle of its own, else the one applied from the inventory
/// ("Custom" opens the day cycle editor, it is never the lit choice: the
/// server does not tell an edited day from an applied item).
pub(super) fn environment_choice(env: &EnvState) -> usize {
    match env {
        EnvState::Ready(e) if e.has_day => 1,
        _ => 0,
    }
}

fn environment(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, lock: bool) {
    let env = world.land.sel.as_ref().map(|s| s.env.clone()).unwrap_or_default();
    let unavailable = super::environment::unavailable(v, &env);
    let summary = match &env {
        EnvState::Ready(e) => Some(e.clone()),
        _ => None,
    };
    // canEdit: LLEnvironment::canAgentUpdateParcelEnvironment and the estate's permission
    let can_edit = unavailable.is_none() && v.owns(powers::LAND_ALLOW_ENVIRONMENT) && parcel.region_allow_env_override;
    let can_enable = can_edit && summary.as_ref().is_some_and(|e| e.has_day);
    let loading = matches!(env, EnvState::Loading);
    let options = [
        (aurora_i18n::tr!("ui-land-manage-env-region"), true),
        (aurora_i18n::tr!("ui-land-manage-env-inventory"), true),
        (aurora_i18n::tr!("ui-land-manage-env-custom"), true),
    ];
    title(ui, p, Section::Environment, |ui| {
        editable(ui, can_edit && !loading, |ui| {
            let current = environment_choice(&env);
            let mut choice = current;
            let r = ui.scope(|ui| widgets::segmented(ui, p, &mut choice, &options)).response;
            // a click on the lit "From inventory" opens the picker again
            let clicked = r.hovered() && ui.input(|i| i.pointer.primary_clicked());
            match choice {
                // onBtnDefault: "Use region settings" = resetParcel, confirmed
                0 if current != 0 && can_enable => s.dialog = Some(Dialog::Confirm(Confirm::ResetEnvironment)),
                // onBtnSelect: the settings picker for the whole day
                1 if current != 1 || clicked => {
                    s.dialog = Some(Dialog::EnvironmentPicker(super::dialogs::EnvironmentPicker {
                        track: None,
                        filter: String::new(),
                        selected: None,
                    }));
                }
                // onBtnEdit: the day cycle editor on the parcel's day
                2 => s.custom_day = true,
                _ => {}
            }
            r.on_hover_text(aurora_i18n::tr!("ui-land-manage-env-custom-tip"));
        });
    });
    match unavailable {
        Some(reason) => why(ui, p, reason),
        None if loading => why(ui, p, aurora_i18n::tr!("common-loading")),
        None => why(ui, p, aurora_i18n::tr!("ui-land-manage-environment-why")),
    }
    if unavailable.is_some() {
        return;
    }
    if !lock
        && let Some(m) = (!v.owns(powers::LAND_ALLOW_ENVIRONMENT))
            .then(|| missing_power(v, powers::LAND_ALLOW_ENVIRONMENT))
            .flatten()
    {
        lock_line(ui, p, &m);
    }
    let mut open = is_open(s, Section::Environment);
    fold_row(ui, p, &mut open, aurora_i18n::tr!("ui-land-manage-environment-advanced"));
    set_open(s, Section::Environment, open);
    if !open {
        return;
    }
    let (mut length_h, mut offset_h) = super::environment::day_hours(summary.as_ref(), s.env_drag);
    let mut dragging = false;
    let mut changed = false;
    srow(ui, p, aurora_i18n::tr!("ui-land-environment-day-length-hours"), "", 300.0, |ui| {
        let r = editable(ui, can_enable, |ui| {
            ui.add(
                egui::Slider::new(&mut length_h, 4.0..=168.0)
                    .custom_parser(aurora_i18n::parse_number)
                    .step_by(0.5)
                    .fixed_decimals(1),
            )
        });
        dragging |= r.dragged() || r.has_focus();
        changed |= r.changed();
    });
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-environment-time-offset-hours"), "", 300.0, |ui| {
        let r = editable(ui, can_enable, |ui| {
            ui.add(
                egui::Slider::new(&mut offset_h, -11.5..=12.0)
                    .custom_parser(aurora_i18n::parse_number)
                    .step_by(0.5)
                    .fixed_decimals(1),
            )
        });
        dragging |= r.dragged() || r.has_focus();
        changed |= r.changed();
    });
    if dragging || changed {
        s.env_drag = Some((length_h, offset_h));
    }
    if !dragging && s.env_drag.is_some() {
        // onDayLenOffsetMouseUp → commitDayLenOffsetChanges
        s.env_drag = None;
        super::environment::commit_day(world, length_h, offset_h);
    }
    let shown = summary.as_ref().map(|e| crate::world::land::EnvSummary {
        day_length: (length_h * 3600.0) as i64,
        day_offset: {
            let o = (offset_h * 3600.0) as i64;
            if o <= 0 { o + 86400 } else { o }
        },
        ..e.clone()
    });
    // udpateApparentTimeOfDay: hidden without a day cycle of the parcel's own
    if let Some((h, m, pct)) = shown.as_ref().filter(|e| e.has_day).and_then(|e| e.apparent_time(v.now)) {
        line(ui, p);
        srow(ui, p, aurora_i18n::tr!("ui-land-manage-apparent-time"), "", 120.0, |ui| {
            ui.label(semibold(ui, &format!("{h}:{m:02} ({pct}%)"), 12.5, p.ink));
        });
    }
    // the tracks: skies 4 to 2 at their altitude (read only for a parcel),
    // the ground's sky and the water; each takes a settings item like the
    // drop targets of the altitudes (onPickerCommitted with its track)
    let name = |i: i32| {
        summary
            .as_ref()
            .map_or_else(|| aurora_i18n::tr!("status-empty").to_owned(), |e| e.track_name(i))
    };
    let mut tracks: Vec<(String, i32)> = (2..=4)
        .rev()
        .map(|sky| {
            let alt = summary
                .as_ref()
                .map_or(1000.0 * (sky - 1) as f32, |e| e.altitudes[(sky - 1) as usize]);
            (
                aurora_i18n::tr!(
                    "ui-land-environment-sky-m",
                    sky = sky.to_string(),
                    alt = aurora_i18n::number_text(&format!("{alt:.0}"))
                ),
                sky,
            )
        })
        .collect();
    tracks.push((aurora_i18n::tr!("ui-land-environment-ground").to_owned(), 1));
    tracks.push((aurora_i18n::tr!("common-water").to_owned(), 0));
    for (label, track) in tracks {
        line(ui, p);
        srow(ui, p, &label, &name(track), 120.0, |ui| {
            if button(ui, p, "", aurora_i18n::tr!("ui-land-manage-choose"), can_edit && !loading).clicked() {
                s.dialog = Some(Dialog::EnvironmentPicker(super::dialogs::EnvironmentPicker {
                    track: Some(track),
                    filter: String::new(),
                    selected: None,
                }));
            }
        });
    }
}

fn experiences(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, lock: bool) {
    // LLPanelLandExperiences::refreshPanel: read only without GP_LAND_OPTIONS
    let can = v.can(powers::LAND_OPTIONS);
    let (allowed, blocked) = world.land.sel.as_ref().map_or((Vec::new(), Vec::new()), |s| {
        (s.allowed_experiences.clone(), s.blocked_experiences.clone())
    });
    let mut open = is_open(s, Section::Experiences);
    let summary = aurora_i18n::tr!(
        "ui-land-manage-experiences-count",
        allowed = allowed.len() as i64,
        blocked = blocked.len() as i64
    );
    title(ui, p, Section::Experiences, |ui| fold_link(ui, p, &mut open, &summary));
    set_open(s, Section::Experiences, open);
    if !open {
        return;
    }
    ui.add_space(4.0);
    if !lock && let Some(m) = missing_power(v, powers::LAND_OPTIONS) {
        lock_line(ui, p, &m);
    }
    let maturity = world.maturity.preferred.unwrap_or(42);
    for (kind, list, name, help) in [
        (
            AL_ALLOW_EXPERIENCE,
            &allowed,
            aurora_i18n::tr!("ui-land-manage-xp-allowed"),
            aurora_i18n::tr!("ui-land-experiences-only-experiences-defined-for-the-parcel-are-allowed-allowed"),
        ),
        (
            AL_BLOCK_EXPERIENCE,
            &blocked,
            aurora_i18n::tr!("ui-land-manage-xp-blocked"),
            aurora_i18n::tr!("ui-land-experiences-any-resident-experience-can-be-blocked-blocked-experiences-cannot"),
        ),
    ] {
        let head = format!(
            "{name}  {}",
            aurora_i18n::tr!(
                "status-parcel-max-experience-list-max",
                value0 = list.len().to_string(),
                PARCEL_MAX_EXPERIENCE_LIST = PARCEL_MAX_EXPERIENCE_LIST.to_string()
            )
        );
        srow(ui, p, &head, help, 120.0, |ui| {
            if button(
                ui,
                p,
                "plus",
                aurora_i18n::tr!("common-add-2"),
                can && list.len() < PARCEL_MAX_EXPERIENCE_LIST,
            )
            .clicked()
            {
                // LLPanelExperienceListEditor::onAdd: the experience picker
                world.land.experience_search = None;
                s.dialog = Some(Dialog::ExperiencePicker(super::dialogs::ExperiencePicker::new(kind, maturity)));
            }
        });
        let mut remove = None;
        for id in list {
            let label = world
                .land
                .experience_name(id)
                .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into());
            ui.horizontal(|ui| {
                ui.set_min_height(26.0);
                ui.add_space(12.0);
                widgets::glyph(ui, "flask-filled", p.muted, 13.0);
                // the list's "Profile" button: the experience profile
                if ui
                    .add(egui::Label::new(RichText::new(label).size(12.5).color(p.ink)).sense(Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(aurora_i18n::tr!("common-profile-2"))
                    .clicked()
                {
                    world.experiences.open_profile(*id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-delete"), can) {
                        remove = Some(*id);
                    }
                });
            });
        }
        if let Some(id) = remove {
            super::experiences::remove_experience(world, kind, id);
        }
        line(ui, p);
    }
}

fn sale(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, parcel: &ParcelInfo, lock: bool) {
    let mut open = is_open(s, Section::Sale);
    title(ui, p, Section::Sale, |ui| {
        let label = if open {
            aurora_i18n::tr!("ui-land-manage-hide")
        } else {
            aurora_i18n::tr!("ui-land-manage-show")
        };
        fold_link(ui, p, &mut open, label);
    });
    set_open(s, Section::Sale, open);
    if !open {
        why(ui, p, aurora_i18n::tr!("ui-land-manage-sale-why"));
        return;
    }
    ui.add_space(4.0);
    let agent = v.agent;
    let is_public = parcel.owner_id.is_nil();
    let is_leased = parcel.status == OS_LEASED;
    let region_owner = v.region.as_ref().is_some_and(|r| r.owner == agent);
    let estate_manager = v.region.as_ref().is_some_and(|r| r.estate_manager);
    // the group (Set...: our own land only, not group owned)
    let groups = world.groups.groups.clone();
    let group = if is_public || parcel.group_id.is_nil() {
        aurora_i18n::tr!("status-none").to_owned()
    } else {
        world
            .land
            .group_name(&parcel.group_id, &groups)
            .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into())
    };
    srow(ui, p, aurora_i18n::tr!("ui-land-manage-group"), &group, 120.0, |ui| {
        if button(
            ui,
            p,
            "",
            aurora_i18n::tr!("common-choose"),
            v.can(powers::NONE) && !parcel.is_group_owned,
        )
        .clicked()
        {
            s.dialog = Some(Dialog::GroupPicker);
        }
    });
    line(ui, p);
    // deeding: our land with a group we are in; doing it needs GP_LAND_DEED
    let enable_deed = parcel.owner_id == agent && !parcel.group_id.is_nil() && v.rights().in_group(&parcel.group_id);
    let allow_deed = parcel.flags & pf::ALLOW_DEED_TO_GROUP != 0;
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-general-allow-deeding-to-group"),
        aurora_i18n::tr!("ui-land-general-a-group-officer-can-deed-this-land-to-the"),
        200.0,
        |ui| {
            let can_deed = v.rights().has_power(&parcel.group_id, powers::LAND_DEED);
            if button(
                ui,
                p,
                "",
                aurora_i18n::tr!("common-deed"),
                allow_deed && !parcel.group_id.is_nil() && can_deed && !parcel.is_group_owned,
            )
            .on_hover_text(aurora_i18n::tr!("ui-land-general-you-can-only-deed-land-if-you-have-an"))
            .clicked()
            {
                s.dialog = Some(Dialog::Confirm(Confirm::Deed));
            }
            editable(ui, enable_deed, |ui| {
                let mut on = allow_deed;
                if widgets::switch(ui, p, &mut on).changed() {
                    world
                        .land
                        .update(|u| u.flags = super::general::set(u.flags, pf::ALLOW_DEED_TO_GROUP, on));
                }
            });
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-general-owner-contributes-when-deeding-land"),
        aurora_i18n::tr!("ui-land-general-when-land-is-deeded-to-a-group-the-previous"),
        40.0,
        |ui| {
            editable(ui, enable_deed && allow_deed, |ui| {
                let mut on = parcel.flags & pf::CONTRIBUTE_WITH_DEED != 0;
                if widgets::switch(ui, p, &mut on).changed() {
                    world
                        .land
                        .update(|u| u.flags = super::general::set(u.flags, pf::CONTRIBUTE_WITH_DEED, on));
                }
            });
        },
    );
    line(ui, p);
    // sale
    let region_xfer = !v.region_flag(rf::BLOCK_LAND_RESELL);
    let estate_sellable = parcel.auction_id == 0 && estate_manager && v.region.as_ref().is_some_and(|r| parcel.owner_id == r.owner);
    let owner_sellable = region_xfer && parcel.auction_id == 0 && v.can(powers::LAND_SET_SALE_INFO);
    let can_be_sold = owner_sellable || estate_sellable;
    let for_sale = parcel.flags & pf::FOR_SALE != 0;
    let mut status = if for_sale {
        let per_m = if parcel.area > 0 {
            parcel.sale_price as f32 / parcel.area as f32
        } else {
            0.0
        };
        let mut t = aurora_i18n::tr!(
            "extra-price-l-l-m",
            value0 = super::money(parcel.sale_price).to_string(),
            per_m = aurora_i18n::number_text(&format!("{:.1}", per_m))
        );
        let to = if parcel.auth_buyer.is_nil() {
            aurora_i18n::tr!("common-everyone").to_owned()
        } else {
            world.social.want_name(parcel.auth_buyer);
            world.social.name_of(&parcel.auth_buyer)
        };
        t.push_str(&format!(" · {}", aurora_i18n::tr!("ui-land-manage-sold-to", who = to)));
        t.push_str(" · ");
        t.push_str(if parcel.flags & pf::SELL_PARCEL_OBJECTS != 0 {
            aurora_i18n::tr!("ui-land-general-objects-included-in-sale")
        } else {
            aurora_i18n::tr!("ui-land-general-objects-not-included-in-sale")
        });
        t
    } else {
        aurora_i18n::tr!("ui-land-general-not-for-sale").to_owned()
    };
    if !is_public && !is_leased && parcel.owner_id == agent {
        status.push_str(&format!(
            " · {}",
            aurora_i18n::tr!("ui-land-general-you-must-approve-your-purchase-before-editing-this-land")
        ));
    } else if !is_public && parcel.auction_id != 0 {
        status.push_str(&format!(
            " · {}",
            aurora_i18n::tr!("ui-land-general-auction-id", value0 = parcel.auction_id.to_string())
        ));
    }
    srow(ui, p, aurora_i18n::tr!("ui-land-overview-sale"), &status, 200.0, |ui| {
        if for_sale {
            if button(ui, p, "", aurora_i18n::tr!("ui-land-general-cancel-land-sale"), can_be_sold).clicked() {
                // onClickStopSellLand
                world.land.update(|u| {
                    u.flags &= !pf::FOR_SALE;
                    u.sale_price = 0;
                    u.auth_buyer = Uuid::nil();
                });
            }
        } else if button(ui, p, "tag-filled", aurora_i18n::tr!("common-sell-land"), can_be_sold).clicked() {
            s.dialog = Some(Dialog::Sell(SellForm::new(parcel)));
        }
    });
    if !lock && !can_be_sold && !for_sale && !is_public {
        lock_line(ui, p, &missing_power(v, powers::LAND_SET_SALE_INFO).unwrap_or_default());
    }
    line(ui, p);
    let release = v.owns(powers::LAND_RELEASE) || (estate_manager && v.region.as_ref().is_some_and(|r| parcel.owner_id != r.owner));
    let acquired = if is_public {
        String::new()
    } else {
        super::claim_date(parcel.claim_date as i64)
    };
    srow(
        ui,
        p,
        aurora_i18n::tr!("ui-land-manage-ownership"),
        &aurora_i18n::tr!("ui-land-manage-acquired-on", date = acquired),
        360.0,
        |ui| {
            if region_owner {
                // reclaimParcel: the region owner takes back someone's parcel
                if button(
                    ui,
                    p,
                    "",
                    aurora_i18n::tr!("ui-land-general-reclaim-land"),
                    !is_public && parcel.owner_id != agent,
                )
                .clicked()
                {
                    world
                        .land
                        .parcel_command(|handle, local_id| LandCommand::Reclaim { handle, local_id });
                }
            } else if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("common-abandon-land"),
                widgets::ActionKind::FlatDanger,
                Vec2::new(0.0, 26.0),
                release,
            )
            .clicked()
            {
                s.dialog = Some(Dialog::Confirm(Confirm::Release));
            }
            // onClickBuyLand(true): for the active group, with its power
            let group = world.groups.active;
            let buy_group = !group.is_nil() && crate::world::land::can_agent_buy(parcel, &v.rights(), Some(group), true);
            if button(ui, p, "", aurora_i18n::tr!("ui-land-general-buy-for-group"), buy_group)
                .on_disabled_hover_text(aurora_i18n::tr!("ui-land-general-this-land-is-not-for-sale-to-you"))
                .clicked()
            {
                world.land.request_buy(true);
            }
            if v.region.as_ref().is_some_and(|r| r.has_land_resources)
                && button(ui, p, "code", aurora_i18n::tr!("ui-land-general-script-info"), true).clicked()
            {
                s.scripts.open();
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::GroupMembership;
    use aurora_net::land::OS_LEASE_PENDING;

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

    fn member(group: Uuid, powers: u64) -> GroupMembership {
        GroupMembership {
            id: group,
            name: "Aurora Builders".into(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    /// The banner says why nothing can be changed, and is gone as soon as
    /// one setting can.
    #[test]
    fn the_banner_names_who_may_change_the_parcel() {
        let (me, other, group) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(9));
        let leased = |owner: Uuid| ParcelInfo {
            owner_id: owner,
            group_id: group,
            is_group_owned: owner == group,
            status: OS_LEASED,
            ..Default::default()
        };
        assert_eq!(lock_banner(&view(leased(me), me, Vec::new()), false, "AB"), None, "the owner");
        assert!(
            lock_banner(&view(leased(me), me, Vec::new()), true, "AB").is_some(),
            "the owner's visitor preview"
        );
        let officer = view(leased(group), me, vec![member(group, powers::LAND_OPTIONS)]);
        assert_eq!(lock_banner(&officer, false, "AB"), None);
        let plain = view(leased(group), me, vec![member(group, 0)]);
        assert_eq!(
            lock_banner(&plain, false, "AB"),
            Some(aurora_i18n::tr!("ui-land-manage-lock-group", group = "AB".to_string()))
        );
        assert_eq!(
            lock_banner(&view(leased(other), me, Vec::new()), false, "AB").as_deref(),
            Some(aurora_i18n::tr!("ui-land-manage-lock-owner"))
        );
        // a purchase waiting for approval
        let pending = ParcelInfo {
            status: OS_LEASE_PENDING,
            ..leased(me)
        };
        assert_eq!(
            lock_banner(&view(pending, me, Vec::new()), false, "AB").as_deref(),
            Some(aurora_i18n::tr!(
                "ui-land-general-you-must-approve-your-purchase-before-editing-this-land"
            ))
        );
        // an officer without terraforming learns which power is missing
        assert_eq!(missing_power(&officer, powers::LAND_OPTIONS), None);
        assert_eq!(
            missing_power(&officer, powers::LAND_EDIT),
            Some(aurora_i18n::tr!(
                "ui-land-manage-missing-power",
                power = aurora_i18n::tr!("ui-land-power-edit").to_string()
            ))
        );
    }

    /// The access choice of the title sets the boxes of the Access tab.
    #[test]
    fn the_access_choice_is_the_access_checkboxes() {
        let open = AccessState {
            public: true,
            group: false,
            payment: false,
            age: true,
            pass: true,
            pass_group: false,
            price: 88,
            hours: 1.0,
        };
        assert_eq!(access_choice(&open), 0);
        let group = with_access_choice(open, 1, true);
        assert!(!group.public && group.group && group.age);
        assert_eq!(access_choice(&group), 1);
        // no group on the parcel: the list only
        let no_group = with_access_choice(open, 1, false);
        assert_eq!(access_choice(&no_group), 2);
        let list = with_access_choice(group, 2, true);
        assert!(!list.public && !list.group);
        assert_eq!(access_choice(&with_access_choice(list, 0, true)), 0);
    }

    #[test]
    fn the_split_bar_shows_every_part() {
        let w = split_widths([47, 12, 2], 3750, 750.0, 4.0);
        assert!((w[0] - 47.0 / 3750.0 * 750.0).abs() < 1e-3);
        assert_eq!(w[2], 4.0, "a few objects stay visible");
        assert!((w.iter().sum::<f32>() - 750.0).abs() < 1e-3);
        // over the capacity: the parts fill the bar
        let over = split_widths([100, 0, 0], 50, 100.0, 4.0);
        assert_eq!(over, [100.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            parcel_capacity(
                &ParcelInfo {
                    max_prims: 3750,
                    prim_bonus: 2.0,
                    ..Default::default()
                },
                5000
            ),
            5000
        );
    }

    #[test]
    fn landing_heading_like_the_options_tab() {
        assert_eq!(heading(Vec3::new(1.0, 0.0, 0.0)), 90);
        assert_eq!(heading(Vec3::new(0.0, 1.0, 0.0)), 0);
    }

    #[test]
    fn every_section_needs_a_named_power() {
        for sec in Section::ALL {
            assert!(!power_name(sec.power()).is_empty(), "{sec:?}");
        }
        assert_eq!(environment_choice(&EnvState::None), 0);
        let custom = crate::world::land::EnvSummary {
            has_day: true,
            ..Default::default()
        };
        assert_eq!(environment_choice(&EnvState::Ready(custom)), 1);
    }
}
