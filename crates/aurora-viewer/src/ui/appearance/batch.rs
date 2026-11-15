//! The outfits' batch portraits window (Fider #447; the app's side
//! `app::outfit_batch`). Before it starts: the outfits shown in the
//! showcase, those without a picture ticked, the others to tick to redo,
//! those RLVa locks greyed, the auto portrait's choices, and what is going
//! to happen (others see the outfits change; the starting outfit comes
//! back). Running: "Photo i of n…", the progress, each outfit's state, and
//! Stop (the starting outfit back) and Pause. The paid HD pictures
//! (`hd_total_ui`): the total for the outfits ticked, against the balance,
//! confirmed once at Start; their count and cost at the end.

use super::super::{icons, widgets};
use super::portrait;
use crate::snapshot::portrait::{HdOffer, PortraitSettings};
use crate::theme::Palette;
use crate::ui::fonts;
use egui::{Rect, RichText, Sense, Vec2};

/// The batch portraits' window.
pub const WINDOW: &str = "outfit_batch";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The outfit of this row ticked or unticked, before the start.
    Toggle(usize),
    Start,
    Pause,
    Resume,
    /// Stop: the starting outfit back on.
    Stop,
    Close,
    /// The HD pictures' total confirmed: the batch starts.
    ConfirmHd,
    /// Back to the choices, nothing paid.
    CancelHd,
}

/// The batch's paid HD pictures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HdTotal<'a> {
    /// The total for the outfits ticked, and whether the balance pays it
    /// (None before it is known: not shown).
    pub offer: Option<HdOffer>,
    /// The price of one picture.
    pub each: Option<i32>,
    pub count: usize,
    pub balance: Option<i32>,
    /// Start asked to confirm this total.
    pub confirm: Option<i32>,
    /// The batch sends them (confirmed at its start).
    pub used: bool,
    pub saved: usize,
    pub paid: i32,
    pub pending: usize,
    pub failed: usize,
    pub error: Option<&'a str>,
}

/// Why Start is not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    /// The current outfit has unsaved changes, which putting it back
    /// would lose.
    Unsaved,
    /// No saved outfit is worn to put back.
    NoStart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Setup(Option<Blocked>),
    /// The `index`th photo (from 1) of `count`; `done` taken or skipped.
    Running {
        index: usize,
        count: usize,
        done: usize,
        paused: bool,
    },
    Restoring,
    Finished {
        done: usize,
        skipped: usize,
        restored: bool,
    },
}

/// Why an outfit was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Locked,
    /// RLVa refused the outfit change itself.
    Refused,
    /// Still loading after this many seconds.
    Slow(u64),
    Gone,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState<'a> {
    /// Before the start: ticked or not.
    Choose {
        chosen: bool,
        had_picture: bool,
    },
    /// Before the start: not offered.
    Locked,
    Empty,
    Waiting,
    Wearing,
    /// Its items worn so far.
    Loading {
        worn: usize,
        total: usize,
    },
    /// Every item worn: the textures and the layers are still coming.
    LoadingLook,
    Framing,
    Shooting,
    Saving,
    Done {
        missing: usize,
    },
    /// Left when the batch stopped.
    NotDone,
    Skipped(Skip),
    Failed(&'a str),
}

pub struct Row<'a> {
    pub name: &'a str,
    /// The live preview (the current row), the photo taken, or the
    /// outfit's picture.
    pub picture: Option<&'a egui::TextureHandle>,
    pub redo: bool,
    pub current: bool,
    pub state: RowState<'a>,
}

pub struct View<'a> {
    pub phase: Phase,
    pub rows: Vec<Row<'a>>,
    pub hd: HdTotal<'a>,
}

const ROW: f32 = 48.0;
const PICTURE: f32 = 36.0;

