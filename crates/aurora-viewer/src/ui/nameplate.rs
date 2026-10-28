//! Avatar name plates: the rounded glass plate floating above an avatar,
//! with its profile picture, a status dot, up to three lines and round
//! badges (docs/BRANDING.md, "Name plate"). `ui::hud` decides which
//! avatars get one and where; this module holds what a plate shows at each
//! distance level, its layout, and what it remembers of an avatar from one
//! frame to the next (level, voice envelope, picture request).
//!
//! The states are those of LLVOAvatar::idleUpdateNameTagText
//! (indra/newview/llvoavatar.cpp, originally LGPL 2.1): away, do not
//! disturb, typing, muted, friend, too complex. The presentation is
//! Aurora's own: Firestorm writes them in a status line above the name.

use crate::settings::ColorSettings;
use crate::theme::Palette;
use egui::{Color32, Pos2, Rect, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(crate) mod aurora;
pub(crate) mod combo;
mod fidget;
mod linden;
pub(crate) mod mark;
mod mesh;
mod paint;
pub(crate) mod playground;
#[cfg(test)]
mod tests;

/// Size of the display name on a full plate: the name size of the classic
/// tags. The mockup the plate comes from is drawn larger (17 px names);
/// every other measure follows in proportion.
pub const NAME_PT: f32 = 13.0;
const MOCKUP_NAME_PX: f32 = 17.0;
/// Mockup pixels to points.
pub const SCALE: f32 = NAME_PT / MOCKUP_NAME_PX;

/// Camera distance up to which a plate is complete.
pub const FULL_DISTANCE: f32 = 15.0;
/// Camera distance up to which a plate is a compact pill; beyond, the name
/// alone.
pub const COMPACT_DISTANCE: f32 = 40.0;
/// How far past a threshold the distance must go before a plate changes
/// level: an avatar standing at 15 m does not flip between two plates.
pub const LEVEL_MARGIN: f32 = 1.5;
/// Seconds a plate takes to glide from one level to the next.
pub const LEVEL_EASE: f64 = 0.18;
/// Complete plates shown at once, nearest first. In a crowd the others
/// are compact pills: forty complete plates would hide each other and the
/// people under them.
pub const MAX_FULL: usize = 8;
/// A complete plate keeps its place until this many more avatars are
/// nearer than the limit allows: two avatars at the same distance do not
/// swap their plates back and forth.
pub const FULL_RANK_MARGIN: usize = 2;
/// Full and compact plates that blur the world behind them, nearest first;
/// the others (and the far pills) only tint it.
pub const MAX_GLASS: usize = 16;
/// Pictures asked per second (profile, then texture): like the radar's
/// profiles per sweep, a crowd does not fire dozens of requests at once.
pub const PICTURES_PER_SECOND: u32 = 4;
/// Widest text of a plate, in mockup pixels; longer lines are elided.
const MAX_TEXT_PX: f32 = 250.0;

/// What a plate shows, by distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Picture, status dot, three lines, floating badges.
    Full,
    /// The display name and the badges in a smaller pill.
    Compact,
    /// The display name alone.
    Far,
}

/// Level of a plate at `distance`, given the level it had: it changes only
/// `LEVEL_MARGIN` past a threshold.
pub fn level_for(distance: f32, previous: Option<Level>) -> Level {
    let m = LEVEL_MARGIN;
    let (full, compact) = match previous {
        None => (FULL_DISTANCE, COMPACT_DISTANCE),
        Some(Level::Full) => (FULL_DISTANCE + m, COMPACT_DISTANCE + m),
        Some(Level::Compact) => (FULL_DISTANCE - m, COMPACT_DISTANCE + m),
        Some(Level::Far) => (FULL_DISTANCE - m, COMPACT_DISTANCE - m),
    };
    if distance <= full {
        Level::Full
    } else if distance <= compact {
        Level::Compact
    } else {
        Level::Far
    }
}

/// "Full plates for friends only" (off by default, the human's choice of
/// 2026-10-14): a resident who is not a friend stops at the compact pill;
/// ours stays complete. The far pill still follows the distance.
pub fn friends_level(level: Level, c: &Content, friends_only: bool) -> Level {
    if friends_only && level == Level::Full && !c.friend && c.role != Role::Own {
        Level::Compact
    } else {
        level
    }
}

/// Set sizes are preferences at close range, not a way to defeat distance,
/// crowd limits or hover expansion. Expanded also admits a non-friend set
/// member to the full-plate candidates, as the friend eligibility rule does.
pub fn preferred_level(level: Level, c: &Content, friends_only: bool) -> Level {
    use crate::world::contact_sets::PlateSize;
    match c.appearance.size {
        PlateSize::Reduced if level == Level::Full => Level::Compact,
        PlateSize::Expanded => level,
        _ => friends_level(level, c, friends_only),
    }
}

