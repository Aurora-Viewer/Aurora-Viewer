// XDBG: temporary measures of the exact blend order (not to be committed).
// The phase of the measurement cycle, and the pipeline statistics of the
// visibility pass (query 0) and of the colour pass (query 1).
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub static PHASE: AtomicU8 = AtomicU8::new(0);
/// The ceiling of a pixel asked for while the viewer runs (0: none).
pub static MOST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// The faintest alpha that takes a slot, asked for while the viewer runs
/// (0: the level of the view).
pub static FAINT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Whether the places of the first slots of the pixels are emptied with
/// their heads (the mode holds "wipe").
pub static WIPE: AtomicBool = AtomicBool::new(false);

/// The mode of the measurements: what the file AURORA_XC names says now,
/// or AURORA_X.
pub fn xmode() -> String {
    if let Some(text) = std::env::var("AURORA_XC").ok().and_then(|path| std::fs::read_to_string(path).ok()) {
        return text.trim().to_string();
    }
    std::env::var("AURORA_X").unwrap_or_default()
}

static XS_SEEN: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);
static XS_RELOADING: AtomicBool = AtomicBool::new(false);
static XS_LOOKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Whether the pipelines are to be made again: the shader AURORA_XS names
/// changed since they were made from it (looked at every 30 calls).
pub fn xs_reload() -> bool {
    if XS_RELOADING.load(Ordering::Relaxed) {
        return true;
    }
    if !XS_LOOKS.fetch_add(1, Ordering::Relaxed).is_multiple_of(30) {
        return false;
    }
    let Ok(path) = std::env::var("AURORA_XS") else {
        return false;
    };
    let Ok(time) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    let mut seen = XS_SEEN.lock().unwrap_or_else(|p| p.into_inner());
    let changed = seen.is_some_and(|before| before != time);
    *seen = Some(time);
    if changed {
        log::info!("xdbg: the shader of AURORA_XS changed, its pipelines are made again");
        XS_RELOADING.store(true, Ordering::Relaxed);
    }
    changed
}

/// The pipelines are made from the shader as it is now.
pub fn xs_reloaded() {
    XS_RELOADING.store(false, Ordering::Relaxed);
}

const SLOT: u64 = 256;

struct Stats {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    ready: Arc<AtomicBool>,
    pending: bool,
    copied: bool,
    written: [bool; 2],
    logged: (u8, [bool; 2], (u32, u32)),
    frame: u64,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_X").as_deref() == Ok("cycle"))
}

/// The query set, made on first use; `None` without the device feature.
pub fn query_set(device: &wgpu::Device, i: usize) -> Option<wgpu::QuerySet> {
    if !on() || !device.features().contains(wgpu::Features::PIPELINE_STATISTICS_QUERY) {
        return None;
    }
    let mut guard = STATS.lock().ok()?;
    let stats = guard.get_or_insert_with(|| {
        let types = wgpu::PipelineStatisticsTypes::VERTEX_SHADER_INVOCATIONS
            | wgpu::PipelineStatisticsTypes::CLIPPER_PRIMITIVES_OUT
            | wgpu::PipelineStatisticsTypes::FRAGMENT_SHADER_INVOCATIONS;
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 2 * SLOT,
                usage,
                mapped_at_creation: false,
            })
        };
        Stats {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("xdbg stats"),
                ty: wgpu::QueryType::PipelineStatistics(types),
                count: 2,
            }),
            resolve: buffer("xdbg resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            readback: buffer("xdbg readback", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            ready: Arc::new(AtomicBool::new(false)),
            pending: false,
            copied: false,
            written: [false; 2],
            logged: (0, [false; 2], (0, 0)),
            frame: 0,
        }
    });
    if stats.pending {
        return None;
    }
    stats.written[i] = true;
    Some(stats.set.clone())
}

/// After the passes: the queries written this frame, to the readback.
pub fn resolve(encoder: &mut wgpu::CommandEncoder, size: (u32, u32)) {
    let Ok(mut guard) = STATS.lock() else { return };
    let Some(stats) = guard.as_mut() else { return };
    stats.frame += 1;
    let written = std::mem::take(&mut stats.written);
    if stats.pending || written == [false; 2] {
        return;
    }
    for (i, _) in written.iter().enumerate().filter(|(_, w)| **w) {
        encoder.resolve_query_set(&stats.set, i as u32..i as u32 + 1, &stats.resolve, i as u64 * SLOT);
    }
    // one frame in twenty is read back
    if stats.frame % 20 != 0 {
        return;
    }
    encoder.copy_buffer_to_buffer(&stats.resolve, 0, &stats.readback, 0, 2 * SLOT);
    stats.copied = true;
    stats.logged = (PHASE.load(Ordering::Relaxed), written, size);
}

pub fn after_submit() {
    dump_after_submit();
    let Ok(mut guard) = STATS.lock() else { return };
    let Some(stats) = guard.as_mut() else { return };
    if stats.pending && stats.ready.load(Ordering::Acquire) {
        if let Ok(view) = stats.readback.get_mapped_range(..) {
            let word = |q: usize, k: usize| {
                let at = q * SLOT as usize + k * 8;
                u64::from_le_bytes(view[at..at + 8].try_into().unwrap_or([0; 8]))
            };
            let (phase, written, size) = stats.logged;
            let mut line = format!("xstat phase={phase} size={}x{}", size.0, size.1);
            for (q, name) in ["v", "s"].iter().enumerate() {
                if written[q] {
                    line += &format!(" | {name}: vs={} prims={} frags={}", word(q, 0), word(q, 1), word(q, 2));
                }
            }
            log::info!("{line}");
        }
        stats.readback.unmap();
        stats.pending = false;
        stats.ready.store(false, Ordering::Release);
    }
    if stats.copied {
        stats.copied = false;
        stats.pending = true;
        let ready = stats.ready.clone();
        stats.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
    }
}

