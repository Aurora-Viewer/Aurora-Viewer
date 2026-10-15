//! The calls of a conversation in Aurora's workspace, between its head and
//! its messages:
//!
//! - an invitation: a banner with the caller, decline and answer
//!   (LLIncomingCallDialog);
//! - the call in progress: a stage with everyone's picture, who speaks (a
//!   green ring), who a moderator muted, who is still rung; the microphone,
//!   the audio output, adding people and hanging up. A click on someone
//!   gives their volume (0–200 %, LLSpeakerVolumeStorage), mutes them for us
//!   and opens their profile. The stage reduces to a one-line strip;
//! - a call of the group or conference we are not in: a strip to join it.
//!
//! What Firestorm shows of a call in its IM window (FSFloaterIM's call
//! button and LLParticipantList), laid out as a desktop chat does.

use super::{ChatUi, ConvAction, call::duration};
use crate::theme::Palette;
use crate::ui::menu;
use crate::world::World;
use crate::world::calls::{Call, CallKind, CallState, InviteAnswer, PendingInvite};
use egui::{Color32, RichText};
use std::collections::HashMap;
use uuid::Uuid;

/// The pictures of the stage, and the width each person takes.
const PERSON: f32 = 48.0;
const PERSON_W: f32 = 70.0;
/// The round buttons of the stage, and those of the strips.
const BUTTON: f32 = 40.0;
const SMALL: f32 = 30.0;

/// Someone on the stage.
struct Person {
    id: Uuid,
    speaking: bool,
    /// Muted by a moderator.
    muted: bool,
    /// Rung, not in the call yet.
    waiting: bool,
}

/// Who the stage shows: the others in the call, then those still rung (the
/// resident of an outgoing call, the residents invited into a conference).
fn people(world: &World, call: &Call) -> Vec<Person> {
    let mut out: Vec<Person> = call
        .members
        .iter()
        .filter(|m| m.id != world.agent_id)
        .map(|m| Person {
            id: m.id,
            speaking: m.speaking,
            muted: m.muted,
            waiting: false,
        })
        .collect();
    let mut waiting: Vec<Uuid> = world.calls.invited.get(&call.session).cloned().unwrap_or_default();
    if let CallKind::P2p { other } = call.kind
        && call.state != CallState::Connected
    {
        waiting.push(other);
    }
    for id in waiting {
        if !out.iter().any(|p| p.id == id) {
            out.push(Person {
                id,
                speaking: false,
                muted: false,
                waiting: true,
            });
        }
    }
    out
}

/// "Connected", "Calling…", "Joining the call…".
fn state_text(call: &Call) -> &'static str {
    match call.state {
        CallState::Connected => aurora_i18n::tr!("ui-chat-call-live"),
        CallState::Ringing if call.p2p() && !call.incoming => aurora_i18n::tr!("ui-call-calling"),
        _ => aurora_i18n::tr!("ui-call-connecting"),
    }
}

fn elapsed(call: &Call) -> Option<String> {
    call.connected_at.map(|t| duration(t.elapsed().as_secs()))
}

/// The calls' area of the conversation shown, if it has one.
pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
) {
    let Some(conversation) = st.selected else {
        return;
    };
    if let Some(call) = world.call_of(&conversation) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(
            if call.state == CallState::Connected { 250 } else { 50 },
        ));
        if st.stage_mini {
            mini(ui, p, pics, world, call, st, actions);
        } else {
            stage(ui, p, pics, world, call, st, actions);
        }
    } else if let Some(invite) = world.calls.invite_for(&conversation) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        incoming(ui, p, pics, world, invite, actions);
    } else if let Some((by, since)) = world.call_launched(&conversation) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(10));
        ongoing(ui, p, pics, world, st, conversation, by, since, actions);
    }
}

/// A card under the head, 12 px from the sides.
fn card<R>(ui: &mut egui::Ui, fill: Color32, edge: Option<Color32>, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(8.0);
    let r = ui
        .horizontal(|ui| {
            ui.add_space(12.0);
            let inner = egui::Frame::new()
                .fill(fill)
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width() - 12.0 - 20.0);
                    body(ui)
                });
            if let Some(edge) = edge {
                ui.painter().rect_stroke(
                    inner.response.rect,
                    ui.visuals().window_corner_radius,
                    egui::Stroke::new(1.0, edge),
                    egui::StrokeKind::Inside,
                );
            }
            inner.inner
        })
        .inner;
    r
}

