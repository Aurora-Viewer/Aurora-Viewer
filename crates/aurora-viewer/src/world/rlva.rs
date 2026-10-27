//! Session-local RestrainedLove command processing.
//!
//! Port of RlvCommand, RlvHandler and RlvActions (indra/newview/rlvhelper.cpp,
//! rlvhandler.cpp and rlvactions.cpp, originally LGPL 2.1, Kitty Barnett).
//! Only object OwnerSay is a command source; the preference is the sole
//! authority for enabling it. Disabling drops all locks and pending work.

use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// What @version answers; About shows it too.
pub const VERSION: &str = version::VERSION_NEW;

pub mod camera;
mod chat;
mod collection;
pub(super) mod communication;
mod controllers;
mod debug;
pub mod diagnostics;
mod editing;
pub mod effects;
pub mod environment;
mod events;
mod groups;
mod im_queries;
mod inventory;
pub mod login;
mod naming;
pub mod offers;
mod policy;
pub mod preferences;
pub mod privacy;
mod relative;
mod rename;
mod sitting;
mod teleport;
mod temporary;
mod version;
mod wardrobe;
pub mod watchdog;

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
        if !text.contains('=') && !text.eq_ignore_ascii_case("clear") {
            return None;
        }
        let (left, value) = text.split_once('=').unwrap_or((text, ""));
        let (name, option) = left.split_once(':').unwrap_or((left, ""));
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_') {
            return None;
        }
        let name = name.to_ascii_lowercase();
        let name = name.as_str();
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

impl Rule {
    fn general(&self) -> bool {
        self.option.is_empty()
            || (matches!(self.behaviour.as_str(), "sendim" | "recvim" | "startim") && im_range(&self.option).is_some())
            || (self.behaviour == "shownametags" && self.option.parse::<f32>().is_ok())
            || matches!(
                canonical(&self.behaviour),
                "setcam_avdist"
                    | "setcam_textures"
                    | "setcam_avdistmin"
                    | "setcam_avdistmax"
                    | "setcam_origindistmin"
                    | "setcam_origindistmax"
                    | "setcam_eyeoffset"
                    | "setcam_eyeoffsetscale"
                    | "setcam_focusoffset"
                    | "setcam_fovmin"
                    | "setcam_fovmax"
                    | "fartouch"
                    | "sittp"
                    | "tplocal"
            )
    }
}

fn im_range(text: &str) -> Option<(f32, f32)> {
    let (min, max) = text.split_once(';').unwrap_or((text, ""));
    let min = min.parse::<f32>().ok()?;
    let max = if max.is_empty() { f32::MAX } else { max.parse::<f32>().ok()? };
    (min.is_finite() && max.is_finite() && min >= 0.0 && max >= 0.0).then_some((min, max))
}

/// Commands listed here have a consumer. Do not advertise unfinished handlers
/// through @getcommand: scripts use it to decide whether a feature is safe.
pub const RESTRICTIONS: &[&str] = &[
    "showloc",
    "shownames",
    "shownearby",
    "setsphere",
    "setcam_avdist",
    "setcam_textures",
    "camavdist",
    "camtextures",
    "showself",
    "showselfhead",
    "setoverlay",
    "setoverlay_touch",
    "acceptpermission",
    "accepttp",
    "accepttprequest",
    "allowidle",
    "notify",
    "tplure",
    "tprequest",
    "edit",
    "editobj",
    "editattach",
    "editworld",
    "viewtransparent",
    "viewwireframe",
    "viewnote",
    "viewscript",
    "viewtexture",
    "setenv",
    "setdebug",
    "attachthis",
    "attachallthis",
    "attachthis_except",
    "attachallthis_except",
    "detachthis",
    "detachallthis",
    "detachthis_except",
    "detachallthis_except",
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
    "camdistmin",
    "camdistmax",
    "camzoommin",
    "camzoommax",
    "camunlock",
    "touchfar",
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
    "touchme",
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
    "standtp",
    "startim",
    "startimto",
    "temprun",
    "tplm",
    "tploc",
    "tplocal",
    "unsit",
];
pub const FORCES: &[&str] = &[
    "setdebug_renderresolutiondivisor",
    "setdebug_avatarsex",
    "adjustheight",
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
    "attachoverorreplace",
    "attachalloverorreplace",
    "attachthis",
    "attachallthis",
    "attachthisover",
    "attachallthisover",
    "attachthisoverorreplace",
    "attachallthisoverorreplace",
    "addoutfit",
    "addoutfitover",
    "addoutfitall",
    "addoutfitallover",
    "addoutfitthis",
    "addoutfitallthis",
    "addoutfitthisover",
    "addoutfitallthisover",
    "detachthis",
    "detachallthis",
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
    "getcam_textures",
    "getdebug_avatarsex",
    "getdebug_aspectratio",
    "getdebug_renderresolutiondivisor",
    "getdebug_restrainedloveforbidgivetorlv",
    "getdebug_restrainedlovenosetenv",
    "getdebug_windlightuseatmosshaders",
    "getcam_avdist",
    "getheightoffset",
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
            | "share"
            | "accepttp"
            | "accepttprequest"
            | "tplure"
            | "tprequest"
            | "shownametags"
            | "shownames"
    )
}

