//! File names of the conversation transcripts, by Firestorm's rules: port of
//! LLLogChat::cleanFileName / makeLogFileName / oldLogFileName
//! (indra/newview/lllogchat.cpp) and LLIMModel::LLIMSession::buildHistoryFileName
//! (indra/newview/llimview.cpp), originally LGPL 2.1.
//!
//! - nearby chat: `chat.txt`;
//! - a resident: the legacy name without " Resident" (UseLegacyIMLogNames,
//!   on by default in Firestorm): `Jane Doe.txt`, `janedoe.txt`;
//! - a group: `<group name> (group).txt`;
//! - a conference we were invited to: `<name> <date> <time> <4 first hex
//!   digits of the session>` (Firestorm names one it started `<name>
//!   hash<md5 of the sorted participant ids>`; Aurora does not start
//!   conferences yet).
//!
//! On top of Firestorm's forbidden characters, what Windows refuses or
//! silently changes is handled too (reserved device names, control
//! characters, trailing blanks and dots, length), so two conversations never
//! end up in one file by accident of the file system.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use uuid::Uuid;

/// File of the nearby chat.
pub const NEARBY: &str = "chat";
/// GROUP_CHAT_SUFFIX.
pub const GROUP_SUFFIX: &str = " (group)";
/// LL_TRANSCRIPT_FILE_EXTENSION.
pub const EXTENSION: &str = "txt";

/// Longest stem kept, in bytes: far under what file systems take (255
/// UTF-16 units on NTFS) with the date suffix and the extension added.
const MAX_STEM: usize = 180;

/// LLLogChat::cleanFileName: the characters that are illegal in a file name
/// or would match a glob become '_'. Then what Windows cannot hold.
pub fn clean_file_name(name: &str) -> String {
    const INVALID: &str = "\"'\\/?*:.<>|[]{}~";
    let mut out: String = name
        .chars()
        .map(|c| if INVALID.contains(c) || c.is_control() { '_' } else { c })
        .collect();
    if out.len() > MAX_STEM {
        let mut cut = MAX_STEM;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    // Windows drops trailing blanks (dots are already '_'): "Jane " and
    // "Jane" would be one file
    let trimmed = out.trim_end_matches(' ').len();
    out.truncate(trimmed);
    if is_reserved(&out) {
        out.push('_');
    }
    out
}

/// A DOS device name, which Windows resolves to the device whatever the
/// folder and the extension ("NUL.txt", "com1.txt").
fn is_reserved(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
}

/// File of a 1:1 conversation from the resident's legacy name ("Jane Doe",
/// "janedoe Resident"): `user_name.substr(0, user_name.find(" Resident"))`.
pub fn resident(legacy_name: &str) -> String {
    match legacy_name.find(" Resident") {
        Some(i) => legacy_name[..i].to_owned(),
        None => legacy_name.to_owned(),
    }
}

/// File of a group chat.
pub fn group(name: &str) -> String {
    format!("{name}{GROUP_SUFFIX}")
}

/// File of a conference someone invited us to: its name, when the session
/// started for us and the start of its id.
pub fn conference_invited(name: &str, started: SystemTime, session: &Uuid) -> String {
    let id = session.to_string();
    format!("{name} {} {}", super::format::stamp(started, true), &id[..4])
}

/// A conference file name: makeLogFileName never adds the date to those
/// (INBOUND_CONFERENCE / OUTBOUND_CONFERENCE).
fn is_conference(stem: &str) -> bool {
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    if let Some(i) = stem.find("Ad-hoc Conference hash") {
        let id = &stem[i + "Ad-hoc Conference hash".len()..];
        return i == 0
            && id
                .get(..36)
                .is_some_and(|u| Uuid::parse_str(u).is_ok() && u == u.to_ascii_lowercase());
    }
    // "<first> <last> Conference 2010/11/19 03:43 f0f4"
    let Some((who, rest)) = stem.split_once(" Conference ") else {
        return false;
    };
    let mut names = who.split(' ');
    let name_ok = |n: Option<&str>| n.is_some_and(|n| (1..=31).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_alphabetic()));
    if !name_ok(names.next()) || !name_ok(names.next()) || names.next().is_some() {
        return false;
    }
    let mut parts = rest.split(' ');
    let (Some(date), Some(time), Some(id)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    date.len() == 10
        && time.len() == 5
        && super::format::parse_stamp(&format!("{date} {time}")).is_some()
        && id.get(..4).is_some_and(|id| hex(id, 4))
}

/// The date suffix of LogFileNamewithDate, from the local date: a file per
/// day for the nearby chat ("-2026-10-12"), per month for the others
/// ("-2026-10").
pub fn date_suffix(stem: &str, (year, month, day): (i64, i64, i64)) -> String {
    if is_conference(stem) {
        String::new()
    } else if stem == NEARBY {
        format!("-{year:04}-{month:02}-{day:02}")
    } else {
        format!("-{year:04}-{month:02}")
    }
}

/// LLLogChat::makeLogFileName: the file of a conversation in a folder.
/// `today` is the local date when file names carry it. None when the name
/// cleans to nothing.
pub fn file_path(dir: &Path, stem: &str, today: Option<(i64, i64, i64)>) -> Option<PathBuf> {
    let stem = stem.trim();
    let dated = match today {
        Some(d) => format!("{stem}{}", date_suffix(stem, d)),
        None => stem.to_owned(),
    };
    let clean = clean_file_name(&dated);
    (!clean.is_empty()).then(|| dir.join(format!("{clean}.{EXTENSION}")))
}

/// The file to read a conversation's history from: the one of today's name
/// when it exists, else the most recent dated file of that conversation or
/// its undated one (LLLogChat::oldLogFileName).
pub fn existing_file(dir: &Path, stem: &str, today: Option<(i64, i64, i64)>) -> Option<PathBuf> {
    let current = file_path(dir, stem, today)?;
    if current.is_file() {
        return Some(current);
    }
    let clean = clean_file_name(stem.trim());
    // "<name>-????-??.txt", or "chat-????-??-??.txt"
    let pattern_len = if clean == NEARBY { "-0000-00-00".len() } else { "-0000-00".len() };
    let mut dated: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|f| {
            f.strip_suffix(&format!(".{EXTENSION}"))
                .and_then(|s| s.strip_prefix(clean.as_str()))
                .is_some_and(|suffix| is_date_suffix(suffix, pattern_len))
        })
        .collect();
    dated.sort();
    let found = match dated.pop() {
        Some(latest) => dir.join(latest),
        None => dir.join(format!("{clean}.{EXTENSION}")),
    };
    found.is_file().then_some(found)
}

fn is_date_suffix(suffix: &str, pattern_len: usize) -> bool {
    suffix.len() == pattern_len
        && suffix
            .bytes()
            .enumerate()
            .all(|(i, b)| if matches!(i, 0 | 5 | 8) { b == b'-' } else { b.is_ascii_digit() })
}

