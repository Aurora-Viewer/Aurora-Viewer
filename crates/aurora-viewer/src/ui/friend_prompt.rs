//! A friendship offer in the middle of the screen (#465): an Aurora
//! addition, on by default (Preferences › Chat › Toasts), where the Legacy
//! Viewer only has the toast of OfferFriendship (notifications.xml). The
//! offer stays the notification center's: answering here answers it there,
//! "Later" leaves it under the bell, and an offer answered from the bell
//! leaves the screen with it. Several offers come one after the other.

use super::icons::Icons;
use super::widgets::{self, ActionKind, action_button};
use crate::theme::Palette;
use crate::world::notifications::{Data, Notifications, Response};
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const WIDTH: f32 = 340.0;
/// Side of the resident's picture.
const PIC: f32 = 84.0;
/// Seconds the card takes to arrive, and the picture to pop in.
const ARRIVE: f64 = 0.35;
const POP: f64 = 0.5;
/// Seconds of one pulse of the rings around the picture.
const PULSE: f64 = 1.8;
/// Hearts rise from the picture for this long after the card arrives,
/// each one in this many seconds.
const HEARTS: f64 = 4.0;
const HEART_RISE: f64 = 1.6;
const BUTTON: Vec2 = Vec2::new(100.0, 30.0);

#[derive(Default)]
pub struct FriendPromptUi {
    /// Offers put off with "Later": they wait under the bell only.
    later: HashSet<u64>,
    /// The offer on screen and when it arrived (its animation).
    shown: Option<(u64, f64)>,
}

/// What the card draws with.
pub struct PromptEnv<'a> {
    pub pics: &'a HashMap<Uuid, egui::TextureHandle>,
    /// The complete name of the resident who offers, when known.
    pub name: Option<String>,
    /// Preferences › Interface › Reduce animations off: the card arrives, the
    /// rings pulse and hearts rise.
    pub motion: bool,
}

#[derive(Debug, Clone)]
pub enum PromptAction {
    /// Accept or Decline, applied like the card's buttons.
    Answer(u64, Response),
    /// The resident's name clicked: their profile (the offer stays).
    Profile(Uuid),
}

impl FriendPromptUi {
    /// The offer to show this frame (its id and who offers). With the prompt
    /// on (`enabled`), every offer it is going to show gets no toast (it is
    /// `quiet`: listed under the bell only); while nothing may cover the view
    /// (`allowed` false: mouselook, the photo's full frame, do not disturb,
    /// the loading screen) it waits, and shows after.
    pub fn next(&mut self, list: &mut Notifications, enabled: bool, allowed: bool) -> Option<(u64, Uuid)> {
        self.later.retain(|id| list.list.iter().any(|n| n.id == *id));
        let mut first = None;
        if enabled {
            for n in list.list.iter_mut().filter(|n| !self.later.contains(&n.id)) {
                if let Data::Friend { from, .. } = n.data {
                    n.quiet = true;
                    first.get_or_insert((n.id, from));
                }
            }
        }
        let first = first.filter(|_| allowed);
        if first.is_none_or(|(id, _)| self.shown.is_some_and(|(s, _)| s != id)) {
            // a new offer arrives afresh
            self.shown = None;
        }
        first
    }

    /// "Later" on the offer shown (AURORA_DEMO_FRIEND_PROMPT=later).
    pub fn put_off(&mut self) {
        if let Some((id, _)) = self.shown.take() {
            self.later.insert(id);
        }
    }

    /// The offers waiting behind the one shown.
    fn behind(&self, list: &Notifications, shown: u64) -> usize {
        list.list
            .iter()
            .filter(|n| n.id != shown && matches!(n.data, Data::Friend { .. }) && !self.later.contains(&n.id))
            .count()
    }