/// Seconds a plate stays full after the pointer has left it: along its edge
/// the pointer goes in and out without the plate blinking.
pub const HOVER_LINGER: f64 = 0.3;

/// "Expand a plate on hover" (on by default, the human's request of
/// 2026-10-14): the plate under the pointer is full whatever its distance,
/// the crowd or "full plates for friends only".
pub fn hover_level(level: Level, hovered: bool, expand_on_hover: bool) -> Level {
    if expand_on_hover && hovered { Level::Full } else { level }
}

/// Whether the plate last under the pointer at `seen` still counts as
/// hovered at `now`.
pub fn hover_holds(seen: f64, now: f64) -> bool {
    now - seen <= HOVER_LINGER
}

/// Seconds the light of a hovered plate takes to come and to go.
pub const HOVER_EASE: f64 = 0.12;
/// Seconds of the bounce of a plate the pointer reaches (#473), and the
/// points its surface grows on each side at most: a plate already full
/// answers the pointer too.
pub const BOUNCE: f64 = 0.36;
pub const BOUNCE_GROW: f32 = 3.0;

/// Points the surface of a plate reached `t` seconds ago has grown on each
/// side: a short damped spring, out about two points, half a point in, then
/// still. Only the surface moves: the text and the picture stay sharp.
pub fn bounce_grow(t: f64) -> f32 {
    if !(0.0..BOUNCE).contains(&t) {
        return 0.0;
    }
    let u = (t / BOUNCE) as f32;
    BOUNCE_GROW * (1.0 - u).powi(2) * (3.0 * std::f32::consts::PI * u).sin()
}

/// Level of a plate among the others: `level` is what its distance gives,
/// `rank` its place among the plates near enough to be complete (0 for the
/// nearest), `was_full` whether it was complete in the last frame.
pub fn crowd_level(level: Level, rank: usize, was_full: bool) -> Level {
    let limit = if was_full { MAX_FULL + FULL_RANK_MARGIN } else { MAX_FULL };
    if level == Level::Full && rank >= limit {
        Level::Compact
    } else {
        level
    }
}

/// Status dot of the picture (ANIM_AGENT_AWAY / ANIM_AGENT_DO_NOT_DISTURB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Online,
    Away,
    Busy,
}

impl Presence {
    /// The same status as the Conversations window's dot (`ui/presence.rs`).
    pub fn status(self) -> crate::ui::presence::Status {
        use crate::ui::presence::Status;
        match self {
            Presence::Online => Status::Online,
            Presence::Away => Status::Away,
            Presence::Busy => Status::Busy,
        }
    }
}

/// Who the avatar is to us: the color and style of its display name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Resident,
    Own,
    Friend,
    Linden,
    Blocked,
}

impl Role {
    /// A blocked resident reads as blocked whoever it is; then Linden Lab
    /// staff, then friends (the order of LLVOAvatar::getNameTagColor).
    pub fn of(own: bool, blocked: bool, linden: bool, friend: bool) -> Role {
        if own {
            Role::Own
        } else if blocked {
            Role::Blocked
        } else if linden {
            Role::Linden
        } else if friend {
            Role::Friend
        } else {
            Role::Resident
        }
    }

