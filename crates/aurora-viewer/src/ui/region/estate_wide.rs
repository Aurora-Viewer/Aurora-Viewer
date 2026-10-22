//! "Manage › Environment", "Experiences" and "Covenant" of the Region /
//! Estate window: what holds for the whole region or estate, as the Legacy
//! Viewer's Environment, Experiences and Covenant tabs change it
//! (LLPanelRegionEnvironment / LLPanelEnvironmentInfo in newview/
//! llfloaterregioninfo.cpp and llpanelenvironment.cpp, LLPanelRegionExperiences,
//! LLPanelEstateCovenant, originally LGPL 2.1). The region environment goes
//! through ExtEnvironment without a parcel (reset, an inventory item for the
//! day or one track, the day's length and offset, the sky altitudes; "Edit
//! the day cycle…" opens the day editor in the region context), the
//! parcels' own environments are the estate flag ALLOW_ENVIRONMENT_OVERRIDE
//! (EstateParcelEnvironmentOverride when turned off, ChangeLindenEstate on a
//! Linden estate), the experiences `estateexperiencedelta` for this estate
//! or all of the owner's (the EstateTrusted / Allowed / BlockedExperience
//! questions), the covenant `estatechangecovenantid` with a notecard's
//! asset or nil (EstateChangeCovenant first). Who may: the estate's owner
//! and managers (canManageEstate).

use super::confirm::Ask;
use super::{RegionAction, RegionUi, View};
use crate::theme::Palette;
use crate::ui::inventory::InvDrag;
use crate::ui::sheet::{self, RowText, editable, line, semibold, srow, srow_ext, why};
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::env_select::{SettingsCatalog, SettingsKind};
use crate::world::estate::Role;
use crate::world::land::EnvSummary;
use aurora_net::estate::{ESTATE_LAST_LINDEN, ESTATE_MAX_EXPERIENCE_IDS, estate_access as ea, estate_experience as xe};
use aurora_net::land::{DEFAULT_SKY_ALTITUDES, FoundExperience, LandCommand, REGION_PARCEL_ID, XP_PROPERTY_GRID, XP_PROPERTY_PRIVILEGED};
use egui::{RichText, Sense, Vec2};
use uuid::Uuid;

/// Altitude slider range and step (sld_altitudes).
const ALT_MIN: f32 = 100.0;
const ALT_MAX: f32 = 4000.0;
/// Height of the covenant before "Read all".
const COVENANT_EXCERPT: f32 = 90.0;

/// The region's three experience lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XpList {
    Trusted,
    Allowed,
    Blocked,
}

impl XpList {
    const ALL: [XpList; 3] = [XpList::Trusted, XpList::Allowed, XpList::Blocked];

    /// ESTATE_EXPERIENCE_*_ADD / _REMOVE.
    fn flag(self, add: bool) -> u32 {
        match (self, add) {
            (XpList::Trusted, true) => xe::TRUSTED_ADD,
            (XpList::Trusted, false) => xe::TRUSTED_REMOVE,
            (XpList::Allowed, true) => xe::ALLOWED_ADD,
            (XpList::Allowed, false) => xe::ALLOWED_REMOVE,
            (XpList::Blocked, true) => xe::BLOCKED_ADD,
            (XpList::Blocked, false) => xe::BLOCKED_REMOVE,
        }
    }

    fn label(self) -> &'static str {
        match self {
            XpList::Trusted => aurora_i18n::tr!("region-xp-trusted"),
            XpList::Allowed => aurora_i18n::tr!("region-xp-allowed"),
            XpList::Blocked => aurora_i18n::tr!("region-xp-blocked"),
        }
    }

    fn desc(self) -> &'static str {
        match self {
            XpList::Trusted => aurora_i18n::tr!("region-xp-trusted-desc"),
            XpList::Allowed => aurora_i18n::tr!("region-xp-allowed-desc"),
            XpList::Blocked => aurora_i18n::tr!("region-xp-blocked-desc"),
        }
    }
}

/// The dialog on screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Dialog {
    /// LLFloaterSettingsPicker for the region: the whole day (`track`
    /// None) or one track (0 water, 1 ground sky, 2–4 skies).
    Environment {
        track: Option<i32>,
        filter: String,
        selected: Option<Uuid>,
    },
    /// The experience picker of a list.
    Experience {
        list: XpList,
        query: String,
        maturity: u8,
        selected: Option<Uuid>,
    },
    /// "This estate" or "all estates" (Estate…ExperienceAdd / Remove).
    Scope { list: XpList, add: bool, id: Uuid, all: bool },
    /// A notecard of the inventory for the covenant.
    Notecard { filter: String, selected: Option<Uuid> },
}

impl Dialog {
    pub(crate) fn environment() -> Dialog {
        Dialog::Environment {
            track: None,
            filter: String::new(),
            selected: None,
        }
    }

    pub(crate) fn experience(list: XpList) -> Dialog {
        Dialog::Experience {
            list,
            query: String::new(),
            maturity: 42,
            selected: None,
        }
    }

    pub(crate) fn notecard() -> Dialog {
        Dialog::Notecard {
            filter: String::new(),
            selected: None,
        }
    }
}

