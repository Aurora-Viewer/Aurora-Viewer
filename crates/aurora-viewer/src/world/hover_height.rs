//! LLFloaterHoverHeight / LLVOAvatarSelf::setHoverOffset (Firestorm,
//! originally LGPL 2.1): preview locally, persist and send on final commit.
//!
//! Aurora addition: sitting on an object and sitting on the ground can each
//! have their own height. The simulator keeps a single hover value
//! (AgentPreferences `hover_height`), so the one of the current posture is
//! sent again whenever the avatar sits down or stands up.
use aurora_net::{NetCommand, RegionHandle};
use crossbeam_channel::{self as mpsc, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

pub const MIN: f32 = -3.0;
pub const MAX: f32 = 3.0;
pub const FILE: &str = "hover_height.json";
static REQUEST: AtomicU64 = AtomicU64::new(1);

pub fn bounded(value: f32) -> Option<f32> {
    value.is_finite().then(|| value.clamp(MIN, MAX))
}

/// Firestorm's +/- buttons: 1 cm, Alt 10 cm, Ctrl/Shift 1 mm.
pub fn step(alt: bool, fine: bool) -> f32 {
    if alt {
        0.1
    } else if fine {
        0.001
    } else {
        0.01
    }
}

/// Which saved height applies to our avatar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Posture {
    /// Standing, walking, flying: the usual (manual or automatic) height.
    #[default]
    Standing,
    /// Sitting on an object (a seat, a pose ball).
    OnObject,
    /// Sitting on the ground.
    OnGround,
}

impl Posture {
    fn seat(self) -> Option<usize> {
        match self {
            Posture::Standing => None,
            Posture::OnObject => Some(0),
            Posture::OnGround => Some(1),
        }
    }
}

/// The height of one sitting posture. Without `separate`, the standing
/// height applies; `height` keeps the last separate value meanwhile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeatedHeight {
    pub separate: bool,
    pub height: Option<f32>,
}

/// The per-account file. Older files hold the standing height alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    standing: f32,
    on_object: SeatedHeight,
    on_ground: SeatedHeight,
}

impl Saved {
    fn parse(bytes: &[u8]) -> Option<Saved> {
        let saved = match serde_json::from_slice::<Saved>(bytes) {
            Ok(saved) => saved,
            Err(_) => Saved {
                standing: serde_json::from_slice::<f32>(bytes).ok()?,
                ..Saved::default()
            },
        };
        let seat = |s: SeatedHeight| SeatedHeight {
            separate: s.separate,
            height: s.height.and_then(bounded),
        };
        Some(Saved {
            standing: bounded(saved.standing).unwrap_or(0.0),
            on_object: seat(saved.on_object),
            on_ground: seat(saved.on_ground),
        })
    }

    fn seats(&self) -> [SeatedHeight; 2] {
        [self.on_object, self.on_ground]
    }

    fn seat_mut(&mut self, seat: usize) -> &mut SeatedHeight {
        if seat == 0 { &mut self.on_object } else { &mut self.on_ground }
    }

    fn height(&self, posture: Posture) -> f32 {
        posture
            .seat()
            .map(|seat| self.seats()[seat])
            .filter(|seat| seat.separate)
            .and_then(|seat| seat.height)
            .unwrap_or(self.standing)
    }
}

struct Storage {
    loaded: Receiver<Saved>,
    writer: Option<Sender<Saved>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub struct HoverHeight {
    /// What the sliders show and our avatar is drawn with.
    preview: Saved,
    /// Saved, and sent to the simulator.
    committed: Saved,
    posture: Posture,
    region: Option<RegionHandle>,
    sent: Option<f32>,
    pending: Option<(u64, RegionHandle, f32)>,
    pub failed: bool,
    storage: Option<Storage>,
    loading: bool,
    edited: bool,
}

impl HoverHeight {
    /// The account path is chosen only after login. Reading and ordered,
    /// atomic writes stay off the frame thread; demo uses Default in memory.
    pub fn load(path: PathBuf) -> Self {
        let (loaded_tx, loaded) = mpsc::unbounded();
        let (writer, writes) = mpsc::unbounded::<Saved>();
        let worker = std::thread::Builder::new().name("aurora-hover-height".into()).spawn(move || {
            let saved = match std::fs::read(&path) {
                Ok(bytes) => Saved::parse(&bytes).unwrap_or_default(),
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("Cannot read hover height: {error}");
                    }
                    Saved::default()
                }
            };
            let _ = loaded_tx.send(saved);
            while let Ok(mut saved) = writes.recv() {
                while let Ok(newer) = writes.try_recv() {
                    saved = newer;
                }
                let save = || -> std::io::Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let json = serde_json::to_vec(&saved).map_err(std::io::Error::other)?;
                    crate::atomic_file::write(&path, &json)
                };
                if let Err(error) = save() {
                    log::warn!("Cannot save hover height: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Self {
                storage: Some(Storage {
                    loaded,
                    writer: Some(writer),
                    worker: Some(worker),
                }),
                loading: true,
                ..Self::default()
            },
            Err(error) => {
                log::warn!("Cannot start hover height storage: {error}");
                Self::default()
            }
        }
    }