/// Whether `stem` names a 1:1 resident conversation rather than nearby chat,
/// a group or an ad-hoc conference.
pub fn is_peer_stem(stem: &str) -> bool {
    let s = stem.trim();
    !s.is_empty() && !s.eq_ignore_ascii_case(NEARBY) && !s.ends_with(GROUP_SUFFIX) && !is_conference(s) && !is_conference_file_stem(s)
}

/// Convert a lowercase `first_last` username stem into `First Last`,
/// returning `None` when `base` is not two ASCII lowercase/digit words.
pub(super) fn title_from_username(base: &str) -> Option<String> {
    let word = |w: &str| !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let capital = |w: &str| w[..1].to_ascii_uppercase() + &w[1..];
    let (first, last) = base.split_once('_')?;
    (word(first) && word(last)).then(|| format!("{} {}", capital(first), capital(last)))
}

/// The Legacy form (`Jane Doe`) and, for a two-word name, the username form
/// (`jane_doe`, `LLCacheName::buildUsername` + `LLLogChat::cleanFileName`)
/// of a 1:1 conversation stem. A one-word name (`xyz`, with `" Resident"`
/// stripped) is the same in both schemes and returns `None` for the second.
pub fn peer_stems(stem: &str) -> (String, Option<String>) {
    let s = resident(stem.trim());
    if let Some((first, last)) = s.split_once(' ')
        && !first.is_empty()
        && !last.is_empty()
        && !last.contains(' ')
    {
        let user = format!("{}_{}", first.to_ascii_lowercase(), last.to_ascii_lowercase());
        return (s, Some(user));
    }
    if let Some(legacy) = title_from_username(&s) {
        return (legacy, Some(s));
    }
    (s, None)
}

/// The 1:1 transcript naming scheme of a shared account folder
/// (`UseLegacyIMLogNames` in the Legacy Viewer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NamingScheme {
    /// `Jane Doe.txt` (default of the Legacy Viewer and Catznip).
    #[default]
    Legacy,
    /// `jane_doe.txt` (default of SecondLife, Alchemy and BlackDragon).
    Username,
}

impl NamingScheme {
    /// Stem for a new 1:1 conversation in this scheme.
    pub fn stem(self, stem: &str) -> String {
        let (legacy, username) = peer_stems(stem);
        match self {
            Self::Legacy => legacy,
            Self::Username => username.unwrap_or_else(|| legacy.to_ascii_lowercase()),
        }
    }
}

const CONVERSATION_LOG: &str = super::conversations::FILE;

/// Read `conversation.log` from `path` for the newest 1:1 participant stem
/// (type 0 P2P entry, using the shared parser in `conversations`).
fn conversation_log_stem_at(path: &Path, participant: Uuid) -> Option<String> {
    if participant.is_nil() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    crate::disk_io::read(crate::disk_io::Origin::Logs, bytes.len());
    let text = String::from_utf8_lossy(&bytes);
    let mut best: Option<(i64, String)> = None;
    for line in text.lines() {
        let Some(entry) = super::conversations::parse_line(line) else {
            continue;
        };
        if entry.kind == super::conversations::Kind::Private
            && entry.participant == participant
            && best.as_ref().is_none_or(|(t, _)| entry.time >= *t)
        {
            let stem = clean_file_name(entry.history_file.trim());
            if !stem.is_empty() {
                best = Some((entry.time, stem));
            }
        }
    }
    best.map(|(_, stem)| stem)
}

/// The cleaned history stem recorded in `conversation.log` for a 1:1
/// participant UUID (type 0), taking the newest timestamp when several
/// entries match. Read-only and lenient: missing or malformed data returns
/// `None`.
#[cfg(test)]
pub fn conversation_log_stem(dir: &Path, participant: Uuid) -> Option<String> {
    conversation_log_stem_at(&dir.join(super::conversations::FILE), participant)
}

const NON_TRANSCRIPT_TXT: &[&str] = &[
    "search_history.txt",
    "teleport_history.txt",
    "typed_locations.txt",
    "plugin_cookies.txt",
    "medialist.txt",
];

fn is_non_transcript_txt(name: &str) -> bool {
    NON_TRANSCRIPT_TXT.iter().any(|s| name.eq_ignore_ascii_case(s))
}

/// Strip a `.txt` extension (case-insensitively) on UTF-8 boundaries,
/// returning `None` for empty stems or non-matching extensions.
fn strip_txt_ext(file_name: &str) -> Option<&str> {
    let split = file_name.len().checked_sub(EXTENSION.len() + 1)?;
    let ext = file_name.get(split..)?;
    (split > 0 && ext.as_bytes()[0] == b'.' && ext[1..].eq_ignore_ascii_case(EXTENSION)).then(|| &file_name[..split])
}

fn strip_transcript_txt_stem(name: &str) -> Option<&str> {
    let stem = strip_txt_ext(name)?;
    (!name.contains(".backup") && !is_non_transcript_txt(name)).then_some(stem)
}

fn match_dated_txt<'a>(file_name: &'a str, clean: &str, clean_lower: &str, pattern_len: usize) -> Option<(&'a str, &'a str)> {
    let without_ext = strip_txt_ext(file_name)?;
    let split = without_ext.len().checked_sub(pattern_len)?;
    if split == 0 {
        return None;
    }
    let date_part = without_ext.get(split..)?;
    if !is_date_suffix(date_part, pattern_len) {
        return None;
    }
    let prefix = &without_ext[..split];
    (prefix.eq_ignore_ascii_case(clean) || prefix.to_lowercase() == clean_lower).then_some((date_part, file_name))
}

fn nearby_or_month_pattern_len(clean: &str) -> usize {
    if clean.eq_ignore_ascii_case(NEARBY) {
        "-0000-00-00".len()
    } else {
        "-0000-00".len()
    }
}

/// Single-pass snapshot of regular files in a transcript directory, reused across
/// lookup steps so one conversation lookup performs one `read_dir` call.
struct DirFile {
    name: String,
    lower: String,
}

struct DirEntries {
    dir: PathBuf,
    files: Vec<DirFile>,
}

impl DirEntries {
    #[cfg(test)]
    fn from_names(dir: PathBuf, names: &[&str]) -> Self {
        let files = names
            .iter()
            .map(|&name| DirFile {
                lower: name.to_lowercase(),
                name: name.to_owned(),
            })
            .collect();
        Self { dir, files }
    }

