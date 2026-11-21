//! Object › Save as › Collada / OBJ / glTF on the app's side: the window
//! opened on the menu's selection, the rights read again as properties,
//! shapes and texture comments arrive, then Save as: the file dialog, the
//! shapes gathered and the file written on a worker, the textures beside it
//! (for glTF, fetched first and written inside the file).
//! Port of ColladaExportFloater (indra/newview/daeexport.cpp, Latif Khalifa
//! and Cinder Biscuits, LGPL 2.1); the rules are in `build/export/`.

use super::App;
use crate::build::export::gather::{self, PrimInput, Waiting};
use crate::build::export::gltf::Embedded;
use crate::build::export::rights::{self, Context};
use crate::build::export::textures::TextureSaver;
use crate::build::export::{Format, Settings, TextureFiles, collada, gltf, model, obj};
use crate::build::ui::export::{Action, Progress, Summary};
use crate::world::inventory::actions::ExportGrid;
use crate::world::objects::ObjKey;
use crossbeam_channel::Receiver;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How often the window reads the rights again while it is open.
const REFRESH: Duration = Duration::from_millis(250);
/// A prim's properties are asked again after this.
const ASK_AGAIN: Duration = Duration::from_secs(10);
/// A texture's comment (its uploader) not read by then counts as absent.
const COMMENT_WAIT: Duration = Duration::from_secs(15);
/// The shapes not held by then are left out of the file.
const SHAPES_WAIT: Duration = Duration::from_secs(30);

/// The export's state on the app's side.
#[derive(Default)]
pub(super) struct Job {
    /// The prims of the menu's selection, the first root first.
    prims: Vec<ObjKey>,
    /// Those the rights allow, as of the last refresh.
    allowed: Vec<ObjKey>,
    /// The textures we may save and their file names.
    files: Vec<(Uuid, String)>,
    /// Texture and sculpt map uploaders read from their J2C comments
    /// (None: no comment, or not read in time).
    comments: HashMap<Uuid, Option<Uuid>>,
    reading: HashMap<Uuid, (Instant, Option<Receiver<Option<Uuid>>>)>,
    asked: HashMap<ObjKey, Instant>,
    refreshed: Option<Instant>,
    stage: Stage,
}

#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    Choosing(Receiver<Option<PathBuf>>),
    Gathering {
        path: PathBuf,
        since: Instant,
        inputs: HashMap<ObjKey, PrimInput>,
    },
    /// glTF: the textures fetched into memory before the file is written.
    Embedding {
        path: PathBuf,
        inputs: HashMap<ObjKey, PrimInput>,
        saver: TextureSaver,
    },
    Writing {
        path: PathBuf,
        reply: Receiver<Result<(), String>>,
        textures: Option<TextureSaver>,
        /// Embedded textures that did not arrive.
        missing: usize,
    },
    Textures {
        path: PathBuf,
        saver: TextureSaver,
    },
}

impl Job {
    pub(super) fn wanted_images(&mut self) -> impl Iterator<Item = Uuid> + '_ {
        let saving = match &mut self.stage {
            Stage::Textures { saver, .. } | Stage::Embedding { saver, .. } => Some(saver.wanted_images.drain()),
            _ => None,
        };
        saving.into_iter().flatten()
    }
}

impl App {
    /// The rules' grid: Second Life, or OpenSim with or without the
    /// region's export (LFSimFeatureHandler::exportPolicy).
    fn export_grid(&self) -> ExportGrid {
        if self.second_life() {
            ExportGrid::SecondLife
        } else {
            ExportGrid::OpenSim {
                export_supported: self.world.export_supported(),
            }
        }
    }

