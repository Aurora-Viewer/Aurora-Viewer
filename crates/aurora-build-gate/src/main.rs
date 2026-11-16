//! Build gate: the compiler wrapper the Aurora scripts give Cargo
//! (`rustc-wrapper` of `<project folder>/.cargo/config.toml`, written by
//! `Sync-BuildCache` in `scripts/common.ps1`). A development tool, not a part
//! of the viewer.
//!
//! Several agents build on one PC, each in its own worktree. Compiling the
//! viewer crate takes 8 to 17 GB of memory: eight of them at once asked for
//! 87 GB on a 32 GB machine, and every build crawled in the page file. Cargo
//! runs every compiler through this program, whatever the command
//! (`cargo test`, `clippy`, `run`, `check.ps1`): a heavy crate waits here for
//! one of `AURORA_BUILD_SLOTS` slots (2 by default), the others go straight
//! through.
//!
//! A slot is a locked file of the `build-slots` folder next to this program:
//! the system frees it when this process ends, however it ends. The gate
//! never fails a build of its own: when the slots cannot be used, the
//! compiler runs without waiting, and when sccache cannot be started,
//! without it.
//!
//! sccache does not get a command line that Windows would refuse once
//! sccache has put its own arguments in it (`NEXT_COMMAND_LIMIT`): the
//! compiler then runs alone, and that compile is not cached.
//!
//! It prints nothing while it waits: Cargo keeps what a compiler writes and
//! shows it again at every later build of the crate.
//!
//! One file and the standard library only: the scripts build it with `rustc`
//! alone, before Cargo can run. Keep it that way.
//!
//! The project's Cargo configuration names the gate built by the last script
//! that wrote it. A fix that every worktree of the PC must get also changes
//! the marker of that file (`$BuildCacheMarker` in `scripts/common.ps1`), or
//! the scripts of an older worktree write their older gate back.

use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus};
use std::thread;
use std::time::Duration;

/// Crates whose compilation takes a slot, by their `--crate-name`.
const HEAVY_CRATES: &[&str] = &["aurora_viewer"];

/// Slots when `AURORA_BUILD_SLOTS` is not set, as `Enter-BuildSlot` of
/// `scripts/common.ps1`.
const DEFAULT_SLOTS: usize = 2;

/// More slots than this is no limit at all, and each one is an open file.
const MAX_SLOTS: usize = 64;

/// Time between two looks at the slots while they are all taken.
const POLL: Duration = Duration::from_millis(250);

/// The longest command line handed over to the next wrapper, in characters,
/// on the one system where it matters: Windows takes 32,767. sccache starts
/// the compiler a first time for the dependency information, with the same
/// arguments and a few of its own (`--emit dep-info -o <a file of the
/// temporary folder>`): they must still fit, and nothing tells how long the
/// user's temporary folder is. Cargo 1.100 names one `-L` folder per
/// dependency: a release build of `aurora-voice` is at 30,221 characters in
/// a work folder of 40, and past the limit in one of 56.
const NEXT_COMMAND_LIMIT: Option<usize> = if cfg!(windows) { Some(30_000) } else { None };

fn main() {
    // Cargo calls a wrapper as `<wrapper> <rustc> <arguments>`
    let mut arguments = env::args_os().skip(1);
    let Some(rustc) = arguments.next() else {
        eprintln!("aurora-build-gate: Cargo's compiler wrapper. Usage: aurora-build-gate <rustc> [arguments]");
        process::exit(2);
    };
    let arguments: Vec<OsString> = arguments.collect();

    // held until the compiler has ended
    let _slot = if is_heavy(&arguments) {
        let count = slot_count(env::var("AURORA_BUILD_SLOTS").ok().as_deref());
        slots_dir().and_then(|dir| take_slot(&dir, count))
    } else {
        None
    };

    let next = env::var_os("AURORA_GATE_NEXT");
    match run(commands(&rustc, &arguments, next.as_deref(), NEXT_COMMAND_LIMIT)) {
        // no code: ended by a signal (Unix)
        Ok(status) => process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("aurora-build-gate: could not run the compiler {rustc:?}: {error}");
            process::exit(1);
        }
    }
}