    /// The card of offer `id` (from `next`), in the middle of the screen
    /// over the world, without blocking it (a profile opened from it stays
    /// usable).
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        icons: &Icons,
        list: &Notifications,
        id: u64,
        env: &PromptEnv<'_>,
    ) -> Option<PromptAction> {
        let n = list.list.iter().find(|n| n.id == id)?;
        let Data::Friend { from, .. } = n.data else {
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
        let age = (now - start).max(0.0);
        let progress = |seconds: f64| if env.motion { (age / seconds).min(1.0) as f32 } else { 1.0 };
        let arrive = ease_out(progress(ARRIVE));
        let behind = self.behind(list, id);
        let mut out = None;
        let mut later = false;
        egui::Area::new(egui::Id::unique("friend_prompt"))
            .fade_in(false)
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::new(0.0, (1.0 - arrive) * 24.0))
            .constrain(true)
            .show(ctx, |ui| {
                ui.multiply_opacity(arrive);
                widgets::overlay(ui, p, egui::Margin::same(18), |ui| {
                    ui.set_width(WIDTH);
                    ui.vertical_centered(|ui| {
                        let initials = env.name.as_deref().unwrap_or(&n.title);
                        picture(ui, p, icons, env, from, initials, age, ease_out_back(progress(POP)));
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
                                    out = Some(PromptAction::Profile(from));
                                }
                                ui.label(RichText::new(aurora_i18n::tr!("ui-friend-prompt-wants")).size(13.0).color(p.muted));
                            }
                            // the name not known yet: the toast's own title
                            None => {
                                super::notifications::text::show(ui, p, &n.title, &n.name_links, 15.0, p.ink, false, true);
                            }
                        }
                        let message = n.body.trim();
                        if !message.is_empty() {
                            ui.add_space(10.0);
                            egui::Frame::new()
                                .fill(p.field)
                                .corner_radius(ui.visuals().window_corner_radius)
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    egui::ScrollArea::vertical().max_height(96.0).show(ui, |ui| {
                                        super::notifications::text::show(ui, p, message, &n.name_links, 12.5, p.ink, false, true);
                                    });
                                });
                        }
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-friend-prompt-note"))
                                .size(11.5)
                                .color(p.muted_dim),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let row = BUTTON.x * 3.0 + 16.0;
                            ui.add_space(((ui.available_width() - row) * 0.5).max(0.0));
                            let accept = aurora_i18n::tr!("ui-notifications-accept");
                            if action_button(ui, p, "user-check", accept, ActionKind::Primary, BUTTON).clicked() {
                                out = Some(PromptAction::Answer(id, Response::Accept));
                            }
                            let decline = aurora_i18n::tr!("ui-notifications-decline");
                            if action_button(ui, p, "x", decline, ActionKind::Flat, BUTTON).clicked() {
                                out = Some(PromptAction::Answer(id, Response::Decline));
                            }
                            if action_button(
                                ui,
                                p,
                                "clock-filled",
                                aurora_i18n::tr!("ui-friend-prompt-later"),
                                ActionKind::Flat,
                                BUTTON,
                            )
                            .on_hover_text(aurora_i18n::tr!("ui-friend-prompt-later-hint"))
                            .clicked()
                            {
                                later = true;
                            }
                        });
                        if behind > 0 {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(aurora_i18n::tr!("ui-friend-prompt-more", count = behind))
                                    .size(11.5)
                                    .color(p.muted),
                            );
                        }
                    });
                });
            });
        if env.motion {
            ctx.request_repaint();
        }
        if later {
            self.put_off();
        }
        out
    }
}