/// What the three sections keep between frames.
#[derive(Default)]
pub(super) struct EstateWideState {
    pub dialog: Option<Dialog>,
    /// AURORA_DEMO_REGION's dialog, shown once the simulator answered.
    pub demo: Option<Dialog>,
    /// The day's length and offset (hours) and the sky altitudes being
    /// dragged, sent once let go (commitDayLenOffsetChanges, onAltSliderMouseUp).
    day_drag: Option<(f32, f32)>,
    alt_drag: Option<[f32; 3]>,
    advanced: bool,
    covenant_open: bool,
}

impl EstateWideState {
    pub fn reset(&mut self) {
        self.dialog = None;
        self.day_drag = None;
        self.alt_drag = None;
    }
}

fn is_linden(world: &World) -> bool {
    world.estate.estate.as_ref().is_some_and(|e| e.id <= ESTATE_LAST_LINDEN)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The sky altitudes of skies 2 to 4 (`EnvSummary::altitudes` 1..4).
fn sky_altitudes(e: Option<&EnvSummary>) -> [f32; 3] {
    e.map_or(DEFAULT_SKY_ALTITUDES, |e| [e.altitudes[1], e.altitudes[2], e.altitudes[3]])
}

/// The day's length and offset in seconds as ExtEnvironment takes them
/// (commitDayLenOffsetChanges: a negative offset is a day later).
pub(crate) fn day_seconds(length_h: f32, offset_h: f32) -> (i32, i32) {
    let length = (length_h * 3600.0).round() as i32;
    let mut offset = (offset_h * 3600.0).round() as i32;
    if offset <= 0 {
        offset += 24 * 3600;
    }
    (length, offset)
}

/// The settings kinds a track takes: any for the whole day, water for
/// track 0, a sky for the others.
fn settings_kinds(track: Option<i32>) -> &'static [SettingsKind] {
    match track {
        None => &[SettingsKind::Day, SettingsKind::Sky, SettingsKind::Water],
        Some(0) => &[SettingsKind::Water],
        Some(_) => &[SettingsKind::Sky],
    }
}

/// "Environment": the region's day, the parcels' permission and the
/// day's timing and sky altitudes.
pub(super) fn environment(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let summary = world.estate.environment.clone().flatten();
    let answered = world.estate.environment.is_some();
    let waiting = world.estate.environment_waiting;
    // canAgentUpdateRegionEnvironment: the estate's owner and managers
    let can = v.manages() && answered && !waiting;
    let options = [
        (aurora_i18n::tr!("region-env-default"), true),
        (aurora_i18n::tr!("ui-land-manage-env-inventory"), true),
        (aurora_i18n::tr!("ui-land-manage-env-custom"), true),
    ];
    let named = summary.as_ref().is_some_and(|e| e.has_day && !e.day_name.is_empty());
    sheet::section_title(ui, p, "sun-high-filled", aurora_i18n::tr!("common-environment"), |ui| {
        if !v.manages() {
            return;
        }
        editable(ui, can, |ui| {
            let current = usize::from(named);
            let mut choice = current;
            let r = ui.scope(|ui| widgets::segmented(ui, p, &mut choice, &options)).response;
            let clicked = r.hovered() && ui.input(|i| i.pointer.primary_clicked());
            match choice {
                // onBtnDefault: back to the grid's default, confirmed
                0 if current != 0 => s.ask = Some(Ask::ResetEnvironment),
                // onBtnSelect: the settings picker for the whole day
                1 if current != 1 || clicked => s.estate_wide.dialog = Some(Dialog::environment()),
                // onBtnEdit: the day editor on the region's day
                2 => s.action = Some(RegionAction::CustomizeEnvironment),
                _ => {}
            }
            r.on_hover_text(aurora_i18n::tr!("region-env-choice-tip"));
        });
    });
    match (&world.estate.environment_error, answered, waiting) {
        (Some(e), ..) => why(ui, p, &aurora_i18n::tr!("region-env-refused", reason = e.clone())),
        (_, false, _) | (_, _, true) => why(ui, p, aurora_i18n::tr!("common-loading")),
        _ => why(ui, p, aurora_i18n::tr!("region-env-why")),
    }
    if !answered {
        return;
    }
    // the day: its name, timing and apparent time
    let name = match &summary {
        Some(e) if named => aurora_i18n::tr!("region-env-day-named", name = e.day_name.clone()),
        Some(e) if e.has_day => aurora_i18n::tr!("region-day-cycle").to_owned(),
        _ => aurora_i18n::tr!("region-default-environment").to_owned(),
    };
    let (length_h, offset_h) = crate::ui::land::day_hours(summary.as_ref(), s.estate_wide.day_drag);
    let mut desc = aurora_i18n::tr!(
        "region-env-day-desc",
        length = aurora_i18n::number(f64::from(length_h), 1),
        offset = aurora_i18n::number(f64::from(offset_h), 1)
    );
    if let Some((h, m, pct)) = summary.as_ref().filter(|e| e.has_day).and_then(|e| e.apparent_time(now())) {
        desc.push(' ');
        desc.push_str(&aurora_i18n::tr!(
            "region-env-apparent",
            time = format!("{h}:{m:02}"),
            percent = pct
        ));
    }
    srow(ui, p, &name, &desc, 200.0, |ui| {
        if v.manages() {
            let label = aurora_i18n::tr!("region-env-edit-day");
            if widgets::action_button_if(ui, p, "pencil-filled", label, ActionKind::Flat, Vec2::new(0.0, 28.0), can).clicked() {
                s.action = Some(RegionAction::CustomizeEnvironment);
            }
        }
    });
    line(ui, p);
    // the estate's flag (LLEstateInfoModel::getAllowEnvironmentOverride)
    let estate = world.estate.estate.clone();
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-env-parcels"),
        aurora_i18n::tr!("region-env-parcels-desc"),
        40.0,
        |ui| {
            let mut on = estate
                .as_ref()
                .is_some_and(|e| e.flags & aurora_net::estate::estate_flags::ALLOW_ENVIRONMENT_OVERRIDE != 0);
            if editable(ui, v.manages() && estate.is_some(), |ui| widgets::switch(ui, p, &mut on).changed()) {
                // onChkAllowOverride: asked when turned off or on a Linden estate
                if !on || is_linden(world) {
                    s.ask = Some(Ask::EnvironmentOverride { allow: on });
                } else {
                    set_override(v, world, on);
                }
            }
        },
    );
    sheet::fold_row(
        ui,
        p,
        &mut s.estate_wide.advanced,
        aurora_i18n::tr!("ui-land-manage-environment-advanced"),
    );
    if s.estate_wide.advanced {
        advanced(ui, p, s, world, summary.as_ref(), can);
    }
}

