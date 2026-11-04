//! Aurora's own pipeline cache on disk (wgpu `PipelineCache`, a Vulkan
//! `VkPipelineCache`): the GPU code the driver compiled for the renderer's
//! pipelines, read back at the next start so the driver does not compile
//! it again.
//!
//! The drivers keep a cache of their own, but it is shared by every program
//! and bounded: NVIDIA's (`%LOCALAPPDATA%\NVIDIA\GLCache`, 1 GB by default)
//! evicts the viewer's entries once other programs fill it, and each start
//! then compiled every pipeline again (17–21 s with nothing on screen,
//! against 0.6 s warm). This one belongs to the viewer and stays.
//!
//! - **One file per GPU** (`<cache>/pipelines/<wgpu key>.bin`, the key of
//!   `wgpu::util::pipeline_cache_key`: the backend, vendor and device ids).
//!   Its header names the driver and the Aurora build that wrote it, plus a
//!   checksum of the data: another driver or build starts an empty cache
//!   (and overwrites the file), a damaged file is ignored. wgpu checks its
//!   own header again and the driver the `VkPipelineCache` one.
//! - **Every pipeline** goes through [`CachedPipelines`] (a test refuses a
//!   direct `create_render_pipeline` / `create_compute_pipeline` in this
//!   crate), those of the render thread's worker (`PipelineBuild`) and the
//!   ones made later included. egui-wgpu's single pipeline has no cache
//!   field.
//! - **Written off the frame path**: a thread of its own saves once no
//!   pipeline was made for [`QUIET`] once the renderer is ready, and the
//!   viewer calls [`flush`] at exit. Unchanged data is not written again. Atomic: a temporary file
//!   renamed over the old one.
//! - **Where the backend has no such cache** (Metal), or the driver lacks
//!   the feature, or `AURORA_PIPELINE_CACHE=0`, nothing changes: the
//!   pipelines are made without one, as before.

use parking_lot::{Condvar, Mutex};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The file's first bytes.
const MAGIC: &[u8; 8] = b"AURPLC\0\x01";
/// Past it the file is not read (a cache is a few MB).
const MAX_FILE: u64 = 512 << 20;
/// No pipeline made for that long: the set is complete, it is saved.
const QUIET: Duration = Duration::from_secs(2);

/// What the viewer gives before the renderer starts.
struct Config {
    dir: PathBuf,
    build: String,
}

/// The cache of the device in use.
struct Active {
    device: wgpu::Device,
    cache: wgpu::PipelineCache,
    path: PathBuf,
    key: String,
    /// Checksum of the data on disk (0: none).
    saved: u64,
}

static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);
/// One save at a time (the thread's, or the exit's).
static SAVING: Mutex<()> = Mutex::new(());
/// When the last pipeline was made since the last save (None: saved).
static DIRTY: Mutex<Option<Instant>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();
/// A lost device's cache is not saved.
static LOST: AtomicBool = AtomicBool::new(false);

/// Where the cache files go and the build that writes them: before
/// `Renderer::new`. Without it the renderer makes its pipelines without a
/// cache.
pub fn configure(dir: PathBuf, build: &str) {
    *CONFIG.lock() = Some(Config {
        dir,
        build: build.to_owned(),
    });
}

/// `AURORA_PIPELINE_CACHE=0` turns the cache off (comparisons).
fn wanted() -> bool {
    std::env::var("AURORA_PIPELINE_CACHE").map_or(true, |v| v.trim() != "0")
}

/// The device feature to ask for, when the adapter has it.
pub(crate) fn feature(adapter: wgpu::Features) -> wgpu::Features {
    if wanted() && adapter.contains(wgpu::Features::PIPELINE_CACHE) {
        wgpu::Features::PIPELINE_CACHE
    } else {
        wgpu::Features::empty()
    }
}

/// The header's identity: what must match for the data to be used.
pub(crate) fn identity(wgpu_key: &str, info: &wgpu::AdapterInfo, build: &str) -> String {
    identity_of(wgpu_key, &info.name, &info.driver, &info.driver_info, build)
}

