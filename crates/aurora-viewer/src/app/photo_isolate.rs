//! The photo's isolation on the app's side (issue #977): the isolation
//! opens and closes with the photo mode, and the brush strokes of the
//! viewfinder become marks once the renderer tells what it drew under
//! them (`Renderer::request_drawn`, the pass that lets a click designate a
//! rigged mesh): one point of the stroke a frame, the front face there,
//! followed up to its avatar or linkset (`snapshot::isolate::root_of`).
//! The rule is `snapshot::isolate`, the panel `ui::viewfinder::isolate`.

use super::App;
use crate::snapshot::isolate::{self, Brush};
use crate::ui::viewfinder::Isolate;
use aurora_render::{DrawnMode, Renderer};

/// Points of a stroke waiting for their answer: past this, the oldest are
/// given up (the brush moves faster than the frames answer).
const PENDING: usize = 8;

impl App {
    /// Every frame, before it is laid out.
    pub(super) fn photo_isolate_frame(&mut self, renderer: &mut Renderer) {
        self.photo_isolate_demo(renderer);
        let open = self.panels.snapshot;
        self.world.photo_isolation.set_open(open);
        let s = &mut self.snapshots;
        if !open {
            s.isolate_point = None;
            for (ticket, _) in s.isolate_picks.drain(..) {
                renderer.forget_drawn(ticket);
            }
            return;
        }
        // the answers of the points asked
        let mut i = 0;
        while i < self.snapshots.isolate_picks.len() {
            let (ticket, brush) = self.snapshots.isolate_picks[i];
            match renderer.take_drawn(ticket) {
                Some(drawn) => {
                    self.snapshots.isolate_picks.remove(i);
                    // the face in front: a strand of hair before the head
                    let face = drawn.blend.or(drawn.opaque);
                    let root = face
                        .and_then(|f| self.scene.record_object(f.record))
                        .and_then(|(idx, _)| isolate::root_of(&self.world, idx));
                    if let Some((_, id)) = root
                        && self.world.photo_isolation.paint(id, brush)
                    {
                        log::info!("photo isolation: {id} {}", if brush == Brush::Add { "marked" } else { "unmarked" });
                    }
                }
                None => i += 1,
            }
        }
        // this frame's point of the stroke
        if let Some(((x, y), brush)) = self.snapshots.isolate_point.take() {
            let picks = &mut self.snapshots.isolate_picks;
            if picks.len() >= PENDING {
                let (ticket, _) = picks.remove(0);
                renderer.forget_drawn(ticket);
            }
            picks.push((renderer.request_drawn(x, y, DrawnMode::Visible), brush));
        }
        // marks of what left the scene (a teleport, an avatar gone)
        if self.frame_count.is_multiple_of(120) {
            let objects = &self.world.objects;
            self.world.photo_isolation.retain(|id| objects.index_of_uuid(id).is_some());
        }
    }

    /// AURORA_DEMO_ISOLATE=<only|hide|brush>[-panel][-opaque], with the
    /// photo mode open (AURORA_DEMO_SNAPSHOT): at frame 640 the own avatar
    /// is marked and the mode set (`only`: shown alone on an empty
    /// background, `-opaque` keeping the sky and the ground; `hide`: left
    /// out); `brush` paints instead, frames 640 to 660 with + at the middle
    /// of the window, 70 % down (the own avatar the viewfinder's camera
    /// frames there), then puts the brush down in "show only"; `panel`
    /// opens the panel.
    fn photo_isolate_demo(&mut self, renderer: &Renderer) {
        if !self.demo || !(640..=680).contains(&self.frame_count) {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_ISOLATE") else {
            return;
        };
        let words: Vec<&str> = mode.split('-').collect();
        let iso = &mut self.world.photo_isolation;
        let frame = self.frame_count;
        if frame == 640 {
            self.snapshots.vf.isolate = words.contains(&"panel");
            iso.set_transparent(!words.contains(&"opaque"));
            match words[0] {
                "brush" => iso.set_brush(Some(Brush::Add)),
                kind => {
                    iso.paint(self.world.agent_id, Brush::Add);
                    let mode = if kind == "hide" { isolate::Mode::Hide } else { isolate::Mode::Only };
                    iso.set_mode(mode);
                    log::info!("isolation demo: own avatar marked, {mode:?}");
                }
            }
        }
        if words[0] == "brush" {
            let (w, h) = renderer.size();
            if frame <= 660 {
                self.snapshots.isolate_point = Some(((w as f32 / 2.0, h as f32 * 0.7), Brush::Add));
            } else if frame == 680 {
                let iso = &mut self.world.photo_isolation;
                iso.set_brush(None);
                iso.set_mode(isolate::Mode::Only);
                log::info!("isolation demo: brush put down, {} marked, show only", iso.count());
            }
        }
    }

    /// What the viewfinder's panel or brush asks (`ppp`: pixels per point).
    pub(super) fn photo_isolate_out(&mut self, o: Isolate, ppp: f32) {
        let iso = &mut self.world.photo_isolation;
        match o {
            Isolate::Mode(mode) => iso.set_mode(mode),
            Isolate::Brush(brush) => iso.set_brush(brush),
            Isolate::Clear => iso.clear(),
            Isolate::Transparent(on) => iso.set_transparent(on),
            Isolate::Point(at) => {
                if let Some(brush) = iso.brush {
                    self.snapshots.isolate_point = Some(((at.x * ppp, at.y * ppp), brush));
                }
            }
        }
    }
}