/// The day's length and offset, the sky altitudes and the tracks.
fn advanced(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, world: &mut World, summary: Option<&EnvSummary>, can: bool) {
    let has_day = summary.is_some_and(|e| e.has_day);
    let (mut length_h, mut offset_h) = crate::ui::land::day_hours(summary, s.estate_wide.day_drag);
    let mut dragging = false;
    let mut changed = false;
    let mut slider =
        |ui: &mut egui::Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, step: f64, decimals: usize, enabled: bool| {
            let r = editable(ui, enabled, |ui| {
                ui.add(
                    egui::Slider::new(value, range)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(step)
                        .fixed_decimals(decimals),
                )
            });
            dragging |= r.dragged() || r.has_focus();
            changed |= r.changed();
        };
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-environment-day-length-hours"), "", 300.0, |ui| {
        slider(ui, &mut length_h, 4.0..=168.0, 0.5, 1, can && has_day);
    });
    line(ui, p);
    srow(ui, p, aurora_i18n::tr!("ui-land-environment-time-offset-hours"), "", 300.0, |ui| {
        slider(ui, &mut offset_h, -11.5..=12.0, 0.5, 1, can && has_day);
    });
    if dragging || changed {
        s.estate_wide.day_drag = Some((length_h, offset_h));
    }
    if !dragging && s.estate_wide.day_drag.take().is_some() {
        let (day_length, day_offset) = day_seconds(length_h, offset_h);
        world.estate.change_environment(LandCommand::EnvironmentUpdate {
            local_id: REGION_PARCEL_ID,
            day_length,
            day_offset,
        });
    }
    // the sky altitudes, highest first, each with its track's settings
    let mut alts = s.estate_wide.alt_drag.unwrap_or_else(|| sky_altitudes(summary));
    let mut alt_dragging = false;
    let mut alt_changed = false;
    let name = |i: i32| summary.map_or_else(|| aurora_i18n::tr!("status-empty").to_owned(), |e| e.track_name(i));
    for sky in (2..=4).rev() {
        line(ui, p);
        let label = aurora_i18n::tr!("region-env-sky", sky = sky.to_string());
        srow(ui, p, &label, &name(sky), 300.0, |ui| {
            if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("ui-land-manage-choose"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                can,
            )
            .clicked()
            {
                s.estate_wide.dialog = Some(Dialog::Environment {
                    track: Some(sky),
                    filter: String::new(),
                    selected: None,
                });
            }
            let r = editable(ui, can, |ui| {
                ui.add(
                    egui::Slider::new(&mut alts[(sky - 2) as usize], ALT_MIN..=ALT_MAX)
                        .step_by(25.0)
                        .fixed_decimals(0)
                        .suffix(" m"),
                )
            });
            alt_dragging |= r.dragged() || r.has_focus();
            alt_changed |= r.changed();
        });
    }
    if alt_dragging || alt_changed {
        s.estate_wide.alt_drag = Some(alts);
    }
    let (day_length, day_offset) = summary.map_or((-1, -1), |e| (e.day_length as i32, e.day_offset as i32));
    if !alt_dragging && s.estate_wide.alt_drag.take().is_some() {
        // onAltSliderMouseUp: the day as it is, the three altitudes
        world.estate.change_environment(LandCommand::EnvironmentAltitudes {
            local_id: REGION_PARCEL_ID,
            day_length,
            day_offset,
            altitudes: alts,
        });
    }
    for (label, track) in [
        (aurora_i18n::tr!("ui-land-environment-ground"), 1),
        (aurora_i18n::tr!("common-water"), 0),
    ] {
        line(ui, p);
        srow(ui, p, label, &name(track), 120.0, |ui| {
            if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("ui-land-manage-choose"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                can,
            )
            .clicked()
            {
                s.estate_wide.dialog = Some(Dialog::Environment {
                    track: Some(track),
                    filter: String::new(),
                    selected: None,
                });
            }
        });
    }
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-env-reset-altitudes"),
        aurora_i18n::tr!("region-env-reset-altitudes-desc"),
        120.0,
        |ui| {
            let at_default = sky_altitudes(summary) == DEFAULT_SKY_ALTITUDES;
            if widgets::action_button_if(
                ui,
                p,
                "arrow-back-up",
                aurora_i18n::tr!("common-reset"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                can && !at_default,
            )
            .clicked()
            {
                // onBtnRstAltitudes
                world.estate.change_environment(LandCommand::EnvironmentAltitudes {
                    local_id: REGION_PARCEL_ID,
                    day_length,
                    day_offset,
                    altitudes: DEFAULT_SKY_ALTITUDES,
                });
            }
        },
    );
}

