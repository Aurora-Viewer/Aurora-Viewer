//! `secondlife:///app/...` commands written in a text (SLapps): what each
//! one asks for, read as the Legacy Viewer's command handlers read it
//! (LLCommandHandler subclasses: LLGroupHandler, LLParcelHandler,
//! LLObjectIMHandler, LLInventoryHandler, LLExperienceHandler,
//! LLClassifiedHandler, LLEventHandler, LLWorldMapHandler,
//! LLChatCommandHandler, LLSearchHandler, LLAgentHandler, LLWearFolderHandler…;
//! indra/newview/llgroupactions.cpp, llpanelplaces.cpp, llchathistory.cpp,
//! llviewerinventory.cpp, llfloaterexperienceprofile.cpp,
//! llpanelprofileclassifieds.cpp, lleventnotifier.cpp, llfloaterworldmap.cpp,
//! llfloaterimnearbychat.cpp, llfloatersearch.cpp, llpanelprofile.cpp,
//! llappearancemgr.cpp, llcommandhandler.cpp, originally LGPL 2.1).
//!
//! The texts that show these links (conversations, bubbles, profiles, group
//! charters and notices, place descriptions, search details) are what the
//! Legacy Viewer calls untrusted content (`trusted_content="false"` of its
//! chat history): a click there is NAV_TYPE_CLICKED from an untrusted
//! source, so the UNTRUSTED_BLOCK commands (wear, add or take off a folder,
//! the L$ balance) are refused with the BlockedSLURL notice, and every
//! other one runs. Agent links that only show a name (about, completename…)
//! and place links are read elsewhere (`ui/chat.rs`, `slurl.rs`).

use glam::Vec3;
use uuid::Uuid;

const PREFIX: &str = "secondlife:///app/";

/// A setting path is a stable, ASCII field identifier, not translated text.
fn valid_preference_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && !path.starts_with('.')
        && !path.ends_with('.')
        && !path.contains("..")
        && path.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
}

/// A read-only deep link to a Preferences row; invalid paths cannot be shared.
pub fn preference_link(path: &str) -> Option<String> {
    valid_preference_path(path).then(|| format!("{PREFIX}preferences/{path}"))
}

/// The longest Preferences search a link carries, in bytes of UTF-8: the
/// Legacy Viewer's search field (`search_prefs_edit`, `max_length_bytes`
/// 255 in floater_preferences.xml), so every search either viewer copies
/// opens here whole. A longer one is cut on a character, as that field
/// cuts what it is given.
const PREFERENCES_SEARCH_MAX: usize = 255;

/// The address of a Preferences search before its value is decoded, longer
/// ones are refused unread: every byte of the longest search escaped, and
/// room for the other keys of the Legacy Viewer's form.
const PREFERENCES_SEARCH_RAW_MAX: usize = 3 * PREFERENCES_SEARCH_MAX + 64;

