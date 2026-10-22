//! Opt-in loopback transport for the Blender add-on. The socket, parsing and
//! mesh encoding live on a dedicated thread; the frame takes only the latest
//! complete snapshot. Session state is deliberately never persisted.

mod addon;
mod demo;
pub mod protocol;
#[cfg(test)]
mod tests;
mod transport;
mod ui;

pub use demo::fixture as demo_fixture;

use parking_lot::Mutex;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

pub const DEFAULT_PORT: u16 = 17655;

#[derive(Default)]
pub struct Status {
    pub connected: bool,
    pub received: u64,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct Shared {
    pub stop: AtomicBool,
    pub status: Mutex<Status>,
    pub latest: Mutex<Option<protocol::Prepared>>,
}

struct Server {
    shared: Arc<Shared>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

pub struct Live {
    pub open: bool,
    pub port: u16,
    server: Option<Server>,
    pub error: Option<String>,
    pub revision: Option<u64>,
    pub triangles: usize,
    /// Render-only objects in a namespace that cannot address a simulator prim.
    pub objects: Vec<Uuid>,
    pub assets: Vec<Uuid>,
    pub clear: bool,
    pub demo_started: bool,
    pub snapshot: Option<protocol::Prepared>,
    pub owner: Option<Uuid>,
    pub addon: addon::Addon,
    pub save_addon: bool,
}
impl Default for Live {
    fn default() -> Self {
        Self {
            open: false,
            port: DEFAULT_PORT,
            server: None,
            error: None,
            revision: None,
            triangles: 0,
            objects: Vec::new(),
            assets: Vec::new(),
            clear: false,
            demo_started: false,
            snapshot: None,
            owner: None,
            addon: Default::default(),
            save_addon: false,
        }
    }
}
impl Live {
    pub fn running(&self) -> bool {
        self.server.is_some()
    }

    pub fn start(&mut self, supported: HashSet<String>, rest_axes: std::collections::HashMap<String, [f32; 16]>, ctx: &egui::Context) {
        if self.running() {
            return;
        }
        match transport::start(self.port, supported, rest_axes, ctx.clone()) {
            Ok(shared) => {
                self.server = Some(Server { shared });
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn stop(&mut self) {
        self.server = None;
        self.clear = true;
    }

    pub fn take_latest(&mut self) -> Option<protocol::Prepared> {
        self.server.as_ref()?.shared.latest.lock().take()
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) -> bool {
        self.addon.poll();
        ui::show(self, ctx, p)
    }
}
