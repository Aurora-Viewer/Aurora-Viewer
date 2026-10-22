//! "Manage › Interventions" of the Region / Estate window: what acts at
//! once on the people in the region or on the region itself, gathered in
//! one section, each behind a dialog that says what will happen (mockup
//! variant A). The messages are the Legacy Viewer's (newview/
//! llfloaterregioninfo.cpp, llfloatergodtools.cpp,
//! llfloaterregionrestartschedule.cpp, originally LGPL 2.1):
//! LLPanelRegionGeneralInfo's message to the region (`simulatormessage`)
//! and teleports home (`teleporthomeuser`, `teleporthomeallusers`),
//! LLPanelEstateInfo's message to the estate (`instantmessage`) and kick
//! (`kickestate`), LLPanelRegionDebugInfo's return of a resident's objects
//! (`estateobjectreturn` estate-wide, else SimWideDeletes) and restart
//! (`restart` with its delay, -1 to cancel), and the RegionSchedule
//! capability of the restart schedule. Who may: the estate's owner and
//! managers (canManageEstate), as those panels enable their buttons.

use super::{RegionUi, View};
use crate::theme::Palette;
use crate::ui::avatar_picker::AvatarPicker;
use crate::ui::sheet::{self, editable, line, semibold, srow, why};
use crate::ui::widgets::{self, ActionKind, DialogButton};
use crate::world::World;
use crate::world::estate::Role;
use aurora_net::estate::{
    self as net, ESTATE_LAST_LINDEN, EstateCommand, RESTART_DELAY_DEFAULT, RESTART_DELAY_MAX, RESTART_DELAY_MIN, RestartSchedule,
    estate_access as ea,
};
use egui::{RichText, Sense, Vec2};
use uuid::Uuid;

/// The restart delays offered before "Other" (seconds); the first is the
/// Legacy Viewer's default.
const DELAYS: [u32; 3] = [RESTART_DELAY_DEFAULT, 300, 600];

/// Who "Teleport home" sends home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Home {
    One(Uuid),
    Everyone,
}

/// The dialog on screen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Dialog {
    Message {
        estate: bool,
        text: String,
    },
    Home {
        query: String,
        target: Option<Home>,
    },
    Kick {
        target: Uuid,
        ban: bool,
    },
    Return {
        target: Uuid,
        scripted: bool,
        others_land: bool,
        estate_wide: bool,
    },
    /// `choice` indexes DELAYS, `DELAYS.len()` is "Other" with `other` seconds.
    Restart {
        choice: usize,
        other: u32,
    },
    Schedule {
        days: [bool; 7],
        hour: u32,
        minute: u32,
    },
}

impl Dialog {
    pub(crate) fn restart() -> Self {
        Dialog::Restart {
            choice: 0,
            other: RESTART_DELAY_DEFAULT,
        }
    }

    pub(crate) fn message() -> Self {
        Dialog::Message {
            estate: false,
            text: String::new(),
        }
    }

    /// The schedule dialog on the region's schedule (none: no day, 00:00).
    fn schedule(s: Option<&RestartSchedule>) -> Self {
        let (days, seconds) = s.map_or(([false; 7], 0), |s| (s.days, s.seconds));
        Dialog::Schedule {
            days,
            hour: seconds / 3600,
            minute: (seconds % 3600) / 60,
        }
    }
}

/// The restart delay a choice of the dialog stands for, in the spinner's
/// range.
pub(crate) fn delay_of(choice: usize, other: u32) -> u32 {
    DELAYS
        .get(choice)
        .copied()
        .unwrap_or(other)
        .clamp(RESTART_DELAY_MIN, RESTART_DELAY_MAX)
}

/// Who the resident picker chooses for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Home,
    Kick,
    Return,
}

/// What the section keeps between frames.
#[derive(Default)]
pub(super) struct InterventionsState {
    pub dialog: Option<Dialog>,
    /// AURORA_DEMO_REGION's dialog, shown once the simulator answered.
    pub demo: Option<Dialog>,
    picker: AvatarPicker,
    picking: Option<Pick>,
}