fn canonical(name: &str) -> &str {
    match name {
        "camdistmin" => "setcam_avdistmin",
        "camavdist" => "setcam_avdist",
        "camtextures" => "setcam_textures",
        "camdistmax" => "setcam_avdistmax",
        "camzoommin" => "setcam_fovmin",
        "camzoommax" => "setcam_fovmax",
        "camunlock" => "setcam_unlock",
        "touchfar" => "fartouch",
        _ => name,
    }
}

fn valid_option(c: &Command) -> bool {
    if matches!(c.behaviour.as_str(), "camzoommin" | "camzoommax") {
        return c.option.is_empty() || c.option.parse::<f32>().is_ok_and(|v| v.is_finite() && v > 0.0);
    }
    match canonical(&c.behaviour) {
        "attachthis"
        | "attachallthis"
        | "attachthis_except"
        | "attachallthis_except"
        | "detachthis"
        | "detachallthis"
        | "detachthis_except"
        | "detachallthis_except" => !c.option.split('/').any(|v| matches!(v, "." | "..")),
        "notify" => events::notification_option(&c.option).is_some(),
        "setcam_fovmin" | "setcam_fovmax" | "setcam_eyeoffsetscale" => c.option.parse::<f32>().is_ok_and(f32::is_finite),
        "setcam_avdist" | "setcam_avdistmin" | "setcam_avdistmax" | "setcam_origindistmin" | "setcam_origindistmax" => {
            c.option.parse::<f32>().is_ok_and(|v| v.is_finite() && v >= 0.0)
        }
        "setcam_eyeoffset" | "setcam_focusoffset" => camera::vector(&c.option).is_some(),
        "fartouch" | "sittp" | "tplocal" => c.option.is_empty() || c.option.parse::<f32>().is_ok_and(|v| v.is_finite() && v >= 0.0),
        "touchthis" | "editobj" => Uuid::parse_str(&c.option).is_ok(),
        "setcam_textures" => c.option.is_empty() || Uuid::parse_str(&c.option).is_ok(),
        "touchworld" | "touchattach" | "touchattachother" | "touchhud" | "edit" => {
            c.option.is_empty() || Uuid::parse_str(&c.option).is_ok()
        }
        "addattach" | "remattach" | "detach" => c.option.is_empty() || inventory::point(&c.option).is_some(),
        "addoutfit" | "remoutfit" => c.option.is_empty() || inventory::layer(&c.option).is_some(),
        "recvim" | "sendim" | "startim" if im_range(&c.option).is_some() => true,
        "shownametags" if c.option.parse::<f32>().is_ok_and(|v| v.is_finite() && v >= 0.0) => true,
        "recvchat" | "recvemote" | "recvim" | "sendim" | "startim" | "share" | "shownametags" | "shownames" | "accepttp"
        | "accepttprequest" | "tplure" | "tprequest" => c.option.is_empty() || Uuid::parse_str(&c.option).is_ok(),
        "showhovertext" => Uuid::parse_str(&c.option).is_ok(),
        "recvchatfrom" | "recvemotefrom" | "recvimfrom" | "sendimto" | "startimto" => Uuid::parse_str(&c.option).is_ok(),
        "sendchannel" | "sendchannel_except" => c.option.is_empty() || c.option.parse::<i32>().is_ok_and(|v| v > 0),
        "redirchat" | "rediremote" => c.option.parse::<i32>().is_ok_and(|v| v > 0 && v != i32::MAX),
        _ => c.option.is_empty(),
    }
}

