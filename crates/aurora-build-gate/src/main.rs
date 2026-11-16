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
//! compiler runs without waiting, and when sccache is gone, without it.
//!
//! It prints nothing while it waits: Cargo keeps what a compiler writes and
//! shows it again at every later build of the crate.
//!
//! One file and the standard library only: the scripts build it with `rustc`
//! alone, before Cargo can run. Keep it that way.

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

    match run(commands(&rustc, &arguments, env::var_os("AURORA_GATE_NEXT").as_deref())) {
        // no code: ended by a signal (Unix)
        Ok(status) => process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("aurora-build-gate: could not run the compiler {rustc:?}: {error}");
            process::exit(1);
        }
    }
}

/// The commands to try, in order: through the next wrapper when the scripts
/// found one (sccache, `AURORA_GATE_NEXT`), then the compiler alone.
fn commands(rustc: &OsStr, arguments: &[OsString], next: Option<&OsStr>) -> Vec<Command> {
    let mut list = Vec::with_capacity(2);
    if let Some(next) = next.filter(|next| !next.is_empty()) {
        let mut command = Command::new(next);
        command.arg(rustc).args(arguments);
        list.push(command);
    }
    let mut command = Command::new(rustc);
    command.args(arguments);
    list.push(command);
    list
}

/// Runs the first command whose program exists and waits for it. A next
/// wrapper that is gone (sccache uninstalled or updated since the scripts
/// last ran) does not stop the build: the compiler runs without it.
fn run(mut commands: Vec<Command>) -> io::Result<ExitStatus> {
    let last = commands.len().saturating_sub(1);
    let mut result = Err(io::ErrorKind::NotFound.into());
    for (index, command) in commands.iter_mut().enumerate() {
        result = command.status();
        let gone = matches!(&result, Err(error) if error.kind() == io::ErrorKind::NotFound);
        if !gone || index == last {
            break;
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
        assert_eq!(expanded(&[missing.clone()]), [missing.to_string_lossy()]);
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
        let list = commands(OsStr::new("rustc"), &arguments(&["-vV"]), Some(OsStr::new("sccache")));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].get_program(), "sccache");
        assert_eq!(list[0].get_args().collect::<Vec<_>>(), ["rustc", "-vV"]);
        assert_eq!(list[1].get_program(), "rustc");
        assert_eq!(list[1].get_args().collect::<Vec<_>>(), ["-vV"]);
        // no next wrapper, or an empty setting: the compiler alone
        for next in [None, Some(OsStr::new(""))] {
            let list = commands(OsStr::new("rustc"), &arguments(&["-vV"]), next);
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].get_program(), "rustc");
        }
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
        assert!(Slots::open(&dir, 2).unwrap().try_take().unwrap().is_some());
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
