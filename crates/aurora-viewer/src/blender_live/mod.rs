//! Opt-in loopback transport for the Blender add-on. The socket, parsing and
//! mesh encoding live on a dedicated thread; the frame takes only the latest
//! complete snapshot. Session state is deliberately never persisted.

mod addon;
mod demo;
pub mod fit_lab;
mod geometry;
pub(crate) mod import;
mod installer;
mod links;
pub mod objects;
pub mod protocol;
pub(crate) mod shape;
#[cfg(test)]
mod tests;
mod thumbnail;
mod token;
mod transport;
mod ui;

pub use demo::fixture as demo_fixture;
pub use demo::sleeves_fixture as demo_sleeves_fixture;
pub use demo::static_fixture as demo_static_fixture;
pub use demo::warnings_fixture as demo_warnings_fixture;
pub(crate) use installer::{Removal as AddonRemoval, remove_everywhere as remove_addon_everywhere};

use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

pub const DEFAULT_PORT: u16 = 17655;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tab {
    #[default]
    Objects,
    Place,
    Avatar,
    Import,
    Plugins,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceState {
    #[default]
    Visible,
    Hidden,
    Removed,
}

#[derive(Default)]
pub struct Status {
    pub connected: bool,
    pub shape_capable: bool,
    pub shape_apply_capable: bool,
    pub import_capable: bool,
    pub received: u64,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct Shared {
    pub stop: AtomicBool,
    pub status: Mutex<Status>,
    pub latest: Mutex<Option<protocol::Prepared>>,
    pub shape: Mutex<Option<shape::Outgoing>>,
    pub shape_ack: Mutex<Option<(Uuid, Option<String>)>>,
    pub shape_cancel: Mutex<Option<Uuid>>,
    pub import_request: Mutex<Option<import::Request>>,
    pub import_expected: Mutex<Option<import::Request>>,
    pub import_result: Mutex<Option<(Uuid, Result<import::Bundle, String>)>>,
}

struct Server {
    shared: Arc<Shared>,
    // Dropped with the server: the add-on can no longer connect once
    // stopped. Only the transport tests run without one.
    _token: Option<token::File>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}

pub struct Live {
    pub open: bool,
    pub(super) tab: Tab,
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
    pub shape: shape::State,
    pub import: import::State,
    pub(super) installer: installer::Installer,
    pub(super) installer_scan: bool,
    pub(super) installer_choose: bool,
    pub(super) installer_target: Option<std::path::PathBuf>,
    pub save_addon: bool,
    pub start_requested: bool,
    pub states: HashMap<Uuid, SourceState>,
    pub selected: Option<Uuid>,
    pub upload_requested: Option<Uuid>,
    pub placements: HashMap<Uuid, objects::Placement>,
    pub instances: HashMap<(Uuid, usize), Uuid>,
    pub object_action: Option<(Uuid, objects::Action)>,
    pub edit_linked: bool,
    pub world_origin: Option<(u32, u32)>,
    pub next_local_id: u32,
    pub attach_choices: Vec<(u8, bool, String)>,
    pub placement_dirty: HashSet<Uuid>,
    /// The Avatar tab's elbow and knee bend, while the preview runs.
    pub bend: crate::scene::anim::PreviewBend,
    /// The Avatar tab's Fit lab (`fit_lab.rs`).
    pub fit: fit_lab::FitLab,
    /// When each source last changed (`PreparedSource::digest`), and when
    /// the last snapshot came: the cards' and the status line's times.
    pub changed: HashMap<Uuid, (u64, std::time::Instant)>,
    pub received_at: Option<std::time::Instant>,
    /// The cards' live thumbnails (`thumbnail.rs`).
    thumbnails: thumbnail::Thumbnails,
    /// The source shown in Quick look, and its camera.
    quick_look: Option<Uuid>,
    quick_view: aurora_render::preview::View,
    /// Quick look's pose: the Avatar tab's bend it was made for (degrees),
    /// and the skeleton bent (`thumbnail::bent_pose`).
    quick_pose: ((u32, u32), Option<aurora_render::preview::Pose>),
    /// The last placement change, while its Undo toast shows.
    pub undo: Option<objects::Undo>,
    /// Set by the toast's Undo; applied with the frame's placement changes.
    pub undo_requested: bool,
    /// Reduce animations: the thumbnails' update sweep stands still.
    pub still: bool,
}
impl Default for Live {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::Objects,
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
            shape: Default::default(),
            import: Default::default(),
            installer: Default::default(),
            installer_scan: false,
            installer_choose: false,
            installer_target: None,
            save_addon: false,
            start_requested: false,
            states: HashMap::new(),
            selected: None,
            upload_requested: None,
            placements: HashMap::new(),
            instances: HashMap::new(),
            object_action: None,
            edit_linked: false,
            world_origin: None,
            next_local_id: 1,
            attach_choices: Vec::new(),
            placement_dirty: HashSet::new(),
            bend: Default::default(),
            fit: Default::default(),
            changed: HashMap::new(),
            received_at: None,
            thumbnails: Default::default(),
            quick_look: None,
            quick_view: Default::default(),
            quick_pose: ((0, 0), None),
            undo: None,
            undo_requested: false,
            still: false,
        }
    }
}
impl Live {
    pub(crate) fn shape_connection(&self) -> Result<Arc<Shared>, String> {
        let shared = self
            .server
            .as_ref()
            .map(|server| &server.shared)
            .filter(|shared| shared.status.lock().connected)
            .ok_or_else(|| aurora_i18n::tr!("blender-shape-connect").to_owned())?;
        if !shared.status.lock().shape_capable {
            return Err(aurora_i18n::tr!("blender-shape-update-plugin").into());
        }
        Ok(shared.clone())
    }