/// The window; `open` is cleared by its X (Stop while running).
pub fn window(ctx: &egui::Context, p: &Palette, settings: &mut PortraitSettings, view: &View, open: &mut bool) -> Vec<Action> {
    let mut actions = Vec::new();
    let title = aurora_i18n::tr!("ui-appearance-batch-title");
    widgets::Floater::new(WINDOW, title, egui::pos2(140.0, 90.0), egui::vec2(520.0, 560.0))
        .steady_size()
        .show(ctx, p, open, |ui| {
            let (mut above, footer) = widgets::above_footer(ui);
            body(&mut above, p, settings, view, &mut actions);
            footer.show(ui, |ui| footer_ui(ui, p, view, &mut actions));
        });
    actions
}

/// The headline, the progress, the choices before the start, the rows.
fn body(ui: &mut egui::Ui, p: &Palette, settings: &mut PortraitSettings, view: &View, actions: &mut Vec<Action>) {
    ui.spacing_mut().item_spacing.y = 10.0;
    let headline = match view.phase {
        Phase::Setup(_) => aurora_i18n::tr!("ui-appearance-batch-choose").to_string(),
        Phase::Running { index, count, paused, .. } => {
            let key = if paused {
                aurora_i18n::tr!("ui-appearance-batch-paused", index = index, count = count)
            } else {
                aurora_i18n::tr!("ui-appearance-batch-progress", index = index, count = count)
            };
            key.to_string()
        }
        Phase::Restoring => aurora_i18n::tr!("ui-appearance-batch-restoring").to_string(),
        Phase::Finished { .. } => aurora_i18n::tr!("ui-appearance-batch-finished").to_string(),
    };
    ui.add(egui::Label::new(RichText::new(headline).size(18.0).family(fonts::semibold(ui.ctx())).color(p.ink)).truncate());
    match view.phase {
        Phase::Running { count, done, .. } => {
            let fraction = if count == 0 { 0.0 } else { done as f32 / count as f32 };
            widgets::progress(ui, p, "outfit_batch_progress", fraction, ui.available_width(), "", None);
        }
        Phase::Finished { done, skipped, restored } => {
            let summary = aurora_i18n::tr!("ui-appearance-batch-summary", done = done, skipped = skipped);
            portrait::note(ui, p, "circle-check-filled", &summary);
            if !restored {
                portrait::note(ui, p, "alert-triangle-filled", aurora_i18n::tr!("ui-appearance-batch-not-restored"));
            }
        }
        _ => {}
    }
    if let Phase::Setup(_) = view.phase {
        widgets::collapsing(
            ui,
            p,
            "outfit_batch_settings",
            aurora_i18n::tr!("ui-appearance-batch-settings"),
            false,
            |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                portrait::choices_ui(ui, p, settings);
            },
        );
        hd_total_ui(ui, p, settings, &view.hd);
        if view.rows.is_empty() {
            portrait::note(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-appearance-batch-none"));
        }
    }
    if matches!(view.phase, Phase::Running { .. } | Phase::Restoring | Phase::Finished { .. }) && view.hd.used {
        hd_result_ui(ui, p, &view.hd);
    }
    egui::ScrollArea::vertical()
        .id_salt("outfit_batch_rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (i, row) in view.rows.iter().enumerate() {
                row_ui(ui, p, i, row, actions);
            }
        });
}

