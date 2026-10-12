//! Measurement: what BC7 compression gives over a folder of cached textures
//! (`<cache>/tex`, complete `.j2c` files). Read-only.
//!
//! Usage: bc7_check <dir> [--sample N] [--discard D]
//!
//! Each texture of the sample is decoded, given its mip chain and passed to
//! `aurora_render::bc7::encode_chain`, as a decode job of the viewer does.
//! Prints how many textures the quality check lets through, the video
//! memory with and without compression, and what the encoding costs next to
//! the JPEG 2000 decode. Build with `--release`.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use aurora_render::{bc7, build_mips};

#[derive(Default, Clone, Copy)]
struct Sums {
    textures: u32,
    compressed: u32,
    rgba_bytes: u64,
    after_bytes: u64,
    pixels: u64,
    decode: Duration,
    mips: Duration,
    encode: Duration,
}

impl Sums {
    fn add(&mut self, o: &Sums) {
        self.textures += o.textures;
        self.compressed += o.compressed;
        self.rgba_bytes += o.rgba_bytes;
        self.after_bytes += o.after_bytes;
        self.pixels += o.pixels;
        self.decode += o.decode;
        self.mips += o.mips;
        self.encode += o.encode;
    }

    fn print(&self, name: &str) {
        if self.textures == 0 {
            return;
        }
        let mb = |b: u64| b as f64 / 1e6;
        println!(
            "  {name:<8} {:>5} textures, {:>5.1}% compressed; video memory {:>8.1} MB -> {:>8.1} MB ({:.2}x less); \
             decode {:>6.1} s, mips {:>5.1} s, BC7 + check {:>5.1} s ({:.0} Mpx/s, +{:.0}% of the decode)",
            self.textures,
            100.0 * self.compressed as f64 / self.textures as f64,
            mb(self.rgba_bytes),
            mb(self.after_bytes),
            self.rgba_bytes as f64 / self.after_bytes.max(1) as f64,
            self.decode.as_secs_f64(),
            self.mips.as_secs_f64(),
            self.encode.as_secs_f64(),
            self.pixels as f64 / 1e6 / self.encode.as_secs_f64().max(1e-9),
            100.0 * self.encode.as_secs_f64() / self.decode.as_secs_f64().max(1e-9),
        );
    }
}

fn to_rgba(components: usize, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / components * 4);
    for p in data.chunks_exact(components) {
        out.extend_from_slice(&match components {
            1 => [p[0], p[0], p[0], 255],
            2 => [p[0], p[0], p[0], p[1]],
            3 => [p[0], p[1], p[2], 255],
            _ => [p[0], p[1], p[2], p[3]],
        });
    }
    out
}

fn main() {
    let mut a = std::env::args().skip(1);
    let Some(dir) = a.next().map(PathBuf::from) else {
        eprintln!("usage: bc7_check <dir> [--sample N] [--discard D]");
        std::process::exit(2);
    };
    let (mut sample, mut discard) = (1000usize, 0u8);
    while let (Some(k), Some(v)) = (a.next(), a.next()) {
        match k.as_str() {
            "--sample" => sample = v.parse().unwrap_or(sample),
            "--discard" => discard = v.parse().unwrap_or(discard),
            _ => {}
        }
    }
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "j2c"))
            .collect(),
        Err(e) => {
            eprintln!("{}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    files.sort();
    let step = (files.len() / sample.max(1)).max(1);
    // [opaque, alpha, not whole blocks]
    let mut sums = [Sums::default(); 3];
    for path in files.iter().step_by(step) {
        let Ok(data) = std::fs::read(path) else { continue };
        let t = Instant::now();
        let Ok(img) = aurora_assets::decode_j2k(&data, discard) else {
            continue;
        };
        let decode = t.elapsed();
        let c = img.components as usize;
        let rgba = to_rgba(c, &img.data);
        // (the viewer's `classify_alpha`: any texel that is not opaque)
        let alpha = (c == 2 || c >= 4) && rgba.as_chunks::<4>().0.iter().any(|p| p[3] < 255);
        let t = Instant::now();
        let mips = build_mips(img.width, img.height, rgba, 16);
        let mips_time = t.elapsed();
        let rgba_bytes: u64 = mips.iter().map(|m| m.2.len() as u64).sum();
        let t = Instant::now();
        let chain = bc7::encode_chain(&mips, alpha);
        let encode = t.elapsed();
        let whole = img.width.is_multiple_of(4) && img.height.is_multiple_of(4);
        let s = Sums {
            textures: 1,
            compressed: chain.is_some() as u32,
            rgba_bytes,
            after_bytes: chain.as_ref().map_or(rgba_bytes, |c| c.iter().map(|l| l.2.len() as u64).sum()),
            pixels: img.width as u64 * img.height as u64,
            decode,
            mips: mips_time,
            encode,
        };
        sums[if !whole { 2 } else { usize::from(alpha) }].add(&s);
    }
    let mut all = Sums::default();
    for s in &sums {
        all.add(s);
    }
    println!("{} (one texture in {step}, discard {discard}), one thread:", dir.display());
    sums[0].print("opaque");
    sums[1].print("alpha");
    sums[2].print("odd size");
    all.print("all");
}
