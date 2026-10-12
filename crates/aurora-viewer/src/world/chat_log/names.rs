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
                .is_some_and(|suffix| {
                    suffix.len() == pattern_len
                        && suffix
                            .bytes()
                            .enumerate()
                            .all(|(i, b)| if matches!(i, 0 | 5 | 8) { b == b'-' } else { b.is_ascii_digit() })
                })
        })
        .collect();
    dated.sort();
    let found = match dated.pop() {
        Some(latest) => dir.join(latest),
        None => dir.join(format!("{clean}.{EXTENSION}")),
    };
    found.is_file().then_some(found)
}

/// Today's local date (LogFileNamewithDate uses localtime); the Second Life
/// date on the platforms whose local clock is not read.
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
    let t = super::format::Slt::of(SystemTime::now());
    (t.year, t.month, t.day)
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
}