/// The query a search link opens: no control nor invisible direction
/// character (the label of the link in chat shows it), cut to
/// [`PREFERENCES_SEARCH_MAX`] bytes, trimmed. Empty: nothing to search.
fn preferences_search_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len().min(PREFERENCES_SEARCH_MAX));
    for c in query.chars() {
        let c = match c {
            // a line break or a tab inside the field's single line
            c if c.is_control() => ' ',
            // zero width space and direction marks, embeddings, overrides
            // and isolates: they would reorder or hide the label's text (the
            // joiners U+200C and U+200D stay: words and emoji need them)
            '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' => continue,
            c => c,
        };
        if out.len() + c.len_utf8() > PREFERENCES_SEARCH_MAX {
            break;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

/// The link of a Preferences search in the Legacy Viewer's form
/// (LLFloaterPreference::onCopySearch, indra/newview/llfloaterpreference.cpp:
/// `openfloater/preferences?search=` and the text through LLURI::escape,
/// `slurl::encode` here), so that it opens in both viewers. None for an
/// empty search. One difference: a final `.` is written `%2E`, which
/// LLURI::unescape reads the same, since the chat's link finder leaves a
/// final full stop to the sentence (`ui/chat.rs`, `next_link`).
pub fn preferences_search_link(query: &str) -> Option<String> {
    let query = preferences_search_query(query);
    if query.is_empty() {
        return None;
    }
    let mut escaped = crate::slurl::encode(&query);
    if escaped.ends_with('.') {
        escaped.pop();
        escaped.push_str("%2E");
    }
    Some(format!("{PREFIX}openfloater/preferences?search={escaped}"))
}

/// `%XX` decoded strictly: None for a `%` without two hex digits after it
/// or bytes that are not UTF-8, where `unescape` keeps what it can.
fn unescape_strict(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            if !hex.bytes().all(|h| h.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `openfloater/preferences?search=…`: the search, or None when the link
/// has no usable one. The key is read as LLURI::queryMap reads it (exact
/// case, the last one wins); a `tab=` beside it is not read, since the
/// Legacy Viewer's tabs are not Aurora's pages (it would open that tab and
/// leave the search).
fn preferences_search(query: &str) -> Option<String> {
    if query.len() > PREFERENCES_SEARCH_RAW_MAX {
        return None;
    }
    let (_, raw) = query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .rfind(|(k, _)| *k == "search")?;
    Some(preferences_search_query(&unescape_strict(raw)?)).filter(|q| !q.is_empty())
}

/// What a verb of `secondlife:///app/agent/<id>/<verb>` does besides showing
/// the profile (LLAgentHandler::handle).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentVerb {
    Im,
    Pay,
    OfferTeleport,
    RequestFriend,
    RemoveFriend,
    /// Block the resident, if not blocked yet.
    Mute,
    /// Unblock the resident, if blocked.
    Unmute,
    /// `block/<object name>`: block an object by its id and name.
    Block(String),
    /// `unblock/<object name>`.
    Unblock(String),
    ReportAbuse,
}

/// What a link's block verb does, given whether its resident or object is
/// blocked now (LLAgentHandler::handle, indra/newview/llpanelprofile.cpp:
/// `mute` only an unblocked resident, `unmute` only a blocked one, the same
/// for an object's `block` and `unblock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStep {
    /// Not a block verb: it runs as it is.
    Other,
    /// Already in the state asked for: nothing happens.
    Nothing,
    /// Add the block at once, as the Legacy Viewer does.
    Block,
    /// Remove the block only once the user confirms. Stricter than the
    /// Legacy Viewer, which unblocks at once: the link comes from a text
    /// someone else wrote (a chat line, a web page, a message outside the
    /// viewer), and must not lift a block the user set against a harasser
    /// without them knowing.
    AskUnblock,
}

impl AgentVerb {
    pub fn block_step(&self, blocked: bool) -> BlockStep {
        match self {
            Self::Mute | Self::Block(_) if blocked => BlockStep::Nothing,
            Self::Mute | Self::Block(_) => BlockStep::Block,
            Self::Unmute | Self::Unblock(_) if blocked => BlockStep::AskUnblock,
            Self::Unmute | Self::Unblock(_) => BlockStep::Nothing,
            _ => BlockStep::Other,
        }
    }
}

/// The object of `secondlife:///app/objectim/<id>?name=…&owner=…&slurl=…`,
/// the inspector's data (LLObjectIMHandler's payload).
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteObject {
    pub id: Uuid,
    pub name: String,
    pub owner: Uuid,
    pub group_owned: bool,
    /// `rlv_shownames`: RLVa's `@shownames` hid the owner when the object
    /// spoke (process_chat_from_simulator and IM_FROM_TASK write it in the
    /// objectim link; LLInspectRemoteObject then shows `rlvanonym`).
    pub owner_hidden: bool,
    /// "Region/x/y/z", escaped as in the link; may be empty.
    pub slurl: String,
}

/// The commands the Legacy Viewer refuses from untrusted content
/// (UNTRUSTED_BLOCK).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocked {
    /// `wear_folder/?folder_id=…` (FSUrlEntryWear shows it as "Wear
    /// inventory folder").
    WearFolder,
    AddFolder,
    RemoveFolder,
    Balance,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppLink {
    Agent {
        id: Uuid,
        verb: AgentVerb,
    },
    /// `group/<id>/about` or `inspect` (both show the group's profile).
    Group(Uuid),
    /// `parcel/<id>/about`.
    Parcel(Uuid),
    ObjectIm(RemoteObject),
    /// `inventory/<id>/select`: the inventory, the item selected; `name=`
    /// is the label.
    Inventory {
        id: Uuid,
        name: Option<String>,
    },
    /// `inventory/show`.
    InventoryShow,
    /// `experience/<id>/profile`.
    Experience(Uuid),
    /// `classified/<id>/about`.
    Classified(Uuid),
    /// `event/<id>/details`.
    Event(u32),
    /// `worldmap/Region/x/y/z`: the map on the place.
    WorldMap {
        region: String,
        pos: Vec3,
    },
    /// `chat/<channel>/<text>`: say the text on a positive channel.
    Chat {
        channel: i32,
        text: String,
    },
    /// `search/<category>/<query>`.
    Search {
        category: String,
        query: String,
    },
    /// `preferences/<setting path>` only opens and highlights a setting.
    Preferences(String),
    /// `openfloater/preferences?search=<query>` (LLFloaterOpenHandler,
    /// LLFloaterPreference::onOpen): Preferences with the query in its
    /// search field, as if typed; no setting changes. The query is
    /// `preferences_search_query`'s, never empty.
    PreferencesSearch(String),
    AreaSearch(Box<crate::settings::area_search::Filters>),
    Blocked(Blocked),
    /// Any other command, or one whose arguments are wrong: the Legacy
    /// Viewer says UnsupportedCommandSLURL.
    Unsupported,
}

/// `%XX` decoded (LLURI::unescape: `+` stays a `+`).
pub fn unescape(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of `key` in a `a=1&b=2` query, unescaped (LLURI::queryMap).
pub(crate) fn query_value(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('=').or(Some((kv, ""))))
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| unescape(v))
}

fn id(s: &str) -> Option<Uuid> {
    Uuid::parse_str(s).ok()
}

/// The command of a `secondlife:///app/...` link (case-insensitive scheme,
/// like the `icase` patterns of LLUrlEntry*), or None for another link.
/// Agent links that only show a name and place links (region, teleport)
/// are read before this.
pub fn parse(url: &str) -> Option<AppLink> {
    let rest = url
        .get(..PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(PREFIX))
        .map(|_| &url[PREFIX.len()..])?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let tokens: Vec<&str> = path.split('/').filter(|t| !t.is_empty()).collect();
    let token = |i: usize| tokens.get(i).copied().unwrap_or("");
    let link = match token(0).to_ascii_lowercase().as_str() {
        "openfloater" if tokens.len() == 2 && token(1) == "area_search" => {
            crate::settings::area_search::Filters::from_query(query).map(|f| AppLink::AreaSearch(Box::new(f)))
        }
        // UNTRUSTED_THROTTLE like every `openfloater`: a click runs it, a
        // link from outside waits its turn (`external_url.rs`)
        "openfloater" if tokens.len() == 2 && token(1) == "preferences" => preferences_search(query).map(AppLink::PreferencesSearch),
        "agent" => agent(&tokens[1..]),
        "group" => match (id(token(1)), token(2)) {
            (Some(g), "about" | "inspect") => Some(AppLink::Group(g)),
            _ => None,
        },
        "parcel" => match (id(token(1)), token(2)) {
            (Some(p), "about") if !p.is_nil() => Some(AppLink::Parcel(p)),
            _ => None,
        },
        "objectim" => id(token(1)).map(|o| {
            AppLink::ObjectIm(RemoteObject {
                id: o,
                name: query_value(query, "name").unwrap_or_default(),
                owner: query_value(query, "owner").and_then(|v| id(&v)).unwrap_or_default(),
                // LLSD asBoolean of the string: "1" / "true"
                group_owned: query_value(query, "groupowned").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
                owner_hidden: query_value(query, "rlv_shownames").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
                slurl: query_value(query, "slurl").unwrap_or_default(),
            })
        }),
        "inventory" => match (token(1), id(token(1)), token(2)) {
            ("show", ..) => Some(AppLink::InventoryShow),
            (_, Some(i), "select") => Some(AppLink::Inventory {
                id: i,
                name: query_value(query, "name").filter(|n| !n.is_empty()),
            }),
            _ => None,
        },
        "experience" if tokens.len() == 3 && token(2) == "profile" => id(token(1)).map(AppLink::Experience),
        "classified" => match (id(token(1)), token(2)) {
            (Some(c), "about") => Some(AppLink::Classified(c)),
            _ => None,
        },
        "event" => match (token(1).parse::<u32>(), token(2)) {
            (Ok(e), "details") => Some(AppLink::Event(e)),
            _ => None,
        },
        "worldmap" if tokens.len() > 1 => {
            // LLWorldMapHandler: missing coordinates are 128, 128, 0
            let n = |i: usize, d: f32| token(i).parse::<f32>().ok().filter(|v| v.is_finite()).unwrap_or(d);
            Some(AppLink::WorldMap {
                region: unescape(token(1)),
                pos: Vec3::new(n(2, 128.0), n(3, 128.0), n(4, 0.0)),
            })
        }
        "chat" => {
            // VWR-19499: channels above 0 only, below CHAT_CHANNEL_DEBUG
            let channel = token(1).parse::<i32>().ok().filter(|c| *c > 0 && *c < i32::MAX);
            match (channel, tokens.get(2)) {
                (Some(channel), Some(_)) => Some(AppLink::Chat {
                    channel,
                    // the text is the second token only (LLSD path array)
                    text: unescape(token(2)),
                }),
                _ => None,
            }
        }
        "search" => Some(AppLink::Search {
            category: token(1).to_owned(),
            query: unescape(token(2)),
        }),
        "preferences" if tokens.len() == 2 => {
            let path = token(1);
            valid_preference_path(path).then(|| AppLink::Preferences(path.to_owned()))
        }
        "wear_folder" => Some(AppLink::Blocked(Blocked::WearFolder)),
        "add_folder" => Some(AppLink::Blocked(Blocked::AddFolder)),
        "remove_folder" => Some(AppLink::Blocked(Blocked::RemoveFolder)),
        "balance" => Some(AppLink::Blocked(Blocked::Balance)),
        _ => None,
    };
    Some(link.unwrap_or(AppLink::Unsupported))
}

/// What a `wear_folder`, `add_folder` or `remove_folder` command asks, read
/// as LLWearFolderHandler, LLAddFolderHandler and LLRemoveFolderHandler do
/// (llappearancemgr.cpp: wear_category). They are UNTRUSTED_BLOCK: only a
/// trusted page (the Avatar Welcome Pack, `ui/avatar_welcome.rs`) runs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCommand {
    pub action: FolderAction,
    /// `folder_name=`: a folder of the Library, found by its name.
    pub name: Option<String>,
    /// `folder_id=`: any folder, used when no Library folder has the name.
    pub id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderAction {
    /// `wear_folder`: the folder replaces the outfit.
    Wear,
    /// `add_folder`: the folder is added to the outfit.
    Add,
    /// `remove_folder`: what the folder holds is taken off (`folder_id` only).
    Remove,
}

/// The folder command of `url`, None for any other link. Both
/// `secondlife:///app/` and `secondlife://app/` (the host form, which
/// LLSLURL reads as an app command too, and which the welcome pack's page
/// uses) are read.
pub fn folder_command(url: &str) -> Option<FolderCommand> {
    let rest = [PREFIX, "secondlife://app/"]
        .into_iter()
        .find(|prefix| url.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix)))
        .map(|prefix| &url[prefix.len()..])?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let action = match path.trim_matches('/').to_ascii_lowercase().as_str() {
        "wear_folder" => FolderAction::Wear,
        "add_folder" => FolderAction::Add,
        "remove_folder" => FolderAction::Remove,
        _ => return None,
    };
    let name = query_value(query, "folder_name").filter(|n| !n.is_empty() && action != FolderAction::Remove);
    let id = query_value(query, "folder_id").and_then(|v| id(&v)).filter(|id| !id.is_nil());
    Some(FolderCommand { action, name, id })
}