#[derive(Default)]
pub struct Rlva {
    enabled: bool,
    pub preferences: preferences::Preferences,
    pub rename: rename::RenameOnWear,
    pub offers: offers::Offers,
    pub controllers: controllers::Controllers,
    pub relative: relative::RelativeWear,
    pub version: version::Version,
    pub diagnostics: diagnostics::Diagnostics,
    pub im: im_queries::Queries,
    rules: BTreeMap<Uuid, BTreeSet<Rule>>,
    pub pending: Vec<(Uuid, Command)>,
    pub out: Vec<NetCommand>,
    pub appearance: Vec<(Uuid, super::appearance::Action)>,
    pub viewer: Vec<(Uuid, Command)>,
    pub camera_values: BTreeMap<String, String>,
    pub groups: groups::Groups,
    pub sitting: sitting::Sitting,
    pub wardrobe: wardrobe::Wardrobe,
    pub visuals: effects::Visuals,
    pub watchdog: watchdog::Watchdog,
    pub(crate) locked_folders: inventory::LockedFoldersCache,
    pub recovery: Vec<NetCommand>,
    pub environment: Option<(Uuid, super::eep::SkyFrame)>,
    pub environment_water: Option<super::eep::WaterFrame>,
    pub environment_day: Option<std::sync::Arc<super::eep::DayCycle>>,
    pub environment_request: Option<(Uuid, Command)>,
    pub pseudo_sex: Option<String>,
    pub resolution_divisor: Option<u32>,
    pub appearance_busy: bool,
    pub self_detach: Option<(Uuid, Uuid)>,
    pub stand_origin: Option<(aurora_net::RegionHandle, glam::Vec3)>,
    pub stand_return: Option<(aurora_net::RegionHandle, glam::Vec3)>,
    stand_seated: bool,
    inventory_fetches: BTreeMap<Uuid, (std::time::Instant, u8)>,
    pub revision: u64,
    gc: Option<std::time::Instant>,
    lookups: BTreeMap<Uuid, (bool, u8)>,
}

