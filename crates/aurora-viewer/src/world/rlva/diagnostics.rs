//! RlvFloaterConsole / RlvFloaterBehaviours (rlvfloaters.cpp, LGPL 2.1).
//! Console commands use the agent as their issuer. Channel zero is local
//! console output only for that issuer, never an object-script reply channel.
use super::{super::World, Command, Param, Rlva};
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
    /// Kept until the login completes (RLV_RET_RETAINED).
    Retained,
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

/// A key in any form someone may type: the Rust forms and the Legacy ones
/// (LLUUID::parseUUID), so that no form of a texture id gets past the rule.
fn any_key(text: &str) -> Option<Uuid> {
    Uuid::parse_str(text)
        .ok()
        .or_else(|| super::numeric::parse_uuid(text).filter(|id| !id.is_nil()))
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
            return match any_key(self.text.trim()) {
                Some(id) => Cow::Owned(ids().id(id).to_string()),
                None => Cow::Borrowed(&self.text),
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
                && let Some(id) = any_key(&text[option_start..option_end])
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

    /// RlvFloaterConsole::onClose (rlvfloaters.cpp): closing the console
    /// clears the agent's own locks, modifiers and pending work, never
    /// another controller's, with the "/clear" every @notify listener hears.
    /// No diagnostic data is persisted.
    pub fn close_console(&mut self, agent: Uuid) {
        let source = self.diagnostics.console_source.take().unwrap_or(agent);
        if !source.is_nil() {
            self.clear_issuer(source);
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

/// The RestrainedLoveDebug line process_chat_from_simulator
/// (llviewermessage.cpp) puts in nearby chat for an OwnerSay of commands.
#[derive(Default)]
pub(super) struct DebugLine {
    executed: String,
    failed: String,
    retained: String,
}

impl DebugLine {
    /// One command, `text` as the comma split gave it, with
    /// RlvStrings::getStringFromReturnCode's suffix.
    pub(super) fn push(&mut self, outcome: Outcome, text: &str, failure: Option<&str>) {
        let (list, suffix) = match outcome {
            Outcome::Retained => (&mut self.retained, None),
            Outcome::Rejected => (&mut self.failed, failure),
            Outcome::Duplicate => (&mut self.executed, Some("duplicate")),
            Outcome::Unset => (&mut self.executed, Some("unset")),
            Outcome::Input | Outcome::Processed | Outcome::Queued | Outcome::Reply => (&mut self.executed, None),
        };
        if !list.is_empty() {
            list.push(',');
        }
        list.push_str(text);
        if let Some(suffix) = suffix {
            list.push_str(" (");
            list.push_str(suffix);
            list.push(')');
        }
    }

    /// The chat text: one verb when one list holds everything, else the
    /// commands as received and a line per list. `message` is the OwnerSay
    /// in lower case, without its '@'.
    pub(super) fn finish(self, message: &str) -> Option<String> {
        let lists = [&self.executed, &self.failed, &self.retained];
        match lists.map(|list| !list.is_empty()) {
            [false, false, false] => None,
            [true, false, false] => Some(format!(" executes: @{}", self.executed)),
            [false, true, false] => Some(format!(" failed: @{}", self.failed)),
            [false, false, true] => Some(format!(" retained: @{}", self.retained)),
            _ => {
                let mut text = format!(": @{message}");
                for (verb, list) in [("executed", &self.executed), ("failed", &self.failed), ("retained", &self.retained)] {
                    if !list.is_empty() {
                        text.push_str(&format!("\n    - {verb}: @{list}"));
                    }
                }
                Some(text)
            }
        }
    }
}

impl Rlva {
    /// Whether RestrainedLoveDebug lists this outcome
    /// (RestrainedLoveDebugHideUnsetDup hides the unset and duplicate ones).
    pub(super) fn debug_lists(&self, outcome: Outcome) -> bool {
        self.preferences.debug_commands
            && !self.startup.replaying
            && (!self.preferences.debug_hide_unset_duplicate || !matches!(outcome, Outcome::Duplicate | Outcome::Unset))
    }

    /// The RLV_RET_FAILED_* code of a refused command, as
    /// RlvStrings::getStringFromReturnCode (rlvcommon.cpp) words it: the
    /// checks of RlvHandler::processCommand and its processors, in their
    /// order. None is a plain RLV_RET_FAILED, which has no suffix.
    pub(super) fn failure(&self, source: Uuid, c: Option<&Command>) -> Option<&'static str> {
        let blocked = |param| !self.controllers.accepts(source, param);
        let Some(c) = c else {
            return Some(if blocked(Param::Add) { "blocked object" } else { "thingy error" });
        };
        if blocked(c.param) {
            return Some("blocked object");
        }
        if c.behaviour == "setenv" && matches!(c.param, Param::Add | Param::Remove) && !self.preferences.allow_environment_locks {
            return Some("disabled command");
        }
        let available = self.preferences.command_available(&c.behaviour);
        match c.param {
            Param::Add | Param::Remove => {
                if !available || !super::RESTRICTIONS.contains(&c.behaviour.as_str()) || (c.strict && !super::supports_strict(&c.behaviour))
                {
                    Some("invalid param")
                } else if c.param == Param::Add
                    && matches!(c.behaviour.as_str(), "setcam" | "setenv" | "setdebug")
                    && self.has_except(&c.behaviour, source)
                {
                    Some("locked command")
                } else if !super::valid_option(c) {
                    Some("invalid option")
                } else {
                    None
                }
            }
            Param::Reply(channel)
                if !(super::valid_reply_channel(channel) || (channel == 0 && self.diagnostics.console_source == Some(source))) =>
            {
                Some("invalid param")
            }
            Param::Force if super::effects::MODIFIERS.contains(&c.behaviour.as_str()) => Some("base behaviour not held"),
            Param::Force | Param::Reply(_) => {
                let known = match c.param {
                    Param::Force => super::FORCES.contains(&c.behaviour.as_str()) || super::environment::supports(&c.behaviour, true),
                    _ => {
                        super::QUERIES.contains(&c.behaviour.as_str())
                            || super::environment::supports(&c.behaviour, false)
                            || c.behaviour.starts_with("getdebug_")
                    }
                };
                (c.strict || !available || !known).then_some("unknown command")
            }
            Param::Clear => None,
        }
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
        assert!(!world.rlva.can_send_typing_start());
        world.rlva.preferences.show_redirect_typing = true;
        assert!(world.rlva.can_send_typing_start());
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
                    param: Param::Reply(0),
                    environment_locked: false,
                    received: Default::default(),
                }
            ),
            Outcome::Rejected
        );
        world.rlva.close_console(agent);
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
        // (the status waits behind the force, as RlvHandler answers in order)
        world.rlva_pending();
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