/// confirmUpdateEstateEnvironment: the estate model takes the flag, then
/// sendEstateInfo.
pub(super) fn set_override(v: &View, world: &mut World, allow: bool) {
    let Some(estate) = world.estate.estate.clone() else {
        return;
    };
    let mut info = (*estate).clone();
    let flag = aurora_net::estate::estate_flags::ALLOW_ENVIRONMENT_OVERRIDE;
    if allow {
        info.flags |= flag;
    } else {
        info.flags &= !flag;
    }
    world.estate.set_estate(v.handle, info);
}

/// resetRegion (SettingsConfirmReset answered yes).
pub(super) fn reset_environment(world: &mut World) {
    world.estate.change_environment(LandCommand::EnvironmentReset {
        local_id: REGION_PARCEL_ID,
    });
}

/// The ids of a list as the panel shows them: the default experience is
/// the trusted list's last, and cannot be removed (setStickyFunction).
fn shown_ids(world: &World, list: XpList) -> (Vec<Uuid>, Option<Uuid>) {
    let Some(Some(x)) = world.estate.experiences.as_ref() else {
        return (Vec::new(), None);
    };
    match list {
        XpList::Trusted => {
            let mut ids = x.trusted.clone();
            if let Some(d) = x.default.filter(|d| !ids.contains(d)) {
                ids.push(d);
            }
            (ids, x.default)
        }
        XpList::Allowed => (x.allowed.clone(), None),
        XpList::Blocked => (x.blocked.clone(), None),
    }
}

/// The experience picker's filters of a list (refreshFromRegion's
/// addFilter): allowed ones are not grid-wide, blocked ones are grid-wide
/// but not privileged; neither takes the default experience; none shows
/// one already there or above the chosen rating.
pub(crate) fn experience_hidden(e: &FoundExperience, list: XpList, maturity: u8, there: &[Uuid], default: Option<Uuid>) -> bool {
    let grid = e.properties & XP_PROPERTY_GRID != 0;
    let by_list = match list {
        XpList::Trusted => false,
        XpList::Allowed => grid || default == Some(e.id),
        XpList::Blocked => !grid || e.properties & XP_PROPERTY_PRIVILEGED != 0 || default == Some(e.id),
    };
    by_list || e.maturity > maturity || there.contains(&e.id)
}

/// "Experiences": the three lists, eight each.
pub(super) fn experiences(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    sheet::section_title(ui, p, "flask-filled", aurora_i18n::tr!("ui-land-manage-experiences"), |_| {});
    why(ui, p, aurora_i18n::tr!("region-xp-why"));
    let Some(answered) = world.estate.experiences.clone() else {
        why(ui, p, aurora_i18n::tr!("common-loading"));
        return;
    };
    if answered.is_none() {
        why(ui, p, aurora_i18n::tr!("region-xp-unavailable"));
        return;
    }
    let can = v.manages();
    for (n, list) in XpList::ALL.into_iter().enumerate() {
        if n > 0 {
            line(ui, p);
        }
        let (ids, sticky) = shown_ids(world, list);
        let count = ids.len();
        let name = format!(
            "{}  {}",
            list.label(),
            aurora_i18n::tr!("region-xp-count", count = count as i64, max = ESTATE_MAX_EXPERIENCE_IDS as i64)
        );
        srow(ui, p, &name, list.desc(), 110.0, |ui| {
            if can {
                let full = count >= ESTATE_MAX_EXPERIENCE_IDS;
                let r = widgets::action_button_if(
                    ui,
                    p,
                    "plus",
                    aurora_i18n::tr!("common-add"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 24.0),
                    !full,
                );
                if r.clicked() {
                    s.estate_wide.dialog = Some(Dialog::experience(list));
                }
            }
        });
        for id in ids {
            let label = world
                .land
                .experience_name(&id)
                .unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
            let fixed = sticky == Some(id);
            if xp_row(ui, p, &label, fixed, can && !fixed) {
                s.estate_wide.dialog = remove_dialog(v, world, list, id);
            }
        }
    }
}

