//! AURORA_DEMO_CHATLOG: the conversation transcripts without a grid. The
//! demo is its own "grid" (`chat_log::grid_name`), so its files live in the
//! demo account's own folder, under the demo settings; this scenario writes
//! there the transcripts it shows, the same at every run.
//!
//! - `im`: a conversation with history from two earlier days, greyed, then
//!   a new message;
//! - `find`: the same, then Ctrl+F (pointer over the window) and a word
//!   typed at frame 300: the find field and its matches (`find:none` a word
//!   that is not there, `:legacy` the Legacy style, `:narrow` the rail);
//! - `history`: the History window on a transcript of 20 000 messages
//!   (`history:find`: Ctrl+F and a word typed at frame 300, searched in the
//!   whole file);
//! - `offline`: the instant messages kept while we were away, as the
//!   request made at login returns them;
//! - `restore`: saved private and group conversations, reopened without
//!   an incoming message, including the old untyped file format; `restore:slow`
//!   holds the mute reply until frame 1100 to check immediate display;
//! - `import`: the import window of Preferences › Chat over a synthetic
//!   Legacy Viewer folder (`import:run` imports and shows the merged
//!   History, `import:again` imports twice, `import:login` as the import at
//!   login does, `import:same` with the folder Aurora writes to, `import:none`
//!   with no Legacy Viewer found);
//! - `long:<n>`: a conversation of `n` lines, nothing written (to measure
//!   the cost of the conversation view); `long:<n>:scroll` scrolls it up
//!   with the wheel from frame 320 to 380, through lines never laid out.

use crate::world::chat_log::{format, names, store};
use aurora_net::{InstantMessage, NetEvent};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// The resident of the scenario's conversation ("Nova" for the demo's name
/// service).
pub const PEER: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0020);

/// Messages of the History window's transcript.
const LONG_MESSAGES: u64 = 19_949;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Im,
    Find,
    History {
        find: bool,
    },
    Offline,
    Restore,
    Import(ImportStage),
    /// The Legacy layout: a synthetic Legacy Viewer folder chosen for the
    /// session only; `on`, the option turned on too.
    Shared {
        on: bool,
    },
    Long {
        lines: usize,
        scroll: bool,
    },
}

/// What `import` goes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStage {
    Window,
    Run,
    Again,
    Login,
    Same,
    None,
}

pub fn mode() -> Option<Mode> {
    let v = std::env::var("AURORA_DEMO_CHATLOG").ok()?;
    let (mode, arg) = v.split_once(':').unwrap_or((v.as_str(), ""));
    Some(match mode {
        "find" => Mode::Find,
        "history" => Mode::History { find: arg == "find" },
        "offline" => Mode::Offline,
        "shared" => Mode::Shared { on: arg == "on" },
        "restore" => Mode::Restore,
        "import" => Mode::Import(match arg {
            "run" => ImportStage::Run,
            "again" => ImportStage::Again,
            "login" => ImportStage::Login,
            "same" => ImportStage::Same,
            "none" => ImportStage::None,
            _ => ImportStage::Window,
        }),
        "long" => {
            let (lines, scroll) = arg.split_once(':').unwrap_or((arg, ""));
            Mode::Long {
                lines: lines.parse().unwrap_or(2000),
                scroll: scroll == "scroll",
            }
        }
        _ => Mode::Im,
    })
}

static DELAYED_MUTES: std::sync::Mutex<Option<NetEvent>> = std::sync::Mutex::new(None);

fn slow_restore() -> bool {
    std::env::var("AURORA_DEMO_CHATLOG").is_ok_and(|value| value == "restore:slow")
}

pub fn mute_list_reply(text: String) -> Vec<NetEvent> {
    let event = NetEvent::MuteList(aurora_net::MuteListSource::File(text));
    if slow_restore() {
        if let Ok(mut held) = DELAYED_MUTES.lock() {
            *held = Some(event);
        }
        Vec::new()
    } else {
        vec![event]
    }
}

pub fn delayed_mute_list(frame: u64) -> Option<NetEvent> {
    if frame < 1100 || !slow_restore() {
        return None;
    }
    DELAYED_MUTES.lock().ok()?.take()
}

/// Exercise the actual account-file reader and login restoration path.
/// The second group receives no invitation/message in the ordinary demo.
pub fn seed_restore(world: &mut crate::world::World) {
    use crate::world::open_ims::{self, Conversation};
    let path = crate::settings::account_dir(&world.agent_id).join(open_ims::FILE);
    open_ims::save(
        &path,
        &[
            Conversation::Resident(PEER),
            Conversation::Group(super::DEMO_GROUP2),
            Conversation::Legacy(super::DEMO_GROUP3),
        ],
    );
    world.reopen_ims = open_ims::load(&path);
}

