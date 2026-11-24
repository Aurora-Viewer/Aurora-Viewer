//! The complexity strip of the wide Inspect: where the inspected avatar stands
//! among the people here, on a scale that follows the crowd. The layout is
//! pure; `paint` draws it.

use super::{Frame, Person};
use crate::theme::Palette;
use crate::ui::{board, widgets};
use egui::{Rect, Sense, Stroke, Vec2};
use uuid::Uuid;

/// The face of someone, and of the inspected avatar.
const FACE: f32 = 20.0;
const MAIN: f32 = 30.0;

/// The scale: its end, the round steps written under it, and whether the
/// limit is on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Scale {
    pub end: f32,
    pub step: f32,
    /// The user's limit, when someone is near enough for it to say something.
    pub limit: Option<f32>,
}

/// A round step for a scale that ends at `end`: 1, 2 or 5 times a power of 10.
fn step(end: f32) -> f32 {
    let raw = (end / 4.0).max(1.0);
    let power = 10f32.powf(raw.log10().floor());
    let n = raw / power;
    power
        * if n <= 1.0 {
            1.0
        } else if n <= 2.0 {
            2.0
        } else if n <= 5.0 {
            5.0
        } else {
            10.0
        }
}

/// The scale of `values`; `limit` 0: none.
pub fn scale(values: impl Iterator<Item = u32>, limit: u32) -> Scale {
    let top = values.max().unwrap_or(0) as f32;
    let limit = (limit > 0).then_some(limit as f32).filter(|l| top >= l * 0.7);
    let end = (top * 1.08).max(limit.map_or(0.0, |l| l * 1.12)).max(1_000.0);
    Scale {
        end,
        step: step(end),
        limit,
    }
}

/// What stands at a place of the scale: one person, or several who would
/// cover each other.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub x: f32,
    pub people: Vec<usize>,
}

/// The marks of `people` over `width`: those in `kept` (the user, friends) on
/// their own, the others gathered where their faces would touch.
pub fn marks(people: &[Person], scale: &Scale, width: f32, face: f32) -> Vec<Mark> {
    let x = |i: usize| people[i].complexity as f32 / scale.end * width;
    let mut order: Vec<usize> = (0..people.len()).collect();
    order.sort_by_key(|i| (people[*i].complexity, people[*i].id));
    let mut out: Vec<Mark> = Vec::new();
    let mut open: Option<Mark> = None;
    let close = |mark: Option<Mark>, out: &mut Vec<Mark>| out.extend(mark);
    for i in order {
        if people[i].kept() {
            out.push(Mark { x: x(i), people: vec![i] });
            continue;
        }
        match &mut open {
            Some(mark) if x(i) - mark.x < face + 2.0 => mark.people.push(i),
            _ => {
                close(open.take(), &mut out);
                open = Some(Mark { x: x(i), people: vec![i] });
            }
        }
    }
    close(open, &mut out);
    // the middle of a gathered mark
    for mark in &mut out {
        let sum: f32 = mark.people.iter().map(|i| x(*i)).sum();
        mark.x = sum / mark.people.len() as f32;
    }
    out
}

/// What the strip shows of the inspected avatar.
pub struct Subject<'a> {
    pub id: Uuid,
    /// Whose picture it shows; None when RLVa hides them.
    pub pic: Option<Uuid>,
    pub name: &'a str,
    /// Its complexity, without what is set aside.
    pub total: u32,
    /// What it would be without the selected item.
    pub without: Option<u32>,
    pub over: bool,
}