/// An experience of a list: its name, "Default" for the grid's, the x
/// that removes it. True when the x was clicked.
fn xp_row(ui: &mut egui::Ui, p: &Palette, name: &str, fixed: bool, removable: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    if r.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.35));
    }
    sheet::icon_at(
        ui,
        "flask-filled",
        egui::pos2(rect.left() + 14.0, rect.center().y),
        14.0,
        p.violet_light,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.5),
        p.ink,
    );
    if fixed {
        ui.painter().text(
            egui::pos2(rect.right() - 10.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            aurora_i18n::tr!("region-xp-default"),
            egui::FontId::proportional(11.5),
            p.muted,
        );
        return false;
    }
    if !removable {
        return false;
    }
    let x = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), Vec2::splat(20.0));
    let xr = ui.interact(x, ui.scope_id().with(("region_xp_remove", name, rect.top() as i32)), Sense::click());
    sheet::icon_at(ui, "x", x.center(), 14.0, if xr.hovered() { p.danger } else { p.muted });
    xr.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("common-remove"))
        .clicked()
}

/// "Covenant": its estate, its date, its text, and the notecard that
/// replaces it.
pub(super) fn covenant(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let can = v.manages();
    sheet::section_title(ui, p, "gavel", aurora_i18n::tr!("region-manage-covenant"), |ui| {
        if can {
            let label = aurora_i18n::tr!("region-cov-reset");
            if widgets::action_button(ui, p, "", label, ActionKind::Danger, Vec2::new(0.0, 24.0)).clicked() {
                s.ask = Some(Ask::Covenant { asset: Uuid::nil() });
            }
        }
    });
    why(ui, p, aurora_i18n::tr!("region-cov-why"));
    let covenant = world.land.covenant.clone().filter(|c| c.handle == v.handle);
    let Some(c) = covenant else {
        why(ui, p, aurora_i18n::tr!("common-loading"));
        return;
    };
    let estate = if c.estate_name.is_empty() {
        v.name.clone()
    } else {
        c.estate_name.clone()
    };
    let when = if c.timestamp == 0 {
        aurora_i18n::tr!("ui-land-overview-never-modified").to_owned()
    } else {
        aurora_i18n::tr!("ui-land-overview-modified-on", date = crate::ui::land::day(i64::from(c.timestamp)))
    };
    let body = c.text.clone().unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
    let name = if c.id.is_nil() {
        aurora_i18n::tr!("region-cov-none").to_owned()
    } else {
        aurora_i18n::tr!("region-cov-of", estate = estate)
    };
    let text = RowText {
        name: &name,
        lock: None,
        desc: &when,
        link: None,
    };
    let open = s.estate_wide.covenant_open;
    srow_ext(ui, p, &text, 120.0, |ui| {
        let label = if open {
            aurora_i18n::tr!("region-cov-read-less")
        } else {
            aurora_i18n::tr!("region-cov-read-all")
        };
        if widgets::action_button(ui, p, "", label, ActionKind::Flat, Vec2::new(0.0, 24.0)).clicked() {
            s.estate_wide.covenant_open = !open;
        }
    });
    ui.add_space(4.0);
    sheet::excerpt(ui, p, &body, open, COVENANT_EXCERPT);
    if can {
        ui.add_space(8.0);
        drop_zone(ui, p, s, world);
    }
}