/// Legacy name of the peer, as an instant message carries it.
pub fn peer_name() -> String {
    format!("Nova {}", aurora_i18n::tr!("demo-example"))
}

/// The word the find bar and the History search look for (`find:none`: a
/// word the conversation does not hold).
pub fn needle() -> &'static str {
    if find_flag("none") { "zqxw" } else { "aurora" }
}

/// `find:<flag>…` (`none`, `legacy`, `narrow`): the find scenario with no
/// match, in the Legacy style, or in a window narrow enough for the rail.
pub fn find_flag(flag: &str) -> bool {
    std::env::var("AURORA_DEMO_CHATLOG").is_ok_and(|v| {
        let mut parts = v.split(':');
        parts.next() == Some("find") && parts.any(|f| f == flag)
    })
}

fn sentences() -> [&'static str; 6] {
    [
        aurora_i18n::tr!("demo-chatlog-line-1"),
        aurora_i18n::tr!("demo-chatlog-line-2"),
        aurora_i18n::tr!("demo-chatlog-line-3"),
        aurora_i18n::tr!("demo-chatlog-line-4"),
        aurora_i18n::tr!("demo-chatlog-line-5"),
        aurora_i18n::tr!("demo-chatlog-line-6"),
    ]
}

/// The synthetic Legacy Viewer of `import`, beside the demo's transcripts.
pub fn legacy_root(dir: &Path) -> PathBuf {
    dir.with_file_name("demo-legacy-viewer")
}

/// The synthetic Legacy Viewer folder of `shared`, beside the demo's
/// transcripts: `<root>/<account folder>/<peer>.txt` holds what the Legacy
/// Viewer wrote, rewritten at each launch, and a file of its own that Aurora
/// must leave as it is.
pub fn seed_shared(root: &Path, folder: &str, own_name: &str) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(root);
    let dir = root.join(folder);
    std::fs::create_dir_all(&dir)?;
    let peer = peer_name();
    let now = SystemTime::now();
    let day = Duration::from_secs(86_400);
    let s = sentences();
    let mut text = String::new();
    for (i, sentence) in s.iter().take(4).enumerate() {
        let from = if i % 2 == 0 { peer.as_str() } else { own_name };
        let stamp = format::stamp(now - day * 3 + Duration::from_secs(60 * i as u64), true);
        text.push_str(&format::format_line(Some(&stamp), from, sentence).replace('\n', store::EOL));
        text.push_str(store::EOL);
    }
    let path = names::file_path(&dir, &names::resident(&peer), None);
    std::fs::write(path.ok_or_else(|| std::io::Error::other("no file name"))?, text)?;
    std::fs::write(dir.join("conversation.log"), "kept as written\n")
}

/// What a Legacy Viewer left for the demo account: the conversation Aurora
/// has too, earlier and with a message both have, a group, the nearby chat
/// by day, and a file of its own that is not a transcript.
pub fn seed_legacy(root: &Path, own_name: &str, stage: ImportStage) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(root);
    if stage == ImportStage::None {
        return Ok(());
    }
    let dir = root.join(own_name.to_lowercase().replace(' ', "_"));
    std::fs::create_dir_all(&dir)?;
    let peer = peer_name();
    let now = SystemTime::now();
    let day = Duration::from_secs(86_400);
    let s = sentences();
    let line =
        |ago: Duration, from: &str, text: &str| format!("{}\n", format::format_line(Some(&format::stamp(now - ago, true)), from, text));
    let mut text = String::new();
    for (i, sentence) in s.iter().enumerate() {
        let from = if i % 2 == 0 { peer.as_str() } else { own_name };
        text.push_str(&line(day * 30 - Duration::from_secs(600 * i as u64), from, sentence));
    }
    // the message Aurora's file has too
    text.push_str(&line(day + Duration::from_secs(300), &peer, s[4]));
    let path = names::file_path(&dir, &names::resident(&peer), None);
    std::fs::write(path.ok_or_else(|| std::io::Error::other("no file name"))?, text)?;
    std::fs::write(
        dir.join("Demo Club (group).txt"),
        // the last two are the group's history written again at a login
        line(day * 3, &peer, s[0]) + &line(day * 2, own_name, s[1]) + &line(day, &peer, s[0]) + &line(day, own_name, s[1]),
    )?;
    std::fs::write(dir.join("chat-2026-10-12.txt"), line(day * 9, &peer, s[2]))?;
    // told in the result line, not imported
    std::fs::write(dir.join("Old Friend.txt"), format!("{}: {}\n", peer, s[3]))?;
    std::fs::write(dir.join("search_history.txt"), "<llsd><array /></llsd>\n")
}