/// "<Name> is calling you": the caller's picture pulsing, decline, answer.
fn incoming(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    invite: &PendingInvite,
    actions: &mut Vec<ConvAction>,
) {
    let inv = &invite.invite;
    let name = if inv.from_name.is_empty() {
        world.social.name_of(&inv.from)
    } else {
        inv.from_name.clone()
    };
    let session = inv.session;
    card(ui, p.success.gamma_multiply(0.14), Some(p.success.gamma_multiply(0.4)), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(44.0), egui::Sense::hover());
            let t = ui.input(|i| i.time) as f32;
            let pulse = (t * 2.6).sin() * 0.5 + 0.5;
            ui.painter().circle_filled(
                rect.center(),
                22.0 + pulse * 6.0,
                p.success.gamma_multiply(0.35 * (1.0 - pulse)),
            );
            crate::ui::widgets::avatar_disc(ui, p, rect, pics.get(&inv.from), &name, false);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add_space(4.0);
                let title = aurora_i18n::tr!("ui-call-calls-you", name = name.as_str());
                ui.label(RichText::new(title).size(14.0).strong().color(p.ink));
                let kind = if inv.p2p {
                    aurora_i18n::tr!("ui-chat-call-private")
                } else {
                    aurora_i18n::tr!("ui-chat-call-group")
                };
                ui.label(RichText::new(kind).size(12.0).color(p.muted));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let answer = aurora_i18n::tr!("ui-call-answer");
                if pill(ui, p, "phone-filled", answer, p.success, egui::vec2(56.0, BUTTON)).clicked() {
                    actions.push(ConvAction::AnswerCall {
                        session,
                        answer: InviteAnswer::Accept,
                    });
                }
                let decline = aurora_i18n::tr!("ui-notifications-decline");
                if pill(ui, p, "phone-off", decline, p.danger, egui::vec2(56.0, BUTTON)).clicked() {
                    actions.push(ConvAction::AnswerCall {
                        session,
                        answer: InviteAnswer::Decline,
                    });
                }
            });
        });
    });
}

/// The stage of the call in progress.
fn stage(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    call: &Call,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
) {
    let conversation = call.conversation();
    let everyone = people(world, call);
    card(ui, p.navy.gamma_multiply(0.75), Some(p.raised.gamma_multiply(0.5)), |ui| {
        // ---- the state, the time, the conversation, the count; reduce
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let live = call.state == CallState::Connected;
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 14.0), egui::Sense::hover());
            let color = if live { p.success } else { p.amber };
            ui.painter().circle_filled(dot.center(), 5.0, color.gamma_multiply(0.25));
            ui.painter().circle_filled(dot.center(), 3.5, color);
            ui.label(RichText::new(state_text(call)).size(12.0).strong().color(color));
            let mut rest = Vec::new();
            if let Some(t) = elapsed(call) {
                rest.push(t);
            }
            rest.push(world.session_title(&conversation));
            if !call.p2p() {
                let n = call.members.len().max(1);
                rest.push(aurora_i18n::tr!("ui-call-in-call", n = n.to_string()));
            }
            let room = ui.available_width() - SMALL - 8.0;
            let text = RichText::new(rest.join(" · ")).size(12.0).color(p.muted);
            ui.add(crate::ui::widgets::elided_label(ui, text, room));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if flat_icon(ui, p, "arrows-minimize", aurora_i18n::tr!("ui-chat-call-reduce")).clicked() {
                    st.stage_mini = true;
                }
            });
        });
        // ---- the people, centred, as many rows as needed
        ui.add_space(10.0);
        let width = ui.available_width();
        let per_row = ((width + 12.0) / (PERSON_W + 12.0)).floor().max(1.0) as usize;
        for row in everyone.chunks(per_row) {
            let row_w = row.len() as f32 * PERSON_W + (row.len().saturating_sub(1)) as f32 * 12.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                ui.add_space(((width - row_w) * 0.5).max(0.0));
                for person in row {
                    person_tile(ui, p, pics, world, st, person, actions);
                }
            });
            ui.add_space(4.0);
        }
        if everyone.is_empty() {
            let alone = RichText::new(aurora_i18n::tr!("ui-chat-call-alone")).size(12.0).color(p.muted_dim);
            ui.vertical_centered(|ui| ui.label(alone));
        }
        // ---- the buttons, centred
        ui.add_space(8.0);
        let group = matches!(call.kind, CallKind::Group);
        let count = 1 + usize::from(st.mic.is_some()) + usize::from(!group);
        let buttons_w = count as f32 * BUTTON + 56.0 + count as f32 * 10.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add_space(((width - buttons_w) * 0.5).max(0.0));
            buttons(ui, p, call, st, actions, BUTTON, !group);
        });
    });
}

