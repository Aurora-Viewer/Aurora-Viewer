//! AURORA_DEMO_PHOTO_ROLL=<n>[:quick][:delete][:send][:upload][:item][:mail][:apply][:applied][:moved][:toolbar][:apart]:
//! the reel filled with `n` real photos of the plaza, saved to a folder of
//! the cache (never the Pictures folder), the focal length changed between
//! them so that they differ. `quick` then opens the quick view on the
//! newest, `delete` asks to delete it, `send` sends it again to the
//! inventory (its price asked over the quick view), `upload` too with its
//! price confirmed: uploaded, the photo shows its check in the strip and
//! Apply to profile takes the upload's place. `item` then presses Show in
//! inventory: the quick view and photo mode close, the inventory shows it. `mail` opens the e-mail's form
//! over it, `apply` uploads it, then asks to make it our profile picture;
//! `applied` accepts: photo mode closes, our profile shows it. `moved` puts the tray at the middle
//! of the screen: the photos fly to it there. `toolbar` puts the Photo roll
//! button on the right toolbar (dimmed until the first photo), then closes
//! photo mode and opens the quick view with it. `apart` takes the photos 600 frames
//! apart, each card and flight over before the next. From AURORA_DEMO_SNAPSHOT_FRAME
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
    /// `send`, its price confirmed at once.
    upload: bool,
    /// `upload`, then its Show in inventory pressed.
    item: bool,
    mail: bool,
    /// `upload`, then Apply to profile pressed.
    apply: bool,
    /// `apply`, accepted.
    applied: bool,
    /// The tray moved to the middle of the screen, the flights following it.
    moved: bool,
    /// The toolbar's Photo roll: shown, then the quick view opened with it.
    toolbar: bool,
    /// Frames between two photos.
    gap: u64,
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
            quick: words
                .iter()
                .any(|w| matches!(*w, "quick" | "delete" | "send" | "upload" | "item" | "mail" | "apply" | "applied")),
            delete: words.contains(&"delete"),
            send: words.iter().any(|w| matches!(*w, "send" | "upload" | "item" | "apply" | "applied")),
            upload: words.iter().any(|w| matches!(*w, "upload" | "item" | "apply" | "applied")),
            item: words.contains(&"item"),
            mail: words.contains(&"mail"),
            apply: words.iter().any(|w| matches!(*w, "apply" | "applied")),
            applied: words.contains(&"applied"),
            moved: words.contains(&"moved"),
            toolbar: words.contains(&"toolbar"),
            gap: if words.contains(&"apart") { 600 } else { 20 },
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
            if demo.toolbar {
                let bars = self.settings.toolbar.get_or_insert_with(Default::default);
                if !bars.right.iter().any(|b| b.command == "photo_roll") {
                    bars.right.push(crate::ui::skin::ToolButton {
                        command: "photo_roll".into(),
                        icon: "carousel-horizontal-filled".into(),
                        tooltip: String::new(),
                    });
                }
            }
            self.snapshots.reel.demo = Some(demo);
        }
        let frame = self.frame_count;
        let busy = self.photo_busy();
        let fov_now = self.settings.camera.fov;
        let uploaded = self
            .snapshots
            .reel
            .reel
            .photos()
            .first()
            .filter(|p| p.item().is_some())
            .map(|p| p.id);
        let Some(d) = self.snapshots.reel.demo.as_mut() else {
            return;
        };
        // `upload`: the price asked for the photo sent again, confirmed
        if d.done && d.upload && self.snapshots.confirm.is_some() {
            d.upload = false;
            log::info!("photo reel demo: price confirmed");
            self.photo_action(crate::ui::snapshot::Action::Confirm(true), renderer);
            return;
        }
        // `item`: uploaded, its Show in inventory pressed
        // `apply`: uploaded, Apply to profile pressed; `applied`: accepted
        if d.done
            && d.apply
            && let Some(id) = uploaded
        {
            d.apply = false;
            let accept = d.applied;
            log::info!("photo reel demo: apply to profile");
            self.photo_reel_action(ReelAction::Send(id, Destination::Profile), renderer);
            if accept {
                self.snapshots.reel.apply = None;
                self.photo_reel_apply(id);
            }
            return;
        }
        if d.done
            && d.item
            && let Some(id) = uploaded
        {
            d.item = false;
            log::info!("photo reel demo: show in inventory");
            self.photo_reel_action(ReelAction::ShowItem(id), renderer);
            return;
        }
        if d.done || busy || frame < d.start || frame < d.last + d.gap {
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
        let toolbar = d.toolbar;
        let (quick, delete, send, mail, fov) = (d.quick, d.delete, d.send, d.mail, d.fov.take());
        if let Some(fov) = fov {
            self.settings.camera.fov = fov;
        }
        let Some(newest) = self.snapshots.reel.reel.photos().first().map(|p| p.id) else {
            log::warn!("photo reel demo: no photo in the reel");
            return;
        };
        log::info!("photo reel demo: {} photos in the reel", self.snapshots.reel.reel.len());
        if toolbar {
            // photo mode closed, the quick view opened from the toolbar
            self.panels.snapshot = false;
            self.photo_reel_show_newest();
            log::info!("photo reel demo: quick view opened from the toolbar");
            return;
        }
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
    }
}