fn identity_of(wgpu_key: &str, name: &str, driver: &str, driver_info: &str, build: &str) -> String {
    format!("{wgpu_key}|{name}|{driver}|{driver_info}|{build}")
}

/// Why a file's data is not used.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reject {
    /// Not one of our files, or too short for its header.
    Format,
    /// Written for another GPU, driver or build.
    Identity,
    /// Shorter or longer than its header says.
    Length,
    /// The checksum does not match (damaged on the disk).
    Checksum,
}

/// A file's content, once checked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stored<'a> {
    /// Pipelines the start that wrote it made.
    pub pipelines: u32,
    pub data: &'a [u8],
}

/// FNV-1a, 64 bits: a checksum, nothing to defend against.
pub(crate) fn checksum(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `MAGIC`, identity length (u16) and identity, pipelines (u32), data
/// length (u64), checksum (u64), data; little-endian.
pub(crate) fn encode(identity: &str, pipelines: u32, data: &[u8]) -> Vec<u8> {
    let id = &identity.as_bytes()[..identity.len().min(u16::MAX as usize)];
    let mut out = Vec::with_capacity(MAGIC.len() + 2 + id.len() + 4 + 16 + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(id.len() as u16).to_le_bytes());
    out.extend_from_slice(id);
    out.extend_from_slice(&pipelines.to_le_bytes());
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&checksum(data).to_le_bytes());
    out.extend_from_slice(data);
    out
}

pub(crate) fn decode<'a>(bytes: &'a [u8], identity: &str) -> Result<Stored<'a>, Reject> {
    fn take<'b>(rest: &mut &'b [u8], n: usize) -> Result<&'b [u8], Reject> {
        if rest.len() < n {
            return Err(Reject::Format);
        }
        let (head, tail) = rest.split_at(n);
        *rest = tail;
        Ok(head)
    }
    let mut rest = bytes;
    if take(&mut rest, MAGIC.len())? != MAGIC {
        return Err(Reject::Format);
    }
    let id_len = u16::from_le_bytes(take(&mut rest, 2)?.try_into().map_err(|_| Reject::Format)?) as usize;
    if take(&mut rest, id_len)? != identity.as_bytes() {
        return Err(Reject::Identity);
    }
    let pipelines = u32::from_le_bytes(take(&mut rest, 4)?.try_into().map_err(|_| Reject::Format)?);
    let len = u64::from_le_bytes(take(&mut rest, 8)?.try_into().map_err(|_| Reject::Format)?);
    let sum = u64::from_le_bytes(take(&mut rest, 8)?.try_into().map_err(|_| Reject::Format)?);
    if rest.len() as u64 != len {
        return Err(Reject::Length);
    }
    if checksum(rest) != sum {
        return Err(Reject::Checksum);
    }
    Ok(Stored { pipelines, data: rest })
}

