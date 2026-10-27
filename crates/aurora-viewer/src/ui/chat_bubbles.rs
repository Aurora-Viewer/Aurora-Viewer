//! Nearby chat in bubbles above the residents' heads (#673, off by
//! default): what each one said lately, under its name tag, which rises to
//! make room. Port of the bubble mode of LLVOAvatar::idleUpdateNameTagText
//! and LLHUDNameTag::renderText (indra/newview/llvoavatar.cpp,
//! llhudnametag.cpp, originally LGPL 2.1): the lines oldest first, left
//! aligned, a whisper in italics and a shout in bold, fading with their
//! age (`world::bubble_chat`), then the typing dots. The bubble follows the
//! name tags' style: a card of the theme in glass under a plate, the
//! classic tag's flat card with shadowed text under a classic tag.

use crate::theme::Palette;
use crate::world::World;
use crate::world::bubble_chat::{BubbleChat, Voice};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Galley, Pos2, Rect, Vec2};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// NAMETAG_MAX_WIDTH: the lines wrap past it.
const MAX_WIDTH: f32 = 298.0;
/// LLHUDNameTag's setMaxLines(11): the rows of the newest lines.
const MAX_ROWS: usize = 11;
const TEXT_SIZE: f32 = 12.0;
const PADDING: Vec2 = Vec2::new(8.0, 5.0);
/// Between two lines, and between the name tag and the bubble.
pub const GAP: f32 = 3.0;
/// The bubbles are drawn up to 40 m away, then fade out over 5 m
/// (LLHUDNameTag::setFadeDistance(CHAT_NORMAL_RADIUS * 2, 5) in bubble
/// mode), whatever the name tags' distance.
const FULL_DISTANCE: f32 = 40.0;
const FADE_DISTANCE: f32 = 5.0;
/// Between the centres of two typing dots.
const DOT_STEP: f32 = 7.0;

/// What the bubbles need besides the world, given while the setting is on.
pub struct Frame<'a> {
    pub chat: &'a BubbleChat,
    /// UseTypingBubbles.
    pub typing: bool,
    /// We type in the nearby chat (`ChatUi::local_typing`).
    pub own_typing: bool,
    /// ChatBubbleOpacity: of the background only.
    pub opacity: f32,
    pub now: Instant,
}

/// A bubble laid out for this frame.
pub struct Bubble {
    rows: Vec<Arc<Galley>>,
    /// The typing dots' row, last, in the newest lines' colour.
    typing: Option<Color32>,
    /// Its whole size, padding included.
    pub size: Vec2,
    /// The distance fade.
    alpha: f32,
}

fn distance_alpha(distance: f32) -> f32 {
    ((FULL_DISTANCE + FADE_DISTANCE - distance) / FADE_DISTANCE).clamp(0.0, 1.0)
}

/// The colour of a line of `tone` (`world::bubble_chat::tone`): new, then
/// normal and old, each a step toward grey (lerp 0.7 to grey 0.8, then to
/// grey 0.6 in the Legacy Viewer; the muted inks of the theme here).
fn line_color(p: &Palette, new: Color32, tone: f32) -> Color32 {
    let normal = new.lerp_to_gamma(p.muted, 0.7);
    let old = normal.lerp_to_gamma(p.muted_dim, 0.7);
    if tone <= 1.0 {
        new.lerp_to_gamma(normal, tone)
    } else {
        normal.lerp_to_gamma(old, tone - 1.0)
    }
}

/// The text a bubble shows for a line: links by their label, an emote
/// after the speaker's name, a whisper or a shout saying so.
fn line_text(world: &World, line: &crate::world::bubble_chat::Utterance) -> String {
    let text = super::chat::resolved_text(world, &line.text);
    if line.emote() {
        let from = world.rlva_filter_text(&line.from);
        return format!("{from}{}", text.get(3..).unwrap_or_default());
    }
    let prefix = match line.voice {
        Voice::Normal => "",
        Voice::Whisper => aurora_i18n::tr!("status-whispers-2"),
        Voice::Shout => aurora_i18n::tr!("status-shouts-2"),
    };
    format!("{prefix}{text}")
}

fn typing(world: &World, frame: &Frame<'_>, id: Uuid) -> bool {
    frame.typing
        && if id == world.agent_id {
            frame.own_typing
        } else {
            world.is_typing_nearby(&id)
        }
}

/// Has the avatar `id`, `distance` metres from the camera, a bubble to
/// show? Its name tag then shows too, whatever the name tags' settings.
pub fn shows(world: &World, frame: &Frame<'_>, id: Uuid, distance: f32) -> bool {
    distance_alpha(distance) > 0.0 && (frame.chat.has_lines(&id, frame.now) || typing(world, frame, id))
}