    fn read(dir: &Path) -> Option<Self> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if !ft.is_file() && !dir.join(&name).is_file() {
                continue;
            }
            files.push(DirFile {
                lower: name.to_lowercase(),
                name,
            });
        }
        Some(Self {
            dir: dir.to_path_buf(),
            files,
        })
    }

    fn find_ci(&self, file_name: &str) -> Option<PathBuf> {
        let target_lower = file_name.to_lowercase();
        let mut ci_match = None;
        for f in &self.files {
            if f.name == file_name {
                return Some(self.dir.join(&f.name));
            }
            if ci_match.is_none() && (f.name.eq_ignore_ascii_case(file_name) || f.lower == target_lower) {
                ci_match = Some(self.dir.join(&f.name));
            }
        }
        ci_match
    }

    fn existing_file_with(&self, stem: &str, today: Option<(i64, i64, i64)>) -> Option<PathBuf> {
        let current = file_path(&self.dir, stem, today)?;
        let current_name = current.file_name()?.to_str()?;
        if let Some(found) = self.find_ci(current_name) {
            return Some(found);
        }
        let clean = clean_file_name(stem.trim());
        let clean_lower = clean.to_lowercase();
        let pattern_len = nearby_or_month_pattern_len(&clean);
        let mut dated: Vec<(&str, &str)> = self
            .files
            .iter()
            .filter_map(|f| match_dated_txt(&f.name, &clean, &clean_lower, pattern_len))
            .collect();
        dated.sort();
        if let Some((_, latest)) = dated.pop() {
            return Some(self.dir.join(latest));
        }
        self.find_ci(&format!("{clean}.{EXTENSION}"))
    }

    fn conversation_log_stem(&self, participant: Uuid) -> Option<String> {
        let conv_path = self.find_ci(CONVERSATION_LOG)?;
        conversation_log_stem_at(&conv_path, participant)
    }

    /// All on-disk files matching `stem` (dated in chronological order, then
    /// undated) using a single directory snapshot.
    fn all_matching_files(&self, stem: &str) -> Vec<PathBuf> {
        let Some(clean) = Some(clean_file_name(stem.trim())).filter(|c| !c.is_empty()) else {
            return Vec::new();
        };
        let clean_lower = clean.to_lowercase();
        let pattern_len = nearby_or_month_pattern_len(&clean);
        let mut matched: Vec<(u8, &str, &str)> = self
            .files
            .iter()
            .filter_map(|f| {
                if f.name.contains(".backup") {
                    return None;
                }
                let without_ext = strip_txt_ext(&f.name)?;
                if without_ext.eq_ignore_ascii_case(&clean) || without_ext.to_lowercase() == clean_lower {
                    return Some((1, "", f.name.as_str()));
                }
                let (date_part, name) = match_dated_txt(&f.name, &clean, &clean_lower, pattern_len)?;
                Some((0, date_part, name))
            })
            .collect();
        matched.sort();
        matched.into_iter().map(|(_, _, name)| self.dir.join(name)).collect()
    }
}

/// `existing_file` matched case-insensitively against `dir`.
#[cfg(test)]
pub fn existing_file_ci(dir: &Path, stem: &str, today: Option<(i64, i64, i64)>) -> Option<PathBuf> {
    DirEntries::read(dir).and_then(|entries| entries.existing_file_with(stem, today))
}

/// Whether two paths refer to the same underlying file on disk.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    if let (Ok(ca), Ok(cb)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return ca == cb;
    }
    false
}

/// Existing candidate transcript files for a conversation in a shared folder,
/// in priority order (`conversation.log`, then Legacy form, then username
/// form, each with dated variants through `existing_file_ci`), deduplicated
/// by file identity: one per spelling (tests; the history reads every file,
/// `all_candidate_files`).
#[cfg(test)]
pub fn candidate_files(dir: &Path, peer: Option<Uuid>, stem: &str, today: Option<(i64, i64, i64)>) -> Vec<PathBuf> {
    let Some(entries) = DirEntries::read(dir) else {
        return Vec::new();
    };
    if !is_peer_stem(stem) {
        return entries.existing_file_with(stem, today).into_iter().collect();
    }
    let mut out: Vec<PathBuf> = Vec::with_capacity(3);
    let mut push_unique = |path: PathBuf| {
        if !out.iter().any(|existing| same_file(existing, &path)) {
            out.push(path);
        }
    };
    if let Some(id) = peer
        && let Some(conv_stem) = entries.conversation_log_stem(id)
        && let Some(path) = entries.existing_file_with(&conv_stem, today)
    {
        push_unique(path);
    }
    let (legacy, username) = peer_stems(stem);
    if let Some(path) = entries.existing_file_with(&legacy, today) {
        push_unique(path);
    }
    if let Some(user) = username
        && let Some(path) = entries.existing_file_with(&user, today)
    {
        push_unique(path);
    }
    out
}

/// All existing transcript files (across all dates and undated variants) for
/// `stem` in `dir`, deduplicated by file identity.
pub fn all_files_for_stem(dir: &Path, stem: &str) -> Vec<PathBuf> {
    DirEntries::read(dir)
        .map(|entries| entries.all_matching_files(stem))
        .unwrap_or_default()
}

/// All existing candidate transcript files (across all dates, undated variants,
/// `conversation.log`, Legacy form, and username form) for a conversation in a
/// shared folder, in priority order and deduplicated by file identity.
pub fn all_candidate_files(dir: &Path, peer: Option<Uuid>, stem: &str) -> Vec<PathBuf> {
    let Some(entries) = DirEntries::read(dir) else {
        return Vec::new();
    };
    if !is_peer_stem(stem) {
        return entries.all_matching_files(stem);
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !out.iter().any(|existing: &PathBuf| same_file(existing, &path)) {
            out.push(path);
        }
    };
    if let Some(id) = peer
        && let Some(conv_stem) = entries.conversation_log_stem(id)
    {
        for path in entries.all_matching_files(&conv_stem) {
            push_unique(path);
        }
    }
    let (legacy, username) = peer_stems(stem);
    for path in entries.all_matching_files(&legacy) {
        push_unique(path);
    }
    if let Some(user) = username {
        for path in entries.all_matching_files(&user) {
            push_unique(path);
        }
    }
    out
}

fn pick_newest_by_mtime(paths: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for path in paths {
        let mtime = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(best_time, _)| mtime > *best_time) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, path)| path)
}

/// The stem of a transcript file: without folder, `.txt` and date suffix
/// (what `conversation.log` records, LLConversation::mHistoryFileName).
pub(super) fn stem_from_matched_file(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let stem = strip_txt_ext(file_name)?;
    Some(strip_date_suffix(stem).to_owned())
}

fn best_peer_file_with(entries: &DirEntries, peer: Option<Uuid>, stem: &str, today: Option<(i64, i64, i64)>) -> Option<PathBuf> {
    if let Some(id) = peer
        && let Some(conv_stem) = entries.conversation_log_stem(id)
        && let Some(path) = entries.existing_file_with(&conv_stem, today)
    {
        return Some(path);
    }
    let (legacy, username) = peer_stems(stem);
    let mut candidates = Vec::with_capacity(2);
    if let Some(path) = entries.existing_file_with(&legacy, today) {
        candidates.push(path);
    }
    if let Some(user) = username
        && let Some(path) = entries.existing_file_with(&user, today)
        && !candidates.iter().any(|p| same_file(p, &path))
    {
        candidates.push(path);
    }
    pick_newest_by_mtime(candidates)
}

