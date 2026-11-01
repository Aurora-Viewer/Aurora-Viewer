//! The card of a resident's inventory offer in the conversation with the
//! giver, UserGiveItem's panel in an IM (LLIMToastNotifyPanel,
//! fschathistory.cpp): who gives what, its kind, and the buttons of its
//! entry in the notification centre, then its answer once given. A button
//! goes through `World::answer_inventory_offer`, which answers the entry:
//! whichever answers first closes the entry and leaves the card greyed.
//!
//! The same card as a teleport invitation's (`invite_card.rs`) with the
//! bubbles, the same line with the legacy lines.

use super::invite_card::{answered, card_frame, icon, notice};
use super::{LineEnv, chat_text_marked};
use crate::theme::Palette;
use crate::ui::context::{CtxAction, request};
use crate::ui::fonts::chat_size;
use crate::ui::widgets;
use crate::world::ChatLine;
use crate::world::inventory_offers::{Answer, Offer, item_label, kind_label};
use egui::{RichText, Vec2};

/// The icon of giving, the same as the notification centre's offers.
const ICON: &str = "gift-filled";
const BUTTON: Vec2 = Vec2::new(88.0, 24.0);

fn title(env: &LineEnv, line: &ChatLine, v: &Offer) -> String {
    let name = env.world.person_name(&v.from).unwrap_or_else(|| line.from.clone());
    aurora_i18n::tr!("world-mod-gives-you", from = name, what = item_label(&v.item))
}

/// None while it waits for an answer, else what to say instead of the
/// buttons.
fn answer_label(v: &Offer) -> Option<&'static str> {
    match v.answer {
        Answer::Pending => None,
        Answer::Kept => Some(aurora_i18n::tr!("ui-chat-offer-kept")),
        Answer::Rejected => Some(aurora_i18n::tr!("ui-chat-offer-rejected")),
    }
}

/// Keep and Reject, as the entry's buttons, sent as a request of the
/// interface (the app answers it).
fn buttons(ui: &mut egui::Ui, p: &Palette, offer: u64, size: Vec2) {
    if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("ui-notifications-keep"), size).clicked() {
        request(ui.ctx(), CtxAction::AnswerInventoryOffer { offer, keep: true });
    }
    if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("ui-notifications-reject"), size).clicked() {
        request(ui.ctx(), CtxAction::AnswerInventoryOffer { offer, keep: false });
    }
}

fn inventory_link(ui: &mut egui::Ui, env: &mut LineEnv, offer: &Offer) {
    if let Some(link) = offer.inventory_link(aurora_i18n::tr!("ui-notifications-show-in-inventory")) {
        chat_text_marked(
            ui,
            env.p,
            env.emoji,
            env.world,
            env.want_names,
            &link,
            chat_size(ui.ctx(), 12.0),
            env.p.ink,
            false,
            None,
        );
    }
}

/// The card, on the giver's side after `indent`.
pub(super) fn draw_card(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, offer: u64, indent: f32) {
    let p = env.p;
    let Some(v) = env.world.inventory_offers.get(offer).cloned() else {
        // the offer was dropped (old and answered): its text only
        notice(ui, env, line);
        return;
    };
    let title = title(env, line, &v);
    let done = answer_label(&v);
    card_frame(ui, env, line, indent, done.is_some(), |ui, env| {
        let (ink, muted) = if done.is_some() { (p.muted, p.muted_dim) } else { (p.ink, p.muted) };
        ui.horizontal(|ui| {
            icon(ui, env, ICON, if done.is_some() { p.muted_dim } else { p.teal });
            ui.add(egui::Label::new(RichText::new(&title).size(chat_size(ui.ctx(), 13.0)).strong().color(ink)).wrap());
        });
        ui.label(RichText::new(kind_label(v.asset_type)).size(chat_size(ui.ctx(), 12.0)).color(muted));
        match done {
            None => {
                ui.add_space(5.0);
                ui.horizontal(|ui| buttons(ui, p, offer, BUTTON));
            }
            Some(label) => {
                answered(ui, p, label);
                inventory_link(ui, env, &v);
            }
        }
    });
}

