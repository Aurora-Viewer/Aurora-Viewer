//! The Poke button and the poke card's jiggle (`world/pokes.rs`; Aurora's
//! own). The button shows in People › Friends, a conversation's head and a
//! profile, the same everywhere: hidden while pokes are off or a poke may
//! never go (blocked, RLVa), greyed out with the reason while the resident
//! is offline or a poke to them just went. The card wobbles when it comes
//! out and at each new poke, with the springs of the jiggly name plates
//! (`nameplate/fidget.rs`), and stays still with Reduce animations.

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
    fn reduce_animations_keeps_the_card_still() {
        let mut j = card();
        j.advance(1, 1.0, true);
        assert!(!j.moving());
        j.advance(2, 1.5, true);
        assert_eq!(j.transform(), TSTransform::IDENTITY);
    }
}
