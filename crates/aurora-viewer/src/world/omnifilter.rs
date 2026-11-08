//! Where the Omnifilter (`crate::omnifilter`) meets what arrives, as the
//! Legacy Viewer's callers fill its haystack and act on a match:
//! - nearby chat (process_chat_from_simulator, llviewermessage.cpp):
//!   said, whispered, shouted, debug, region, owner and direct lines; a
//!   resident's is NearbyChat, an object's ObjectChat; the sender is the
//!   name the simulator sends, the owner key the speaker's id; a match
//!   also ends the speaker's "is typing";
//! - instant messages (LLIMProcessing::processNewMessage,
//!   llimprocessing.cpp), each dialog as its kind (group chat, object IM,
//!   offers, invitations, notices…), the others as no kind; and the first
//!   message of a group or conference chat (LLIMMgr::addMessage,
//!   IM_SESSION_INVITE, llimview.cpp);
//! - a matched line or IM is dropped, or its text replaced by
//!   "Omnifilter Replacement: <text>" when the rule has one;
//! - script dialogs and text boxes (LLScriptFloaterManager::
//!   onAddNotification, llscriptfloater.cpp) and llLoadURL (LoadWebPage):
//!   a matched one is never shown and is answered as if the button named
//!   by the rule had been clicked (callback_script_dialog,
//!   callback_load_url): a script's button sends its ScriptDialogReply,
//!   `Client_Side_Mute` blocks the object, `Gotopage` opens the link, any
//!   other name answers nothing; a text box always sends the rule's reply,
//!   empty or not. The answer waits the rule's delay (up to 3 s,
//!   FIRE-36590).
//!
//! Nothing is sent unless the user wrote a rule that answers: the filter is
//! off by default and a new rule is disabled.

use super::World;
use crate::omnifilter::{Haystack, Source};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetCommand};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The script dialog form's own buttons, before the script's
/// (notifications.xml, ScriptDialog), with the names a rule's button reply
/// uses.
const MUTE_BUTTON: &str = "Client_Side_Mute";
const IGNORE_BUTTON: &str = "Client_Side_Ignore";
/// LoadWebPage's "Go to page" button.
const GO_TO_PAGE: &str = "Gotopage";
/// The only button of an llTextBox (TEXTBOX_MAGIC_TOKEN).
const TEXTBOX: &str = "!!llTextBox!!";

/// An answer to a matched dialog, waiting for its delay.
#[derive(Debug, Clone)]
pub enum Reply {
    Send(NetCommand),
    /// LLMuteList::add of the dialog's object.
    Block {
        object: Uuid,
        name: String,
    },
    OpenUrl(String),
}

/// The kind of an IM dialog (processNewMessage's switch).
fn im_source(dialog: u8) -> Option<Source> {
    use super::groups::dialog as g;
    use super::notifications::im;
    match dialog {
        0 => Some(Source::InstantMessage),
        g::SESSION_SEND | g::SESSION_INVITE | 20 => Some(Source::GroupChat),
        19 => Some(Source::ObjectInstantMessage),
        im::GROUP_INVITATION => Some(Source::GroupInvite),
        im::INVENTORY_OFFERED => Some(Source::InventoryOffer),
        im::FRIENDSHIP_OFFERED => Some(Source::FriendshipOffer),
        im::LURE_USER => Some(Source::Lure),
        im::TELEPORT_REQUEST => Some(Source::TeleportRequest),
        im::GROUP_NOTICE => Some(Source::GroupNotice),
        _ => None,
    }
}

/// The kind of a chat line, or None for a line the filter does not see
/// (typing starts and stops).
fn chat_source(c: &ChatMessage) -> Option<Option<Source>> {
    let filtered = matches!(
        c.chat_type,
        ChatType::Whisper
            | ChatType::Normal
            | ChatType::Shout
            | ChatType::Debug
            | ChatType::Region
            | ChatType::OwnerSay
            | ChatType::Other(9)
    );
    filtered.then_some(match c.source_type {
        ChatSourceType::Agent => Some(Source::NearbyChat),
        ChatSourceType::Object => Some(Source::ObjectChat),
        _ => None,
    })
}

