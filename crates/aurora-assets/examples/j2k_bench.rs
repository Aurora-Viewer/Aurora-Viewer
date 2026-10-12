//! Measurement: what JPEG2000 decoding costs over a folder of cached textures
//! (`<cache>/tex`, complete `.j2c` files). Read-only: nothing is written and
//! the files are not marked as used.
//!
//! Usage: j2k_bench <dir> [--sample N] [--threads T] [--levels 0,1,2,3]
//!
//! Prints the make-up of the folder (sizes, components), the single-thread
//! decode time per size class at each discard level (codec / conversion
//! split), then the throughput of T decodes in parallel, as the viewer's
//! background pool runs them. Build with `--release`.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use aurora_assets::{DecodeTimings, J2kInfo, decode_j2k_timed, j2k_info};
use rayon::prelude::*;

/// Size classes: largest dimension up to 64, 128, … 2048 and beyond.
const CLASSES: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

fn class_of(info: &J2kInfo) -> usize {
    let m = info.width.max(info.height);
    CLASSES.iter().position(|&c| m <= c).unwrap_or(CLASSES.len())
}

fn class_name(i: usize) -> String {
    match CLASSES.get(i) {
        Some(c) => format!("<={c}"),
        None => ">2048".to_string(),
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// The main header is in the first bytes: no need to read whole files to sort them.
fn header(path: &Path) -> Option<(J2kInfo, u64)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = vec![0u8; 4096.min(len as usize)];
    f.read_exact(&mut head).ok()?;
    Some((j2k_info(&head)?, len))
}

struct Args {
    dir: PathBuf,
    sample: usize,
    threads: usize,
    levels: Vec<u8>,
}

fn args() -> Option<Args> {
    let mut a = std::env::args().skip(1);
    let mut out = Args {
        dir: PathBuf::from(a.next()?),
        sample: 2000,
        threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .saturating_sub(2)
            .max(1),
        levels: vec![0, 1, 2, 3],
    };
    while let Some(k) = a.next() {
        let v = a.next()?;
        match k.as_str() {
            "--sample" => out.sample = v.parse().ok()?,
            "--threads" => out.threads = v.parse().ok()?,
            "--levels" => out.levels = v.split(',').map(|l| l.parse().ok()).collect::<Option<_>>()?,
            _ => return None,
        }
    }
    Some(out)
}

#[derive(Default, Clone)]
struct Row {
    times: Vec<Duration>,
    codec: Duration,
    convert: Duration,
    out_pixels: u64,
}

impl Row {
    fn add(&mut self, t: DecodeTimings, pixels: u64) {
        self.times.push(t.codec + t.convert);
        self.codec += t.codec;
        self.convert += t.convert;
        self.out_pixels += pixels;
    }

    fn merge(&mut self, o: &Row) {
        self.times.extend_from_slice(&o.times);
        self.codec += o.codec;
        self.convert += o.convert;
        self.out_pixels += o.out_pixels;
    }

    fn print(&mut self, name: &str) {
        if self.times.is_empty() {
            return;
        }
        self.times.sort_unstable();
        let total = self.codec + self.convert;
        println!(
            "  {name:>7} {:>6} {:>9.2} {:>8.2} {:>8.2} {:>8.2} {:>8.1} {:>7.1} {:>6.1}%",
            self.times.len(),
            ms(total) / self.times.len() as f64,
            ms(percentile(&self.times, 0.5)),
            ms(percentile(&self.times, 0.95)),
            ms(percentile(&self.times, 1.0)),
            ms(total),
            self.out_pixels as f64 / 1e6 / total.as_secs_f64().max(1e-9),
            100.0 * self.convert.as_secs_f64() / total.as_secs_f64().max(1e-9),
        );
    }
}

fn main() {
    let Some(args) = args() else {
        eprintln!("usage: j2k_bench <dir> [--sample N] [--threads T] [--levels 0,1,2,3]");
        std::process::exit(2);
    };
    let (mut complete, mut partial) = (Vec::new(), 0usize);
    match std::fs::read_dir(&args.dir) {
        Ok(rd) => {
            for p in rd.flatten().map(|e| e.path()) {
                match p.extension().and_then(|e| e.to_str()) {
                    Some("j2c") => complete.push(p),
                    Some("part") => partial += 1,
                    _ => {}
                }
            }
        }
        Err(e) => {
            eprintln!("{}: {e}", args.dir.display());
            std::process::exit(1);
        }
    }
    complete.sort();

    // --- the folder ---
    let headers: Vec<(PathBuf, J2kInfo, u64)> = complete
        .par_iter()
        .filter_map(|p| header(p).map(|(i, len)| (p.clone(), i, len)))
        .collect();
    println!(
        "{}: {} complete textures ({} with a readable header), {} partial ones (not measured)",
        args.dir.display(),
        complete.len(),
        headers.len(),
        partial
    );
    println!("\nfolder    count   1c   2c     3c     4c   file MB   megapixels  RGBA8+mips MB  bits/px");
    let mut all = (0usize, 0u64, 0u64);
    for c in 0..=CLASSES.len() {
        let of: Vec<_> = headers.iter().filter(|h| class_of(&h.1) == c).collect();
        if of.is_empty() {
            continue;
        }
        let comps = |n: u8| of.iter().filter(|h| h.1.components == n).count();
        let bytes: u64 = of.iter().map(|h| h.2).sum();
        let px: u64 = of.iter().map(|h| h.1.width as u64 * h.1.height as u64).sum();
        all = (all.0 + of.len(), all.1 + bytes, all.2 + px);
        println!(
            "{:>7} {:>6} {:>4} {:>4} {:>6} {:>6} {:>9.1} {:>12.1} {:>14.1} {:>8.2}",
            class_name(c),
            of.len(),
            comps(1),
            comps(2),
            comps(3),
            comps(4),
            bytes as f64 / 1e6,
            px as f64 / 1e6,
            px as f64 * 4.0 * 4.0 / 3.0 / 1e6,
            bytes as f64 * 8.0 / px.max(1) as f64,
        );
    }
    println!(
        "{:>7} {:>6} {:>32.1} {:>12.1} {:>14.1} {:>8.2}",
        "all",
        all.0,
        all.1 as f64 / 1e6,
        all.2 as f64 / 1e6,
        all.2 as f64 * 4.0 * 4.0 / 3.0 / 1e6,
        all.1 as f64 * 8.0 / all.2.max(1) as f64,
    );
    let mut levels = [0usize; 8];
    for h in &headers {
        levels[(h.1.levels as usize).min(7)] += 1;
    }
    println!("decomposition levels 0..=7+: {levels:?}");

    // --- the sample, in memory ---
    let step = (headers.len() / args.sample.max(1)).max(1);
    let sample: Vec<(J2kInfo, Vec<u8>)> = headers
        .iter()
        .step_by(step)
        .filter_map(|(p, i, _)| std::fs::read(p).ok().map(|d| (*i, d)))
        .collect();
    if sample.is_empty() {
        eprintln!("no decodable texture");
        std::process::exit(1);
    }
    let sample_px: u64 = sample.iter().map(|s| s.0.width as u64 * s.0.height as u64).sum();
    println!(
        "\nsample: {} textures (one in {step}), {:.1} megapixels",
        sample.len(),
        sample_px as f64 / 1e6
    );
    for (_, d) in sample.iter().take(16) {
        let _ = decode_j2k_timed(d, 0); // warm-up
    }

    // --- one thread ---
    let mut mean_full = 0.0;
    for &level in &args.levels {
        let mut rows = vec![Row::default(); CLASSES.len() + 1];
        let mut failed = 0;
        for (info, data) in &sample {
            match decode_j2k_timed(data, level) {
                Ok((img, t)) => rows[class_of(info)].add(t, img.width as u64 * img.height as u64),
                Err(_) => failed += 1,
            }
        }
        println!("\none thread, discard {level} ({failed} failed)");
        println!("    class  count   mean ms   p50 ms   p95 ms   max ms total ms   MP/s  convert");
        let mut total = Row::default();
        for (c, r) in rows.iter_mut().enumerate() {
            total.merge(r);
            r.print(&class_name(c));
        }
        if level == 0 && !total.times.is_empty() {
            mean_full = ms(total.codec + total.convert) / total.times.len() as f64;
        }
        total.print("all");
    }

    // --- the background pool ---
    let pool = match rayon::ThreadPoolBuilder::new().num_threads(args.threads).build() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("thread pool: {e}");
            std::process::exit(1);
        }
    };
    println!("\n{} threads (the viewer's background pool: cores - 2)", args.threads);
    println!("  discard  wall ms  textures/s   MP/s  cpu ms  speed-up");
    for &level in &args.levels {
        let start = Instant::now();
        let (cpu, px) = pool.install(|| {
            sample
                .par_iter()
                .filter_map(|(_, d)| decode_j2k_timed(d, level).ok())
                .map(|(img, t)| (t.codec + t.convert, img.width as u64 * img.height as u64))
                .reduce(|| (Duration::ZERO, 0), |a, b| (a.0 + b.0, a.1 + b.1))
        });
        let wall = start.elapsed();
        println!(
            "  {level:>7} {:>8.0} {:>11.0} {:>6.0} {:>7.0} {:>8.1}x",
            ms(wall),
            sample.len() as f64 / wall.as_secs_f64(),
            px as f64 / 1e6 / wall.as_secs_f64(),
            ms(cpu),
            cpu.as_secs_f64() / wall.as_secs_f64(),
        );
    }
    if mean_full > 0.0 {
        println!(
            "\n9000 textures of this mix at full resolution: {:.1} s of decoding on one thread",
            mean_full * 9000.0 / 1e3
        );
    }
}