/// The microphone, the audio output, adding people (`add`), hanging up.
fn buttons(ui: &mut egui::Ui, p: &Palette, call: &Call, st: &mut ChatUi, actions: &mut Vec<ConvAction>, side: f32, add: bool) {
    if let Some(on) = st.mic {
        let (icon, tip) = if on {
            ("microphone-filled", aurora_i18n::tr!("ui-call-mute"))
        } else {
            ("microphone-off", aurora_i18n::tr!("ui-call-unmute"))
        };
        if round(ui, p, icon, tip, side, !on).clicked() {
            actions.push(ConvAction::ToggleMic);
        }
    }
    let output = round(ui, p, "headphones-filled", aurora_i18n::tr!("ui-options-audio-output"), side, false);
    if output.clicked() {
        actions.push(ConvAction::ListOutputDevices);
    }
    output_menu(&output, p, st, actions);
    if add && round(ui, p, "user-plus", aurora_i18n::tr!("ui-call-add-people"), side, false).clicked() {
        actions.push(ConvAction::PickPeople(call.conversation()));
    }
    let hang_up = aurora_i18n::tr!("ui-call-hang-up");
    if pill(ui, p, "phone-off", hang_up, p.danger, egui::vec2(side * 1.4, side)).clicked() {
        actions.push(ConvAction::HangUp);
    }
}

/// The audio outputs, the one in use ticked ("" is the system's).
fn output_menu(resp: &egui::Response, p: &Palette, st: &ChatUi, actions: &mut Vec<ConvAction>) {
    menu::dropdown(resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(220.0);
        let current = st.output_device.as_str();
        if menu::check(
            ui,
            p,
            "headphones-filled",
            aurora_i18n::tr!("ui-options-system-default"),
            current.is_empty(),
            true,
        ) {
            actions.push(ConvAction::SetOutputDevice(String::new()));
        }
        for device in &st.output_devices {
            if menu::check(ui, p, "headphones-filled", device, current == device, true) {
                actions.push(ConvAction::SetOutputDevice(device.clone()));
            }
        }
    });
}

/// Someone of the stage: their picture (a green ring while they speak, a
/// red badge when a moderator muted them, dimmed and pulsing while rung)
/// and their name; a click gives their volume, mute and profile.
fn person_tile(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    person: &Person,
    actions: &mut Vec<ConvAction>,
) {
    st.wanted_pics.insert(person.id);
    let name = world.social.name_of(&person.id);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(PERSON_W, PERSON + 22.0), egui::Sense::click());
    let pic = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.top() + 3.0 + PERSON * 0.5), egui::Vec2::splat(PERSON));
    let painter = ui.painter();
    let volume = st.volumes.get(&person.id).copied().unwrap_or(0.5);
    if person.speaking {
        painter.circle_filled(pic.center(), PERSON * 0.5 + 7.0, p.success.gamma_multiply(0.18));
        painter.circle_stroke(pic.center(), PERSON * 0.5 + 3.0, egui::Stroke::new(2.0, p.success));
    } else if person.waiting {
        let t = ui.input(|i| i.time) as f32;
        let pulse = (t * 2.2).sin() * 0.5 + 0.5;
        painter.circle_stroke(
            pic.center(),
            PERSON * 0.5 + 3.0 + pulse * 3.0,
            egui::Stroke::new(2.0, p.raised.gamma_multiply(1.0 - pulse * 0.6)),
        );
    }
    crate::ui::widgets::avatar_disc(ui, p, pic, pics.get(&person.id), &name, person.waiting);
    let badge = |icon: &str, fill: Color32| {
        let c = egui::pos2(pic.right() - 6.0, pic.bottom() - 6.0);
        painter.circle_filled(c, 10.0, p.navy);
        painter.circle_filled(c, 8.0, fill);
        super::thread::paint_icon(painter, icon, egui::Rect::from_center_size(c, egui::Vec2::splat(11.0)), p.on_accent);
    };
    if person.muted {
        badge("microphone-off", p.danger);
    } else if volume <= 0.001 {
        badge("volume-off", p.muted_dim);
    }
    let label = if person.waiting {
        aurora_i18n::tr!("ui-chat-call-ringing-person", name = name.as_str()).to_owned()
    } else {
        name.clone()
    };
    let color = if person.speaking { p.ink } else { p.muted };
    crate::ui::widgets::paint_elided(
        ui,
        &label,
        egui::pos2(rect.left() + 2.0, pic.bottom() + 11.0),
        PERSON_W - 4.0,
        egui::FontId::proportional(11.0),
        color,
    );
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("ui-call-volume-of", name = name.as_str()));
    person_menu(&resp, p, &name, person.id, volume, actions);
}

