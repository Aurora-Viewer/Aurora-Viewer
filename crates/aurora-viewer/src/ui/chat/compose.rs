//! The field of a conversation in Aurora's workspace: who is typing above
//! it, then a wide box with the emoji, the gestures (nearby chat; the
//! gesture combo of Firestorm's nearby chat bar) or the inventory to give
//! (a resident: what is dragged onto the conversation is offered, like
//! LLFloaterIMSession::handleDragAndDrop), the text, the channel and the
//! volume of the nearby chat (whisper, say, shout: Ctrl+Enter shouts and
//! Shift+Enter whispers whatever is chosen, like the chat bar) and send.
//! Each conversation keeps its draft, and its field takes the keyboard as
//! in the other styles (#422, #425).

use super::{ChatUi, ConvAction, OutgoingChat, completion, parse_channel, sidebar, typing};
use crate::theme::Palette;
use crate::ui::menu;
use crate::world::World;
use aurora_net::ChatType;
use egui::RichText;

/// The line of who is typing, and the least height of the box.
const TYPING_H: f32 = 18.0;
pub(super) const BOX_H: f32 = 42.0;
/// What the field takes under the messages: the typing line, the box and
/// the margin under it.
pub(super) const HEIGHT: f32 = TYPING_H + BOX_H + 12.0;
const BUTTON: f32 = 28.0;

/// The volume of what we say in the nearby chat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Volume {
    Whisper,
    #[default]
    Say,
    Shout,
}

impl Volume {
    fn chat_type(self) -> ChatType {
        match self {
            Volume::Whisper => ChatType::Whisper,
            Volume::Say => ChatType::Normal,
            Volume::Shout => ChatType::Shout,
        }
    }
}

