//! A plate to play with in Preferences › Just for fun: the pill of a
//! sample resident that grows into a full plate under the pointer and
//! answers it, and clicks, as the plates in the world do with the options
//! of the page. Its clicks are counted for the session only: the tally of
//! the residents (`combo::Tally`) is not touched.

use super::{Content, Level, Picture, Presence, Role, Tag, combo, fidget, paint};
use crate::theme::Palette;
use egui::{Pos2, Rect, Sense, Vec2};
use std::sync::{Arc, Mutex};

/// What the sample plate keeps from one frame to the next.
#[derive(Clone, Copy)]
struct Play {
    level: Level,
    from: Level,
    changed: f64,
    seen: f64,
    /// Under the pointer last frame, and when it last was.
    pointer: bool,
    hover_seen: f64,
    hover: f32,
    bounced: f64,
    wobble: fidget::Wobble,
    combo: combo::Combo,
    clicks: u32,
    /// The plate's rectangle last frame: what the pointer is tested on.
    rect: Rect,
}

impl Default for Play {
    fn default() -> Self {
        Play {
            level: Level::Compact,
            from: Level::Compact,
            changed: f64::NEG_INFINITY,
            seen: f64::NEG_INFINITY,
            pointer: false,
            hover_seen: f64::NEG_INFINITY,
            hover: 0.0,
            bounced: f64::NEG_INFINITY,
            wobble: fidget::Wobble::default(),
            combo: combo::Combo::default(),
            clicks: 0,
            rect: Rect::NOTHING,
        }
    }
}

fn sample() -> Content {
    Content {
        appearance: Default::default(),
        name: aurora_i18n::tr!("contact-visual-sample-name").to_owned(),
        username: None,
        title: Some(aurora_i18n::tr!("contact-visual-sample-title").to_owned()),
        detail: None,
        role: Role::Friend,
        presence: Presence::Online,
        in_voice: false,
        typing: false,
        friend: true,
        too_complex: false,
        render_never: false,
        blocked: false,
        mark: None,
    }
}

/// The sample plate, `jiggle` and `click_combo` as the options are.
pub fn show(ui: &mut egui::Ui, p: &Palette, jiggle: bool, click_combo: bool, still: bool) {
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 130.0), Sense::click());
    let id = ui.make_persistent_id(egui::IdSalt::new("plate-playground"));
    let shared: Arc<Mutex<Play>> = ui.data_mut(|d| d.get_temp_mut_or_default::<Arc<Mutex<Play>>>(id).clone());
    let Ok(mut play) = shared.lock() else {
        return;
    };
    let now = ui.input(|i| i.time);
    let dt = (now - play.seen).clamp(0.0, 0.25) as f32;
    play.seen = now;
    let jiggle = jiggle && !still;

    let pointer = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|at| play.rect.contains(*at) && ui.is_rect_visible(area));
    let over = pointer.is_some() && ui.rect_contains_pointer(area);
    if over {
        play.hover_seen = now;
    }
    if jiggle && over != play.pointer {
        if over {
            play.wobble.enter();
        } else {
            play.wobble.leave();
        }
    }
    let hovered = super::hover_holds(play.hover_seen, now);
    if over && !play.pointer && !still && !jiggle {
        play.bounced = now;
    }
    play.pointer = over;
    let level = if hovered { Level::Full } else { Level::Compact };
    if level != play.level {
        if jiggle && super::grows(play.level, level) {
            play.wobble.expand();
        }
        play.from = play.level;
        play.level = level;
        play.changed = now;
    }
    if over && click_combo && ui.input(|i| i.pointer.primary_pressed()) {
        play.clicks = play.clicks.saturating_add(1);
        if !still {
            play.wobble.press();
        }
        let total = play.clicks;
        play.combo.click(now, total, still);
    }
    if still {
        play.wobble = fidget::Wobble::default();
    }
    play.wobble.step(dt);
    play.combo.step(dt);
    let step = dt / super::HOVER_EASE as f32;
    play.hover = if hovered {
        (play.hover + step).min(1.0)
    } else {
        (play.hover - step).max(0.0)
    };

    let ease = if jiggle { fidget::EASE } else { super::LEVEL_EASE };
    let progress = ((now - play.changed) / ease).clamp(0.0, 1.0) as f32;
    let smooth = progress * progress * (3.0 - 2.0 * progress);
    let state = paint::State {
        level: play.level,
        from: (progress < 1.0).then_some(play.from),
        progress: smooth,
        size_progress: if jiggle && super::grows(play.from, play.level) {
            fidget::overshoot(progress)
        } else {
            smooth
        },
        halo: None,
        glass: 0.0,
        hover: play.hover * play.hover * (3.0 - 2.0 * play.hover),
        grow: if still {
            Vec2::ZERO
        } else {
            Vec2::splat(super::bounce_grow(now - play.bounced)) + play.wobble.grow()
        },
        still,
    };
    let tag = Tag {
        id: uuid::Uuid::nil(),
        anchor: Pos2::new(area.center().x, area.bottom() - 40.0),
        at: glam::Vec3::ZERO,
        distance: 5.0,
        fade: 1.0,
        content: sample(),
        voice: None,
        loading: None,
        picture: Picture::None,
    };
    let icons = crate::ui::icons::Icons::default();
    let env = paint::Env::new(ui.painter(), p, &icons, now);
    let (rect, surface) = paint::plate(&env, &tag, &state, &mut paint::Cache::default());
    play.rect = rect;
    if click_combo && play.combo.busy(now) {
        combo::paint(&env, surface, &play.combo, play.clicks, 1.0, still);
    }
    if over && click_combo {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let easing = play.hover > 0.0 && play.hover < 1.0;
    let moving = !play.wobble.still() || play.combo.busy(now) || progress < 1.0 || easing;
    if moving || hovered || now - play.bounced < super::BOUNCE {
        ui.ctx().request_repaint();
    }
}