/// A participant's volume (0–200 %: the 0..1 of LLSpeakerVolumeStorage,
/// 0.5 as they send it), muted for us (volume 0), their profile.
fn person_menu(resp: &egui::Response, p: &Palette, name: &str, id: Uuid, volume: f32, actions: &mut Vec<ConvAction>) {
    menu::dropdown(resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(210.0);
        let title = aurora_i18n::tr!("ui-call-volume-of", name = name);
        ui.label(RichText::new(title).size(11.5).color(p.muted));
        let mut v = volume;
        ui.add(egui::Slider::new(&mut v, 0.0..=1.0).show_value(false));
        ui.horizontal(|ui| {
            ui.label(RichText::new("0 %").size(11.0).color(p.muted_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("200 %").size(11.0).color(p.muted_dim));
                ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                    let percent = format!("{} %", (v * 200.0).round() as i32);
                    ui.label(RichText::new(percent).size(11.5).strong().color(p.ink));
                });
            });
        });
        if (v - volume).abs() > f32::EPSILON {
            actions.push(ConvAction::SetVolume(id, v));
        }
        let muted = volume <= 0.001;
        if menu::check(ui, p, "volume-off", aurora_i18n::tr!("ui-chat-call-mute-for-me"), muted, true) {
            actions.push(ConvAction::SetVolume(id, if muted { 0.5 } else { 0.0 }));
            ui.close();
        }
        if menu::item_if(
            ui,
            p,
            "restore",
            aurora_i18n::tr!("ui-call-volume-reset"),
            (volume - 0.5).abs() > 0.001,
        ) {
            actions.push(ConvAction::SetVolume(id, 0.5));
        }
        if menu::item(ui, p, "user-filled", aurora_i18n::tr!("common-profile")) {
            actions.push(ConvAction::Profile(id));
            ui.close();
        }
    });
}

/// The stage reduced to a line: the state and time, who speaks, their
/// pictures, the microphone, expand, hang up.
fn mini(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    call: &Call,
    st: &mut ChatUi,
    actions: &mut Vec<ConvAction>,
) {
    let everyone = people(world, call);
    card(ui, p.success.gamma_multiply(0.12), Some(p.success.gamma_multiply(0.35)), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            crate::ui::widgets::glyph(ui, "phone-filled", p.success, 17.0);
            let mut text = state_text(call).to_owned();
            if let Some(t) = elapsed(call) {
                text = format!("{text} · {t}");
            }
            ui.label(RichText::new(text).size(12.0).strong().color(p.success));
            let speakers: Vec<String> = everyone
                .iter()
                .filter(|m| m.speaking)
                .map(|m| first_name(&world.social.name_of(&m.id)))
                .collect();
            let said = match speakers.as_slice() {
                [] => world.session_title(&call.conversation()),
                [one] => aurora_i18n::tr!("ui-chat-call-speaks-one", name = one.as_str()).to_owned(),
                [a, b] => aurora_i18n::tr!("ui-chat-call-speaks-two", a = a.as_str(), b = b.as_str()).to_owned(),
                _ => aurora_i18n::tr!("ui-chat-call-speaks-many").to_owned(),
            };
            let shown = everyone.len().min(4);
            let stack_w = if shown > 0 { 22.0 + (shown - 1) as f32 * 16.0 } else { 0.0 };
            let buttons_w = SMALL * 2.0 + SMALL * 1.4 + 24.0 + if st.mic.is_some() { 0.0 } else { -SMALL - 8.0 };
            let room = (ui.available_width() - stack_w - buttons_w - 16.0).max(30.0);
            let label = RichText::new(format!("· {said}")).size(12.0).color(p.muted);
            ui.add(crate::ui::widgets::elided_label(ui, label, room));
            // the pictures, overlapping
            let (stack, _) = ui.allocate_exact_size(egui::vec2(stack_w, 22.0), egui::Sense::hover());
            for (i, m) in everyone.iter().take(shown).enumerate() {
                st.wanted_pics.insert(m.id);
                let r = egui::Rect::from_min_size(egui::pos2(stack.left() + i as f32 * 16.0, stack.top()), egui::Vec2::splat(22.0));
                let ring = if m.speaking { p.success } else { p.panel };
                ui.painter().circle_filled(r.center(), 12.5, ring);
                let name = world.social.name_of(&m.id);
                crate::ui::widgets::avatar_disc(ui, p, r, pics.get(&m.id), &name, m.waiting);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let hang_up = aurora_i18n::tr!("ui-call-hang-up");
                if pill(ui, p, "phone-off", hang_up, p.danger, egui::vec2(SMALL * 1.4, SMALL)).clicked() {
                    actions.push(ConvAction::HangUp);
                }
                if round(ui, p, "arrows-maximize", aurora_i18n::tr!("ui-chat-call-expand"), SMALL, false).clicked() {
                    st.stage_mini = false;
                }
                if let Some(on) = st.mic {
                    let (icon, tip) = if on {
                        ("microphone-filled", aurora_i18n::tr!("ui-call-mute"))
                    } else {
                        ("microphone-off", aurora_i18n::tr!("ui-call-unmute"))
                    };
                    if round(ui, p, icon, tip, SMALL, !on).clicked() {
                        actions.push(ConvAction::ToggleMic);
                    }
                }
            });
        });
    });
}

