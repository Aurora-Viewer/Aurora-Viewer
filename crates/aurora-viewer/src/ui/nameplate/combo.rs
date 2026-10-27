//! "Click combo counter" (Preferences › Just for fun, off by default,
//! #1114): every click on a plate counts, and the count of each resident
//! is kept on this computer, per account (`plate-clicks.json` in the
//! account's folder; nothing is sent). Five clicks within five seconds
//! start a combo: a counter pops out of the plate's bottom right corner
//! with the resident's total, in the Aurora team's northern light
//! (`aurora.rs`): a glass pill whose border flows in the borealis colours
//! over a halo, its figures green to violet with a shine running along
//! them. While the clicks keep coming each one punches it and sends a "+1"
//! up, and the streak heats it: the colours flow faster, the halo widens
//! and curtains of light rise behind it. A streak of 10, 25, 50, 100 (then
//! every hundred) or a round hundred clicks in all bursts into rings and
//! sparks, the curtains flaring, the streak floating up. A pause of `HOLD`
//! seconds ends the combo and the counter fades out. Aurora's own: nothing
//! of the kind in the Legacy Viewer.

use super::aurora;
use super::fidget::Spring;
use super::mesh::Outline;
use super::paint::Env;
use egui::epaint::TextShape;
use egui::text::LayoutJob;
use egui::{Mesh, Pos2, Rect, Shape, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Clicks that start a combo, within `WINDOW` seconds.
pub const START: usize = 5;
pub const WINDOW: f64 = 5.0;
/// Seconds without a click that end a combo; the counter fades out over
/// the last `FADE` of them.
pub const HOLD: f64 = 2.5;
const FADE: f64 = 0.5;
/// Seconds a "+1" takes to rise and fade, and how many fly at once.
const POP: f64 = 0.6;
const POPS: usize = 8;
/// Seconds of a milestone's burst.
const BURST: f64 = 0.9;
const SPARKS: usize = 14;
/// The counter's punch: its spring (rad/s, damping) and the scale a click
/// adds, per second.
const PUNCH_OMEGA: f32 = 32.0;
const PUNCH_ZETA: f32 = 0.28;
const PUNCH_KICK: f32 = 7.0;
/// Where the counter starts when it appears (a scale of about 0.6): it
/// springs out past its size.
const ENTRANCE: f32 = -0.9;
/// The streak at which the counter is at its hottest.
const HOTTEST: f32 = 60.0;
/// Opacity of the counter's glass, as the badges of the plate.
const GLASS: f32 = 0.92;

/// A streak worth a burst.
pub fn milestone(streak: u32) -> bool {
    matches!(streak, 10 | 25 | 50) || (streak >= 100 && streak.is_multiple_of(100))
}

/// What a click did to the combo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Counted, no combo yet (or the counter already out).
    Counted,
    /// The counter has just appeared.
    Started,
    /// A streak or a total worth a burst.
    Milestone,
}

/// The combo of one plate.
#[derive(Debug, Clone, Copy)]
pub struct Combo {
    /// The last `START` clicks of the run, oldest at `next`.
    times: [f64; START],
    next: usize,
    /// Clicks of the run: each within `HOLD` of the previous one.
    streak: u32,
    last: f64,
    /// The counter is out (`START` clicks within `WINDOW`).
    shown: bool,
    punch: Spring,
    /// When each of the last "+1" started.
    pops: [f64; POPS],
    pop_next: usize,
    /// The last burst: when, and the streak it celebrates.
    burst: (f64, u32),
    /// Seconds of the light's motion: faster as the streak heats.
    flow: f32,
}

impl Default for Combo {
    fn default() -> Self {
        Combo {
            times: [f64::NEG_INFINITY; START],
            next: 0,
            streak: 0,
            last: f64::NEG_INFINITY,
            shown: false,
            punch: Spring::default(),
            pops: [f64::NEG_INFINITY; POPS],
            pop_next: 0,
            burst: (f64::NEG_INFINITY, 0),
            flow: 0.0,
        }
    }
}