    /// The standing height (manual, or the automatic estimate).
    pub fn value(&self) -> f32 {
        self.preview.standing
    }

    /// The height our avatar has now, in its current posture.
    pub fn effective(&self) -> f32 {
        self.preview.height(self.posture)
    }

    pub fn posture(&self) -> Posture {
        self.posture
    }

    /// Sitting down or standing up switches the height that is drawn and sent.
    pub fn set_posture(&mut self, posture: Posture) {
        self.posture = posture;
    }

    pub fn ready(&self) -> bool {
        !self.loading
    }

    pub fn preview(&mut self, value: f32) {
        if let Some(value) = bounded(value) {
            self.preview.standing = value;
        }
    }

    pub fn commit(&mut self) {
        if self.preview.standing != self.committed.standing {
            self.committed.standing = self.preview.standing;
            self.save();
        }
    }

    /// A sitting posture's own height, previewed (`Standing` has none).
    pub fn seated(&self, posture: Posture) -> SeatedHeight {
        posture.seat().map(|seat| self.preview.seats()[seat]).unwrap_or_default()
    }

    /// The height a sitting posture uses: its own, or the standing one.
    pub fn seated_value(&self, posture: Posture) -> f32 {
        self.preview.height(posture)
    }

    /// Turning a posture's own height on starts from the standing height the
    /// first time, then from its last value. Saved at once.
    pub fn set_separate(&mut self, posture: Posture, separate: bool) {
        let Some(seat) = posture.seat() else { return };
        let standing = self.committed.standing;
        for saved in [&mut self.preview, &mut self.committed] {
            let seat = saved.seat_mut(seat);
            seat.separate = separate;
            seat.height.get_or_insert(standing);
        }
        self.save();
    }

    pub fn preview_seated(&mut self, posture: Posture, value: f32) {
        if let (Some(seat), Some(value)) = (posture.seat(), bounded(value)) {
            let seat = self.preview.seat_mut(seat);
            if seat.separate {
                seat.height = Some(value);
            }
        }
    }

    pub fn commit_seated(&mut self, posture: Posture) {
        let Some(seat) = posture.seat() else { return };
        let preview = self.preview.seats()[seat];
        if preview != self.committed.seats()[seat] {
            *self.committed.seat_mut(seat) = preview;
            self.save();
        }
    }

    fn save(&mut self) {
        self.edited = true;
        self.failed = false;
        if let Some(writer) = self.storage.as_ref().and_then(|storage| storage.writer.as_ref()) {
            let _ = writer.send(self.committed);
        }
    }

    pub fn retry(&mut self) {
        self.failed = false;
    }