/// OmnifilterReplacement.
fn replacement(text: &str) -> String {
    aurora_i18n::tr!("omnifilter-replacement", replacement = text.to_owned()).to_string()
}

/// What a script dialog's notification says (the English ScriptDialog
/// template, "[NAME]'s '<nolink>[TITLE]</nolink>'\n[MESSAGE]"), with the
/// name of each button of its form (FIRE-36647), the Block and Ignore
/// buttons first.
pub fn dialog_content(owner: &str, object: &str, message: &str, buttons: &[String]) -> String {
    let mut content = format!("{owner}'s '<nolink>{object}</nolink>'\n{message}");
    for b in [MUTE_BUTTON, IGNORE_BUTTON].into_iter().chain(buttons.iter().map(String::as_str)) {
        content.push_str("\nbutton_name=");
        content.push_str(b);
    }
    content
}

/// What a LoadWebPage notification says (its English template).
pub fn load_url_content(url: &str, message: &str, object: &str, owner: Uuid, owner_is_group: bool) -> String {
    let kind = if owner_is_group { "group" } else { "agent" };
    format!("Load web page {url} ?\n\n{message}\n\nFrom object: <nolink>{object}</nolink>, owner: secondlife:///app/{kind}/{owner}/about")
}

/// The answer of a matched script dialog (callback_script_dialog), if any.
pub fn dialog_reply(
    m: &crate::omnifilter::Match,
    sender: std::net::SocketAddr,
    object: Uuid,
    object_name: &str,
    channel: i32,
    buttons: &[String],
) -> Option<Reply> {
    let reply = |index: usize, label: String| {
        Reply::Send(NetCommand::ScriptDialogReply {
            sender: Some(sender),
            object_id: object,
            channel,
            index: index as i32,
            label,
        })
    };
    if buttons.first().is_some_and(|b| b == TEXTBOX) {
        // the answer goes through the same clean-up and cut as one typed
        // in the prompt (a rule from a file may hold CRs or be too long)
        let mut text = m.textbox_reply.clone();
        super::notifications::clamp_text_box_reply(&mut text);
        return Some(reply(0, text));
    }
    if m.button_reply == MUTE_BUTTON {
        return Some(Reply::Block {
            object,
            name: object_name.to_owned(),
        });
    }
    if m.button_reply.is_empty() {
        return None;
    }
    buttons
        .iter()
        .position(|b| *b == m.button_reply)
        .map(|i| reply(i, m.button_reply.clone()))
}

impl World {
    /// A line of nearby chat: false when the filter drops it; its text may
    /// be replaced.
    pub(super) fn omnifilter_chat(&mut self, c: &mut ChatMessage) -> bool {
        if !self.omnifilter.enabled {
            return true;
        }
        let Some(source) = chat_source(c) else {
            return true;
        };
        let h = Haystack {
            source,
            sender_name: &c.from_name,
            content: &c.message,
            region_name: "",
            owner_id: c.source_id,
        };
        let Some(m) = self.omnifilter.matches(&h) else {
            return true;
        };
        // the speaker stops typing (LLVOAvatar::stopTyping)
        if c.source_type == ChatSourceType::Agent {
            self.nearby_typing.remove(&c.source_id);
        }
        if m.chat_replace.is_empty() {
            return false;
        }
        c.message = replacement(&m.chat_replace);
        true
    }

    /// An instant message of any dialog: false when the filter drops it;
    /// its text may be replaced.
    pub(super) fn omnifilter_im(&mut self, im: &mut InstantMessage) -> bool {
        if !self.omnifilter.enabled {
            return true;
        }
        let h = Haystack {
            source: im_source(im.dialog),
            sender_name: &im.from_name,
            content: &im.message,
            region_name: "",
            owner_id: im.from_agent_id,
        };
        let Some(m) = self.omnifilter.matches(&h) else {
            return true;
        };
        if m.chat_replace.is_empty() {
            return false;
        }
        im.message = replacement(&m.chat_replace);
        true
    }