    /// Color of the display name: the theme's ink for everyone, unless the
    /// names are colored by relation (Preferences › Colors, off by default,
    /// shared with the classic tags; the human's choice of 2026-10-14).
    pub fn name_color(self, p: &Palette, k: &ColorSettings) -> Color32 {
        use crate::ui::colors::c;
        if !k.relation_names {
            return p.ink;
        }
        match self {
            Role::Resident => p.ink,
            Role::Own => c(k.tag_me),
            Role::Friend => c(k.tag_friend),
            Role::Linden => c(k.tag_linden),
            Role::Blocked => c(k.tag_muted),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Contact {
        icon: usize,
        color: [u8; 4],
    },
    More(usize),
    /// In the voice channel.
    Voice,
    Typing,
    /// A green heart (the human's choice of 2026-10-14; it was light violet).
    Friend,
    /// Drawn as a silhouette: over the complexity or surface limit.
    TooComplex,
    /// Drawn as a silhouette by the resident's Never fully render setting.
    RenderNever,
    Blocked,
}

/// The badges of a plate, in drawing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Badges {
    list: [Option<Badge>; 14],
    len: usize,
}

impl Badges {
    fn push(&mut self, badge: Badge) {
        self.list[self.len] = Some(badge);
        self.len += 1;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn iter(&self) -> impl Iterator<Item = Badge> + '_ {
        self.list.iter().flatten().copied()
    }
}

/// Complexity of the avatar (Firestorm FSTagShowARW), when the options show it.
#[derive(Debug, Clone, PartialEq)]
pub struct Detail {
    /// On a line of its own: "Complexity: 245,000".
    pub full: String,
    /// After the username when the plate already has its three lines.
    pub short: String,
    /// Over the limit: in the color of the too-complex badge.
    pub alert: bool,
}

/// Everything a plate says about an avatar.
#[derive(Debug, Clone, PartialEq)]
pub struct Content {
    pub appearance: std::sync::Arc<crate::world::contact_sets::Appearance>,
    pub name: String,
    pub username: Option<String>,
    pub title: Option<String>,
    pub detail: Option<Detail>,
    pub role: Role,
    pub presence: Presence,
    pub in_voice: bool,
    pub typing: bool,
    pub friend: bool,
    pub too_complex: bool,
    pub render_never: bool,
    pub blocked: bool,
    /// A member of the Aurora team or a Linden Lab employee
    /// (`world::staff`): the plate has its light and its tag (`mark.rs`).
    pub mark: Option<crate::world::staff::Mark>,
}

/// A live editor preview shares the live geometry, contrast and badge code.
pub fn preview(ui: &mut egui::Ui, p: &Palette, appearance: std::sync::Arc<crate::world::contact_sets::Appearance>) {
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 100.0), egui::Sense::hover());
    let level = if appearance.size == crate::world::contact_sets::PlateSize::Reduced {
        Level::Compact
    } else {
        Level::Full
    };
    let content = Content {
        appearance,
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
    };
    let tag = Tag {
        id: Uuid::nil(),
        anchor: Pos2::new(area.center().x, area.bottom() - 12.0),
        at: glam::Vec3::ZERO,
        distance: 5.0,
        fade: 1.0,
        content,
        voice: None,
        loading: None,
        picture: Picture::None,
    };
    let icons = crate::ui::icons::Icons::default();
    let env = paint::Env::new(ui.painter(), p, &icons, ui.input(|i| i.time));
    let state = paint::State {
        level,
        from: None,
        progress: 1.0,
        size_progress: 1.0,
        halo: None,
        glass: 0.0,
        hover: 0.0,
        grow: Vec2::ZERO,
        still: false,
    };
    paint::plate(&env, &tag, &state, &mut paint::Cache::default());
}

/// A line under the display name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SubLine<'a> {
    /// The username, followed by the short complexity when the group title
    /// takes the third line.
    Username(&'a str, Option<&'a Detail>),
    Title(&'a str),
    Detail(&'a Detail),
}

/// The lines under the name: none below the full level; at most two, a
/// missing one leaving no empty line.
pub fn sub_lines(level: Level, c: &Content) -> [Option<SubLine<'_>>; 2] {
    if level != Level::Full {
        return [None, None];
    }
    let (user, title, detail) = (c.username.as_deref(), c.title.as_deref(), c.detail.as_ref());
    match (user, title, detail) {
        (Some(u), Some(t), d) => [Some(SubLine::Username(u, d)), Some(SubLine::Title(t))],
        (Some(u), None, d) => [Some(SubLine::Username(u, None)), d.map(SubLine::Detail)],
        (None, Some(t), d) => [Some(SubLine::Title(t)), d.map(SubLine::Detail)],
        (None, None, d) => [d.map(SubLine::Detail), None],
    }
}

/// The badges of a plate: none on the far pill; a green microphone in the
/// voice channel, always first, and nothing outside it (the human's choice
/// of 2026-10-14, replacing the red struck-through microphone).
pub fn badges(level: Level, c: &Content) -> Badges {
    let mut b = Badges::default();
    if level == Level::Far {
        return b;
    }
    for (on, badge) in [
        (c.in_voice, Badge::Voice),
        (c.typing, Badge::Typing),
        (c.friend && !c.appearance.replace_friend, Badge::Friend),
        (c.too_complex, if c.render_never { Badge::RenderNever } else { Badge::TooComplex }),
        (c.blocked, Badge::Blocked),
    ] {
        if on {
            b.push(badge);
        }
    }
    for icon in c.appearance.icons.iter().take(8) {
        b.push(Badge::Contact {
            icon: icon.icon,
            color: crate::ui::colors::set_color(icon.color).to_array(),
        });
    }
    if c.appearance.icons.len() > 8 {
        b.push(Badge::More(c.appearance.icons.len() - 8));
    }
    b
}

/// The status dot: on the picture only, and not for a blocked resident.
pub fn status_dot(level: Level, c: &Content) -> Option<Presence> {
    (level == Level::Full && !c.blocked).then_some(c.presence)
}

/// Measures of a plate in points, from the mockup's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub height: f32,
    /// From the plate's left edge to its picture or text, border included.
    pub pad_left: f32,
    pub pad_right: f32,
    /// Between the picture and the text, the text and the inline badges.
    pub gap: f32,
    pub name_pt: f32,
    pub sub_pt: f32,
    /// Diameter of the picture; 0 without one.
    pub photo: f32,
    pub badge: f32,
    pub badge_icon: f32,
    pub badge_gap: f32,
    pub max_text: f32,
    /// Mockup pixels to points, for the small fixed measures.
    pub scale: f32,
}