    pub(crate) fn shape_apply_connection(&self) -> Result<Arc<Shared>, String> {
        let shared = self.shape_connection()?;
        if !shared.status.lock().shape_apply_capable {
            return Err(aurora_i18n::tr!("blender-shape-update-apply").into());
        }
        Ok(shared)
    }
    pub fn state(&self, id: Uuid) -> SourceState {
        self.states.get(&id).copied().unwrap_or_default()
    }

    pub fn set_state(&mut self, id: Uuid, state: SourceState) {
        self.states.insert(id, state);
        self.revision = None;
        if state == SourceState::Removed && self.selected == Some(id) {
            self.selected = None;
        } else if state == SourceState::Visible && self.selected.is_none() {
            self.selected = Some(id);
        }
    }

    pub fn receive(&mut self, snapshot: protocol::Prepared) -> Option<protocol::Prepared> {
        self.states.retain(|id, _| snapshot.sources.iter().any(|source| source.id == *id));
        let now = std::time::Instant::now();
        self.received_at = Some(now);
        self.changed.retain(|id, _| snapshot.sources.iter().any(|source| source.id == *id));
        for source in &snapshot.sources {
            match self.changed.get_mut(&source.id) {
                Some(entry) if entry.0 == source.digest => {}
                Some(entry) => *entry = (source.digest, now),
                None => {
                    self.changed.insert(source.id, (source.digest, now));
                }
            }
        }
        // Pausing a Blender entry temporarily omits it from the wire. Keep a
        // bounded session history so resuming cannot rez a detached source.
        self.placements
            .retain(|id, _| !snapshot.sources.iter().any(|source| source.id == *id && source.rigged()));
        if self.placements.len() > 512 {
            self.placements
                .retain(|id, _| snapshot.sources.iter().any(|source| source.id == *id));
        }
        if self
            .selected
            .is_none_or(|id| !snapshot.sources.iter().any(|s| s.id == id) || self.state(id) == SourceState::Removed)
        {
            self.selected = snapshot
                .sources
                .iter()
                .find(|s| self.state(s.id) != SourceState::Removed)
                .map(|s| s.id);
        }
        self.revision = None;
        self.snapshot.replace(snapshot)
    }

    /// After a GPU recovery (app/gpu_recovery.rs): the images of the snapshot
    /// shown were staged in the lost device's pool. They are staged again
    /// from the pixels kept with them into `pool`, and the snapshot is
    /// installed again at the next frame; nothing is asked of Blender.
    pub fn gpu_reset(&mut self, pool: Option<&aurora_render::StagingPool>) {
        self.revision = None;
        let Some(snapshot) = self.snapshot.as_mut() else {
            return;
        };
        for image in &mut snapshot.images {
            let pixels = &image.pixels;
            let levels = aurora_render::build_mips(pixels.width, pixels.height, pixels.data.clone(), 12);
            let refs: Vec<_> = levels.iter().map(|(w, h, bytes)| (*w, *h, bytes.as_slice())).collect();
            image.staged = pool
                .and_then(|pool| aurora_render::StagedTexture::new(pool, &refs))
                .map(std::sync::Arc::new);
        }
    }

