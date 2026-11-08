//! The Poke button and the poke card's jiggle (`world/pokes.rs`; Aurora's
//! own). The button shows in People › Friends, a conversation's head and a
//! profile, the same everywhere: hidden while pokes are off or a poke may
//! never go (blocked, RLVa), greyed out with the reason while the resident
//! is offline or a poke to them just went. The card wobbles when it comes
//! out and at each new poke, with the springs of the jiggly name plates
//! (`nameplate/fidget.rs`), and stays still with Reduce animations. A poke
//! that goes out jabs: a finger shoots from where it was clicked and a ring
//! pops with sparks where it lands (`jab`, painted by `paint_jabs`).

use crate::theme::Palette;
use crate::world::World;
use crate::world::pokes::Refusal;
use egui::emath::TSTransform;
use std::time::Instant;
use uuid::Uuid;

use super::nameplate::fidget::Spring;

/// The icon of a poke, everywhere (Tabler has no filled one).
pub const ICON: &str = "hand-finger-right";

/// What a Poke button shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub enabled: bool,
    /// Its tooltip: "Poke", or why it is greyed out.
    pub tip: &'static str,
}

/// The Poke button for `to`, or `None` when it is hidden. While a poke to
/// them cools down, the frame is asked again when it ends.
pub fn button(ctx: &egui::Context, world: &World, to: &Uuid) -> Option<Button> {
    let now = Instant::now();
    let refusal = world.poke_refusal(to, now);
    if refusal == Some(Refusal::Cooling) {
        ctx.request_repaint_after(world.pokes.cooldown_left(to, now));
    }
    state(refusal)
}

/// The button for a refusal (or none).
fn state(refusal: Option<Refusal>) -> Option<Button> {
    match refusal {
        None => Some(Button {
            enabled: true,
            tip: aurora_i18n::tr!("ui-poke"),
        }),
        Some(r) if r.hides() => None,
        Some(Refusal::Cooling) => Some(Button {
            enabled: false,
            tip: aurora_i18n::tr!("ui-poke-cooling"),
        }),
        Some(_) => Some(Button {
            enabled: false,
            tip: aurora_i18n::tr!("ui-poke-offline"),
        }),
    }
}

/// Angular frequency and damping of the springs: the plates' feel, a
/// little looser so that a card shakes a few times.
const OMEGA: f32 = 26.0;
const ZETA: f32 = 0.16;
/// Speeds (points per second) a poke gives: the card swells and shakes
/// sideways.
const POP_KICK: f32 = 120.0;
const SHAKE_KICK: f32 = 170.0;
const MAX_SPEED: f32 = 300.0;

/// The motion of one resident's card.
#[derive(Debug, Clone, Copy, Default)]
struct Jiggle {
    /// The card it was kicked for: a new poke is a new card.
    card: u64,
    pop: Spring,
    shake: Spring,
    /// Time of the last frame, and the card's place then (it scales about
    /// its middle).
    at: f64,
    rect: Option<egui::Rect>,
}

impl Jiggle {
    fn advance(&mut self, card: u64, now: f64, still: bool) {
        let dt = if self.at > 0.0 {
            (now - self.at).clamp(0.0, 0.1) as f32
        } else {
            0.0
        };
        self.at = now;
        if self.card != card {
            self.card = card;
            self.pop.pump(POP_KICK, MAX_SPEED);
            self.shake.pump(SHAKE_KICK, MAX_SPEED);
        }
        if still {
            self.pop = Spring::default();
            self.shake = Spring::default();
            return;
        }
        self.pop.step(dt, OMEGA, ZETA);
        // a little slower: the two swings do not stay in step
        self.shake.step(dt, OMEGA * 0.8, ZETA);
    }

    fn moving(&self) -> bool {
        !(self.pop.still() && self.shake.still())
    }

    /// The card grown by `pop` points on each side about its middle, moved
    /// `shake` points sideways.
    fn transform(&self) -> TSTransform {
        let Some(rect) = self.rect.filter(|r| r.width() > 1.0) else {
            return TSTransform::IDENTITY;
        };
        let scale = 1.0 + 2.0 * self.pop.x / rect.width();
        let c = rect.center().to_vec2();
        TSTransform::from_translation(c + egui::vec2(self.shake.x, 0.0))
            * TSTransform::from_scaling(scale)
            * TSTransform::from_translation(-c)
    }
}