struct Dump {
    readback: wgpu::Buffer,
    ready: Arc<AtomicBool>,
    pending: bool,
    copied: bool,
    frame: u64,
    heads: u64,
    pages: u64,
}

static DUMP: Mutex<Option<Dump>> = Mutex::new(None);

/// Now and then, while the mode holds "stats", a frame of the new order:
/// the heads of the pixels and the words of the pool to the CPU. `pool`:
/// the byte of the fronts where the pool starts.
pub fn dump(device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, fronts: &wgpu::Buffer, pool: u64) {
    let Ok(mut guard) = DUMP.lock() else { return };
    // (the heads: four words of the sixteen of a pixel; and the two planes
    // after them, the places of its slots)
    let heads = pool / 4;
    let planes = heads * 3;
    let size = planes + 64;
    if guard.as_ref().is_some_and(|d| d.readback.size() != size && !d.pending) {
        *guard = None;
    }
    let dump = guard.get_or_insert_with(|| Dump {
        readback: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("xdbg fronts"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        ready: Arc::new(AtomicBool::new(false)),
        pending: false,
        copied: false,
        frame: 0,
        heads,
        pages: 0,
    });
    dump.frame += 1;
    if dump.frame < 120 || dump.pending || dump.readback.size() != size {
        return;
    }
    dump.frame = 0;
    if !xmode().contains("stats") {
        return;
    }
    dump.pages = (fronts.size() - pool) / 64 - 1;
    encoder.copy_buffer_to_buffer(fronts, 0, &dump.readback, 0, planes);
    encoder.copy_buffer_to_buffer(fronts, pool, &dump.readback, planes, 64);
    dump.copied = true;
}

fn dump_after_submit() {
    let Ok(mut guard) = DUMP.lock() else { return };
    let Some(dump) = guard.as_mut() else { return };
    if dump.pending && dump.ready.load(Ordering::Acquire) {
        if let Ok(view) = dump.readback.get_mapped_range(..) {
            let words: &[u32] = bytemuck::cast_slice(&view);
            let pixels = dump.heads as usize / 16;
            let counters = &words[pixels * 12..];
            // of the pixels of six strong fragments or fewer: the fragments
            // that were lit though those before them let nothing through
            // (or that lie under the cut), and the pixels by the fragments
            // that do show (0 to 6)
            let (mut small, mut hidden) = (0u64, 0u64);
            let mut shown = [0u64; 7];
            // strong fragments of a pixel: 0, 1-6, 7-12 ... 37-42, 43-84,
            // 85-168, 169-336, more
            let mut by_count = [0u64; 12];
            // the fragments over 42 of the pixels of the last four classes
            let mut over = [0u64; 4];
            let (mut strong, mut cut, mut hazy, mut paged, mut needed, mut starved, mut short) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
            for p in 0..pixels {
                let head = &words[p * 4..p * 4 + 4];
                let count = u64::from(head[0] & 0xffff);
                strong += count;
                let class = match count {
                    0 => 0,
                    1..=42 => (count as usize).div_ceil(6),
                    43..=84 => 8,
                    85..=168 => 9,
                    169..=336 => 10,
                    _ => 11,
                };
                by_count[class] += 1;
                if class >= 8 {
                    over[class - 8] += count - 42;
                }
                cut += u64::from(head[1] != 0);
                hazy += u64::from(head[2] != 0 || head[0] >> 16 != 0);
                paged += u64::from(head[3] != 0);
                if (1..=6).contains(&count) {
                    let mut keys = [0u32; 6];
                    for (s, key) in keys.iter_mut().enumerate().take(count as usize) {
                        *key = if s < 4 { words[pixels * 4 + p * 4 + s] } else { words[pixels * 8 + p * 4 + s - 4] };
                    }
                    keys.sort_unstable_by(|a, b| b.cmp(a));
                    let mut t = 1.0_f32;
                    let mut seen = 0;
                    for key in &keys[..count as usize] {
                        if t < 0.002 || (key >> 8) < (head[1] >> 8) {
                            hidden += 1;
                        } else {
                            seen += 1;
                            t *= 1.0 - (key & 0xff) as f32 / 255.0;
                        }
                    }
                    small += count;
                    shown[seen] += 1;
                }
                let want = (count.min(42).saturating_sub(6)).div_ceil(6);
                needed += want;
                // (no page at all though it wants some: a pixel the pool
                // had nothing for)
                starved += u64::from(want != 0 && head[3] == 0);
                short += if want != 0 && head[3] == 0 { count.min(42) - 6 } else { 0 };
            }
            log::info!(
                "xdump pixels={pixels} strong={strong} by_count={by_count:?} over42={over:?} cut_px={cut} glow_px={hazy} paged_px={paged} pages_needed={needed} pages_taken={} pool={} starved_px={starved} starved_frags={short} small_frags={small} small_hidden={hidden} small_shown_px={shown:?} counters={:?}",
                counters[0],
                dump.pages,
                &counters[1..]
            );
        }
        dump.readback.unmap();
        dump.pending = false;
        dump.ready.store(false, Ordering::Release);
    }
    if dump.copied {
        dump.copied = false;
        dump.pending = true;
        let ready = dump.ready.clone();
        dump.readback.map_async(wgpu::MapMode::Read, .., move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
    }
}