impl Combo {
    /// A click at `now`, `total` the resident's count with it.
    pub fn click(&mut self, now: f64, total: u32, still: bool) -> Hit {
        if !self.alive(now) {
            *self = Combo {
                punch: self.punch,
                flow: self.flow,
                ..Combo::default()
            };
        }
        self.streak = self.streak.saturating_add(1);
        self.last = now;
        self.times[self.next] = now;
        self.next = (self.next + 1) % START;
        // the oldest of the last five is the next one to be replaced
        let started = !self.shown && now - self.times[self.next] <= WINDOW;
        self.shown |= started;
        if !self.shown {
            return Hit::Counted;
        }
        if started && !still {
            self.punch = Spring { x: ENTRANCE, v: 0.0 };
        } else if !still {
            self.punch.pump(PUNCH_KICK, PUNCH_KICK * 2.5);
        }
        if !still {
            self.pops[self.pop_next] = now;
            self.pop_next = (self.pop_next + 1) % POPS;
        }
        if milestone(self.streak) || (total > 0 && total.is_multiple_of(100)) {
            self.burst = (now, self.streak);
            Hit::Milestone
        } else if started {
            Hit::Started
        } else {
            Hit::Counted
        }
    }

    /// Whether the run goes on: the last click less than `HOLD` ago.
    pub fn alive(&self, now: f64) -> bool {
        now - self.last <= HOLD
    }

    #[cfg(test)]
    pub fn streak(&self) -> u32 {
        self.streak
    }

    /// Opacity of the counter: 1 during the combo, fading at its end.
    pub fn alpha(&self, now: f64) -> f32 {
        if !self.shown {
            return 0.0;
        }
        ((HOLD - (now - self.last)) / FADE).clamp(0.0, 1.0) as f32
    }

    /// Whether anything of the combo still moves or shows.
    pub fn busy(&self, now: f64) -> bool {
        self.alpha(now) > 0.0 || !self.punch.still() || now - self.burst.0 < BURST
    }

    pub fn step(&mut self, dt: f32) {
        self.punch.step(dt, PUNCH_OMEGA, PUNCH_ZETA);
        self.flow = (self.flow + dt * (1.0 + 3.0 * fervor(self.streak))).rem_euclid(3600.0);
    }
}

/// How hot a streak is, 0 when the counter appears to 1 at `HOTTEST`:
/// quick at first, then slower.
pub fn fervor(streak: u32) -> f32 {
    ((streak as f32 - START as f32) / (HOTTEST - START as f32)).clamp(0.0, 1.0).sqrt()
}

/// `text` in the semibold of the plates at `size`, each letter in the
/// borealis with the shine running along (`aurora::letter_color`), the
/// first one `lead` letters along the word.
fn borealis_text(env: &Env<'_>, text: &str, size: f32, lead: usize, t: f32) -> std::sync::Arc<egui::Galley> {
    let n = text.chars().count() + lead;
    let mut job = LayoutJob {
        break_on_newline: false,
        ..Default::default()
    };
    for (i, ch) in text.chars().enumerate() {
        let format = crate::ui::fonts::semibold_format(&env.semibold, size, aurora::letter_color(env.p, i + lead, n, t));
        job.append(ch.encode_utf8(&mut [0; 4]), 0.0, format);
    }
    env.painter.layout_job(job)
}