/// The names of who types in the conversation shown: the resident of an
/// IM (IM_TYPING_START), the avatars in chat range in the nearby chat
/// (their typing animation). A group's session carries no typing.
fn typing_names(world: &World, st: &ChatUi) -> Vec<String> {
    match st.selected {
        None => st.nearby_typing_names(world),
        Some(id) if sidebar::kind(world, &id) == sidebar::Kind::Private && world.social.is_typing(&id) => {
            vec![world.social.display_name_of(&id)]
        }
        Some(_) => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &crate::ui::icons::Icons,
    emoji: &mut crate::ui::emoji::Emoji,
    gestures: &crate::gestures::Manager,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
    quote: Option<String>,
) {
    let conv_input = egui::Id::new(("conv_input", st.selected));
    let kind = st.selected.map(|id| sidebar::kind(world, &id));
    let width = ui.available_width();
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.set_width(width - 28.0);
            // ---- who is typing
            let (line, _) = ui.allocate_exact_size(egui::vec2(width - 28.0, TYPING_H), egui::Sense::hover());
            let names = typing_names(world, st);
            if let Some(text) = typing::text(&names) {
                let t = ui.input(|i| i.time) as f32;
                for k in 0..3 {
                    let phase = ((t * 1.2 - k as f32 * 0.17).rem_euclid(1.0) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
                    let c = egui::pos2(line.left() + 6.0 + k as f32 * 6.0, line.center().y);
                    ui.painter().circle_filled(c, 2.0, p.muted.gamma_multiply(0.25 + 0.75 * phase));
                }
                // the names in the ink, semi-bold (the mockup's `.typing b`)
                // (the job is built before the fonts are taken: it reads them)
                let job = typing_job(ui.ctx(), p, &text, &names, line.width() - 26.0);
                let galley = ui.painter().layout_job(job);
                ui.painter().galley(
                    egui::pos2(line.left() + 26.0, line.center().y - galley.size().y * 0.5),
                    galley,
                    p.muted,
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
            }
            // ---- the box
            let mut draft = std::mem::take(st.draft_mut(st.selected));
            if let Some(q) = quote {
                draft = format!("{q}{draft}");
                st.conv_focus_request = true;
            }
            let mut emoji_btn = None;
            let mut send_clicked = false;
            let local = st.selected.is_none();
            let roomy = width > 520.0;
            let field = egui::Frame::new()
                .fill(p.raised)
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.set_min_height(BOX_H - 8.0);
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let b = box_button(ui, p, "mood-smile-filled", aurora_i18n::tr!("ui-chat-emoji"), st.emoji_open);
                        emoji_btn = Some(b.rect);
                        if b.clicked() {
                            st.emoji_open = !st.emoji_open;
                        }
                        match kind {
                            None => gestures_button(ui, p, world, gestures, actions),
                            Some(sidebar::Kind::Private) => {
                                let tip = aurora_i18n::tr!("ui-chat-give-inventory-tip");
                                if box_button(ui, p, "paperclip", tip, false).clicked() {
                                    actions.push(ConvAction::OpenInventory);
                                }
                            }
                            Some(_) => {}
                        }
                        // the right end first: what it takes fixes the field's width
                        let (channel, _) = parse_channel(&draft);
                        let segments = [
                            (aurora_i18n::tr!("ui-chat-volume-whisper"), true),
                            (aurora_i18n::tr!("ui-chat-volume-say"), true),
                            (aurora_i18n::tr!("ui-chat-volume-shout"), true),
                        ];
                        let seg_w = if local && roomy {
                            crate::ui::widgets::segmented_size(ui, &segments).x + 4.0
                        } else {
                            0.0
                        };
                        // narrow, the channel and the volume give their room
                        // to the text (Ctrl / Shift + Enter and "/n" still work)
                        let chip_w = if local && roomy { 34.0 } else { 0.0 };
                        let field_w = (ui.available_width() - seg_w - chip_w - BUTTON - 12.0).max(40.0);
                        let hint = match st.selected {
                            None => aurora_i18n::tr!("ui-chat-write-local").to_owned(),
                            Some(id) => aurora_i18n::tr!("ui-chat-write-to", name = world.session_title(&id)),
                        };
                        let mut output = emoji.text_field(ui, &mut draft, |edit| {
                            edit.id(conv_input)
                                .hint_text(hint)
                                .frame(egui::Frame::NONE)
                                .desired_width(field_w)
                                .min_size(egui::vec2(field_w, BUTTON))
                                .vertical_align(egui::Align::Center)
                        });
                        if local {
                            completion::complete(ui, &mut output, &mut draft, gestures);
                            if roomy {
                                channel_chip(ui, p, &mut draft, channel);
                                let mut selected = match st.volume {
                                    Volume::Whisper => 0,
                                    Volume::Say => 1,
                                    Volume::Shout => 2,
                                };
                                if crate::ui::widgets::segmented(ui, p, &mut selected, &segments) {
                                    st.volume = [Volume::Whisper, Volume::Say, Volume::Shout][selected.min(2)];
                                }
                            }
                        }
                        let has_text = !draft.trim().is_empty();
                        send_clicked = send_button(ui, p, has_text).clicked() && has_text;
                        output.response
                    })
                    .inner
                });
            let resp = field.inner;
            if st.emoji_open {
                let anchor = emoji_btn.unwrap_or(resp.rect).left_top();
                let mut close = false;
                egui::Area::new(egui::Id::new("emoji_picker"))
                    .fade_in(false)
                    .order(egui::Order::Foreground)
                    .pivot(egui::Align2::LEFT_BOTTOM)
                    .fixed_pos(anchor - egui::vec2(0.0, 8.0))
                    .show(ui.ctx(), |ui| {
                        crate::ui::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                            ui.set_width(300.0);
                            let picked = emoji.picker(ui, p, icons);
                            if let Some(c) = picked.chosen {
                                draft.push(c);
                                close = !ui.input(|i| i.modifiers.shift);
                            }
                            close |= picked.escape;
                        });
                    });
                if close {
                    st.emoji_open = false;
                    ui.ctx().memory_mut(|m| m.request_focus(conv_input));
                }
            }
            if st.conv_focus_request {
                st.conv_focus_request = false;
                resp.request_focus();
                super::cursor_to_end(ui.ctx(), conv_input, &draft);
            }
            let mut sent = false;
            if send_clicked
                || ((resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt))
            {
                let text = draft.trim().to_owned();
                if !text.is_empty() {
                    sent = true;
                    match st.selected {
                        None => {
                            let (channel, message) = parse_channel(&text);
                            let volume = st.volume.chat_type();
                            actions.push(ConvAction::Local(OutgoingChat {
                                message,
                                channel,
                                chat_type: ui.input(|i| {
                                    if i.modifiers.ctrl {
                                        ChatType::Shout
                                    } else if i.modifiers.shift {
                                        ChatType::Whisper
                                    } else {
                                        volume
                                    }
                                }),
                            }));
                        }
                        Some(to) => actions.push(ConvAction::Im { to, text }),
                    }
                    // what we say ends the "New" line
                    st.first_unread.remove(&st.selected);
                }
                draft.clear();
                resp.request_focus();
            }
            st.typing_field = Some(typing::Field {
                to: st
                    .selected
                    .filter(|_| kind == Some(sidebar::Kind::Private) && (resp.has_focus() || sent)),
                typed: resp.changed(),
                empty: draft.trim().is_empty(),
                sent,
            });
            if !draft.is_empty() {
                *st.draft_mut(st.selected) = draft;
            } else {
                st.drafts.remove(&st.selected);
            }
        });
    });
}