impl Rlva {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn preview_mask(&self) -> u8 {
        u8::from(self.has("viewtexture")) | (u8::from(self.has("viewnote")) << 1) | (u8::from(self.has("viewscript")) << 2)
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
                            | "attachthis"
                            | "attachallthis"
                            | "attachthis_except"
                            | "attachallthis_except"
                            | "detachthis"
                            | "detachallthis"
                            | "detachthis_except"
                            | "detachallthis_except"
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
        self.rename = Default::default();
        self.offers = Default::default();
        self.controllers = Default::default();
        self.relative = Default::default();
        self.version = Default::default();
        self.im = Default::default();
        self.pending.clear();
        self.out.clear();
        self.appearance.clear();
        self.viewer.clear();
        self.camera_values.clear();
        self.groups = Default::default();
        self.sitting = Default::default();
        self.wardrobe = Default::default();
        self.visuals = Default::default();
        self.watchdog = Default::default();
        self.diagnostics = Default::default();
        self.recovery.clear();
        self.clear_environment();
        self.pseudo_sex = None;
        self.resolution_divisor = None;
        self.appearance_busy = false;
        self.self_detach = None;
        self.stand_origin = None;
        self.stand_return = None;
        self.stand_seated = false;
        self.inventory_fetches.clear();
        self.lookups.clear();
        self.gc = None;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn forget(&mut self, source: Uuid) {
        self.relative.forget(source);
        self.version.forget(source);
        let was_fov_locked = self.has("setcam_fovmin") || self.has("setcam_fovmax");
        self.watchdog.sources.remove(&source);
        self.visuals.forget(source);
        if self.self_detach.is_some_and(|(id, _)| id == source) {
            self.self_detach = None;
        }
        if self.environment.is_some_and(|(id, _)| id == source) {
            self.clear_environment();
        }
        if self.environment_request.as_ref().is_some_and(|(id, _)| *id == source) {
            self.environment_request = None;
        }
        self.lookups.remove(&source);
        if self.rules.remove(&source).is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
        if was_fov_locked && !self.has("setcam_fovmin") && !self.has("setcam_fovmax") {
            self.camera_values.remove("setcam_fov");
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
            .filter(move |(_, r)| canonical(&r.behaviour) == canonical(name))
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
            || chat.message.len() <= 3
        {
            return false;
        }
        // Simulator chat is bounded too, but keep synthetic/untrusted events
        // from allocating an unbounded command queue or restriction table.
        if chat.message.len() > 4096 {
            return true;
        }
        // process_chat_from_simulator folds the complete OwnerSay command,
        // including options/parameters, before RlvCommand parsing.
        let message = chat.message[1..].to_ascii_lowercase();
        for text in message.split(',').take(256) {
            if let Some(command) = Command::parse(text) {
                let notify = matches!(command.param, Param::Add | Param::Remove | Param::Clear);
                let outcome = self.process(chat.source_id, command);
                self.trace(chat.source_id, text, outcome);
                if notify {
                    self.notify(text);
                }
            } else if !text.is_empty() {
                self.trace(chat.source_id, text, diagnostics::Outcome::Rejected);
            }
        }
        true
    }

    fn process(&mut self, source: Uuid, c: Command) -> diagnostics::Outcome {
        use diagnostics::Outcome;
        if !self.controllers.accepts(source, c.param)
            || (c.behaviour == "setenv" && matches!(c.param, Param::Add | Param::Remove) && !self.preferences.allow_environment_locks)
            || (!self.preferences.command_available(&c.behaviour) && !matches!(c.param, Param::Remove | Param::Clear))
        {
            return Outcome::Rejected;
        }
        if effects::MODIFIERS.contains(&c.behaviour.as_str()) {
            if c.param == Param::Force && !c.strict && self.rules.get(&source).is_some_and(|r| !r.is_empty()) {
                self.visuals.command(source, &c, std::time::Instant::now());
                return Outcome::Processed;
            }
            return Outcome::Rejected;
        }
        let mut outcome = Outcome::Processed;
        let was_environment_locked = self.has("setenv");
        let was_fov_locked = self.has("setcam_fovmin") || self.has("setcam_fovmax");
        match c.param {
            Param::Add | Param::Remove => {
                if c.param == Param::Add
                    && c.behaviour == "setsphere"
                    && self.visuals.sphere_order.len() >= 6
                    && !self.visuals.sphere_order.contains(&source)
                {
                    return Outcome::Rejected;
                }
                if c.param == Param::Add
                    && matches!(c.behaviour.as_str(), "setcam" | "setenv" | "setdebug")
                    && self.has_except(&c.behaviour, source)
                {
                    return Outcome::Rejected;
                }
                if !RESTRICTIONS.contains(&c.behaviour.as_str()) || (c.strict && !supports_strict(&c.behaviour)) || !valid_option(&c) {
                    return Outcome::Rejected;
                }
                let rule = Rule::from(&c);
                let changed = if c.param == Param::Add {
                    if self.rules.len() >= 1024 && !self.rules.contains_key(&source) {
                        return Outcome::Rejected;
                    }
                    let rules = self.rules.entry(source).or_default();
                    if rules.len() >= 256 && !rules.contains(&rule) {
                        return Outcome::Rejected;
                    }
                    rules.insert(rule)
                } else {
                    self.rules.get_mut(&source).is_some_and(|rules| rules.remove(&rule))
                };
                if self.rules.get(&source).is_some_and(BTreeSet::is_empty) {
                    self.rules.remove(&source);
                }
                if changed {
                    self.revision = self.revision.wrapping_add(1);
                } else {
                    outcome = if c.param == Param::Add {
                        Outcome::Duplicate
                    } else {
                        Outcome::Unset
                    };
                }
            }
            Param::Clear => {
                outcome = Outcome::Unset;
                if let Some(rules) = self.rules.get_mut(&source) {
                    let old = rules.len();
                    rules.retain(|r| !rule_text(r).contains(&c.option));
                    if rules.len() != old {
                        self.revision = self.revision.wrapping_add(1);
                        outcome = Outcome::Processed;
                    }
                }
                if self.rules.get(&source).is_some_and(BTreeSet::is_empty) {
                    self.rules.remove(&source);
                }
            }
            Param::Force
                if !c.strict
                    && (FORCES.contains(&c.behaviour.as_str()) || environment::supports(&c.behaviour, true))
                    && self.pending.len() < 256 =>
            {
                self.pending.push((source, c));
                outcome = Outcome::Queued;
            }
            Param::Reply(channel)
                if ((channel > 0 && channel != i32::MAX) || (channel == 0 && self.diagnostics.console_source == Some(source)))
                    && !c.strict
                    && (QUERIES.contains(&c.behaviour.as_str())
                        || environment::supports(&c.behaviour, false)
                        || c.behaviour.starts_with("getdebug_")) =>
            {
                let reply = match c.behaviour.as_str() {
                    "version" | "versionnew" | "versionnum" => self.version.query(source, &c),
                    "getstatus" => Some(self.status(Some(source), &c.option)),
                    "getstatusall" => Some(self.status(None, &c.option)),
                    "getcommand" => Some(command_list_with(&c.option, &self.preferences)),
                    _ => None,
                };
                if let Some(text) = reply {
                    self.reply(channel, text);
                } else if self.pending.len() < 256 {
                    self.pending.push((source, c));
                    outcome = Outcome::Queued;
                } else {
                    outcome = Outcome::Rejected;
                }
            }
            _ => outcome = Outcome::Rejected,
        }
        if was_environment_locked && !self.has("setenv") {
            self.clear_environment();
        }
        if was_fov_locked && !self.has("setcam_fovmin") && !self.has("setcam_fovmax") {
            self.camera_values.remove("setcam_fov");
        }
        for name in ["setsphere", "setoverlay"] {
            let active = self
                .rules
                .get(&source)
                .is_some_and(|rs| rs.iter().any(|r| r.behaviour == name && r.option.is_empty()));
            self.visuals.activate(source, name, active);
        }
        outcome
    }

    pub fn reply(&mut self, channel: i32, message: String) {
        if self.enabled && channel == 0 && self.diagnostics.console_source.is_some() {
            self.diagnostics.push_reply(message);
            return;
        }
        if self.enabled && channel > 0 && channel != i32::MAX && self.out.len() < 256 {
            self.out.push(NetCommand::ChatReply { channel, message });
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
                .any(|rules| rules.iter().any(|r| canonical(&r.behaviour) == canonical(behaviour) && r.general()))
    }

    pub fn has_except(&self, behaviour: &str, actor: Uuid) -> bool {
        self.enabled
            && self
                .rules
                .iter()
                .any(|(id, rules)| *id != actor && rules.iter().any(|r| canonical(&r.behaviour) == canonical(behaviour) && r.general()))
    }

    /// RlvHandler::isException: secure exceptions require the secure issuers;
    /// permissive=n expands that set to all issuers. Non-strict behaviours
    /// always share their exceptions, even under permissive=n.
    pub fn allows(&self, behaviour: &str, target: &str) -> bool {
        !self.enabled || !self.has(behaviour) || self.is_exception(behaviour, target)
    }

    fn is_exception(&self, behaviour: &str, target: &str) -> bool {
        if !self.enabled {
            return false;
        }
        let exception = |rs: &BTreeSet<Rule>| rs.iter().any(|r| r.behaviour == behaviour && r.option.eq_ignore_ascii_case(target));
        if !self.rules.values().any(exception) {
            return false;
        }
        let all_issuers = self.has("permissive");
        let strict = self.has(behaviour)
            && supports_strict(behaviour)
            && (all_issuers
                || self
                    .rules
                    .values()
                    .any(|rs| rs.iter().any(|r| r.behaviour == behaviour && r.general() && r.strict)));
        if strict {
            // Native object selection checks empty-option restrictions.
            // A range is reference-counted but supplies no required object
            // here; at least one actual matching exception is still needed.
            self.rules
                .values()
                .filter(|rs| {
                    rs.iter()
                        .any(|r| r.behaviour == behaviour && r.option.is_empty() && (all_issuers || r.strict))
                })
                .all(exception)
        } else {
            true
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
        self.allows_im_at(name, target, None)
    }

    pub fn allows_im_at(&self, name: &str, target: Uuid, distance: Option<f32>) -> bool {
        if !self.enabled {
            return true;
        }
        let id = target.to_string();
        let specific = match name {
            "sendim" => "sendimto",
            "startim" => "startimto",
            _ => "recvimfrom",
        };
        let mut ranges = self.rules_for(name, Uuid::nil()).filter_map(|(_, option)| im_range(option));
        let range = ranges
            .next()
            .map(|first| ranges.fold(first, |(min, max), r| (min.max(r.0), max.min(r.1))));
        let within = range.is_some_and(|(min, max)| {
            let d = distance.unwrap_or(f32::MAX);
            min < max && min <= d && d <= max
        });
        (self.allows(name, &id) || within) && !self.targets(specific, &id)
    }

    pub fn nametag_visible(&self, target: Uuid, own: Uuid, distance: f32) -> bool {
        if !self.enabled() {
            return true;
        }
        if target == own || self.allows("shownametags", &target.to_string()) {
            return true;
        }
        let radius = self
            .rules_for("shownametags", Uuid::nil())
            .filter_map(|(_, v)| v.parse::<f32>().ok())
            .reduce(f32::min)
            .unwrap_or(0.0);
        radius > 0.0 && distance < radius
    }

    pub fn allows_channel(&self, channel: i32) -> bool {
        self.allows("sendchannel", &channel.to_string())
            && (!self.has("sendchannel_except") || !self.is_exception("sendchannel_except", &channel.to_string()))
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

#[cfg(test)]
fn command_list(option: &str) -> String {
    command_list_with(option, &preferences::Preferences::default())
}

fn command_list_with(option: &str, preferences: &preferences::Preferences) -> String {
    let mut parts = option.splitn(3, ';');
    let filter = parts.next().unwrap_or_default();
    let kind = parts.next().unwrap_or("any");
    let separator = parts.next().unwrap_or(";");
    let mut names = BTreeSet::new();
    for (name, list) in [("add", RESTRICTIONS), ("force", FORCES), ("reply", QUERIES)] {
        if kind.is_empty() || kind == "any" || kind == name {
            names.extend(
                list.iter()
                    .filter(|n| n.contains(filter) && preferences.command_available(n))
                    .map(|n| (*n).to_owned()),
            );
            if name == "add" {
                names.extend(
                    list.iter()
                        .filter(|n| supports_strict(n) && preferences.command_available(n))
                        .map(|n| format!("{n}_sec"))
                        .filter(|n| n.contains(filter)),
                );
            }
        }
    }
    for prefix in ["getenv_", "setenv_"] {
        if !(kind.is_empty() || kind == "any" || kind == if prefix == "getenv_" { "reply" } else { "force" }) {
            continue;
        }
        for name in environment::command_names(prefix == "setenv_") {
            if name.contains(filter) {
                names.insert(name);
            }
        }
    }
    if matches!(kind, "" | "any" | "force") {
        names.extend(
            effects::MODIFIERS
                .iter()
                .filter(|name| name.contains(filter))
                .map(|name| (*name).to_owned()),
        );
    }
    names.into_iter().collect::<Vec<_>>().join(separator)
}

pub fn is_emote(text: &str) -> bool {
    text.starts_with("/me ") || text.starts_with("/me'")
}

#[cfg(test)]
#[path = "rlva/tests.rs"]
mod tests;