/// Before the start: the HD pictures' box with the total for the outfits
/// ticked (count × price), greyed with the reason when the balance cannot
/// pay the total or the price is not known.
fn hd_total_ui(ui: &mut egui::Ui, p: &Palette, s: &mut PortraitSettings, hd: &HdTotal) {
    let Some(offer) = hd.offer else {
        return;
    };
    let label = match (offer, hd.each) {
        (HdOffer::Ready(total) | HdOffer::Short(total), Some(each)) => {
            aurora_i18n::tr!("ui-appearance-batch-hd", total = total, count = hd.count, each = each).to_string()
        }
        _ => aurora_i18n::tr!("ui-appearance-portrait-hd-no-price").to_string(),
    };
    let ready = matches!(offer, HdOffer::Ready(_)) && hd.confirm.is_none();
    let on = portrait::hd_box(
        ui,
        p,
        s,
        aurora_i18n::tr!("ui-appearance-batch-hd-title"),
        label,
        ready,
        matches!(offer, HdOffer::Ready(_)),
    );
    let balance = hd
        .balance
        .map(|b| aurora_i18n::tr!("ui-appearance-batch-hd-balance", balance = b).to_string());
    match offer {
        HdOffer::Short(total) => {
            let text = aurora_i18n::tr!("ui-appearance-batch-hd-short", total = total, balance = hd.balance.unwrap_or(0));
            portrait::note(ui, p, "alert-triangle-filled", &text);
        }
        HdOffer::Unknown => portrait::note(
            ui,
            p,
            "alert-triangle-filled",
            aurora_i18n::tr!("ui-appearance-portrait-hd-unknown"),
        ),
        HdOffer::Ready(_) if on => {
            let mut text = aurora_i18n::tr!("ui-appearance-batch-hd-about").to_string();
            if let Some(balance) = &balance {
                text = format!("{text} {balance}");
            }
            portrait::note(ui, p, "info-circle-filled", &text);
        }
        HdOffer::Ready(_) => portrait::note(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-appearance-portrait-hd-about")),
    }
}

/// Once started: the HD pictures saved and their cost, those still on
/// their way, those not sent.
fn hd_result_ui(ui: &mut egui::Ui, p: &Palette, hd: &HdTotal) {
    let saved = aurora_i18n::tr!("ui-appearance-batch-hd-saved", saved = hd.saved, paid = hd.paid);
    portrait::note(ui, p, "photo-filled", &saved);
    if hd.pending > 0 {
        let text = aurora_i18n::tr!("ui-appearance-batch-hd-pending", pending = hd.pending);
        portrait::note(ui, p, "info-circle-filled", &text);
    }
    if hd.failed > 0 {
        let text = aurora_i18n::tr!("ui-appearance-batch-hd-failed", failed = hd.failed, error = hd.error.unwrap_or(""));
        portrait::note(ui, p, "alert-triangle-filled", &text);
    }
}

/// One outfit: its tick before the start, its picture, its name and its
/// state, a mark on the right.
fn row_ui(ui: &mut egui::Ui, p: &Palette, index: usize, row: &Row, actions: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    if row.current {
        ui.painter().rect_filled(rect, radius, p.field);
    }
    let mut left = rect.left() + 8.0;
    if let RowState::Choose { chosen, .. } = row.state {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(left + 24.0, rect.bottom()),
                ))
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .id_salt(("outfit_batch_tick", index)),
        );
        let mut on = chosen;
        if widgets::checkbox(&mut child, p, &mut on, "").changed() {
            actions.push(Action::Toggle(index));
        }
        left += 28.0;
    }
    let square = Rect::from_min_size(egui::pos2(left, rect.center().y - PICTURE / 2.0), Vec2::splat(PICTURE));
    ui.painter().rect_filled(square, radius, p.bar);
    if let Some(t) = row.picture {
        ui.painter().image(
            t.id(),
            square,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            widgets::PICTURE_TINT,
        );
    }
    let (state, ink, mark) = state_text(p, row.state);
    let mark_room = 30.0;
    let text_left = square.right() + 10.0;
    let width = (rect.right() - mark_room - text_left).max(10.0);
    let name = if row.redo {
        aurora_i18n::tr!("ui-appearance-batch-redo", name = row.name).to_string()
    } else {
        row.name.to_owned()
    };
    let painter = ui.painter_at(rect);
    let name_galley = painter.layout(name, egui::FontId::new(13.0, fonts::semibold(ui.ctx())), p.ink, f32::INFINITY);
    let state_galley = painter.layout(state, egui::FontId::proportional(11.0), ink, f32::INFINITY);
    let gap = 2.0;
    let top = rect.center().y - (name_galley.size().y + gap + state_galley.size().y) / 2.0;
    let clip = Rect::from_min_max(egui::pos2(text_left, rect.top()), egui::pos2(text_left + width, rect.bottom()));
    let text = ui.painter_at(clip);
    let state_top = top + name_galley.size().y + gap;
    text.galley(egui::pos2(text_left, top), name_galley, p.ink);
    text.galley(egui::pos2(text_left, state_top), state_galley, ink);
    let at = Rect::from_center_size(egui::pos2(rect.right() - mark_room / 2.0, rect.center().y), Vec2::splat(16.0));
    match mark {
        Mark::None => {}
        Mark::Busy => egui::Spinner::new().color(p.violet_light).paint_at(ui, at),
        Mark::Icon(name) => {
            if let Some(t) = icons::global(name) {
                painter.image(t.id(), at, Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), ink);
            }
        }
    }
}