/// Draw the counter of a combo on the plate whose surface is `surface`.
pub(super) fn paint(env: &Env<'_>, surface: Rect, combo: &Combo, total: u32, fade: f32, still: bool) {
    let now = env.now;
    let alpha = combo.alpha(now) * fade;
    let (painter, p, ppp) = (env.painter, env.p, env.pixels_per_point);
    let t = if still { aurora::time(now, true) } else { combo.flow };
    let heat = fervor(combo.streak);
    // punched by the clicks, shrinking a little as it fades out
    let scale = (1.0 + 0.45 * (combo.punch.x / 0.45).tanh()) * (0.85 + 0.15 * alpha);
    let digits = crate::ui::hud::group_digits(total);
    let number = borealis_text(env, &digits, 11.5 * scale, 1, t);
    let icon = 11.0 * scale;
    let h = 18.0 * scale;
    let w = 7.0 * scale + icon + 3.0 * scale + number.size().x + 8.0 * scale;
    // out of the bottom right corner, clear of the plate's text: the
    // number grows to the right
    let center = Pos2::new(surface.right() - 12.0 * scale + w * 0.5, surface.bottom() - 2.0);
    let pill = Rect::from_center_size(center, Vec2::new(w, h));
    let (start, streak) = combo.burst;
    let burst = if still { 1.0 } else { ((now - start) / BURST) as f32 };
    let flare = if (0.0..1.0).contains(&burst) { 1.0 - burst } else { 0.0 };
    // a milestone: two rings of light and sparks, behind the counter
    if flare > 0.0 && !still {
        let out = 1.0 - (1.0 - burst).powi(3);
        let mut sparks = Mesh::default();
        for (ring, delay, color) in [(0.0, 0.0, p.borealis_at(0.1)), (1.0, 0.15, p.borealis_at(0.85))] {
            let k = ((burst - delay) / (1.0 - delay)).clamp(0.0, 1.0);
            if k <= 0.0 || k >= 1.0 {
                continue;
            }
            let reach = 1.0 - (1.0 - k).powi(3);
            let radius = h * 0.5 + (24.0 + 8.0 * ring) * reach;
            let line = aurora::light(color, fade * (1.0 - k), 0.6);
            Outline::circle(center, radius, ppp).stroke(&mut sparks, 2.6 * (1.0 - k), line);
        }
        for k in 0..SPARKS {
            // spread by the golden angle, each a little farther than the
            // last, falling a little as they fade
            let angle = k as f32 * 2.399_963 + streak as f32;
            let reach = 22.0 + 18.0 * ((k * 7) % SPARKS) as f32 / SPARKS as f32;
            let at = center + Vec2::angled(angle) * (h * 0.7 + reach * out) + Vec2::new(0.0, 6.0 * burst * burst);
            let color = p.borealis_at(k as f32 / (SPARKS - 1) as f32);
            Outline::circle(at, 4.5 * flare, ppp).dot(&mut sparks, aurora::light(color, 0.35 * fade * flare, 0.4));
            Outline::circle(at, 1.8 * flare + 0.4, ppp).dot(&mut sparks, aurora::light(color, fade * flare, 0.8));
        }
        painter.add(Shape::mesh(sparks));
    }
    if alpha > 0.0 {
        let edge = |s: f32| aurora::border_color(p, s, t);
        let mut light = Mesh::default();
        // the curtains rise behind it as the streak heats, and flare up
        let curtains = heat.max(flare);
        if curtains > 0.0 && !still {
            aurora::curtains(&mut light, p, pill, ppp, t, alpha * curtains);
        }
        aurora::border(&mut light, pill, ppp, alpha * (0.5 + 0.5 * curtains), 1.2, 0.8 + 1.6 * heat, edge);
        aurora::tag(&mut light, pill, h * 0.5, ppp, env.tint.gamma_multiply(GLASS), alpha, 1.2, edge);
        painter.add(Shape::mesh(light));
        if let Some(texture) = env.icon("click-filled") {
            let at = Pos2::new(pill.left() + 7.0 * scale + icon * 0.5, pill.center().y);
            let color = aurora::letter_color(p, 0, digits.chars().count() + 1, t);
            painter.image(
                texture.id(),
                Rect::from_center_size(at, Vec2::splat(icon)),
                super::paint::WHOLE,
                color.gamma_multiply(alpha),
            );
        }
        let at = Pos2::new(
            pill.right() - 8.0 * scale - number.size().x,
            pill.center().y - number.size().y * 0.5,
        );
        let mut text = TextShape::new(at, number, p.ink);
        text.opacity_factor = alpha;
        painter.add(text);
    }
    if still {
        return;
    }
    // the "+1" of the last clicks, rising from the counter, each a little
    // further along the borealis
    for (i, start) in combo.pops.iter().enumerate() {
        let k = ((now - start) / POP) as f32;
        if !(0.0..1.0).contains(&k) {
            continue;
        }
        let rise = 1.0 - (1.0 - k).powi(3);
        let drift = [4.0, 9.0, 1.0, 12.0, 6.0, 14.0, 3.0, 10.0][i % 8];
        let at = Pos2::new(pill.right() + drift * rise, pill.center().y - 6.0 - 20.0 * rise);
        let galley = borealis_text(env, "+1", 10.5 * (1.0 + 0.25 * (1.0 - rise)), i % 4, t);
        let mut text = TextShape::new(at - galley.size() * 0.5, galley, p.ink);
        text.opacity_factor = fade * (1.0 - k * k);
        painter.add(text);
    }
    // a milestone: the streak floating up
    if flare > 0.0 {
        let out = 1.0 - (1.0 - burst).powi(3);
        let galley = borealis_text(env, &format!("×{streak}"), 15.0 + 4.0 * flare, 0, t * 2.0);
        let at = Pos2::new(center.x - 4.0, pill.top() - 14.0 - 30.0 * out);
        let mut text = TextShape::new(at - galley.size() * 0.5, galley, p.ink);
        text.opacity_factor = fade * (1.0 - burst * burst);
        painter.add(text);
    }
}