/// The commands to try, in order: through the next wrapper when the scripts
/// found one (sccache, `AURORA_GATE_NEXT`) and it can take the command line
/// (`next_takes`), then the compiler alone.
fn commands(rustc: &OsStr, arguments: &[OsString], next: Option<&OsStr>, limit: Option<usize>) -> Vec<Command> {
    let mut list = Vec::with_capacity(2);
    if let Some(next) = next.filter(|next| !next.is_empty() && next_takes(next, rustc, arguments, limit)) {
        let mut command = Command::new(next);
        command.arg(rustc).args(arguments);
        list.push(command);
    }
    let mut command = Command::new(rustc);
    command.args(arguments);
    list.push(command);
    list
}

/// Whether the next wrapper can be given this command line. Not when the
/// system would refuse it once the wrapper has added its own arguments
/// (`limit`, in characters), and not when Cargo already found it too long
/// and handed it over as an argument file (`@<file>`): sccache puts the
/// arguments of the file back on one command line for its first pass, and
/// the system refuses it (`sccache: error: failed to spawn … os error 206`,
/// a release build of a library crate; a crate built incrementally and a
/// program go through sccache untouched, as it does not cache them).
fn next_takes(next: &OsStr, rustc: &OsStr, arguments: &[OsString], limit: Option<usize>) -> bool {
    let Some(limit) = limit else {
        return true;
    };
    if arguments.iter().any(|argument| argument.as_encoded_bytes().starts_with(b"@")) {
        return false;
    }
    // each argument as the system gets it at most: its quotes, its space,
    // and a backslash before each quote it contains (`feature="std"`)
    let length = |text: &OsStr| {
        let bytes = text.as_encoded_bytes();
        bytes.len() + 3 + bytes.iter().filter(|byte| **byte == b'"').count()
    };
    length(next) + length(rustc) + arguments.iter().map(|argument| length(argument)).sum::<usize>() <= limit
}

/// Runs the first command that starts and waits for it. A next wrapper that
/// cannot be started does not stop the build, whatever the reason: the
/// compiler runs without it, and that one compile is not cached. sccache
/// may be gone (uninstalled or updated since the scripts last ran), or the
/// system may refuse the command line with the wrapper's path in front of
/// it (os error 206 on Windows; `NEXT_COMMAND_LIMIT` keeps well under it,
/// this is the net below). The compiler alone always fits: its command is
/// Cargo's, less this program's path.
fn run(mut commands: Vec<Command>) -> io::Result<ExitStatus> {
    let mut result = Err(io::ErrorKind::NotFound.into());
    for command in &mut commands {
        match command.spawn() {
            Ok(mut child) => return child.wait(),
            Err(error) => result = Err(error),
        }
    }
    result
}

/// Whether these compiler arguments build one of the heavy crates. Cargo
/// names the crate with `--crate-name <name>`; under clippy the real
/// compiler comes first, so the pair is looked for anywhere.
fn is_heavy(arguments: &[OsString]) -> bool {
    expanded(arguments)
        .windows(2)
        .any(|pair| pair[0] == "--crate-name" && HEAVY_CRATES.contains(&pair[1].as_str()))
}

/// The arguments, with those of an argument file in its place. When the
/// command line is too long for the system, Cargo hands it over as
/// `@<file>`, one argument per line (rustc's own syntax): on Windows, the
/// viewer's test build, the heaviest compile of all, always comes that way.
/// A file that cannot be read stays as it is: the compiler will say why.
fn expanded(arguments: &[OsString]) -> Vec<String> {
    let mut list = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let text = argument.to_string_lossy();
        match text.strip_prefix('@').and_then(|file| fs::read(file).ok()) {
            Some(content) => list.extend(String::from_utf8_lossy(&content).lines().map(str::to_owned)),
            None => list.push(text.into_owned()),
        }
    }
    list
}