/// The notecard drop target (LLPanelEstateCovenant::handleDragAndDrop) and
/// the picker that does the same without a drag.
fn drop_zone(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, world: &mut World) {
    let height = 46.0;
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let carried = egui::DragAndDrop::payload::<InvDrag>(ui.ctx()).and_then(|d| world.inventory.items.get(&d.0).map(|i| i.asset_type == 7));
    let over = carried.is_some() && r.contains_pointer();
    let color = match carried {
        Some(true) if over => p.violet_light,
        Some(false) if over => p.danger,
        _ => p.muted_dim,
    };
    let radius = ui.visuals().window_corner_radius;
    ui.painter()
        .rect_stroke(rect, radius, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
    sheet::icon_at(ui, "file-import", egui::pos2(rect.left() + 20.0, rect.center().y), 16.0, p.muted);
    ui.painter().text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        aurora_i18n::tr!("region-cov-drop"),
        egui::FontId::proportional(12.5),
        p.muted,
    );
    let button = egui::Rect::from_min_size(egui::pos2(rect.right() - 170.0, rect.center().y - 14.0), Vec2::new(160.0, 28.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(button)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let label = aurora_i18n::tr!("region-cov-choose");
    if widgets::action_button(&mut child, p, "", label, ActionKind::Flat, Vec2::new(0.0, 28.0)).clicked() {
        s.estate_wide.dialog = Some(Dialog::notecard());
    }
    if let Some(d) = r.dnd_release_payload::<InvDrag>()
        && let Some(item) = world.inventory.items.get(&d.0)
        && item.asset_type == 7
    {
        // EstateChangeCovenant first
        s.ask = Some(Ask::Covenant { asset: item.asset_id });
    }
}

/// The covenant replaced (or cleared) once confirmed, then asked again.
pub(super) fn change_covenant(v: &View, world: &mut World, asset: Uuid) {
    let current = world
        .land
        .covenant
        .as_ref()
        .filter(|c| c.handle == v.handle)
        .map_or(Uuid::nil(), |c| c.id);
    world.estate.change_covenant(v.handle, current, asset);
    let request = world.land.covenant_request(v.handle);
    world.estate.push(request);
}

/// The dialogs of the three sections.
pub(super) fn dialogs(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    if let Some(d) = s.estate_wide.demo.take() {
        // once the lists are there
        if world.estate.experiences.is_some() && world.estate.environment.is_some() {
            s.estate_wide.dialog = Some(d);
        } else {
            s.estate_wide.demo = Some(d);
        }
    }
    let Some(dialog) = s.estate_wide.dialog.take() else {
        return;
    };
    s.estate_wide.dialog = show_dialog(ctx, p, s, v, world, dialog);
}

fn show_dialog(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World, mut dialog: Dialog) -> Option<Dialog> {
    let mut answer: Option<bool> = None;
    let modal = widgets::Modal::new(egui::Id::unique("region_estate_wide")).show(ctx, |ui| {
        ui.set_width(420.0);
        match &mut dialog {
            Dialog::Environment { track, filter, selected } => {
                if super::interventions::head(
                    ui,
                    p,
                    "sun-high-filled",
                    aurora_i18n::tr!("ui-land-picker-environment-title"),
                    false,
                ) {
                    answer = Some(false);
                }
                let loading = crate::world::env_select::scan_step(&mut world.inventory) > 0;
                let order = crate::ui::inventory::view::picker_order(ui.ctx(), &world.inventory);
                let catalog = SettingsCatalog::build(&world.inventory, |id| order.rank(id));
                ui.horizontal(|ui| widgets::search_box(ui, p, filter, aurora_i18n::tr!("common-search"), ui.available_width()));
                ui.add_space(4.0);
                let needle = filter.trim().to_lowercase();
                let mut double = false;
                egui::ScrollArea::vertical()
                    .id_salt("region_env_picker")
                    .max_height(240.0)
                    .min_scrolled_height(240.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut any = false;
                        for (kind, c) in catalog.choices(settings_kinds(*track), |id| order.rank(id)) {
                            let icon = match kind {
                                SettingsKind::Day => "sunrise-filled",
                                SettingsKind::Sky => "sun-filled",
                                SettingsKind::Water => "droplet-filled",
                            };
                            if !needle.is_empty() && !c.name.to_lowercase().contains(&needle) {
                                continue;
                            }
                            any = true;
                            let r = crate::ui::land::pick_row(ui, p, icon, &c.name, *selected == Some(c.asset));
                            if r.clicked() {
                                *selected = Some(c.asset);
                            }
                            if r.double_clicked() {
                                *selected = Some(c.asset);
                                double = true;
                            }
                        }
                        if !any {
                            let text = if loading {
                                aurora_i18n::tr!("common-loading")
                            } else {
                                aurora_i18n::tr!("ui-land-picker-environment-none")
                            };
                            ui.label(RichText::new(text).size(12.0).color(p.muted));
                        }
                    });
                let ok = selected.is_some();
                if let Some(main) = super::interventions::footer(
                    ui,
                    p,
                    "",
                    aurora_i18n::tr!("ui-land-picker-environment-apply"),
                    ActionKind::Primary,
                    ok,
                ) {
                    answer = Some(main);
                }
                if double {
                    answer = Some(true);
                }
                if answer == Some(true)
                    && let Some(asset) = *selected
                {
                    let name = settings_kinds(*track)
                        .iter()
                        .find_map(|k| catalog.name_of(*k, asset))
                        .unwrap_or_default()
                        .to_owned();
                    apply_settings(world, *track, asset, name);
                }
            }
            Dialog::Experience {
                list,
                query,
                maturity,
                selected,
            } => {
                if super::interventions::head(ui, p, "flask-filled", aurora_i18n::tr!("ui-land-picker-experience-title"), false) {
                    answer = Some(false);
                }
                let mut search = false;
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 90.0;
                    let r = widgets::search_box(ui, p, query, aurora_i18n::tr!("ui-land-picker-experience-hint"), w);
                    search = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("common-search")).clicked() {
                        search = true;
                    }
                });
                if search {
                    world.land.search_experiences(query);
                    *selected = None;
                }
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-land-picker-experience-rating"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    let ratings = [13u8, 21, 42];
                    let mut choice = ratings.iter().position(|r| *r >= *maturity).unwrap_or(2);
                    let labels = [
                        (aurora_i18n::tr!("common-general"), true),
                        (aurora_i18n::tr!("common-moderate"), true),
                        (aurora_i18n::tr!("common-adult"), true),
                    ];
                    if widgets::segmented(ui, p, &mut choice, &labels) {
                        *maturity = ratings[choice];
                    }
                });
                ui.label(RichText::new(list.desc()).size(11.5).color(p.muted));
                ui.add_space(4.0);
                let (there, default) = shown_ids(world, *list);
                let results = world.land.experience_search.clone();
                let mut double = false;
                egui::ScrollArea::vertical()
                    .id_salt("region_xp_picker")
                    .max_height(220.0)
                    .min_scrolled_height(220.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let dim = |ui: &mut egui::Ui, t: &str| {
                            ui.label(RichText::new(t).size(12.0).color(p.muted));
                        };
                        match results {
                            None => dim(ui, aurora_i18n::tr!("ui-land-picker-experience-start")),
                            Some((_, None)) => dim(ui, aurora_i18n::tr!("common-search")),
                            Some((_, Some(None))) => dim(ui, aurora_i18n::tr!("ui-land-picker-experience-failed")),
                            Some((_, Some(Some(found)))) => {
                                let shown: Vec<&FoundExperience> = found
                                    .iter()
                                    .filter(|e| !experience_hidden(e, *list, *maturity, &there, default))
                                    .collect();
                                if shown.is_empty() {
                                    dim(ui, aurora_i18n::tr!("common-no-results"));
                                }
                                for e in shown {
                                    let label = if e.name.is_empty() {
                                        aurora_i18n::tr!("ui-land-picker-experience-untitled").to_owned()
                                    } else {
                                        e.name.clone()
                                    };
                                    let r = crate::ui::land::pick_row(ui, p, "flask-filled", &label, *selected == Some(e.id));
                                    if r.clicked() {
                                        *selected = Some(e.id);
                                    }
                                    if r.double_clicked() {
                                        *selected = Some(e.id);
                                        double = true;
                                    }
                                }
                            }
                        }
                    });
                let ok = selected.is_some();
                if let Some(main) = super::interventions::footer(ui, p, "", aurora_i18n::tr!("common-choose"), ActionKind::Primary, ok) {
                    answer = Some(main);
                }
                if double {
                    answer = Some(true);
                }
            }
            Dialog::Scope { list, add, id, all } => {
                let title = match (*list, *add) {
                    (XpList::Trusted, true) => aurora_i18n::tr!("region-xp-ask-add-trusted"),
                    (XpList::Allowed, true) => aurora_i18n::tr!("region-xp-ask-add-allowed"),
                    (XpList::Blocked, true) => aurora_i18n::tr!("region-xp-ask-add-blocked"),
                    (XpList::Trusted, false) => aurora_i18n::tr!("region-xp-ask-remove-trusted"),
                    (XpList::Allowed, false) => aurora_i18n::tr!("region-xp-ask-remove-allowed"),
                    (XpList::Blocked, false) => aurora_i18n::tr!("region-xp-ask-remove-blocked"),
                };
                if super::interventions::head(ui, p, "flask-filled", title, *list == XpList::Blocked && *add) {
                    answer = Some(false);
                }
                let name = world
                    .land
                    .experience_name(id)
                    .unwrap_or_else(|| aurora_i18n::tr!("common-loading").to_owned());
                let text = if *add {
                    aurora_i18n::tr!("region-xp-ask-add", name = name, list = list.label())
                } else {
                    aurora_i18n::tr!("region-xp-ask-remove", name = name, list = list.label())
                };
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
                ui.add_space(10.0);
                ui.label(semibold(ui, aurora_i18n::tr!("region-people-applies-to"), 11.5, p.muted));
                ui.add_space(2.0);
                // all_estates_text
                let all_label = match v.role {
                    Role::Owner => aurora_i18n::tr!("region-people-all-owned").to_owned(),
                    _ => aurora_i18n::tr!("region-people-all-managed", owner = sheet::names(world, &v.owner).0),
                };
                widgets::radio_value(ui, p, all, false, aurora_i18n::tr!("region-people-this-estate"));
                widgets::radio_value(ui, p, all, true, all_label.as_str());
                let (label, kind) = if *add {
                    (aurora_i18n::tr!("common-add"), ActionKind::Primary)
                } else {
                    (aurora_i18n::tr!("common-remove"), ActionKind::Primary)
                };
                if let Some(main) = super::interventions::footer(ui, p, "", label, kind, true) {
                    answer = Some(main);
                }
            }
            Dialog::Notecard { filter, selected } => {
                if super::interventions::head(ui, p, "file-text-filled", aurora_i18n::tr!("region-cov-choose-title"), false) {
                    answer = Some(false);
                }
                ui.horizontal(|ui| widgets::search_box(ui, p, filter, aurora_i18n::tr!("common-search"), ui.available_width()));
                ui.add_space(4.0);
                let needle = filter.trim().to_lowercase();
                let mut cards: Vec<(Uuid, String)> = world
                    .inventory
                    .items
                    .values()
                    .filter(|i| i.asset_type == 7 && (needle.is_empty() || i.name.to_lowercase().contains(&needle)))
                    .map(|i| (i.id, i.name.clone()))
                    .collect();
                let order = crate::ui::inventory::view::picker_order(ctx, &world.inventory);
                cards.sort_by_key(|(id, _)| order.rank(*id));
                let mut double = false;
                egui::ScrollArea::vertical()
                    .id_salt("region_cov_picker")
                    .max_height(240.0)
                    .min_scrolled_height(240.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if cards.is_empty() {
                            ui.label(RichText::new(aurora_i18n::tr!("region-cov-no-notecard")).size(12.0).color(p.muted));
                        }
                        for (id, name) in &cards {
                            let r = crate::ui::land::pick_row(ui, p, "file-text-filled", name, *selected == Some(*id));
                            if r.clicked() {
                                *selected = Some(*id);
                            }
                            if r.double_clicked() {
                                *selected = Some(*id);
                                double = true;
                            }
                        }
                    });
                let ok = selected.is_some();
                if let Some(main) = super::interventions::footer(ui, p, "", aurora_i18n::tr!("common-choose"), ActionKind::Primary, ok) {
                    answer = Some(main);
                }
                if double {
                    answer = Some(true);
                }
            }
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    match answer {
        None => Some(dialog),
        Some(false) => None,
        Some(true) => match dialog {
            Dialog::Environment { .. } => None,
            Dialog::Experience { list, selected, .. } => {
                let id = selected?;
                scope_or_send(v, world, list, true, id)
            }
            Dialog::Scope { list, add, id, all } => {
                send_experience(v, world, list, add, id, all);
                None
            }
            Dialog::Notecard { selected, .. } => {
                let asset = selected.and_then(|id| world.inventory.items.get(&id)).map(|i| i.asset_id)?;
                s.ask = Some(Ask::Covenant { asset });
                None
            }
        },
    }
}

