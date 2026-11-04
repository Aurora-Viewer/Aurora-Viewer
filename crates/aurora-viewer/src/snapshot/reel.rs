//! The photo reel: the session's last photos, newest first, whatever their
//! destination, to look at again, send elsewhere or delete (Aurora's own:
//! the Legacy Viewer only shows the last photo). Each photo keeps its full
//! pixels in a file: the one it was saved to on disk, or the reel's own
//! copy in a folder of the session (`Folder`), emptied when the session
//! ends. The thumbnails and the pictures on screen belong to the app.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;
use uuid::Uuid;

/// Photos the reel keeps; an older one leaves it.
pub const SIZE: usize = 10;

/// Where a photo went.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    Disk(PathBuf),
    /// The Photo Album item it became.
    Inventory(Uuid),
    Profile,
    Email,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Photo {
    /// Stable for the session: the app's textures and the quick view hold
    /// it, not an index.
    pub id: u64,
    /// When it was taken (unix seconds).
    pub taken: i64,
    pub size: (u32, u32),
    /// A cut-out photo (transparent background, `isolate`).
    pub cutout: bool,
    /// The full photo; None while the reel's copy is being written.
    pub file: Option<PathBuf>,
    /// `file` is the reel's own copy, gone with the photo or the session.
    pub own_copy: bool,
    /// Where it went, in order; empty when its sending failed (kept to be
    /// sent again).
    pub places: Vec<Place>,
}

impl Photo {
    /// The file it was saved to on disk, the first one.
    pub fn on_disk(&self) -> Option<&Path> {
        self.places.iter().find_map(|p| match p {
            Place::Disk(path) => Some(path.as_path()),
            _ => None,
        })
    }

    pub fn item(&self) -> Option<Uuid> {
        self.places.iter().find_map(|p| match p {
            Place::Inventory(id) => Some(*id),
            _ => None,
        })
    }

    pub fn went(&self, place: &Place) -> bool {
        self.places
            .iter()
            .any(|p| std::mem::discriminant(p) == std::mem::discriminant(place))
    }
}

/// The reel of the session.
#[derive(Debug, Default)]
pub struct Reel {
    photos: Vec<Photo>,
    next: u64,
}

