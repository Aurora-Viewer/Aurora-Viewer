//! RlvFloaterConsole / RlvFloaterBehaviours (rlvfloaters.cpp, LGPL 2.1).
//! Console commands use the agent as their issuer. Channel zero is local
//! console output only for that issuer, never an object-script reply channel.
use super::{super::World, Rlva};
use crate::world::inventory::asset_ids::AssetIds;
use std::{borrow::Cow, collections::VecDeque};
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
    /// A reply that is an asset id as a whole (`@getcam_textures=0`).
    asset: bool,
}

/// The commands whose option is a texture id (by their canonical name).
const ASSET_OPTIONS: &[&str] = &[
    "setcam_textures",
    "setoverlay_texture",
    "setenv_cloudtexture",
    "setenv_moontexture",
    "setenv_suntexture",
];

/// The queries that reply with a texture id.
fn replies_asset(behaviour: &str) -> bool {
    matches!(
        behaviour,
        "getcam_textures" | "setoverlay_texture" | "getenv_cloudtexture" | "getenv_moontexture" | "getenv_suntexture"
    )
}

impl Entry {
    /// The text the console shows: every texture id it holds goes through the
    /// asset id rule (`hidden-`), in a command's option, in a rule that
    /// @getstatus lists, and in a texture query's reply. RlvFloaterConsole
    /// echoes them as they came; this is stricter, as for any asset id that
    /// leaves the viewer through a window. The log file keeps them. `ids` is
    /// only taken when the entry holds such an id.
    pub fn shown<'a>(&'a self, ids: impl FnOnce() -> &'a AssetIds) -> Cow<'a, str> {
        if self.asset {
            return match Uuid::parse_str(self.text.trim()) {
                Ok(id) => Cow::Owned(ids().id(id).to_string()),
                Err(_) => Cow::Borrowed(&self.text),
            };
        }
        let text = self.text.as_str();
        let is_delimiter = |c: char| matches!(c, '@' | ',' | '/' | ';' | '=') || c.is_whitespace();
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(colon) = text[from..].find(':').map(|at| from + at) {
            let name_start = text[..colon].rfind(is_delimiter).map_or(0, |at| at + 1);
            let option_start = colon + 1;
            let option_end = text[option_start..].find(is_delimiter).map_or(text.len(), |at| option_start + at);
            if ASSET_OPTIONS.contains(&super::canonical(&text[name_start..colon].to_ascii_lowercase()))
                && let Ok(id) = Uuid::parse_str(&text[option_start..option_end])
            {
                found.push((option_start..option_end, id));
            }
            from = option_start;
        }
        if found.is_empty() {
            return Cow::Borrowed(text);
        }
        let ids = ids();
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for (range, id) in found {
            out.push_str(&text[last..range.start]);
            out.push_str(&ids.id(id).to_string());
            last = range.end;
        }
        out.push_str(&text[last..]);
        Cow::Owned(out)
    }
}

#[derive(Default)]
pub struct Diagnostics {
    pub entries: VecDeque<Entry>,
    pub console_source: Option<Uuid>,
}

impl Diagnostics {
    fn push(&mut self, source: Uuid, text: &str, outcome: Outcome) {
        self.push_entry(source, text, outcome, false);
    }

    fn push_entry(&mut self, source: Uuid, text: &str, outcome: Outcome, asset: bool) {
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
            asset,
        });
    }

    /// `behaviour` is the query answered, so that a texture id is known as one.
    pub(super) fn push_reply(&mut self, behaviour: &str, message: String) {
        if let Some(source) = self.console_source {
            self.push_entry(source, &message, Outcome::Reply, replies_asset(behaviour));
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
            region: None,
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
    fn rlva_console_hides_texture_ids_the_rule_does_not_allow() {
        const TEXTURE: &str = "00000000-0000-0000-0000-0000000000bb";
        const FRIEND: &str = "00000000-0000-0000-0000-00000000000a";
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.rlva.configure(true);
        world.rlva_console_input(&format!("@camtextures:{TEXTURE}=n,sendim:{FRIEND}=add"));
        world.rlva_console_input(&format!("@setenv_suntexture:{}=force", TEXTURE.replace('-', "")));
        world.rlva_console_input("@getstatus=0");
        world.rlva.reply(0, "getcam_textures", TEXTURE.to_owned());
        world.rlva.reply(0, "getsitid", FRIEND.to_owned());

        let inventory = crate::world::inventory::Inventory::default();
        let hidden = AssetIds::new(&inventory, [], false);
        let shown: Vec<String> = world
            .rlva
            .diagnostics
            .entries
            .iter()
            .map(|entry| entry.shown(|| &hidden).into_owned())
            .collect();
        let all = shown.join("\n");
        assert!(!all.contains(TEXTURE) && !all.contains(&TEXTURE.replace('-', "")), "{all}");
        assert!(shown[0].starts_with("@camtextures:hidden-"), "{}", shown[0]);
        assert!(
            shown[0].ends_with(&format!("=n,sendim:{FRIEND}=add")),
            "a resident id is not an asset"
        );
        assert!(
            shown
                .iter()
                .any(|text| text.starts_with("/camtextures:hidden-") && text.contains(FRIEND)),
            "@getstatus: {all}"
        );
        assert!(shown.iter().any(|text| text.starts_with("@setenv_suntexture:hidden-")), "{all}");
        assert!(shown[shown.len() - 2].starts_with("hidden-"), "the texture query's reply");
        assert_eq!(shown[shown.len() - 1], FRIEND, "an object id reply stays");

        let id = Uuid::parse_str(TEXTURE).unwrap();
        let local = AssetIds::new(&inventory, [id], false);
        let entry = world.rlva.diagnostics.entries.front().unwrap();
        assert_eq!(entry.shown(|| &local), entry.text, "the user's own local texture shows");
        let plain = world
            .rlva
            .diagnostics
            .entries
            .iter()
            .find(|entry| entry.text == "@getstatus=0")
            .unwrap();
        assert_eq!(plain.shown(|| panic!("no texture id here")), "@getstatus=0");
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
