//! The field of a conversation in Aurora's workspace: who is typing above
//! it, then a wide box with the emoji, the volume of the nearby chat (a
//! menu of whisper, say, shout, kept at every width: Ctrl+Enter shouts and
//! Shift+Enter whispers whatever is chosen, like the chat bar), the text, the
//! channel of the nearby chat (when the preferences show it,
//! FSShowChatChannel) and send. The gestures are played from their
//! window, their keys or their trigger typed in the nearby chat.
//! Each conversation keeps its draft, and its field takes the keyboard as
//! in the other styles (#422, #425).

use super::{ChatUi, ConvAction, OutgoingChat, parse_channel, sidebar, suggest, typing};
use crate::theme::Palette;
use crate::ui::menu;
use crate::world::World;
use aurora_net::ChatType;
use egui::RichText;

/// The line of who is typing, and the least height of the box.
const TYPING_H: f32 = 18.0;
pub(super) const BOX_H: f32 = 42.0;
const BUTTON: f32 = 28.0;

/// What the field takes under the messages: typing line, box and margin.
pub(super) fn height(ctx: &egui::Context, conversation: Option<uuid::Uuid>) -> f32 {
    let field = egui::Id::unique(("conv_input", conversation));
    let bars = ctx.data(|d| d.get_temp::<f32>(bars_id(conversation))).unwrap_or(0.0);
    TYPING_H + bars + BOX_H.max(super::input::height(ctx, field, BUTTON) + 8.0) + 12.0
}

/// Where the height of the translation bars above the field is kept, for
/// [`height`] at the next frame.
fn bars_id(conversation: Option<uuid::Uuid>) -> egui::Id {
    egui::Id::unique(("translate_bars", conversation))
}

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

    fn label(self) -> &'static str {
        match self {
            Volume::Whisper => aurora_i18n::tr!("ui-chat-volume-whisper"),
            Volume::Say => aurora_i18n::tr!("ui-chat-volume-say"),
            Volume::Shout => aurora_i18n::tr!("ui-chat-volume-shout"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Volume::Whisper => "volume-2",
            Volume::Say => "volume",
            Volume::Shout => "speakerphone",
        }
    }
}

