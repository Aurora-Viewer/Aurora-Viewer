//! Compares this decoder with OpenJPEG on JPEG 2000 codestreams, sample for
//! sample, and measures both. Read-only: nothing is written.
//!
//! Usage: j2k_compare <folder or file>... [--sample N] [--cuts N] [--bench] [--rounds N]
//!
//! - whole files: every file at every reduce level up to 5, both decoders,
//!   every sample compared;
//! - `--cuts N`: N prefixes of every file (what a download that stopped
//!   there holds), at reduce 0, 1, 2, 3 and 5. A cut decode may differ from
//!   OpenJPEG's only where this decoder says it kept what OpenJPEG drops
//!   (`Image::cut_in_decoded`); such a decode must then be at least as
//!   close to the whole file's as OpenJPEG's is;
//! - `--bench`: the time of whole decodes on one thread, by size class and
//!   reduce level, for both decoders (`--rounds N` decodes of each, the best
//!   kept). Build with `--release`, on a quiet machine.
//!
//! The files are raw codestreams (`.j2c`, `.j2k`), as the viewer caches
//! textures.

#[path = "../tests/oracle/mod.rs"]
mod oracle;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use aurora_j2k::{Decoder, Limits, Options};
use oracle::{Plane, Verdict};
use rayon::prelude::*;

struct Args {
    paths: Vec<PathBuf>,
    sample: usize,
    cuts: usize,
    bench: bool,
    rounds: usize,
}

fn args() -> Option<Args> {
    let mut out = Args {
        paths: Vec::new(),
        sample: usize::MAX,
        cuts: 0,
        bench: false,
        rounds: 3,
    };
    let mut a = std::env::args().skip(1);
    while let Some(k) = a.next() {
        match k.as_str() {
            "--sample" => out.sample = a.next()?.parse().ok()?,
            "--cuts" => out.cuts = a.next()?.parse().ok()?,
            "--rounds" => out.rounds = a.next()?.parse().ok()?,
            "--bench" => out.bench = true,
            _ => out.paths.push(PathBuf::from(k)),
        }
    }
    (!out.paths.is_empty()).then_some(out)
}

fn files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let Ok(dir) = std::fs::read_dir(p) else { continue };
            for e in dir.flatten() {
                let path = e.path();
                if matches!(path.extension().and_then(|e| e.to_str()), Some("j2c" | "j2k")) {
                    out.push(path);
                }
            }
        } else {
            out.push(p.clone());
        }
    }
    out.sort();
    out
}

fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn ours(dec: &mut Decoder, cs: &[u8], reduce: u8) -> Result<aurora_j2k::Image, aurora_j2k::Error> {
    dec.decode(
        cs,
        &Options {
            reduce,
            components: 0,
            limits: Limits::NONE,
        },
    )
}

/// Sum of squared differences between two decodes of the same size.
fn distance(a: &[Plane], b: &[Plane]) -> Option<u64> {
    let mut sum = 0u64;
    for (pa, pb) in a.iter().zip(b) {
        if (pa.w, pa.h) != (pb.w, pb.h) {
            return None;
        }
        for (x, y) in pa.data.iter().zip(&pb.data) {
            let d = i64::from(*x) - i64::from(*y);
            sum += (d * d) as u64;
        }
    }
    Some(sum)
}

#[derive(Default)]
struct Counters {
    requests: AtomicUsize,
    same: AtomicUsize,
    both_refused: AtomicUsize,
    kept_more: AtomicUsize,
    kept_more_worse: AtomicUsize,
    outcome_ours_only: AtomicUsize,
    outcome_oracle_only: AtomicUsize,
    unexplained: AtomicUsize,
}

fn bump(c: &AtomicUsize) {
    c.fetch_add(1, Ordering::Relaxed);
}

/// The cut points of a file: the header estimate and fractions the viewer's
/// range requests come close to, then pseudo-random ones.
fn cut_points(len: usize, n: usize, seed: u64) -> Vec<usize> {
    let mut cuts: Vec<usize> = [600, 1024, len / 64, len / 16, len / 8, len / 4, len / 2, len * 3 / 4, len - 1]
        .into_iter()
        .filter(|&c| c > 0 && c < len)
        .collect();
    let mut s = seed | 1;
    while cuts.len() < n && len > 2 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        cuts.push(1 + (s >> 33) as usize % (len - 1));
    }
    cuts.truncate(n);
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}