fn shared_chosen_stem(entries: &DirEntries, peer: Option<Uuid>, stem: &str, today: Option<(i64, i64, i64)>) -> Option<String> {
    best_peer_file_with(entries, peer, stem, today).and_then(|p| stem_from_matched_file(&p))
}

/// Best single existing file path for a conversation on disk (used when
/// opening a transcript externally from the Conversation Log window).
pub fn existing_source_file(dir: &Path, peer: Option<Uuid>, stem: &str, today: Option<(i64, i64, i64)>) -> Option<PathBuf> {
    let entries = DirEntries::read(dir)?;
    if !is_peer_stem(stem) {
        return entries.existing_file_with(stem, today);
    }
    best_peer_file_with(&entries, peer, stem, today)
}

/// The file a conversation is appended to in a shared folder:
/// picks the spelling (`conversation.log` entry if its file exists, else the
/// spelling of the most recently modified existing candidate, else the folder's
/// detected `scheme`) and formats the current target via `file_path(dir, stem, today)`
/// so dated rollovers and undated-vs-dated preferences always write to the
/// right period's file.
pub fn shared_file_path(
    dir: &Path,
    peer: Option<Uuid>,
    stem: &str,
    today: Option<(i64, i64, i64)>,
    scheme: NamingScheme,
) -> Option<PathBuf> {
    let entries = DirEntries::read(dir);
    let resolve_target = |chosen_stem: &str| -> Option<PathBuf> {
        let target = file_path(dir, chosen_stem, today)?;
        if let Some(entries) = entries.as_ref()
            && let Some(name) = target.file_name().and_then(|n| n.to_str())
            && let Some(found) = entries.find_ci(name)
        {
            return Some(found);
        }
        Some(target)
    };
    if !is_peer_stem(stem) {
        return resolve_target(stem);
    }
    let chosen = entries.as_ref().and_then(|e| shared_chosen_stem(e, peer, stem, today));
    resolve_target(chosen.as_deref().unwrap_or(&scheme.stem(stem)))
}

pub(super) fn strip_date_suffix(stem: &str) -> &str {
    for len in ["-0000-00-00".len(), "-0000-00".len()] {
        if let Some(split) = stem.len().checked_sub(len)
            && split > 0
            && let Some(tail) = stem.get(split..)
            && is_date_suffix(tail, len)
        {
            return &stem[..split];
        }
    }
    stem
}

/// Whether an undated on-disk file stem is a conference file (`hash<uuid>`,
/// `" Conference"`, or a `<date> <time> <4 hex>` suffix with `/`/`:` cleaned
/// to `_`).
fn is_conference_file_stem(stem: &str) -> bool {
    if is_conference(stem) || stem.ends_with(" Conference") {
        return true;
    }
    let mut rest = stem;
    while let Some(i) = rest.find("hash") {
        let after = &rest[i + 4..];
        if after.get(..36).is_some_and(|u| Uuid::parse_str(u).is_ok()) {
            return true;
        }
        rest = after;
    }
    // "<name> YYYY_MM_DD HH_MM xxxx" (21 bytes at the end after a space)
    if let Some(tail_start) = stem.len().checked_sub(22)
        && tail_start > 0
    {
        let b = &stem.as_bytes()[tail_start..];
        let sep = |c: u8| matches!(c, b'/' | b':' | b'_' | b'-');
        let is_dated_conf = b[0] == b' '
            && b[1..5].iter().all(u8::is_ascii_digit)
            && sep(b[5])
            && b[6..8].iter().all(u8::is_ascii_digit)
            && sep(b[8])
            && b[9..11].iter().all(u8::is_ascii_digit)
            && b[11] == b' '
            && b[12..14].iter().all(u8::is_ascii_digit)
            && sep(b[14])
            && b[15..17].iter().all(u8::is_ascii_digit)
            && b[17] == b' '
            && b[18..22].iter().all(u8::is_ascii_hexdigit);
        if is_dated_conf {
            return true;
        }
    }
    false
}

/// Classify a file name in an account folder by the 1:1 naming scheme it
/// shows (`None` for one-word names, `chat*.txt`, `* (group).txt`,
/// conferences, viewer state `.txt` files, and `*.backup*`).
fn classify_scheme_file(file_name: &str) -> Option<NamingScheme> {
    let stem = strip_transcript_txt_stem(file_name)?;
    let undated = strip_date_suffix(stem);
    if undated.is_empty() || undated.eq_ignore_ascii_case(NEARBY) || undated.ends_with(GROUP_SUFFIX) || is_conference_file_stem(undated) {
        return None;
    }
    // Username form: ^[a-z0-9]+_[a-z0-9]+(-\d{4}-\d{2}(-\d{2})?)?\.txt$
    if file_name.ends_with(&format!(".{EXTENSION}"))
        && let Some((first, last)) = undated.split_once('_')
        && !first.is_empty()
        && !last.is_empty()
        && first.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && last.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        return Some(NamingScheme::Username);
    }
    // Legacy form: two words separated by a space
    if let Some((first, last)) = undated.split_once(' ')
        && !first.is_empty()
        && !last.is_empty()
        && !last.contains(' ')
        && !first.contains('_')
        && !last.contains('_')
    {
        return Some(NamingScheme::Legacy);
    }
    None
}

const SCHEME_SAMPLE: usize = 20;

fn pick_scheme_from_samples(mut files: Vec<(SystemTime, NamingScheme)>) -> Option<NamingScheme> {
    if files.is_empty() {
        return None;
    }
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    let sample = &files[..files.len().min(SCHEME_SAMPLE)];
    let newest_scheme = sample[0].1;
    let newest_min = sample
        .iter()
        .filter(|(_, s)| *s == newest_scheme)
        .map(|(t, _)| *t)
        .min()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let other_max = sample.iter().filter(|(_, s)| *s != newest_scheme).map(|(t, _)| *t).max();
    if other_max.is_none_or(|other| newest_min > other) {
        return Some(newest_scheme);
    }
    let username = sample.iter().filter(|(_, s)| *s == NamingScheme::Username).count();
    let legacy = sample.len() - username;
    if username > legacy {
        Some(NamingScheme::Username)
    } else if legacy > username {
        Some(NamingScheme::Legacy)
    } else {
        Some(newest_scheme)
    }
}