impl Metrics {
    pub fn of(level: Level, scale: f32) -> Metrics {
        // (height, left, right, gap, name, photo, badge, badge icon)
        let (h, l, r, gap, name, photo, badge, icon) = match level {
            Level::Full => (66.0, 7.0, 23.0, 12.0, MOCKUP_NAME_PX, 54.0, 25.0, 14.0),
            Level::Compact => (40.0, 15.0, 15.0, 8.0, 14.5, 0.0, 21.0, 12.0),
            Level::Far => (26.0, 12.0, 12.0, 0.0, 12.5, 0.0, 0.0, 0.0),
        };
        Metrics {
            // an even height: the pill's ends are exact half circles
            height: (h * scale * 0.5).round() * 2.0,
            pad_left: l * scale,
            pad_right: r * scale,
            gap: gap * scale,
            name_pt: name * scale,
            sub_pt: 11.5 * scale,
            photo: photo * scale,
            badge: badge * scale,
            badge_icon: icon * scale,
            badge_gap: 5.0 * scale,
            max_text: MAX_TEXT_PX * scale,
            scale,
        }
    }
}

/// Line box of a text of `size`, as the mockup's `line-height: 1.18`.
const LINE_HEIGHT: f32 = 1.18;

/// Sizes of the laid out lines of a plate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextSize {
    pub name: Vec2,
    pub subs: [Option<Vec2>; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle {
    pub center: Pos2,
    pub radius: f32,
}

/// Where everything goes, relative to the plate's top left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub size: Vec2,
    pub photo: Option<Circle>,
    /// The status dot with its ring in the plate's color.
    pub dot: Option<Circle>,
    /// Top left of the name and of the lines under it.
    pub name: Pos2,
    pub subs: [Option<Pos2>; 2],
    /// Left and right of the text column.
    pub text_x: (f32, f32),
    pub badges: [Rect; 14],
}

/// Lay a plate out: the picture at the left of a full plate, the lines
/// stacked and centered in its height, the badges floating over the top
/// right edge (full) or after the name (compact).
#[cfg(test)]
pub fn layout(level: Level, m: &Metrics, text: &TextSize, badges: usize) -> Layout {
    layout_with(level, m, text, badges, 0.0)
}

/// `layout`, with room for a tag `lead` points wide floating over the top
/// edge from the text's left (the "Aurora Viewer" tag): the plate is at
/// least as wide, and on a full plate its floating badges start past it.
pub fn layout_with(level: Level, m: &Metrics, text: &TextSize, badges: usize, lead: f32) -> Layout {
    let s = m.scale;
    let text_left = if m.photo > 0.0 { m.pad_left + m.photo + m.gap } else { m.pad_left };
    let text_w = text.subs.iter().flatten().map(|v| v.x).fold(text.name.x, f32::max);
    let row = if badges > 0 {
        badges as f32 * m.badge + (badges - 1) as f32 * m.badge_gap
    } else {
        0.0
    };
    let inline = if level == Level::Compact && badges > 0 { m.gap + row } else { 0.0 };
    let mut width = text_left + text_w + inline + m.pad_right;
    if level == Level::Full {
        // room for the floating badges, which start right of the picture
        width = width.max(text_left + row + 21.0 * s);
    }
    if lead > 0.0 {
        let past = if level == Level::Full && badges > 0 {
            m.gap + row + 21.0 * s
        } else {
            m.pad_right
        };
        width = width.max(text_left + lead + past);
    }
    let width = width.ceil();

    // the lines, each centered in its line box, the block in the height
    let name_box = (m.name_pt * LINE_HEIGHT).max(text.name.y);
    let sub_box = text.subs.iter().flatten().map(|size| size.y).fold(m.sub_pt * LINE_HEIGHT, f32::max);
    let lines = text.subs.iter().flatten().count() as f32;
    let h = m.height.max(name_box + lines * sub_box + 6.0 * s);
    let mut y = (h - name_box - lines * sub_box) * 0.5;
    let name = Pos2::new(text_left, y + (name_box - text.name.y) * 0.5);
    y += name_box;
    let mut subs = [None, None];
    for (slot, size) in subs.iter_mut().zip(text.subs.iter().flatten()) {
        *slot = Some(Pos2::new(text_left, y + (sub_box - size.y) * 0.5));
        y += sub_box;
    }

    let photo = (m.photo > 0.0).then(|| Circle {
        center: Pos2::new(m.pad_left + m.photo * 0.5, h * 0.5),
        radius: m.photo * 0.5,
    });
    // a 15 px disc (3 px of ring) in the bottom right corner of the picture
    let dot = photo.map(|c| Circle {
        center: c.center + Vec2::splat(c.radius - 7.5 * s),
        radius: 7.5 * s,
    });

    let mut rects = [Rect::NOTHING; 14];
    let (mut x, top) = match level {
        // 15 px above the inner top edge, 20 px from the inner right edge
        Level::Full => (width - 21.0 * s - row, -14.0 * s),
        _ => (text_left + text_w + m.gap, (h - m.badge) * 0.5),
    };
    for r in rects.iter_mut().take(badges) {
        *r = Rect::from_min_size(Pos2::new(x, top), Vec2::splat(m.badge));
        x += m.badge + m.badge_gap;
    }
    Layout {
        size: Vec2::new(width, h),
        photo,
        dot,
        name,
        subs,
        text_x: (text_left, text_left + text_w),
        badges: rects,
    }
}

