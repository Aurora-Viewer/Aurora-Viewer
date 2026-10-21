//! What asks for the user's attention while Aurora is in the background:
//! the taskbar flash of the messages (LLIMProcessing::processNewMessage →
//! LLWindow::flashIcon, FSFlashOnMessage / FSFlashOnObjectIM /
//! FSFlashOnScriptDialog) and the system notifications of the Legacy
//! Viewer's Growl manager (growlmanager.cpp, growl_notifications.xml),
//! originally LGPL 2.1. The world only records what happened; the app
//! decides from the settings, the focus and do not disturb
//! (`app/attention.rs`).

use super::{ChatKind, World};
use crate::ui_sound::{ImKind, ImMessage};
use uuid::Uuid;

/// Why the attention is asked: it picks the setting of the flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// An instant message, a group chat or conference line, an offer or a
    /// notice sent as an IM (FSFlashOnMessage).
    Message,
    /// An object's llInstantMessage (FSFlashOnObjectIM, under FSFlashOnMessage).
    ObjectIm,
    /// A script dialog or text box (FSFlashOnScriptDialog).
    ScriptDialog,
    /// A line that mentions the agent (secondlife:///app/agent/<id>/mention).
    /// The Legacy Viewer flashes for it only when it is an IM; Aurora
    /// flashes for a mention in nearby chat too, with the message setting.
    Mention,
    /// A line that holds one of the keywords (FSKeywords::notify): its
    /// sound, told to the system, no flash of its own.
    Keyword,
    /// A notice that never flashes (a friend online, a simulator alert, the
    /// region restart, whose alert flashes on its own…).
    Notice,
}

/// What a click on the system notification opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    /// The conversation of this session (a resident, a group, a conference).
    Conversation(Uuid),
    /// The nearby chat.
    NearbyChat,
    /// The list under the bell.
    Notifications,
    /// Only the window (the session has ended).
    Window,
}

/// The text of a system notification (GrowlManager::notify): its title, its
/// body as received (links still in; the app writes them as names).
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub open: Open,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attention {
    pub cause: Cause,
    /// None: a flash only (a group chat line, which the Legacy Viewer's
    /// Growl leaves out).
    pub notice: Option<Notice>,
}

impl World {
    /// Something to flash for and, with `notice`, to tell the system.
    pub(crate) fn ask_attention(&mut self, cause: Cause, notice: Option<Notice>) {
        self.attention.push(Attention { cause, notice });
    }

    /// A conversation message: every one flashes; a resident's IM is told
    /// to the system with its sender and text (GrowlManager::onInstantMessage,
    /// P2P sessions only), a group chat or conference line only when it
    /// mentions the agent (the Legacy Viewer tells them on a keyword alert).
    pub fn message_attention(&mut self, m: &ImMessage) {
        let line = self
            .social
            .ims
            .iter()
            .find(|s| s.other == m.session)
            .and_then(|s| s.lines.back())
            // only what a resident wrote (no system lines, no objects)
            .filter(|l| l.kind == ChatKind::Im && !l.source.is_nil() && l.source != self.agent_id && !self.mutes.is_muted_id(&l.source));
        let open = Open::Conversation(m.session);
        let (cause, notice) = match line {
            Some(l) if m.kind == ImKind::Private => (
                Cause::Message,
                Some(Notice {
                    title: l.from.clone(),
                    body: strip_me(&l.text).to_owned(),
                    open,
                }),
            ),
            Some(l) if crate::ui::chat::mentions(&l.text, &self.agent_id) => (
                Cause::Mention,
                Some(Notice {
                    title: self.session_title(&m.session),
                    body: format!("{}: {}", l.from, strip_me(&l.text)),
                    open,
                }),
            ),
            _ => (Cause::Message, None),
        };
        self.ask_attention(cause, notice);
    }

    /// A line that arrived, if it holds a keyword (FSKeywords::
    /// chatContainsKeyword then notify): the keyword sound and "Keyword
    /// alert" told to the system with the sender and the text. `local`: a
    /// nearby chat line (objects' IMs and the notices shown there
    /// included). Never for a blocked `sender` (FSKeywords::notify); the
    /// callers leave out our own lines and the debug channel.
    pub(crate) fn keyword_alert(&mut self, sender: Uuid, from: &str, text: &str, local: bool, open: Open) {
        if !crate::keywords::matches(from, text, local) || self.mutes.is_muted_id(&sender) {
            return;
        }
        let body = if strip_me(text).len() == text.len() {
            format!("{from}: {text}")
        } else {
            // an emote reads as "Name waves" (FSCommon::is_irc_me_prefix)
            format!("{from}{}", &text[3..])
        };
        let notice = Notice {
            title: aurora_i18n::tr!("world-keyword-alert").to_owned(),
            body,
            open,
        };
        self.ask_attention(Cause::Keyword, Some(notice));
    }

    /// A system notification of the notification just pushed: its title
    /// and body (the Legacy list names it, growl_notifications.xml).
    pub(crate) fn notice_of(&self, id: u64, open: Open) -> Option<Notice> {
        let n = self.notifications.list.iter().find(|n| n.id == id)?;
        Some(Notice {
            title: n.title.clone(),
            body: n.body.clone(),
            open,
        })
    }
}

/// The text of an emote without its "/me" (FSCommon::is_irc_me_prefix).
fn strip_me(text: &str) -> &str {
    match text.get(..3) {
        Some(p) if p.eq_ignore_ascii_case("/me") && text[3..].starts_with([' ', '\'']) => text[3..].trim_start(),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::strip_me;

    #[test]
    fn an_emote_loses_its_me() {
        assert_eq!(strip_me("/me waves"), "waves");
        assert_eq!(strip_me("/me's here"), "'s here");
        assert_eq!(strip_me("/meow"), "/meow");
        assert_eq!(strip_me("hello"), "hello");
        assert_eq!(strip_me("/m"), "/m");
    }
}
