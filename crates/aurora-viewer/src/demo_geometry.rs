//! AURORA_DEMO_GEOMETRY=<mb>[,<mb per second>]: geometry load test. As when
//! a crowded region arrives, meshes keep entering the geometry arena until
//! it holds `mb` megabytes of them (vertices and indices), at 25 MB a
//! second unless told otherwise (KB: about 22 MB a second for 35
//! s). The arena outgrows its buffers several times on the way: read the
//! `vertex arena grown to` lines of the log and, with AURORA_PROFILE=1,
//! `frame_max`, `r_acquire` and `r_resources` in `max:` around them. The
//! meshes are not drawn: only the arena's growth is measured, in the frames
//! of the ordinary demo.

use aurora_render::{MeshAlloc, Renderer, Vertex};
use std::time::{Duration, Instant};

/// Not before this frame, nor before this long after the scenario was
/// first driven (the loading screen still covers the view).
const FIRST_FRAME: u64 = 300;
const FIRST_AFTER: Duration = Duration::from_secs(4);
/// One mesh: about the most a face holds with 16-bit indices.
const VERTICES: usize = 60_000;
const INDICES: usize = 180_000;
const MESH_BYTES: u64 = (VERTICES * std::mem::size_of::<Vertex>() + INDICES * 2) as u64;
const MB: f64 = 1024.0 * 1024.0;

pub struct GeometryDemo {
    total: u64,
    /// Bytes a second.
    rate: f64,
    armed: Option<Instant>,
    started: Option<Instant>,
    held: Vec<MeshAlloc>,
    done: bool,
    vertices: Vec<Vertex>,
    indices: Vec<u16>,
}

/// `<mb>[,<mb per second>]`: megabytes to load (800 if not a number), and
/// how fast (25).
fn parse(value: &str) -> (f64, f64) {
    let mut parts = value.split(',').map(|p| p.trim().parse::<f64>().ok().filter(|v| *v > 0.0));
    let total = parts.next().flatten().unwrap_or(800.0);
    let rate = parts.next().flatten().unwrap_or(25.0);
    (total, rate)
}

impl GeometryDemo {
    pub fn from_env() -> Option<Self> {
        let (total, rate) = parse(&std::env::var("AURORA_DEMO_GEOMETRY").ok()?);
        Some(GeometryDemo {
            total: (total * MB) as u64,
            rate: rate * MB,
            armed: None,
            started: None,
            held: Vec::new(),
            done: false,
            vertices: (0..VERTICES)
                .map(|k| Vertex::new([k as f32 * 1e-3, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0]))
                .collect(),
            indices: (0..INDICES).map(|k| (k % VERTICES) as u16).collect(),
        })
    }

    pub fn tick(&mut self, frame: u64, renderer: &mut Renderer) {
        let armed = *self.armed.get_or_insert_with(Instant::now);
        if self.done || frame < FIRST_FRAME || armed.elapsed() < FIRST_AFTER {
            return;
        }
        let started = *self.started.get_or_insert_with(Instant::now);
        let due = ((started.elapsed().as_secs_f64() * self.rate) as u64).min(self.total);
        while self.held.len() as u64 * MESH_BYTES < due {
            let Some(mesh) = renderer.upload_mesh(&self.vertices, &self.indices) else {
                log::warn!("demo geometry: no room left in the arena after {} meshes", self.held.len());
                self.done = true;
                return;
            };
            self.held.push(mesh);
        }
        if self.held.len() as u64 * MESH_BYTES >= self.total {
            self.done = true;
            log::info!(
                "demo geometry: {} meshes, {:.0} MB in the arena after {:.1} s",
                self.held.len(),
                (self.held.len() as u64 * MESH_BYTES) as f64 / MB,
                started.elapsed().as_secs_f64()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn value_gives_the_load_and_its_rate() {
        assert_eq!(super::parse("400"), (400.0, 25.0));
        assert_eq!(super::parse("1200, 100"), (1200.0, 100.0));
        assert_eq!(super::parse("1"), (1.0, 25.0));
        assert_eq!(super::parse("x,-3"), (800.0, 25.0));
    }
}
