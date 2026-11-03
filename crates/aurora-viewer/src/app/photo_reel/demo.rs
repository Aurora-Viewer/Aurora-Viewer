//! AURORA_DEMO_PHOTO_ROLL=<n>[:quick][:delete][:send][:mail][:post][:moved]: the reel filled with
//! `n` real photos of the plaza, saved to a folder of the cache (never the
//! Pictures folder), the focal length changed between them so that they
//! differ. `quick` then opens the quick view on the newest, `delete` asks
//! to delete it, `send` sends it again to the inventory (its price asked over
//! the quick view), `mail` opens the e-mail's form over it, `post` posts it to
//! the demo's web profile. `moved` puts the tray at the middle
//! of the screen: the photos fly to it there. From AURORA_DEMO_SNAPSHOT_FRAME
//! (700 by default); the photo mode's look follows AURORA_DEMO_SNAPSHOT.

use super::super::App;
use super::super::snapshot::Purpose;
use crate::ui::photo_reel::ReelAction;
use crate::ui::snapshot::Destination;
use aurora_render::Renderer;

/// The focal lengths the photos go through (mm).
const FOCALS: [f32; 4] = [35.0, 50.0, 24.0, 85.0];

#[derive(Debug, Default)]
pub(in crate::app) struct Demo {
    count: usize,
    quick: bool,
    delete: bool,
    send: bool,
    mail: bool,
    post: bool,
    /// The tray moved to the middle of the screen, the flights following it.
    moved: bool,
    start: u64,
    taken: usize,
    /// The frame of the last step.
    last: u64,
    /// The field of view before the photos, given back after them.
    fov: Option<f32>,
    done: bool,
}

impl Demo {
    fn from_env() -> Option<Demo> {
        let value = std::env::var("AURORA_DEMO_PHOTO_ROLL").ok()?;
        let mut parts = value.split(':');
        let count = parts.next()?.trim().parse::<usize>().ok()?.clamp(1, 12);
        let words: Vec<&str> = parts.collect();
        let start = std::env::var("AURORA_DEMO_SNAPSHOT_FRAME")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(700);
        Some(Demo {
            count,
            quick: words.iter().any(|w| matches!(*w, "quick" | "delete" | "send" | "mail" | "post")),
            delete: words.contains(&"delete"),
            send: words.contains(&"send"),
            mail: words.contains(&"mail"),
            post: words.contains(&"post"),
            moved: words.contains(&"moved"),
            start,
            ..Demo::default()
        })
    }
}

impl App {
    /// Every frame of the demo: the next photo, then the quick view.
    pub(in crate::app) fn photo_reel_demo(&mut self, renderer: &Renderer) {
        if !self.demo {
            return;
        }
        if self.snapshots.reel.demo.is_none() {
            let Some(demo) = Demo::from_env() else {
                return;
            };
            log::info!("photo reel demo: {} photos from frame {}", demo.count, demo.start);
            if demo.moved {
                self.settings.snapshot.viewfinder.reel_at = Some([0.55, 0.3]);
            }
            self.snapshots.reel.demo = Some(demo);
        }
        let frame = self.frame_count;
        let busy = self.photo_busy();
        let fov_now = self.settings.camera.fov;
        let Some(d) = self.snapshots.reel.demo.as_mut() else {
            return;
        };
        if d.done || busy || frame < d.start || frame < d.last + 20 {
            return;
        }
        d.last = frame;
        if d.taken < d.count {
            let (i, count) = (d.taken, d.count);
            d.taken += 1;
            d.fov.get_or_insert(fov_now);
            self.panels.snapshot = true;
            self.settings.camera.fov = crate::snapshot::fov_of_focal(FOCALS[i % FOCALS.len()]);
            let folder = crate::settings::cache_dir().join("demo-photo-reel");
            if let Err(e) = std::fs::create_dir_all(&folder) {
                log::warn!("photo reel demo: no folder: {e}");
                return;
            }
            log::info!("photo reel demo: photo {} of {count}", i + 1);
            self.photo_job(Purpose::Save(Some(folder.join(format!("reel-{}.png", i + 1)))), renderer);
            return;
        }
        // the photos all saved: the view given back, then the quick view
        d.done = true;
        let (quick, delete, send, mail, post, fov) = (d.quick, d.delete, d.send, d.mail, d.post, d.fov.take());
        if let Some(fov) = fov {
            self.settings.camera.fov = fov;
        }
        let Some(newest) = self.snapshots.reel.reel.photos().first().map(|p| p.id) else {
            log::warn!("photo reel demo: no photo in the reel");
            return;
        };
        log::info!("photo reel demo: {} photos in the reel", self.snapshots.reel.reel.len());
        if quick {
            self.photo_reel_action(ReelAction::Show(newest), renderer);
        }
        if delete {
            self.snapshots.reel.quick.ask_delete();
            log::info!("photo reel demo: deletion asked");
        }
        if send {
            self.photo_reel_action(ReelAction::Send(newest, Destination::Inventory), renderer);
            log::info!("photo reel demo: sent again to the inventory");
        }
        if mail {
            self.photo_reel_action(ReelAction::Send(newest, Destination::Email), renderer);
            log::info!("photo reel demo: e-mail form opened");
        }
        if post {
            self.photo_reel_action(ReelAction::Send(newest, Destination::Profile), renderer);
            log::info!("photo reel demo: posted again to the profile");
        }
    }
}