/// Draw the poke card `card` of `from` with its jiggle: kicked when a new
/// card comes (its first frame or a new poke), still with `still` (Reduce
/// animations). Only what is drawn moves; the clicks keep their place.
pub fn jiggle<R>(ui: &mut egui::Ui, from: Uuid, card: u64, still: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let id = egui::Id::unique(("poke-jiggle", from));
    let now = ui.input(|i| i.time);
    let mut j: Jiggle = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    j.advance(card, now, still);
    let shown = ui.with_visual_transform(j.transform(), body);
    j.rect = Some(shown.response.rect);
    if j.moving() {
        ui.ctx().request_repaint();
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, j));
    shown.inner
}

/// Seconds of a jab, and when the finger lands (its part of them).
const JAB: f64 = 0.7;
const LAND: f32 = 0.3;
/// Where the finger starts and lands, from the click (points): straight
/// out to the right, clear of the pointer, so that the ring stays on the
/// row or the bar that was clicked.
const FROM: egui::Vec2 = egui::vec2(12.0, 0.0);
const TO: egui::Vec2 = egui::vec2(38.0, 0.0);
const FINGER: f32 = 18.0;
/// The ring's last radius and the sparks around it.
const RING: f32 = 16.0;
const SPARKS: usize = 6;

/// A poke that went, clicked at `at` (time `start`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Jab {
    at: egui::Pos2,
    start: f64,
}

fn jabs_id() -> egui::Id {
    egui::Id::unique("poke-jabs")
}

/// A poke went from a click at `at`: its jab plays there.
pub fn jab(ctx: &egui::Context, at: egui::Pos2) {
    let start = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<Jab>>(jabs_id()).push(Jab { at, start }));
    ctx.request_repaint();
}

/// What a jab shows `u` (0 to 1) of the way: the finger's place, size and
/// opacity, then the ring's radius and the sparks' reach with their
/// opacity once the finger has landed.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Frame {
    finger: egui::Pos2,
    size: f32,
    finger_alpha: f32,
    tip: egui::Pos2,
    ring: Option<(f32, f32)>,
}

fn frame(at: egui::Pos2, u: f32) -> Frame {
    let u = u.clamp(0.0, 1.0);
    // out with an overshoot, then it stays while it fades
    let go = super::nameplate::fidget::overshoot(u / LAND);
    let finger = at + FROM + (TO - FROM) * go;
    // a punch in size at the landing
    let size = FINGER * (1.0 + 0.35 * (1.0 - ((u - LAND).abs() / 0.15).min(1.0)));
    let finger_alpha = 1.0 - ((u - 0.55) / 0.45).clamp(0.0, 1.0);
    let tip = finger + egui::vec2(FINGER * 0.5, -FINGER * 0.15);
    let ring = (u >= LAND).then(|| {
        let r = ((u - LAND) / (1.0 - LAND)).clamp(0.0, 1.0);
        // fast out, slow at the end
        let grow = 1.0 - (1.0 - r) * (1.0 - r);
        (3.0 + (RING - 3.0) * grow, 1.0 - r)
    });
    Frame {
        finger,
        size,
        finger_alpha,
        tip,
        ring,
    }
}

