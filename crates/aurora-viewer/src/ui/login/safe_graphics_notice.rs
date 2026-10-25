//! The login screen's card after the graphics card stopped answering (or
//! was lost, or the window froze) in the last session (`app/safe_graphics.rs`):
//! the safe graphics offered, then, once chosen, the way back to the
//! user's own settings. Same caution card as `driver_notice.rs`, above it.
use crate::theme::Palette;
use crate::ui::{
    icons::Icons,
    widgets::{self, ActionKind},
};
use egui::{RichText, Sense, Vec2};

/// What the card says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeGraphics {
    /// The last session ended with the GPU in trouble: offer the safe graphics.
    Offer,
    /// The safe graphics are on, the user's own settings kept aside.
    Active,
}

/// What the user chose on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Offer: switch to the safe graphics.
    Use,
    /// Offer: keep the graphics as they are.
    Keep,
    /// Active: back to the settings kept aside.
    Restore,
    /// Active: stay on the safe graphics (the settings kept aside are dropped).
    Stay,
}

pub fn card(ui: &mut egui::Ui, p: &Palette, icons: &Icons, state: SafeGraphics, width: f32) -> Option<Choice> {
    let mut choice = None;
    super::card_caution(ui, p, 16, |ui| {
        ui.set_width(width - 32.0);
        let (title, detail) = match state {
            SafeGraphics::Offer => (
                aurora_i18n::tr!("login-safe-graphics-offer-title"),
                aurora_i18n::tr!("login-safe-graphics-offer"),
            ),
            SafeGraphics::Active => (
                aurora_i18n::tr!("login-safe-graphics-active-title"),
                aurora_i18n::tr!("login-safe-graphics-active"),
            ),
        };
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            if let Some(icon) = icons.get("alert-triangle-filled") {
                egui::Image::new(icon).tint(p.amber).paint_at(ui, r);
            }
            ui.add(egui::Label::new(RichText::new(title).size(15.0).strong().color(p.ink)).wrap());
        });
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(detail).size(12.5).color(p.ink)).wrap());
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let size = Vec2::new(0.0, 30.0);
            match state {
                SafeGraphics::Offer => {
                    let use_it = aurora_i18n::tr!("login-safe-graphics-use");
                    if widgets::action_button(ui, p, "shield-filled", use_it, ActionKind::Caution, size).clicked() {
                        choice = Some(Choice::Use);
                    }
                    let keep = aurora_i18n::tr!("login-safe-graphics-keep");
                    if widgets::action_button(ui, p, "x", keep, ActionKind::Flat, size).clicked() {
                        choice = Some(Choice::Keep);
                    }
                }
                SafeGraphics::Active => {
                    let restore = aurora_i18n::tr!("login-safe-graphics-restore");
                    if widgets::action_button(ui, p, "arrow-back-up", restore, ActionKind::Caution, size).clicked() {
                        choice = Some(Choice::Restore);
                    }
                    let stay = aurora_i18n::tr!("login-safe-graphics-stay");
                    if widgets::action_button(ui, p, "x", stay, ActionKind::Flat, size).clicked() {
                        choice = Some(Choice::Stay);
                    }
                }
            }
        });
    });
    choice
}
