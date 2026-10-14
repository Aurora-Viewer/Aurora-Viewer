//! The call of a conversation, under its head while it is in progress:
//! its state, who is in it and who speaks, the microphone and hanging up.
//! What Firestorm's IM window shows of a call (the call button turned into
//! "Leave call", the speakers' list with their voice indicators, FSFloaterIM
//! and LLParticipantList), in one strip.

use super::{ChatUi, ConvAction, tool};
use crate::theme::Palette;
use crate::ui::widgets::{ActionKind, action_button, avatar_disc};
use crate::world::World;
use crate::world::calls::{Call, CallState};
use egui::RichText;
use std::collections::HashMap;

/// Side of a participant's picture.
const MEMBER: f32 = 22.0;
/// Pictures shown before "+n".
const SHOWN: usize = 8;

/// "m:ss", or "h:mm:ss" past an hour.
pub(super) fn duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// The state of the call, as a sentence.
fn status(call: &Call) -> String {
    match call.state {
        CallState::Requesting => aurora_i18n::tr!("ui-call-connecting").to_owned(),
        CallState::Ringing if call.p2p() && !call.incoming => aurora_i18n::tr!("ui-call-calling").to_owned(),
        CallState::Ringing => aurora_i18n::tr!("ui-call-connecting").to_owned(),
        CallState::Connected => {
            let secs = call.connected_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            aurora_i18n::tr!("ui-call-connected", time = duration(secs))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn strip(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &crate::ui::icons::Icons,
    pics: &HashMap<uuid::Uuid, egui::TextureHandle>,
    world: &World,
    call: &Call,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
) {
    let connected = call.state == CallState::Connected;
    // the timer, and the pulse while it rings
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(if connected { 1000 } else { 50 }));
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let color = if connected {
                    p.success
                } else {
                    let t = ui.input(|i| i.time) as f32;
                    p.amber.gamma_multiply(0.55 + 0.45 * (t * 4.0).sin().abs())
                };
                crate::ui::widgets::glyph(ui, "phone-filled", color, 16.0);
                ui.label(RichText::new(status(call)).size(12.0).strong().color(p.ink));
                // the participants, a green ring around who speaks
                let others: Vec<_> = call.members.iter().filter(|m| m.id != world.agent_id).collect();
                if !others.is_empty() {
                    ui.add_space(4.0);
                    for m in others.iter().take(SHOWN) {
                        st.wanted_pics.insert(m.id);
                        let name = world.social.name_of(&m.id);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(MEMBER, MEMBER), egui::Sense::hover());
                        avatar_disc(ui, p, rect, pics.get(&m.id), &name, false);
                        if m.speaking {
                            ui.painter()
                                .circle_stroke(rect.center(), MEMBER * 0.5 + 1.5, egui::Stroke::new(2.0, p.success));
                        }
                        resp.on_hover_text(name);
                    }
                    if others.len() > SHOWN {
                        ui.label(RichText::new(format!("+{}", others.len() - SHOWN)).size(11.5).color(p.muted));
                    }
                    if !call.p2p() {
                        let n = others.len() + 1;
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-call-in-call", n = n.to_string()))
                                .size(11.0)
                                .color(p.muted),
                        );
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let hang_up = aurora_i18n::tr!("ui-call-hang-up");
                    if action_button(ui, p, "phone-off", hang_up, ActionKind::Danger, egui::vec2(0.0, 24.0)).clicked() {
                        actions.push(ConvAction::HangUp);
                    }
                    // the microphone, in toggle mode (held, it is the bar's button)
                    if let Some(on) = st.mic {
                        let (icon, tip) = if on {
                            ("microphone-filled", aurora_i18n::tr!("ui-call-mute"))
                        } else {
                            ("microphone-off", aurora_i18n::tr!("ui-call-unmute"))
                        };
                        if tool(ui, p, icons, icon, tip, true, on) {
                            actions.push(ConvAction::ToggleMic);
                        }
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_like_a_phone() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(75), "1:15");
        assert_eq!(duration(3725), "1:02:05");
    }
}
