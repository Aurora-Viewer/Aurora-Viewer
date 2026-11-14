//! The conversation log's window (`ui/conversation_log.rs`) and what it
//! asks: a transcript in the History window or outside, an entry removed;
//! its view options saved; its demo (`demo_conversation_log.rs`).
use super::*;
use crate::world::notifications::{Data, Kind};

impl App {
    /// Each frame, with the other windows.
    pub(super) fn conversation_log_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_conversation_log();
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
                let (title, file) = self.world.logged_transcript(&c);
                self.history_ui.show_file(title, file);
            }
            Some(ui::conversation_log::Action::TranscriptExternally(c)) => {
                // chat_history_external: the file, else ChatHistoryIsMissing
                match self.world.logged_transcript(&c).1 {
                    Some(file) => {
                        if let Err(e) = crate::open_path::open(&file) {
                            log::warn!("conversation log: the transcript does not open outside ({e})");
                        }
                    }
                    None => {
                        self.world
                            .notifications
                            .push(Kind::Info, aurora_i18n::tr!("conversation-log-transcript-missing"), "", Data::None);
                    }
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
