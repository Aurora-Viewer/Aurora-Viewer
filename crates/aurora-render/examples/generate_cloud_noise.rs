//! Writes `assets/clouds/cloud_noise.bin`, the volumetric clouds' noise
//! textures (see `aurora_render::clouds_noise`). CPU only, no seed other than
//! the hash functions' constants: the same file every time.
//!
//! ```text
//! cargo run -p aurora-render --example generate_cloud_noise [-- out.bin]
//! ```
use aurora_render::clouds_noise::{BASE_SIZE, Noise, base_layer, curl, detail, pack};
use rayon::prelude::*;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "assets/clouds/cloud_noise.bin".into());
    let noise = Noise {
        base: (0..BASE_SIZE).into_par_iter().flat_map_iter(base_layer).collect(),
        detail: detail(),
        curl: curl(),
    };
    let bytes = pack(&noise);
    std::fs::write(&out, &bytes).expect("write the file");
    println!("{out}: {} bytes", bytes.len());
}