/// Seconds after the last click before the counts are written: a combo is
/// written once, at its end.
const SAVE_AFTER: f64 = 1.5;
const FILE: &str = "plate-clicks.json";

/// The clicks given to each resident's plate, by our account.
#[derive(Default)]
pub struct Tally {
    owner: Option<Uuid>,
    counts: HashMap<Uuid, u32>,
    /// When the counts last changed, while they are not written.
    unsaved: Option<f64>,
}

/// Numbers the writes, and the last one written: a slow write never
/// replaces a newer one.
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WRITTEN: std::sync::Mutex<u64> = std::sync::Mutex::new(0);

impl Tally {
    /// The counts of `owner` (our account): read the first time, written
    /// first for the account before.
    pub fn open(&mut self, owner: Uuid) {
        if owner.is_nil() || self.owner == Some(owner) {
            return;
        }
        self.flush(false);
        self.counts = read(&path(owner));
        self.owner = Some(owner);
    }

    /// One more click on `id`'s plate at `now`: its total.
    pub fn add(&mut self, id: Uuid, now: f64) -> u32 {
        let count = self.counts.entry(id).or_insert(0);
        *count = count.saturating_add(1);
        self.unsaved = Some(now);
        *count
    }

    pub fn get(&self, id: Uuid) -> u32 {
        self.counts.get(&id).copied().unwrap_or(0)
    }

    /// Write the counts once the clicks have paused, on a thread.
    pub fn save_when_idle(&mut self, now: f64) {
        if self.unsaved.is_some_and(|at| now - at >= SAVE_AFTER) {
            self.flush(true);
        }
    }

    fn flush(&mut self, background: bool) {
        let (Some(owner), Some(_)) = (self.owner, self.unsaved.take()) else {
            return;
        };
        let bytes = match encode(&self.counts) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!("{FILE} not written: {e}");
                return;
            }
        };
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let write = move || write_newest(&path(owner), &bytes, generation);
        if background {
            std::thread::spawn(write);
        } else {
            write();
        }
    }
}

impl Drop for Tally {
    /// The clicks of the last seconds before quitting are kept too.
    fn drop(&mut self) {
        self.flush(false);
    }
}

fn path(owner: Uuid) -> PathBuf {
    crate::settings::account_dir(&owner).join(FILE)
}

pub(super) fn encode(counts: &HashMap<Uuid, u32>) -> serde_json::Result<Vec<u8>> {
    let map: std::collections::BTreeMap<String, u32> = counts.iter().map(|(id, n)| (id.to_string(), *n)).collect();
    serde_json::to_vec_pretty(&map)
}

pub(super) fn decode(bytes: &[u8]) -> serde_json::Result<HashMap<Uuid, u32>> {
    let map: HashMap<String, u32> = serde_json::from_slice(bytes)?;
    Ok(map
        .into_iter()
        .filter_map(|(id, n)| Some((Uuid::parse_str(&id).ok()?, n)))
        .collect())
}

fn read(path: &Path) -> HashMap<Uuid, u32> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    decode(&bytes).unwrap_or_else(|e| {
        log::warn!("{FILE} not read ({e}): the counts start again");
        HashMap::new()
    })
}

fn write_newest(path: &Path, bytes: &[u8], generation: u64) {
    let Ok(mut written) = WRITTEN.lock() else {
        return;
    };
    if generation <= *written {
        return;
    }
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| crate::atomic_file::write(path, bytes));
    match result {
        Ok(()) => *written = generation,
        Err(e) => log::warn!("{FILE} not written: {e}"),
    }
}