/// Voice level of a speaking avatar (0 to 1), smoothed: fast attack, slower
/// release, and a gate that closes the halo when it stops speaking.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VoiceEnvelope {
    pub gate: f32,
    pub level: f32,
    /// 0..1: how far past the overdriven level the voice goes, smoothed
    /// like the level (`overdrive`).
    pub heat: f32,
}

const VOICE_ATTACK: f32 = 0.04;
const VOICE_RELEASE: f32 = 0.28;
const GATE_RELEASE: f32 = 0.22;
/// Raw level over the overdriven one at which a plate is fully red: 6 dB,
/// so that the color slides instead of switching.
const OVERDRIVE_SPAN: f32 = 0.03;
/// A participant level for a microphone at full scale: WebRTC's
/// `0.18 + 0.005 × 20 log10(rms)` with the RMS in 16-bit units
/// (LLWebRTCVoiceClient::predUpdateOwnVolume, aurora_voice::transmit).
const FULL_SCALE_LEVEL: f32 = 0.6315;
/// Loudness (dBFS of the RMS over the 300 ms the level is averaged on) at
/// which a plate starts turning red. The Legacy Viewer's 0.7 needs more
/// than full scale, and the -10 dBFS the classic tag uses
/// (`voice_tag::LEVEL_MAX`) asks a 300 ms average to stay near clipping:
/// a real shout, peaks near full scale with the gaps between syllables in
/// the average, reads -18 to -12 dBFS, normal speech at a sane microphone
/// gain -30 to -24. Red from -18 to -12 then marks a shout and leaves
/// normal speech green, with 6 dB to spare.
const SHOUT_DBFS: f32 = -18.0;

/// The participant level of a voice `dbfs` loud (RMS over 300 ms).
pub fn level_at_dbfs(dbfs: f32) -> f32 {
    FULL_SCALE_LEVEL + 0.005 * dbfs
}

/// The loudness (dBFS) a participant level stands for.
pub fn dbfs_of(raw: f32) -> f32 {
    (raw - FULL_SCALE_LEVEL) / 0.005
}