    /// One HTTP write at a time, coalescing rapid commits. A teleport must
    /// send the saved value again, even when it equals the previous region's.
    pub fn next_update(&mut self, region: Option<RegionHandle>) -> Option<NetCommand> {
        if self.loading
            && let Some(storage) = &self.storage
        {
            match storage.loaded.try_recv() {
                Ok(saved) => {
                    if !self.edited {
                        self.preview = saved;
                        self.committed = saved;
                    }
                    self.loading = false;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.loading = false,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.region != region {
            self.region = region;
            self.sent = None;
            self.failed = false;
            self.preview = self.committed;
        }
        let region = region?;
        let target = self.committed.height(self.posture);
        if self.loading || self.pending.is_some() || self.failed || self.sent == Some(target) {
            return None;
        }
        let request = REQUEST.fetch_add(1, Ordering::Relaxed);
        self.pending = Some((request, region, target));
        Some(NetCommand::SetHoverHeight {
            region,
            request,
            value: target,
        })
    }

    pub fn reply(&mut self, request: u64, region: RegionHandle, success: bool) {
        let Some((pending, target, value)) = self.pending else { return };
        if request != pending || region != target {
            return;
        }
        self.pending = None;
        if self.region == Some(region) {
            if success {
                self.sent = Some(value);
            }
            // A failed older value must not block a newer user edit or posture.
            self.failed = !success && self.committed.height(self.posture) == value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(state: &mut HoverHeight, region: u64) -> (u64, f32) {
        let Some(NetCommand::SetHoverHeight { request, value, .. }) = state.next_update(Some(region)) else {
            panic!("queued height")
        };
        (request, value)
    }

    #[test]
    fn previews_wait_for_commit_and_writes_are_ordered_and_coalesced() {
        let mut height = HoverHeight::default();
        let (first, _) = request(&mut height, 1);
        height.reply(first, 1, true);
        height.preview(0.25);
        assert_eq!(height.value(), 0.25);
        assert!(height.next_update(Some(1)).is_none());
        height.commit();
        let (second, value) = request(&mut height, 1);
        assert_eq!(value, 0.25);
        for value in [0.5, 0.75] {
            height.preview(value);
            height.commit();
        }
        assert!(height.next_update(Some(1)).is_none());
        height.reply(first, 1, true);
        assert!(height.next_update(Some(1)).is_none());
        height.reply(second, 1, true);
        assert_eq!(request(&mut height, 1).1, 0.75);
    }

    #[test]
    fn region_changes_failures_and_retry_preserve_the_committed_preference() {
        let mut height = HoverHeight::default();
        height.preview(0.2);
        height.commit();
        let (first, _) = request(&mut height, 1);
        assert!(height.next_update(None).is_none());
        height.reply(first, 1, false);
        let (second, value) = request(&mut height, 2);
        assert_eq!(value, 0.2);
        height.reply(second, 2, false);
        assert!(height.failed);
        assert!(height.next_update(Some(2)).is_none());
        height.retry();
        let (third, _) = request(&mut height, 2);
        height.reply(third, 2, true);
        assert!(height.next_update(Some(2)).is_none());
        assert_eq!(request(&mut height, 1).1, 0.2);
    }

    #[test]
    fn limits_reject_non_finite_values_and_buttons_keep_firestorm_steps() {
        assert_eq!(bounded(f32::NAN), None);
        assert_eq!(bounded(f32::INFINITY), None);
        assert_eq!(bounded(4.0), Some(MAX));
        assert_eq!(bounded(-4.0), Some(MIN));
        assert_eq!([step(false, false), step(false, true), step(true, true)], [0.01, 0.001, 0.1]);
    }

    #[test]
    fn sitting_sends_the_posture_height_and_standing_up_restores_the_usual_one() {
        let mut height = HoverHeight::default();
        height.preview(0.2);
        height.commit();
        let (first, _) = request(&mut height, 1);
        height.reply(first, 1, true);
        // Without a separate height, sitting keeps the standing one: nothing to send.
        height.set_posture(Posture::OnObject);
        assert_eq!(height.effective(), 0.2);
        assert!(height.next_update(Some(1)).is_none());
        height.set_separate(Posture::OnObject, true);
        assert_eq!(
            height.seated(Posture::OnObject).height,
            Some(0.2),
            "starts from the standing height"
        );
        height.preview_seated(Posture::OnObject, -0.15);
        assert_eq!(height.effective(), -0.15, "drawn while dragging");
        assert!(height.next_update(Some(1)).is_none(), "sent on release only");
        height.commit_seated(Posture::OnObject);
        let (second, value) = request(&mut height, 1);
        assert_eq!(value, -0.15);
        height.reply(second, 1, true);
        // The ground posture has no separate height yet: the standing one.
        height.set_posture(Posture::OnGround);
        let (third, value) = request(&mut height, 1);
        assert_eq!(value, 0.2);
        height.reply(third, 1, true);
        height.set_separate(Posture::OnGround, true);
        height.preview_seated(Posture::OnGround, 0.4);
        height.commit_seated(Posture::OnGround);
        let (fourth, value) = request(&mut height, 1);
        assert_eq!(value, 0.4);
        height.reply(fourth, 1, true);
        height.set_posture(Posture::Standing);
        assert_eq!(height.effective(), 0.2);
        assert_eq!(request(&mut height, 1).1, 0.2);
        // A standing edit (manual or automatic) leaves the sitting heights alone.
        height.preview(0.3);
        height.commit();
        assert_eq!(height.seated_value(Posture::OnObject), -0.15);
        assert_eq!(height.seated_value(Posture::OnGround), 0.4);
        // Off again: the standing height applies, the separate value is kept.
        height.set_separate(Posture::OnObject, false);
        assert_eq!(height.seated_value(Posture::OnObject), 0.3);
        height.preview_seated(Posture::OnObject, 1.0);
        assert_eq!(height.seated(Posture::OnObject).height, Some(-0.15), "inactive slider");
        height.set_separate(Posture::OnObject, true);
        assert_eq!(height.seated_value(Posture::OnObject), -0.15);
    }

    #[test]
    fn a_failed_posture_height_does_not_block_the_next_posture() {
        let mut height = HoverHeight::default();
        height.set_separate(Posture::OnGround, true);
        height.preview_seated(Posture::OnGround, 0.5);
        height.commit_seated(Posture::OnGround);
        let (first, _) = request(&mut height, 1);
        height.reply(first, 1, true);
        height.set_posture(Posture::OnGround);
        let (second, value) = request(&mut height, 1);
        assert_eq!(value, 0.5);
        height.set_posture(Posture::Standing); // stood up while the write was in flight
        height.reply(second, 1, false);
        assert!(!height.failed);
        assert!(height.next_update(Some(1)).is_none(), "the simulator kept the standing height");
        height.set_posture(Posture::OnGround);
        assert_eq!(request(&mut height, 1).1, 0.5, "sitting again retries");
    }

    #[test]
    fn older_files_hold_only_the_standing_height() {
        assert_eq!(
            Saved::parse(b"0.25"),
            Some(Saved {
                standing: 0.25,
                ..Saved::default()
            })
        );
        let saved = Saved::parse(br#"{"standing":9,"on_ground":{"separate":true,"height":-0.1}}"#).unwrap();
        assert_eq!(saved.standing, MAX);
        assert_eq!(saved.on_object, SeatedHeight::default());
        assert_eq!(saved.height(Posture::OnGround), -0.1);
        assert_eq!(Saved::parse(b"garbage"), None);
    }

    #[test]
    fn all_posture_heights_and_separate_switches_remain_independent_between_accounts() {
        let root = std::env::temp_dir().join(format!("aurora-hover-accounts-{}", uuid::Uuid::new_v4()));
        let first_path = root.join("first").join(FILE);
        let second_path = root.join("second").join(FILE);
        for (path, standing, object, ground, ground_separate) in
            [(&first_path, 0.2, -0.15, 0.4, true), (&second_path, -0.1, 0.3, -0.2, false)]
        {
            let mut height = HoverHeight::load(path.clone());
            height.preview(standing);
            height.commit();
            height.set_separate(Posture::OnObject, true);
            height.preview_seated(Posture::OnObject, object);
            height.commit_seated(Posture::OnObject);
            height.set_separate(Posture::OnGround, true);
            height.preview_seated(Posture::OnGround, ground);
            height.commit_seated(Posture::OnGround);
            height.set_separate(Posture::OnGround, ground_separate);
            drop(height);
        }
        for (path, standing, object, ground, ground_separate) in
            [(&second_path, -0.1, 0.3, -0.2, false), (&first_path, 0.2, -0.15, 0.4, true)]
        {
            let mut height = HoverHeight::load(path.clone());
            for _ in 0..1000 {
                let _ = height.next_update(None);
                if height.ready() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(height.ready(), "account height loaded");
            assert_eq!(height.value(), standing);
            assert_eq!(
                height.seated(Posture::OnObject),
                SeatedHeight {
                    separate: true,
                    height: Some(object)
                }
            );
            assert_eq!(
                height.seated(Posture::OnGround),
                SeatedHeight {
                    separate: ground_separate,
                    height: Some(ground)
                }
            );
            height.set_posture(Posture::OnObject);
            assert_eq!(height.effective(), object);
            height.set_posture(Posture::OnGround);
            assert_eq!(height.effective(), if ground_separate { ground } else { standing });
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persisted_heights_are_per_account_and_the_latest_commit_survives_restart() {
        let root = std::env::temp_dir().join(format!("aurora-hover-{}", uuid::Uuid::new_v4()));
        let path = root.join("first").join(FILE);
        let mut height = HoverHeight::load(path.clone());
        height.preview(0.2);
        height.commit();
        height.preview(-0.135);
        height.commit();
        height.set_separate(Posture::OnObject, true);
        height.preview_seated(Posture::OnObject, 0.07);
        height.commit_seated(Posture::OnObject);
        drop(height); // drains the ordered writer
        let saved = Saved::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.standing, -0.135);
        assert_eq!(saved.height(Posture::OnObject), 0.07);
        let mut reloaded = HoverHeight::load(path);
        reloaded.set_posture(Posture::OnObject);
        for _ in 0..1000 {
            if let Some(NetCommand::SetHoverHeight { value, .. }) = reloaded.next_update(Some(1)) {
                assert_eq!(value, 0.07);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!reloaded.loading);
        assert_eq!(reloaded.value(), -0.135);
        assert!(!root.join("second").join(FILE).exists());
        drop(reloaded);
        std::fs::remove_dir_all(root).unwrap();
    }
}
