//! The conversation log's window (`ui/conversation_log.rs`) and what it
//! asks: a transcript in the History window or outside, an entry removed;
//! its view options saved; its demo (`demo_conversation_log.rs`).
use super::*;
use crate::world::notifications::{Data, Kind};

impl App {
    /// The History window on a conversation: a direct one under the Chat
    /// Service shows its stitched history and puts its resident first in
    /// the queue (LLFloaterConversationPreview, LL viewer 26.5); any other
    /// its transcript source. The source's stem stands in for a resident
    /// not named yet (a Conversation Log entry's `history_file`, else its
    /// name), so its plaintext is still read.
    pub(super) fn open_history(
        &mut self,
        resident: Option<uuid::Uuid>,
        title: String,
        source: Option<crate::world::chat_log::HistorySource>,
    ) {
        let fallback = source.as_ref().and_then(|s| s.stem()).map(str::to_owned);
        match resident {
            Some(id) if self.world.open_direct_preview(id, fallback.as_deref()) => self.history_ui.show_stitched(title, id, source),
            _ => {
                let peer = resident.and_then(|id| self.world.private_peer(id));
                self.history_ui.show_source(title, peer, source)
            }
        }
    }

    /// ChatHistoryIsMissing: the transcript of an entry is not on disk.
    fn transcript_missing(&mut self) {
        self.world
            .notifications
            .push(Kind::Info, aurora_i18n::tr!("conversation-log-transcript-missing"), "", Data::None);
    }

    /// Each frame, with the other windows.
    pub(super) fn conversation_log_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_conversation_log();
        }
        // the transcript asked to open outside Aurora, once found
        match self.world.chat_log.take_located() {
            Some(Some(file)) => {
                if let Err(e) = crate::open_path::open(&file) {
                    log::warn!("conversation log: the transcript does not open outside ({e})");
                }
            }
            Some(None) => self.transcript_missing(),
            None => {}
        }
        let input = ui::conversation_log::Inputs {
            world: &self.world,
            pics: &self.avatar_pics,
            images: &self.ui_images,
        };
        let (action, changed) = self.conversation_log_ui.show(
            ctx,
            p,
            &input,
            &mut self.settings.conversation_log,
            &mut self.panels.conversation_log,
        );
        if changed {
            self.settings.save();
        }
        match action {
            Some(ui::conversation_log::Action::Transcript(c)) => {
                let (title, source) = self.world.logged_transcript_source(&c);
                let resident = (c.kind == crate::world::chat_log::conversations::Kind::Private).then_some(c.participant);
                self.open_history(resident, title, source);
            }
            Some(ui::conversation_log::Action::TranscriptExternally(c)) => {
                // chat_history_external: the file (looked for on the
                // transcript thread, opened below once found), else
                // ChatHistoryIsMissing
                if !self.world.locate_logged_transcript(&c) {
                    self.transcript_missing();
                }
            }
            Some(ui::conversation_log::Action::Remove { session, time }) => {
                self.world.chat_log.conversations.remove(session, time);
            }
            None => {}
        }
    }

    /// AURORA_DEMO_CONVERSATION_LOG (`demo_conversation_log.rs`).
    fn demo_conversation_log(&mut self) {
        use crate::demo::conversation_log::{self as demo, Mode};
        let Some(mode) = demo::mode() else { return };
        if self.frame_count == demo::FILL_FRAME {
            match mode {
                Mode::Empty => self.world.chat_log.conversations.demo(Vec::new()),
                // logging off, with what an earlier session logged
                Mode::Off => {
                    let list = demo::entries(&self.world, crate::world::chat_log::conversations::now());
                    self.world.chat_log.conversations.demo(list);
                    self.world.chat_log.conversations.logging = false;
                }
                _ => {
                    let list = demo::entries(&self.world, crate::world::chat_log::conversations::now());
                    self.world.chat_log.conversations.demo(list);
                }
            }
            if mode == Mode::Name {
                self.settings.conversation_log.by_date = false;
            }
        }
        if self.frame_count == demo::OPEN_FRAME {
            ui::widgets::open_floater(&self.egui_ctx, ui::conversation_log::WINDOW, &mut self.panels.conversation_log);
            if mode == Mode::Filter {
                self.conversation_log_ui.demo_filter(demo::FILTER);
            }
            if mode == Mode::Live {
                // a message from a resident the log does not have yet
                self.world.apply(demo::new_message(self.world.agent_id));
            }
            let log = &self.world.chat_log.conversations;
            log::info!(
                "conversation log demo: {} entries, logging {}",
                log.list.len(),
                if log.logging { "on" } else { "off" }
            );
        }
    }
}
