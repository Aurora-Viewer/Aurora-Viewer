//! `fuzz-local`: a mutation fuzzer on stable Rust, for Windows where
//! libFuzzer is not at hand. No coverage feedback: it mutates the targets'
//! seeds (and `corpus/<target>/`, where the libFuzzer runs keep theirs) and
//! reports what libFuzzer reports: a panic, an input that runs for more
//! than `--timeout` seconds, an allocation above `--max-alloc-mb`. Each
//! finding is written to `artifacts/<target>/` with the panic message.
//!
//! cargo run --release --manifest-path fuzz/Cargo.toml --bin fuzz-local -- <target|all> [--seconds N] [--seed S]
//! cargo run --release --manifest-path fuzz/Cargo.toml --bin fuzz-local -- --replay <target> <file>

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Refuses a single allocation above the limit: a reader must bound what it
/// allocates by the size of its input, whatever the input claims.
struct Limit;
static MAX_ALLOC: AtomicUsize = AtomicUsize::new(usize::MAX);
static OVERSIZED: AtomicUsize = AtomicUsize::new(0);

/// The input being run and its target: an infallible allocation refused
/// ends the process (`handle_alloc_error`), so the input is saved first.
static CURRENT: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static CURRENT_TARGET: Mutex<&str> = Mutex::new("");

fn refuse(size: usize) -> *mut u8 {
    // Only the first refusal of a run is saved; the small allocations of
    // the save itself come back here below the limit.
    if OVERSIZED.swap(size, Ordering::Relaxed) == 0
        && let (Ok(data), Ok(target)) = (CURRENT.try_lock(), CURRENT_TARGET.try_lock())
    {
        let path = save(&target, "oom", &data, &format!("an allocation of {size} bytes was refused"));
        eprintln!("{}: allocation of {size} bytes refused, input saved to {}", *target, path.display());
    }
    std::ptr::null_mut()
}

// SAFETY: forwards to the system allocator; a refused allocation returns
// null, which GlobalAlloc allows.
unsafe impl GlobalAlloc for Limit {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > MAX_ALLOC.load(Ordering::Relaxed) {
            return refuse(layout.size());
        }
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr came from System.alloc with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > MAX_ALLOC.load(Ordering::Relaxed) {
            return refuse(new_size);
        }
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Limit = Limit;

/// xorshift64*: reproducible from `--seed`.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const INTERESTING: &[u64] = &[
    0,
    1,
    2,
    0x7F,
    0x80,
    0xFF,
    0x100,
    0x7FFF,
    0x8000,
    0xFFFF,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    u64::MAX,
];

fn mutate(rng: &mut Rng, input: &mut Vec<u8>, corpus: &[Vec<u8>]) {
    for _ in 0..1 + rng.below(6) {
        let len = input.len();
        match rng.below(10) {
            0 if len > 0 => {
                let i = rng.below(len);
                input[i] ^= 1 << rng.below(8);
            }
            1 if len > 0 => {
                let i = rng.below(len);
                input[i] = rng.next() as u8;
            }
            2 => {
                let i = rng.below(len + 1);
                let n = 1 + rng.below(16);
                let bytes: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                input.splice(i..i, bytes);
            }
            3 if len > 0 => {
                let i = rng.below(len);
                let n = 1 + rng.below((len - i).min(64));
                input.drain(i..i + n);
            }
            4 if len > 0 => {
                // an integer field set to an edge value, in either byte order
                let width = [1usize, 2, 4, 8][rng.below(4)].min(len);
                let i = rng.below(len - width + 1);
                let v = INTERESTING[rng.below(INTERESTING.len())]
                    .wrapping_add(rng.below(3) as u64)
                    .wrapping_sub(1);
                let bytes = if rng.below(2) == 0 { v.to_le_bytes() } else { v.to_be_bytes() };
                let start = if rng.below(2) == 0 { 0 } else { 8 - width };
                input[i..i + width].copy_from_slice(&bytes[start..start + width]);
            }
            5 if len > 1 => {
                // a range copied elsewhere (repeated structures)
                let from = rng.below(len);
                let n = 1 + rng.below((len - from).min(128));
                let chunk = input[from..from + n].to_vec();
                let to = rng.below(len + 1);
                input.splice(to..to, chunk);
            }
            6 if !corpus.is_empty() => {
                // spliced with another input
                let other = &corpus[rng.below(corpus.len())];
                let cut = rng.below(len + 1);
                let from = rng.below(other.len() + 1);
                input.truncate(cut);
                input.extend_from_slice(&other[from..]);
            }
            7 if len > 0 => input.truncate(rng.below(len)),
            8 if len > 0 => {
                let i = rng.below(len);
                let n = 1 + rng.below((len - i).min(32));
                let b = rng.next() as u8;
                input[i..i + n].fill(b);
            }
            _ => {
                if len > 0 {
                    let i = rng.below(len);
                    input[i] = input[i].wrapping_add(rng.below(5) as u8).wrapping_sub(2);
                }
            }
        }
    }
    input.truncate(1 << 20);
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_dir(dir: &Path) -> Vec<Vec<u8>> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.flatten().filter_map(|e| std::fs::read(e.path()).ok()).collect()
}

fn hash(data: &[u8]) -> u64 {
    data.iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn save(target: &str, kind: &str, data: &[u8], note: &str) -> PathBuf {
    let dir = root().join("artifacts").join(target);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("{kind}-{:016x}", hash(data)));
    let _ = std::fs::write(&path, data);
    let _ = std::fs::write(path.with_extension("txt"), note);
    path
}

fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "panic".into())
}