/// Number of slots: `AURORA_BUILD_SLOTS` when it is a whole number of at
/// least 1, the default otherwise.
fn slot_count(setting: Option<&str>) -> usize {
    setting
        .and_then(|text| text.trim().parse::<usize>().ok())
        .filter(|count| *count >= 1)
        .map_or(DEFAULT_SLOTS, |count| count.min(MAX_SLOTS))
}

/// The folder of the slot files, next to this program: every build of the
/// PC goes through the same one.
fn slots_dir() -> Option<PathBuf> {
    Some(env::current_exe().ok()?.parent()?.join("build-slots"))
}

/// The slot files of a folder, opened and not locked yet.
struct Slots {
    files: Vec<File>,
}

impl Slots {
    fn open(dir: &Path, count: usize) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let files = (1..=count)
            .map(|number| {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(dir.join(format!("slot-{number}.lock")))
            })
            .collect::<io::Result<_>>()?;
        Ok(Self { files })
    }

    /// Locks a free slot, when there is one. The file holds the slot until
    /// it is dropped.
    fn try_take(&mut self) -> io::Result<Option<File>> {
        for index in 0..self.files.len() {
            match self.files[index].try_lock() {
                Ok(()) => return Ok(Some(self.files.swap_remove(index))),
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => return Err(error),
            }
        }
        Ok(None)
    }
}