    /// Object.ExportCollada (FSObjectExportCollada), and its OBJ sibling:
    /// the window on the menu's selection.
    pub(super) fn open_object_export(&mut self, id: Uuid, format: Format) {
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        if self.object_export.progress.is_some() {
            crate::ui::widgets::open_floater(&self.egui_ctx, crate::build::ui::export::WINDOW, &mut self.object_export.open);
            return;
        }
        let prims = gather::prims_of(&self.build, &self.world, idx);
        let root = prims
            .first()
            .and_then(|k| self.world.objects.index_of(k))
            .and_then(|i| self.world.objects.get(i));
        let name = root
            .and_then(|o| self.object_export_props(o.full_id))
            .map(|p| p.name)
            .unwrap_or_default();
        self.object_export_job = Job {
            prims,
            ..Default::default()
        };
        if self.settings.build.export.format != format {
            self.settings.build.export.format = format;
            self.settings.save();
        }
        self.object_export.name = name;
        self.object_export.summary = Summary::default();
        self.object_export.notice = None;
        self.object_export.progress = None;
        crate::ui::widgets::open_floater(&self.egui_ctx, crate::build::ui::export::WINDOW, &mut self.object_export.open);
        self.refresh_object_export(Instant::now());
    }

    /// The full properties of a prim, from the selection or the menu's.
    fn object_export_props(&self, id: Uuid) -> Option<aurora_net::build::ObjectProps> {
        self.build
            .props
            .get(&id)
            .filter(|p| p.full)
            .or_else(|| self.interactions.props.get(&id).filter(|p| p.full))
            .cloned()
    }