/// Publishes `bytes` at `path` whole or not at all: a temporary file of this
/// process in the same folder, renamed over the old one. Windows refuses
/// the rename for a moment while another process reads the file: retried.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        drop(file);
        let mut attempt = 0;
        loop {
            match std::fs::rename(&temporary, path) {
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && attempt < 5 => {
                    std::thread::sleep(Duration::from_millis(1 << attempt));
                    attempt += 1;
                }
                other => return other,
            }
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Makes the device's cache from the file of its GPU, when the device has
/// the feature and the viewer configured a folder; from `Renderer::new`,
/// before the first pipeline.
pub(crate) fn open(device: &wgpu::Device, info: &wgpu::AdapterInfo) {
    LOST.store(false, Ordering::Relaxed);
    if !wanted() {
        log::info!("pipeline cache: off (AURORA_PIPELINE_CACHE=0)");
        return;
    }
    if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
        log::info!("pipeline cache: not available on this device, the driver's own cache only");
        return;
    }
    let Some(wgpu_key) = wgpu::util::pipeline_cache_key(info) else {
        log::info!("pipeline cache: not available on {:?}", info.backend);
        return;
    };
    let Some((dir, build)) = CONFIG.lock().as_ref().map(|c| (c.dir.clone(), c.build.clone())) else {
        return;
    };
    let path = dir.join(format!("{wgpu_key}.bin"));
    let identity = identity(&wgpu_key, info, &build);
    let started = Instant::now();
    let bytes = match std::fs::metadata(&path) {
        Ok(m) if m.len() > MAX_FILE => {
            log::warn!("pipeline cache: {} is {} MB, not read", path.display(), m.len() >> 20);
            None
        }
        Ok(_) => match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!("pipeline cache: {} not read: {e}", path.display());
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            log::warn!("pipeline cache: {}: {e}", path.display());
            None
        }
    };
    let stored = bytes.as_deref().map(|b| decode(b, &identity));
    let (data, saved) = match &stored {
        Some(Ok(s)) => {
            crate::start_progress::expect(s.pipelines);
            log::info!(
                "pipeline cache: {} KB read for {} pipelines in {:.1} ms",
                s.data.len() >> 10,
                s.pipelines,
                started.elapsed().as_secs_f64() * 1000.0
            );
            (Some(s.data), checksum(s.data))
        }
        Some(Err(why)) => {
            log::info!("pipeline cache: {} not used ({why:?}): starting an empty one", path.display());
            (None, 0)
        }
        None => {
            log::info!("pipeline cache: none yet for this GPU: starting an empty one");
            (None, 0)
        }
    };
    // SAFETY: `data` is what `PipelineCache::get_data` gave an earlier run:
    // the file's identity (this GPU, driver and build) and its checksum
    // were checked above, and `fallback` makes wgpu start an empty cache
    // when its own header (adapter, driver key) does not match; the driver
    // checks the VkPipelineCache header (vendor, device, cache UUID) too.
    let cache = unsafe {
        device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
            label: Some("aurora pipelines"),
            data,
            fallback: true,
        })
    };
    *ACTIVE.lock() = Some(Active {
        device: device.clone(),
        cache,
        path,
        key: identity,
        saved,
    });
    start_saver();
}

/// The device was lost: what its cache holds now is not trusted.
pub(crate) fn device_lost() {
    LOST.store(true, Ordering::Relaxed);
}

/// The cache to give a pipeline made on `device`.
fn cache_for(device: &wgpu::Device) -> Option<wgpu::PipelineCache> {
    ACTIVE.lock().as_ref().filter(|a| a.device == *device).map(|a| a.cache.clone())
}

/// The start's pipelines: their time, and the slowest few, for the log.
struct StartTimes {
    total: Duration,
    slowest: Vec<(String, Duration)>,
}
static START: Mutex<StartTimes> = Mutex::new(StartTimes {
    total: Duration::ZERO,
    slowest: Vec::new(),
});

fn made(label: Option<&str>, took: Duration) {
    if crate::start_progress::made_at_ready().is_none() {
        let mut s = START.lock();
        s.total += took;
        s.slowest.push((label.unwrap_or("unnamed").to_owned(), took));
        s.slowest.sort_by_key(|a| std::cmp::Reverse(a.1));
        s.slowest.truncate(3);
    }
    crate::start_progress::pipeline_made();
    *DIRTY.lock() = Some(Instant::now());
    WAKE.notify_one();
}

/// The save thread: waits for a pipeline, then for [`QUIET`] without one.
fn start_saver() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new().name("aurora-pipeline-cache".into()).spawn(|| {
        loop {
            {
                let mut dirty = DIRTY.lock();
                loop {
                    match *dirty {
                        None => WAKE.wait(&mut dirty),
                        // (and not before the renderer is ready: a slow
                        // driver leaves gaps between the start's pipelines)
                        Some(at) if at.elapsed() < QUIET || crate::start_progress::made_at_ready().is_none() => {
                            let left = QUIET.saturating_sub(at.elapsed()).max(Duration::from_millis(100));
                            WAKE.wait_for(&mut dirty, left);
                        }
                        Some(_) => break,
                    }
                }
                *dirty = None;
            }
            save();
        }
    });
    if let Err(e) = spawned {
        log::warn!("pipeline cache: no save thread ({e}), saved at exit only");
    }
}