/// The legacy lines: who gives what and its kind on one line, its buttons
/// (or its answer) at the end.
pub(super) fn draw_line(ui: &mut egui::Ui, env: &mut LineEnv, line: &ChatLine, offer: u64) {
    let p = env.p;
    let Some(v) = env.world.inventory_offers.get(offer).cloned() else {
        notice(ui, env, line);
        return;
    };
    let text = format!("{} ({})", title(env, line, &v), kind_label(v.asset_type).to_lowercase());
    let done = answer_label(&v);
    ui.horizontal_wrapped(|ui| {
        icon(ui, env, ICON, if done.is_some() { p.muted_dim } else { p.teal });
        let world = env.world;
        let color = if done.is_some() { p.muted } else { p.ink };
        chat_text_marked(
            ui,
            p,
            env.emoji,
            world,
            env.want_names,
            &text,
            chat_size(ui.ctx(), 12.5),
            color,
            false,
            None,
        );
        match done {
            None => buttons(ui, p, offer, Vec2::new(80.0, 20.0)),
            Some(label) => {
                ui.label(
                    RichText::new(format!("· {label}"))
                        .size(chat_size(ui.ctx(), 12.0))
                        .color(p.muted_dim),
                );
                inventory_link(ui, env, &v);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::{Frames, Text};
    use crate::world::notifications::im;
    use crate::world::{ChatKind, World};
    use std::collections::{HashMap, HashSet};
    use uuid::Uuid;

    const JANE: Uuid = Uuid::from_u128(2);

    /// A world with Jane's offer of a notecard, and the line of its card.
    fn offered() -> (World, ChatLine) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        let mut bucket = vec![7u8];
        bucket.extend_from_slice(Uuid::from_u128(0x20).as_bytes());
        world.apply(aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: JANE,
            from_name: "Jane Doe".into(),
            to_agent_id: world.agent_id,
            dialog: im::INVENTORY_OFFERED,
            session_id: Uuid::from_u128(0x10),
            message: "Welcome note".into(),
            offline: false,
            from_group: false,
            binary_bucket: bucket,
            timestamp: 0,
        }));
        let line = world.social.ims[0].lines.back().expect("the card").clone();
        (world, line)
    }

    struct Card {
        frames: Frames,
        icons: crate::ui::icons::Icons,
        emoji: crate::ui::emoji::Emoji,
    }

    impl Card {
        fn new() -> Card {
            let frames = Frames::new();
            let icons = crate::ui::icons::Icons::load(&frames.ctx, None);
            Card {
                frames,
                icons,
                emoji: crate::ui::emoji::Emoji::load(),
            }
        }

        /// One frame of the card (`bubbles`) or of its legacy line.
        fn frame(&mut self, world: &World, line: &ChatLine, bubbles: bool, events: Vec<egui::Event>) -> Vec<Text> {
            let p = crate::theme::Theme::default().palette();
            let pics = HashMap::new();
            let (mut names, mut wanted) = (HashSet::new(), HashSet::new());
            let ChatKind::InventoryOffer(offer) = line.kind else {
                panic!("not an offer's card: {:?}", line.kind);
            };
            let Card { frames, icons, emoji } = self;
            frames.run(events, |ui| {
                let mut env = LineEnv {
                    p: &p,
                    icons,
                    emoji,
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
                    draw_card(ui, &mut env, line, offer, 0.0);
                } else {
                    draw_line(ui, &mut env, line, offer);
                }
            })
        }

        /// A click on `label`, then the actions the card asked for.
        fn click(&mut self, world: &World, line: &ChatLine, bubbles: bool, label: &str) -> Vec<CtxAction> {
            let texts = self.frame(world, line, bubbles, Vec::new());
            // Native links reserve an invisible marker glyph before their label.
            let at = texts
                .iter()
                .find(|t| t.source.ends_with(label))
                .unwrap_or_else(|| panic!("{label}: {:?}", texts.iter().map(|t| &t.source).collect::<Vec<_>>()))
                .rect
                .center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(world, line, bubbles, vec![egui::Event::PointerMoved(at)]);
            self.frame(world, line, bubbles, vec![button(true)]);
            self.frame(world, line, bubbles, vec![button(false)]);
            crate::ui::context::take_requests(&self.frames.ctx)
        }
    }

    #[test]
    fn the_card_s_buttons_answer_the_offer_and_the_card_greys() {
        for (bubbles, keep) in [(true, true), (false, false)] {
            let (mut world, line) = offered();
            let mut card = Card::new();
            let label = if keep {
                aurora_i18n::tr!("ui-notifications-keep")
            } else {
                aurora_i18n::tr!("ui-notifications-reject")
            };
            let asked = card.click(&world, &line, bubbles, label);
            let [CtxAction::AnswerInventoryOffer { offer, keep: k }] = asked[..] else {
                panic!("no answer asked: {asked:?}");
            };
            assert_eq!(k, keep);
            // what the app does with it: the entry's own answer
            let cmds = world.answer_inventory_offer(offer, k);
            assert_eq!(cmds.len(), 1, "{cmds:?}");
            assert!(world.notifications.list.is_empty());
            // the buttons give way to the answer
            let texts = card.frame(&world, &line, bubbles, Vec::new());
            let answer = if keep {
                aurora_i18n::tr!("ui-chat-offer-kept")
            } else {
                aurora_i18n::tr!("ui-chat-offer-rejected")
            };
            assert!(texts.iter().any(|t| t.source.contains(answer)), "{bubbles}");
            assert!(!texts.iter().any(|t| t.source == label), "{bubbles}");
        }
    }

    #[test]
    fn kept_offer_links_select_the_gift_in_both_chat_presentations() {
        let label = aurora_i18n::tr!("ui-notifications-show-in-inventory");
        for bubbles in [true, false] {
            for keep in [true, false] {
                let (mut world, line) = offered();
                let mut card = Card::new();
                let texts = card.frame(&world, &line, bubbles, Vec::new());
                assert!(!texts.iter().any(|t| t.source.contains(label)), "pending offer");
                let offer = world.inventory_offers.list[0].id;
                world.answer_inventory_offer(offer, keep);
                if keep {
                    let texts = card.frame(&world, &line, bubbles, Vec::new());
                    assert!(texts.iter().any(|t| t.source.contains(label)), "kept offer");
                    let asked = card.click(&world, &line, bubbles, label);
                    assert!(
                        matches!(&asked[..], [CtxAction::AppLink(crate::slapp::AppLink::Inventory { id, .. })]
                            if *id == Uuid::from_u128(0x20)),
                        "{bubbles}: {asked:?}"
                    );
                } else {
                    let texts = card.frame(&world, &line, bubbles, Vec::new());
                    assert!(!texts.iter().any(|t| t.source.contains(label)), "rejected offer");
                }
            }
        }
    }
}