/// Draw the scale over `rect`. Returns the person whose face was clicked.
pub fn paint(
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    wanted: &mut std::collections::HashSet<Uuid>,
    rect: Rect,
    subject: &Subject<'_>,
    large: bool,
) {
    let others: Vec<Person> = frame.people.iter().filter(|o| o.id != subject.id).cloned().collect();
    let scale = scale(
        others.iter().map(|o| o.complexity).chain([subject.total]).chain(subject.without),
        frame.max_complexity,
    );
    let (face, main) = if large { (FACE + 6.0, MAIN + 6.0) } else { (FACE, MAIN) };
    let axis = Rect::from_min_max(
        egui::pos2(rect.left() + main / 2.0, rect.top()),
        egui::pos2(rect.right() - main / 2.0, rect.bottom()),
    );
    let text = 14.0 * crate::ui::fonts::scale(ui.ctx());
    let line = rect.bottom() - text - 5.0;
    let face_y = line - main / 2.0 - 5.0;
    let at = |v: f32| axis.left() + (v / scale.end).clamp(0.0, 1.0) * axis.width();
    if let Some(limit) = scale.limit {
        // above the limit: hatched
        let zone = Rect::from_min_max(egui::pos2(at(limit), rect.top()), egui::pos2(rect.right(), line));
        hatch(ui, zone, p.warn.gamma_multiply(0.22));
        ui.painter()
            .vline(zone.left(), zone.y_range(), Stroke::new(1.0, p.warn.gamma_multiply(0.8)));
    }
    ui.painter().hline(axis.x_range(), line, Stroke::new(1.5, p.muted_dim));
    for i in 0.. {
        let v = i as f32 * scale.step;
        if v > scale.end || i > 12 {
            break;
        }
        let label = if i == 0 { "0".to_owned() } else { super::model::short(v as u32) };
        let galley = ui.painter().layout_no_wrap(label, egui::FontId::proportional(11.0), p.muted);
        let x = at(v) - galley.size().x / 2.0;
        widgets::paint_ink_left(ui.painter(), galley, egui::pos2(x, rect.bottom() - text / 2.0), p.muted);
    }
    if frame.max_complexity > 0 {
        let limit = aurora_i18n::tr!("inspect-your-limit", limit = super::model::short(frame.max_complexity));
        let y = if scale.limit.is_some() {
            rect.top() + 6.0
        } else {
            rect.bottom() - text / 2.0
        };
        let galley = ui
            .painter()
            .layout_no_wrap(limit.clone(), egui::FontId::proportional(11.0), p.muted);
        let left = rect.right() - galley.size().x;
        // (over the last figure of the scale, if it comes that far)
        let ground = Rect::from_min_max(egui::pos2(left - 6.0, y - text / 2.0), egui::pos2(rect.right(), y + text / 2.0));
        if scale.limit.is_none() {
            ui.painter().rect_filled(
                ground,
                ui.visuals().widgets.inactive.corner_radius,
                p.raised.gamma_multiply(0.45).blend(p.panel.to_opaque()),
            );
        }
        widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left, y), p.muted);
    }
    let room = axis.width();
    for mark in marks(&others, &scale, room, face) {
        for i in &mark.people {
            let x = at(others[*i].complexity as f32);
            ui.painter().vline(x, line - 3.0..=line + 3.0, Stroke::new(1.0, p.muted));
        }
        let center = egui::pos2(axis.left() + mark.x, face_y + (main - face) / 2.0);
        let r = Rect::from_center_size(center, Vec2::splat(face));
        let first = &others[mark.people[0]];
        if mark.people.len() > 1 {
            ui.painter()
                .circle_filled(center, face / 2.0, p.raised.blend(p.muted.gamma_multiply(0.25)));
            let n = aurora_i18n::tr!("board-more-short", count = mark.people.len());
            let galley = ui.painter().layout_no_wrap(n, egui::FontId::proportional(10.5), p.ink);
            let left = center.x - galley.size().x / 2.0;
            widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left, center.y), p.ink);
        } else {
            wanted.extend(first.pic);
            let pic = first.pic.and_then(|id| frame.pics.get(&id));
            if first.you && pic.is_none() {
                ui.painter().circle_filled(center, face / 2.0, p.violet);
                let you = aurora_i18n::tr!("inspect-you");
                let galley = ui
                    .painter()
                    .layout_no_wrap(you.to_owned(), egui::FontId::proportional(9.0), p.on_accent);
                let left = center.x - galley.size().x / 2.0;
                widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left, center.y), p.on_accent);
            } else {
                widgets::avatar_disc(ui, p, r, pic, &first.name, false);
            }
        }
        let tip: Vec<String> = mark
            .people
            .iter()
            .map(|i| {
                let o = &others[*i];
                format!("{}  {}", o.shown_name(), aurora_i18n::integer(i64::from(o.complexity)))
            })
            .collect();
        let id = ui.scope_id().with(("inspect_face", first.id));
        ui.interact(r, id, Sense::hover()).on_hover_text(tip.join("\n"));
    }
    if let Some(without) = subject.without {
        let x = at(without as f32);
        let center = egui::pos2(x, face_y);
        ui.painter().circle_stroke(center, main / 2.0 - 3.0, Stroke::new(1.5, p.muted));
        ui.painter().vline(x, line - 3.0..=line + 3.0, Stroke::new(1.5, p.muted));
        let label = aurora_i18n::tr!("inspect-without-it", value = aurora_i18n::integer(i64::from(without)));
        let galley = ui
            .painter()
            .layout_no_wrap(label, egui::FontId::proportional(11.0), p.panel.to_opaque());
        let w = galley.size().x + 10.0;
        let left = (x - w / 2.0).clamp(rect.left(), (rect.right() - w).max(rect.left()));
        let r = Rect::from_min_size(egui::pos2(left, rect.bottom() - text), Vec2::new(w, text));
        ui.painter().rect_filled(r, ui.visuals().widgets.inactive.corner_radius, p.ink);
        widgets::paint_ink_left(ui.painter(), galley, egui::pos2(left + 5.0, r.center().y), p.panel.to_opaque());
    }
    let x = at(subject.total as f32);
    let center = egui::pos2(x, face_y);
    ui.painter().vline(x, line - 5.0..=line + 4.0, Stroke::new(2.0, p.ink));
    let ring = if subject.over { p.warn } else { p.ink };
    ui.painter().circle_filled(center, main / 2.0, p.panel.to_opaque());
    let r = Rect::from_center_size(center, Vec2::splat(main - 5.0));
    wanted.extend(subject.pic);
    let pic = subject.pic.and_then(|id| frame.pics.get(&id));
    widgets::avatar_disc(ui, p, r, pic, subject.name, false);
    ui.painter().circle_stroke(center, main / 2.0 - 1.0, Stroke::new(2.0, ring));
}

