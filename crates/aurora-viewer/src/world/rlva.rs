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

mod boundary;
pub mod camera;
mod chat;
mod collection;
pub(super) mod communication;
mod controllers;
mod debug;
pub mod diagnostics;
mod dictionary;
mod editing;
pub mod effects;
pub mod environment;
mod events;
mod force_wear;
mod groups;
mod im_queries;
mod interaction;
mod inventory;
pub(crate) mod legacy_strings;
pub mod login;
pub mod modifiers;
mod naming;
pub(crate) mod numeric;
pub mod offers;
pub(crate) mod policy;
pub use policy::DoubleClickRlva;
pub mod preferences;
pub mod privacy;
mod relative;
mod rename;
mod sitting;
pub mod startup;
mod teleport;
mod temporary;
mod version;
mod wardrobe;
pub mod watchdog;
mod wear_locks;

pub use boundary::{Boundary, command_name, refusal_shown, refused_reply};
pub(crate) use inventory::shared_folder_exists;
pub use inventory::{LAYERS, POINTS};
use numeric::option_bool;
pub use wear_locks::WearLocks;

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
    /// `@setenv=n` was held when this force was queued.
    pub environment_locked: bool,
    /// The rules it was received under, once queued.
    pub received: Received,
}

/// Every object's rules at one moment.
#[derive(Debug)]
pub struct LockSnapshot {
    revision: u64,
    rules: BTreeMap<Uuid, BTreeSet<Rule>>,
}

/// The rules a queued command was received under. RlvHandler::processCommand
/// (rlvhandler.cpp) runs a force or a query when it arrives, so a lock set
/// or released after it, later in the same message included, changes
/// nothing for it; here such a command waits for the frame's turn, or for an
/// outfit change to end, and reads those rules then (`Rlva::rewind`). Not a
/// part of what the command says: it takes no part in comparisons.
#[derive(Debug, Clone, Default)]
pub struct Received(Option<std::sync::Arc<LockSnapshot>>);