/// Write the transcripts of the scenario in the demo account's folder
/// (each run starts from the same files).
pub fn seed(dir: &Path, mode: Mode, own_name: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    if let Mode::Import(_) = mode {
        // what the last run imported
        for entry in std::fs::read_dir(dir)?.flatten() {
            std::fs::remove_file(entry.path())?;
        }
        let _ = std::fs::remove_file(dir.with_extension("legacy-import.json"));
    }
    let peer = peer_name();
    let Some(path) = names::file_path(dir, &names::resident(&peer), None) else {
        return Ok(());
    };
    let now = SystemTime::now();
    let s = sentences();
    let mut text = String::new();
    let mut add = |ago: Duration, from: &str, line: &str| {
        let stamp = format::stamp(now - ago, true);
        text.push_str(&format::format_line(Some(&stamp), from, line).replace('\n', store::EOL));
        text.push_str(store::EOL);
    };
    let day = Duration::from_secs(86_400);
    if let Mode::History { .. } = mode {
        // months of conversation: a message every ten minutes
        for i in 0..LONG_MESSAGES {
            let ago = Duration::from_secs((LONG_MESSAGES - i) * 600);
            let from = if i % 3 == 1 { own_name } else { &peer };
            let body = s[(i % 6) as usize];
            add(ago, from, &format!("{body} ({})", i + 1));
        }
    } else {
        add(day * 2 + Duration::from_secs(900), &peer, s[0]);
        add(day * 2 + Duration::from_secs(840), own_name, s[1]);
        add(day * 2 + Duration::from_secs(800), &peer, &format!("{}\n{}", s[2], s[3]));
        add(day + Duration::from_secs(400), "", aurora_i18n::tr!("demo-chatlog-system"));
        add(day + Duration::from_secs(300), &peer, s[4]);
        add(day + Duration::from_secs(240), own_name, s[5]);
    }
    std::fs::write(path, text)
}

fn im(agent: Uuid, text: &str, offline: bool, timestamp: u32) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: PEER,
        from_name: peer_name(),
        to_agent_id: agent,
        dialog: 0,
        session_id: PEER,
        message: text.into(),
        offline,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp,
    })
}

/// The message that opens the conversation.
pub fn new_message(agent: Uuid) -> NetEvent {
    im(agent, aurora_i18n::tr!("demo-chatlog-new"), false, 0)
}

/// A message of the peer.
pub fn message(agent: Uuid, text: &str) -> NetEvent {
    im(agent, text, false, 0)
}

/// What the offline message request answers: messages kept by the grid
/// while we were away, with the time they were sent.
pub fn offline_messages(agent: Uuid) -> Vec<NetEvent> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    vec![
        im(
            agent,
            aurora_i18n::tr!("demo-chatlog-offline-1"),
            true,
            now.saturating_sub(9 * 3600),
        ),
        im(
            agent,
            aurora_i18n::tr!("demo-chatlog-offline-2"),
            true,
            now.saturating_sub(9 * 3600 - 40),
        ),
        im(
            agent,
            aurora_i18n::tr!("demo-chatlog-offline-3"),
            true,
            now.saturating_sub(2 * 3600),
        ),
    ]
}

/// What the scenario types and scrolls: Ctrl+F then the searched word for
/// `find` and `history:find` (the shortcut goes to the window in front), the
/// wheel over the lines of the conversation window for `long:<n>:scroll`.
pub fn input_events(frame: u64) -> Vec<egui::Event> {
    let (find, scroll) = match mode() {
        Some(Mode::Find | Mode::History { find: true }) => (true, false),
        Some(Mode::Long { scroll: true, .. }) => (false, true),
        _ => return Vec::new(),
    };
    match frame {
        // the pointer over the lines of the window, then the shortcut
        296 if find => vec![egui::Event::PointerMoved(egui::pos2(300.0, 300.0))],
        300 if find => vec![egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
        306 if find => vec![egui::Event::Text(needle().to_owned())],
        320..=380 if scroll => vec![
            egui::Event::PointerMoved(egui::pos2(420.0, 320.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 150.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        _ => Vec::new(),
    }
}

/// Lines of the measured conversation: mixed lengths, a link and an emoji
/// now and then, three speakers.
pub fn long_line(i: usize) -> (bool, String) {
    let s = sentences();
    let mut text = s[i % 6].to_owned();
    if i.is_multiple_of(5) {
        text.push(' ');
        text.push_str(s[(i + 1) % 6]);
        text.push(' ');
        text.push_str(s[(i + 2) % 6]);
    }
    if i.is_multiple_of(11) {
        text.push_str(" https://example.invalid/aurora ✨");
    }
    (i % 4 == 1, format!("{text} ({})", i + 1))
}
