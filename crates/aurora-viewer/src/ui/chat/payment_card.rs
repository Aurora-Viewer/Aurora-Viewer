//! The card of a direct payment in the conversation with the other resident
//! (#1198, `world/payments.rs`): who paid how much, ours included, then the
//! payer's message. The same frame as an inventory offer's (`offer_card.rs`), with
//! no buttons: there is nothing to answer.
//!
//! The same line with the legacy lines: the Legacy Viewer's sentence
//! (paid_you_ldollars_gift, you_paid_ldollars_gift,
//! indra/newview/skins/default/xui/en/strings.xml).

use super::invite_card::{card_frame, icon, notice};
use super::{LineEnv, chat_text_marked};
use crate::ui::fonts::chat_size;
use crate::world::ChatLine;
use crate::world::payments::{self, Payment};
use egui::RichText;

/// The icon of paying, the same as the Pay menu items'.
const ICON: &str = "coin-filled";

fn title(env: &LineEnv, line: &ChatLine, v: &Payment) -> String {
    let name = env.world.person_name(&v.other).unwrap_or_else(|| line.from.clone());
    payments::title(name, v.sent, v.amount)
}

/// The card, on the other resident's side after `indent`.
pub(super) fn draw_card(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, payment: u64, indent: f32) {
    let p = env.p;
    let Some(v) = env.world.payments.get(payment).cloned() else {
        // an old payment, dropped: its text only
        notice(ui, env, line);
        return;
    };
    let title = title(env, line, &v);
    card_frame(ui, env, line, indent, false, |ui, env| {
        ui.horizontal(|ui| {
            icon(ui, env, ICON, p.teal);
            ui.add(egui::Label::new(RichText::new(&title).size(chat_size(ui.ctx(), 13.0)).strong().color(p.ink)).wrap());
        });
        if !v.message.is_empty() {
            let world = env.world;
            chat_text_marked(
                ui,
                p,
                env.emoji,
                world,
                env.want_names,
                &v.message,
                chat_size(ui.ctx(), 12.5),
                p.ink,
                false,
                None,
            );
        }
    });
}

/// The legacy lines: the sentence on one line after the icon.
pub(super) fn draw_line(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, payment: u64) {
    let p = env.p;
    if env.world.payments.get(payment).is_none() {
        notice(ui, env, line);
        return;
    }
    ui.horizontal_wrapped(|ui| {
        icon(ui, env, ICON, p.teal);
        let world = env.world;
        chat_text_marked(
            ui,
            p,
            env.emoji,
            world,
            env.want_names,
            &line.text,
            chat_size(ui.ctx(), 12.5),
            p.ink,
            false,
            None,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::{Frames, Text};
    use crate::world::{ChatKind, World};
    use std::collections::{HashMap, HashSet};
    use uuid::Uuid;

    const JANE: Uuid = Uuid::from_u128(2);

    /// A world where Jane, whose conversation is open, paid us L$1,500.
    fn paid(message: &str) -> (World, ChatLine) {
        payment(false, message)
    }

    /// The same payment, or ours to Jane when `sent`.
    fn payment(sent: bool, message: &str) -> (World, ChatLine) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.social.names.insert(JANE, "Jane Doe".into());
        world.social.session_mut(JANE);
        let (source, dest) = if sent { (world.agent_id, JANE) } else { (JANE, world.agent_id) };
        world.payment(&aurora_net::NetEvent::MoneyTransaction {
            id: Uuid::from_u128(0x10),
            success: true,
            kind: 5001,
            source,
            source_group: false,
            dest,
            dest_group: false,
            amount: 1500,
            item: message.into(),
            description: String::new(),
        });
        let line = world.social.ims[0].lines.back().expect("the card").clone();
        (world, line)
    }

    fn frame(world: &World, line: &ChatLine, bubbles: bool) -> Vec<Text> {
        let mut frames = Frames::new();
        let mut icons = crate::ui::icons::Icons::load(&frames.ctx, None);
        let mut emoji = crate::ui::emoji::Emoji::load();
        let p = crate::theme::Theme::default().palette();
        let pics = HashMap::new();
        let (mut names, mut wanted) = (HashSet::new(), HashSet::new());
        let ChatKind::Payment(payment) = line.kind else {
            panic!("not a payment's card: {:?}", line.kind);
        };
        frames.run(Vec::new(), |ui| {
            let mut env = LineEnv {
                p: &p,
                icons: &mut icons,
                emoji: &mut emoji,
                pics: &pics,
                world,
                want_names: &mut names,
                wanted: &mut wanted,
                own_name: "Aurora Demo",
                plain_history: false,
                compact_automated: false,
                compact_notices: false,
                menu: None,
                bubbles: Some(false),
                previews: None,
            };
            if bubbles {
                draw_card(ui, &mut env, line, payment, 0.0);
            } else {
                draw_line(ui, &mut env, line, payment);
            }
        })
    }

    #[test]
    fn the_card_names_the_payer_the_amount_and_the_message() {
        let (world, line) = paid("Thanks for the set!");
        let texts: Vec<_> = frame(&world, &line, true).into_iter().map(|t| t.source).collect();
        let title = aurora_i18n::tr!("world-payments-paid-you", name = "Jane Doe", amount = payments::amount_label(1500));
        assert!(texts.contains(&title), "{texts:?}");
        assert!(texts.iter().any(|t| t.contains("Thanks for the set!")), "{texts:?}");
        // the simulator's "Payment" is no message
        let (world, line) = paid("Payment");
        let texts: Vec<_> = frame(&world, &line, true).into_iter().map(|t| t.source).collect();
        assert!(!texts.iter().any(|t| t.contains("Payment")), "{texts:?}");
    }

    #[test]
    fn our_card_names_the_payee() {
        let (world, line) = payment(true, "Rent");
        let texts: Vec<_> = frame(&world, &line, true).into_iter().map(|t| t.source).collect();
        let title = aurora_i18n::tr!("world-payments-you-paid", name = "Jane Doe", amount = payments::amount_label(1500));
        assert!(texts.contains(&title), "{texts:?}");
        assert!(texts.iter().any(|t| t.contains("Rent")), "{texts:?}");
    }

    #[test]
    fn the_legacy_line_says_the_whole_sentence() {
        let (world, line) = paid("Rent");
        let texts: Vec<_> = frame(&world, &line, false).into_iter().map(|t| t.source).collect();
        let all = texts.concat();
        assert!(
            all.contains("Jane Doe") && all.contains(&payments::amount_label(1500)) && all.contains("Rent"),
            "{texts:?}"
        );
    }
}