impl PartialEq for Received {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Received {}

/// The current rules, set aside while a queued command reads those it was
/// received under (`Rlva::rewind`, `Rlva::resume`).
pub struct LiveRules {
    revision: u64,
    rules: BTreeMap<Uuid, BTreeSet<Rule>>,
}

impl Command {
    pub fn parse(text: &str) -> Option<Self> {
        if !text.contains('=') && !text.eq_ignore_ascii_case("clear") {
            return None;
        }
        let (left, value) = text.split_once('=').unwrap_or((text, ""));
        let (name, option) = left.split_once(':').unwrap_or((left, ""));
        // RlvCommand::parseCommand (rlvhelper.cpp) takes any text before the
        // option as the behaviour: an unknown one (" fly", "foo1") is a valid
        // command that fails, which the @notify listeners still hear.
        if name.is_empty() {
            return None;
        }
        let name = name.to_ascii_lowercase();
        let name = name.as_str();
        let param = match (name, value) {
            (_, "n" | "add") => Param::Add,
            (_, "y" | "rem") => Param::Remove,
            ("clear", _) => Param::Clear,
            (_, "force") => Param::Force,
            // RlvCommand::RlvCommand: a reply when LLStringUtil::convertToS32
            // reads the parameter (blanks around it, text after the number).
            (_, v) => Param::Reply(numeric::convert_to_s32(v)?),
        };
        // RlvExtGetSet::processCommand (rlvextensions.cpp) takes everything
        // after "getdebug_" / "setdebug_" as the setting's name, a "_sec" end
        // included: `@getdebug_x_sec=5` asks for the setting "x_sec".
        let debug = name.starts_with("getdebug_") || name.starts_with("setdebug_");
        let (behaviour, strict) = match name.strip_suffix("_sec") {
            Some(n) if !debug => (n, true),
            _ => (name, false),
        };
        Some(Self {
            behaviour: behaviour.to_owned(),
            option: if param == Param::Clear { value } else { option }.to_owned(),
            strict,
            param,
            environment_locked: false,
            received: Received::default(),
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
            || (self.behaviour == "shownametags" && numeric::uuid_set(&self.option).is_none() && numeric::stof(&self.option).is_some())
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

/// RlvBehaviourRecvSendStartIMHandler::onCommand (rlvhandler.cpp): an option
/// that is not a key (LLUUID::set, tried first) is `<dist_min>[;<dist_max>]`,
/// each read by `std::stof` and not negative; a `;` needs a maximum after it.
/// A NaN passes the Legacy test (`nan < 0` is false) and is kept; it never
/// compares as within.
fn im_range(text: &str) -> Option<(f32, f32)> {
    if numeric::uuid_set(text).is_some() {
        return None;
    }
    let mut tokens = text.split(';');
    let min = numeric::stof(tokens.next()?)?;
    let max = match tokens.next() {
        Some(max) => numeric::stof(max)?,
        None => f32::MAX,
    };
    let not_negative = |v: f32| v.is_nan() || v >= 0.0;
    (tokens.next().is_none() && not_negative(min) && not_negative(max)).then_some((min, max))
}

/// RlvUtil::isValidReplyChannel (rlvcommon.h), without the loopback.
fn valid_reply_channel(channel: i32) -> bool {
    channel > 0 && channel != i32::MAX
}

/// Memory bounds the Legacy Viewer does not have (RlvObject::addCommand,
/// RlvHandler::processCommand, rlvhelper.cpp and rlvhandler.cpp keep
/// everything). They sit far above what scripts do, so that only a flood of
/// synthetic events can reach them: an OwnerSay longer than simulator chat
/// allows, issuers and rules per issuer by the ten thousand, and commands or
/// replies waiting for the frame's turn.
const MAX_MESSAGE: usize = 4096;
const MAX_ISSUERS: usize = 65_536;
const MAX_RULES: usize = 65_536;
const MAX_QUEUED: usize = 65_536;
/// The behaviour of a queued @notify line (`Rlva::notice_reply`): a comma
/// cannot be in a behaviour read from chat, which is split on commas.
const NOTICE: &str = "notify,queued";

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

/// The folder locks and their exceptions (`@attachthis`, `@detachallthis_except`…).
fn folder_lock(name: &str) -> bool {
    matches!(
        name,
        "attachthis"
            | "attachallthis"
            | "attachthis_except"
            | "attachallthis_except"
            | "detachthis"
            | "detachallthis"
            | "detachthis_except"
            | "detachallthis_except"
    )
}

/// Whether a restriction's option is one its handler accepts, read as the
/// Legacy Viewer reads it (`numeric`): RlvCommandOptionHelper::parseOption
/// for keys (LLUUID::set: empty is the null key) and numbers (`std::stoi`,
/// `std::stof`), RlvBehaviourModifier::convertOptionValue for modifiers
/// (rlvhelper.cpp), and the handlers of rlvhandler.cpp named below.
fn valid_option(c: &Command) -> bool {
    use numeric::{parse_uuid, scan_floats, stof, stoi, uuid_set};
    let option = c.option.as_str();
    // RlvBehaviourCamZoomMinMaxHandler::onCommand: any float, or none.
    if matches!(c.behaviour.as_str(), "camzoommin" | "camzoommax") {
        return option.is_empty() || stof(option).is_some();
    }
    match canonical(&c.behaviour) {
        "attachthis"
        | "attachallthis"
        | "attachthis_except"
        | "attachallthis_except"
        | "detachthis"
        | "detachallthis"
        | "detachthis_except"
        | "detachallthis_except" => {
            // RlvHandler::onAddRemFolderLock and onAddRemFolderLockException
            // (rlvhandler.cpp) read the option with the generic parser: a
            // lock takes nothing, a layer, a point or a shared folder, never
            // a UUID (LLUUID::set); an exception takes a shared folder only.
            // Whether the folder exists is asked by `receive_with`, which
            // sees the inventory.
            let named = inventory::layer(option).is_some() || inventory::point(option).is_some();
            !option.split('/').any(|v| matches!(v, "." | ".."))
                && (option.is_empty() || uuid_set(option).is_none())
                && (!c.behaviour.ends_with("_except") || (!option.is_empty() && !named))
        }
        "notify" => events::notification_option(option).is_some(),
        // RlvBehaviourGenericHandler<RLV_OPTION_MODIFIER>: a float modifier
        // takes any value `std::stof` reads, negative ones included.
        "setcam_fovmin"
        | "setcam_fovmax"
        | "setcam_eyeoffsetscale"
        | "setcam_avdist"
        | "setcam_avdistmin"
        | "setcam_avdistmax"
        | "setcam_origindistmin"
        | "setcam_origindistmax" => stof(option).is_some(),
        "setcam_eyeoffset" => scan_floats::<3>(option).is_some(),
        // The modifier's default is an LLVector3d (addModifier
        // RLV_MODIFIER_SETCAM_FOCUSOFFSET, rlvhelper.cpp), a type
        // convertOptionValue does not read: the Legacy Viewer fails every
        // `@setcam_focusoffset:<vector>=n|y` with an invalid option.
        "setcam_focusoffset" => false,
        // RLV_OPTION_NONE_OR_MODIFIER with a float modifier.
        "fartouch" | "sittp" | "tplocal" => option.is_empty() || stof(option).is_some(),
        // RLV_OPTION_NONE_OR_MODIFIER with a key modifier (LLUUID::parseUUID).
        "setcam_textures" => option.is_empty() || parse_uuid(option).is_some(),
        // RlvBehaviourGenericHandler<RLV_OPTION_EXCEPTION> and
        // <RLV_OPTION_NONE_OR_EXCEPTION>: LLUUID::set, where no option is
        // the null key: the restriction is stored, @getstatus lists it, and
        // it names no object.
        "touchthis" | "editobj" | "touchworld" | "touchattach" | "touchattachother" | "touchhud" | "edit" => uuid_set(option).is_some(),
        "addattach" | "remattach" | "detach" => option.is_empty() || inventory::point(option).is_some(),
        "addoutfit" | "remoutfit" => option.is_empty() || inventory::layer(option).is_some(),
        "recvim" | "sendim" | "startim" => uuid_set(option).is_some() || im_range(option).is_some(),
        // RlvBehaviourHandler<RLV_BHVR_SHOWNAMETAGS>: a key, else a float
        // modifier.
        "shownametags" => uuid_set(option).is_some() || stof(option).is_some(),
        "recvchat" | "recvemote" | "share" | "shownames" | "accepttp" | "accepttprequest" | "tplure" | "tprequest" => {
            uuid_set(option).is_some()
        }
        "showhovertext" => uuid_set(option).is_some(),
        // RlvBehaviourHandler<RLV_BHVR_SETOVERLAY_TOUCH>::onCommand
        // (rlvhandler.cpp) never reads its option: any one is accepted.
        "setoverlay_touch" => true,
        // (the same null key, naming no one; @sendimto's mere presence
        // still makes `rlva_can_send_session` check every session, as
        // FSFloaterIM::sendMsgFromInputEditor does)
        "recvchatfrom" | "recvemotefrom" | "recvimfrom" | "startimto" | "sendimto" => uuid_set(option).is_some(),
        // RlvBehaviourSendChannelHandler::onCommand: `std::stoi`, above 0.
        "sendchannel" | "sendchannel_except" => option.is_empty() || stoi(option).is_some_and(|v| v > 0),
        // RlvHandler::processAddRemCommand: LLStringUtil::convertToS32 and
        // RlvUtil::isValidReplyChannel.
        "redirchat" | "rediremote" => numeric::convert_to_s32(option).is_some_and(valid_reply_channel),
        _ => option.is_empty(),
    }
}

#[derive(Default)]
pub struct Rlva {
    enabled: bool,
    pub preferences: preferences::Preferences,
    pub rename: rename::RenameOnWear,
    pub offers: offers::Offers,
    /// The inventory's Delete being sent: its moves to the Trash are
    /// removals, judged by the remove rule (`rlva_inventory_delete_allowed`).
    pub deletes: std::collections::HashSet<Uuid>,
    pub controllers: controllers::Controllers,
    pub relative: relative::RelativeWear,
    pub version: version::Version,
    pub diagnostics: diagnostics::Diagnostics,
    pub im: im_queries::Queries,
    rules: BTreeMap<Uuid, BTreeSet<Rule>>,
    /// The rules of each object in the order it issued them, as
    /// RlvObject::m_Commands keeps them: what @getstatus lists.
    issued: BTreeMap<Uuid, Vec<Rule>>,
    /// Every rule in the order it was stored, whatever its object, as
    /// RlvHandler::addException (rlvhandler.cpp) appends to the
    /// m_Exceptions multimap: what @redirchat walks.
    issued_order: Vec<(Uuid, Rule)>,
    /// The rules as the last queued command found them, shared by every
    /// command queued until they change.
    snapshot: Option<std::sync::Arc<LockSnapshot>>,
    pub pending: Vec<(Uuid, Command)>,
    pub out: Vec<NetCommand>,
    pub appearance: Vec<(Uuid, super::appearance::Action)>,
    /// Attachments a forced batch asked for, not arrived yet (`rlva_force_settled`).
    pub requested_wears: std::collections::HashMap<Uuid, std::time::Instant>,
    pub viewer: Vec<(Uuid, Command)>,
    pub camera_values: BTreeMap<String, String>,
    pub groups: groups::Groups,
    pub sitting: sitting::Sitting,
    pub wardrobe: wardrobe::Wardrobe,
    pub visuals: effects::Visuals,
    pub watchdog: watchdog::Watchdog,
    /// The residents whose profile window is open, as the interface last
    /// showed them (LLAvatarActions::profileVisible, which
    /// RlvUIEnabler::hasOpenProfile asks, indra/newview/rlvui.cpp).
    pub open_profiles: std::collections::HashSet<Uuid>,
    pub(crate) locked_folders: inventory::LockedFoldersCache,
    pub(crate) wear_locks: wear_locks::WearLocksCache,
    pub recovery: Vec<NetCommand>,
    /// The ENV_EDIT instance RlvEnvironment works on while `@setenv=n` is
    /// held (getTargetEnvironment, rlvenvironment.cpp): its holder and
    /// fixed sky, its fixed water, its day. Without the lock RLVa writes
    /// into the user's own local environment (`eep::rlva_local_sky`).
    pub environment: Option<(Uuid, super::eep::SkyFrame)>,
    pub environment_water: Option<super::eep::WaterFrame>,
    pub environment_day: Option<std::sync::Arc<super::eep::DayCycle>>,
    /// The day of `environment_day` is held at the fixed sky of
    /// `environment` (DayInstance::NO_ANIMATE_SKY: a sky set over a day
    /// keeps the day in the instance, for the next `daytime`).
    pub environment_day_sky_fixed: bool,
    /// The user's field of view when the first `@setcam_fovmin` /
    /// `@setcam_fovmax` came, given back when the last goes
    /// (RlvBehaviourSetCamFovHandler's `s_nLastCameraAngle`).
    pub fov_before_limits: Option<f32>,
    /// `@setcam_unlock` and RLVa's own view (CAMERA_RLV_SETCAM_VIEW) as the
    /// camera last saw them (`camera_toggles`).
    camera_seen: (bool, bool),
    /// When each value of the modifiers without a comparator was added
    /// (`camera_rule`): the first one added wins.
    modifier_order: BTreeMap<(Uuid, String, String), u64>,
    modifier_seq: u64,
    pub environment_request: Option<(Uuid, Command)>,
    pub pseudo_sex: Option<String>,
    pub resolution_divisor: Option<u32>,
    pub appearance_busy: bool,
    pub self_detach: Option<(Uuid, Uuid)>,
    pub stand_origin: Option<(aurora_net::RegionHandle, glam::Vec3)>,
    pub stand_return: Option<(aurora_net::RegionHandle, glam::Vec3)>,
    /// @fly, @alwaysrun and @temprun as last seen (`rlva_movement_tick`).
    movement_rules: u8,
    /// A teleport the boundary already let through, sent again once the
    /// maturity preference is in sync: LLAgent::startTeleportRequest runs a
    /// pending request without asking RLVa again (`App::update`).
    pub teleport_already_checked: bool,
    /// What a script question RLVa stripped says once it is answered yes:
    /// the "rlv_blocked" of process_script_question's payload
    /// (llviewermessage.cpp), by task and item, the latest 16.
    pub(crate) permission_notices: Vec<(Uuid, Uuid, String)>,
    inventory_fetches: BTreeMap<Uuid, (std::time::Instant, u8)>,
    /// The inventory entry (a link, or the item) last asked to be worn for
    /// an item, with its folder: the wear locks read that folder
    /// (`rlva_can_add_in`).
    pub(crate) wear_entries: std::collections::HashMap<Uuid, (Uuid, std::time::Instant)>,
    pub revision: u64,
    gc: Option<std::time::Instant>,
    lookups: BTreeMap<Uuid, (bool, u8)>,
    startup: startup::Startup,
    /// The RestrainedLoveDebug line of the last OwnerSay, shown in nearby
    /// chat as the object's (process_chat_from_simulator).
    debug_line: Option<String>,
    /// The floating texts as they were filtered (`HoverTexts`).
    pub hover_texts: HoverTexts,
}

/// The floating texts as LLHUDText::setString (indra/newview/llhudtext.cpp)
/// left them: filtered once, when the text comes, with the names and places
/// hidden at that moment, and filtered again only when one of the
/// restrictions LLHUDText::refreshAllObjectText follows comes or goes
/// (@shownames and its exceptions, @shownearby, @showloc,
/// @showhovertext*: RlvUIEnabler::onRefreshHoverText, rlvui.cpp, and the
/// @shownames / @shownearby handlers, rlvhandler.cpp). A resident who comes
/// around later keeps their name in a text already shown, as there.
#[derive(Default)]
pub struct HoverTexts {
    /// The rules' revision the key was read at, and the key: the monitored
    /// restrictions.
    key: std::sync::Mutex<(u64, Option<Vec<String>>)>,
    /// By object: the text received, and the text shown.
    shown: std::sync::Mutex<std::collections::HashMap<Uuid, (String, String)>>,
}

impl HoverTexts {
    fn shown(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<Uuid, (String, String)>> {
        self.shown.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The text `object` shows for `raw`, `filter` run when it changed or
    /// a monitored restriction came or went since.
    pub fn get(&self, rlva: &Rlva, object: Uuid, raw: &str, filter: impl FnOnce(&str) -> String) -> String {
        {
            let mut key = self.key.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if key.1.is_none() || key.0 != rlva.revision {
                let now = Some(rlva.hover_refresh_key());
                if key.1 != now {
                    self.shown().clear();
                }
                *key = (rlva.revision, now);
            }
        }
        if let Some((received, text)) = self.shown().get(&object)
            && received == raw
        {
            return text.clone();
        }
        // (filtered unlocked: the filter reads the world, never this cache)
        let text = filter(raw);
        self.shown().insert(object, (raw.to_owned(), text.clone()));
        text
    }

    /// Forget the objects `keep` refuses (those gone).
    pub fn retain(&self, keep: impl Fn(&Uuid) -> bool) {
        self.shown().retain(|id, _| keep(id));
    }

    /// How many objects' texts are kept.
    pub fn count(&self) -> usize {
        self.shown().len()
    }
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

    /// What LLHUDText::refreshAllObjectText follows (`HoverTexts`): whether
    /// RLVa is on, the general @shownames, @shownearby, @showloc and
    /// @showhovertext* restrictions, and the @shownames exceptions and
    /// @showhovertext objects.
    fn hover_refresh_key(&self) -> Vec<String> {
        let mut key = vec![self.enabled.to_string()];
        for behaviour in [
            "shownames",
            "shownearby",
            "showloc",
            "showhovertextall",
            "showhovertextworld",
            "showhovertexthud",
        ] {
            if self.has(behaviour) {
                key.push(behaviour.to_owned());
            }
        }
        let mut options: Vec<String> = ["shownames", "showhovertext"]
            .iter()
            .flat_map(|b| {
                self.rules_for(b, Uuid::nil())
                    .filter(|(_, option)| numeric::uuid_set(option).is_some_and(|id| !id.is_nil()))
                    .map(move |(_, option)| format!("{b}:{option}"))
            })
            .collect();
        options.sort();
        options.dedup();
        key.extend(options);
        key
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
        self.startup = Default::default();
        self.debug_line = None;
        self.rules.clear();
        self.issued.clear();
        self.issued_order.clear();
        self.modifier_order.clear();
        self.snapshot = None;
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
        self.movement_rules = 0;
        self.inventory_fetches.clear();
        self.wear_entries.clear();
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
        if self.environment_request.as_ref().is_some_and(|(id, _)| *id == source) {
            self.environment_request = None;
        }
        self.lookups.remove(&source);
        self.issued.remove(&source);
        self.issued_order.retain(|(id, _)| *id != source);
        self.modifier_order.retain(|(id, _, _), _| *id != source);
        if self.rules.remove(&source).is_some() {
            self.revision = self.revision.wrapping_add(1);
        }
        // RlvBehaviourToggleHandler<RLV_BHVR_SETENV> (rlvhandler.cpp): ENV_EDIT
        // goes with the last `@setenv=n`, not with the object that wrote it;
        // a force made without the lock is the user's own local environment
        // and stays
        if !self.has("setenv") && (self.environment.is_some() || self.environment_water.is_some() || self.environment_day.is_some()) {
            let request = self.environment_request.take();
            self.clear_environment();
            self.environment_request = request;
        }
        if was_fov_locked && !self.has("setcam_fovmin") && !self.has("setcam_fovmax") {
            self.camera_values.remove("setcam_fov");
        }
        self.pending.retain(|(id, _)| *id != source);
        self.appearance.retain(|(id, _)| *id != source);
        self.viewer.retain(|(id, _)| *id != source);
    }

    /// An issuer that is gone, detached (RlvHandler::onDetach) or collected
    /// (RlvHandler::onGC): both run processCommand(id, "clear", true)
    /// (rlvhandler.cpp), which the @notify listeners of the other objects
    /// hear as "/clear" (RlvBehaviourNotifyHandler::onCommand, rlvhelper.cpp).
    pub fn release(&mut self, source: Uuid) {
        let held = self.rules.contains_key(&source);
        self.forget(source);
        if held {
            self.notify("clear");
        }
    }

    /// The viewer clears an issuer itself: RlvFloaterConsole::onClose
    /// (rlvfloaters.cpp) for the console, RlvHandler::
    /// getAttachmentResourcesCoro (rlvhandler.cpp) for a blocked experience's
    /// object. Both run processCommand(id, "clear", true), whose "/clear"
    /// the @notify listeners hear even when it had nothing to clear.
    pub(crate) fn clear_issuer(&mut self, source: Uuid) {
        self.forget(source);
        self.notify("clear");
    }

    /// The last rule of an issuer went: its RlvObject is erased
    /// (RlvHandler::processCommand), and with it the order of its rules and
    /// what the collector knew of it.
    fn drop_if_empty(&mut self, source: Uuid) {
        if self.rules.get(&source).is_some_and(BTreeSet::is_empty) {
            self.rules.remove(&source);
            self.issued.remove(&source);
            self.issued_order.retain(|(id, _)| *id != source);
            self.lookups.remove(&source);
        }
    }

    /// RlvObject::RlvObject (rlvhelper.cpp) looks its object up when its
    /// first restriction is stored (m_fLookup), not at the first collection:
    /// an issuer seen then and gone before that collection expires at once
    /// instead of after twenty misses. Run when rules were just stored;
    /// `collect` covers the issuers that came another way.
    pub fn resolve_sources(&mut self, present: impl Fn(Uuid) -> bool) {
        if self.lookups.len() == self.rules.len() {
            return;
        }
        for &id in self.rules.keys() {
            self.lookups.entry(id).or_insert_with(|| (present(id), 0));
        }
    }

    /// RlvHandler::onGC runs every 30 seconds. Unknown issuers get twenty
    /// lookup attempts; an object resolved once expires on its next absence.
    pub fn collect(&mut self, now: std::time::Instant, present: impl Fn(Uuid) -> bool) {
        if !self.enabled {
            return;
        }
        self.resolve_sources(&present);
        if self.gc.is_some_and(|last| now.saturating_duration_since(last).as_secs() < 30) {
            return;
        }
        self.gc = Some(now);
        let mut expired = Vec::new();
        for (&id, entry) in &mut self.lookups {
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
            self.release(id);
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

    /// The smallest distance of a RLV_OPTION_NONE_OR_MODIFIER restriction
    /// (RlvBehaviourModifierCompMin, rlvhelper.cpp): `std::stof` of each
    /// option, the default for one without (the modifier is added with
    /// `fAddDefaultOnEmpty`). A NaN, which the Legacy comparisons would
    /// place anywhere, counts as the default.
    pub fn distance_limit(&self, name: &str, default: f32, actor: Uuid) -> f32 {
        self.rules_for(name, actor)
            .map(|(_, option)| numeric::stof(option).filter(|v| !v.is_nan()).unwrap_or(default))
            .fold(f32::INFINITY, f32::min)
    }

    /// Returns true for an RLVa OwnerSay, including unsupported commands:
    /// protocol traffic never becomes user chat when RLVa is enabled. The
    /// chat handler passes the shared folders (`receive_with`).
    #[cfg(test)]
    pub fn receive(&mut self, chat: &ChatMessage) -> bool {
        self.receive_with(chat, None)
    }

    /// `receive`, with `shared_folder` answering whether a path names a
    /// folder under #RLV: a folder lock or exception naming a folder that
    /// does not exist is refused when it is issued, set or released
    /// (RlvCommandOptionGeneric::isSharedFolder in onAddRemFolderLock and
    /// onAddRemFolderLockException, rlvhandler.cpp).
    pub fn receive_with(&mut self, chat: &ChatMessage, shared_folder: Option<&dyn Fn(&str) -> bool>) -> bool {
        if !self.enabled
            || chat.source_type != ChatSourceType::Object
            || chat.chat_type != ChatType::OwnerSay
            || chat.source_id.is_nil()
            || !chat.message.starts_with('@')
            || chat.message.len() <= 3
        {
            return false;
        }
        // Simulator chat is at most 1023 bytes (MAX_MSG_STR_LEN): this bound
        // only keeps a synthetic event from allocating without end.
        if chat.message.len() > MAX_MESSAGE {
            return true;
        }
        // process_chat_from_simulator folds the complete OwnerSay command,
        // including options/parameters, before RlvCommand parsing.
        let message = chat.message[1..].to_ascii_lowercase();
        let mut debug = diagnostics::DebugLine::default();
        // (boost::char_separator with drop_empty_tokens: an empty command
        // is no command)
        for text in message.split(',').filter(|text| !text.is_empty()) {
            let outcome = if self.retains() {
                self.retain(chat, text);
                debug.push(diagnostics::Outcome::Retained, text, None);
                diagnostics::Outcome::Retained
            } else if let Some(command) = Command::parse(text).filter(|command| {
                // RlvCommandOptionGeneric::isSharedFolder in onAddRemFolderLock
                // and onAddRemFolderLockException (rlvhandler.cpp): a folder
                // lock naming a shared folder that does not exist is refused.
                !shared_folder.is_some_and(|exists| {
                    matches!(command.param, Param::Add | Param::Remove)
                        && folder_lock(canonical(&command.behaviour))
                        && !command.option.is_empty()
                        && inventory::layer(&command.option).is_none()
                        && inventory::point(&command.option).is_none()
                        && !exists(&command.option)
                })
            }) {
                let notice = self.notice(chat.source_id, &command, text);
                let failure = self.failure(chat.source_id, Some(&command));
                let outcome = self.process(chat.source_id, command);
                if self.debug_lists(outcome) {
                    debug.push(outcome, text, failure);
                }
                if let Some((subject, suffix)) = notice {
                    self.notify_command(&subject, suffix);
                }
                outcome
            } else {
                if self.debug_lists(diagnostics::Outcome::Rejected) {
                    debug.push(diagnostics::Outcome::Rejected, text, self.failure(chat.source_id, None));
                }
                diagnostics::Outcome::Rejected
            };
            self.trace(chat.source_id, text, outcome);
        }
        // (RlvFloaterConsole::onInput runs its commands itself: no chat line)
        let console = self.diagnostics.console_source == Some(chat.source_id);
        self.debug_line = if self.preferences.debug_commands && !self.startup.replaying && !console {
            debug.finish(&message)
        } else {
            None
        };
        true
    }

    /// The RestrainedLoveDebug line of the OwnerSay `receive` just took, to
    /// show in nearby chat as the object's.
    pub fn take_debug_line(&mut self) -> Option<String> {
        self.debug_line.take()
    }

    /// The rules of this moment, for a command that is queued.
    fn received(&mut self) -> Received {
        if self.snapshot.as_ref().is_none_or(|s| s.revision != self.revision) {
            self.snapshot = Some(std::sync::Arc::new(LockSnapshot {
                revision: self.revision,
                rules: self.rules.clone(),
            }));
        }
        Received(self.snapshot.clone())
    }

    /// Puts back the rules a queued command was received under, when they
    /// changed since, for the time it runs. `resume` restores what this
    /// returns. The revision goes back with them, so that what is cached by
    /// revision (the folder and wear locks) is resolved for these rules,
    /// then again for the current ones.
    pub(crate) fn rewind(&mut self, command: &Command) -> Option<LiveRules> {
        let then = command.received.0.as_ref().filter(|then| then.revision != self.revision)?;
        Some(LiveRules {
            revision: std::mem::replace(&mut self.revision, then.revision),
            rules: std::mem::replace(&mut self.rules, then.rules.clone()),
        })
    }

    pub(crate) fn resume(&mut self, live: Option<LiveRules>) {
        if let Some(live) = live {
            self.rules = live.rules;
            self.revision = live.revision;
        }
    }

    /// A blocked object (anything but a release) and a command disabled by a
    /// setting: RlvHandler::processCommand (rlvhandler.cpp) returns
    /// RLV_RET_FAILED_BLOCKED / RLV_RET_FAILED_DISABLED before it runs the
    /// command or tells the @notify listeners.
    fn refused(&self, source: Uuid, c: &Command) -> bool {
        !self.controllers.accepts(source, c.param)
            || (c.behaviour == "setenv" && matches!(c.param, Param::Add | Param::Remove) && !self.preferences.allow_environment_locks)
    }

    /// What the @notify listeners are told about a restriction command, as
    /// the text the filters test and what follows it
    /// (RlvBehaviourNotifyHandler::onCommand, rlvhelper.cpp). RlvCommand::
    /// asString (rlvhelper.h) writes `@clear=<filter>` as "clear:<filter>".
    /// Refused commands tell no one; the ones that fail later still do.
    fn notice<'a>(&self, source: Uuid, c: &Command, text: &'a str) -> Option<(String, &'a str)> {
        if self.refused(source, c) {
            return None;
        }
        match c.param {
            Param::Add | Param::Remove => Some((c.restriction(), text.find('=').map_or("", |at| &text[at..]))),
            Param::Clear if c.option.is_empty() => Some(("clear".to_owned(), "")),
            Param::Clear => Some((format!("clear:{}", c.option), "")),
            Param::Force | Param::Reply(_) => None,
        }
    }

    fn process(&mut self, source: Uuid, c: Command) -> diagnostics::Outcome {
        use diagnostics::Outcome;
        if self.refused(source, &c)
            || (!self.preferences.command_available(&c.behaviour) && !matches!(c.param, Param::Remove | Param::Clear))
        {
            return Outcome::Rejected;
        }
        if effects::MODIFIERS.contains(&c.behaviour.as_str()) {
            // RlvBehaviourInfo::processModifier (rlvhelper.cpp): a modifier
            // of a behaviour its object does not hold fails
            // (RLV_RET_FAILED_UNHELDBEHAVIOUR); another rule is not enough,
            // or a later @setoverlay=n would start from values set before it.
            let base = if c.behaviour.starts_with("setoverlay_") {
                "setoverlay"
            } else {
                "setsphere"
            };
            let held = self
                .rules
                .get(&source)
                .is_some_and(|rs| rs.iter().any(|r| r.behaviour == base && r.option.is_empty()));
            if c.param == Param::Force && !c.strict && held {
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
                    // RlvHandler::processCommand: a removal of what the
                    // object does not hold is RLV_RET_SUCCESS_UNSET, whatever
                    // it names
                    return if c.param == Param::Remove {
                        Outcome::Unset
                    } else {
                        Outcome::Rejected
                    };
                }
                let rule = Rule::from(&c);
                let changed = if c.param == Param::Add {
                    if self.rules.len() >= MAX_ISSUERS && !self.rules.contains_key(&source) {
                        return Outcome::Rejected;
                    }
                    let rules = self.rules.entry(source).or_default();
                    if rules.len() >= MAX_RULES && !rules.contains(&rule) {
                        return Outcome::Rejected;
                    }
                    let added = rules.insert(rule.clone());
                    if added {
                        self.issued_order.push((source, rule.clone()));
                        self.note_modifier_added(source, &rule);
                        self.issued.entry(source).or_default().push(rule);
                    }
                    added
                } else {
                    let removed = self.rules.get_mut(&source).is_some_and(|rules| rules.remove(&rule));
                    if removed && let Some(order) = self.issued.get_mut(&source) {
                        order.retain(|r| *r != rule);
                    }
                    if removed {
                        self.issued_order.retain(|(id, r)| *id != source || *r != rule);
                    }
                    removed
                };
                self.drop_if_empty(source);
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
            // RlvHandler::processClearCommand returns RLV_RET_SUCCESS, with
            // something to clear or not.
            Param::Clear => {
                if let Some(rules) = self.rules.get_mut(&source) {
                    let old = rules.len();
                    rules.retain(|r| !rule_text(r).contains(&c.option));
                    if rules.len() != old {
                        self.revision = self.revision.wrapping_add(1);
                        if let Some(order) = self.issued.get_mut(&source) {
                            order.retain(|r| !rule_text(r).contains(&c.option));
                        }
                        self.issued_order
                            .retain(|(id, r)| *id != source || !rule_text(r).contains(&c.option));
                    }
                }
                self.drop_if_empty(source);
            }
            Param::Force
                if !c.strict
                    && (FORCES.contains(&c.behaviour.as_str()) || environment::supports(&c.behaviour, true))
                    && self.pending.len() < MAX_QUEUED =>
            {
                // A foreign environment lock refuses the force when received.
                // Releasing it before the queue drains must not revive that command.
                if was_environment_locked && c.behaviour.starts_with("setenv_") && self.has_except("setenv", source) {
                    return Outcome::Rejected;
                }
                let received = self.received();
                self.pending.push((
                    source,
                    Command {
                        environment_locked: was_environment_locked,
                        received,
                        ..c
                    },
                ));
                outcome = Outcome::Queued;
            }
            Param::Reply(channel)
                if ((channel > 0 && channel != i32::MAX) || (channel == 0 && self.diagnostics.console_source == Some(source)))
                    && !c.strict
                    && (QUERIES.contains(&c.behaviour.as_str())
                        || environment::supports(&c.behaviour, false)
                        || (c.behaviour.starts_with("getdebug_") && c.behaviour.len() > "getdebug_".len())) =>
            {
                // RlvHandler::processCommand answers every query as it comes,
                // so a script reading one channel gets the replies in order:
                // one answered here waits behind the queued ones.
                let reply = if self.pending.is_empty() && self.viewer.is_empty() {
                    self.inline_reply(source, &c)
                } else {
                    None
                };
                if let Some(text) = reply {
                    self.reply(channel, &c.behaviour, text);
                } else if self.pending.len() < MAX_QUEUED {
                    let received = self.received();
                    self.pending.push((source, Command { received, ..c }));
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

    /// The queries answered from RLVa's own state: @version*, @getstatus*
    /// and @getcommand (RlvHandler::processReplyCommand, rlvhandler.cpp).
    pub(crate) fn inline_reply(&self, source: Uuid, c: &Command) -> Option<String> {
        match c.behaviour.as_str() {
            "version" | "versionnew" | "versionnum" => self.version.query(source, c),
            "getstatus" => Some(self.status(Some(source), &c.option)),
            "getstatusall" => Some(self.status(None, &c.option)),
            "getcommand" => Some(command_list_with(&c.option, &self.preferences)),
            NOTICE => Some(c.option.clone()),
            _ => None,
        }
    }

    /// `behaviour` is the query answered (the console masks a texture id).
    pub fn reply(&mut self, channel: i32, behaviour: &str, message: String) {
        let behaviour = if behaviour == NOTICE { "notify" } else { behaviour };
        // RlvExtGetSet and RlvEnvironment answer through RlvUtil::
        // sendChatReply (rlvextensions.cpp, rlvenvironment.cpp), which refuses
        // channel 0: the console gets no answer to them.
        if channel == 0 && (behaviour.starts_with("getdebug_") || behaviour.starts_with("getenv_")) {
            return;
        }
        if self.enabled && channel == 0 && self.diagnostics.console_source.is_some() {
            self.diagnostics.push_reply(behaviour, message);
            return;
        }
        if self.enabled && channel > 0 && channel != i32::MAX && self.out.len() < MAX_QUEUED {
            self.out.push(NetCommand::ChatReply { channel, message });
        }
    }

    /// rlvParseGetStatusOption (rlvhandler.cpp): the filter, then the
    /// separator; what follows a second `;` is ignored. Each object lists its
    /// rules in the order it issued them (RlvObject::getStatusString,
    /// rlvhelper.cpp), the objects in the order of their keys.
    fn status(&self, source: Option<Uuid>, option: &str) -> String {
        let mut tokens = option.split(';');
        let filter = tokens.next().unwrap_or_default();
        let separator = tokens.next().filter(|s| !s.is_empty()).unwrap_or("/");
        let rules: Vec<_> = self
            .issued
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
        // RlvBehaviourSendChannelHandler::onCommand (rlvhandler.cpp) stores
        // the channel as a number: `@sendchannel:05=n` is channel 5.
        let channel = matches!(behaviour, "sendchannel" | "sendchannel_except")
            .then(|| numeric::stoi(target))
            .flatten();
        let exception = |rs: &BTreeSet<Rule>| {
            rs.iter().any(|r| {
                r.behaviour == behaviour
                    && (option_names(&r.option, target) || channel.is_some_and(|c| numeric::stoi(&r.option) == Some(c)))
            })
        };
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
                .any(|rs| rs.iter().any(|r| r.behaviour == behaviour && option_names(&r.option, target)))
    }

    /// `targets` for @recvchatfrom, @recvemotefrom, @recvimfrom, @sendimto
    /// and @startimto, which RlvActions (indra/newview/rlvactions.cpp) and
    /// RlvHandler::filterChat read through RlvHandler::isException
    /// (indra/newview/rlvhandler.cpp): under @permissive=n, or once one of
    /// them was set with `_sec`, every object holding the bare rule (the
    /// null key; under `_sec` alone, a `_sec` one) must name the resident
    /// too, or none does.
    pub(crate) fn targets_strictly(&self, behaviour: &str, target: &str) -> bool {
        if !self.targets(behaviour, target) {
            return false;
        }
        let all_issuers = self.has("permissive");
        let strict = all_issuers
            || self
                .rules
                .values()
                .any(|rs| rs.iter().any(|r| r.behaviour == behaviour && r.strict));
        !strict
            || self
                .rules
                .values()
                .filter(|rs| {
                    rs.iter()
                        .any(|r| r.behaviour == behaviour && r.option.is_empty() && (all_issuers || r.strict))
                })
                .all(|rs| rs.iter().any(|r| r.behaviour == behaviour && r.option.eq_ignore_ascii_case(target)))
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
        (self.allows(name, &id) || within) && !self.targets_strictly(specific, &id)
    }

    pub fn nametag_visible(&self, target: Uuid, own: Uuid, distance: f32) -> bool {
        if !self.enabled() {
            return true;
        }
        if target == own || self.allows("shownametags", &target.to_string()) {
            return true;
        }
        // RlvActions::canShowNameTag (rlvactions.cpp): the smallest distance
        // (RLV_MODIFIER_SHOWNAMETAGSDIST, default 0 for a restriction without
        // one), not 0, compared squared, so that a negative distance works
        // as its opposite. A key is an exception, not a distance; a NaN
        // counts as no distance.
        let radius = self
            .rules_for("shownametags", Uuid::nil())
            .filter_map(|(_, v)| match v {
                "" => Some(0.0),
                v if numeric::uuid_set(v).is_some() => None,
                v => Some(numeric::stof(v).filter(|v| !v.is_nan()).unwrap_or(0.0)),
            })
            .reduce(f32::min)
            .unwrap_or(0.0);
        radius != 0.0 && distance * distance < radius * radius
    }

    pub fn allows_channel(&self, channel: i32) -> bool {
        self.allows("sendchannel", &channel.to_string())
            && (!self.has("sendchannel_except") || !self.is_exception("sendchannel_except", &channel.to_string()))
    }
}

/// Whether a restriction's option names `target`. The Legacy Viewer stores
/// an exception as the key LLUUID::set read (RlvBehaviourGenericHandler<
/// RLV_OPTION_EXCEPTION>::onCommand, rlvhandler.cpp), so the old 35-character
/// form and any character in the dash places name the same object.
fn option_names(option: &str, target: &str) -> bool {
    option.eq_ignore_ascii_case(target)
        || (!option.is_empty() && Uuid::parse_str(target).is_ok_and(|id| numeric::uuid_set(option) == Some(id)))
}

fn rule_text(r: &Rule) -> String {
    Command {
        behaviour: r.behaviour.clone(),
        option: r.option.clone(),
        strict: r.strict,
        param: Param::Add,
        environment_locked: false,
        received: Received::default(),
    }
    .restriction()
}

#[cfg(test)]
fn command_list(option: &str) -> String {
    command_list_with(option, &preferences::Preferences::default())
}

/// RlvReplyHandler<RLV_BHVR_GETCOMMAND>::onCommand (indra/newview/rlvhandler.cpp):
/// `@getcommand[:<filter>[;<type>[;<separator>]]]`, the option split on every
/// ';' by parseStringList. A type other than any, add, force or reply fails
/// the option, and the reply is empty, as Legacy still answers a failed reply
/// command. The experimental entries follow the setting read when RLVa
/// started, as the dictionary is built then.
fn command_list_with(option: &str, preferences: &preferences::Preferences) -> String {
    let tokens: Vec<&str> = if option.is_empty() {
        Vec::new()
    } else {
        option.split(';').collect()
    };
    let kind = match tokens.get(1).copied() {
        None | Some("" | "any") => 0,
        Some("add") => dictionary::ADD,
        Some("force") => dictionary::FORCE,
        Some("reply") => dictionary::REPLY,
        Some(_) => return String::new(),
    };
    let filter = tokens.first().copied().unwrap_or_default();
    let separator = tokens.get(2).copied().unwrap_or(";");
    dictionary::commands(filter, kind, preferences.experimental_commands).join(separator)
}

/// RlvUtil::isEmote (indra/newview/rlvcommon.h): more than the four bytes
/// of "/me " or "/me'", so that "/me " alone is a short "/" line.
pub fn is_emote(text: &str) -> bool {
    text.len() > 4 && (text.starts_with("/me ") || text.starts_with("/me'"))
}

#[cfg(test)]
#[path = "rlva/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rlva/link_lock_tests.rs"]
pub(crate) mod link_lock_tests;

#[cfg(test)]
#[path = "rlva/outfit_swap_tests.rs"]
mod outfit_swap_tests;

#[cfg(test)]
#[path = "rlva/core_parity_tests.rs"]
mod core_parity_tests;
#[cfg(test)]
#[path = "rlva/inventory_parity_tests.rs"]
mod inventory_parity_tests;
#[cfg(test)]
#[path = "rlva/movement_tests.rs"]
mod movement_tests;
#[cfg(test)]
#[path = "rlva/privacy_map_tests.rs"]
mod privacy_map_tests;