/// itemChanged: a Linden estate's change is this estate's alone, asked of
/// nobody (forceResponse 0); others ask "this estate or all of them".
fn scope_or_send(v: &View, world: &mut World, list: XpList, add: bool, id: Uuid) -> Option<Dialog> {
    if is_linden(world) {
        send_experience(v, world, list, add, id, false);
        return None;
    }
    Some(Dialog::Scope { list, add, id, all: false })
}

/// experienceCoreConfirm: "all estates" is the owner's APPLY_TO_ALL_ESTATES
/// or a manager's APPLY_TO_MANAGED_ESTATES.
fn send_experience(v: &View, world: &mut World, list: XpList, add: bool, id: Uuid, all: bool) {
    let mut flags = list.flag(add);
    if all {
        flags |= match v.role {
            Role::Owner => ea::APPLY_TO_ALL_ESTATES,
            _ => ea::APPLY_TO_MANAGED_ESTATES,
        };
    }
    world.estate.change_experiences(v.handle, flags, vec![id]);
}

/// onPickerCommitted for the region: the day as it is, with the item's
/// settings for the whole day or one track.
fn apply_settings(world: &mut World, track: Option<i32>, asset: Uuid, name: String) {
    let owner_mask = world
        .inventory
        .items
        .values()
        .find(|i| i.asset_id == asset)
        .map_or(u32::MAX, |i| i.owner_mask);
    let (day_length, day_offset) = world
        .estate
        .environment
        .clone()
        .flatten()
        .map_or((-1, -1), |e| (e.day_length as i32, e.day_offset as i32));
    world.estate.change_environment(LandCommand::EnvironmentApply {
        local_id: REGION_PARCEL_ID,
        track,
        asset,
        name,
        flags: crate::ui::land::settings_flags(owner_mask),
        day_length,
        day_offset,
    });
}

