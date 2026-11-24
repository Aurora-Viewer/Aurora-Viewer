//! Phase 1 cases of the chat and IM rules that follow the Legacy Viewer's
//! RLVa (indra/newview/rlvhandler.cpp, rlvactions.cpp, fsnearbychathub.cpp):
//! each rule is issued through the real controller and read from the same
//! predicates the send boundary uses. Nothing is said, sent or shown: the
//! lines and requests below are probes, never sent.
use super::cases::{can, holds, test};
use super::{Case, Ctx, Step, Test, Who, World};
use crate::world::rlva::legacy_strings::{Remote, remote};
use aurora_net::{ChatType, NetCommand};
use uuid::Uuid;

fn case(id: &'static str, title: &'static str, steps: Vec<Step>) -> Case {
    Case {
        id,
        title,
        phase: 1,
        gate: false,
        known: None,
        steps,
    }
}

/// What a line typed in the nearby chat would become: (channel, volume,
/// text) of each chat packet, in order.
fn said(w: &World, text: &str, kind: ChatType) -> Vec<(i32, ChatType, String)> {
    w.rlva
        .outgoing_chat(text, 0, kind)
        .into_iter()
        .filter_map(|command| match command {
            NetCommand::Chat {
                message,
                channel,
                chat_type,
            } => Some((channel, chat_type, message)),
            // a redirected line: a shout on the object's channel
            NetCommand::ChatReply { channel, message } => Some((channel, ChatType::Shout, message)),
            _ => None,
        })
        .collect()
}

/// The volume a line typed at `kind` leaves with, if it leaves on channel 0.
fn volume(w: &World, kind: ChatType) -> Option<ChatType> {
    match &said(w, "hello", kind)[..] {
        [(0, sent, _)] => Some(*sent),
        _ => None,
    }
}

/// The message a teleport request to the made-up resident would carry.
fn request_text(w: &World, c: &Ctx) -> String {
    let mut command = NetCommand::SendImDialog {
        to: c.other,
        dialog: crate::world::notifications::im::TELEPORT_REQUEST,
        id: Uuid::nil(),
        message: "self-test request".into(),
        bucket: Vec::new(),
    };
    w.rlva_filter_lure(&mut command);
    match command {
        NetCommand::SendImDialog { message, .. } => message,
        _ => String::new(),
    }
}

fn redirected_to(w: &World, c: &Ctx, text: &str) -> usize {
    said(w, text, ChatType::Normal)
        .iter()
        .filter(|(channel, ..)| *channel == c.channel)
        .count()
}

/// `text` said in the nearby chat reaches the test's channel `copies` times.
fn count(text: &'static str, copies: usize) -> Test {
    test(move |w, c| match redirected_to(w, c, text) {
        n if n == copies => Ok(()),
        n => Err(format!("{n} copies")),
    })
}