/// Raw participant level (what `aurora_voice` reports) to 0..1 over the
/// range a voice really covers, as the classic tag's outline.
pub fn voice_level(raw: f32) -> f32 {
    let (min, max) = (super::voice_tag::LEVEL_MIN, super::voice_tag::LEVEL_MAX);
    if raw.is_finite() {
        ((raw - min) / (max - min)).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// How far a raw participant level goes past the overdriven level (0..1):
/// 0 up to a shout (`SHOUT_DBFS`), 1 one `OVERDRIVE_SPAN` above. The
/// speaking indicator of the Legacy Viewer turns red above
/// LLVoiceClient::OVERDRIVEN_POWER_LEVEL (RED_THRESHOLD of
/// llvoicevisualizer.cpp, llspeakers.cpp), which WebRTC's levels never
/// reach: here it is where a real shout gets to.
pub fn overdrive(raw: f32) -> f32 {
    if raw.is_finite() {
        ((raw - level_at_dbfs(SHOUT_DBFS)) / OVERDRIVE_SPAN).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// AURORA_VOICE_LEVEL_LOG=1 (temporary): once a second, the loudest moment
/// of our own plate's voice in the log, to read a real voice's levels.
pub fn level_log() -> bool {
    static SET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SET.get_or_init(|| std::env::var_os("AURORA_VOICE_LEVEL_LOG").is_some())
}

/// The loudest moment of one plate's voice over a second, for
/// `level_log`.
#[derive(Debug, Default)]
struct LevelProbe {
    since: f64,
    raw: f32,
    level: f32,
    heat: f32,
    speaking: bool,
    in_voice: bool,
}

impl LevelProbe {
    fn take(&mut self, voice: Option<(f32, bool)>, envelope: &VoiceEnvelope, now: f64) {
        if let Some((raw, speaking)) = voice {
            self.in_voice = true;
            self.speaking |= speaking;
            if raw.is_finite() {
                self.raw = self.raw.max(raw);
            }
        }
        self.level = self.level.max(envelope.level);
        self.heat = self.heat.max(envelope.heat);
        if now - self.since < 1.0 {
            return;
        }
        if self.in_voice {
            let color = match self.heat {
                h if h <= 0.0 => "green".to_owned(),
                h if h < 0.5 => format!("green→amber {:.0}%", h * 200.0),
                h => format!("amber→red {:.0}%", (h - 0.5) * 200.0),
            };
            log::info!(
                "voice level (own plate, loudest over 1 s): raw {:.3} ({:.1} dBFS, mic meter {:.2}), speaking {}, smoothed {:.2}, red from {:.3} ({SHOUT_DBFS} dBFS), heat {:.2}: {color}",
                self.raw,
                dbfs_of(self.raw),
                0.8 + dbfs_of(self.raw) / 100.0,
                self.speaking,
                self.level,
                level_at_dbfs(SHOUT_DBFS),
                self.heat,
            );
        } else {
            log::info!("voice level (own plate): not in the voice channel");
        }
        *self = LevelProbe {
            since: now,
            ..LevelProbe::default()
        };
    }
}

impl VoiceEnvelope {
    /// One frame later (`dt` seconds), from the raw participant level.
    pub fn update_raw(&mut self, raw: f32, speaking: bool, dt: f32) {
        let target = if speaking { overdrive(raw) } else { 0.0 };
        let tau = if target > self.heat { VOICE_ATTACK } else { VOICE_RELEASE };
        self.heat += (target - self.heat) * (1.0 - (-dt / tau).exp());
        self.update(voice_level(raw), speaking, dt);
    }

    /// One frame later (`dt` seconds): `level` is 0..1 (`voice_level`).
    pub fn update(&mut self, level: f32, speaking: bool, dt: f32) {
        let toward = |value: f32, target: f32, tau: f32| value + (target - value) * (1.0 - (-dt / tau).exp());
        let target = if speaking { level.clamp(0.0, 1.0) } else { 0.0 };
        let tau = if target > self.level { VOICE_ATTACK } else { VOICE_RELEASE };
        self.level = toward(self.level, target, tau);
        self.gate = toward(
            self.gate,
            if speaking { 1.0 } else { 0.0 },
            if speaking { VOICE_ATTACK } else { GATE_RELEASE },
        );
        if self.gate < 0.01 && !speaking {
            *self = VoiceEnvelope::default();
        }
    }

    /// The halo to draw now; None when silent.
    pub fn halo(&self) -> Option<Halo> {
        (self.gate > 0.0).then_some(Halo {
            level: self.level,
            gate: self.gate,
            heat: self.heat,
        })
    }
}

/// How loud a speaking plate shines: the smoothed level, the gate and the
/// overdrive, all 0..1. `paint::voice_light` turns it into lights for the
/// plate's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Halo {
    pub level: f32,
    /// 0..1: how far the plate has lit up (the outline turned green).
    pub gate: f32,
    /// 0..1: how far past the overdriven level (`overdrive`), red at 1.
    pub heat: f32,
}

/// One of the three typing dots at `time` seconds: (opacity, lift in mockup
/// pixels). The mockup's `dot` keyframes: a dot rises and lights up in the
/// first 30 % of its 1.1 s, comes back by 60 %, then rests; each dot starts
/// 0.18 s after the previous one.
pub fn typing_dot(time: f64, index: usize) -> (f32, f32) {
    let phase = ((time - index as f64 * 0.18) / 1.1).rem_euclid(1.0) as f32;
    let up = if phase < 0.3 {
        phase / 0.3
    } else if phase < 0.6 {
        1.0 - (phase - 0.3) / 0.3
    } else {
        0.0
    };
    // ease in and out, as the CSS default
    let up = up * up * (3.0 - 2.0 * up);
    (0.3 + 0.7 * up, -2.5 * up)
}

/// The profile picture of a plate.
#[derive(Clone, Copy)]
pub enum Picture<'a> {
    Loaded(&'a egui::TextureHandle),
    /// Not there yet: asked for when the plate is complete.
    Wanted,
    /// The resident has none (or we do not ask): the placeholder stays.
    None,
}

/// A name plate to draw this frame.
pub struct Tag<'a> {
    pub id: Uuid,
    /// Bottom center of the plate, in points.
    pub anchor: Pos2,
    /// Where the plate stands in the world: the scene hides what it has in
    /// front of this point (`ui::hud::tag_point`).
    pub at: glam::Vec3,
    /// Distance to the camera.
    pub distance: f32,
    /// Opacity of the whole plate (the distance fade of the name tags).
    pub fade: f32,
    pub content: Content,
    /// (raw level, speaking) while in the voice channel.
    pub voice: Option<(f32, bool)>,
    /// Still loading: progress 0..1.
    pub loading: Option<f32>,
    pub picture: Picture<'a>,
}

/// What a plate keeps of its avatar between two frames.
struct Entry {
    level: Level,
    /// Level it is leaving, while `changed` is recent.
    from: Level,
    changed: f64,
    voice: VoiceEnvelope,
    /// 1 when the plate blurs the world, 0 when it only tints it.
    glass: f32,
    /// 1 while the pointer is on it (lighter surface and border), eased.
    hover: f32,
    hovered: bool,
    /// When its last bounce started.
    bounced: f64,
    /// Its jelly (Preferences › Just for fun), and its click combo.
    wobble: fidget::Wobble,
    combo: combo::Combo,
    picture_asked: bool,
    seen: f64,
    /// Its geometry, kept while its state does not change.
    cache: paint::Cache,
}

/// At most `PICTURES_PER_SECOND` new pictures a second.
#[derive(Debug, Default)]
pub struct PictureBudget {
    window: f64,
    used: u32,
}

impl PictureBudget {
    /// May one more picture be asked for at `now`?
    pub fn take(&mut self, now: f64) -> bool {
        if now - self.window >= 1.0 || now < self.window {
            self.window = now;
            self.used = 0;
        }
        let ok = self.used < PICTURES_PER_SECOND;
        if ok {
            self.used += 1;
        }
        ok
    }
}

/// The name plates of the avatars on screen.
#[derive(Default)]
pub struct Plates {
    entries: HashMap<Uuid, Entry>,
    budget: PictureBudget,
    /// Avatars whose picture a complete plate is waiting for (drained by
    /// the viewer, which asks for the profile and the texture).
    pub wanted_pics: HashSet<Uuid>,
    /// Kept from frame to frame: the level of each plate, which ones blur
    /// the world, and the complete ones still without their picture.
    levels: Vec<Level>,
    glass: Vec<bool>,
    waiting: Vec<Uuid>,
    /// Preferences › Aurora › "Full plates for friends only".
    pub friends_only_full: bool,
    /// Preferences › Aurora › "Expand a plate on hover".
    pub expand_on_hover: bool,
    /// The avatar whose plate is under the pointer this frame (set by the
    /// viewer from the plates of the last frame, outside the windows).
    pub hovered: Option<Uuid>,
    /// The last plate under the pointer, and when: it lingers a moment.
    hover_seen: Option<(Uuid, f64)>,
    /// Preferences › "Reduce animations": a hovered plate lights up without
    /// bouncing.
    pub still: bool,
    /// Preferences › Just for fun › "Jiggly name plates" (`fidget.rs`).
    pub jiggle: bool,
    /// Preferences › Just for fun › "Click combo counter" (`combo.rs`), and
    /// the clicks counted for each resident.
    pub click_combo: bool,
    pub tally: combo::Tally,
    /// The plate under the pointer last frame, without the linger: each
    /// time the pointer reaches a jiggly plate it wobbles.
    pointer_was: Option<Uuid>,
    /// The plate whose voice goes to the log (`level_log`): our own.
    pub probe: Option<Uuid>,
    level_probe: LevelProbe,
}

/// Whether a plate going from `from` to `to` grows.
fn grows(from: Level, to: Level) -> bool {
    let rank = |level| match level {
        Level::Far => 0,
        Level::Compact => 1,
        Level::Full => 2,
    };
    rank(to) > rank(from)
}

impl Plates {
    /// A click on `id`'s plate at `now`, for the click combo: counted,
    /// and the plate pressed in. None when it has no plate on screen.
    pub fn click(&mut self, id: Uuid, now: f64) -> Option<combo::Hit> {
        let entry = self.entries.get_mut(&id)?;
        let total = self.tally.add(id, now);
        if !self.still {
            entry.wobble.press();
        }
        Some(entry.combo.click(now, total, self.still))
    }

    /// Draw `tags` (far first, the last one on top) and report the rectangle
    /// of each plate, which takes the clicks like its avatar.
    pub fn paint(
        &mut self,
        painter: &egui::Painter,
        p: &Palette,
        icons: &super::icons::Icons,
        tags: &[Tag<'_>],
        now: f64,
        mut placed: impl FnMut(Uuid, Rect, glam::Vec3),
    ) {
        let env = paint::Env::new(painter, p, icons, now);
        // nearest first (`tags` ends with them): the level of each plate,
        // the complete ones limited in number, and the blur for the first ones
        let mut glass_left = if p.panel_alpha < 255 { MAX_GLASS } else { 0 };
        let mut glass = std::mem::take(&mut self.glass);
        glass.clear();
        glass.resize(tags.len(), false);
        let mut levels = std::mem::take(&mut self.levels);
        levels.clear();
        levels.resize(tags.len(), Level::Far);
        if let Some(id) = self.hovered {
            self.hover_seen = Some((id, now));
        }
        let hover = self.hover_seen.filter(|&(_, seen)| hover_holds(seen, now)).map(|(id, _)| id);
        // the plates the pointer has just reached and left, for the jelly
        let jiggle = self.jiggle && !self.still;
        let (entered, left) = if self.hovered != self.pointer_was {
            (self.hovered, self.pointer_was)
        } else {
            (None, None)
        };
        self.pointer_was = self.hovered;
        let mut rank = 0;
        for (i, tag) in tags.iter().enumerate().rev() {
            let previous = self.entries.get(&tag.id).map(|e| e.level);
            let mut level = preferred_level(level_for(tag.distance, previous), &tag.content, self.friends_only_full);
            if level == Level::Full {
                level = crowd_level(level, rank, previous == Some(Level::Full));
                rank += 1;
            }
            level = hover_level(level, hover == Some(tag.id), self.expand_on_hover);
            levels[i] = level;
            if level != Level::Far && glass_left > 0 {
                glass_left -= 1;
                glass[i] = true;
            }
        }
        let mut waiting = std::mem::take(&mut self.waiting);
        waiting.clear();
        for ((tag, glass), level) in tags.iter().zip(glass.iter().copied()).zip(levels.iter().copied()) {
            let entry = self.entries.entry(tag.id).or_insert_with(|| Entry {
                level,
                from: level,
                changed: f64::NEG_INFINITY,
                voice: VoiceEnvelope::default(),
                glass: if glass { 1.0 } else { 0.0 },
                hover: 0.0,
                hovered: false,
                bounced: f64::NEG_INFINITY,
                wobble: fidget::Wobble::default(),
                combo: combo::Combo::default(),
                picture_asked: false,
                seen: now,
                cache: paint::Cache::default(),
            });
            let dt = (now - entry.seen).clamp(0.0, 0.25) as f32;
            entry.seen = now;
            if level != entry.level {
                if jiggle && grows(entry.level, level) {
                    entry.wobble.expand();
                }
                entry.from = entry.level;
                entry.level = level;
                entry.changed = now;
            }
            if jiggle && entered == Some(tag.id) {
                entry.wobble.enter();
            } else if jiggle && left == Some(tag.id) {
                entry.wobble.leave();
            }
            if self.still {
                entry.wobble = fidget::Wobble::default();
            }
            entry.wobble.step(dt);
            entry.combo.step(dt);
            match tag.voice {
                Some((raw, speaking)) => entry.voice.update_raw(raw, speaking, dt),
                None => entry.voice = VoiceEnvelope::default(),
            }
            if self.probe == Some(tag.id) {
                self.level_probe.take(tag.voice, &entry.voice, now);
            }
            // the blur comes and goes over the level's ease, not in one frame
            let step = dt / LEVEL_EASE as f32;
            entry.glass = if glass {
                (entry.glass + step).min(1.0)
            } else {
                (entry.glass - step).max(0.0)
            };
            let hovered = hover == Some(tag.id);
            // a jiggly plate has its jelly instead of the bounce
            if hovered && !entry.hovered && !self.still && !jiggle {
                entry.bounced = now;
            }
            entry.hovered = hovered;
            let step = dt / HOVER_EASE as f32;
            entry.hover = if hovered {
                (entry.hover + step).min(1.0)
            } else {
                (entry.hover - step).max(0.0)
            };
            // a jiggly plate that grows goes past its size and comes back
            let ease = if jiggle { fidget::EASE } else { LEVEL_EASE };
            let progress = ((now - entry.changed) / ease).clamp(0.0, 1.0) as f32;
            let smooth = progress * progress * (3.0 - 2.0 * progress);
            let grow = if self.still {
                Vec2::ZERO
            } else {
                Vec2::splat(bounce_grow(now - entry.bounced)) + entry.wobble.grow()
            };
            let state = paint::State {
                level: entry.level,
                from: (progress < 1.0).then_some(entry.from),
                progress: smooth,
                size_progress: if jiggle && grows(entry.from, entry.level) {
                    fidget::overshoot(progress)
                } else {
                    smooth
                },
                halo: entry.voice.halo(),
                glass: entry.glass,
                hover: entry.hover * entry.hover * (3.0 - 2.0 * entry.hover),
                grow,
                still: self.still,
            };
            aurora_render::world_label_at(painter.ctx(), Some(tag.at));
            let (rect, surface) = paint::plate(&env, tag, &state, &mut entry.cache);
            placed(tag.id, rect, tag.at);
            if self.click_combo && entry.combo.busy(now) {
                combo::paint(&env, surface, &entry.combo, self.tally.get(tag.id), tag.fade, self.still);
            }
            if entry.level == Level::Full && matches!(tag.picture, Picture::Wanted) {
                waiting.push(tag.id);
            }
        }
        // nearest first, a few new ones a second
        for id in waiting.iter().rev() {
            let Some(entry) = self.entries.get_mut(id) else { continue };
            if !entry.picture_asked && self.budget.take(now) {
                entry.picture_asked = true;
            }
            if entry.picture_asked {
                self.wanted_pics.insert(*id);
            }
        }
        (self.levels, self.glass, self.waiting) = (levels, glass, waiting);
        // avatars that left the screen
        self.entries.retain(|_, e| now - e.seen < 5.0);
        self.tally.save_when_idle(now);
    }
}