impl InterventionsState {
    pub fn reset(&mut self) {
        self.dialog = None;
        self.picker.open = false;
        self.picking = None;
    }
}

/// The residents in the region besides the agent (CoarseLocationUpdate),
/// nearest first, with their distance.
fn present(world: &World, v: &View) -> Vec<(Uuid, f32)> {
    let me = world.agent.position;
    let mut out: Vec<(Uuid, f32)> = world
        .coarse
        .get(&v.handle)
        .map(|list| {
            list.iter()
                .filter(|(id, _)| *id != world.agent_id && !id.is_nil())
                .map(|(id, pos)| (*id, pos.distance(me)))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

/// How many are in the region, the agent included: the simulator's count
/// (SimStats), else the map's dots.
fn people_here(world: &World, v: &View, present: usize) -> usize {
    let stats = world
        .estate
        .stats_of(v.handle)
        .and_then(|s| s.get(net::sim_stat::NUMAGENTMAIN))
        .map(|n| n.round().max(0.0) as usize);
    stats.unwrap_or(0).max(present + 1)
}

/// LLAgentUI::buildFullname: the legacy name that signs a message.
fn own_name(world: &World) -> String {
    world
        .login
        .as_ref()
        .map(|l| crate::world::social::format_name(&l.first_name, &l.last_name))
        .unwrap_or_default()
}

/// "Interventions": a row per action, its button at the right end.
pub(super) fn section(ui: &mut egui::Ui, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let icon = "bolt-filled";
    sheet::section_title(ui, p, icon, aurora_i18n::tr!("region-manage-interventions"), |_| {});
    why(ui, p, aurora_i18n::tr!("region-int-why"));
    let can = v.manages();
    let size = Vec2::new(0.0, 28.0);
    let button = |ui: &mut egui::Ui, icon: &str, label: &str, kind: ActionKind| {
        widgets::action_button_if(ui, p, icon, label, kind, size, can).clicked()
    };
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-int-message"),
        aurora_i18n::tr!("region-int-message-desc"),
        120.0,
        |ui| {
            if button(ui, "speakerphone", aurora_i18n::tr!("region-int-write"), ActionKind::Flat) {
                s.interventions.dialog = Some(Dialog::message());
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-int-home"),
        aurora_i18n::tr!("region-int-home-desc"),
        120.0,
        |ui| {
            if button(ui, "home-filled", aurora_i18n::tr!("region-int-choose"), ActionKind::Flat) {
                s.interventions.dialog = Some(Dialog::Home {
                    query: String::new(),
                    target: None,
                });
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-int-kick"),
        aurora_i18n::tr!("region-int-kick-desc"),
        120.0,
        |ui| {
            if button(ui, "door-exit", aurora_i18n::tr!("region-int-choose"), ActionKind::Flat) {
                open_picker(&mut s.interventions, Pick::Kick);
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-int-return"),
        aurora_i18n::tr!("region-int-return-desc"),
        120.0,
        |ui| {
            if button(ui, "arrow-back-up", aurora_i18n::tr!("region-int-choose"), ActionKind::Flat) {
                open_picker(&mut s.interventions, Pick::Return);
            }
        },
    );
    line(ui, p);
    srow(
        ui,
        p,
        aurora_i18n::tr!("region-int-restart"),
        aurora_i18n::tr!("region-int-restart-desc"),
        300.0,
        |ui| {
            // right to left: the main action, then its undo
            if button(ui, "refresh", aurora_i18n::tr!("region-restart"), ActionKind::SoftDanger) {
                s.interventions.dialog = Some(Dialog::restart());
            }
            if button(ui, "", aurora_i18n::tr!("region-int-cancel-restart"), ActionKind::Flat) {
                world.estate.intervene(EstateCommand::Restart {
                    handle: v.handle,
                    seconds: -1,
                });
            }
        },
    );
    line(ui, p);
    // the schedule's button shows only where the region has the
    // capability (LLFloaterRegionRestartSchedule::canUse)
    let schedule = world.estate.schedule;
    let desc = match &schedule {
        Some(Ok(Some(sch))) => format!("{}.", super::overview::schedule_text(sch)),
        Some(Ok(None)) => aurora_i18n::tr!("region-int-schedule-none").to_owned(),
        Some(Err(())) => aurora_i18n::tr!("region-int-schedule-unavailable").to_owned(),
        None => aurora_i18n::tr!("common-loading").to_owned(),
    };
    srow(ui, p, aurora_i18n::tr!("region-int-schedule"), &desc, 120.0, |ui| {
        let usable = matches!(schedule, Some(Ok(_)));
        if editable(ui, usable, |ui| {
            button(ui, "calendar-event-filled", aurora_i18n::tr!("region-int-edit"), ActionKind::Flat)
        }) && let Some(Ok(sch)) = &schedule
        {
            s.interventions.dialog = Some(Dialog::schedule(sch.as_ref()));
        }
    });
}

fn open_picker(s: &mut InterventionsState, pick: Pick) {
    s.picking = Some(pick);
    // one resident, as LLFloaterAvatarPicker::show(…, false, …)
    s.picker.open("region_intervention_picker", false);
}

/// The section's picker and dialogs, over the window.
pub(super) fn dialogs(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    if let Some(ids) = s.interventions.picker.show(ctx, p, world)
        && let (Some(pick), Some(&id)) = (s.interventions.picking.take(), ids.first())
    {
        s.interventions.dialog = Some(match (pick, s.interventions.dialog.take()) {
            (Pick::Home, Some(Dialog::Home { query, .. })) => Dialog::Home {
                query,
                target: Some(Home::One(id)),
            },
            (Pick::Home, _) => Dialog::Home {
                query: String::new(),
                target: Some(Home::One(id)),
            },
            (Pick::Kick, _) => Dialog::Kick { target: id, ban: false },
            // "Change…" keeps the filters
            (
                Pick::Return,
                Some(Dialog::Return {
                    scripted,
                    others_land,
                    estate_wide,
                    ..
                }),
            ) => Dialog::Return {
                target: id,
                scripted,
                others_land,
                estate_wide,
            },
            (Pick::Return, _) => Dialog::Return {
                target: id,
                scripted: false,
                others_land: false,
                estate_wide: false,
            },
        });
    }
    if !s.interventions.picker.open {
        s.interventions.picking = None;
    }
    // the picker stands over the dialog it fills
    if s.interventions.picking.is_some() {
        return;
    }
    if world.estate.schedule.is_some()
        && let Some(d) = s.interventions.demo.take()
    {
        s.interventions.dialog = Some(d);
    }
    let Some(dialog) = s.interventions.dialog.take() else {
        return;
    };
    let (keep, send) = show_dialog(ctx, p, s, v, world, dialog);
    s.interventions.dialog = keep;
    if let Some(c) = send {
        apply(s, v, world, c);
    }
}

/// What a dialog sends once confirmed.
enum Send {
    Command(EstateCommand),
    /// Kicked, then banned from this estate (accessCoreConfirm's delta).
    KickAndBan(Uuid),
}

fn apply(_s: &mut RegionUi, v: &View, world: &mut World, send: Send) {
    match send {
        Send::Command(c) => world.estate.intervene(c),
        Send::KickAndBan(target) => {
            world.estate.intervene(EstateCommand::KickFromEstate { handle: v.handle, target });
            world.estate.change_access(v.handle, ea::BANNED_AGENT_ADD, vec![target], true);
        }
    }
}

/// Whether "Ban them too" may be offered: never on a Linden estate, nor
/// for a manager or the estate owner (ProblemBanningEstateManager,
/// OwnerCanNotBeDenied), and only to whom may change the bans.
fn can_ban(world: &World, v: &View, target: Uuid) -> bool {
    let linden = world.estate.estate.as_ref().is_some_and(|e| e.id <= ESTATE_LAST_LINDEN);
    let manager = world.estate.access.clone().flatten().is_some_and(|a| a.managers.contains(&target));
    v.shown != Role::Resident && !linden && !manager && target != v.owner
}

/// The dialog's header: icon, title and the closing cross (true when
/// clicked).
pub(super) fn head(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, danger: bool) -> bool {
    let mut closed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let tint = if danger { p.danger } else { p.violet_light };
        widgets::glyph(ui, icon, tint, 16.0);
        ui.label(semibold(ui, title, 13.5, p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (rect, r) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
            sheet::icon_at(ui, "x", rect.center(), 15.0, if r.hovered() { p.ink } else { p.muted });
            closed = r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        });
    });
    ui.add_space(8.0);
    closed
}

/// A line of fine print under a dialog's content.
fn caption(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(6.0);
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.muted)).wrap());
}

/// Cancel and the main action; Some(true) for the main one.
pub(super) fn footer(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, kind: ActionKind, enabled: bool) -> Option<bool> {
    ui.add_space(14.0);
    let buttons = [
        DialogButton {
            icon: "",
            label: aurora_i18n::tr!("common-cancel"),
            kind: ActionKind::Flat,
            enabled: true,
        },
        DialogButton {
            icon,
            label,
            kind,
            enabled,
        },
    ];
    widgets::dialog_buttons(ui, p, &buttons, 28.0).map(|i| i == 1)
}

/// A resident row of the dialogs: their round picture and name, `extra` at
/// the right end; its response.
fn person(ui: &mut egui::Ui, p: &Palette, world: &mut World, id: Uuid, extra: &str, lit: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if lit {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
    } else if r.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.35));
    }
    let (name, _) = sheet::names(world, &id);
    let disc = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), Vec2::splat(22.0));
    widgets::avatar_disc(ui, p, disc, None, &name, false);
    let mut job = semibold(ui, &name, 12.5, p.ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 110.0);
    let galley = ui.painter().layout_job(job);
    let at = widgets::row_text_origin(ui.painter(), &galley, rect.left() + 36.0, rect.center().y);
    ui.painter().galley(at, galley, p.ink);
    if !extra.is_empty() {
        ui.painter().text(
            egui::pos2(rect.right() - 8.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            extra,
            egui::FontId::proportional(11.5),
            p.muted,
        );
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Shows `dialog`: what stays on screen and what is sent.
fn show_dialog(
    ctx: &egui::Context,
    p: &Palette,
    s: &mut RegionUi,
    v: &View,
    world: &mut World,
    mut dialog: Dialog,
) -> (Option<Dialog>, Option<Send>) {
    let present = present(world, v);
    let here = people_here(world, v, present.len());
    let mut answer: Option<bool> = None;
    let mut repick = false;
    let id = egui::Id::unique("region_estate_intervention");
    let modal = widgets::Modal::new(id).show(ctx, |ui| {
        ui.set_width(if matches!(dialog, Dialog::Schedule { .. }) { 480.0 } else { 420.0 });
        match &mut dialog {
            Dialog::Message { estate, text } => {
                if head(ui, p, "speakerphone", aurora_i18n::tr!("region-int-message-title"), false) {
                    answer = Some(false);
                }
                let this = aurora_i18n::tr!("region-int-message-region", count = here as i64);
                let options = [(this.as_str(), true), (aurora_i18n::tr!("region-int-message-estate"), true)];
                let mut choice = usize::from(*estate);
                if widgets::segmented(ui, p, &mut choice, &options) {
                    *estate = choice == 1;
                }
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::multiline(text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text(aurora_i18n::tr!("region-int-message-hint")),
                );
                caption(ui, p, &aurora_i18n::tr!("region-int-message-cap", name = own_name(world)));
                let ok = !text.trim().is_empty();
                if let Some(main) = footer(ui, p, "send-filled", aurora_i18n::tr!("region-int-send"), ActionKind::Primary, ok) {
                    answer = Some(main);
                }
            }
            Dialog::Home { query, target } => {
                if head(ui, p, "home-filled", aurora_i18n::tr!("region-int-home-title"), false) {
                    answer = Some(false);
                }
                // in a row: the field takes its own height
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    widgets::search_box(ui, p, query, aurora_i18n::tr!("region-int-home-search"), w);
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    let mut shown = 0;
                    for (id, distance) in &present {
                        let (name, user) = sheet::names(world, id);
                        if !sheet::matches(query, &name, user.as_deref()) {
                            continue;
                        }
                        shown += 1;
                        let extra = format!("{} m", aurora_i18n::integer(distance.round() as i64));
                        if person(ui, p, world, *id, &extra, *target == Some(Home::One(*id))).clicked() {
                            *target = Some(Home::One(*id));
                        }
                    }
                    if present.is_empty() {
                        // the simulator counts people the map has not placed yet
                        let text = if here > 1 {
                            aurora_i18n::tr!("region-int-home-unlisted")
                        } else {
                            aurora_i18n::tr!("region-int-home-none")
                        };
                        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
                    } else if shown == 0 {
                        ui.label(RichText::new(aurora_i18n::tr!("common-no-results")).size(12.0).color(p.muted));
                    }
                    // someone listed by name only, as the Legacy Viewer's picker
                    if let Some(Home::One(id)) = *target
                        && !present.iter().any(|(who, _)| *who == id)
                    {
                        person(ui, p, world, id, "", true);
                    }
                });
                ui.add_space(4.0);
                let others = here.saturating_sub(1);
                let everyone = aurora_i18n::tr!("region-int-home-all", count = others as i64);
                ui.horizontal(|ui| {
                    if widgets::radio(ui, p, *target == Some(Home::Everyone), everyone.as_str()).clicked() {
                        *target = Some(Home::Everyone);
                    }
                    ui.label(
                        RichText::new(aurora_i18n::tr!("region-int-home-all-desc"))
                            .size(11.5)
                            .color(p.muted),
                    );
                });
                ui.add_space(2.0);
                if ui
                    .add(
                        egui::Label::new(semibold(ui, aurora_i18n::tr!("region-int-other-resident"), 12.0, p.violet_light))
                            .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    repick = true;
                }
                let label = match *target {
                    Some(Home::One(id)) => aurora_i18n::tr!("region-int-home-one", name = sheet::names(world, &id).0),
                    Some(Home::Everyone) => aurora_i18n::tr!("region-int-home-everyone", count = others as i64),
                    None => aurora_i18n::tr!("region-int-home").to_owned(),
                };
                let ok = match *target {
                    Some(Home::Everyone) => others > 0,
                    Some(Home::One(_)) => true,
                    None => false,
                };
                if let Some(main) = footer(ui, p, "home-filled", &label, ActionKind::Danger, ok) {
                    answer = Some(main);
                }
            }
            Dialog::Kick { target, ban } => {
                if head(ui, p, "door-exit", aurora_i18n::tr!("region-int-kick-title"), true) {
                    answer = Some(false);
                }
                let name = sheet::names(world, target).0;
                ui.add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("region-int-kick-text", name = name))
                            .size(12.5)
                            .color(p.ink),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                let bannable = can_ban(world, v, *target);
                if !bannable {
                    *ban = false;
                }
                editable(ui, bannable, |ui| {
                    widgets::checkbox(ui, p, ban, aurora_i18n::tr!("region-int-kick-ban"))
                })
                .on_disabled_hover_text(aurora_i18n::tr!("region-int-kick-ban-no"));
                caption(ui, p, aurora_i18n::tr!("region-int-kick-ban-desc"));
                if let Some(main) = footer(ui, p, "", aurora_i18n::tr!("region-int-kick-ok"), ActionKind::Danger, true) {
                    answer = Some(main);
                }
            }
            Dialog::Return {
                target,
                scripted,
                others_land,
                estate_wide,
            } => {
                if head(ui, p, "arrow-back-up", aurora_i18n::tr!("region-int-return-title"), true) {
                    answer = Some(false);
                }
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 90.0;
                    ui.allocate_ui(Vec2::new(w, 30.0), |ui| person(ui, p, world, *target, "", true));
                    let change = aurora_i18n::tr!("region-int-change");
                    if widgets::action_button(ui, p, "", change, ActionKind::Flat, Vec2::new(0.0, 28.0)).clicked() {
                        repick = true;
                    }
                });
                ui.add_space(6.0);
                widgets::checkbox(ui, p, scripted, aurora_i18n::tr!("region-int-return-scripted"));
                widgets::checkbox(ui, p, others_land, aurora_i18n::tr!("region-int-return-others"));
                ui.label(
                    RichText::new(aurora_i18n::tr!("region-int-return-others-desc"))
                        .size(11.5)
                        .color(p.muted),
                );
                widgets::checkbox(ui, p, estate_wide, aurora_i18n::tr!("region-int-return-estate"));
                caption(ui, p, aurora_i18n::tr!("region-int-return-cap"));
                if let Some(main) = footer(ui, p, "", aurora_i18n::tr!("region-int-return-ok"), ActionKind::Danger, true) {
                    answer = Some(main);
                }
            }
            Dialog::Restart { choice, other } => {
                let title = aurora_i18n::tr!("region-int-restart-title", region = v.name.as_str());
                if head(ui, p, "refresh", &title, true) {
                    answer = Some(false);
                }
                let text = aurora_i18n::tr!("region-int-restart-text", count = here as i64);
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
                ui.add_space(10.0);
                ui.label(semibold(ui, aurora_i18n::tr!("region-int-delay"), 11.5, p.muted));
                ui.add_space(2.0);
                let labels: Vec<String> = DELAYS.iter().map(|d| delay_text(*d)).collect();
                let mut options: Vec<(&str, bool)> = labels.iter().map(|l| (l.as_str(), true)).collect();
                options.push((aurora_i18n::tr!("region-int-delay-other"), true));
                widgets::segmented(ui, p, choice, &options);
                if *choice == DELAYS.len() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let mut s = f64::from(*other);
                        ui.add_sized(
                            [90.0, 24.0],
                            egui::DragValue::new(&mut s)
                                .range(f64::from(RESTART_DELAY_MIN)..=f64::from(RESTART_DELAY_MAX))
                                .speed(1.0)
                                .custom_parser(aurora_i18n::parse_number)
                                .custom_formatter(|v, _| format!("{} s", aurora_i18n::integer(v.round() as i64))),
                        );
                        *other = s.round() as u32;
                    });
                }
                caption(ui, p, aurora_i18n::tr!("region-int-delay-cap"));
                let delay = delay_of(*choice, *other);
                let ok = aurora_i18n::tr!("region-int-restart-ok", time = delay_text(delay));
                if let Some(main) = footer(ui, p, "refresh", &ok, ActionKind::Danger, true) {
                    answer = Some(main);
                }
            }
            Dialog::Schedule { days, hour, minute } => {
                if head(ui, p, "calendar-event-filled", aurora_i18n::tr!("region-int-schedule-title"), false) {
                    answer = Some(false);
                }
                ui.label(semibold(ui, aurora_i18n::tr!("region-int-schedule-days"), 11.5, p.muted));
                ui.horizontal_wrapped(|ui| {
                    for (i, on) in days.iter_mut().enumerate() {
                        widgets::checkbox(ui, p, on, aurora_i18n::tr!("region-weekday", day = i as i64));
                    }
                });
                ui.add_space(8.0);
                ui.label(semibold(ui, aurora_i18n::tr!("region-int-schedule-time"), 11.5, p.muted));
                ui.horizontal(|ui| {
                    let field = |ui: &mut egui::Ui, value: &mut u32, max: u32| {
                        let mut v = f64::from(*value);
                        ui.add_sized(
                            [52.0, 24.0],
                            egui::DragValue::new(&mut v)
                                .range(0.0..=f64::from(max))
                                .speed(0.2)
                                .custom_formatter(|v, _| format!("{:02}", v.round() as i64)),
                        );
                        *value = v.round() as u32;
                    };
                    field(ui, hour, 23);
                    ui.label(RichText::new(":").size(13.0).color(p.ink));
                    field(ui, minute, 59);
                    ui.label(RichText::new("UTC").size(12.0).color(p.muted));
                });
                caption(ui, p, aurora_i18n::tr!("region-int-schedule-cap"));
                if let Some(main) = footer(ui, p, "", aurora_i18n::tr!("region-int-save"), ActionKind::Primary, true) {
                    answer = Some(main);
                }
            }
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    if repick {
        let pick = if matches!(dialog, Dialog::Home { .. }) {
            Pick::Home
        } else {
            Pick::Return
        };
        open_picker(&mut s.interventions, pick);
        return (Some(dialog), None);
    }
    match answer {
        None => (Some(dialog), None),
        Some(false) => (None, None),
        Some(true) => (None, send_of(dialog, v, world, here.saturating_sub(1))),
    }
}

