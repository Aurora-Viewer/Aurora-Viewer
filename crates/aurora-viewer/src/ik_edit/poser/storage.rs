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
    Saved(PathBuf),
    /// Saved outside the library by Save as…
    SavedTo(PathBuf),
    Prepared(clip::Prepared),
    Failed(String),
    Cancelled,
}
pub(super) enum Job {
    List,
    Load(Option<PathBuf>, LoadMode, Uuid),
    Save(Save),
    Import(Arc<SubjectPose>, Vec<usize>, String, i32),
}
/// Whether a library entry named `name` shows under the search `filter`
/// (case-insensitive, anywhere in the name).
pub(super) fn filter_matches(name: &str, filter: &str) -> bool {
    name.to_lowercase().contains(&filter.to_lowercase())
}
/// A save request. `pick` asks for the file in the system dialog (Save as…)
/// instead of the library; the dialog asks about an existing file itself, so
/// `confirm` only applies to the library.
pub(super) struct Save {
    pub document: Document,
    pub pose: Arc<SubjectPose>,
    pub bvh: bool,
    pub unlock: bool,
    pub confirm: bool,
    pub pick: bool,
}
/// The pose file a Save as… writes: the chosen name, with
/// `.aurora-pose.json` added when it does not already end in `.json`, so
/// that the library and Open a file find it.
fn pose_file(path: PathBuf) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if name.ends_with(".json") {
        path
    } else {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".aurora-pose.json");
        path.with_file_name(name)
    }
}
/// The `.bvh` written next to a pose file: its name without
/// `.aurora-pose.json` (or `.json`).
fn bvh_file(pose: &Path) -> PathBuf {
    let name = pose.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    let stem = [".aurora-pose.json", ".json"]
        .iter()
        .find(|ext| lower.ends_with(*ext))
        .map_or(name.as_str(), |ext| &name[..name.len() - ext.len()]);
    pose.with_file_name(format!("{stem}.bvh"))
}
pub(super) struct Service {
    pub entries: Vec<Entry>,
    pub busy: bool,
    pub message: Option<String>,
    pub selected: Option<usize>,
    pub saved_path: Option<PathBuf>,
    pub filter: String,
    /// The saved pose was just selected: the library list scrolls to it once.
    pub scroll_to_selected: bool,
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
            saved_path: None,
            filter: String::new(),
            scroll_to_selected: false,
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
                    let saved_path = self.saved_path.take();
                    let kept = self.selected.and_then(|i| self.entries.get(i)).map(|e| e.path.clone());
                    let from_save = kept.is_none() && saved_path.is_some();
                    let selected = kept.or(saved_path);
                    self.selected = selected.and_then(|path| entries.iter().position(|e| e.path == path));
                    // the pose just saved: shown even under a search that
                    // would hide it, and scrolled to once
                    if from_save && let Some(entry) = self.selected.and_then(|i| entries.get(i)) {
                        if !filter_matches(&entry.name, &self.filter) {
                            self.filter.clear();
                        }
                        self.scroll_to_selected = true;
                    }
                    self.entries = entries;
                    self.ready = true;
                }
                Output::Loaded(subject, value) => loaded.push((subject, value)),
                Output::Overwrite(document, pose, bvh, unlock) => self.overwrite = Some((document, pose, bvh, unlock)),
                Output::Prepared(value) => self.prepared.push(value),
                Output::Saved(path) => {
                    self.ready = false;
                    self.selected = None;
                    self.message = Some(aurora_i18n::tr!("poser-saved-to", path = path.display().to_string()).to_string());
                    self.saved_path = Some(path);
                }
                Output::SavedTo(path) => {
                    self.ready = false;
                    let path = path.display().to_string();
                    self.message = Some(aurora_i18n::tr!("poser-saved-to", path = path.as_str()).to_string());
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
                                let mut dialog = rfd::FileDialog::new();
                                if root.is_dir() {
                                    dialog = dialog.set_directory(&root);
                                }
                                let Some(p) = dialog
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
                    Job::Save(Save {
                        document,
                        pose,
                        bvh,
                        unlock,
                        confirm,
                        pick,
                    }) => {
                        if !document.valid(&rig) {
                            return Err(aurora_i18n::tr!("anim-edit-invalid").to_string());
                        }
                        let name = clip::files::basename(&document.name);
                        let path = if pick {
                            // Best effort: the dialog opens in the library when it exists.
                            let _ = std::fs::create_dir_all(&root);
                            let Some(path) = rfd::FileDialog::new()
                                .set_directory(&root)
                                .set_file_name(format!("{name}.aurora-pose.json"))
                                .add_filter(aurora_i18n::tr!("poser-pose-filter"), &["json"])
                                .save_file()
                            else {
                                return Ok(Output::Cancelled);
                            };
                            pose_file(path)
                        } else {
                            root.join(format!("{name}.aurora-pose.json"))
                        };
                        let bvh_path = bvh_file(&path);
                        if confirm && !pick && (path.exists() || bvh && bvh_path.exists()) {
                            return Ok(Output::Overwrite(document, pose, bvh, unlock));
                        }
                        let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
                        let bvh = if bvh { Some(bvh::encode(&rig, &pose, unlock)?) } else { None };
                        let dir = path.parent().unwrap_or(&root);
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                        let temp = dir.join(format!("{}.tmp", Uuid::new_v4()));
                        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
                        if let Err(e) = std::fs::rename(&temp, &path) {
                            let _ = std::fs::remove_file(&temp);
                            return Err(e.to_string());
                        }
                        if let Some(bvh) = bvh {
                            std::fs::write(&bvh_path, bvh).map_err(|e| e.to_string())?;
                        }
                        if pick && dir != root {
                            Output::SavedTo(path)
                        } else {
                            Output::Saved(path)
                        }
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
#[cfg(test)]
mod save_as_tests {
    use super::{bvh_file, pose_file};
    use std::path::{Path, PathBuf};

    #[test]
    fn pose_file_adds_the_pose_extension_unless_json() {
        assert_eq!(pose_file(PathBuf::from("/d/Wave")), Path::new("/d/Wave.aurora-pose.json"));
        assert_eq!(pose_file(PathBuf::from("/d/Wave.v2")), Path::new("/d/Wave.v2.aurora-pose.json"));
        assert_eq!(
            pose_file(PathBuf::from("/d/Wave.aurora-pose.json")),
            Path::new("/d/Wave.aurora-pose.json")
        );
        assert_eq!(pose_file(PathBuf::from("/d/Wave.JSON")), Path::new("/d/Wave.JSON"));
    }

    #[test]
    fn bvh_sits_next_to_the_pose_without_its_extension() {
        assert_eq!(bvh_file(Path::new("/d/Wave.aurora-pose.json")), Path::new("/d/Wave.bvh"));
        assert_eq!(bvh_file(Path::new("/d/Wave.Aurora-Pose.JSON")), Path::new("/d/Wave.bvh"));
        assert_eq!(bvh_file(Path::new("/d/Wave.json")), Path::new("/d/Wave.bvh"));
    }
}