/// The viewer's exit, once the renderer is dropped: saves when the data
/// changed (waiting at most `timeout` for a save already running), then
/// lets the cache and its device go.
pub fn flush(timeout: Duration) {
    if let Some(_saving) = SAVING.try_lock_for(timeout) {
        *DIRTY.lock() = None;
        save_locked();
    }
    drop(ACTIVE.lock().take());
}

fn save() {
    let _saving = SAVING.lock();
    save_locked();
}

/// With `SAVING` held.
fn save_locked() {
    if LOST.load(Ordering::Relaxed) {
        return;
    }
    let Some((cache, path, key, saved)) = ACTIVE
        .lock()
        .as_ref()
        .map(|a| (a.cache.clone(), a.path.clone(), a.key.clone(), a.saved))
    else {
        return;
    };
    let started = Instant::now();
    let Some(data) = cache.get_data() else {
        return;
    };
    let sum = checksum(&data);
    if sum == saved || data.is_empty() {
        return;
    }
    let pipelines = crate::start_progress::made_at_ready().unwrap_or_else(|| crate::start_progress::snapshot().pipelines);
    match write_atomic(&path, &encode(&key, pipelines, &data)) {
        Ok(()) => {
            log::info!(
                "pipeline cache: {} KB saved in {:.1} ms",
                data.len() >> 10,
                started.elapsed().as_secs_f64() * 1000.0
            );
            if let Some(a) = ACTIVE.lock().as_mut().filter(|a| a.path == path) {
                a.saved = sum;
            }
        }
        Err(e) => log::warn!("pipeline cache: {} not written: {e}", path.display()),
    }
}

/// One line on the start's pipelines, once the renderer is ready: how
/// many, how long the driver took, and the slowest.
pub(crate) fn log_start() {
    let s = START.lock();
    let slowest: Vec<String> = s
        .slowest
        .iter()
        .map(|(l, t)| format!("{l} {:.0} ms", t.as_secs_f64() * 1000.0))
        .collect();
    log::info!(
        "pipelines at start: {} made in {:.0} ms (slowest: {})",
        crate::start_progress::made_at_ready().unwrap_or(0),
        s.total.as_secs_f64() * 1000.0,
        slowest.join(", ")
    );
}