/// The picture in its rings, with the friendship badge and the hearts that
/// rise from it; `pop` (0 to a little over 1) scales it as it arrives.
#[allow(clippy::too_many_arguments)]
fn picture(ui: &mut egui::Ui, p: &Palette, icons: &Icons, env: &PromptEnv<'_>, from: Uuid, name: &str, age: f64, pop: f32) {
    let (area, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), PIC + 40.0), egui::Sense::hover());
    let center = area.center() + Vec2::new(0.0, 8.0);
    let radius = PIC * 0.5 * pop.max(0.0);
    let painter = ui.painter();
    // the rings: two pulses going out, or one still ring
    if env.motion {
        for k in 0..2 {
            let phase = ((age / PULSE) + f64::from(k) * 0.5).fract() as f32;
            let alpha = (1.0 - phase) * 0.55;
            painter.circle_stroke(
                center,
                radius + 4.0 + phase * 16.0,
                egui::Stroke::new(2.0, p.violet_light.gamma_multiply(alpha)),
            );
        }
    } else {
        painter.circle_stroke(center, radius + 4.0, egui::Stroke::new(2.0, p.violet_light.gamma_multiply(0.55)));
    }
    // not before it has a size: its initials' font cannot be 0 px
    if radius >= 1.0 {
        widgets::avatar_disc(
            ui,
            p,
            egui::Rect::from_center_size(center, Vec2::splat(radius * 2.0)),
            env.pics.get(&from),
            name,
            false,
        );
    }
    // the friendship kind's icon (`notifications::kind_style`), as a badge
    let badge = center + Vec2::splat(radius * 0.72);
    let badge_radius = 13.0 * pop.clamp(0.0, 1.0);
    painter.circle_filled(badge, badge_radius + 2.0, p.panel);
    painter.circle_filled(badge, badge_radius, p.violet);
    if let Some(t) = icons.get("heart-handshake") {
        egui::Image::new(t)
            .tint(p.on_accent)
            .paint_at(ui, egui::Rect::from_center_size(badge, Vec2::splat(badge_radius * 1.15)));
    }
    if !env.motion {
        return;
    }
    // hearts rising beside the picture, each from where the last one left
    let Some(heart) = icons.get("heart-filled") else {
        return;
    };
    const SIDES: [f32; 4] = [-0.7, 0.75, -0.35, 0.45];
    for (i, side) in SIDES.iter().enumerate() {
        let local = age - i as f64 * 0.45;
        if local < 0.0 {
            continue;
        }
        let cycle = (local / HEART_RISE).floor();
        if i as f64 * 0.45 + cycle * HEART_RISE >= HEARTS {
            continue;
        }
        let t = (local / HEART_RISE - cycle) as f32;
        let at = center
            + Vec2::new(
                side * radius + (t * std::f32::consts::TAU).sin() * 4.0,
                -radius * 0.3 - t * (radius + 18.0),
            );
        let size = 10.0 + 4.0 * t;
        let alpha = (t * std::f32::consts::PI).sin();
        egui::Image::new(heart)
            .tint(p.violet_light.gamma_multiply(alpha))
            .paint_at(ui, egui::Rect::from_center_size(at, Vec2::splat(size)));
    }
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Overshoots a little before settling: the picture pops in.
fn ease_out_back(t: f32) -> f32 {
    const C: f32 = 1.70158;
    1.0 + (C + 1.0) * (t - 1.0).powi(3) + C * (t - 1.0).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOVA: Uuid = Uuid::from_u128(3);
    const TESS: Uuid = Uuid::from_u128(4);

    fn offer(list: &mut Notifications, from: Uuid) -> u64 {
        list.push(
            crate::world::notifications::Kind::Friendship,
            "Nova offers you friendship",
            "Hi!",
            Data::Friend {
                from,
                tx: Uuid::from_u128(9),
                offline: false,
            },
        )
    }

    /// On, the offers get no toast and show one after the other; off,
    /// nothing changes.
    #[test]
    fn offers_show_in_turn_without_toasts_only_when_enabled() {
        let mut list = Notifications::default();
        let mut ui = FriendPromptUi::default();
        let first = offer(&mut list, NOVA);
        let second = offer(&mut list, TESS);
        list.push(crate::world::notifications::Kind::Info, "info", "", Data::None);
        assert_eq!(ui.next(&mut list, false, true), None);
        assert!(list.list.iter().all(|n| !n.quiet));
        assert_eq!(ui.next(&mut list, true, true), Some((first, NOVA)));
        assert_eq!(list.list.iter().filter(|n| n.quiet).count(), 2);
        assert_eq!(ui.behind(&list, first), 1);
        // the first answered (from the card or the bell): the second
        list.list.retain(|n| n.id != first);
        assert_eq!(ui.next(&mut list, true, true), Some((second, TESS)));
        assert_eq!(ui.behind(&list, second), 0);
    }

    /// While the view must stay clear the offer waits, still without a
    /// toast, and shows after.
    #[test]
    fn a_blocked_prompt_waits() {
        let mut list = Notifications::default();
        let mut ui = FriendPromptUi::default();
        let id = offer(&mut list, NOVA);
        assert_eq!(ui.next(&mut list, true, false), None);
        assert!(list.list[0].quiet);
        assert_eq!(ui.next(&mut list, true, true), Some((id, NOVA)));
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

    /// Accept answers the offer; Later puts it off (under the bell) and the
    /// next one shows.
    #[test]
    fn buttons_accept_and_put_off() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let icons = Icons::default();
        let pics = HashMap::new();
        let env = PromptEnv {
            pics: &pics,
            name: Some("Nova Exemple".into()),
            motion: true,
        };
        let mut list = Notifications::default();
        let first = offer(&mut list, NOVA);
        let second = offer(&mut list, TESS);
        let mut prompt = FriendPromptUi::default();
        let mut time = 0.0;
        let mut step = |prompt: &mut FriendPromptUi, list: &mut Notifications, events: Vec<egui::Event>| {
            time += 0.5;
            let mut action = None;
            let shown = prompt.next(list, true, true);
            let output = frame(&ctx, time, events, |ctx| {
                if let Some((id, _)) = shown {
                    action = prompt.show(ctx, &p, &icons, list, id, &env);
                }
            });
            (output, shown, action)
        };
        let click = |output: &egui::FullOutput, label: &str| -> Vec<Vec<egui::Event>> {
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{label} shown"));
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            vec![vec![egui::Event::PointerMoved(pos)], vec![button(true)], vec![button(false)]]
        };
        let later = aurora_i18n::tr!("ui-friend-prompt-later");
        let mut output = None;
        for _ in 0..3 {
            output = Some(step(&mut prompt, &mut list, vec![]).0);
        }
        for events in click(output.as_ref().expect("frames ran"), later) {
            let (_, shown, action) = step(&mut prompt, &mut list, events);
            assert_eq!(shown.map(|(id, _)| id), Some(first));
            assert!(action.is_none());
        }
        // put off: still listed, the second offer on screen
        assert!(list.list.iter().any(|n| n.id == first));
        let mut answer = None;
        for _ in 0..3 {
            let (out, shown, _) = step(&mut prompt, &mut list, vec![]);
            assert_eq!(shown.map(|(id, _)| id), Some(second));
            output = Some(out);
        }
        for events in click(output.as_ref().expect("frames ran"), aurora_i18n::tr!("ui-notifications-accept")) {
            answer = step(&mut prompt, &mut list, events).2.or(answer);
        }
        assert!(matches!(answer, Some(PromptAction::Answer(id, Response::Accept)) if id == second));
    }
}