fn main() {
    let Some(args) = args() else {
        eprintln!("usage: j2k_compare <folder or file>... [--sample N] [--cuts N] [--bench] [--rounds N]");
        std::process::exit(2);
    };
    let mut list = files(&args.paths);
    if list.len() > args.sample {
        // an even sample over the sorted names
        let step = list.len() as f64 / args.sample as f64;
        list = (0..args.sample).map(|i| list[(i as f64 * step) as usize].clone()).collect();
    }
    println!("{} files", list.len());

    // ---- whole files ----------------------------------------------------------------------
    let whole = Counters::default();
    let cut = Counters::default();
    let report = std::sync::Mutex::new(Vec::<String>::new());
    let note = |line: String| {
        let mut r = report.lock().unwrap_or_else(|e| e.into_inner());
        if r.len() < 60 {
            r.push(line);
        }
    };
    list.par_iter().for_each_init(Decoder::new, |dec, path| {
        let Ok(cs) = std::fs::read(path) else { return };
        let levels = aurora_j2k::read_header(&cs, &Limits::NONE).map(|h| h.max_reduce()).unwrap_or(0);
        let mut full: Vec<Option<Vec<Plane>>> = Vec::new();
        for reduce in 0..=levels.min(5) {
            let a = oracle::decode(&cs, u32::from(reduce));
            let b = ours(dec, &cs, reduce);
            bump(&whole.requests);
            match oracle::compare(&a, &b) {
                Verdict::Same => bump(if a.is_some() { &whole.same } else { &whole.both_refused }),
                Verdict::Outcome { oracle, ours: _ } => {
                    bump(if oracle { &whole.outcome_oracle_only } else { &whole.outcome_ours_only });
                    note(format!("WHOLE {} reduce {reduce}: oracle {} / ours {:?}", name(path), if oracle { "ok" } else { "refused" }, b.as_ref().map(|_| "ok").map_err(|e| *e)));
                }
                // a file of the cache can be a download that stopped early
                Verdict::Differ { .. } if b.as_ref().is_ok_and(|img| img.cut_in_decoded) => bump(&whole.kept_more),
                Verdict::Differ { comp, at, count } => {
                    bump(&whole.unexplained);
                    note(format!("WHOLE {} reduce {reduce}: {count} samples differ, first in component {comp} at {at}", name(path)));
                }
            }
            full.push(a);
        }
        if args.cuts == 0 {
            return;
        }
        // ---- prefixes --------------------------------------------------------------------
        let seed = cs.len() as u64 * 2654435761 + u64::from(cs.get(cs.len() / 2).copied().unwrap_or(0));
        for at in cut_points(cs.len(), args.cuts, seed) {
            let part = &cs[..at];
            for reduce in [0u8, 1, 2, 3, 5] {
                if reduce > levels {
                    continue;
                }
                let a = oracle::decode(part, u32::from(reduce));
                let b = ours(dec, part, reduce);
                bump(&cut.requests);
                match oracle::compare(&a, &b) {
                    Verdict::Same => bump(if a.is_some() { &cut.same } else { &cut.both_refused }),
                    Verdict::Outcome { oracle, ours: _ } => {
                        bump(if oracle { &cut.outcome_oracle_only } else { &cut.outcome_ours_only });
                        note(format!("CUT {} at {at} reduce {reduce}: oracle {} / ours {:?}", name(path), if oracle { "ok" } else { "refused" }, b.as_ref().map(|_| "ok").map_err(|e| *e)));
                    }
                    Verdict::Differ { comp, at: sample, count } => {
                        let kept = b.as_ref().is_ok_and(|img| img.cut_in_decoded);
                        if !kept {
                            bump(&cut.unexplained);
                            note(format!("CUT {} at {at} reduce {reduce}: {count} samples differ (first: component {comp}, sample {sample}) and the decode kept nothing more", name(path)));
                            continue;
                        }
                        bump(&cut.kept_more);
                        // closer to the whole file than OpenJPEG's, or as close
                        let reference = full.get(usize::from(reduce)).and_then(|f| f.as_ref());
                        if let (Some(reference), Some(a), Ok(b)) = (reference, &a, &b) {
                            let b = oracle::planes_of(b);
                            if let (Some(da), Some(db)) = (distance(a, reference), distance(&b, reference))
                                && db > da
                            {
                                bump(&cut.kept_more_worse);
                                note(format!("CUT {} at {at} reduce {reduce}: kept more, yet further from the whole file ({db} against {da})", name(path)));
                            }
                        }
                    }
                }
            }
        }
    });
    let show = |title: &str, c: &Counters| {
        let n = |a: &AtomicUsize| a.load(Ordering::Relaxed);
        println!("{title}: {} decodes", n(&c.requests));
        println!("  identical to OpenJPEG ............. {}", n(&c.same));
        println!("  refused by both ................... {}", n(&c.both_refused));
        println!(
            "  kept more than OpenJPEG (cuts) .... {} (of which further from the whole file: {})",
            n(&c.kept_more),
            n(&c.kept_more_worse)
        );
        println!("  decoded where OpenJPEG refuses .... {}", n(&c.outcome_ours_only));
        println!("  refused where OpenJPEG decodes .... {}", n(&c.outcome_oracle_only));
        println!("  DIFFERENT, UNEXPLAINED ............ {}", n(&c.unexplained));
    };
    show("whole files", &whole);
    if args.cuts > 0 {
        show("cut files", &cut);
    }
    for line in report.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        println!("  {line}");
    }

    // ---- bench --------------------------------------------------------------------------
    if !args.bench {
        return;
    }
    const CLASSES: [u32; 6] = [64, 128, 256, 512, 1024, 2048];
    // [class][reduce] -> (files, pixels, oracle time, our time)
    let mut table = vec![[(0usize, 0u64, Duration::ZERO, Duration::ZERO); 6]; CLASSES.len() + 1];
    let mut dec = Decoder::new();
    for path in &list {
        let Ok(cs) = std::fs::read(path) else { continue };
        let Ok(header) = aurora_j2k::read_header(&cs, &Limits::NONE) else {
            continue;
        };
        let (w, h) = (header.x1 - header.x0, header.y1 - header.y0);
        let class = CLASSES.iter().position(|&c| w.max(h) <= c).unwrap_or(CLASSES.len());
        for reduce in 0..=header.max_reduce().min(5) {
            let best = |f: &mut dyn FnMut() -> bool| -> Option<Duration> {
                let mut best = None;
                for _ in 0..args.rounds.max(1) {
                    let t = Instant::now();
                    if !f() {
                        return None;
                    }
                    let d = t.elapsed();
                    best = Some(best.map_or(d, |b: Duration| b.min(d)));
                }
                best
            };
            let a = best(&mut || oracle::decode(&cs, u32::from(reduce)).is_some());
            let b = best(&mut || ours(&mut dec, &cs, reduce).is_ok());
            if let (Some(a), Some(b)) = (a, b) {
                let e = &mut table[class][usize::from(reduce)];
                e.0 += 1;
                e.1 += u64::from(w >> reduce) * u64::from(h >> reduce);
                e.2 += a;
                e.3 += b;
            }
        }
    }
    println!("\none thread, whole files, mean ms a decode (OpenJPEG / aurora-j2k / ratio):");
    println!(
        "{:>7} {:>6} | {}",
        "class",
        "files",
        (0..6)
            .map(|r| format!("{:^24}", format!("reduce {r}")))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let mut total = [(Duration::ZERO, Duration::ZERO); 6];
    for (i, row) in table.iter().enumerate() {
        if row.iter().all(|e| e.0 == 0) {
            continue;
        }
        let class = CLASSES.get(i).map_or(">2048".to_string(), |c| format!("<={c}"));
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(r, e)| {
                total[r].0 += e.2;
                total[r].1 += e.3;
                if e.0 == 0 {
                    return format!("{:^24}", "-");
                }
                let (a, b) = (e.2.as_secs_f64() * 1e3 / e.0 as f64, e.3.as_secs_f64() * 1e3 / e.0 as f64);
                format!("{a:8.3} {b:8.3} {:5.2}", b / a)
            })
            .collect();
        println!("{class:>7} {:>6} | {}", row[0].0, cells.join(" | "));
    }
    let cells: Vec<String> = total
        .iter()
        .map(|(a, b)| {
            format!(
                "{:8.2}s {:7.2}s {:5.2}",
                a.as_secs_f64(),
                b.as_secs_f64(),
                b.as_secs_f64() / a.as_secs_f64().max(1e-9)
            )
        })
        .collect();
    println!("{:>7} {:>6} | {}", "total", "", cells.join(" | "));
    let px: u64 = table.iter().map(|row| row[0].1).sum();
    let (a, b) = total[0];
    println!(
        "full resolution: OpenJPEG {:.1} Mpx/s, aurora-j2k {:.1} Mpx/s",
        px as f64 / 1e6 / a.as_secs_f64().max(1e-9),
        px as f64 / 1e6 / b.as_secs_f64().max(1e-9)
    );
}