/// "Nova is typing…" with the names in the ink, semi-bold, cut at `width`.
fn typing_job(ctx: &egui::Context, p: &Palette, text: &str, names: &[String], width: f32) -> egui::text::LayoutJob {
    let plain = egui::TextFormat::simple(egui::FontId::proportional(11.0), p.muted);
    let bold = crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ctx), 11.0, p.ink);
    let mut job = egui::text::LayoutJob::default();
    let mut rest = text;
    while !rest.is_empty() {
        let next = names
            .iter()
            .filter(|n| !n.is_empty())
            .filter_map(|n| rest.find(n.as_str()).map(|at| (at, n.len())))
            .min();
        match next {
            Some((at, len)) => {
                job.append(&rest[..at], 0.0, plain.clone());
                job.append(&rest[at..at + len], 0.0, bold.clone());
                rest = &rest[at + len..];
            }
            None => {
                job.append(rest, 0.0, plain.clone());
                rest = "";
            }
        }
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    job
}

/// A button of the box: an icon, lit while its picker is open.
fn box_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, active: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(BUTTON), egui::Sense::hover());
    let resp = ui.interact(rect, sidebar::control_id(&format!("box-{icon}")), egui::Sense::click());
    if active || resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field.gamma_multiply(0.8));
    }
    super::thread::paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(19.0)),
        if active || resp.hovered() { p.ink } else { p.muted },
    );
    resp.on_hover_text(tip)
}

/// Our active gestures, to play one (the gesture combo of Firestorm's
/// nearby chat bar, FSNearbyChatControl).
fn gestures_button(ui: &mut egui::Ui, p: &Palette, world: &World, gestures: &crate::gestures::Manager, actions: &mut Vec<ConvAction>) {
    let resp = box_button(ui, p, "hand-finger", aurora_i18n::tr!("ui-chat-gestures"), false);
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(200.0);
        let mut list: Vec<(uuid::Uuid, String, String)> = gestures
            .active
            .iter()
            .map(|id| {
                let name = world
                    .inventory
                    .items
                    .get(id)
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| aurora_i18n::tr!("common-loading-2").to_owned());
                let trigger = gestures.entries.get(id).map(|e| e.definition.trigger.clone()).unwrap_or_default();
                (*id, name, trigger)
            })
            .collect();
        list.sort_by_key(|(_, n, _)| n.to_lowercase());
        if list.is_empty() {
            menu::info(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-chat-no-gestures"));
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (id, name, trigger) in list {
                let label = if trigger.is_empty() {
                    name
                } else {
                    format!("{name}  ·  {trigger}")
                };
                if menu::item(ui, p, "hand-finger", &label) {
                    actions.push(ConvAction::PlayGesture(id));
                }
            }
        });
    });
}

/// The nearby chat's channel ("/0"): a click opens its number, which
/// rewrites the "/n" opening the text.
fn channel_chip(ui: &mut egui::Ui, p: &Palette, draft: &mut String, channel: i32) {
    let text = format!("/{channel}");
    let (rect, _) = ui.allocate_exact_size(egui::vec2(30.0, 22.0), egui::Sense::hover());
    let resp = ui.interact(rect, sidebar::control_id("box-channel"), egui::Sense::click());
    ui.painter().rect_filled(
        rect,
        ui.visuals().widgets.inactive.corner_radius,
        if resp.hovered() { p.field.gamma_multiply(1.3) } else { p.field },
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(11.0),
        if channel == 0 { p.muted } else { p.amber },
    );
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-chat-channel-tip"));
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(170.0);
        ui.label(RichText::new(aurora_i18n::tr!("ui-chat-channel")).size(11.5).color(p.muted));
        let mut n = channel;
        ui.add(egui::DragValue::new(&mut n).range(-2_147_483_648..=2_147_483_647).speed(0.2));
        if n != channel {
            let (_, message) = parse_channel(draft);
            *draft = if n == 0 { message } else { format!("/{n} {message}") };
        }
        if channel != 0 && menu::item(ui, p, "x", aurora_i18n::tr!("ui-chat-channel-reset")) {
            let (_, message) = parse_channel(draft);
            *draft = message;
            ui.close();
        }
    });
}

/// Send, in the accent (dimmed while there is nothing to send).
fn send_button(ui: &mut egui::Ui, p: &Palette, enabled: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(BUTTON), egui::Sense::hover());
    let resp = ui.interact(rect, sidebar::control_id("box-send"), egui::Sense::click());
    if enabled && resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field.gamma_multiply(0.8));
    }
    super::thread::paint_icon(
        ui.painter(),
        "send-filled",
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(18.0)),
        if enabled {
            p.violet_light
        } else {
            p.violet_light.gamma_multiply(0.45)
        },
    );
    resp.on_hover_text(aurora_i18n::tr!("ui-chat-send"))
}
