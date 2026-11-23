//! Replay a directory of synthetic reverse-Z captures through the same Bend
//! dispatch and filter used by the glasses bridge regression.
use super::{Gpu, Mat4, Scene, Vec3, Vec4};
use std::io;
use std::path::Path;

pub(super) fn run(directory: &Path) -> io::Result<()> {
    let (width, height) = (384, 288);
    let focal = 1.0 / 30.0_f32.to_radians().tan();
    let projection = Mat4::from_cols(
        Vec4::new(focal * height as f32 / width as f32, 0., 0., 0.),
        Vec4::new(0., focal, 0., 0.),
        Vec4::new(0., 0., 0., -1.),
        Vec4::new(0., 0., 0.01, 0.),
    );
    let (gpu, _) = Gpu::new();
    let mut replayed = 0;
    for entry in directory.read_dir()? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("raw") {
            continue;
        }
        let raw = std::fs::read(&path)?;
        let pixels = (width * height) as usize;
        if raw.len() != 4 * (3 + pixels) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected grid size in {}", path.display()),
            ));
        }
        let mut floats = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()));
        let light = Vec3::new(
            floats.next().unwrap(),
            floats.next().unwrap(),
            floats.next().unwrap(),
        );
        let scene = Scene {
            name: String::new(),
            w: width,
            h: height,
            proj: projection,
            depth: floats.collect(),
            receiver: Vec::new(),
            light,
        };
        let (_, filtered) = gpu.run(&scene, 0.05);
        let occlusion: Vec<u8> = filtered.iter().map(|p| p[0]).collect();
        std::fs::write(path.with_extension("bend"), occlusion)?;
        replayed += 1;
    }
    if replayed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No .raw depth grids",
        ));
    }
    Ok(())
}
