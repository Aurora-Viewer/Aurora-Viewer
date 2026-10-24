//! Pose library, native animations and BVH files use the background job pool.
use super::*;
use document::{Document, LoadMode};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone)]
pub(super) struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub date: u64,
}
impl Entry {
    pub fn date_label(&self) -> String {
        // Proleptic Gregorian civil date from an epoch-day count (Hinnant).
        let z = self.date as i64 / 86400 + 719468;
        let era = z / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let mut year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = mp + if mp < 10 { 3 } else { -9 };
        year += i64::from(month <= 2);
        format!("{year:04}-{month:02}-{day:02}")
    }
}
pub(super) enum Loaded {
    Pose(Document, LoadMode),
    Animation(aurora_assets::Animation, LoadMode),
}
enum Output {
    Listed(Vec<Entry>),
    Loaded(Uuid, Loaded),
    Overwrite(Document, Arc<SubjectPose>, bool, bool),
    Saved,
    Prepared(clip::Prepared),
    Failed(String),
    Cancelled,
}
pub(super) enum Job {
    List,
    Load(Option<PathBuf>, LoadMode, Uuid),
    Save(Document, Arc<SubjectPose>, bool, bool, bool),
    Import(Arc<SubjectPose>, Vec<usize>, String, i32),
}
pub(super) struct Service {
    pub entries: Vec<Entry>,
    pub busy: bool,
    pub message: Option<String>,
    pub selected: Option<usize>,
    pub filter: String,
    pub job: Option<Job>,
    pub ready: bool,
    pub prepared: Vec<clip::Prepared>,
    pub overwrite: Option<(Document, Arc<SubjectPose>, bool, bool)>,
    tx: crossbeam_channel::Sender<Output>,
    rx: crossbeam_channel::Receiver<Output>,
}
impl Default for Service {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            entries: Vec::new(),
            busy: false,
            message: None,
            selected: None,
            filter: String::new(),
            job: None,
            ready: false,
            prepared: Vec::new(),
            overwrite: None,
            tx,
            rx,
        }
    }
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(aurora_i18n::tr!("poser-file-too-large").to_string());
    }
    Ok(bytes)
}
pub(super) fn native(rig: &Rig, pose: &SubjectPose, translations: &[usize], name: String, priority: i32) -> Result<clip::Prepared, String> {
    let mut document = clip::document::Document::new();
    document.name = name;
    document.meta.base_priority = priority;
    let frame = Frame {
        model: Mat4::IDENTITY,
        joints: pose.matrices(rig, Mat4::IDENTITY),
        at: Instant::now(),
    };
    document
        .record(rig, &frame, 0, &(0..rig.len()).collect::<Vec<_>>(), translations)
        .map_err(|k| aurora_i18n::text(k).to_owned())?;
    let animation = document.animation().map_err(|k| aurora_i18n::text(k).to_owned())?;
    let bytes = aurora_assets::import_animation::encode(&animation).map_err(|e| e.to_string())?;
    let animation = aurora_assets::anim::parse_animation_upload(&bytes).map_err(|e| e.to_string())?;
    Ok(clip::Prepared {
        name: document.name,
        bytes: Arc::new(bytes),
        animation: Arc::new(animation),
    })
}
fn list(root: &Path, rig: &Rig) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())?.take(512) {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_file()
            && path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".aurora-pose.json"))
            && let Ok(document) = read(&path).and_then(|bytes| serde_json::from_slice::<Document>(&bytes).map_err(|e| e.to_string()))
        {
            if !document.valid(rig) {
                continue;
            }
            let date = path
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |t| t.as_secs());
            out.push(Entry {
                path,
                name: document.name,
                date,
            });
        }
    }
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}
impl Service {
    pub fn tick(
        &mut self,
        jobs: &Jobs,
        ctx: &egui::Context,
        owner: Uuid,
        rig: Arc<Rig>,
        skeleton: Option<&aurora_assets::Skeleton>,
    ) -> Vec<(Uuid, Loaded)> {
        let mut loaded = Vec::new();
        while let Ok(output) = self.rx.try_recv() {
            self.busy = false;
            match output {
                Output::Listed(entries) => {
                    let selected = self.selected.and_then(|i| self.entries.get(i)).map(|e| e.path.clone());
                    self.selected = selected.and_then(|path| entries.iter().position(|e| e.path == path));
                    self.entries = entries;
                    self.ready = true;
                }
                Output::Loaded(subject, value) => loaded.push((subject, value)),
                Output::Overwrite(document, pose, bvh, unlock) => self.overwrite = Some((document, pose, bvh, unlock)),
                Output::Prepared(value) => self.prepared.push(value),
                Output::Saved => {
                    self.ready = false;
                    self.message = Some(aurora_i18n::tr!("poser-saved").to_string());
                }
                Output::Failed(message) => {
                    self.message = Some(message);
                    self.ready = true;
                }
                Output::Cancelled => {}
            }
        }
        if self.busy {
            return loaded;
        }
        let Some(job) = self.job.take().or_else(|| (!self.ready).then_some(Job::List)) else {
            return loaded;
        };
        self.busy = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let skeleton = skeleton.cloned().map(Arc::new);
        let root = crate::settings::account_dir(&owner).join("poses");
        jobs.execute(move || {
            let work = || -> Result<Output, String> {
                Ok(match job {
                    Job::List => Output::Listed(list(&root, &rig)?),
                    Job::Load(path, mode, subject) => {
                        let path = match path {
                            Some(p) => p,
                            None => {
                                let Some(p) = rfd::FileDialog::new()
                                    .add_filter(aurora_i18n::tr!("poser-file-filter"), &["json", "anim", "bvh"])
                                    .pick_file()
                                else {
                                    return Ok(Output::Cancelled);
                                };
                                p
                            }
                        };
                        let bytes = read(&path)?;
                        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("anim")) {
                            Output::Loaded(
                                subject,
                                Loaded::Animation(
                                    aurora_assets::anim::parse_animation_upload(&bytes).map_err(|e| e.to_string())?,
                                    mode,
                                ),
                            )
                        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("bvh")) {
                            let skeleton = skeleton.as_ref().ok_or_else(|| aurora_i18n::tr!("anim-edit-invalid").to_string())?;
                            Output::Loaded(
                                subject,
                                Loaded::Animation(
                                    aurora_assets::import_animation::parse_bvh_pose(&bytes, skeleton)
                                        .map_err(|e| e.to_string())?
                                        .animation,
                                    mode,
                                ),
                            )
                        } else {
                            let document: Document = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                            if !document.valid(&rig) {
                                return Err(aurora_i18n::tr!("anim-edit-invalid").to_string());
                            }
                            Output::Loaded(subject, Loaded::Pose(document, mode))
                        }
                    }
                    Job::Save(document, pose, bvh, unlock, confirm) => {
                        if !document.valid(&rig) {
                            return Err(aurora_i18n::tr!("anim-edit-invalid").to_string());
                        }
                        let name = clip::files::basename(&document.name);
                        let path = root.join(format!("{name}.aurora-pose.json"));
                        if confirm && (path.exists() || bvh && root.join(format!("{name}.bvh")).exists()) {
                            return Ok(Output::Overwrite(document, pose, bvh, unlock));
                        }
                        let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
                        let bvh = if bvh { Some(bvh::encode(&rig, &pose, unlock)?) } else { None };
                        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
                        let temp = root.join(format!("{}.tmp", Uuid::new_v4()));
                        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
                        if let Err(e) = std::fs::rename(&temp, &path) {
                            let _ = std::fs::remove_file(&temp);
                            return Err(e.to_string());
                        }
                        if let Some(bvh) = bvh {
                            std::fs::write(root.join(format!("{name}.bvh")), bvh).map_err(|e| e.to_string())?;
                        }
                        Output::Saved
                    }
                    Job::Import(pose, translations, name, priority) => {
                        Output::Prepared(native(&rig, &pose, &translations, name, priority)?)
                    }
                })
            };
            let _ = tx.send(work().unwrap_or_else(Output::Failed));
            ctx.request_repaint();
        });
        loaded
    }
}