/// A call of the group or conference that goes on without us: who started
/// it and when, and Join.
#[allow(clippy::too_many_arguments)]
fn ongoing(
    ui: &mut egui::Ui,
    p: &Palette,
    pics: &HashMap<Uuid, egui::TextureHandle>,
    world: &World,
    st: &mut ChatUi,
    conversation: Uuid,
    by: Uuid,
    since: std::time::Instant,
    actions: &mut Vec<ConvAction>,
) {
    card(ui, p.success.gamma_multiply(0.12), Some(p.success.gamma_multiply(0.35)), |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            crate::ui::widgets::glyph(ui, "volume", p.success, 17.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-chat-call-ongoing"))
                    .size(12.0)
                    .strong()
                    .color(p.ink),
            );
            let name = world.social.name_of(&by);
            let minutes = since.elapsed().as_secs() / 60;
            let when = if minutes == 0 {
                aurora_i18n::tr!("ui-chat-call-started-now", name = name.as_str()).to_owned()
            } else {
                aurora_i18n::tr!("ui-chat-call-started-ago", name = name.as_str(), n = minutes.to_string()).to_owned()
            };
            let join_w = 90.0;
            let room = (ui.available_width() - join_w - 30.0).max(30.0);
            ui.add(crate::ui::widgets::elided_label(ui, RichText::new(format!("· {when}")).size(12.0).color(p.muted), room));
            st.wanted_pics.insert(by);
            let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
            crate::ui::widgets::avatar_disc(ui, p, r, pics.get(&by), &name, false);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let join = aurora_i18n::tr!("ui-chat-join-call");
                let enabled = st.voice_on;
                let resp = crate::ui::widgets::action_button_if(
                    ui,
                    p,
                    "phone-filled",
                    join,
                    crate::ui::widgets::ActionKind::Primary,
                    egui::vec2(0.0, 26.0),
                    enabled,
                );
                let resp = if enabled {
                    resp
                } else {
                    resp.on_disabled_hover_text(aurora_i18n::tr!("ui-call-voice-off"))
                };
                if resp.clicked() {
                    actions.push(ConvAction::Call(conversation));
                }
            });
        });
    });
}

fn first_name(name: &str) -> String {
    name.split_whitespace().next().unwrap_or(name).to_owned()
}

/// A round button of a call (`off`: the red tint of a muted microphone).
fn round(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, side: f32, off: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(side), egui::Sense::click());
    let face = match (off, resp.hovered()) {
        (true, false) => p.danger.gamma_multiply(0.2),
        (true, true) => p.danger.gamma_multiply(0.3),
        (false, false) => p.raised,
        (false, true) => p.raised.gamma_multiply(1.35),
    };
    ui.painter().circle_filled(rect.center(), side * 0.5, face);
    super::thread::paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(side * 0.47)),
        if off { p.danger } else { p.ink },
    );
    resp.on_hover_text(tip)
}

/// A pill of a call in a strong colour: answer, decline, hang up.
fn pill(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, color: Color32, size: egui::Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let face = if resp.hovered() { color } else { color.gamma_multiply(0.88) };
    ui.painter().rect_filled(rect, rect.height() * 0.5, face);
    super::thread::paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(size.y * 0.47)),
        p.on_accent,
    );
    resp.on_hover_text(tip)
}

/// A flat icon button of a card's top line.
fn flat_icon(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    super::thread::paint_icon(
        ui.painter(),
        icon,
        egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(16.0)),
        if resp.hovered() { p.ink } else { p.muted },
    );
    resp.on_hover_text(tip)
}