/// The bubble of the avatar `id`, `distance` metres from the camera; None
/// when it has nothing to show (then its name tag follows its own rules).
pub fn layout(painter: &egui::Painter, p: &Palette, world: &World, frame: &Frame<'_>, id: Uuid, distance: f32) -> Option<Bubble> {
    let alpha = distance_alpha(distance);
    if alpha <= 0.0 {
        return None;
    }
    let own = id == world.agent_id;
    let typing = typing(world, frame, id);
    let lines: Vec<_> = frame.chat.lines(&id, frame.now).collect();
    if lines.is_empty() && !typing {
        return None;
    }
    // UserChatColor / AgentChatColor, then the contact set's chat colour
    let new = world
        .contact_sets
        .appearance(&id)
        .chat_text
        .map(|c| crate::interface_colors::on_panel(super::colors::set_color(c)))
        .unwrap_or(if own { p.violet_pale } else { p.ink });
    let semibold = super::fonts::semibold(painter.ctx());
    // the newest lines first, while they fit in the rows (the typing dots
    // take one)
    let room = MAX_ROWS - usize::from(typing);
    let mut rows: Vec<Arc<Galley>> = Vec::new();
    let mut used = 0;
    for (line, tone) in lines.iter().rev() {
        if used >= room {
            break;
        }
        let format = TextFormat {
            color: line_color(p, new, *tone),
            font_id: FontId::new(
                TEXT_SIZE,
                if line.voice == Voice::Shout {
                    semibold.clone()
                } else {
                    egui::FontFamily::Proportional
                },
            ),
            italics: line.voice == Voice::Whisper,
            ..Default::default()
        };
        let mut job = LayoutJob::single_section(line_text(world, line), format);
        job.wrap = TextWrapping {
            max_width: MAX_WIDTH,
            max_rows: room - used,
            ..Default::default()
        };
        let galley = super::bidi::galley(painter.ctx(), job, super::bidi::Base::Auto);
        used += galley.rows.len().max(1);
        rows.push(galley);
    }
    rows.reverse();
    let text_w = rows.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let text_h: f32 = rows.iter().map(|g| g.size().y).sum::<f32>() + GAP * rows.len().saturating_sub(1) as f32;
    let dots_h = if typing { TEXT_SIZE } else { 0.0 };
    let dots_w = if typing { 3.0 * DOT_STEP } else { 0.0 };
    let gap = if typing && !rows.is_empty() { GAP } else { 0.0 };
    Some(Bubble {
        rows,
        typing: typing.then_some(new),
        size: Vec2::new(text_w.max(dots_w), text_h + gap + dots_h) + PADDING * 2.0,
        alpha,
    })
}

/// Paint `bubble` with its top centre at `top`, at `fade` (the name tag's).
/// `plate`: under a name plate, in glass; else under a classic tag.
#[allow(clippy::too_many_arguments)]
pub fn paint(painter: &egui::Painter, p: &Palette, bubble: &Bubble, top: Pos2, fade: f32, opacity: f32, plate: bool, now: f64) {
    let alpha = bubble.alpha * fade;
    if alpha <= 0.0 {
        return;
    }
    // on whole pixels, as the plates: the text stays sharp as the avatar moves
    let ppp = painter.pixels_per_point();
    let min = Pos2::new(top.x - bubble.size.x * 0.5, top.y);
    let rect = Rect::from_min_size(Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), bubble.size);
    let radius = painter.ctx().global_style().visuals.window_corner_radius;
    let tint = p.bar.gamma_multiply(opacity);
    if plate && p.panel_alpha < 255 {
        painter.add(aurora_render::glass_panel_faded(rect, f32::from(radius.nw), tint, alpha));
    } else {
        painter.rect_filled(rect, radius, tint.gamma_multiply(alpha));
    }
    // each line's colour is in its galley: the text fades as a whole
    let mut faded = painter.clone();
    faded.multiply_opacity(alpha);
    let mut y = rect.top() + PADDING.y;
    for galley in &bubble.rows {
        let at = Pos2::new(rect.left() + PADDING.x, y);
        if !plate {
            // the classic tag's drop shadow (LLFontGL::DROP_SHADOW)
            faded.galley_with_override_text_color(at + Vec2::splat(1.0), galley.clone(), p.navy.gamma_multiply(0.7));
        }
        faded.galley(at, galley.clone(), Color32::PLACEHOLDER);
        y += galley.size().y + GAP;
    }
    if let Some(color) = bubble.typing {
        dots(&faded, color, Pos2::new(rect.left() + PADDING.x, y + TEXT_SIZE * 0.5), plate, now);
    }
}

/// The typing dots: the plates' three bouncing dots (`nameplate::typing_dot`),
/// or the Legacy Viewer's ".", "..", "..." every third of a second.
fn dots(painter: &egui::Painter, color: Color32, left: Pos2, plate: bool, now: f64) {
    let shown = if plate { 3 } else { ((now * 3.0).floor() as usize + 2) % 3 + 1 };
    for k in 0..shown {
        let (opacity, lift) = if plate { super::nameplate::typing_dot(now, k) } else { (1.0, 0.0) };
        let c = Pos2::new(left.x + DOT_STEP * (k as f32 + 0.5), left.y + lift);
        painter.circle_filled(c, 2.0, color.gamma_multiply(opacity));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubbles_show_to_40_metres_then_fade_over_5() {
        assert_eq!(distance_alpha(10.0), 1.0);
        assert_eq!(distance_alpha(40.0), 1.0);
        assert!((distance_alpha(42.5) - 0.5).abs() < 1e-5);
        assert_eq!(distance_alpha(45.0), 0.0);
        assert_eq!(distance_alpha(120.0), 0.0);
    }

    #[test]
    fn older_lines_turn_grey_step_by_step() {
        let p = crate::theme::Theme::default().palette();
        let new = p.ink;
        assert_eq!(line_color(&p, new, 0.0), new);
        let normal = line_color(&p, new, 1.0);
        let old = line_color(&p, new, 2.0);
        assert_ne!(normal, new);
        assert_ne!(old, normal);
        // the blend between two steps lies between them
        let half = line_color(&p, new, 1.5);
        let between = |a: u8, b: u8, c: u8| a.min(b) <= c && c <= a.max(b);
        assert!(between(normal.r(), old.r(), half.r()));
    }
}