    pub(super) fn object_export_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.object_export.open && self.object_export.progress.is_none() {
            return;
        }
        let now = Instant::now();
        if self.object_export.progress.is_none() && self.object_export_job.refreshed.is_none_or(|t| now.duration_since(t) >= REFRESH) {
            self.refresh_object_export(now);
        }
        self.advance_object_export(ctx, now);
        let mut settings = self.settings.build.export.clone();
        let action = self.object_export.show(ctx, p, &mut settings, &self.ui_images);
        if settings != self.settings.build.export {
            self.settings.build.export = settings;
            self.settings.save();
        }
        if action == Some(Action::Save) {
            self.object_export_choose_file(ctx);
        }
    }

    /// ColladaExportFloater::addSelectedObjects and updateTextureInfo: the
    /// prims and textures the rules let through, asking what is missing.
    fn refresh_object_export(&mut self, now: Instant) {
        let job = &mut self.object_export_job;
        job.refreshed = Some(now);
        self.collect_comments(now);
        let grid = self.export_grid();
        let props = |id: Uuid| self.object_export_props(id);
        // every texture and sculpt map of the selection, its items read in
        // one pass of the inventory
        let assets: HashSet<Uuid> = self
            .object_export_job
            .prims
            .iter()
            .filter_map(|k| self.world.objects.index_of(k).and_then(|i| self.world.objects.get(i)))
            .flat_map(|o| {
                let map = o.volume.sculpt.filter(|s| !s.is_mesh()).map(|s| s.texture);
                let faces = o.te.iter().flat_map(|te| te.faces.iter().map(|f| f.texture));
                map.into_iter().chain(faces)
            })
            .collect();
        let cx = Context::new(
            self.world.agent_id,
            grid,
            self.world.prim_limits().max,
            &self.world.inventory,
            &assets,
        );
        let mut total = 0;
        let mut waiting = false;
        let mut allowed = Vec::new();
        let mut maps = HashSet::new();
        let mut meshes = HashSet::new();
        let mut ask = Vec::new();
        for key in &self.object_export_job.prims {
            let Some(facts) = gather::facts(&self.world, &self.scene, *key, &props, &self.object_export_job.comments) else {
                continue;
            };
            total += 1;
            let facts = match facts {
                Ok(facts) => facts,
                Err(reason) => {
                    waiting = true;
                    match reason {
                        Waiting::Properties => ask.push(*key),
                        Waiting::Sculpt | Waiting::Mesh => {
                            if let Some(s) = self
                                .world
                                .objects
                                .index_of(key)
                                .and_then(|i| self.world.objects.get(i))
                                .and_then(|o| o.volume.sculpt)
                            {
                                if s.is_mesh() {
                                    meshes.insert(s.texture);
                                } else {
                                    maps.insert(s.texture);
                                }
                            }
                        }
                    }
                    continue;
                }
            };
            // a mesh whose header is not read yet: asked for, counted later
            if let rights::Shape::Mesh { creator: None } = facts.shape {
                waiting = true;
                if let Some(s) = self
                    .world
                    .objects
                    .index_of(key)
                    .and_then(|i| self.world.objects.get(i))
                    .and_then(|o| o.volume.sculpt)
                {
                    meshes.insert(s.texture);
                }
            }
            if rights::prim(&cx, &facts) {
                allowed.push(*key);
            }
        }
        // the textures of the prims we may export (updateTextureInfo)
        let mut textures: Vec<Uuid> = Vec::new();
        for key in &allowed {
            if let Some(o) = self.world.objects.index_of(key).and_then(|i| self.world.objects.get(i))
                && let Some(te) = &o.te
            {
                let count = match o.volume.sculpt {
                    Some(s) if s.is_mesh() => aurora_prim::te::MAX_TES,
                    _ => aurora_prim::volume::num_faces(&o.volume).max(1),
                };
                for i in 0..count.min(te.faces.len()) {
                    let id = te.face(i).texture;
                    if !textures.contains(&id) {
                        textures.push(id);
                    }
                }
            }
        }
        let mut files = Vec::new();
        let mut unread = Vec::new();
        for id in &textures {
            let uploader = match self.object_export_job.comments.get(id) {
                Some(u) => *u,
                None => {
                    unread.push(*id);
                    None
                }
            };
            if let Some(name) = rights::texture(&cx, *id, uploader) {
                files.push((*id, name));
            }
        }
        let summary = Summary {
            total,
            allowed: allowed.len(),
            waiting,
            textures_total: textures.len(),
            textures: {
                let names = TextureFiles::new(files.clone(), self.settings.build.export.texture_format);
                let mut list: Vec<(Uuid, String)> = names.names.into_iter().collect();
                list.sort_by(|a, b| a.1.cmp(&b.1));
                list
            },
        };
        self.object_export.summary = summary;
        // the root's name once its properties are here (NameText)
        if self.object_export.name.is_empty()
            && let Some(name) = self
                .object_export_job
                .prims
                .first()
                .and_then(|k| self.world.objects.index_of(k))
                .and_then(|i| self.world.objects.get(i))
                .and_then(|o| self.object_export_props(o.full_id))
                .map(|p| p.name)
        {
            self.object_export.name = name;
        }
        for id in meshes {
            self.scene.meshes.want(id, 3);
        }
        let job = &mut self.object_export_job;
        job.allowed = allowed;
        job.files = files;
        // the comments matter on Second Life only
        let comments_wanted: Vec<Uuid> = if grid == ExportGrid::SecondLife {
            maps.into_iter().chain(unread).collect()
        } else {
            for id in maps.into_iter().chain(unread) {
                job.comments.insert(id, None);
            }
            Vec::new()
        };
        for id in comments_wanted {
            self.read_comment(id, now);
        }
        for key in ask {
            let again = self
                .object_export_job
                .asked
                .get(&key)
                .is_none_or(|t| now.duration_since(*t) >= ASK_AGAIN);
            if again {
                self.object_export_job.asked.insert(key, now);
                for command in self.build.prim_properties_query(&self.world, key) {
                    self.send(command);
                }
            }
        }
    }

    /// Read a texture's J2C comment (its uploader, "a=") on a worker once
    /// its first bytes are here.
    fn read_comment(&mut self, id: Uuid, now: Instant) {
        if self.object_export_job.comments.contains_key(&id) {
            return;
        }
        let entry = self.object_export_job.reading.entry(id).or_insert((now, None));
        if entry.1.is_some() {
            return;
        }
        if now.duration_since(entry.0) > COMMENT_WAIT {
            self.object_export_job.reading.remove(&id);
            self.object_export_job.comments.insert(id, None);
            return;
        }
        let source = self.scene.textures.preview_source(&id);
        if source.local.is_some() || source.procedural {
            // made by the viewer: no comment
            self.object_export_job.reading.remove(&id);
            self.object_export_job.comments.insert(id, None);
            return;
        }
        if source.bytes.is_empty() && source.decoded.is_none() {
            self.object_export.wanted_images.insert(id);
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        entry.1 = Some(rx);
        let wake = self.egui_ctx.clone();
        self.scene.jobs.execute(move || {
            let bytes = if source.bytes.is_empty() {
                crate::cache::read(&source.jpeg).ok().map(std::sync::Arc::new)
            } else {
                Some(source.bytes.clone())
            };
            let uploader = bytes.and_then(|b| crate::ui::texture_library::texture_view::metadata(&b).uploader);
            let _ = tx.send(uploader);
            wake.request_repaint();
        });
    }

    fn collect_comments(&mut self, now: Instant) {
        let job = &mut self.object_export_job;
        let mut done = Vec::new();
        for (id, (since, rx)) in &job.reading {
            match rx.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok(uploader)) => done.push((*id, uploader)),
                Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => done.push((*id, None)),
                _ if now.duration_since(*since) > COMMENT_WAIT => done.push((*id, None)),
                _ => {}
            }
        }
        for (id, uploader) in done {
            job.reading.remove(&id);
            job.comments.insert(id, uploader);
        }
    }

    /// onClickExport: the file dialog, off the frame thread.
    fn object_export_choose_file(&mut self, ctx: &egui::Context) {
        if !matches!(self.object_export_job.stage, Stage::Idle) || self.object_export_job.allowed.is_empty() {
            return;
        }
        let format = self.settings.build.export.format;
        let name = format!("{}.{}", rights::file_stem(&self.object_export.name), format.extension());
        let (filter, title) = match format {
            Format::Collada => (
                aurora_i18n::tr!("object-export-collada-files"),
                aurora_i18n::tr!("object-export-collada"),
            ),
            Format::Obj => (aurora_i18n::tr!("object-export-obj-files"), aurora_i18n::tr!("object-export-obj")),
            Format::Gltf => (aurora_i18n::tr!("object-export-gltf-files"), aurora_i18n::tr!("object-export-gltf")),
        };
        let (filter, title) = (filter.to_owned(), title.to_owned());
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("object-export-dialog".into()).spawn(move || {
            let path = rfd::FileDialog::new()
                .set_title(title)
                .add_filter(filter, &[format.extension()])
                .set_file_name(name)
                .save_file();
            let _ = tx.send(path);
            wake.request_repaint();
        });
        if let Err(e) = spawned {
            log::warn!("object export: no file dialog: {e}");
            return;
        }
        self.object_export_job.stage = Stage::Choosing(rx);
        self.object_export.progress = Some(Progress::Choosing);
        self.object_export.notice = None;
    }

    /// The steps after the file is chosen, a little each frame.
    fn advance_object_export(&mut self, ctx: &egui::Context, now: Instant) {
        let stage = std::mem::take(&mut self.object_export_job.stage);
        self.object_export_job.stage = match stage {
            Stage::Idle => Stage::Idle,
            Stage::Choosing(rx) => match rx.try_recv() {
                Ok(Some(path)) => {
                    self.object_export.progress = Some(Progress::Writing);
                    Stage::Gathering {
                        path,
                        since: now,
                        inputs: HashMap::new(),
                    }
                }
                Ok(None) | Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.object_export.progress = None;
                    Stage::Idle
                }
                Err(crossbeam_channel::TryRecvError::Empty) => Stage::Choosing(rx),
            },
            Stage::Gathering { path, since, mut inputs } => {
                for key in self.object_export_job.allowed.clone() {
                    if inputs.contains_key(&key) {
                        continue;
                    }
                    if let Ok(input) = gather::input(&self.world, &mut self.scene, key, now) {
                        inputs.insert(key, input);
                    }
                }
                let complete = inputs.len() == self.object_export_job.allowed.len();
                if complete || now.duration_since(since) > SHAPES_WAIT {
                    if !complete {
                        log::warn!(
                            "object export: {} of {} shapes not loaded in time, left out",
                            self.object_export_job.allowed.len() - inputs.len(),
                            self.object_export_job.allowed.len()
                        );
                    }
                    self.object_export_write(ctx, path, inputs)
                } else {
                    Stage::Gathering { path, since, inputs }
                }
            }
            Stage::Embedding { path, inputs, mut saver } => {
                saver.poll(ctx, &mut self.scene, now);
                self.object_export.progress = Some(Progress::Textures {
                    left: saver.total - saver.written - saver.failed.len(),
                    total: saver.total,
                });
                if saver.done() {
                    self.object_export.progress = Some(Progress::Writing);
                    let missing = saver.failed.len();
                    self.object_export_write_file(ctx, path, inputs, saver.embedded, missing)
                } else {
                    Stage::Embedding { path, inputs, saver }
                }
            }
            Stage::Writing {
                path,
                reply,
                textures,
                missing,
            } => match reply.try_recv() {
                Ok(Ok(())) => match textures {
                    Some(saver) => Stage::Textures { path, saver },
                    None => {
                        self.object_export_done(&path, Ok(missing));
                        Stage::Idle
                    }
                },
                Ok(Err(reason)) => {
                    self.object_export_done(&path, Err(reason));
                    Stage::Idle
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.object_export_done(&path, Err("stopped".into()));
                    Stage::Idle
                }
                Err(crossbeam_channel::TryRecvError::Empty) => Stage::Writing {
                    path,
                    reply,
                    textures,
                    missing,
                },
            },
            Stage::Textures { path, mut saver } => {
                saver.poll(ctx, &mut self.scene, now);
                self.object_export.progress = Some(Progress::Textures {
                    left: saver.total - saver.written - saver.failed.len(),
                    total: saver.total,
                });
                if saver.done() {
                    self.object_export_done(&path, Ok(saver.failed.len()));
                    Stage::Idle
                } else {
                    Stage::Textures { path, saver }
                }
            }
        };
    }

    /// The textures this export may save, named for their files.
    fn object_export_files(&self, settings: &Settings) -> TextureFiles {
        TextureFiles::new(
            if settings.textures {
                self.object_export_job.files.clone()
            } else {
                Vec::new()
            },
            settings.texture_format,
        )
    }

    /// The shapes are here: a glTF file first fetches the textures it holds,
    /// the other formats are written at once.
    fn object_export_write(&mut self, ctx: &egui::Context, path: PathBuf, inputs: HashMap<ObjKey, PrimInput>) -> Stage {
        let settings = &self.settings.build.export;
        let files = self.object_export_files(settings);
        if settings.format == Format::Gltf && settings.textures && !files.names.is_empty() {
            let saver = TextureSaver::embed(&files);
            self.object_export.progress = Some(Progress::Textures {
                left: saver.total,
                total: saver.total,
            });
            return Stage::Embedding { path, inputs, saver };
        }
        self.object_export_write_file(ctx, path, inputs, HashMap::new(), 0)
    }

    /// DAESaver::saveDAE and its OBJ and glTF siblings, on a worker: the
    /// shapes made at their highest detail, the model, the file(s).
    fn object_export_write_file(
        &mut self,
        ctx: &egui::Context,
        path: PathBuf,
        mut inputs: HashMap<ObjKey, PrimInput>,
        images: HashMap<uuid::Uuid, Embedded>,
        missing: usize,
    ) -> Stage {
        let settings: Settings = self.settings.build.export.clone();
        // the file's prims in the selection's order, the first root first
        let ordered: Vec<PrimInput> = self.object_export_job.allowed.iter().filter_map(|k| inputs.remove(k)).collect();
        let format = settings.texture_format;
        let files = self.object_export_files(&settings);
        let tool = format!("Aurora Viewer {}", aurora_version::version());
        let author = Some(self.world.own_name())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Unknown".into());
        let date = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        let beside = settings.format != Format::Gltf && settings.textures && !files.names.is_empty();
        let textures = beside.then(|| TextureSaver::new(&files, &path, format));
        let (tx, rx) = crossbeam_channel::bounded(1);
        let wake = ctx.clone();
        let target = path.clone();
        let spawned = std::thread::Builder::new().name("object-export".into()).spawn(move || {
            use rayon::prelude::*;
            let prims: Vec<model::SourcePrim> = ordered.into_par_iter().map(gather::generate).collect();
            let model = model::build(&prims, &settings);
            let result = match settings.format {
                Format::Collada => {
                    let header = collada::Header { author, tool, date };
                    let text = collada::write(&model, &files, &settings, &header);
                    crate::atomic_file::write(&target, text.as_bytes())
                }
                Format::Obj => {
                    let mtl = target.with_extension("mtl");
                    let mtl_name = mtl.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    crate::atomic_file::write(&mtl, obj::write_mtl(&model, &files, &settings, &tool).as_bytes())
                        .and_then(|()| crate::atomic_file::write(&target, obj::write_obj(&model, &mtl_name, &tool).as_bytes()))
                }
                Format::Gltf => crate::atomic_file::write(&target, &gltf::write_glb(&model, &files, &images, &settings, &tool)),
            };
            log::info!(
                "object export: {} prims, {} triangles, {} materials",
                model.prims.len(),
                model.triangles(),
                model.materials.len()
            );
            let _ = tx.send(result.map_err(|e| e.to_string()));
            wake.request_repaint();
        });
        if let Err(e) = spawned {
            self.object_export_done(&path, Err(e.to_string()));
            return Stage::Idle;
        }
        Stage::Writing {
            path,
            reply: rx,
            textures,
            missing,
        }
    }

    /// onTexturesSaved: ExportColladaSuccess or ExportColladaFailure, and
    /// the window closes after a success.
    fn object_export_done(&mut self, path: &std::path::Path, result: Result<usize, String>) {
        use crate::world::notifications::{Data, Kind};
        self.object_export.progress = None;
        let object = self.object_export.name.clone();
        let file = path.display().to_string();
        match result {
            Ok(missing) => {
                log::info!("object export: saved");
                let mut body = aurora_i18n::tr!("object-export-success", object = object.as_str(), file = file.as_str());
                if missing > 0 {
                    body.push(' ');
                    body.push_str(&aurora_i18n::tr!("object-export-textures-missing", count = missing));
                }
                self.world
                    .notifications
                    .push(Kind::Info, aurora_i18n::tr!("object-export-title").to_owned(), body, Data::None);
                self.object_export.open = false;
            }
            Err(reason) => {
                log::warn!("object export failed: {reason}");
                let body = aurora_i18n::tr!("object-export-failure", object = object.as_str(), file = file.as_str());
                self.object_export.notice = Some(body.clone());
                self.world
                    .notifications
                    .push(Kind::Alert, aurora_i18n::tr!("object-export-title").to_owned(), body, Data::None);
            }
        }
    }
}

