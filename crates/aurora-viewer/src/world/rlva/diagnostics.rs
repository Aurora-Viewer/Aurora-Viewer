//! RlvFloaterConsole / RlvFloaterBehaviours (rlvfloaters.cpp, LGPL 2.1).
//! Console commands use the agent as their issuer. Channel zero is local
//! console output only for that issuer, never an object-script reply channel.
use super::{super::World, Rlva};
use std::collections::VecDeque;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Input,
    Processed,
    Queued,
    Duplicate,
    Unset,
    Rejected,
    Reply,
}

pub struct Entry {
    pub source: Uuid,
    pub text: String,
    pub outcome: Outcome,
}

#[derive(Default)]
pub struct Diagnostics {
    pub entries: VecDeque<Entry>,
    pub console_source: Option<Uuid>,
}

impl Diagnostics {
    fn push(&mut self, source: Uuid, text: &str, outcome: Outcome) {
        let mut end = text.len().min(4096);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if self.entries.len() >= 256 {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            source,
            text: text[..end].into(),
            outcome,
        });
    }

    pub(super) fn push_reply(&mut self, message: String) {
        if let Some(source) = self.console_source {
            self.push(source, &message, Outcome::Reply);
        }
    }
}

impl Rlva {
    pub(super) fn trace(&mut self, source: Uuid, text: &str, outcome: Outcome) {
        let console = self.diagnostics.console_source == Some(source);
        if (console || self.preferences.debug_commands)
            && (console || !self.preferences.debug_hide_unset_duplicate || !matches!(outcome, Outcome::Duplicate | Outcome::Unset))
        {
            self.diagnostics.push(source, text, outcome);
        }
    }

    /// Closing the console releases its own locks/modifiers and pending work;
    /// it cannot clear another controller. No diagnostic data is persisted.
    pub fn close_console(&mut self) {
        if let Some(source) = self.diagnostics.console_source.take() {
            self.forget(source);
        }
    }
}

impl World {
    pub fn rlva_console_input(&mut self, input: &str) {
        let text = input.trim();
        let source = self.agent_id;
        self.rlva.diagnostics.push(source, text, Outcome::Input);
        if !self.rlva.enabled() || source.is_nil() || text.len() > 4096 || text.len() <= 3 || !text.starts_with('@') {
            self.rlva.diagnostics.push(source, text, Outcome::Rejected);
            return;
        }
        self.rlva.diagnostics.console_source = Some(source);
        self.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            source_id: source,
            owner_id: source,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: self.agent.position,
            from_name: String::new(),
            message: text.to_owned(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::rlva::{Command, Param};

    #[test]
    fn rlva_console_queries_stay_local_and_closing_releases_only_the_console_issuer() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let agent = Uuid::from_u128(1);
        let controller = Uuid::from_u128(2);
        world.agent_id = agent;
        world.rlva.configure(true);
        world.rlva.process(controller, Command::parse("redirchat:123=n").unwrap());
        let typing = aurora_net::NetCommand::AgentAnimation {
            anim: crate::world::radar::ANIM_AGENT_TYPE,
            start: true,
        };
        assert!(!world.rlva_allows_command(&typing, Uuid::nil()));
        world.rlva.preferences.show_redirect_typing = true;
        assert!(world.rlva_allows_command(&typing, Uuid::nil()));
        world.rlva.process(controller, Command::parse("jump=n").unwrap());
        world.rlva_console_input(" @fly=n,version=0 ");
        assert!(world.rlva.has("fly") && world.rlva.has("jump"));
        assert!(world.rlva.out.is_empty(), "console answers never leak to nearby chat");
        assert!(
            world
                .rlva
                .diagnostics
                .entries
                .iter()
                .any(|entry| entry.outcome == Outcome::Reply && entry.text.contains("v3.4.3"))
        );
        world.rlva_console_input("@getgroup=0");
        assert!(world.rlva_pending().is_empty());
        assert!(
            world
                .rlva
                .diagnostics
                .entries
                .iter()
                .any(|entry| entry.outcome == Outcome::Reply && entry.text == "none")
        );
        assert!(world.rlva.out.is_empty());
        assert_eq!(
            world.rlva.process(
                controller,
                Command {
                    behaviour: "version".into(),
                    option: String::new(),
                    strict: false,
                    param: Param::Reply(0)
                }
            ),
            Outcome::Rejected
        );
        world.rlva.close_console();
        assert!(!world.rlva.has("fly") && world.rlva.has("jump"));
        assert!(world.rlva.diagnostics.console_source.is_none());
        world.rlva.configure(false);
        assert!(world.rlva.diagnostics.entries.is_empty());
    }

    #[test]
    fn rlva_diagnostics_are_opt_in_bounded_and_can_hide_duplicate_unset_feedback() {
        let mut engine = Rlva::default();
        engine.configure(true);
        let source = Uuid::from_u128(3);
        engine.trace(source, "fly=n", Outcome::Processed);
        assert!(engine.diagnostics.entries.is_empty());
        engine.preferences.debug_commands = true;
        engine.preferences.debug_hide_unset_duplicate = true;
        for outcome in [Outcome::Duplicate, Outcome::Unset] {
            engine.trace(source, "fly=n", outcome);
        }
        assert!(engine.diagnostics.entries.is_empty());
        for _ in 0..300 {
            engine.trace(source, &"é".repeat(3000), Outcome::Rejected);
        }
        assert_eq!(engine.diagnostics.entries.len(), 256);
        assert_eq!(engine.diagnostics.entries.back().unwrap().text.len(), 4096);
    }

    #[test]
    fn rlva_experimental_gate_controls_discovery_new_commands_and_retains_cleanup() {
        let mut engine = Rlva::default();
        engine.configure(true);
        let source = Uuid::from_u128(4);
        engine.process(source, Command::parse("showself=n").unwrap());
        engine.preferences.experimental_commands = false;
        assert_eq!(engine.process(source, Command::parse("shownearby=n").unwrap()), Outcome::Rejected);
        assert!(!engine.has("shownearby"));
        engine.process(source, Command::parse("showself=y").unwrap());
        assert!(!engine.has("showself"));
        assert_eq!(super::super::command_list_with("showself;add", &engine.preferences), "");
        assert_eq!(
            super::super::command_list_with("shownames;add", &engine.preferences),
            "shownames;shownames_sec"
        );
        assert_eq!(engine.process(source, Command::parse("fly=n").unwrap()), Outcome::Processed);
        assert_eq!(engine.process(source, Command::parse("fly=n").unwrap()), Outcome::Duplicate);
        assert_eq!(engine.process(source, Command::parse("fly=y").unwrap()), Outcome::Processed);
        assert_eq!(engine.process(source, Command::parse("fly=y").unwrap()), Outcome::Unset);
    }
}
