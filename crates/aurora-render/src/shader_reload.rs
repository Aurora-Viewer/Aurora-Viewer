//! Development builds only (`dev`, `ci`, `debugging`): the renderer's WGSL
//! read again from the source checkout while the viewer runs, so a shader
//! change is seen without a restart and a new login (Dev Toolbox T2,
//! `AURORA_SHADER_RELOAD`, docs/TESTING.md).
//!
//! A request ([`request`]) is taken by every backend at its next frame
//! (`Backend::poll_shader_reload`): on its pipeline worker it reads the
//! files once ([`load`]), has naga check every module made from them
//! ([`validate`]), then compiles the scene's pipelines for the samples, SSR
//! and soft shadows it draws with, the impostors' and the water's, and
//! swaps them in at a later frame. A file that does not validate changes
//! nothing: the error is logged with naga's report, the old pipelines stay,
//! and [`take_outcome`] gives the interface a short notice.
//!
//! A published build never reads the checkout this way: the viewer tells
//! this module its profile at start ([`set_development_build`]); without
//! it, [`request`] does nothing and `AURORA_SHADER_RELOAD` is not read.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The files a reload reads, with the copy baked into the build: the scene
/// (common + spectrum, object, env) and the water (simulation, caustics,
/// swept wake). The other shaders keep their baked copy.
pub const FILES: &[(&str, &str)] = &[
    ("common.wgsl", include_str!("shaders/common.wgsl")),
    ("water_spectrum.wgsl", include_str!("shaders/water_spectrum.wgsl")),
    ("object.wgsl", include_str!("shaders/object.wgsl")),
    ("env.wgsl", include_str!("shaders/env.wgsl")),
    ("water_sim.wgsl", include_str!("shaders/water_sim.wgsl")),
    ("water_caustics.wgsl", include_str!("shaders/water_caustics.wgsl")),
    ("water_wake.wgsl", include_str!("shaders/water_wake.wgsl")),
];

static DEVELOPMENT: AtomicBool = AtomicBool::new(false);
/// The reloadable shaders come from the checkout, not from the build:
/// `AURORA_SHADER_RELOAD` at start, or once a reload validated (a refused
/// one leaves the baked copy, which nothing checked would replace).
static FROM_CHECKOUT: AtomicBool = AtomicBool::new(false);
/// `AURORA_SHADER_RELOAD=watch`: a saved file asks for a reload.
static WATCH: AtomicBool = AtomicBool::new(false);
/// Reloads asked for so far; each backend keeps the last one it took.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The checkout's text of the last reload that validated: what any module
/// made later reads (a settings change, the stochastic transparency turned
/// on), so that it draws like the rest.
static VALIDATED: parking_lot::Mutex<Option<HashMap<&'static str, Arc<str>>>> = parking_lot::Mutex::new(None);
/// What the interface has not shown yet.
static OUTCOME: parking_lot::Mutex<Option<Outcome>> = parking_lot::Mutex::new(None);

thread_local! {
    /// The files a reload checks and compiles on this worker thread.
    static CANDIDATE: std::cell::RefCell<Option<HashMap<&'static str, Arc<str>>>> = const { std::cell::RefCell::new(None) };
}

/// The folder of the shaders in the checkout this viewer was built from.
fn folder() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders")
}

/// What `AURORA_SHADER_RELOAD` asks for: (read the checkout from the start,
/// watch the files).
fn mode_from(value: Option<&str>) -> (bool, bool) {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("1" | "on" | "true" | "yes") => (true, false),
        Some("watch") => (true, true),
        _ => (false, false),
    }
}

/// Tells the reload the viewer's build profile, at start: only a
/// development build reloads, and reads `AURORA_SHADER_RELOAD`.
pub fn set_development_build(on: bool) {
    DEVELOPMENT.store(on, Ordering::Relaxed);
    if !on {
        return;
    }
    let (checkout, watch) = mode_from(std::env::var("AURORA_SHADER_RELOAD").ok().as_deref());
    if checkout {
        FROM_CHECKOUT.store(true, Ordering::Relaxed);
        WATCH.store(watch, Ordering::Relaxed);
        log::info!(
            "shader reload: WGSL from {}{}",
            folder().display(),
            if watch { ", reloaded when a file is saved" } else { "" }
        );
    }
}

/// Whether this build can reload its shaders (the Dev Toolbox lists T2).
pub fn available() -> bool {
    DEVELOPMENT.load(Ordering::Relaxed)
}

/// Asks every renderer to read its shaders from the checkout again. False
/// (nothing asked) in a published build.
pub fn request() -> bool {
    if !available() {
        return false;
    }
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    log::info!("shader reload {generation}: asked for, from {}", folder().display());
    true
}

pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Whether a module made on this thread now reads the reloadable files
/// from the checkout: a reload's worker, or once one validated.
pub(crate) fn active() -> bool {
    FROM_CHECKOUT.load(Ordering::Relaxed) || CANDIDATE.with(|c| c.borrow().is_some())
}