/// The names of who types in the conversation shown: the resident of an
/// IM (IM_TYPING_START), the avatars in chat range in the nearby chat
/// (their typing notices or animation). A group's session carries no typing.
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
    let conv_input = egui::Id::unique(("conv_input", st.selected));
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
                crate::ui::widgets::paint_row_text(ui.painter(), galley, line.left() + 26.0, line.center().y, p.muted);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
            }
            // ---- the box
            let mut draft = std::mem::take(st.draft_mut(st.selected));
            let top = ui.cursor().top();
            if kind == Some(sidebar::Kind::Private)
                && let Some(session) = st.selected
            {
                translate_bars(ui, p, world, session, &mut draft, actions);
            }
            let bars = ui.cursor().top() - top;
            let id = bars_id(st.selected);
            if ui.ctx().data(|d| d.get_temp::<f32>(id)) != Some(bars) {
                ui.ctx().data_mut(|d| d.insert_temp(id, bars));
                ui.ctx().request_repaint();
            }
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
                            st.gif_open = false;
                        }
                        // shown once a key is entered (Preferences › Chat › GIFs)
                        if st.gifs.usable() {
                            let b = box_button(ui, p, "gif", aurora_i18n::tr!("ui-chat-gif"), st.gif_open);
                            super::gif_picker::button(&b, p, st);
                        }
                        // the narrow layout keeps the room for the field: the
                        // button only while the conversation is translated
                        // (`/tr` works everywhere)
                        if kind == Some(sidebar::Kind::Private)
                            && let Some(session) = st.selected
                            && ((width > 320.0 && world.translate_view.built) || world.translate_view.partners.contains_key(&session))
                        {
                            translate_button(ui, p, world, session, actions);
                        }
                        // narrow, only its icon. An IM has no attach button:
                        // inventory is given by dropping it on the
                        // conversation, which says so during the drag (#627)
                        if kind.is_none() {
                            volume_button(ui, p, st, roomy);
                        }
                        // the right end first: what it takes fixes the field's width
                        let (channel, _) = parse_channel(&draft);
                        // the channel's chip: when the preferences show it, not
                        // in the narrow layout ("/n" before the text still works)
                        let chip = local && roomy && st.show_channel;
                        let chip_w = if chip { 34.0 } else { 0.0 };
                        let field_w = (ui.available_width() - chip_w - BUTTON - 12.0).max(40.0);
                        let hint = match st.selected {
                            None => aurora_i18n::tr!("ui-chat-write-local").to_owned(),
                            Some(id) => match world.translate_view.partners.get(&id) {
                                Some(l) => aurora_i18n::tr!("translate-write-to", name = world.session_title(&id), lang = l.native),
                                None => aurora_i18n::tr!("ui-chat-write-to", name = world.session_title(&id)),
                            },
                        };
                        super::input::prepare(ui, conv_input, local);
                        let recalled = st.input_history.entry(st.selected).or_default().recall(ui, conv_input, &mut draft);
                        let nav = crate::ui::emoji::codes::keys(ui, conv_input);
                        let suggest_nav = if nav.is_none() { suggest::keys(ui, conv_input) } else { None };
                        let mut output = super::input::show(ui, Some(&mut *emoji), conv_input, &mut draft, field_w, BUTTON, |edit| {
                            edit.id(conv_input).hint_text(hint).frame(egui::Frame::NONE)
                        });
                        if recalled {
                            output.response.mark_changed();
                        }
                        crate::ui::emoji::codes::after(ui, p, Some(&mut *emoji), &mut output, &mut draft, nav);
                        crate::ui::spelling::field(ui, p, &mut output, &mut draft, true);
                        let selected = st.selected;
                        let people = move || selected.map_or_else(Vec::new, |id| suggest::members(world, id));
                        let scope = match selected {
                            None => suggest::Scope::Nearby(gestures),
                            Some(id) => suggest::Scope::Session(id, &people),
                        };
                        suggest::after(ui, p, &mut output, &mut draft, suggest_nav, &mut st.suggest, scope);
                        // `chip` is only for the nearby chat
                        if chip {
                            channel_chip(ui, p, &mut draft, channel);
                        }
                        let has_text = !draft.trim().is_empty();
                        send_clicked = send_button(ui, p, has_text).clicked() && has_text;
                        output
                            .response
                            .response
                            .on_hover_text(aurora_i18n::tr!("ui-chat-compose-shortcuts"))
                    })
                    .inner
                });
            let resp = field.inner;
            if st.emoji_open && super::popout::claim_picker(ui.ctx()) {
                let anchor = emoji_btn.unwrap_or(resp.rect).left_top();
                let mut close = false;
                // one per conversation: one in each window (`popout.rs`)
                egui::Area::new(egui::Id::unique(("emoji_picker", st.selected)))
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
            let anchor = emoji_btn.unwrap_or(resp.rect).left_top();
            let chat_type = st.volume.chat_type();
            super::gif_picker::popup(ui.ctx(), p, icons, st, anchor, 8.0, conv_input, chat_type, actions);
            if st.conv_focus_request {
                st.conv_focus_request = false;
                resp.request_focus();
                super::cursor_to_end(ui.ctx(), conv_input, &draft);
            }
            let mut sent = false;
            if send_clicked
                || ((resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.alt))
            {
                let text = st.suggest.expand(st.selected, draft.trim());
                if !text.is_empty() {
                    sent = true;
                    st.input_history.entry(st.selected).or_default().record(conv_input, &draft);
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
            if local {
                let said = typing::says_aloud(&draft, st.type_during_emote, st.colon_emote);
                st.local_field(&resp, said, sent);
            }
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
/// The translation of a private conversation (`crate::translate`): its
/// languages in the tooltip, the menu of languages on a click.
fn translate_button(ui: &mut egui::Ui, p: &Palette, world: &World, session: uuid::Uuid, actions: &mut Vec<ConvAction>) {
    let view = &world.translate_view;
    let partner = view.partners.get(&session).copied();
    let tip = match (partner, view.me) {
        (Some(theirs), Some(mine)) => {
            let mut tip = aurora_i18n::tr!("translate-button-on", mine = mine.native, theirs = theirs.native).to_string();
            if view.offline {
                tip = format!("{tip}\n{}", aurora_i18n::tr!("translate-offline"));
            } else if view.status.is_some_and(|s| s.loaded && !s.on_gpu) {
                tip = format!("{tip}\n{}", aurora_i18n::tr!("translate-on-cpu"));
            }
            tip
        }
        _ => aurora_i18n::tr!("translate-button-off").to_string(),
    };
    let b = box_button(ui, p, "language", &tip, partner.is_some());
    menu::dropdown(&b, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        if menu::check(
            ui,
            p,
            "language-off",
            aurora_i18n::tr!("translate-menu-off"),
            partner.is_none(),
            true,
        ) {
            actions.push(ConvAction::Translate { session, to: None });
        }
        menu::separator(ui, p);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for l in aurora_translate::LANGS.iter().filter(|l| Some(*l) != view.me) {
                let label = if l.native == l.english {
                    l.native.to_owned()
                } else {
                    format!("{} · {}", l.native, l.english)
                };
                if menu::check(ui, p, "language", &label, partner == Some(l), true) {
                    actions.push(ConvAction::Translate { session, to: Some(l) });
                }
            }
        });
    });
}

