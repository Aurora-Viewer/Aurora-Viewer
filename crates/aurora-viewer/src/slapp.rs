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

/// The object of `secondlife:///app/objectim/<id>?name=…&owner=…&slurl=…`,
/// the inspector's data (LLObjectIMHandler's payload).
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteObject {
    pub id: Uuid,
    pub name: String,
    pub owner: Uuid,
    pub group_owned: bool,
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
fn query_value(query: &str, key: &str) -> Option<String> {
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
        "wear_folder" => Some(AppLink::Blocked(Blocked::WearFolder)),
        "add_folder" => Some(AppLink::Blocked(Blocked::AddFolder)),
        "remove_folder" => Some(AppLink::Blocked(Blocked::RemoveFolder)),
        "balance" => Some(AppLink::Blocked(Blocked::Balance)),
        _ => None,
    };
    Some(link.unwrap_or(AppLink::Unsupported))
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
}