/// Slanted lines over `zone`.
fn hatch(ui: &egui::Ui, zone: Rect, color: egui::Color32) {
    if zone.width() < 1.0 {
        return;
    }
    let painter = ui.painter().with_clip_rect(zone.intersect(ui.clip_rect()));
    let mut x = zone.left() - zone.height();
    while x < zone.right() {
        painter.line_segment(
            [egui::pos2(x, zone.bottom()), egui::pos2(x + zone.height(), zone.top())],
            Stroke::new(1.5, color),
        );
        x += 7.0;
    }
}

/// The people of the grown strip: heaviest first.
pub fn table(
    ui: &mut egui::Ui,
    p: &Palette,
    frame: &Frame<'_>,
    wanted: &mut std::collections::HashSet<Uuid>,
    subject: &Subject<'_>,
    sort: &mut Option<(usize, bool)>,
) {
    let cols = [
        board::Col::text(aurora_i18n::tr!("inspect-person"), 1.0),
        board::Col::figure(aurora_i18n::tr!("inspect-totals-complexity"), 130.0),
        board::Col::figure(aurora_i18n::tr!("inspect-difference"), 120.0),
        board::Col::text(aurora_i18n::tr!("inspect-drawn-in-full"), 0.6),
    ];
    let mut people: Vec<Person> = frame.people.to_vec();
    if let Some(me) = people.iter_mut().find(|o| o.id == subject.id) {
        me.complexity = subject.total;
    }
    people.sort_by(|a, b| crate::natural_order::cmp(&a.name, &b.name));
    match sort.unwrap_or((1, true)) {
        (0, down) => {
            if down {
                people.reverse();
            }
        }
        (3, down) => people.sort_by_key(|o| o.drawn != down),
        (_, down) => people.sort_by_key(|o| {
            let v = i64::from(o.complexity);
            if down { -v } else { v }
        }),
    }
    board::header(ui, p, "inspect_people", &cols, sort);
    if people.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(aurora_i18n::tr!("inspect-nobody")).size(12.0).color(p.muted));
        return;
    }
    let nav = board::Nav::begin(ui, "inspect_people", people.len(), 8);
    egui::ScrollArea::vertical()
        .id_salt("inspect_people")
        .auto_shrink([false, false])
        .show_rows(ui, 27.0, people.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let o = &people[i];
                let me = o.id == subject.id;
                let lit = if me { board::Lit::On } else { board::Lit::Off };
                let (line, resp) = board::row(ui, p, &nav, i, 27.0, lit);
                let cells = board::col_rects(&cols, line);
                let pic = Rect::from_center_size(egui::pos2(cells[0].left() + 17.0, line.center().y), Vec2::splat(18.0));
                wanted.extend(o.pic);
                widgets::avatar_disc(ui, p, pic, o.pic.and_then(|id| frame.pics.get(&id)), &o.name, false);
                board::cell(ui, cells[0].with_min_x(pic.right()), o.shown_name(), p.ink);
                let value = aurora_i18n::integer(i64::from(o.complexity));
                board::figure_cell(ui, cells[1], &value, if o.drawn { p.ink } else { p.warn });
                let gap = i64::from(o.complexity) - i64::from(subject.total);
                let difference = match (me, gap) {
                    (true, _) => String::new(),
                    (_, g) if g > 0 => format!("+{}", aurora_i18n::integer(g)),
                    (_, g) => aurora_i18n::integer(g).replace('-', "−"),
                };
                board::figure_cell(ui, cells[2], &difference, p.muted);
                let drawn = if o.drawn {
                    aurora_i18n::tr!("common-yes")
                } else {
                    aurora_i18n::tr!("common-no")
                };
                board::cell(ui, cells[3], drawn, p.muted);
                let spoken = format!("{}, {value}, {difference}, {} {drawn}", o.shown_name(), cols[3].title);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &spoken));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(n: u128, complexity: u32, friend: bool) -> Person {
        Person {
            id: Uuid::from_u128(n),
            pic: None,
            name: format!("Person {n}"),
            complexity,
            you: false,
            friend,
            drawn: true,
        }
    }

    #[test]
    fn the_scale_follows_the_crowd_and_shows_a_limit_that_is_near() {
        let quiet = scale([20_000, 90_000].into_iter(), 350_000);
        assert_eq!(quiet.limit, None, "a far limit leaves the scale to the people");
        assert!(quiet.end < 120_000.0 && quiet.end > 90_000.0);
        assert_eq!(quiet.step, 50_000.0);
        let heavy = scale([400_000].into_iter(), 350_000);
        assert_eq!(heavy.limit, Some(350_000.0));
        assert!(heavy.end > 400_000.0);
        // no limit, nobody
        assert_eq!(
            scale([].into_iter(), 0),
            Scale {
                end: 1_000.0,
                step: 500.0,
                limit: None
            }
        );
    }

    #[test]
    fn faces_that_would_touch_are_gathered_but_never_a_friend() {
        let people = [
            person(1, 1_000, false),
            person(2, 1_010, false),
            person(3, 1_020, true),
            person(4, 9_000, false),
        ];
        let s = Scale {
            end: 10_000.0,
            step: 5_000.0,
            limit: None,
        };
        let marks = marks(&people, &s, 500.0, 20.0);
        let mut sizes: Vec<Vec<usize>> = marks.iter().map(|m| m.people.clone()).collect();
        sizes.sort();
        assert_eq!(sizes, [vec![0, 1], vec![2], vec![3]]);
        assert!(marks.iter().all(|m| m.x >= 0.0 && m.x <= 500.0));
    }
}