/// Render and compute pipelines with the device's pipeline cache (when it
/// has one) and counted for the start's progress (`start_progress.rs`).
pub trait CachedPipelines {
    fn create_render_pipeline_cached(&self, desc: &wgpu::RenderPipelineDescriptor<'_>) -> wgpu::RenderPipeline;
    fn create_compute_pipeline_cached(&self, desc: &wgpu::ComputePipelineDescriptor<'_>) -> wgpu::ComputePipeline;
}

impl CachedPipelines for wgpu::Device {
    fn create_render_pipeline_cached(&self, desc: &wgpu::RenderPipelineDescriptor<'_>) -> wgpu::RenderPipeline {
        let started = Instant::now();
        let cache = desc.cache.is_none().then(|| cache_for(self)).flatten();
        let pipeline = match &cache {
            Some(cache) => {
                let mut d: wgpu::RenderPipelineDescriptor<'_> = desc.clone();
                d.cache = Some(cache);
                self.create_render_pipeline(&d)
            }
            None => self.create_render_pipeline(desc),
        };
        made(desc.label, started.elapsed());
        pipeline
    }

    fn create_compute_pipeline_cached(&self, desc: &wgpu::ComputePipelineDescriptor<'_>) -> wgpu::ComputePipeline {
        let started = Instant::now();
        let cache = desc.cache.is_none().then(|| cache_for(self)).flatten();
        let pipeline = match &cache {
            Some(cache) => {
                let mut d: wgpu::ComputePipelineDescriptor<'_> = desc.clone();
                d.cache = Some(cache);
                self.create_compute_pipeline(&d)
            }
            None => self.create_compute_pipeline(desc),
        };
        made(desc.label, started.elapsed());
        pipeline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "wgpu_pipeline_cache_vulkan_4318_11524|RTX|NVIDIA|596.10|0.8.4+3.gabc";

    #[test]
    fn a_file_reads_back_what_was_written() {
        let data: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8).collect();
        let file = encode(ID, 117, &data);
        assert_eq!(
            decode(&file, ID),
            Ok(Stored {
                pipelines: 117,
                data: &data
            })
        );
        assert_eq!(decode(&encode(ID, 0, &[]), ID).map(|s| s.data.len()), Ok(0));
    }

    #[test]
    fn another_gpu_driver_or_build_starts_empty() {
        let file = encode(ID, 117, b"data");
        for other in [
            "wgpu_pipeline_cache_vulkan_4098_29772|RX|AMD|24.10|0.8.4+3.gabc",
            "wgpu_pipeline_cache_vulkan_4318_11524|RTX|NVIDIA|597.02|0.8.4+3.gabc",
            "wgpu_pipeline_cache_vulkan_4318_11524|RTX|NVIDIA|596.10|0.8.5",
        ] {
            assert_eq!(decode(&file, other), Err(Reject::Identity), "{other}");
        }
        assert_eq!(
            identity_of("wgpu_pipeline_cache_vulkan_4318_11524", "RTX", "NVIDIA", "596.10", "0.8.4+3.gabc"),
            ID
        );
    }

    #[test]
    fn damaged_files_are_refused() {
        let data = vec![0x5a; 4096];
        let file = encode(ID, 117, &data);
        assert_eq!(decode(&[], ID), Err(Reject::Format));
        assert_eq!(decode(b"not a cache at all, just text", ID), Err(Reject::Format));
        for cut in [4, 12, 40, file.len() - 1] {
            assert!(decode(&file[..cut], ID).is_err(), "cut at {cut}");
        }
        assert_eq!(decode(&file[..file.len() - 1], ID), Err(Reject::Length));
        let mut longer = file.clone();
        longer.push(0);
        assert_eq!(decode(&longer, ID), Err(Reject::Length));
        let mut flipped = file.clone();
        let last = flipped.len() - 100;
        flipped[last] ^= 1;
        assert_eq!(decode(&flipped, ID), Err(Reject::Checksum));
        let mut magic = file;
        magic[0] = b'X';
        assert_eq!(decode(&magic, ID), Err(Reject::Format));
    }

    #[test]
    fn the_write_replaces_the_file_whole() {
        let dir = std::env::temp_dir().join(format!("aurora-pipeline-cache-test-{}", std::process::id()));
        let path = dir.join("sub").join("cache.bin");
        write_atomic(&path, b"first version, longer than the second").expect("written");
        write_atomic(&path, b"second").expect("replaced");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        let left: Vec<_> = std::fs::read_dir(path.parent().expect("folder"))
            .expect("listed")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 1, "no temporary file left: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No pipeline of this crate skips the cache: every one goes through
    /// `CachedPipelines`.
    #[test]
    fn every_pipeline_goes_through_the_cache() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let mut found = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("source folder").filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && path.file_name().is_some_and(|n| n != "pipeline_cache.rs") {
                    let text = std::fs::read_to_string(&path).expect("source file");
                    for (n, line) in text.lines().enumerate() {
                        if line.contains(".create_render_pipeline(") || line.contains(".create_compute_pipeline(") {
                            found.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(found.is_empty(), "use create_*_pipeline_cached (pipeline_cache.rs): {found:#?}");
    }
}