impl Reel {
    /// A new photo at the front; the ones pushed past SIZE come back (their
    /// own copies are the caller's to delete).
    pub fn add(
        &mut self,
        taken: i64,
        size: (u32, u32),
        cutout: bool,
        file: Option<PathBuf>,
        own_copy: bool,
        places: Vec<Place>,
    ) -> (u64, Vec<Photo>) {
        self.next += 1;
        let id = self.next;
        self.photos.insert(
            0,
            Photo {
                id,
                taken,
                size,
                cutout,
                file,
                own_copy,
                places,
            },
        );
        let dropped = if self.photos.len() > SIZE {
            self.photos.split_off(SIZE)
        } else {
            Vec::new()
        };
        (id, dropped)
    }

    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn len(&self) -> usize {
        self.photos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.photos.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Photo> {
        self.photos.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Photo> {
        self.photos.iter_mut().find(|p| p.id == id)
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.photos.iter().position(|p| p.id == id)
    }

    /// The reel's copy of a photo is written.
    pub fn copied(&mut self, id: u64, file: PathBuf) {
        if let Some(photo) = self.get_mut(id) {
            photo.file = Some(file);
        }
    }

    /// The photo went somewhere more. Saved to disk, the file becomes its
    /// source: the reel's own copy comes back, to delete.
    pub fn sent(&mut self, id: u64, place: Place) -> Option<PathBuf> {
        let photo = self.get_mut(id)?;
        let mut freed = None;
        if let Place::Disk(path) = &place {
            if photo.own_copy {
                freed = photo.file.replace(path.clone());
                photo.own_copy = false;
            } else if photo.file.is_none() {
                photo.file = Some(path.clone());
            }
        }
        if !photo.places.contains(&place) {
            photo.places.push(place);
        }
        freed
    }

    pub fn remove(&mut self, id: u64) -> Option<Photo> {
        let i = self.index_of(id)?;
        Some(self.photos.remove(i))
    }

    /// Every photo, for the end of the session.
    pub fn clear(&mut self) -> Vec<Photo> {
        std::mem::take(&mut self.photos)
    }
}

/// The quick view's photo after a step of `delta` (the wheel, the arrow
/// keys): no wrapping, like a phone's gallery.
pub fn step(len: usize, cursor: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    (cursor as i64 + i64::from(delta)).clamp(0, len as i64 - 1) as usize
}

/// The photo shown after the one at `cursor` was deleted: the next older
/// one, or the newer one when it was the oldest; None when the reel is
/// empty.
pub fn after_removal(len_after: usize, cursor: usize) -> Option<usize> {
    (len_after > 0).then(|| cursor.min(len_after - 1))
}

/// What deleting a photo does, for its confirmation and its work.
#[derive(Debug, Default, PartialEq)]
pub struct Deletion {
    /// Its files on disk, to the Recycle Bin.
    pub recycle: Vec<PathBuf>,
    /// Its Photo Album items, to the inventory's Trash.
    pub trash: Vec<Uuid>,
    /// The reel's own copy, deleted.
    pub copy: Option<PathBuf>,
    /// It was posted or e-mailed: that stays.
    pub sent: bool,
}

pub fn deletion(photo: &Photo) -> Deletion {
    let mut d = Deletion {
        copy: photo.file.clone().filter(|_| photo.own_copy),
        ..Deletion::default()
    };
    for place in &photo.places {
        match place {
            Place::Disk(path) => {
                if !d.recycle.contains(path) {
                    d.recycle.push(path.clone());
                }
            }
            Place::Inventory(id) => {
                if !d.trash.contains(id) {
                    d.trash.push(*id);
                }
            }
            Place::Profile | Place::Email => d.sent = true,
        }
    }
    d
}

/// The size a photo is sent at: inside the destination's `target`, its
/// proportions kept, never enlarged.
pub fn fit_inside(size: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (w, h) = (size.0.max(1) as f64, size.1.max(1) as f64);
    let scale = (target.0 as f64 / w).min(target.1 as f64 / h).min(1.0);
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// The session's folder of copies, under `root`. Each viewer running holds
/// a lock on its own: a folder whose lock can be taken was left by one
/// that ended without emptying it (a crash), and is deleted.
pub struct Folder {
    dir: PathBuf,
    lock: Option<File>,
}

const LOCK: &str = ".lock";

impl Folder {
    pub fn open(root: &Path) -> std::io::Result<Folder> {
        sweep(root);
        let dir = root.join(Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&dir)?;
        let lock = File::create(dir.join(LOCK))?;
        lock.try_lock().map_err(std::io::Error::other)?;
        Ok(Folder { dir, lock: Some(lock) })
    }

    /// The copy of the photo `id`.
    pub fn path(&self, id: u64) -> PathBuf {
        self.dir.join(format!("{id}.png"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // the lock closed first: Windows deletes no open file
        drop(self.lock.take());
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            log::warn!("photo reel: {} not deleted: {e}", self.dir.display());
        }
    }
}

/// Delete the folders no running viewer holds.
fn sweep(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let free = match File::open(dir.join(LOCK)) {
            Ok(f) => f.try_lock().is_ok(),
            // no lock: never finished opening
            Err(_) => true,
        };
        if free && let Err(e) = std::fs::remove_dir_all(&dir) {
            log::warn!("photo reel: {} left over, not deleted: {e}", dir.display());
        }
    }
}

/// Write the reel's copy of a photo (PNG, its alpha kept), beside its name
/// then renamed: a copy is never read half written.
pub fn write_copy(path: &Path, size: (u32, u32), rgba: &[u8]) -> Result<(), String> {
    let part = path.with_extension("part");
    let file = File::create(&part).map_err(|e| e.to_string())?;
    let mut out = std::io::BufWriter::new(file);
    super::save::encode(&mut out, size.0, size.1, rgba, super::FileFormat::Png, 0, &AtomicU32::new(0))?;
    std::io::Write::flush(&mut out).map_err(|e| e.to_string())?;
    drop(out);
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}

/// A photo's pixels (RGBA8) read back, reduced to `max_side` when given.
pub fn load(path: &Path, max_side: Option<u32>) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    Ok(match max_side {
        Some(side) => super::save::downscale(w, h, rgba.as_raw(), side),
        None => (w, h, rgba.into_raw()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(name: &str) -> Place {
        Place::Disk(PathBuf::from(name))
    }

    #[test]
    fn newest_first_and_ten_kept() {
        let mut reel = Reel::default();
        let mut dropped = Vec::new();
        for i in 0..12 {
            let (_, d) = reel.add(i, (4, 4), false, Some(format!("{i}.png").into()), true, vec![Place::Email]);
            dropped.extend(d);
        }
        assert_eq!(reel.len(), SIZE);
        assert_eq!(reel.photos()[0].taken, 11);
        assert_eq!(reel.photos()[SIZE - 1].taken, 2);
        // the two oldest came back, one per photo added, for their copies
        assert_eq!(dropped.iter().map(|p| p.taken).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn ids_stay_with_their_photo() {
        let mut reel = Reel::default();
        let (a, _) = reel.add(1, (4, 4), false, None, true, vec![]);
        let (b, _) = reel.add(2, (4, 4), false, None, true, vec![]);
        assert_ne!(a, b);
        assert_eq!(reel.index_of(a), Some(1));
        reel.copied(a, "a.png".into());
        assert_eq!(reel.get(a).and_then(|p| p.file.clone()), Some("a.png".into()));
        assert_eq!(reel.remove(b).map(|p| p.taken), Some(2));
        assert_eq!(reel.index_of(a), Some(0));
        assert!(reel.remove(b).is_none());
    }

    #[test]
    fn saved_to_disk_the_file_replaces_the_copy() {
        let mut reel = Reel::default();
        let (id, _) = reel.add(1, (4, 4), false, Some("copy.png".into()), true, vec![Place::Profile]);
        assert_eq!(reel.sent(id, disk("mine.png")), Some("copy.png".into()));
        let p = reel.get(id).expect("photo");
        assert_eq!(p.file, Some("mine.png".into()));
        assert!(!p.own_copy);
        assert_eq!(p.on_disk(), Some(Path::new("mine.png")));
        // a second destination adds a place, frees nothing
        assert_eq!(reel.sent(id, Place::Inventory(Uuid::nil())), None);
        assert_eq!(reel.sent(id, Place::Inventory(Uuid::nil())), None);
        assert_eq!(reel.get(id).map(|p| p.places.len()), Some(3));
        assert!(reel.get(id).is_some_and(|p| p.went(&Place::Inventory(Uuid::max()))));
    }

    #[test]
    fn steps_stop_at_the_ends() {
        assert_eq!(step(0, 0, 1), 0);
        assert_eq!(step(5, 0, -1), 0);
        assert_eq!(step(5, 0, 3), 3);
        assert_eq!(step(5, 3, 4), 4);
        assert_eq!(step(5, 4, -2), 2);
    }

    #[test]
    fn after_a_deletion_the_next_older_shows() {
        assert_eq!(after_removal(0, 0), None);
        // the third of five deleted: the fourth, now third
        assert_eq!(after_removal(4, 2), Some(2));
        // the oldest deleted: the one before it
        assert_eq!(after_removal(4, 4), Some(3));
    }

    #[test]
    fn deletion_names_what_goes() {
        let item = Uuid::from_u128(7);
        let photo = Photo {
            id: 1,
            taken: 0,
            size: (4, 4),
            cutout: false,
            file: Some("mine.png".into()),
            own_copy: false,
            places: vec![disk("mine.png"), Place::Inventory(item), Place::Email],
        };
        assert_eq!(
            deletion(&photo),
            Deletion {
                recycle: vec!["mine.png".into()],
                trash: vec![item],
                copy: None,
                sent: true,
            }
        );
        let only_sent = Photo {
            file: Some("1.png".into()),
            own_copy: true,
            places: vec![Place::Profile],
            ..photo
        };
        let d = deletion(&only_sent);
        assert_eq!(d.copy, Some("1.png".into()));
        assert!(d.recycle.is_empty() && d.trash.is_empty() && d.sent);
    }

    #[test]
    fn sent_again_inside_the_destination() {
        assert_eq!(fit_inside((7680, 4320), (2048, 2048)), (2048, 1152));
        assert_eq!(fit_inside((1000, 2000), (1024, 768)), (384, 768));
        // never enlarged
        assert_eq!(fit_inside((640, 480), (2048, 2048)), (640, 480));
    }

    #[test]
    fn folder_copies_and_leftovers() {
        let root = std::env::temp_dir().join(format!("aurora-reel-test-{}", Uuid::new_v4().simple()));
        // a crashed viewer's folder: no lock held
        let stale = root.join("stale");
        std::fs::create_dir_all(&stale).expect("stale dir");
        File::create(stale.join(LOCK)).expect("stale lock");
        let folder = Folder::open(&root).expect("folder");
        assert!(!stale.exists());
        // another running viewer's folder survives its sweep
        let other = Folder::open(&root).expect("other");
        assert!(folder.dir.exists());

        let path = folder.path(3);
        let rgba: Vec<u8> = (0..4 * 3 * 2).map(|i| i as u8 * 10).collect();
        write_copy(&path, (3, 2), &rgba).expect("copy");
        assert_eq!(load(&path, None).expect("load"), (3, 2, rgba));
        assert!(!path.with_extension("part").exists());

        let dir = folder.dir.clone();
        drop(folder);
        assert!(!dir.exists());
        drop(other);
        let _ = std::fs::remove_dir_all(&root);
    }
}