/// Whether a reload reads `file` ([`FILES`]): the others keep their baked
/// copy (outside the demo's `AURORA_DEMO_WATER_SHADERS`).
pub(crate) fn reloadable(file: &str) -> bool {
    FILES.iter().any(|(name, _)| *name == file)
}

/// A file's text as the next module made on this thread takes it: a
/// reload's own (on its worker), else the last that validated, else the
/// checkout's as it is now.
pub(crate) fn read(file: &str) -> std::io::Result<Arc<str>> {
    if let Some(text) = CANDIDATE.with(|c| c.borrow().as_ref().and_then(|m| m.get(file).cloned())) {
        return Ok(text);
    }
    if let Some(text) = VALIDATED.lock().as_ref().and_then(|m| m.get(file).cloned()) {
        return Ok(text);
    }
    std::fs::read_to_string(folder().join(file)).map(Arc::from)
}

/// The files of one reload, read once: what it checks is what it compiles,
/// even when a file is saved again meanwhile.
pub(crate) struct Candidate {
    texts: HashMap<&'static str, Arc<str>>,
    /// The files that differ from what the renderer drew with before.
    pub changed: Vec<&'static str>,
}

/// Why a reload changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// `object.wgsl:812` when the error's line was found in one file.
    pub place: Option<String>,
    /// naga's (or wgpu's) message, on one line.
    pub message: String,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.place {
            Some(place) => write!(f, "{place}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Reads every file of [`FILES`] from the checkout.
pub(crate) fn load() -> Result<Candidate, Failure> {
    let before = VALIDATED.lock().clone();
    let mut texts = HashMap::new();
    let mut changed = Vec::new();
    for &(file, baked) in FILES {
        let text: Arc<str> = std::fs::read_to_string(folder().join(file))
            .map_err(|e| Failure {
                place: Some(file.to_string()),
                message: e.to_string(),
            })?
            .into();
        let old = before.as_ref().and_then(|m| m.get(file).cloned());
        if old.as_deref().unwrap_or(baked) != &*text {
            changed.push(file);
        }
        texts.insert(file, text);
    }
    Ok(Candidate { texts, changed })
}

/// Runs `f` with this thread's modules made from `candidate`.
pub(crate) fn with_candidate<T>(candidate: &Candidate, f: impl FnOnce() -> T) -> T {
    let old = CANDIDATE.with(|c| c.replace(Some(candidate.texts.clone())));
    let out = f();
    CANDIDATE.with(|c| *c.borrow_mut() = old);
    out
}

/// The candidate's files become the ones every later module reads.
pub(crate) fn promote(candidate: &Candidate) {
    *VALIDATED.lock() = Some(candidate.texts.clone());
    FROM_CHECKOUT.store(true, Ordering::Relaxed);
}

/// Has naga parse and validate a module made from the candidate (`label`
/// for the log). The whole report goes to the log; the [`Failure`] keeps
/// its first line and the file and line it points at.
pub(crate) fn validate(label: &str, source: &str, candidate: &Candidate) -> Result<(), Failure> {
    let (message, location, report) = match naga::front::wgsl::parse_str(source) {
        Err(e) => (e.message().to_string(), e.location(source), e.emit_to_string(source)),
        Ok(module) => {
            let flags = naga::valid::ValidationFlags::all();
            match naga::valid::Validator::new(flags, naga::valid::Capabilities::all()).validate(&module) {
                Ok(_) => return Ok(()),
                Err(e) => (chain(&e), e.location(source), e.emit_to_string(source)),
            }
        }
    };
    log::error!("shader reload: the {label} module does not validate:\n{report}");
    let place = location.and_then(|l| place_of(source, l.line_number as usize, &candidate.texts));
    Err(Failure { place, message })
}

/// A refusal of the device (wgpu's report: a title, "Caused by:", then a
/// cause a line, deeper and deeper) as a [`Failure`] on one line: the call
/// and its label, then the two deepest causes.
pub(crate) fn device_failure(report: &str) -> Failure {
    let lines: Vec<&str> = report
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "Caused by:")
        .collect();
    let call = lines.iter().find(|l| l.starts_with("In ")).map(|l| l.trim_start_matches("In "));
    let deepest = lines.iter().rev().take(2).rev().copied().collect::<Vec<_>>().join(": ");
    let message = match call {
        Some(call) if !deepest.contains(call) => format!("{call}: {deepest}"),
        _ => deepest,
    };
    Failure { place: None, message }
}

/// An error and its causes, on one line.
fn chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        text.push_str(": ");
        text.push_str(&c.to_string());
        cause = c.source();
    }
    text
}

/// Where line `line` (1-based) of an assembled module comes from: the
/// module is several files put together and rewritten (attributes taken
/// out, passes left out), so the line is looked for by its text, in the
/// one file that has it.
fn place_of(source: &str, line: usize, files: &HashMap<&'static str, Arc<str>>) -> Option<String> {
    let wanted = source.lines().nth(line.checked_sub(1)?)?.trim();
    if wanted.is_empty() {
        return None;
    }
    let mut found = None;
    for &(file, _) in FILES {
        let Some(text) = files.get(file) else { continue };
        for (n, l) in text.lines().enumerate() {
            if l.trim() == wanted {
                if found.is_some() {
                    // (the same line in two places: no guess)
                    return None;
                }
                found = Some(format!("{file}:{}", n + 1));
            }
        }
    }
    found
}