/// The panic's location, kept by the hook: findings are grouped by it.
static LOCATION: Mutex<String> = Mutex::new(String::new());

fn fuzz(t: &'static aurora_fuzz::Target, seconds: u64, seed: u64, timeout: u64) -> usize {
    let mut corpus = (t.seeds)();
    corpus.extend(read_dir(&root().join("corpus").join(t.name)));
    corpus.retain(|c| !c.is_empty());
    if corpus.is_empty() {
        corpus.push(vec![0]);
    }
    if let Ok(mut name) = CURRENT_TARGET.lock() {
        *name = t.name;
    }
    let busy_since = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    // A hang cannot be interrupted: the watchdog saves the input and ends
    // the process, as libFuzzer does.
    let watchdog = {
        let (busy_since, stop) = (busy_since.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let since = busy_since.load(Ordering::Relaxed);
                if since != 0 && start.elapsed().as_millis() as u64 - since > timeout * 1000 {
                    let data = CURRENT.lock().map(|d| d.clone()).unwrap_or_default();
                    let path = save(t.name, "timeout", &data, &format!("ran for more than {timeout} s"));
                    eprintln!("{}: TIMEOUT, input saved to {}", t.name, path.display());
                    std::process::exit(2);
                }
            }
        })
    };
    let mut rng = Rng(seed | 1);
    let mut seen = std::collections::HashSet::new();
    let (mut runs, mut found) = (0u64, 0usize);
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        let mut input = corpus[rng.below(corpus.len())].clone();
        mutate(&mut rng, &mut input, &corpus);
        if let Ok(mut c) = CURRENT.lock() {
            c.clone_from(&input);
        }
        OVERSIZED.store(0, Ordering::Relaxed);
        busy_since.store(start.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
        let result = std::panic::catch_unwind(|| (t.run)(&input));
        busy_since.store(0, Ordering::Relaxed);
        runs += 1;
        let oversized = OVERSIZED.load(Ordering::Relaxed);
        let failure = match result {
            Err(e) => Some((
                "crash",
                format!("{} at {}", panic_text(&*e), LOCATION.lock().map(|l| l.clone()).unwrap_or_default()),
            )),
            Ok(()) if oversized > 0 => Some(("oom", format!("an allocation of {oversized} bytes was refused"))),
            Ok(()) => None,
        };
        if let Some((kind, note)) = failure {
            let key = format!("{kind}:{}", note.rsplit(" at ").next().unwrap_or(&note));
            if seen.insert(key) {
                // (an "oom" input is already saved by `refuse`; saving again
                // writes the same file)
                let path = save(t.name, kind, &input, &note);
                eprintln!("{}: {kind}: {note}\n  input: {}", t.name, path.display());
                found += 1;
            }
        }
        // (no survivor joins the corpus: without coverage feedback they
        // drift away from the formats, and most runs end at the first check)
    }
    stop.store(true, Ordering::Relaxed);
    let _ = watchdog.join();
    println!("{}: {runs} runs in {seconds} s, {found} distinct finding(s)", t.name);
    found
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str, default: u64| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    MAX_ALLOC.store((value("--max-alloc-mb", 1024) as usize) << 20, Ordering::Relaxed);
    std::panic::set_hook(Box::new(|info| {
        if let (Ok(mut l), Some(loc)) = (LOCATION.lock(), info.location()) {
            *l = format!("{}:{}", loc.file(), loc.line());
        }
    }));
    // The seeds as files, for libFuzzer's corpus folders (the Fuzz workflow).
    if args.first().map(String::as_str) == Some("--write-seeds") {
        for t in aurora_fuzz::TARGETS {
            let dir = root().join("corpus").join(t.name);
            let _ = std::fs::create_dir_all(&dir);
            for (i, seed) in (t.seeds)().iter().enumerate() {
                if let Err(e) = std::fs::write(dir.join(format!("seed-{i}")), seed) {
                    eprintln!("{}: {e}", dir.display());
                    std::process::exit(73);
                }
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("--replay") {
        let (Some(name), Some(file)) = (args.get(1), args.get(2)) else {
            eprintln!("usage: fuzz-local --replay <target> <file>");
            std::process::exit(64);
        };
        let Some(t) = aurora_fuzz::target(name) else {
            eprintln!("unknown target {name}");
            std::process::exit(64);
        };
        let data = std::fs::read(file).unwrap_or_else(|e| {
            eprintln!("{file}: {e}");
            std::process::exit(66)
        });
        let _ = std::panic::take_hook();
        (t.run)(&data);
        println!("{name}: no failure on {file}");
        return;
    }
    let Some(which) = args.first().filter(|a| !a.starts_with("--")) else {
        let names: Vec<_> = aurora_fuzz::TARGETS.iter().map(|t| t.name).collect();
        eprintln!(
            "usage: fuzz-local <target|all> [--seconds N] [--seed S] [--timeout S] [--max-alloc-mb N]\ntargets: {}",
            names.join(", ")
        );
        std::process::exit(64);
    };
    let seconds = value("--seconds", 60);
    let seed = value("--seed", 0x5eed_2026);
    let timeout = value("--timeout", 10);
    let targets: Vec<_> = if which == "all" {
        aurora_fuzz::TARGETS.iter().collect()
    } else {
        match aurora_fuzz::target(which) {
            Some(t) => vec![t],
            None => {
                eprintln!("unknown target {which}");
                std::process::exit(64);
            }
        }
    };
    let found: usize = targets.into_iter().map(|t| fuzz(t, seconds, seed, timeout)).sum();
    if found > 0 {
        std::process::exit(1);
    }
}