/// What the Second Life sign-up page asks of the window that shows it:
/// `secondlife:///app/floaterjoin/<action>/…`, read as LLFloaterJoinHandler
/// does (indra/newview/llfloaterjoin.cpp, originally LGPL 2.1). Only the
/// sign-up window acts on them (`ui/signup.rs`).
#[derive(Clone, PartialEq, Eq)]
pub enum JoinCommand {
    /// `close`: the page is done (its "Cancel signup").
    Close,
    /// `tos`: the Terms of Service, in a window of their own.
    TermsOfService,
    /// `tc`: the Terms and Conditions, in a window of their own.
    TermsAndConditions,
    /// `privacy`: the Privacy Policy, in a window of their own.
    Privacy,
    /// `join/<username>/<password>`: the account was created; its name and
    /// password go to the login form.
    Join { username: String, password: String },
}

/// Never prints the name nor the password (LLFloaterJoinHandler::isSensitive
/// keeps the command out of the log).
impl std::fmt::Debug for JoinCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Close => f.write_str("Close"),
            Self::TermsOfService => f.write_str("TermsOfService"),
            Self::TermsAndConditions => f.write_str("TermsAndConditions"),
            Self::Privacy => f.write_str("Privacy"),
            Self::Join { .. } => f.write_str("Join { .. }"),
        }
    }
}