/// What a reload did, for the interface's notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The pipelines were swapped: the files that changed, and how long
    /// the compile took.
    Reloaded { changed: Vec<String>, ms: u64 },
    /// Nothing changed: the old pipelines stay.
    Failed(Failure),
}

/// A backend's reload ended. The first word of a request is kept (the VR
/// eyes and a second view reload too, from the same files).
pub(crate) fn report(generation: u64, outcome: Outcome) {
    static REPORTED: AtomicU64 = AtomicU64::new(0);
    if REPORTED.fetch_max(generation, Ordering::Relaxed) >= generation {
        return;
    }
    *OUTCOME.lock() = Some(outcome);
}

/// The last reload's outcome, once.
pub fn take_outcome() -> Option<Outcome> {
    OUTCOME.lock().take()
}

/// `AURORA_SHADER_RELOAD=watch`: asks for a reload when a file of
/// [`FILES`] was saved since the last look (twice a second at most, from
/// any backend's frame).
pub(crate) fn watch_tick() {
    if !WATCH.load(Ordering::Relaxed) {
        return;
    }
    static LAST: parking_lot::Mutex<Option<(std::time::Instant, Vec<Option<std::time::SystemTime>>)>> = parking_lot::Mutex::new(None);
    let mut last = LAST.lock();
    if last
        .as_ref()
        .is_some_and(|(at, _)| at.elapsed() < std::time::Duration::from_millis(500))
    {
        return;
    }
    let dir = folder();
    let times: Vec<_> = FILES
        .iter()
        .map(|(file, _)| std::fs::metadata(dir.join(file)).and_then(|m| m.modified()).ok())
        .collect();
    let saved = last.as_ref().is_some_and(|(_, before)| *before != times);
    *last = Some((std::time::Instant::now(), times));
    drop(last);
    if saved {
        request();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_reads_the_checkout_or_watches_it() {
        assert_eq!(mode_from(None), (false, false));
        assert_eq!(mode_from(Some("0")), (false, false));
        assert_eq!(mode_from(Some("1")), (true, false));
        assert_eq!(mode_from(Some(" Watch ")), (true, true));
    }

    #[test]
    fn the_baked_files_are_the_checkout_s() {
        // the build bakes the checkout: a reload of an untouched tree changes nothing
        let candidate = load().expect("the shaders are in the checkout");
        assert!(candidate.changed.is_empty(), "{:?}", candidate.changed);
    }

    #[test]
    fn an_error_is_placed_in_its_file() {
        let mut texts = HashMap::new();
        texts.insert("object.wgsl", Arc::from("fn a() {}\nfn broken( {\n"));
        texts.insert("env.wgsl", Arc::from("fn b() {}\n"));
        let candidate = Candidate {
            texts,
            changed: Vec::new(),
        };
        let source = "fn b() {}\nfn a() {}\nfn broken( {\n";
        let failure = validate("test", source, &candidate).expect_err("a broken module");
        assert_eq!(failure.place.as_deref(), Some("object.wgsl:2"));
        assert!(!failure.message.is_empty());
    }

    #[test]
    fn a_line_found_twice_is_not_placed() {
        let mut texts = HashMap::new();
        texts.insert("object.wgsl", Arc::from("let x = 1;\n"));
        texts.insert("env.wgsl", Arc::from("let x = 1;\n"));
        assert_eq!(place_of("let x = 1;", 1, &texts), None);
    }

    #[test]
    fn a_device_refusal_keeps_its_label_and_deepest_causes() {
        let report = "Validation Error\n\nCaused by:\n  In Device::create_render_pipeline, label = 'sky'\n    Error matching ShaderStages(FRAGMENT) shader requirements against the pipeline\n      Shader global ResourceBinding { group: 1, binding: 9 } is not available in the pipeline layout\n        Binding is missing from the pipeline layout\n";
        assert_eq!(
            device_failure(report).message,
            "Device::create_render_pipeline, label = 'sky': Shader global ResourceBinding { group: 1, binding: 9 } is not available in the pipeline layout: Binding is missing from the pipeline layout"
        );
        assert_eq!(device_failure("Out of memory").message, "Out of memory");
    }

    #[test]
    fn only_the_listed_files_reload() {
        assert!(reloadable("object.wgsl"));
        assert!(reloadable("water_wake.wgsl"));
        assert!(!reloadable("clouds.wgsl"));
        assert!(!reloadable("oit_stochastic.wgsl"));
    }

    #[test]
    fn a_valid_module_passes() {
        let candidate = Candidate {
            texts: HashMap::new(),
            changed: Vec::new(),
        };
        assert_eq!(validate("test", "fn a() -> f32 { return 1.0; }", &candidate), Ok(()));
    }
}
