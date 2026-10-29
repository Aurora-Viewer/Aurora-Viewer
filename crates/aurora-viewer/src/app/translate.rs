//! The app's side of the IM translation (`crate::translate`): `/tr`, our
//! messages held for their translation, the received ones handed over, and
//! the effects of finished translations on the conversations and the
//! network.

use super::App;
use crate::translate::{Effect, LineKey, SetResult, TrCommand, TranslateService, models_dir};
use crate::world::notice_topics::Topic;
use aurora_net::NetCommand;
use std::time::Instant;
use uuid::Uuid;

/// Messages already received that are translated when a conversation
/// becomes translated.
const RECENT: usize = 5;

impl App {
    /// `/tr` typed in a private conversation.
    pub(super) fn translate_command(&mut self, session: Uuid, cmd: TrCommand) {
        let notice = match cmd {
            TrCommand::Set(l) => self.translate_set(session, Some(l)),
            TrCommand::Off => self.translate_set(session, None),
            TrCommand::Status => Some(self.translate_status(session)),
            TrCommand::Unknown(word) => Some(aurora_i18n::tr!("translate-unknown", lang = word).to_string()),
        };
        if let Some(text) = notice {
            self.world.conversation_notice(session, text, Topic::Info);
        }
    }

    /// Translate a conversation to `to` (or stop): the notice to show, if
    /// any (the download dialog says the rest).
    pub(super) fn translate_set(&mut self, session: Uuid, to: Option<&'static aurora_translate::Lang>) -> Option<String> {
        let r = self.translate.set_partner(&mut self.settings.translate, session, to, &models_dir());
        match r {
            SetResult::Ready | SetResult::Off => self.settings.save(),
            SetResult::NeedsDownload | SetResult::NotBuilt | SetResult::Disabled => {}
        }
        if r == SetResult::Ready {
            self.translate_recent(session);
        }
        match r {
            SetResult::Ready => Some(self.translate_status(session)),
            SetResult::Off => Some(aurora_i18n::tr!("translate-off").to_string()),
            SetResult::NotBuilt => Some(aurora_i18n::tr!("translate-not-built").to_string()),
            SetResult::Disabled => Some(aurora_i18n::tr!("translate-disabled").to_string()),
            SetResult::NeedsDownload => None,
        }
    }

    /// The conversation just became translated: its last messages too (the
    /// ones that made us turn it on).
    fn translate_recent(&mut self, session: Uuid) {
        for (time, text) in self.world.recent_incoming(session, RECENT) {
            self.translate.on_incoming(&self.settings.translate, session, time, &text);
        }
    }

    fn translate_status(&self, session: Uuid) -> String {
        let s = &self.settings.translate;
        match TranslateService::partner(s, session) {
            Some(theirs) => aurora_i18n::tr!("translate-on", mine = TranslateService::me(s).native, theirs = theirs.native).to_string(),
            None => aurora_i18n::tr!("translate-status-off").to_string(),
        }
    }

    /// Our message in a translated private conversation: shown at once,
    /// sent when its translation is ready. `false`: not translated, the
    /// caller sends it as usual.
    pub(super) fn translate_outgoing(&mut self, to: Uuid, text: &str) -> bool {
        if TranslateService::partner(&self.settings.translate, to).is_none() {
            return false;
        }
        let time = self.world.own_im(to, text);
        let fx = self.translate.submit_outgoing(&self.settings.translate, to, time, text);
        self.apply_translation(fx);
        true
    }

    /// Every frame: the messages received, the finished translations.
    pub(super) fn translate_frame(&mut self) {
        for (session, time, text) in std::mem::take(&mut self.world.incoming_ims) {
            self.translate.on_incoming(&self.settings.translate, session, time, &text);
        }
        let polled = self.translate.poll(&mut self.settings.translate, Instant::now());
        if let Some(session) = polled.now_translated {
            self.settings.save();
            self.translate_recent(session);
        }
        self.apply_translation(polled.effects);
        let seen = (self.translate.version(), self.settings.translate.clone());
        if self.translate_seen.as_ref() != Some(&seen) {
            let view = self.translate.view(&self.settings.translate);
            let old = std::mem::replace(&mut self.world.translate_view, view);
            let v = &self.world.translate_view;
            let sessions: Vec<Uuid> = old
                .partners
                .keys()
                .chain(v.partners.keys())
                .chain(old.held.iter().map(|h| &h.key.session))
                .copied()
                .collect();
            for s in sessions {
                self.world.touch_conversation(s);
            }
            self.translate_seen = Some(seen);
        } else {
            self.world.translate_view.status = self.translate.status();
            self.world.translate_view.offline = self.translate.offline();
        }
    }

    fn apply_translation(&mut self, fx: Vec<Effect>) {
        for e in fx {
            match e {
                Effect::Send { to, text } => self.send(NetCommand::SendIm { to, message: text }),
                Effect::Replace { key, text } => self.translate_replace(&key, &text),
            }
        }
    }

    fn translate_replace(&mut self, key: &LineKey, text: &str) {
        if let Some(original) = self.translate.original(key) {
            self.world.replace_line(key, &original, text);
        }
    }

    /// A choice made in a conversation (`ui::chat::ConvAction::Translate*`).
    pub(super) fn translate_action(&mut self, act: crate::ui::chat::ConvAction) {
        use crate::ui::chat::ConvAction;
        match act {
            ConvAction::Translate { session, to } => {
                if let Some(text) = self.translate_set(session, to) {
                    self.world.conversation_notice(session, text, Topic::Info);
                }
            }
            ConvAction::TranslateRetry(key) => self.translate_retry(key),
            ConvAction::TranslateSendOriginal(key) => self.translate_send_original(key),
            ConvAction::TranslateDiscard(key) => {
                let fx = self.translate.discard(key);
                self.world.remove_line(&key);
                self.apply_translation(fx);
            }
            ConvAction::TranslateDismiss(session) => {
                self.translate.dismiss(&mut self.settings.translate, session);
                self.settings.save();
            }
            _ => {}
        }
    }

    /// "Retry" on a line whose translation failed.
    fn translate_retry(&mut self, key: LineKey) {
        self.translate.retry(&self.settings.translate, key);
    }

    /// "Send original" on our line whose translation failed or is too long.
    fn translate_send_original(&mut self, key: LineKey) {
        let fx = self.translate.send_original(key);
        self.apply_translation(fx);
    }
}