/// The message a confirmed dialog sends; `others` are in the region
/// besides the agent.
fn send_of(dialog: Dialog, v: &View, world: &World, others: usize) -> Option<Send> {
    let handle = v.handle;
    Some(match dialog {
        Dialog::Message { estate, text } => {
            let text = text.trim().to_owned();
            if text.is_empty() {
                return None;
            }
            Send::Command(EstateCommand::Message {
                handle,
                estate,
                name: own_name(world),
                text,
            })
        }
        Dialog::Home { target, .. } => match target? {
            Home::One(id) => Send::Command(EstateCommand::TeleportHome { handle, target: Some(id) }),
            Home::Everyone if others > 0 => Send::Command(EstateCommand::TeleportHome { handle, target: None }),
            Home::Everyone => return None,
        },
        Dialog::Kick { target, ban: true } => Send::KickAndBan(target),
        Dialog::Kick { target, ban: false } => Send::Command(EstateCommand::KickFromEstate { handle, target }),
        Dialog::Return {
            target,
            scripted,
            others_land,
            estate_wide,
        } => Send::Command(EstateCommand::ReturnObjects {
            handle,
            target,
            flags: net::return_flags(scripted, others_land),
            estate_wide,
        }),
        Dialog::Restart { choice, other } => Send::Command(EstateCommand::Restart {
            handle,
            seconds: delay_of(choice, other) as i32,
        }),
        Dialog::Schedule { days, hour, minute } => Send::Command(EstateCommand::SetRestartSchedule {
            handle,
            days,
            seconds: hour.min(23) * 3600 + minute.min(59) * 60,
        }),
    })
}