/// Waits for a free slot of `dir`. `None` when the slots cannot be used
/// (folder or lock refused): the build then runs without waiting.
fn take_slot(dir: &Path, count: usize) -> Option<File> {
    let mut slots = Slots::open(dir, count).ok()?;
    loop {
        match slots.try_take() {
            Ok(Some(slot)) => return Some(slot),
            Ok(None) => thread::sleep(POLL),
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::Instant;

    fn arguments(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    /// An empty folder of its own for one test.
    fn scratch(test: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("aurora-build-gate-{}-{test}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// A slot that has just been freed, tried for a few seconds. On Unix a
    /// lock outlives its file for an instant when another test starts a
    /// process at that moment: until it runs its program, the new process
    /// holds a copy of every open file of this one, the lock with it. A
    /// build never sees it: it waits for its slot (`take_slot`).
    fn take_soon(dir: &Path, count: usize) -> Option<File> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(slot) = Slots::open(dir, count).unwrap().try_take().unwrap() {
                return Some(slot);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn the_viewer_crate_is_heavy_whatever_the_command() {
        assert!(is_heavy(&arguments(&["--crate-name", "aurora_viewer", "--edition=2024"])));
        assert!(is_heavy(&arguments(&["--crate-name", "aurora_viewer", "--test"])));
        // clippy: `<wrapper> clippy-driver <rustc> <arguments>`
        assert!(is_heavy(&arguments(&["C:\\toolchain\\rustc.exe", "--crate-name", "aurora_viewer"])));
    }

    #[test]
    fn other_crates_and_compiler_queries_go_straight_through() {
        assert!(!is_heavy(&arguments(&["--crate-name", "aurora_net"])));
        assert!(!is_heavy(&arguments(&["-vV"])));
        assert!(!is_heavy(&arguments(&["-", "--crate-name", "___", "--print=file-names"])));
        assert!(!is_heavy(&arguments(&["--crate-name"])));
        assert!(!is_heavy(&arguments(&[])));
        // the heavy name, but not as the crate being built
        assert!(!is_heavy(&arguments(&["--crate-name", "smoke", "--extern", "aurora_viewer"])));
    }

    #[test]
    fn a_crate_named_in_an_argument_file_is_seen() {
        let dir = scratch("argfile");
        fs::create_dir_all(&dir).unwrap();
        let file = |name: &str, content: &str| {
            let path = dir.join(name);
            fs::write(&path, content).unwrap();
            OsString::from(format!("@{}", path.display()))
        };
        // as Cargo writes it: one argument per line
        let viewer = file("viewer", "--crate-name\naurora_viewer\n--edition=2024\n--test\n");
        assert!(is_heavy(&[OsString::from("C:\\toolchain\\rustc.exe"), viewer]));
        let windows_lines = file("crlf", "--crate-name\r\naurora_viewer\r\n--test\r\n");
        assert!(is_heavy(&[windows_lines]));
        let other = file("other", "--crate-name\naurora_net\n--extern\naurora_viewer\n");
        assert!(!is_heavy(&[other]));
        // a file that is not there: not heavy, and the argument is kept
        let missing = OsString::from(format!("@{}", dir.join("missing").display()));
        assert!(!is_heavy(std::slice::from_ref(&missing)));
        assert_eq!(expanded(std::slice::from_ref(&missing)), [missing.to_string_lossy()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn slot_count_follows_the_setting() {
        assert_eq!(slot_count(None), DEFAULT_SLOTS);
        assert_eq!(slot_count(Some("1")), 1);
        assert_eq!(slot_count(Some(" 4 ")), 4);
        assert_eq!(slot_count(Some("0")), DEFAULT_SLOTS);
        assert_eq!(slot_count(Some("-1")), DEFAULT_SLOTS);
        assert_eq!(slot_count(Some("two")), DEFAULT_SLOTS);
        assert_eq!(slot_count(Some("")), DEFAULT_SLOTS);
        assert_eq!(slot_count(Some("100000")), MAX_SLOTS);
    }

    #[test]
    fn the_next_wrapper_comes_first_then_the_compiler_alone() {
        for limit in [None, Some(30_000)] {
            let list = commands(OsStr::new("rustc"), &arguments(&["-vV"]), Some(OsStr::new("sccache")), limit);
            assert_eq!(list.len(), 2);
            assert_eq!(list[0].get_program(), "sccache");
            assert_eq!(list[0].get_args().collect::<Vec<_>>(), ["rustc", "-vV"]);
            assert_eq!(list[1].get_program(), "rustc");
            assert_eq!(list[1].get_args().collect::<Vec<_>>(), ["-vV"]);
        }
        // no next wrapper, or an empty setting: the compiler alone
        for next in [None, Some(OsStr::new(""))] {
            let list = commands(OsStr::new("rustc"), &arguments(&["-vV"]), next, None);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].get_program(), "rustc");
        }
    }

    /// The programs of `commands` for a compile through sccache.
    fn programs(arguments: &[OsString], limit: Option<usize>) -> Vec<String> {
        commands(OsStr::new("rustc"), arguments, Some(OsStr::new("sccache")), limit)
            .iter()
            .map(|command| command.get_program().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_command_line_near_the_limit_goes_to_the_compiler_alone() {
        // 300 folders of 100 characters, as Cargo 1.100 names them: 31,500
        // characters with their quotes, spaces and `-L`
        let folder = format!("dependency={}", "x".repeat(89));
        let mut long = arguments(&["--crate-name", "aurora_voice"]);
        for _ in 0..300 {
            long.extend([OsString::from("-L"), OsString::from(&folder)]);
        }
        assert_eq!(programs(&long, Some(30_000)), ["rustc"]);
        // the same with room to spare, and on a system without such a limit
        assert_eq!(programs(&long, Some(40_000)), ["sccache", "rustc"]);
        assert_eq!(programs(&long, None), ["sccache", "rustc"]);
        // the length counts the wrapper's own path and the compiler's, each
        // with its quotes and its space: 10 + 8 + 6 for `sccache rustc -vV`
        let short = arguments(&["-vV"]);
        assert_eq!(programs(&short, Some(24)), ["sccache", "rustc"]);
        assert_eq!(programs(&short, Some(23)), ["rustc"]);
        // a quote inside an argument takes a backslash on the command line:
        // 10 + 8 + 8 + 18 for `sccache rustc --cfg "feature=\"std\""`
        let quoted = arguments(&["--cfg", "feature=\"std\""]);
        assert_eq!(programs(&quoted, Some(44)), ["sccache", "rustc"]);
        assert_eq!(programs(&quoted, Some(43)), ["rustc"]);
        // the compiler alone gets every argument
        let alone = commands(OsStr::new("rustc"), &long, Some(OsStr::new("sccache")), Some(30_000));
        assert_eq!(alone[0].get_args().count(), long.len());
    }

    #[test]
    fn an_argument_file_goes_to_the_compiler_alone() {
        // Cargo found the command line too long for the system: sccache
        // would put it back on one line
        let file = arguments(&["@C:\\Temp\\cargo-argfile.abc123"]);
        assert_eq!(programs(&file, Some(30_000)), ["rustc"]);
        // under clippy the real compiler comes first
        let clippy = arguments(&["C:\\toolchain\\rustc.exe", "@C:\\Temp\\cargo-argfile.abc123"]);
        assert_eq!(programs(&clippy, Some(30_000)), ["rustc"]);
        // where no command line is too long, Cargo writes no such file
        assert_eq!(programs(&file, None), ["sccache", "rustc"]);
        // an `@` inside an argument is not a file
        assert_eq!(programs(&arguments(&["--cfg", "a@b"]), Some(30_000)), ["sccache", "rustc"]);
    }

    #[test]
    fn a_next_wrapper_that_is_gone_does_not_stop_the_build() {
        let gone = scratch("gone").join("no-such-program");
        // stands for the compiler: this test program, which only lists its tests
        let mut compiler = Command::new(env::current_exe().unwrap());
        compiler.arg("--list").stdout(Stdio::null());
        assert!(run(vec![Command::new(&gone), compiler]).unwrap().success());
        // the compiler itself missing is an error
        assert_eq!(run(vec![Command::new(&gone)]).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn a_next_wrapper_the_system_refuses_does_not_stop_the_build() {
        // a command line past what every system takes (32,767 characters on
        // Windows, 128 KB for one argument on Linux, 1 MB in all on macOS):
        // the program exists, and still cannot be started
        let too_long = || {
            let mut wrapper = Command::new(env::current_exe().unwrap());
            wrapper.arg("x".repeat(4 << 20)).stdout(Stdio::null());
            wrapper
        };
        let refused = too_long().status().unwrap_err();
        assert_ne!(refused.kind(), io::ErrorKind::NotFound);
        // stands for the compiler, whose own command line fits
        let mut compiler = Command::new(env::current_exe().unwrap());
        compiler.arg("--list").stdout(Stdio::null());
        assert!(run(vec![too_long(), compiler]).unwrap().success());
        // nothing after it to run: the system's refusal is the error
        assert_eq!(run(vec![too_long()]).unwrap_err().kind(), refused.kind());
    }

    #[test]
    fn a_taken_slot_is_not_given_twice() {
        let dir = scratch("twice");
        let first = Slots::open(&dir, 2).unwrap().try_take().unwrap();
        let second = Slots::open(&dir, 2).unwrap().try_take().unwrap();
        assert!(first.is_some() && second.is_some());
        // both taken: a third build finds none, whatever its own count
        assert!(Slots::open(&dir, 2).unwrap().try_take().unwrap().is_none());
        // a build that asks for more slots sees the next one
        let third = Slots::open(&dir, 3).unwrap().try_take().unwrap();
        assert!(third.is_some());
        drop(first);
        assert!(take_soon(&dir, 2).is_some());
        drop((second, third));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_waiting_build_gets_the_slot_once_it_is_free() {
        let dir = scratch("wait");
        let held = take_slot(&dir, 1).unwrap();
        let hold = Duration::from_millis(600);
        let holder = thread::spawn(move || {
            thread::sleep(hold);
            drop(held);
        });
        let started = Instant::now();
        let slot = take_slot(&dir, 1);
        assert!(slot.is_some());
        // it waited for the holder: not given at once
        assert!(started.elapsed() >= hold - POLL);
        holder.join().unwrap();
        drop(slot);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unusable_slots_do_not_hold_the_build() {
        // a file where the folder should be: the slots cannot be created
        let dir = scratch("unusable");
        fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("not-a-folder");
        fs::write(&blocker, b"").unwrap();
        assert!(take_slot(&blocker.join("build-slots"), 2).is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