    pub fn visible_snapshot(&self) -> Option<protocol::Prepared> {
        let mut snapshot = self.snapshot.clone()?;
        snapshot.sources.retain(|s| self.state(s.id) == SourceState::Visible);
        snapshot.meshes.retain(|m| {
            self.state(m.source) == SourceState::Visible
                && self.state(self.linked_root(m.source)) == SourceState::Visible
                && (m.skin.is_some()
                    || self
                        .placements
                        .get(&self.linked_root(m.source))
                        .is_none_or(|p| p.location != objects::Location::Detached))
        });
        snapshot.triangles = snapshot
            .sources
            .iter()
            .filter(|s| snapshot.meshes.iter().any(|m| m.source == s.id))
            .flat_map(|s| &s.scene.models)
            .map(|m| m.triangles())
            .sum();
        let textures: HashSet<_> = snapshot
            .meshes
            .iter()
            .flat_map(|m| &m.materials)
            .flatten()
            .flat_map(|(_, m)| {
                [
                    m.base_color_texture,
                    m.normal_texture,
                    m.metallic_roughness_texture,
                    m.emissive_texture,
                ]
                .into_iter()
                .flatten()
            })
            .collect();
        snapshot.images.retain(|image| textures.contains(&image.id));
        Some(snapshot)
    }

    pub fn running(&self) -> bool {
        self.server.is_some()
    }

    pub fn start(
        &mut self,
        supported: HashSet<String>,
        rest_axes: std::collections::HashMap<String, [f32; 16]>,
        ctx: &egui::Context,
        pool: Arc<aurora_render::StagingPool>,
    ) {
        if self.running() {
            return;
        }
        // The secret is written before the port opens, so the add-on never
        // meets a listening preview with a stale file.
        let token = token::folder()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
            .and_then(|folder| token::File::write(&folder, self.port));
        let started = token.and_then(|token| {
            let shared = transport::start(self.port, token.secret().to_owned(), supported, rest_axes, ctx.clone(), Some(pool))?;
            Ok(Server {
                shared,
                _token: Some(token),
            })
        });
        match started {
            Ok(server) => {
                self.server = Some(server);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    pub fn stop(&mut self) {
        self.import = Default::default();
        self.received_at = None;
        self.changed.clear();
        self.thumbnails.clear();
        self.quick_look = None;
        self.undo = None;
        self.undo_requested = false;
        self.start_requested = false;
        self.server = None;
        self.clear = true;
    }

    /// Builds the thumbnails of the objects that changed, on the job pool;
    /// only while the window is open (closed, nothing is drawn).
    pub fn update_thumbnails(
        &mut self,
        jobs: &crate::scene::jobs::Jobs,
        lib: &std::sync::Arc<crate::scene::avatar::AvatarLibrary>,
        ctx: &egui::Context,
    ) {
        if !self.open {
            return;
        }
        self.thumbnails.set_rig(lib);
        // Quick look bends its rigged parts as the Avatar tab bends ours
        let key = (self.bend.elbows.to_bits(), self.bend.knees.to_bits());
        if self.quick_pose.0 != key {
            self.quick_pose = (key, thumbnail::bent_pose(&lib.rig, self.bend.elbows, self.bend.knees));
        }
        let Some(snapshot) = &self.snapshot else {
            self.thumbnails.clear();
            return;
        };
        let groups = thumbnail::Group::of(&snapshot.sources, &self.changed, |id| self.state(id) != SourceState::Removed);
        let wait = self.thumbnails.update(&groups, &snapshot.images, std::time::Instant::now(), |job| {
            let ctx = ctx.clone();
            jobs.execute(move || {
                job();
                ctx.request_repaint();
            });
        });
        if let Some(wait) = wait {
            ctx.request_repaint_after(wait);
        }
    }

    /// Opens Quick look on `source` (the demo's `quick-look` tab).
    pub fn open_quick_look(&mut self, source: Uuid) {
        self.quick_look = Some(source);
    }

    pub fn take_latest(&mut self) -> Option<protocol::Prepared> {
        self.server.as_ref()?.shared.latest.lock().take()
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, can_upload: bool) -> bool {
        self.addon.poll();
        self.installer.poll();
        ui::show(self, ctx, p, can_upload)
    }
}