/// The removal of an experience asks the same question as an addition.
pub(super) fn remove_dialog(v: &View, world: &mut World, list: XpList, id: Uuid) -> Option<Dialog> {
    scope_or_send(v, world, list, false, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xp(n: u128, maturity: u8, properties: i32) -> FoundExperience {
        FoundExperience {
            id: Uuid::from_u128(n),
            name: String::new(),
            owner: Uuid::nil(),
            maturity,
            properties,
        }
    }

    /// refreshFromRegion's filters: allowed takes land-scope ones, blocked
    /// grid-wide ones that are not privileged, trusted any; none the
    /// default experience (but trusted), one already there or over the rating.
    #[test]
    fn each_list_takes_its_experiences() {
        let land = xp(1, 13, 0);
        let grid = xp(2, 13, XP_PROPERTY_GRID);
        let privileged = xp(3, 13, XP_PROPERTY_GRID | XP_PROPERTY_PRIVILEGED);
        let adult = xp(4, 42, 0);
        let hidden = |e: &FoundExperience, list| experience_hidden(e, list, 21, &[], None);
        assert!(!hidden(&land, XpList::Allowed) && hidden(&grid, XpList::Allowed));
        assert!(hidden(&land, XpList::Blocked) && !hidden(&grid, XpList::Blocked));
        assert!(hidden(&privileged, XpList::Blocked));
        assert!(!hidden(&privileged, XpList::Trusted) && !hidden(&grid, XpList::Trusted));
        assert!(hidden(&adult, XpList::Trusted), "over the chosen rating");
        assert!(experience_hidden(&land, XpList::Trusted, 42, &[land.id], None), "already trusted");
        assert!(experience_hidden(&land, XpList::Allowed, 42, &[], Some(land.id)), "the default one");
    }

    #[test]
    fn the_flags_of_each_list() {
        assert_eq!(XpList::Trusted.flag(true), xe::TRUSTED_ADD);
        assert_eq!(XpList::Allowed.flag(false), xe::ALLOWED_REMOVE);
        assert_eq!(XpList::Blocked.flag(true), xe::BLOCKED_ADD);
    }

    /// commitDayLenOffsetChanges: a negative offset is the day after.
    #[test]
    fn the_day_s_seconds() {
        assert_eq!(day_seconds(4.0, -8.0), (4 * 3600, 16 * 3600));
        assert_eq!(day_seconds(24.0, 3.5), (24 * 3600, 3 * 3600 + 1800));
        assert_eq!(day_seconds(4.0, 0.0), (4 * 3600, 24 * 3600));
    }
}