pub(super) fn cases() -> Vec<Case> {
    vec![
        case(
            "chat-G1-teleport-request-hidden",
            "@sendim hides a teleport request's message",
            vec![
                Step::Now(
                    "the request keeps its message",
                    holds(|w, c| request_text(w, c) == "self-test request", "already hidden"),
                ),
                Step::Say(Who::Controller, "@sendim=n"),
                Step::Expect(
                    "the message is hidden",
                    holds(|w, c| request_text(w, c) == remote(Remote::HiddenGeneric), "still sent as written"),
                ),
                Step::Say(Who::Controller, "@sendim=y"),
                Step::Expect(
                    "the message is kept again",
                    holds(|w, c| request_text(w, c) == "self-test request", "still hidden"),
                ),
            ],
        ),
        case(
            "chat-G3-volume-two-passes",
            "@chatnormal with @chatwhisper: the volume rules applied twice",
            vec![
                Step::Say(Who::Controller, "@chatnormal=n,chatwhisper=n"),
                Step::Expect(
                    "whisper stays whisper, normal and shout leave normal",
                    holds(
                        |w, _| {
                            volume(w, ChatType::Whisper) == Some(ChatType::Whisper)
                                && volume(w, ChatType::Normal) == Some(ChatType::Normal)
                                && volume(w, ChatType::Shout) == Some(ChatType::Normal)
                        },
                        "another volume",
                    ),
                ),
                Step::Say(Who::Controller, "@chatnormal=y,chatwhisper=y"),
                Step::Expect(
                    "every volume as typed",
                    holds(|w, _| volume(w, ChatType::Whisper) == Some(ChatType::Whisper), "still changed"),
                ),
            ],
        ),
        case(
            "chat-G7-redirect-per-object",
            "@redirchat to one channel from two objects: one copy each",
            vec![
                Step::Say(Who::Controller, "@redirchat:{ch}=n"),
                Step::Say(Who::Second, "@redirchat:{ch}=n"),
                Step::Expect("two copies on the channel", count("hello", 2)),
                Step::Say(Who::Second, "@redirchat:{ch}=y"),
                Step::Expect("one copy", count("hello", 1)),
                Step::Say(Who::Controller, "@redirchat:{ch}=y"),
            ],
        ),
        case(
            "chat-G10-bare-me",
            "\"/me \" alone is not an emote",
            vec![
                Step::Say(Who::Controller, "@rediremote:{ch}=n"),
                Step::Expect(
                    "an emote is redirected",
                    holds(|w, c| redirected_to(w, c, "/me waves") == 1, "not redirected"),
                ),
                Step::Now(
                    "\"/me \" leaves on channel 0",
                    holds(
                        |w, c| redirected_to(w, c, "/me ") == 0 && said(w, "/me ", ChatType::Normal).iter().any(|(ch, ..)| *ch == 0),
                        "taken for an emote",
                    ),
                ),
                Step::Say(Who::Controller, "@rediremote:{ch}=y"),
            ],
        ),
        case(
            "chat-G11-emote-cut",
            "@sendchat cuts an emote where the Legacy Viewer does",
            vec![
                Step::Say(Who::Controller, "@sendchat=n"),
                Step::Expect(
                    "\"/me ééé. waves\" leaves as \"/me ééé. wa\"",
                    holds(
                        |w, _| matches!(&said(w, "/me ééé. waves", ChatType::Normal)[..], [(0, _, text)] if text == "/me ééé. wa"),
                        "another cut",
                    ),
                ),
                Step::Say(Who::Controller, "@sendchat=y"),
            ],
        ),
        case(
            "chat-G12-filtered-line-still-sent",
            "a line @sendchat filters still leaves",
            vec![
                Step::Say(Who::Controller, "@sendchat=n"),
                Step::Expect(
                    "one line on channel 0, \"...\" or empty",
                    holds(
                        |w, _| matches!(&said(w, "hello", ChatType::Normal)[..], [(0, _, text)] if text == "..." || text.is_empty()),
                        "nothing or the text itself",
                    ),
                ),
                Step::Say(Who::Controller, "@sendchat=y"),
            ],
        ),
        case(
            "chat-G15-typing-animation",
            "@redirchat holds back the typing start, not the animation",
            vec![
                Step::Say(Who::Controller, "@redirchat:{ch}=n"),
                Step::Expect(
                    "a typing animation played by other means passes",
                    holds(
                        |w, _| {
                            can(
                                w,
                                &NetCommand::AgentAnimation {
                                    anim: crate::world::radar::ANIM_AGENT_TYPE,
                                    start: true,
                                },
                            )
                        },
                        "refused",
                    ),
                ),
                Step::Say(Who::Controller, "@redirchat:{ch}=y"),
            ],
        ),
        case(
            "chat-G23-strict-from",
            "@permissive=n with a bare @recvimfrom stops another object's @recvimfrom",
            vec![
                Step::Say(Who::Controller, "@recvimfrom:{other}=n"),
                Step::Expect(
                    "IMs from the resident hidden",
                    holds(|w, c| !w.rlva_allows_im("recvim", c.other), "still received"),
                ),
                Step::Say(Who::Second, "@recvimfrom=n,permissive=n"),
                Step::Expect(
                    "the second object does not name the resident: received again",
                    holds(|w, c| w.rlva_allows_im("recvim", c.other), "still hidden"),
                ),
                Step::Say(Who::Second, "@recvimfrom=y,permissive=y"),
                Step::Expect("hidden again", holds(|w, c| !w.rlva_allows_im("recvim", c.other), "still received")),
                Step::Say(Who::Controller, "@recvimfrom:{other}=y"),
            ],
        ),
    ]
}
