//! `AURORA_DEMO_MENTION=list|names`: the chat bar's suggestions of residents
//! (#980), typed at frame 550 in the nearby chat: `list` writes `Hi @` (the
//! residents within chat range to mention), `names` turns the name
//! prediction on and writes the first three letters of the nearest
//! resident's user name after `Hi ` (the rest predicted, selected).
//! The gestures' list is `AURORA_DEMO_GESTURES=list` (`gestures.rs`).

use super::*;
use std::sync::OnceLock;

/// The scenario asked for ("" without it).
pub(super) fn mode() -> &'static str {
    static MODE: OnceLock<String> = OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_MENTION").unwrap_or_default())
}

impl App {
    pub(super) fn suggest_demo_input(&mut self, raw: &mut egui::RawInput) {
        if !matches!(mode(), "list" | "names") {
            return;
        }
        // the synthetic keystrokes own egui focus even when the demo window
        // is behind the user's current window
        raw.focused = true;
        match self.frame_count {
            540 => self.chat_ui.focus_request = true,
            550 if mode() == "list" => raw.events.push(egui::Event::Text("Hi @".into())),
            550 => raw.events.push(egui::Event::Text("Hi ".into())),
            560 if mode() == "names" => {
                let nearest = ui::chat::suggest::nearby(&self.world).into_iter().next();
                let letters: String = nearest.map(|(p, _)| p.user.chars().take(3).collect()).unwrap_or_default();
                log::info!("demo mention: predicting from {letters:?}");
                raw.events.push(egui::Event::Text(letters));
            }
            _ => {}
        }
    }
}
