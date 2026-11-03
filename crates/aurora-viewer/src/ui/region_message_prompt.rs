//! A message to the whole region in the middle of the screen (#1404): an
//! Aurora addition, on by default (Preferences › Chat › Toasts), where the
//! Legacy Viewer only has the toast of GodMessage (notifications.xml) and
//! the line of nearby chat, which stays. Built like the friendship offer's
//! card (`friend_prompt.rs`): the message stays the notification center's,
//! "OK" closes it there too, one answered from the bell leaves the screen
//! with it, and several come one after the other.

use super::icons::Icons;
use super::widgets::{self, ActionKind, action_button};
use crate::theme::Palette;
use crate::world::notifications::{Data, Notifications, Response};
use egui::{RichText, Vec2};
use uuid::Uuid;

const WIDTH: f32 = 380.0;
/// Side of the region's badge.
const BADGE: f32 = 52.0;
/// Seconds the card takes to arrive.
const ARRIVE: f64 = 0.35;
/// The message scrolls past this height.
const MESSAGE_HEIGHT: f32 = 180.0;
const BUTTON: Vec2 = Vec2::new(110.0, 30.0);

#[derive(Default)]
pub struct RegionMessagePromptUi {
    /// The message on screen and when it arrived (its animation).
    shown: Option<(u64, f64)>,
}

/// What the card draws with.
pub struct PromptEnv {
    /// The complete name of the sender, when known.
    pub name: Option<String>,
    /// Preferences › Interface › Reduce animations off: the card arrives.
    pub motion: bool,
}

#[derive(Debug, Clone)]
pub enum PromptAction {
    /// "OK": the message read, closed under the bell too.
    Answer(u64, Response),
    /// The sender's name clicked: their profile (the message stays).
    Profile(Uuid),
}

impl RegionMessagePromptUi {
    /// The message to show this frame (its id and its sender). With the
    /// prompt on (`enabled`), every region message gets no toast (it is
    /// `quiet`: listed under the bell only); while nothing may cover the view
    /// (`allowed` false: mouselook, the photo's full frame, do not disturb,
    /// the loading screen) it waits, and shows after.
    pub fn next(&mut self, list: &mut Notifications, enabled: bool, allowed: bool) -> Option<(u64, Uuid)> {
        let mut first = None;
        if enabled {
            for n in list.list.iter_mut() {
                if let Data::RegionMessage { from, .. } = n.data {
                    n.quiet = true;
                    first.get_or_insert((n.id, from));
                }
            }
        }
        let first = first.filter(|_| allowed);
        if first.is_none_or(|(id, _)| self.shown.is_some_and(|(s, _)| s != id)) {
            // a new message arrives afresh
            self.shown = None;
        }
        first
    }

    /// The messages waiting behind the one shown.
    fn behind(list: &Notifications, shown: u64) -> usize {
        list.list
            .iter()
            .filter(|n| n.id != shown && matches!(n.data, Data::RegionMessage { .. }))
            .count()
    }

    /// The card of message `id` (from `next`), in the middle of the screen
    /// over the world, without blocking it.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        icons: &Icons,
        list: &Notifications,
        id: u64,
        env: &PromptEnv,
    ) -> Option<PromptAction> {
        let n = list.list.iter().find(|n| n.id == id)?;
        let Data::RegionMessage { from, region } = &n.data else {
            return None;
        };
        let now = ctx.input(|i| i.time);
        let start = match self.shown {
            Some((s, t)) if s == id => t,
            _ => {
                self.shown = Some((id, now));
                now
            }
        };
        let arrive = if env.motion {
            ease_out(((now - start).max(0.0) / ARRIVE).min(1.0) as f32)
        } else {
            1.0
        };
        let behind = Self::behind(list, id);
        let mut out = None;
        egui::Area::new(egui::Id::unique("region_message_prompt"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, (1.0 - arrive) * 24.0))
            .constrain(true)
            .show(ctx, |ui| {
                ui.multiply_opacity(arrive);
                widgets::overlay(ui, p, egui::Margin::same(18), |ui| {
                    ui.set_width(WIDTH);
                    ui.vertical_centered(|ui| {
                        badge(ui, p, icons);
                        ui.add_space(10.0);
                        match &env.name {
                            Some(name) => {
                                let link = ui
                                    .add(
                                        egui::Label::new(RichText::new(name).size(18.0).strong().underline().color(p.violet_light))
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text(aurora_i18n::tr!("common-view-profile"));
                                if link.clicked() {
                                    out = Some(PromptAction::Profile(*from));
                                }
                                let sent = if region.is_empty() {
                                    aurora_i18n::tr!("ui-region-message-prompt-sent").to_owned()
                                } else {
                                    aurora_i18n::tr!("ui-region-message-prompt-sent-to", region = region.clone())
                                };
                                ui.label(RichText::new(sent).size(13.0).color(p.muted));
                            }
                            // the name not known yet: the notification's title
                            None => {
                                super::notifications::text::show(ui, p, &n.title, &n.name_links, 15.0, p.ink, false, true);
                            }
                        }
                        ui.add_space(10.0);
                        egui::Frame::new()
                            .fill(p.field)
                            .corner_radius(ui.visuals().window_corner_radius)
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                egui::ScrollArea::vertical().max_height(MESSAGE_HEIGHT).show(ui, |ui| {
                                    super::notifications::text::show(ui, p, n.body.trim(), &n.name_links, 14.0, p.ink, false, true);
                                });
                            });
                        ui.add_space(14.0);
                        if action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, BUTTON).clicked() {
                            out = Some(PromptAction::Answer(id, Response::Dismiss));
                        }
                        if behind > 0 {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(aurora_i18n::tr!("ui-region-message-prompt-more", count = behind))
                                    .size(11.5)
                                    .color(p.muted),
                            );
                        }
                    });
                });
            });
        if env.motion && arrive < 1.0 {
            ctx.request_repaint();
        }
        out
    }
}