/// Probe a shared account folder `dir` under `root` in a single directory
/// scan, returning its detected 1:1 naming scheme and the number of existing
/// transcript / `conversation.log` files.
pub fn probe_shared_dir(dir: &Path, root: Option<&Path>) -> (NamingScheme, usize) {
    let default = if crate::legacy_profile::uses_username_log_names(root) {
        NamingScheme::Username
    } else {
        NamingScheme::Legacy
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (default, 0);
    };
    // Compare modification time (`mtime`) rather than creation time (`btime`):
    // copying folders, extracting backups or syncing with OneDrive resets
    // `btime` while preserving `mtime`.
    let mut files: Vec<(SystemTime, NamingScheme)> = Vec::new();
    let mut txt_count = 0usize;
    let mut has_conv_log = false;
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else {
            continue;
        };
        if ft.is_dir() {
            continue;
        }
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if !ft.is_file() && !dir.join(&name).is_file() {
            continue;
        }
        if name.eq_ignore_ascii_case(CONVERSATION_LOG) {
            has_conv_log = true;
            continue;
        }
        if is_transcript_txt_name(&name) {
            txt_count += 1;
        }
        if let Some(scheme) = classify_scheme_file(&name) {
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            files.push((mtime, scheme));
        }
    }
    let legacy_count = if txt_count > 0 { txt_count } else { usize::from(has_conv_log) };
    let scheme = pick_scheme_from_samples(files).unwrap_or(default);
    (scheme, legacy_count)
}

/// Detect the 1:1 naming scheme of an account folder `dir` under `root`.
/// The newest qualifying 1:1 files win over an older opposite-scheme
/// majority; when none exist, falls back to the default of the viewer that
/// owns `root`.
#[cfg(test)]
pub fn detect_scheme(dir: &Path, root: Option<&Path>) -> NamingScheme {
    probe_shared_dir(dir, root).0
}

/// Count of existing transcript `.txt` files (or `1` when only
/// `conversation.log` is present) in a single directory pass without opening
/// or stat'ing files.
pub fn legacy_log_count(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut txt_count = 0usize;
    let mut has_conv_log = false;
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else {
            continue;
        };
        if ft.is_dir() {
            continue;
        }
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if !ft.is_file() && !dir.join(&name).is_file() {
            continue;
        }
        if name.eq_ignore_ascii_case(CONVERSATION_LOG) {
            has_conv_log = true;
        } else if is_transcript_txt_name(&name) {
            txt_count += 1;
        }
    }
    if txt_count > 0 { txt_count } else { usize::from(has_conv_log) }
}

fn is_transcript_txt_name(name: &str) -> bool {
    strip_transcript_txt_stem(name).is_some()
}

fn has_stamped_line(path: &Path) -> bool {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(64);
    if f.take(4096).read_to_end(&mut head).is_err() {
        return false;
    }
    crate::disk_io::read(crate::disk_io::Origin::Logs, head.len());
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&head);
    String::from_utf8_lossy(head).lines().any(super::format::starts_with_stamp)
}

/// Whether `dir` contains a `conversation.log` file (case-insensitively).
#[cfg(test)]
pub fn has_conversation_log(dir: &Path) -> bool {
    DirEntries::read(dir).and_then(|e| e.find_ci(CONVERSATION_LOG)).is_some() || dir.join(CONVERSATION_LOG).is_file()
}

/// Whether `dir` directly holds Legacy Viewer per-account files
/// (`conversation.log`, `settings_per_account.xml`, viewer history `.txt`
/// files, nearby `chat*.txt`, timestamped transcripts, or any transcript `.txt`
/// when `dir` sits inside a known viewer root). Stops at the first match.
pub fn has_legacy_account_files(dir: &Path, under_viewer_root: bool) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut txt_candidates = Vec::new();
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else {
            continue;
        };
        if ft.is_dir() {
            continue;
        }
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if !ft.is_file() && !dir.join(&name).is_file() {
            continue;
        }
        if name.eq_ignore_ascii_case(CONVERSATION_LOG)
            || name.eq_ignore_ascii_case("settings_per_account.xml")
            || is_non_transcript_txt(&name)
        {
            return true;
        }
        let Some(stem) = strip_transcript_txt_stem(&name) else {
            continue;
        };
        if under_viewer_root || strip_date_suffix(stem).eq_ignore_ascii_case(NEARBY) {
            return true;
        }
        txt_candidates.push(name);
    }
    txt_candidates.into_iter().any(|name| has_stamped_line(&dir.join(name)))
}

/// Count of existing transcript `.txt` files in `dir` (excluding `.backup*`
/// and known viewer state files), using directory entries only.
#[cfg(test)]
pub fn transcript_file_count(dir: &Path) -> usize {
    DirEntries::read(dir)
        .map(|e| e.files.iter().filter(|f| is_transcript_txt_name(&f.name)).count())
        .unwrap_or(0)
}

/// Today's local date (LogFileNamewithDate uses localtime).
#[cfg(windows)]
pub fn local_date() -> (i64, i64, i64) {
    let mut time = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    // SAFETY: GetLocalTime writes one initialized SYSTEMTIME through this
    // valid exclusive pointer, retains nothing, and needs no external input.
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut time);
    }
    (time.wYear.into(), time.wMonth.into(), time.wDay.into())
}