/// Above the field of a private conversation: our message held back by its
/// translation, or the suggestion to translate the conversation.
fn translate_bars(ui: &mut egui::Ui, p: &Palette, world: &World, session: uuid::Uuid, draft: &mut String, actions: &mut Vec<ConvAction>) {
    use crate::ui::widgets::{ActionKind, action_button};
    let view = &world.translate_view;
    let size = egui::vec2(0.0, 24.0);
    if let Some(h) = view.held_in(session).next() {
        ui.horizontal_wrapped(|ui| {
            let short: String = h.original.chars().take(40).collect();
            let text = if h.too_long {
                aurora_i18n::tr!("translate-held-too-long", text = short)
            } else {
                aurora_i18n::tr!("translate-held-failed", text = short)
            };
            ui.label(RichText::new(text).color(p.muted));
            if !h.too_long && action_button(ui, p, "refresh", aurora_i18n::tr!("translate-retry"), ActionKind::Primary, size).clicked() {
                actions.push(ConvAction::TranslateRetry(h.key));
            }
            if action_button(
                ui,
                p,
                "send-filled",
                aurora_i18n::tr!("translate-send-original"),
                ActionKind::Flat,
                size,
            )
            .clicked()
            {
                actions.push(ConvAction::TranslateSendOriginal(h.key));
            }
            if action_button(ui, p, "pencil-filled", aurora_i18n::tr!("translate-edit"), ActionKind::Flat, size).clicked() {
                *draft = h.original.clone();
                actions.push(ConvAction::TranslateDiscard(h.key));
            }
        });
        ui.add_space(4.0);
    } else if let (Some(theirs), Some(mine)) = (view.suggestions.get(&session).copied(), view.me) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("translate-suggest", lang = theirs.native)).color(p.muted));
            let pair = format!("{} ⇄ {}", mine.code.to_uppercase(), theirs.code.to_uppercase());
            if action_button(ui, p, "language", &pair, ActionKind::Primary, size).clicked() {
                actions.push(ConvAction::Translate { session, to: Some(theirs) });
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("translate-not-now"), ActionKind::Flat, size).clicked() {
                actions.push(ConvAction::TranslateDismiss(session));
            }
        });
        ui.add_space(4.0);
    }
}

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

/// The volume of the nearby chat: its icon, its name when `named`, and a
/// menu of the three.
fn volume_button(ui: &mut egui::Ui, p: &Palette, st: &mut ChatUi, named: bool) {
    let galley = named.then(|| {
        ui.painter().layout_no_wrap(
            st.volume.label().to_owned(),
            egui::FontId::proportional(12.5),
            egui::Color32::PLACEHOLDER,
        )
    });
    let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + 5.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0 + 17.0 + text_w + 3.0 + 12.0 + 5.0, BUTTON), egui::Sense::hover());
    let resp = ui.interact(rect, sidebar::control_id("box-volume"), egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let lit = open || resp.hovered();
    if lit {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field.gamma_multiply(0.8));
    }
    let tint = if lit { p.ink } else { p.muted };
    let mut x = rect.left() + 6.0;
    super::thread::paint_icon(
        ui.painter(),
        st.volume.icon(),
        egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.5), egui::Vec2::splat(17.0)),
        tint,
    );
    x += 17.0;
    if let Some(g) = galley {
        x += 5.0;
        let size = g.size();
        ui.painter().galley(egui::pos2(x, rect.center().y - size.y * 0.5), g, tint);
        x += size.x;
    }
    super::thread::paint_icon(
        ui.painter(),
        "chevron-down",
        egui::Rect::from_min_size(egui::pos2(x + 3.0, rect.center().y - 6.0), egui::Vec2::splat(12.0)),
        tint,
    );
    let resp = resp.on_hover_text(aurora_i18n::tr!("ui-chat-volume-tip"));
    if std::mem::take(&mut st.demo_volume_menu) {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
    }
    // above the field, at the bottom of the window
    let place = Some((egui::RectAlign::TOP_START, 6.0));
    menu::dropdown_placed(&resp, p, egui::PopupCloseBehavior::CloseOnClick, place, |ui| {
        for v in [Volume::Whisper, Volume::Say, Volume::Shout] {
            if menu::check(ui, p, v.icon(), v.label(), st.volume == v, true) {
                st.volume = v;
            }
        }
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