impl App {
    /// AURORA_DEMO_OBJECT_EXPORT (collada, obj or gltf): the garden lamp before us, then the window
    /// on it; with
    /// AURORA_DEMO_OBJECT_EXPORT_FILE, Save as to that file without the
    /// dialog.
    pub(super) fn object_export_demo_steps(&mut self) {
        use crate::demo::object_export as fixture;
        let Some(mode) = fixture::mode().filter(|_| self.demo) else {
            return;
        };
        let root = fixture::id(fixture::ROOT);
        match self.frame_count {
            // saved panels must not cover the lamp
            580 => {
                self.panels.chat = false;
                self.panels.people = false;
                self.panels.inventory = false;
                self.panels.minimap = false;
                self.panels.settings = false;
            }
            600 => {
                let forward = self.camera.forward().with_z(0.0).normalize_or(glam::Vec3::X);
                let side = glam::Vec3::new(forward.y, -forward.x, 0.0);
                let at = self.world.agent.position + forward * 3.5 + side * 1.4;
                let at = glam::Vec3::new(at.x, at.y, at.z - 0.6);
                for ev in fixture::events(at) {
                    self.world.apply(ev);
                }
                let parent = self.world.inventory.root;
                self.world.inventory.add_items(fixture::inventory_items(parent));
                log::info!("demo object export: lamp at {at:?}");
            }
            700 => {
                let format = match mode {
                    "obj" => Format::Obj,
                    "gltf" => Format::Gltf,
                    _ => Format::Collada,
                };
                self.open_object_export(root, format);
            }
            900 => {
                if let Some(path) = fixture::file()
                    && self.object_export.open
                    && matches!(self.object_export_job.stage, Stage::Idle)
                {
                    log::info!(
                        "demo object export: {}/{} prims, {}/{} textures, saving",
                        self.object_export.summary.allowed,
                        self.object_export.summary.total,
                        self.object_export.summary.textures.len(),
                        self.object_export.summary.textures_total
                    );
                    self.object_export_job.stage = Stage::Gathering {
                        path,
                        since: Instant::now(),
                        inputs: HashMap::new(),
                    };
                    self.object_export.progress = Some(Progress::Writing);
                }
            }
            _ => {}
        }
    }
}