enum Mark {
    None,
    Busy,
    Icon(&'static str),
}

/// A row's state line, its colour and its mark.
fn state_text(p: &Palette, state: RowState) -> (String, egui::Color32, Mark) {
    let busy = |text: &str| (text.to_owned(), p.violet_light, Mark::Busy);
    match state {
        RowState::Choose { had_picture: false, .. } => (aurora_i18n::tr!("ui-appearance-batch-no-picture").to_owned(), p.muted, Mark::None),
        RowState::Choose { had_picture: true, .. } => (aurora_i18n::tr!("ui-appearance-batch-has-picture").to_owned(), p.muted, Mark::None),
        RowState::Locked => (
            aurora_i18n::tr!("ui-appearance-batch-locked").to_owned(),
            p.warn,
            Mark::Icon("lock-filled"),
        ),
        RowState::Empty => (aurora_i18n::tr!("ui-appearance-batch-empty").to_owned(), p.muted, Mark::None),
        RowState::Waiting => (aurora_i18n::tr!("ui-appearance-batch-waiting").to_owned(), p.muted, Mark::None),
        RowState::Wearing => busy(aurora_i18n::tr!("ui-appearance-batch-wearing")),
        RowState::Loading { worn, total } => busy(&aurora_i18n::tr!("ui-appearance-batch-loading", worn = worn, total = total)),
        RowState::LoadingLook => busy(aurora_i18n::tr!("ui-appearance-batch-loading-look")),
        RowState::Framing => busy(aurora_i18n::tr!("ui-appearance-batch-framing")),
        RowState::Shooting => busy(aurora_i18n::tr!("ui-appearance-batch-shooting")),
        RowState::Saving => busy(aurora_i18n::tr!("ui-appearance-batch-saving")),
        RowState::Done { missing: 0 } => (
            aurora_i18n::tr!("ui-appearance-batch-done").to_owned(),
            p.success,
            Mark::Icon("circle-check-filled"),
        ),
        RowState::Done { missing } => (
            aurora_i18n::tr!("ui-appearance-batch-done-missing", missing = missing).to_string(),
            p.success,
            Mark::Icon("circle-check-filled"),
        ),
        RowState::NotDone => (aurora_i18n::tr!("ui-appearance-batch-not-done").to_owned(), p.muted, Mark::None),
        RowState::Skipped(skip) => {
            let text = match skip {
                Skip::Locked => aurora_i18n::tr!("ui-appearance-batch-skipped-locked").to_owned(),
                Skip::Refused => aurora_i18n::tr!("ui-appearance-batch-skipped-refused").to_owned(),
                Skip::Slow(seconds) => aurora_i18n::tr!("ui-appearance-batch-skipped-slow", seconds = seconds).to_string(),
                Skip::Gone => aurora_i18n::tr!("ui-appearance-batch-skipped-gone").to_owned(),
                Skip::Empty => aurora_i18n::tr!("ui-appearance-batch-skipped-empty").to_owned(),
            };
            (text, p.warn, Mark::Icon("alert-triangle-filled"))
        }
        RowState::Failed(reason) => (
            aurora_i18n::tr!("ui-appearance-batch-failed", reason = reason).to_string(),
            p.warn,
            Mark::Icon("alert-triangle-filled"),
        ),
    }
}

/// What is going to happen, then the buttons of the phase.
fn footer_ui(ui: &mut egui::Ui, p: &Palette, view: &View, actions: &mut Vec<Action>) {
    ui.spacing_mut().item_spacing.y = 10.0;
    if !matches!(view.phase, Phase::Finished { .. }) {
        widgets::surface(ui, p, |ui| {
            ui.set_width(ui.available_width());
            let mut job = egui::text::LayoutJob::default();
            let font = egui::FontId::proportional(12.0);
            let muted = egui::TextFormat::simple(font.clone(), p.muted);
            job.append(&format!("{} ", aurora_i18n::tr!("ui-appearance-batch-how")), 0.0, muted.clone());
            job.append(
                aurora_i18n::tr!("ui-appearance-batch-seen"),
                0.0,
                fonts::semibold_format(&fonts::semibold(ui.ctx()), 12.0, p.ink),
            );
            job.append(&format!(" {}", aurora_i18n::tr!("ui-appearance-batch-back")), 0.0, muted);
            job.wrap.max_width = ui.available_width();
            ui.add(egui::Label::new(job).wrap());
        });
    }
    // the HD pictures' total to confirm before Start pays it
    if let (Phase::Setup(_), Some(total)) = (view.phase, view.hd.confirm) {
        let text = aurora_i18n::tr!(
            "ui-appearance-batch-hd-confirm",
            count = view.hd.count,
            total = total,
            each = view.hd.each.unwrap_or(0)
        );
        let balance = view
            .hd
            .balance
            .map(|b| aurora_i18n::tr!("ui-appearance-batch-hd-balance", balance = b).to_string());
        let pay = aurora_i18n::tr!("ui-appearance-batch-hd-pay", total = total).to_string();
        match portrait::pay_or_back(ui, p, &text, balance.as_deref(), &pay, false) {
            Some(true) => actions.push(Action::ConfirmHd),
            Some(false) => actions.push(Action::CancelHd),
            None => {}
        }
        return;
    }
    if let Phase::Setup(Some(blocked)) = view.phase {
        let text = match blocked {
            Blocked::Unsaved => aurora_i18n::tr!("ui-appearance-batch-unsaved"),
            Blocked::NoStart => aurora_i18n::tr!("ui-appearance-batch-no-start"),
        };
        portrait::note(ui, p, "alert-triangle-filled", text);
    }
    let height = 36.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        match view.phase {
            Phase::Setup(blocked) => {
                let cancel = aurora_i18n::tr!("common-cancel");
                let width = widgets::action_button_fit(ui, "x", cancel);
                if widgets::action_button(ui, p, "x", cancel, widgets::ActionKind::Flat, Vec2::new(width, height)).clicked() {
                    actions.push(Action::Close);
                }
                let count = view
                    .rows
                    .iter()
                    .filter(|r| matches!(r.state, RowState::Choose { chosen: true, .. }))
                    .count();
                let start = aurora_i18n::tr!("ui-appearance-batch-start", count = count);
                let ready = blocked.is_none() && count > 0;
                let size = Vec2::new(ui.available_width(), height);
                if widgets::action_button_if(ui, p, "camera-filled", &start, widgets::ActionKind::Primary, size, ready).clicked() && ready {
                    actions.push(Action::Start);
                }
            }
            Phase::Running { paused, .. } => {
                let stop = aurora_i18n::tr!("ui-appearance-batch-stop");
                let width = widgets::action_button_fit(ui, "arrow-back-up", stop);
                if widgets::action_button(ui, p, "arrow-back-up", stop, widgets::ActionKind::Flat, Vec2::new(width, height)).clicked() {
                    actions.push(Action::Stop);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (icon, label, action) = if paused {
                        ("player-play-filled", aurora_i18n::tr!("ui-appearance-batch-resume"), Action::Resume)
                    } else {
                        ("player-pause-filled", aurora_i18n::tr!("ui-appearance-batch-pause"), Action::Pause)
                    };
                    let width = widgets::action_button_fit(ui, icon, label);
                    if widgets::action_button(ui, p, icon, label, widgets::ActionKind::Flat, Vec2::new(width, height)).clicked() {
                        actions.push(action);
                    }
                });
            }
            Phase::Restoring => {
                ui.add_sized(Vec2::new(ui.available_width(), height), |ui: &mut egui::Ui| {
                    widgets::busy_button(ui, p, aurora_i18n::tr!("ui-appearance-batch-restoring"))
                });
            }
            Phase::Finished { .. } => {
                let close = aurora_i18n::tr!("common-close");
                let size = Vec2::new(ui.available_width(), height);
                if widgets::action_button(ui, p, "check", close, widgets::ActionKind::Primary, size).clicked() {
                    actions.push(Action::Close);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(state: RowState<'static>) -> Vec<Row<'static>> {
        [
            "Cloud pyjamas",
            "Ball gown",
            "Sportswear",
            "Velvet evening",
            "Beach day",
            "Winter coat",
            "Club night",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, name)| Row {
            name,
            picture: None,
            redo: i % 2 == 1,
            current: i == 1,
            state,
        })
        .collect()
    }

    #[test]
    fn the_window_keeps_its_size() {
        for (phase, state) in [
            (
                Phase::Setup(Some(Blocked::Unsaved)),
                RowState::Choose {
                    chosen: true,
                    had_picture: false,
                },
            ),
            (
                Phase::Running {
                    index: 2,
                    count: 7,
                    done: 1,
                    paused: false,
                },
                RowState::Loading { worn: 14, total: 21 },
            ),
            (
                Phase::Finished {
                    done: 5,
                    skipped: 2,
                    restored: false,
                },
                RowState::Done { missing: 0 },
            ),
        ] {
            crate::ui::widgets::steady_tests::assert_window_steady(
                WINDOW,
                &[egui::vec2(520.0, 560.0), egui::vec2(420.0, 420.0)],
                match phase {
                    Phase::Setup(_) => aurora_i18n::tr!("common-cancel"),
                    Phase::Finished { .. } => aurora_i18n::tr!("common-close"),
                    _ => aurora_i18n::tr!("ui-appearance-batch-stop"),
                },
                PortraitSettings::default,
                |settings, ctx, p| {
                    let view = View {
                        phase,
                        rows: rows(state),
                        hd: hd(),
                    };
                    window(ctx, p, settings, &view, &mut true);
                },
            );
        }
    }

    fn hd() -> HdTotal<'static> {
        HdTotal {
            offer: Some(HdOffer::Short(70)),
            each: Some(10),
            count: 7,
            balance: Some(65),
            used: true,
            saved: 3,
            paid: 30,
            pending: 1,
            failed: 1,
            error: Some("The server refused the HD picture."),
            ..Default::default()
        }
    }

    #[test]
    fn the_window_keeps_its_size_with_the_hd_total_to_confirm() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[egui::vec2(520.0, 560.0), egui::vec2(420.0, 420.0)],
            aurora_i18n::tr!("ui-appearance-portrait-hd-back"),
            || PortraitSettings {
                hd: true,
                ..Default::default()
            },
            |settings, ctx, p| {
                let view = View {
                    phase: Phase::Setup(None),
                    rows: rows(RowState::Choose {
                        chosen: true,
                        had_picture: false,
                    }),
                    hd: HdTotal {
                        offer: Some(HdOffer::Ready(70)),
                        confirm: Some(70),
                        balance: Some(500),
                        ..hd()
                    },
                };
                window(ctx, p, settings, &view, &mut true);
            },
        );
    }
}