#[cfg(not(windows))]
pub fn local_date() -> (i64, i64, i64) {
    use chrono::Datelike;
    let t = chrono::Local::now();
    (t.year().into(), t.month().into(), t.day().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firestorms_forbidden_characters() {
        assert_eq!(clean_file_name("Jane Doe"), "Jane Doe");
        assert_eq!(
            clean_file_name("a\"b'c\\d/e?f*g:h.i<j>k|l[m]n{o}p~q"),
            "a_b_c_d_e_f_g_h_i_j_k_l_m_n_o_p_q"
        );
        assert_eq!(clean_file_name("Ælfrëd 猫"), "Ælfrëd 猫");
        assert_eq!(clean_file_name("../../etc/passwd"), "______etc_passwd");
    }

    #[test]
    fn what_windows_refuses() {
        for name in ["CON", "nul", "Com1", "LPT9", "aux", "PRN"] {
            assert_eq!(clean_file_name(name), format!("{name}_"), "{name}");
        }
        assert_eq!(clean_file_name("COM0"), "COM0");
        assert_eq!(clean_file_name("CONSOLE"), "CONSOLE");
        assert_eq!(clean_file_name("tab\there\u{7}"), "tab_here_");
        // trailing blanks and dots would name the same file as without them
        assert_eq!(clean_file_name("Jane  "), "Jane");
        assert_eq!(clean_file_name("Jane."), "Jane_");
        let long = "é".repeat(400);
        let clean = clean_file_name(&long);
        assert!(clean.len() <= MAX_STEM && clean.chars().all(|c| c == 'é'));
    }

    #[test]
    fn conversation_files() {
        assert_eq!(resident("Jane Doe"), "Jane Doe");
        assert_eq!(resident("janedoe Resident"), "janedoe");
        assert_eq!(group("Les Amis: du Loup"), "Les Amis: du Loup (group)");
        let dir = Path::new("logs");
        assert_eq!(file_path(dir, NEARBY, None), Some(dir.join("chat.txt")));
        assert_eq!(file_path(dir, &resident("janedoe Resident"), None), Some(dir.join("janedoe.txt")));
        assert_eq!(
            file_path(dir, &group("Les Amis: du Loup"), None),
            Some(dir.join("Les Amis_ du Loup (group).txt"))
        );
        assert_eq!(file_path(dir, "  ", None), None);
    }

    #[test]
    fn conference_files() {
        let session = Uuid::parse_str("f0f4c0de-0000-4000-8000-000000000001").unwrap();
        let started = super::super::format::parse_stamp("2010/11/19 03:43").unwrap();
        let stem = conference_invited("Jane Doe Conference", started, &session);
        assert_eq!(stem, "Jane Doe Conference 2010/11/19 03:43 f0f4");
        assert!(is_conference(&stem));
        assert_eq!(
            file_path(Path::new("logs"), &stem, Some((2026, 10, 12))),
            Some(Path::new("logs").join("Jane Doe Conference 2010_11_19 03_43 f0f4.txt"))
        );
        // Firestorm's name of a conference started by the user
        assert!(is_conference("Ad-hoc Conference hash0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0"));
        assert!(!is_conference("Ad-hoc Conference hashed"));
        assert!(!is_conference("Jane Doe"));
        assert!(!is_conference("Conference of the birds (group)"));
    }

    #[test]
    fn date_in_the_names() {
        let dir = Path::new("logs");
        let today = Some((2026, 3, 7));
        assert_eq!(file_path(dir, NEARBY, today), Some(dir.join("chat-2026-03-07.txt")));
        assert_eq!(file_path(dir, "Jane Doe", today), Some(dir.join("Jane Doe-2026-03.txt")));
        assert_eq!(file_path(dir, &group("Loups"), today), Some(dir.join("Loups (group)-2026-03.txt")));
    }

    #[test]
    fn history_comes_from_the_latest_file() {
        let dir = std::env::temp_dir().join(format!("aurora-chatlog-names-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(existing_file(&dir, "Jane Doe", None), None);
        std::fs::write(dir.join("Jane Doe.txt"), "a").unwrap();
        assert_eq!(existing_file(&dir, "Jane Doe", None), Some(dir.join("Jane Doe.txt")));
        // dated names on: no file for this month yet, the undated one serves
        assert_eq!(
            existing_file(&dir, "Jane Doe", Some((2026, 10, 12))),
            Some(dir.join("Jane Doe.txt"))
        );
        std::fs::write(dir.join("Jane Doe-2026-08.txt"), "b").unwrap();
        std::fs::write(dir.join("Jane Doe-2026-09.txt"), "c").unwrap();
        // someone else's file with a longer name is not ours
        std::fs::write(dir.join("Jane Doela-2026-09.txt"), "d").unwrap();
        assert_eq!(
            existing_file(&dir, "Jane Doe", Some((2026, 10, 12))),
            Some(dir.join("Jane Doe-2026-09.txt"))
        );
        std::fs::write(dir.join("Jane Doe-2026-10.txt"), "e").unwrap();
        assert_eq!(
            existing_file(&dir, "Jane Doe", Some((2026, 10, 12))),
            Some(dir.join("Jane Doe-2026-10.txt"))
        );
        // dated names off again: the undated file is the current one
        assert_eq!(existing_file(&dir, "Jane Doe", None), Some(dir.join("Jane Doe.txt")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct TempDir(PathBuf);

    impl std::ops::Deref for TempDir {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> TempDir {
        let dir = std::env::temp_dir().join(format!("aurora-chatlog-names-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        TempDir(dir)
    }

    #[test]
    fn conversation_log_parsing_and_newest_entry() {
        let dir = temp_dir();
        let jane = Uuid::from_u128(0x1111);
        let adhoc_peer = Uuid::from_u128(0x2222);
        let group_id = Uuid::from_u128(0x3333);
        let session = Uuid::from_u128(0x9999);
        let content = format!(
            "not a valid line\n\
             [bad] 0 0 0 Jane| {jane} {session} jane.doe|\n\
             [1700000100] 0 0 0 Jane%20Doe| {jane} {session} Jane%20Doe|\n\
             [1700000200] 0 1 0 Jane%20Doe| {jane} {session} jane.doe:custom|\n\
             [1700000150] 0 0 0 Jane%20Doe| {jane} {session} older.entry|\n\
             [1700000300] 1 0 0 Group| {group_id} {session} Group%20(group)|\n\
             [1700000400] 2 0 0 Ad-hoc%20Conf| {adhoc_peer} {session} Ad-hoc%20Conference hash{session}|\n"
        );
        std::fs::write(dir.join("conversation.log"), content).expect("write conversation.log");

        // Type 0 entry with the newest timestamp wins, URL-decoded and cleaned
        assert_eq!(conversation_log_stem(&dir, jane).as_deref(), Some("jane_doe_custom"));
        // Group (type 1) and ad-hoc (type 2) entries are not returned for 1:1 P2P lookups
        assert_eq!(conversation_log_stem(&dir, group_id), None);
        assert_eq!(conversation_log_stem(&dir, adhoc_peer), None);

        // Ad-hoc line is not URL-decoded (LLConversationLog::loadFromFile)
        let adhoc_line = format!("[1700000400] 2 0 0 Ad-hoc%20Conf| {adhoc_peer} {session} Ad-hoc%20Conference hash{session}|");
        let parsed = crate::world::chat_log::conversations::parse_line(&adhoc_line).expect("parsed ad-hoc");
        assert_eq!(parsed.kind, crate::world::chat_log::conversations::Kind::Conference);
        assert_eq!(parsed.name, "Ad-hoc%20Conf");
        assert_eq!(parsed.history_file, format!("Ad-hoc%20Conference hash{session}"));
    }

    #[test]
    fn candidates_for_two_word_one_word_dated_and_case_insensitive() {
        let dir = temp_dir();
        let jane = Uuid::from_u128(0x42);
        let session = Uuid::from_u128(0x99);

        // Both forms exist for a two-word name, plus a custom stem in conversation.log
        std::fs::write(
            dir.join("conversation.log"),
            format!("[1700000500] 0 0 0 Jane%20Doe| {jane} {session} jane.custom|\n"),
        )
        .unwrap();
        let f_conv = dir.join("jane_custom.txt");
        let f_legacy = dir.join("Jane Doe.txt");
        let f_user = dir.join("jane_doe-2026-09.txt");
        std::fs::write(&f_conv, "1").unwrap();
        std::fs::write(&f_legacy, "2").unwrap();
        std::fs::write(&f_user, "3").unwrap();

        let found = candidate_files(&dir, Some(jane), "Jane Doe", None);
        assert_eq!(found, vec![f_conv, f_legacy, f_user]);

        // One-word and "xyz Resident" names produce a single candidate and match case-insensitively
        let f_xyz = dir.join("XYZ.txt");
        std::fs::write(&f_xyz, "hi").unwrap();
        let xyz_direct = candidate_files(&dir, None, "xyz", None);
        let xyz_resident = candidate_files(&dir, None, "xyz Resident", None);
        assert_eq!(xyz_direct.len(), 1);
        assert_eq!(xyz_resident.len(), 1);
        assert!(same_file(&xyz_direct[0], &f_xyz));
        assert!(same_file(&xyz_resident[0], &f_xyz));
    }

    #[test]
    fn scheme_detection_newest_wins_and_root_defaults() {
        let root_fs = temp_dir();
        let fs_dir = root_fs.join("Firestorm_x64").join("bob_smith");
        std::fs::create_dir_all(&fs_dir).unwrap();

        // Empty folder falls back to the root viewer's default
        assert_eq!(detect_scheme(&fs_dir, fs_dir.parent()), NamingScheme::Legacy);
        for (viewer, expected) in [
            ("Firestorm_x64", NamingScheme::Legacy),
            ("Aperture Viewer_x64", NamingScheme::Legacy),
            ("Catznip", NamingScheme::Legacy),
            ("CustomFolder", NamingScheme::Legacy),
            ("SecondLife", NamingScheme::Username),
            ("AlchemyNext", NamingScheme::Username),
            ("Alchemy", NamingScheme::Username),
            ("BlackDragon", NamingScheme::Username),
        ] {
            let v_root = root_fs.join(viewer);
            let v_dir = v_root.join("bob_smith");
            assert_eq!(detect_scheme(&v_dir, Some(&v_root)), expected, "{viewer}");
        }

        // Ignored file kinds (including viewer history state .txt files) do not count toward either scheme
        for ignored in [
            "chat.txt",
            "chat-2026-10-12.txt",
            "Northern Wolves (group).txt",
            "Northern Wolves (group)-2026-10.txt",
            "Ad-hoc Conference hash0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0.txt",
            "Jane Doe Conference 2010_11_19 03_43 f0f4.txt",
            "xyz.txt",
            "jane_doe.txt.backup1",
            "search_history.txt",
            "teleport_history.txt",
            "typed_locations.txt",
            "plugin_cookies.txt",
            "medialist.txt",
        ] {
            std::fs::write(fs_dir.join(ignored), "ignored").unwrap();
        }
        let sl_root = root_fs.join("SecondLife");
        assert_eq!(detect_scheme(&fs_dir, Some(&sl_root)), NamingScheme::Username);
        assert_eq!(detect_scheme(&fs_dir, Some(&root_fs.join("Firestorm_x64"))), NamingScheme::Legacy);

        // Newer non-transcript .txt state files cannot outweigh an older Legacy transcript
        let t_old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let t_mid = t_old + std::time::Duration::from_secs(1_800);
        let t_new = t_old + std::time::Duration::from_secs(3_600);
        let legacy_one = fs_dir.join("Older Friend.txt");
        std::fs::File::create(&legacy_one).unwrap().set_modified(t_old).unwrap();
        for state_name in ["search_history.txt", "teleport_history.txt", "typed_locations.txt"] {
            std::fs::File::options()
                .write(true)
                .open(fs_dir.join(state_name))
                .unwrap()
                .set_modified(t_new)
                .unwrap();
        }
        assert_eq!(detect_scheme(&fs_dir, fs_dir.parent()), NamingScheme::Legacy);
        std::fs::remove_file(&legacy_one).unwrap();

        // Newest files win over a larger opposite-scheme majority
        for i in 0..10 {
            let p = fs_dir.join(format!("Resident One{i}.txt"));
            let f = std::fs::File::create(&p).unwrap();
            f.set_modified(t_old).unwrap();
        }
        for i in 0..2 {
            let p = fs_dir.join(format!("resident_two{i}.txt"));
            let f = std::fs::File::create(&p).unwrap();
            f.set_modified(t_new).unwrap();
        }
        assert_eq!(detect_scheme(&fs_dir, fs_dir.parent()), NamingScheme::Username);

        // Mixed overlapping mtimes within the SCHEME_SAMPLE window fall back to the window's majority
        let mix_dir = root_fs.join("MixedSample").join("bob_smith");
        std::fs::create_dir_all(&mix_dir).unwrap();
        for i in 0..25 {
            let p = mix_dir.join(format!("Very Old{i}.txt"));
            std::fs::File::create(&p).unwrap().set_modified(t_old).unwrap();
        }
        for i in 0..8 {
            let p = mix_dir.join(format!("Sample Legacy{i}.txt"));
            std::fs::File::create(&p).unwrap().set_modified(t_mid).unwrap();
        }
        for i in 0..12 {
            let p = mix_dir.join(format!("sample_user{i}.txt"));
            let t = if i == 0 { t_mid } else { t_new };
            std::fs::File::create(&p).unwrap().set_modified(t).unwrap();
        }
        assert_eq!(detect_scheme(&mix_dir, None), NamingScheme::Username);
    }

    #[test]
    fn shared_append_target_priority_and_dated_rollover() {
        let dir = temp_dir();
        let jane = Uuid::from_u128(0x77);
        let session = Uuid::from_u128(0x88);
        let oct = Some((2026, 10, 12));

        // 1. No existing candidate: uses the folder's detected scheme
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", None, NamingScheme::Username),
            Some(dir.join("jane_doe.txt"))
        );
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", None, NamingScheme::Legacy),
            Some(dir.join("Jane Doe.txt"))
        );

        // 2. Old dated candidate picks the spelling without reusing the expired file
        let f_user_sept = dir.join("jane_doe-2026-09.txt");
        std::fs::write(&f_user_sept, "sept").unwrap();
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", oct, NamingScheme::Legacy),
            Some(dir.join("jane_doe-2026-10.txt"))
        );
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", None, NamingScheme::Legacy),
            Some(dir.join("jane_doe.txt"))
        );
        std::fs::remove_file(&f_user_sept).unwrap();

        // 3. Undated non-1:1 appends do not write into an old dated file
        std::fs::write(dir.join("chat-2026-09-10.txt"), "old nearby").unwrap();
        std::fs::write(dir.join("Northern Wolves (group)-2026-09.txt"), "old group").unwrap();
        assert_eq!(
            shared_file_path(&dir, None, NEARBY, None, NamingScheme::Legacy),
            Some(dir.join("chat.txt"))
        );
        assert_eq!(
            shared_file_path(&dir, None, "Northern Wolves (group)", None, NamingScheme::Legacy),
            Some(dir.join("Northern Wolves (group).txt"))
        );

        // 4. Undated candidate exists: never creates the second spelling even if scheme differs
        let f_user = dir.join("jane_doe.txt");
        let f_legacy = dir.join("Jane Doe.txt");
        std::fs::write(&f_user, "u").unwrap();
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", None, NamingScheme::Legacy),
            Some(f_user.clone())
        );

        // 5. Both exist: the most recently modified candidate wins
        let t_old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let t_new = t_old + std::time::Duration::from_secs(3_600);
        std::fs::File::options()
            .write(true)
            .open(&f_user)
            .unwrap()
            .set_modified(t_old)
            .unwrap();
        let f = std::fs::File::create(&f_legacy).unwrap();
        f.set_modified(t_new).unwrap();
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", None, NamingScheme::Username),
            Some(f_legacy.clone())
        );

        // 6. conversation.log entry whose file exists wins and rolls forward with preserved casing
        std::fs::write(dir.join("Jane_Custom-2026-09.txt"), "custom").unwrap();
        std::fs::write(
            dir.join("conversation.log"),
            format!("[1700000900] 0 0 0 Jane%20Doe| {jane} {session} jane.custom|\n"),
        )
        .unwrap();
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", oct, NamingScheme::Legacy),
            Some(dir.join("Jane_Custom-2026-10.txt"))
        );
        assert_eq!(
            shared_file_path(&dir, Some(jane), "Jane Doe", None, NamingScheme::Legacy),
            Some(dir.join("Jane_Custom.txt"))
        );
    }

    #[test]
    fn non_ascii_names_and_subdirs_never_panic() {
        let dir = temp_dir();
        std::fs::create_dir_all(dir.join("日本語")).unwrap();
        for (name, body) in [
            ("Club 💜 (group).txt", "group"),
            ("café.md", "note"),
            ("💜.md", "emoji note"),
            ("💜.txt", "[2026/10/12 14:00]  A: hi\n"),
            ("a💜b.txt", "text"),
            ("💜aaaaaaa.txt", "bytes"),
            ("💜a💜aaaaaa.txt", "boundary"),
            ("Jane Doe-2026-09.txt", "[2026/09/12 14:00]  Jane Doe: hi\n"),
        ] {
            std::fs::write(dir.join(name), body).unwrap();
            let _ = classify_scheme_file(name);
            let _ = is_transcript_txt_name(name);
            let _ = strip_date_suffix(name);
            let _ = is_conference_file_stem(name);
        }
        assert_eq!(classify_scheme_file("💜.md"), None);
        assert_eq!(classify_scheme_file("Club 💜 (group).txt"), None);
        assert_eq!(transcript_file_count(&dir), 6);
        assert!(has_legacy_account_files(&dir, false));
        assert_eq!(detect_scheme(&dir, None), NamingScheme::Legacy);
        assert_eq!(
            existing_file_ci(&dir, "Club 💜 (group)", None),
            Some(dir.join("Club 💜 (group).txt"))
        );
        assert_eq!(candidate_files(&dir, None, "💜", Some((2026, 10, 12))), vec![dir.join("💜.txt")]);
        assert_eq!(
            shared_file_path(&dir, None, "Club 💜 (group)", Some((2026, 10, 12)), NamingScheme::Username),
            Some(dir.join("Club 💜 (group)-2026-10.txt"))
        );
    }

    #[test]
    fn legacy_account_files_detection() {
        let dir = temp_dir();
        // Plain unstamped note is ignored outside a viewer root, counted inside one
        std::fs::write(dir.join("notes.txt"), "hello world\n").unwrap();
        assert!(!has_legacy_account_files(&dir, false));
        assert!(has_legacy_account_files(&dir, true));
        assert!(!has_conversation_log(&dir));

        // Stamped transcript line or chat.txt or settings_per_account.xml or conversation.log counts anywhere
        let sub_stamped = dir.join("stamped");
        std::fs::create_dir_all(&sub_stamped).unwrap();
        std::fs::write(sub_stamped.join("Jane Doe.txt"), "[2026/10/12 14:00]  Jane Doe: hi\n").unwrap();
        assert!(has_legacy_account_files(&sub_stamped, false));

        let sub_chat = dir.join("chatdir");
        std::fs::create_dir_all(&sub_chat).unwrap();
        std::fs::write(sub_chat.join("chat-2026-10-12.txt"), "").unwrap();
        assert!(has_legacy_account_files(&sub_chat, false));

        let sub_settings = dir.join("settingsdir");
        std::fs::create_dir_all(&sub_settings).unwrap();
        std::fs::write(sub_settings.join("settings_per_account.xml"), "<llsd/>").unwrap();
        assert!(has_legacy_account_files(&sub_settings, false));

        let sub_conv = dir.join("convdir");
        std::fs::create_dir_all(&sub_conv).unwrap();
        std::fs::write(sub_conv.join("conversation.log"), "").unwrap();
        assert!(has_conversation_log(&sub_conv));
        assert!(has_legacy_account_files(&sub_conv, false));
        assert_eq!(legacy_log_count(&sub_conv), 1);
    }

    #[test]
    fn case_sensitive_directory_lookup_preserves_on_disk_casing() {
        // Simulate a case-sensitive directory where multiple casings can coexist and lookups
        // cannot rely on `Path::is_file()` matching across case differences.
        let virtual_dir = PathBuf::from("/case-sensitive/account");
        let simulated = DirEntries::from_names(
            virtual_dir.clone(),
            &["CONVERSATION.LOG", "JANE_DOE-2026-09.TXT", "jane doe-2026-10.Txt", "Jane Doe.TXT"],
        );
        assert_eq!(simulated.find_ci("conversation.log"), Some(virtual_dir.join("CONVERSATION.LOG")));
        assert_eq!(
            simulated.existing_file_with("jane_doe", Some((2026, 9, 1))),
            Some(virtual_dir.join("JANE_DOE-2026-09.TXT"))
        );
        assert_eq!(
            simulated.all_matching_files("Jane Doe"),
            vec![virtual_dir.join("jane doe-2026-10.Txt"), virtual_dir.join("Jane Doe.TXT")]
        );

        // Also check real on-disk directory entry lookup preserves stored casing end-to-end.
        let dir = temp_dir();
        let jane = Uuid::from_u128(0x5151);
        let session = Uuid::from_u128(0x6161);
        std::fs::write(
            dir.join("CONVERSATION.LOG"),
            format!("[1700000500] 0 0 0 Jane%20Doe| {jane} {session} jane.custom|\n"),
        )
        .unwrap();
        std::fs::write(dir.join("Jane_Custom-2026-09.TXT"), "sept\n").unwrap();
        std::fs::write(dir.join("JANE_DOE.TXT"), "user\n").unwrap();

        let target = shared_file_path(&dir, Some(jane), "Jane Doe", Some((2026, 9, 1)), NamingScheme::Legacy).unwrap();
        assert_eq!(target.file_name().and_then(|n| n.to_str()), Some("Jane_Custom-2026-09.TXT"));
        let all = all_candidate_files(&dir, Some(jane), "Jane Doe");
        let file_names: Vec<&str> = all.iter().filter_map(|p| p.file_name()?.to_str()).collect();
        assert_eq!(file_names, ["Jane_Custom-2026-09.TXT", "JANE_DOE.TXT"]);
    }
}
