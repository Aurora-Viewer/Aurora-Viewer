//! Session-local RestrainedLove command processing.
//!
//! Port of RlvCommand, RlvHandler and RlvActions (indra/newview/rlvhelper.cpp,
//! rlvhandler.cpp and rlvactions.cpp, originally LGPL 2.1, Kitty Barnett).
//! Only object OwnerSay is a command source; the preference is the sole
//! authority for enabling it. Disabling drops all locks and pending work.

use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub mod camera;
mod inventory;
mod policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    Add,
    Remove,
    Clear,
    Force,
    Reply(i32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub behaviour: String,
    pub option: String,
    pub strict: bool,
    pub param: Param,
}

impl Command {
    pub fn parse(text: &str) -> Option<Self> {
        let (left, value) = text.split_once('=').unwrap_or((text, ""));
        let (name, option) = left.split_once(':').unwrap_or((left, ""));
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return None;
        }
        let param = match (name, value) {
            (_, "n" | "add") => Param::Add,
            (_, "y" | "rem") => Param::Remove,
            ("clear", _) => Param::Clear,
            (_, "force") => Param::Force,
            (_, v) => Param::Reply(v.parse().ok()?),
        };
        let (behaviour, strict) = name.strip_suffix("_sec").map_or((name, false), |n| (n, true));
        Some(Self {
            behaviour: behaviour.to_owned(),
            option: if param == Param::Clear { value } else { option }.to_owned(),
            strict,
            param,
        })
    }

    fn restriction(&self) -> String {
        let suffix = if self.strict { "_sec" } else { "" };
        if self.option.is_empty() {
            format!("{}{suffix}", self.behaviour)
        } else {
            format!("{}{suffix}:{}", self.behaviour, self.option)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Rule {
    behaviour: String,
    option: String,
    strict: bool,
}

impl From<&Command> for Rule {
    fn from(c: &Command) -> Self {
        Self {
            behaviour: c.behaviour.clone(),
            option: c.option.clone(),
            strict: c.strict,
        }
    }
}

/// Commands listed here have a consumer. Do not advertise unfinished handlers
/// through @getcommand: scripts use it to decide whether a feature is safe.
pub const RESTRICTIONS: &[&str] = &[
    "shownametags",
    "showhovertext",
    "showhovertextall",
    "showhovertexthud",
    "showhovertextworld",
    "share",
    "sendgesture",
    "setcam",
    "setcam_avdistmin",
    "setcam_avdistmax",
    "setcam_origindistmin",
    "setcam_origindistmax",
    "setcam_eyeoffset",
    "setcam_eyeoffsetscale",
    "setcam_focusoffset",
    "setcam_fovmin",
    "setcam_fovmax",
    "setcam_mouselook",
    "setcam_unlock",
    "addattach",
    "remattach",
    "detach",
    "addoutfit",
    "remoutfit",
    "sharedwear",
    "sharedunwear",
    "unsharedwear",
    "unsharedunwear",
    "touchall",
    "touchworld",
    "touchattach",
    "touchattachself",
    "touchattachother",
    "touchhud",
    "touchthis",
    "interact",
    "fartouch",
    "sittp",
    "alwaysrun",
    "buy",
    "chatnormal",
    "chatshout",
    "chatwhisper",
    "emote",
    "fly",
    "jump",
    "pay",
    "permissive",
    "recvchat",
    "recvchatfrom",
    "recvemote",
    "recvemotefrom",
    "recvim",
    "recvimfrom",
    "redirchat",
    "rediremote",
    "rez",
    "sendchannel",
    "sendchannel_except",
    "sendchat",
    "sendim",
    "sendimto",
    "setgroup",
    "showinv",
    "showminimap",
    "showworldmap",
    "sit",
    "sitground",
    "startim",
    "startimto",
    "temprun",
    "tplm",
    "tploc",
    "tplocal",
    "unsit",
];
pub const FORCES: &[&str] = &[
    "setcam_eyeoffset",
    "setcam_eyeoffsetscale",
    "setcam_focusoffset",
    "setcam_fov",
    "setcam_focus",
    "setcam_mode",
    "attach",
    "attachover",
    "attachall",
    "attachallover",
    "detach",
    "detachall",
    "detachme",
    "remattach",
    "remoutfit",
    "fly",
    "setgroup",
    "setrot",
    "sit",
    "sitground",
    "tpto",
    "unsit",
];
pub const QUERIES: &[&str] = &[
    "getcam_avdistmin",
    "getcam_avdistmax",
    "getcam_fov",
    "getcam_fovmin",
    "getcam_fovmax",
    "getinv",
    "getinvworn",
    "findfolder",
    "findfolders",
    "getattach",
    "getattachnames",
    "getaddattachnames",
    "getremattachnames",
    "getoutfit",
    "getoutfitnames",
    "getaddoutfitnames",
    "getremoutfitnames",
    "getpath",
    "getpathnew",
    "getcommand",
    "getgroup",
    "getsitid",
    "getstatus",
    "getstatusall",
    "version",
    "versionnew",
    "versionnum",
];

fn supports_strict(name: &str) -> bool {
    matches!(
        name,
        "recvchat"
            | "recvchatfrom"
            | "recvemote"
            | "recvemotefrom"
            | "recvim"
            | "recvimfrom"
            | "sendim"
            | "sendimto"
            | "startim"
            | "startimto"
            | "sendchannel"
            | "sendchannel_except"
    )
}

fn valid_option(c: &Command) -> bool {
    match c.behaviour.as_str() {
        "setcam_fovmin" | "setcam_fovmax" => c
            .option
            .parse::<f32>()
            .is_ok_and(|v| v.is_finite() && v > 0.0 && v < std::f32::consts::PI),
        "setcam_avdistmin" | "setcam_avdistmax" | "setcam_origindistmin" | "setcam_origindistmax" | "setcam_eyeoffsetscale" => {
            c.option.parse::<f32>().is_ok_and(|v| v.is_finite() && v >= 0.0)
        }
        "setcam_eyeoffset" | "setcam_focusoffset" => camera::vector(&c.option).is_some(),
        "fartouch" | "sittp" | "tplocal" => c.option.is_empty() || c.option.parse::<f32>().is_ok_and(|v| v.is_finite() && v >= 0.0),
        "touchthis" => Uuid::parse_str(&c.option).is_ok(),
        "touchworld" | "touchattach" | "touchattachother" | "touchhud" => c.option.is_empty() || Uuid::parse_str(&c.option).is_ok(),
        "addattach" | "remattach" | "detach" => c.option.is_empty() || inventory::point(&c.option).is_some(),
        "addoutfit" | "remoutfit" => c.option.is_empty() || inventory::layer(&c.option).is_some(),
        "recvchat" | "recvemote" | "recvim" | "sendim" | "startim" | "share" | "shownametags" => {
            c.option.is_empty() || Uuid::parse_str(&c.option).is_ok()
        }
        "showhovertext" => Uuid::parse_str(&c.option).is_ok(),
        "recvchatfrom" | "recvemotefrom" | "recvimfrom" | "sendimto" | "startimto" => Uuid::parse_str(&c.option).is_ok(),
        "sendchannel" | "sendchannel_except" => c.option.is_empty() || c.option.parse::<i32>().is_ok_and(|v| v != 0),
        "redirchat" | "rediremote" => c.option.parse::<i32>().is_ok_and(|v| v > 0 && v != i32::MAX),
        _ => c.option.is_empty(),
    }
}

#[derive(Default)]
pub struct Rlva {
    enabled: bool,
    rules: BTreeMap<Uuid, BTreeSet<Rule>>,
    pub pending: Vec<(Uuid, Command)>,
    pub out: Vec<NetCommand>,
    pub appearance: Vec<(Uuid, super::appearance::Action)>,
    pub viewer: Vec<(Uuid, Command)>,
    pub camera_values: BTreeMap<String, String>,
    pub revision: u64,
    gc: Option<std::time::Instant>,
    lookups: BTreeMap<Uuid, (bool, u8)>,
}

impl Rlva {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn inventory_locked(&self) -> bool {
        self.enabled
            && self.rules.values().any(|rs| {
                rs.iter().any(|r| {
                    matches!(
                        r.behaviour.as_str(),
                        "addattach"
                            | "remattach"
                            | "detach"
                            | "addoutfit"
                            | "remoutfit"
                            | "sharedwear"
                            | "sharedunwear"
                            | "unsharedwear"
                            | "unsharedunwear"
                    )
                })
            })
    }

    pub fn restrictions(&self) -> Vec<(Uuid, String)> {
        if !self.enabled {
            return Vec::new();
        }
        self.rules
            .iter()
            .flat_map(|(id, rules)| rules.iter().map(move |r| (*id, rule_text(r))))
            .collect()
    }

    pub fn hover_visible(&self, object: Uuid, hud: bool) -> bool {
        if !self.enabled {
            return true;
        }
        !self.has("showhovertextall")
            && !self.has(if hud { "showhovertexthud" } else { "showhovertextworld" })
            && !self.targets("showhovertext", &object.to_string())
    }

    pub fn configure(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        self.rules.clear();
        self.pending.clear();
        self.out.clear();
        self.appearance.clear();
        self.viewer.clear();
        self.camera_values.clear();
        self.lookups.clear();
        self.gc = None;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn forget(&mut self, source: Uuid) {
        self.lookups.remove(&source);
        if self.rules.remove(&source).is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
        self.pending.retain(|(id, _)| *id != source);
        self.appearance.retain(|(id, _)| *id != source);
        self.viewer.retain(|(id, _)| *id != source);
    }

    /// RlvHandler::onGC runs every 30 seconds. Unknown issuers get twenty
    /// lookup attempts; an object resolved once expires on its next absence.
    pub fn collect(&mut self, now: std::time::Instant, present: impl Fn(Uuid) -> bool) {
        if !self.enabled || self.gc.is_some_and(|last| now.saturating_duration_since(last).as_secs() < 30) {
            return;
        }
        self.gc = Some(now);
        let mut expired = Vec::new();
        for &id in self.rules.keys() {
            let entry = self.lookups.entry(id).or_default();
            if present(id) {
                *entry = (true, 0);
            } else {
                entry.1 = entry.1.saturating_add(1);
                if entry.0 || entry.1 > 20 {
                    expired.push(id);
                }
            }
        }
        for id in expired {
            self.forget(id);
        }
        self.lookups.retain(|id, _| self.rules.contains_key(id));
    }

    pub fn rules_for<'a>(&'a self, name: &str, actor: Uuid) -> impl Iterator<Item = (Uuid, &'a str)> {
        self.rules
            .iter()
            .filter(move |(id, _)| self.enabled && **id != actor)
            .flat_map(|(id, rs)| rs.iter().map(move |r| (*id, r)))
            .filter(move |(_, r)| r.behaviour == name)
            .map(|(id, r)| (id, r.option.as_str()))
    }

    pub fn distance_limit(&self, name: &str, default: f32, actor: Uuid) -> f32 {
        self.rules_for(name, actor)
            .map(|(_, option)| option.parse::<f32>().unwrap_or(default))
            .fold(f32::INFINITY, f32::min)
    }

    /// Returns true for an RLVa OwnerSay, including unsupported commands:
    /// protocol traffic never becomes user chat when RLVa is enabled.
    pub fn receive(&mut self, chat: &ChatMessage) -> bool {
        if !self.enabled
            || chat.source_type != ChatSourceType::Object
            || chat.chat_type != ChatType::OwnerSay
            || chat.source_id.is_nil()
            || !chat.message.starts_with('@')
        {
            return false;
        }
        // Simulator chat is bounded too, but keep synthetic/untrusted events
        // from allocating an unbounded command queue or restriction table.
        if chat.message.len() > 4096 {
            return true;
        }
        for text in chat.message[1..].split(',').take(256) {
            if let Some(command) = Command::parse(text) {
                self.process(chat.source_id, command);
            }
        }
        true
    }

    fn process(&mut self, source: Uuid, c: Command) {
        match c.param {
            Param::Add | Param::Remove => {
                if c.param == Param::Add && c.behaviour == "setcam" && self.has_except("setcam", source) {
                    return;
                }
                if !RESTRICTIONS.contains(&c.behaviour.as_str()) || (c.strict && !supports_strict(&c.behaviour)) || !valid_option(&c) {
                    return;
                }
                let rule = Rule::from(&c);
                let changed = if c.param == Param::Add {
                    if self.rules.len() >= 1024 && !self.rules.contains_key(&source) {
                        return;
                    }
                    let rules = self.rules.entry(source).or_default();
                    rules.len() < 256 && rules.insert(rule)
                } else {
                    self.rules.get_mut(&source).is_some_and(|rules| rules.remove(&rule))
                };
                if self.rules.get(&source).is_some_and(BTreeSet::is_empty) {
                    self.rules.remove(&source);
                }
                if changed {
                    self.revision = self.revision.wrapping_add(1);
                }
            }
            Param::Clear => {
                if let Some(rules) = self.rules.get_mut(&source) {
                    let old = rules.len();
                    rules.retain(|r| !rule_text(r).contains(&c.option));
                    if rules.len() != old {
                        self.revision = self.revision.wrapping_add(1);
                    }
                }
                if self.rules.get(&source).is_some_and(BTreeSet::is_empty) {
                    self.rules.remove(&source);
                }
            }
            Param::Force if FORCES.contains(&c.behaviour.as_str()) && self.pending.len() < 256 => self.pending.push((source, c)),
            Param::Reply(channel) if channel > 0 && channel != i32::MAX && QUERIES.contains(&c.behaviour.as_str()) => {
                let reply = match c.behaviour.as_str() {
                    // Do not claim a full specification version before parity.
                    "version" | "versionnew" => Some("RestrainedLove viewer v0.0 (Aurora experimental RLVa)".to_owned()),
                    "versionnum" => Some("0".to_owned()),
                    "getstatus" => Some(self.status(Some(source), &c.option)),
                    "getstatusall" => Some(self.status(None, &c.option)),
                    "getcommand" => Some(command_list(&c.option)),
                    _ => None,
                };
                if let Some(text) = reply {
                    self.reply(channel, text);
                } else if self.pending.len() < 256 {
                    self.pending.push((source, c));
                }
            }
            _ => {}
        }
    }

    pub fn reply(&mut self, channel: i32, message: String) {
        if self.enabled && channel > 0 && channel != i32::MAX && self.out.len() < 256 {
            self.out.push(NetCommand::Chat {
                channel,
                message,
                chat_type: ChatType::Shout,
            });
        }
    }

    fn status(&self, source: Option<Uuid>, option: &str) -> String {
        let (filter, separator) = option.split_once(';').unwrap_or((option, "/"));
        let separator = if separator.is_empty() { "/" } else { separator };
        let rules: Vec<_> = self
            .rules
            .iter()
            .filter(|(id, _)| source.is_none_or(|s| s == **id))
            .flat_map(|(_, r)| r.iter().map(rule_text))
            .filter(|r| r.contains(filter))
            .collect();
        if rules.is_empty() {
            String::new()
        } else {
            format!("{separator}{}", rules.join(separator))
        }
    }

    pub fn has(&self, behaviour: &str) -> bool {
        self.enabled
            && self
                .rules
                .values()
                .any(|rules| rules.iter().any(|r| r.behaviour == behaviour && r.option.is_empty()))
    }

    pub fn has_except(&self, behaviour: &str, actor: Uuid) -> bool {
        self.enabled
            && self
                .rules
                .iter()
                .any(|(id, rules)| *id != actor && rules.iter().any(|r| r.behaviour == behaviour && r.option.is_empty()))
    }

    /// A normal exception is shared. Under _sec (or @permissive=n), every
    /// object imposing that restriction must grant the exception itself.
    pub fn allows(&self, behaviour: &str, target: &str) -> bool {
        if !self.enabled {
            return true;
        }
        let strict = self.has("permissive")
            || self
                .rules
                .values()
                .any(|rs| rs.iter().any(|r| r.behaviour == behaviour && r.option.is_empty() && r.strict));
        let exception = |rs: &BTreeSet<Rule>| rs.iter().any(|r| r.behaviour == behaviour && r.option.eq_ignore_ascii_case(target));
        let restricting: Vec<_> = self
            .rules
            .values()
            .filter(|rs| rs.iter().any(|r| r.behaviour == behaviour && r.option.is_empty()))
            .collect();
        restricting.is_empty()
            || if strict {
                restricting.into_iter().all(exception)
            } else {
                self.rules.values().any(exception)
            }
    }

    pub fn targets(&self, behaviour: &str, target: &str) -> bool {
        self.enabled
            && self
                .rules
                .values()
                .any(|rs| rs.iter().any(|r| r.behaviour == behaviour && r.option.eq_ignore_ascii_case(target)))
    }

    pub fn allows_im(&self, name: &str, target: Uuid) -> bool {
        if !self.enabled {
            return true;
        }
        let id = target.to_string();
        let specific = match name {
            "sendim" => "sendimto",
            "startim" => "startimto",
            _ => "recvimfrom",
        };
        self.allows(name, &id) && !self.targets(specific, &id)
    }

    pub fn allows_channel(&self, channel: i32) -> bool {
        self.allows("sendchannel", &channel.to_string()) && !self.targets("sendchannel_except", &channel.to_string())
    }

    pub fn receive_chat(&self, chat: &mut ChatMessage, own: Uuid) -> bool {
        if !self.enabled || chat.source_id == own || chat.source_type == ChatSourceType::System || chat.chat_type == ChatType::OwnerSay {
            return true;
        }
        let emote = is_emote(&chat.message);
        let (all, specific) = if emote {
            ("recvemote", "recvemotefrom")
        } else {
            ("recvchat", "recvchatfrom")
        };
        let id = chat.source_id.to_string();
        if !self.allows(all, &id) || self.targets(specific, &id) {
            chat.message = if emote { "/me ..." } else { "..." }.to_owned();
        }
        true
    }

    /// Transform one chat line into zero or more sends. Redirection is checked
    /// before sendchat, and redirected text never returns to public chat.
    pub fn outgoing_chat(&self, message: &str, channel: i32, mut kind: ChatType) -> Vec<NetCommand> {
        let make = |message: String, channel, chat_type| NetCommand::Chat {
            message,
            channel,
            chat_type,
        };
        if !self.enabled {
            return vec![make(message.to_owned(), channel, kind)];
        }
        if channel != 0 {
            return if self.allows_channel(channel) {
                vec![make(message.to_owned(), channel, kind)]
            } else {
                Vec::new()
            };
        }
        if matches!(kind, ChatType::StartTyping | ChatType::StopTyping) {
            return if kind == ChatType::StartTyping && self.has_options("redirchat") {
                Vec::new()
            } else {
                vec![make(message.to_owned(), channel, kind)]
            };
        }
        let emote = is_emote(message);
        let redirect = if emote { "rediremote" } else { "redirchat" };
        if self.has_options(redirect) && (emote || filtered_chat(message, false, false) != message) {
            let channels: BTreeSet<_> = self
                .rules
                .values()
                .flat_map(|rs| rs.iter())
                .filter(|r| r.behaviour == redirect)
                .filter_map(|r| r.option.parse::<i32>().ok())
                .filter(|c| self.allows_channel(*c))
                .collect();
            return channels.into_iter().map(|c| make(message.to_owned(), c, ChatType::Shout)).collect();
        }
        let text = if self.has("sendchat") {
            filtered_chat(message, true, self.has("emote"))
        } else {
            message.to_owned()
        };
        if kind == ChatType::Shout && self.has("chatshout") {
            kind = ChatType::Normal;
        }
        if kind == ChatType::Whisper && self.has("chatwhisper") {
            kind = ChatType::Normal;
        }
        if kind == ChatType::Normal && self.has("chatnormal") {
            kind = if self.has("chatwhisper") {
                ChatType::Shout
            } else {
                ChatType::Whisper
            };
        }
        if (kind == ChatType::Shout && self.has("chatshout")) || text.is_empty() {
            Vec::new()
        } else {
            vec![make(text, 0, kind)]
        }
    }

    fn has_options(&self, name: &str) -> bool {
        self.enabled && self.rules.values().any(|rs| rs.iter().any(|r| r.behaviour == name))
    }
}

fn rule_text(r: &Rule) -> String {
    Command {
        behaviour: r.behaviour.clone(),
        option: r.option.clone(),
        strict: r.strict,
        param: Param::Add,
    }
    .restriction()
}

fn command_list(option: &str) -> String {
    let mut parts = option.splitn(3, ';');
    let filter = parts.next().unwrap_or_default();
    let kind = parts.next().unwrap_or("any");
    let separator = parts.next().filter(|v| !v.is_empty()).unwrap_or(";");
    let mut names = BTreeSet::new();
    for (name, list) in [("add", RESTRICTIONS), ("force", FORCES), ("reply", QUERIES)] {
        if kind.is_empty() || kind == "any" || kind == name {
            names.extend(list.iter().copied().filter(|n| n.contains(filter)));
        }
    }
    names.into_iter().collect::<Vec<_>>().join(separator)
}

pub fn is_emote(text: &str) -> bool {
    text.starts_with("/me ") || text.starts_with("/me'")
}

fn filtered_chat(text: &str, filter_emote: bool, allow_long_emote: bool) -> String {
    if is_emote(text) {
        if !filter_emote {
            return text.to_owned();
        }
        if text.contains(['"', '(', ')', '*', '=', '^', '_', '?', '~']) || text.contains(" -") || text.contains("- ") || text.contains("''")
        {
            return "...".to_owned();
        }
        if !allow_long_emote {
            return text
                .chars()
                .take(
                    text.chars()
                        .position(|c| c == '.')
                        .filter(|i| *i > 0 && *i < 20)
                        .map_or(20, |i| i + 1),
                )
                .collect();
        }
        return text.to_owned();
    }
    if (text.starts_with('/') && text.chars().count() <= 7) || (text.len() >= 4 && text.starts_with("((") && text.ends_with("))")) {
        text.to_owned()
    } else {
        "...".to_owned()
    }
}

#[cfg(test)]
#[path = "rlva/tests.rs"]
mod tests;