    /// The first message of a group or conference chat
    /// (ChatterBoxInvitation): false when the filter drops it.
    pub(super) fn omnifilter_session_invite(&mut self, inv: &mut aurora_net::SessionInvite) -> bool {
        if !self.omnifilter.enabled {
            return true;
        }
        let h = Haystack {
            source: Some(Source::GroupChat),
            sender_name: &inv.from_name,
            content: &inv.message,
            region_name: "",
            owner_id: inv.from,
        };
        let Some(m) = self.omnifilter.matches(&h) else {
            return true;
        };
        if m.chat_replace.is_empty() {
            return false;
        }
        inv.message = replacement(&m.chat_replace);
        true
    }

    /// A script dialog or text box: true when a rule took it (it is not
    /// shown; its answer, if any, is on its way).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn omnifilter_script_dialog(
        &mut self,
        sender: std::net::SocketAddr,
        object: Uuid,
        object_name: &str,
        owner_name: &str,
        owner_id: Uuid,
        message: &str,
        channel: i32,
        buttons: &[String],
    ) -> bool {
        if !self.omnifilter.enabled {
            return false;
        }
        // [NAME]: LLCacheName::buildFullName, "Resident" left out as set
        let owner = if self.social.avatar_names.options.trim_resident {
            owner_name.strip_suffix(" Resident").unwrap_or(owner_name)
        } else {
            owner_name
        };
        let content = dialog_content(owner, object_name, message, buttons);
        let h = Haystack {
            source: Some(Source::ScriptDialog),
            sender_name: object_name,
            content: &content,
            region_name: "",
            owner_id,
        };
        let Some(m) = self.omnifilter.matches(&h) else {
            return false;
        };
        if let Some(reply) = dialog_reply(&m, sender, object, object_name, channel, buttons) {
            self.omnifilter_answer(reply, m.reply_delay);
        }
        true
    }

    /// llLoadURL: true when a rule took it.
    pub(super) fn omnifilter_load_url(
        &mut self,
        object_name: &str,
        owner_id: Uuid,
        owner_is_group: bool,
        message: &str,
        url: &str,
    ) -> bool {
        if !self.omnifilter.enabled {
            return false;
        }
        let content = load_url_content(url, message, object_name, owner_id, owner_is_group);
        let h = Haystack {
            source: Some(Source::UrlRequest),
            sender_name: object_name,
            content: &content,
            region_name: "",
            owner_id,
        };
        let Some(m) = self.omnifilter.matches(&h) else {
            return false;
        };
        if m.button_reply == GO_TO_PAGE {
            self.omnifilter_answer(Reply::OpenUrl(url.to_owned()), m.reply_delay);
        }
        true
    }

    /// Now, or after the rule's delay (createDelayedResponse).
    fn omnifilter_answer(&mut self, reply: Reply, delay: f32) {
        if delay > 0.0 {
            let due = Instant::now() + Duration::from_secs_f32(delay);
            self.omnifilter_replies.push((due, reply));
        } else {
            self.omnifilter_send(reply);
        }
    }

    fn omnifilter_send(&mut self, reply: Reply) {
        match reply {
            Reply::Send(cmd) => {
                log::info!("omnifilter: script dialog answered");
                self.groups.out.push(cmd);
            }
            Reply::Block { object, name } => {
                if let Err(error) = self.block(object, &name, super::mutes::MuteType::Object) {
                    self.system_message(error);
                }
            }
            Reply::OpenUrl(url) => self.omnifilter_urls.push(url),
        }
    }

    /// Once a frame: the rules' file and settings, and the answers whose
    /// delay is over (sendDelayedResponse).
    pub fn omnifilter_frame(&mut self, settings: crate::omnifilter::Settings, now: Instant) {
        self.omnifilter.enabled = settings.enabled;
        self.omnifilter.select(settings.rule_set);
        self.omnifilter.update(now);
        self.omnifilter_flush(now);
    }

    fn omnifilter_flush(&mut self, now: Instant) {
        if self.omnifilter_replies.is_empty() {
            return;
        }
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.omnifilter_replies)
            .into_iter()
            .partition(|(at, _)| *at <= now);
        self.omnifilter_replies = waiting;
        for (_, reply) in due {
            self.omnifilter_send(reply);
        }
    }

    /// Links a rule asked to open ("Go to page"), for the app.
    pub fn take_omnifilter_urls(&mut self) -> Vec<String> {
        std::mem::take(&mut self.omnifilter_urls)
    }
}

#[cfg(test)]
mod tests;