/// The sign-up command of `url`, None for any other link. Both app forms are
/// read, as for the folder commands. The name and password of `join` are
/// unescaped after the path is split (the page escapes them with
/// encodeURIComponent, so a `/` in the password is `%2F`).
pub fn join_command(url: &str) -> Option<JoinCommand> {
    let rest = [PREFIX, "secondlife://app/"]
        .into_iter()
        .find(|prefix| url.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix)))
        .map(|prefix| &url[prefix.len()..])?;
    let path = rest.split_once('?').map_or(rest, |(path, _)| path);
    let mut tokens = path.split('/').filter(|t| !t.is_empty());
    if !tokens.next()?.eq_ignore_ascii_case("floaterjoin") {
        return None;
    }
    Some(match tokens.next()? {
        "close" => JoinCommand::Close,
        "tos" => JoinCommand::TermsOfService,
        "tc" => JoinCommand::TermsAndConditions,
        "privacy" => JoinCommand::Privacy,
        "join" => {
            let (username, password) = (tokens.next()?, tokens.next()?);
            JoinCommand::Join {
                username: unescape(username),
                password: unescape(password),
            }
        }
        _ => return None,
    })
}

/// `agent/<id>/<verb>[/<name>]` for the verbs that act (LLAgentHandler).
fn agent(tokens: &[&str]) -> Option<AppLink> {
    let id = id(tokens.first()?)?;
    let name = || tokens.get(2).map(|n| unescape(n));
    let verb = match *tokens.get(1)? {
        "im" => AgentVerb::Im,
        "pay" => AgentVerb::Pay,
        "offerteleport" => AgentVerb::OfferTeleport,
        "requestfriend" => AgentVerb::RequestFriend,
        "removefriend" => AgentVerb::RemoveFriend,
        "mute" => AgentVerb::Mute,
        "unmute" => AgentVerb::Unmute,
        // without a name the Legacy Viewer does nothing (and says nothing)
        "block" => AgentVerb::Block(name()?),
        "unblock" => AgentVerb::Unblock(name()?),
        "reportAbuse" => AgentVerb::ReportAbuse,
        _ => return None,
    };
    Some(AppLink::Agent { id, verb })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0e346d8b-4433-4d66-a6b0-fd37083abc4c";

    fn uid() -> Uuid {
        Uuid::parse_str(ID).expect("test id")
    }

    #[test]
    fn generated_preference_links_round_trip() {
        for path in ["chat_timestamps_nearby", "colors.chat_beyond_range"] {
            let url = preference_link(path).expect("valid setting path");
            assert_eq!(parse(&url), Some(AppLink::Preferences(path.to_owned())));
        }
        for path in ["", ".secret", "bad..path", "chat/other", "contains space"] {
            assert!(preference_link(path).is_none(), "{path}");
        }
    }

    #[test]
    fn preferences_links_accept_only_setting_paths() {
        assert_eq!(
            parse("secondlife:///app/preferences/chat_timestamps_nearby"),
            Some(AppLink::Preferences("chat_timestamps_nearby".to_owned()))
        );
        assert_eq!(
            parse("secondlife:///app/preferences/colors.chat_beyond_range"),
            Some(AppLink::Preferences("colors.chat_beyond_range".to_owned()))
        );
        for url in [
            "secondlife:///app/preferences/",
            "secondlife:///app/preferences/../../secret",
            "secondlife:///app/preferences/%2E%2E",
            "secondlife:///app/preferences/chat/extra",
            "secondlife:///app/preferences/chat%20name",
        ] {
            assert_eq!(parse(url), Some(AppLink::Unsupported), "{url}");
        }
    }

    fn search(query: &str) -> Option<AppLink> {
        Some(AppLink::PreferencesSearch(query.to_owned()))
    }

    /// LLURI::escape (indra/llcommon/lluri.cpp): the unreserved characters
    /// as they are, every other byte of the UTF-8 text as `%XX` in capitals;
    /// a space is `%20`, never `+`.
    #[test]
    fn a_copied_search_is_escaped_like_the_legacy_viewer() {
        let link = |q: &str| preferences_search_link(q).expect("a search");
        let head = "secondlife:///app/openfloater/preferences?search=";
        // the example of LLFloaterPreference::onOpen
        assert_eq!(link("# of lines"), format!("{head}%23%20of%20lines"));
        for (query, escaped) in [
            ("draw distance", "draw%20distance"),
            ("a&b", "a%26b"),
            ("why?", "why%3F"),
            ("100%", "100%25"),
            ("1+1=2", "1%2B1%3D2"),
            ("café", "caf%C3%A9"),
            ("日本", "%E6%97%A5%E6%9C%AC"),
            ("say \"hi\" 'there'", "say%20%22hi%22%20%27there%27"),
            ("a/b\\c", "a%2Fb%5Cc"),
            ("Az09-._~", "Az09-._~"),
            // a final full stop stays in the link the chat finds
            ("v1.", "v1%2E"),
        ] {
            assert_eq!(link(query), format!("{head}{escaped}"), "{query}");
        }
    }

    #[test]
    fn a_copied_search_opens_the_same_search() {
        for query in [
            "# of lines",
            "draw distance",
            "a&b=c?d#e",
            "100% sure",
            "1+1",
            "café crème",
            "日本語の設定",
            "\"quoted\" and 'single'",
            "v1.",
            "~tilde",
            // the joiners of a word or an emoji are kept
            "می\u{200C}خواهم",
            "👩\u{200D}💻",
        ] {
            let url = preferences_search_link(query).expect("a search");
            assert_eq!(parse(&url), search(query), "{url}");
        }
        // what the field holds around it is cleaned the same way both sides
        let url = preferences_search_link("  two\twords \u{202E}\n").expect("a search");
        assert_eq!(parse(&url), search("two words"));
        let url = preferences_search_link("\u{200F}a\u{200B}b\u{2066}c\u{2069}\u{FEFF}").expect("a search");
        assert_eq!(parse(&url), search("abc"));
        // nothing to share
        for query in ["", "   ", "\u{200B}", "\n\t"] {
            assert_eq!(preferences_search_link(query), None, "{query:?}");
        }
    }

    #[test]
    fn legacy_search_links_open_their_search() {
        let head = "secondlife:///app/openfloater/preferences";
        assert_eq!(parse(&format!("{head}?search=%23%20of%20lines")), search("# of lines"));
        // the Legacy Viewer's own links (llstatusbar.cpp), lower case hex,
        // `secondlife://app/` handed over from outside (`external_url.rs`)
        assert_eq!(parse(&format!("{head}?search=StatusBarTimeFormat")), search("StatusBarTimeFormat"));
        assert_eq!(parse(&format!("{head}?search=caf%c3%a9")), search("café"));
        assert_eq!(parse(&format!("{head}/?search=x")), search("x"));
        // LLURI::unescape keeps `+`; LLURI::queryMap keeps the last key
        assert_eq!(parse(&format!("{head}?search=a+b")), search("a+b"));
        assert_eq!(parse(&format!("{head}?search=first&search=last")), search("last"));
        assert_eq!(parse(&format!("{head}?tab=chat&search=volume")), search("volume"));
        // control and direction characters never reach the field nor the label
        assert_eq!(parse(&format!("{head}?search=a%0Ab%E2%80%AEc%00")), search("a bc"));
    }

    #[test]
    fn a_search_link_without_a_usable_search_is_unsupported() {
        let head = "secondlife:///app/openfloater/preferences";
        for url in [
            head.to_owned(),
            format!("{head}?"),
            format!("{head}?search"),
            format!("{head}?search="),
            format!("{head}?search=%20%20"),
            format!("{head}?search=%E2%80%8B"),
            format!("{head}?tab=chat"),
            format!("{head}?Search=volume"),
            // bad escapes: no digits, one digit, a lone byte of UTF-8
            format!("{head}?search=%ZZ"),
            format!("{head}?search=abc%2"),
            format!("{head}?search=abc%"),
            format!("{head}?search=caf%C3"),
            format!("{head}?search=%FF%FE"),
            // longer than any search either viewer copies
            format!("{head}?search={}", "%41".repeat(PREFERENCES_SEARCH_MAX + 30)),
            // another floater, or more than its name
            "secondlife:///app/openfloater/preferences/chat?search=volume".to_owned(),
            "secondlife:///app/openfloater/Preferences?search=volume".to_owned(),
            "secondlife:///app/openfloater/camera?search=volume".to_owned(),
        ] {
            assert_eq!(parse(&url), Some(AppLink::Unsupported), "{url}");
        }
    }

    /// A search a little longer than the field is cut on a character (the
    /// address much longer than any copy is refused unread, above).
    #[test]
    fn a_long_search_is_cut_on_a_character_like_the_legacy_field() {
        let head = "secondlife:///app/openfloater/preferences?search=";
        // 260 bytes: two-byte letters, the 128th would end past 255
        let long = "é".repeat(130);
        let Some(AppLink::PreferencesSearch(query)) = parse(&format!("{head}{}", crate::slurl::encode(&long))) else {
            panic!("a long search still opens");
        };
        assert_eq!(query, "é".repeat(127));
        // the copy is cut the same way: what is copied is what opens, even
        // from a much longer field
        for typed in [long, "é".repeat(400)] {
            let url = preferences_search_link(&typed).expect("a search");
            assert_eq!(parse(&url), search(&query));
        }
        // a search of the full length opens whole
        let full = "a".repeat(PREFERENCES_SEARCH_MAX);
        let url = preferences_search_link(&full).expect("a search");
        assert_eq!(parse(&url), search(&full));
    }

    /// The links to one setting (#4414) are read as before.
    #[test]
    fn setting_links_and_search_links_stay_apart() {
        assert_eq!(
            parse("secondlife:///app/preferences/chat_timestamps_nearby"),
            Some(AppLink::Preferences("chat_timestamps_nearby".to_owned()))
        );
        assert_eq!(parse("secondlife:///app/preferences/?search=volume"), Some(AppLink::Unsupported));
        assert_eq!(
            parse("secondlife:///app/openfloater/area_search?nm=box").map(|l| matches!(l, AppLink::AreaSearch(_))),
            Some(true)
        );
    }

    #[test]
    fn block_verbs_ask_before_unblocking() {
        let object = || "Sign".to_owned();
        for blocked in [false, true] {
            for verb in [
                AgentVerb::Im,
                AgentVerb::Pay,
                AgentVerb::OfferTeleport,
                AgentVerb::RequestFriend,
                AgentVerb::RemoveFriend,
                AgentVerb::ReportAbuse,
            ] {
                assert_eq!(verb.block_step(blocked), BlockStep::Other, "{verb:?}");
            }
        }
        assert_eq!(AgentVerb::Mute.block_step(false), BlockStep::Block);
        assert_eq!(AgentVerb::Mute.block_step(true), BlockStep::Nothing);
        assert_eq!(AgentVerb::Block(object()).block_step(false), BlockStep::Block);
        assert_eq!(AgentVerb::Block(object()).block_step(true), BlockStep::Nothing);
        assert_eq!(AgentVerb::Unmute.block_step(true), BlockStep::AskUnblock);
        assert_eq!(AgentVerb::Unmute.block_step(false), BlockStep::Nothing);
        assert_eq!(AgentVerb::Unblock(object()).block_step(true), BlockStep::AskUnblock);
        assert_eq!(AgentVerb::Unblock(object()).block_step(false), BlockStep::Nothing);
    }

    #[test]
    fn ids_and_verbs() {
        assert_eq!(parse(&format!("secondlife:///app/group/{ID}/about")), Some(AppLink::Group(uid())));
        assert_eq!(parse(&format!("SecondLife:///app/group/{ID}/inspect")), Some(AppLink::Group(uid())));
        assert_eq!(parse(&format!("secondlife:///app/parcel/{ID}/about")), Some(AppLink::Parcel(uid())));
        assert_eq!(
            parse(&format!("secondlife:///app/experience/{ID}/profile")),
            Some(AppLink::Experience(uid()))
        );
        assert_eq!(
            parse(&format!("secondlife:///app/classified/{ID}/about")),
            Some(AppLink::Classified(uid()))
        );
        assert_eq!(parse("secondlife:///app/event/1234/details"), Some(AppLink::Event(1234)));
        assert_eq!(parse("secondlife:///app/inventory/show"), Some(AppLink::InventoryShow));
        assert_eq!(
            parse(&format!("secondlife:///app/inventory/{ID}/select?name=My%20hat")),
            Some(AppLink::Inventory {
                id: uid(),
                name: Some("My hat".into())
            })
        );
        assert_eq!(
            parse(&format!("secondlife:///app/agent/{ID}/offerteleport")),
            Some(AppLink::Agent {
                id: uid(),
                verb: AgentVerb::OfferTeleport
            })
        );
        assert_eq!(
            parse(&format!("secondlife:///app/agent/{ID}/block/Old%20Sign")),
            Some(AppLink::Agent {
                id: uid(),
                verb: AgentVerb::Block("Old Sign".into())
            })
        );
        assert_eq!(parse("https://example.org/app/group"), None);
    }

    #[test]
    fn wrong_arguments_are_unsupported() {
        for url in [
            "secondlife:///app/group/nope/about",
            "secondlife:///app/group/create",
            "secondlife:///app/parcel/00000000-0000-0000-0000-000000000000/about",
            "secondlife:///app/event/x/details",
            "secondlife:///app/experience/0e346d8b-4433-4d66-a6b0-fd37083abc4c",
            "secondlife:///app/agent/0e346d8b-4433-4d66-a6b0-fd37083abc4c/block",
            "secondlife:///app/chat/0/hello",
            "secondlife:///app/chat/-5/hello",
            "secondlife:///app/chat/5",
            "secondlife:///app/frobnicate/1",
        ] {
            assert_eq!(parse(url), Some(AppLink::Unsupported), "{url}");
        }
    }

    #[test]
    fn object_im_query() {
        let url = format!(
            "secondlife:///app/objectim/{ID}?name=Old%20Sign&owner=3de548e1-57be-cfea-2b78-83ae3ad95998&slurl=Danger%21/200/200/30/&groupowned=1"
        );
        let Some(AppLink::ObjectIm(o)) = parse(&url) else {
            panic!("objectim");
        };
        assert_eq!(o.id, uid());
        assert_eq!(o.name, "Old Sign");
        assert_eq!(o.owner, Uuid::parse_str("3de548e1-57be-cfea-2b78-83ae3ad95998").expect("owner"));
        assert!(o.group_owned);
        assert_eq!(o.slurl, "Danger!/200/200/30/");
    }

    #[test]
    fn map_chat_search_and_blocked() {
        assert_eq!(
            parse("secondlife:///app/worldmap/Pine%20Grove/40/80"),
            Some(AppLink::WorldMap {
                region: "Pine Grove".into(),
                pos: Vec3::new(40.0, 80.0, 0.0)
            })
        );
        assert_eq!(
            parse("secondlife:///app/chat/42/Hello%20there"),
            Some(AppLink::Chat {
                channel: 42,
                text: "Hello there".into()
            })
        );
        assert_eq!(
            parse("secondlife:///app/search/places/beach%20club"),
            Some(AppLink::Search {
                category: "places".into(),
                query: "beach club".into()
            })
        );
        assert_eq!(
            parse("secondlife:///app/wear_folder/?folder_id=bedd047e-a3d7-23e6-57bc-1ef367d848e7"),
            Some(AppLink::Blocked(Blocked::WearFolder))
        );
        assert_eq!(parse("secondlife:///app/balance/request"), Some(AppLink::Blocked(Blocked::Balance)));
        // `+` stays a plus, as LLURI::unescape leaves it
        assert_eq!(unescape("a+b%2Bc%C3%A9"), "a+b+cé");
    }

    #[test]
    fn folder_commands_read_both_app_forms_and_their_folder() {
        let wear = folder_command("secondlife://app/wear_folder/?folder_name=Female%20body%20-%20A").expect("wear");
        assert_eq!(wear.action, FolderAction::Wear);
        assert_eq!(wear.name.as_deref(), Some("Female body - A"));
        assert_eq!(wear.id, None);
        let add = folder_command("SECONDLIFE:///app/add_folder?folder_name=Male look - Club").expect("add");
        assert_eq!(add.action, FolderAction::Add);
        assert_eq!(add.name.as_deref(), Some("Male look - Club"));
        let id = Uuid::from_u128(7);
        let remove = folder_command(&format!("secondlife:///app/remove_folder/?folder_id={id}&folder_name=x")).expect("remove");
        assert_eq!(remove.action, FolderAction::Remove);
        assert_eq!((remove.name, remove.id), (None, Some(id)));
        // other commands and links are not folder commands
        assert_eq!(folder_command("secondlife:///app/teleport/Region/1/2/3"), None);
        assert_eq!(folder_command("https://example.org/wear_folder/?folder_name=x"), None);
        assert_eq!(folder_command("secondlife://Region/1/2/3"), None);
    }

    #[test]
    fn join_commands_are_read_like_the_sign_up_handler() {
        assert_eq!(join_command("secondlife:///app/floaterjoin/close"), Some(JoinCommand::Close));
        assert_eq!(join_command("secondlife://app/floaterjoin/tos"), Some(JoinCommand::TermsOfService));
        assert_eq!(
            join_command("SecondLife:///app/FloaterJoin/tc/"),
            Some(JoinCommand::TermsAndConditions)
        );
        assert_eq!(
            join_command("secondlife:///app/floaterjoin/privacy?x=1"),
            Some(JoinCommand::Privacy)
        );
        // escaped after the split: a `/` and spaces in the password stay its own
        assert_eq!(
            join_command("secondlife:///app/floaterjoin/join/synthetic.resident/a%2Fb%20c%C3%A9"),
            Some(JoinCommand::Join {
                username: "synthetic.resident".into(),
                password: "a/b cé".into(),
            })
        );
        // the action is case-sensitive, and join needs both values
        for url in [
            "secondlife:///app/floaterjoin",
            "secondlife:///app/floaterjoin/Close",
            "secondlife:///app/floaterjoin/join/synthetic.resident",
            "secondlife:///app/floaterjoin/delete",
            "secondlife:///app/wear_folder/?folder_name=x",
            "https://join.secondlife.com/floaterjoin/close",
        ] {
            assert_eq!(join_command(url), None, "{url}");
        }
    }

    #[test]
    fn a_join_command_never_prints_its_values() {
        let join = JoinCommand::Join {
            username: "synthetic.resident".into(),
            password: "synthetic-secret".into(),
        };
        let printed = format!("{join:?}");
        assert!(!printed.contains("synthetic"), "{printed}");
    }
}