/// "2 min", "90 s", "1 h".
pub(crate) fn delay_text(seconds: u32) -> String {
    if seconds >= 3600 && seconds.is_multiple_of(3600) {
        aurora_i18n::tr!("region-int-hours", n = i64::from(seconds / 3600))
    } else if seconds >= 60 && seconds.is_multiple_of(60) {
        aurora_i18n::tr!("region-int-minutes", n = i64::from(seconds / 60))
    } else {
        aurora_i18n::tr!("region-int-seconds", n = i64::from(seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restart_delay_stays_in_the_spinner_s_range() {
        assert_eq!(delay_of(0, 999), 120, "the Legacy Viewer's default first");
        assert_eq!(delay_of(1, 999), 300);
        assert_eq!(delay_of(DELAYS.len(), 5), RESTART_DELAY_MIN);
        assert_eq!(delay_of(DELAYS.len(), 90), 90);
        assert_eq!(delay_of(DELAYS.len(), 99_999), RESTART_DELAY_MAX);
    }

    #[test]
    fn a_delay_is_said_in_its_unit() {
        assert_eq!(delay_text(120), aurora_i18n::tr!("region-int-minutes", n = 2));
        assert_eq!(delay_text(90), aurora_i18n::tr!("region-int-seconds", n = 90));
        assert_eq!(delay_text(3600), aurora_i18n::tr!("region-int-hours", n = 1));
    }

    #[test]
    fn the_schedule_dialog_starts_on_the_region_s() {
        let s = RestartSchedule {
            days: [true, false, false, false, false, false, false],
            seconds: 6 * 3600 + 30 * 60,
        };
        assert_eq!(
            Dialog::schedule(Some(&s)),
            Dialog::Schedule {
                days: s.days,
                hour: 6,
                minute: 30
            }
        );
        assert_eq!(
            Dialog::schedule(None),
            Dialog::Schedule {
                days: [false; 7],
                hour: 0,
                minute: 0
            }
        );
    }
}