/// Paint the jabs in play over the whole interface, and forget the ones
/// done. `still` (Reduce animations): none plays.
pub fn paint_jabs(ctx: &egui::Context, p: &Palette, icons: &super::icons::Icons, still: bool) {
    let now = ctx.input(|i| i.time);
    let mut jabs: Vec<Jab> = ctx.data_mut(|d| d.get_temp(jabs_id())).unwrap_or_default();
    jabs.retain(|j| !still && now - j.start < JAB);
    if jabs.is_empty() {
        ctx.data_mut(|d| d.remove::<Vec<Jab>>(jabs_id()));
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("poke-jabs-layer")));
    for j in &jabs {
        let f = frame(j.at, ((now - j.start) / JAB) as f32);
        if let Some((radius, alpha)) = f.ring {
            painter.circle_stroke(
                f.tip,
                radius,
                egui::Stroke::new(2.0 * alpha + 0.5, p.violet_light.gamma_multiply(alpha)),
            );
            for k in 0..SPARKS {
                let angle = k as f32 / SPARKS as f32 * std::f32::consts::TAU + 0.3;
                let dir = egui::vec2(angle.cos(), angle.sin());
                let a = f.tip + dir * (radius + 2.0);
                let b = f.tip + dir * (radius + 2.0 + 5.0 * alpha);
                painter.line_segment([a, b], egui::Stroke::new(1.5, p.amber.gamma_multiply(alpha)));
            }
        }
        if let Some(t) = icons.get(ICON) {
            painter.image(
                t.id(),
                egui::Rect::from_center_size(f.finger, egui::Vec2::splat(f.size)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                p.violet_light.gamma_multiply(f.finger_alpha),
            );
        }
    }
    ctx.data_mut(|d| d.insert_temp(jabs_id(), jabs));
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card laid out once, 300 by 60 points.
    fn card() -> Jiggle {
        Jiggle {
            rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 60.0))),
            ..Default::default()
        }
    }

    #[test]
    fn the_button_follows_the_refusal() {
        assert_eq!(state(None).map(|b| b.enabled), Some(true));
        assert_eq!(state(Some(Refusal::Off)), None);
        assert_eq!(state(Some(Refusal::Blocked)), None);
        assert_eq!(state(Some(Refusal::Restricted)), None);
        assert_eq!(state(Some(Refusal::NotAResident)), None);
        let offline = state(Some(Refusal::Offline)).expect("shown");
        assert!(!offline.enabled);
        assert_eq!(offline.tip, aurora_i18n::tr!("ui-poke-offline"));
        let cooling = state(Some(Refusal::Cooling)).expect("shown");
        assert!(!cooling.enabled);
        assert_eq!(cooling.tip, aurora_i18n::tr!("ui-poke-cooling"));
    }

    #[test]
    fn a_new_card_jiggles_then_settles() {
        let mut j = card();
        j.advance(1, 1.0, false);
        assert!(j.moving(), "kicked by its first frame");
        let mut t = 1.0;
        let mut moved = false;
        for _ in 0..10 {
            t += 1.0 / 60.0;
            j.advance(1, t, false);
            moved |= j.transform() != TSTransform::IDENTITY;
        }
        assert!(moved, "it visibly moves");
        for _ in 0..600 {
            t += 1.0 / 60.0;
            j.advance(1, t, false);
        }
        assert!(!j.moving(), "settled after a few seconds");
        assert_eq!(j.transform(), TSTransform::IDENTITY);
        // the next poke is a new card: kicked again
        j.advance(2, t + 0.1, false);
        assert!(j.moving());
    }

    #[test]
    fn a_jab_shoots_out_lands_with_a_ring_and_fades() {
        let at = egui::pos2(100.0, 100.0);
        // its ring stays on the clicked row (rows are 42 points high)
        for u in [0.0, 0.3, 0.6, 1.0] {
            assert!((frame(at, u).tip.y - at.y).abs() < 5.0, "{u}");
        }
        let start = frame(at, 0.0);
        assert_eq!(start.finger, at + FROM);
        assert!(start.ring.is_none(), "no ring before the landing");
        let landed = frame(at, LAND);
        assert!((landed.finger - (at + TO)).length() < 0.01, "it lands at its end");
        assert!(landed.size > FINGER, "a punch at the landing");
        let (r0, a0) = landed.ring.expect("the ring pops at the landing");
        let (r1, a1) = frame(at, 0.8).ring.expect("still there");
        assert!(r1 > r0 && a1 < a0, "it grows and fades");
        let end = frame(at, 1.0);
        assert_eq!(end.finger_alpha, 0.0);
        assert_eq!(end.ring.map(|(_, a)| a), Some(0.0));
    }

    #[test]
    fn reduce_animations_keeps_the_card_still() {
        let mut j = card();
        j.advance(1, 1.0, true);
        assert!(!j.moving());
        j.advance(2, 1.5, true);
        assert_eq!(j.transform(), TSTransform::IDENTITY);
    }
}