/// The region's icon (`world-filled`, the region's notices in nearby chat)
/// on the accent, as the friendship card's badge.
fn badge(ui: &mut egui::Ui, p: &Palette, icons: &Icons) {
    let (area, _) = ui.allocate_exact_size(Vec2::splat(BADGE), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(area.center(), BADGE * 0.5, p.violet);
    if let Some(t) = icons.get("world-filled") {
        egui::Image::new(t)
            .tint(p.on_accent)
            .paint_at(ui, egui::Rect::from_center_size(area.center(), Vec2::splat(BADGE * 0.55)));
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::notifications::Kind;

    const MANAGER: Uuid = Uuid::from_u128(2);
    const OWNER: Uuid = Uuid::from_u128(3);

    fn message(list: &mut Notifications, from: Uuid, text: &str) -> u64 {
        list.push(
            Kind::Info,
            "Region message from Estate Manager",
            text,
            Data::RegionMessage {
                from,
                region: "Aurora Demo".into(),
            },
        )
    }

    /// On, the messages get no toast and show one after the other; off,
    /// nothing changes; while the view must stay clear they wait.
    #[test]
    fn messages_show_in_turn_without_toasts_only_when_enabled() {
        let mut list = Notifications::default();
        let mut ui = RegionMessagePromptUi::default();
        let first = message(&mut list, MANAGER, "Restart in 5 minutes");
        let second = message(&mut list, OWNER, "Back soon");
        list.push(Kind::Info, "info", "", Data::None);
        assert_eq!(ui.next(&mut list, false, true), None);
        assert!(list.list.iter().all(|n| !n.quiet));
        assert_eq!(ui.next(&mut list, true, false), None);
        assert_eq!(list.list.iter().filter(|n| n.quiet).count(), 2);
        assert_eq!(ui.next(&mut list, true, true), Some((first, MANAGER)));
        assert_eq!(RegionMessagePromptUi::behind(&list, first), 1);
        list.list.retain(|n| n.id != first);
        assert_eq!(ui.next(&mut list, true, true), Some((second, OWNER)));
        assert_eq!(RegionMessagePromptUi::behind(&list, second), 0);
    }

    fn frame(ctx: &egui::Context, time: f64, events: Vec<egui::Event>, mut run: impl FnMut(&egui::Context)) -> egui::FullOutput {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| run(ui.ctx()),
        );
        output.textures_delta.clear();
        output
    }

    /// OK answers the message shown; the card keeps its size frame after
    /// frame, a long message scrolling inside it.
    #[test]
    fn ok_closes_the_message_and_the_card_stays_steady() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let env = PromptEnv {
            name: Some("Estate Manager".into()),
            motion: false,
        };
        let mut list = Notifications::default();
        let long = "The region restarts tonight for maintenance. ".repeat(40);
        let id = message(&mut list, MANAGER, &long);
        let mut prompt = RegionMessagePromptUi::default();
        let mut time = 0.0;
        let mut sizes = Vec::new();
        let mut last = None;
        for _ in 0..4 {
            time += 0.5;
            let shown = prompt.next(&mut list, true, true);
            let output = frame(&ctx, time, vec![], |ctx| {
                if let Some((id, _)) = shown {
                    prompt.show(ctx, &p, &icons, &list, id, &env);
                }
            });
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::unique("region_message_prompt")))
                .expect("the card");
            sizes.push(rect.size());
            last = Some(output);
        }
        assert!(sizes.windows(2).skip(1).all(|w| w[0] == w[1]), "{sizes:?}");
        assert!(sizes[3].y < 520.0, "the message scrolls: {sizes:?}");
        let ok = aurora_i18n::tr!("common-ok");
        let pos = last
            .expect("frames ran")
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == ok => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .expect("OK shown");
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut answer = None;
        for events in [vec![egui::Event::PointerMoved(pos)], vec![button(true)], vec![button(false)]] {
            time += 0.1;
            let shown = prompt.next(&mut list, true, true);
            frame(&ctx, time, events, |ctx| {
                if let Some((id, _)) = shown {
                    answer = prompt.show(ctx, &p, &icons, &list, id, &env).or(answer.take());
                }
            });
        }
        assert!(matches!(answer, Some(PromptAction::Answer(a, Response::Dismiss)) if a == id));
    }
}
