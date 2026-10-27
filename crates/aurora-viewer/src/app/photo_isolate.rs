//! The photo's isolation on the app's side (issue #977): the isolation
//! opens and closes with the photo mode, and while choosing in the scene
//! the viewfinder's clicks and the pointer become choices once the
//! renderer tells what it drew under them (`Renderer::request_drawn`, the
//! pass that lets a click designate a rigged mesh): the front face there,
//! followed up to its avatar or linkset (`snapshot::isolate::root_of`).
//! The names the panel lists come from the avatars' name-values or the
//! objects' properties, asked as a hover tip asks them. The rule is
//! `snapshot::isolate`, the panel `ui::viewfinder::isolate`.

use super::App;
use crate::snapshot::isolate;
use crate::ui::viewfinder::{Isolate, IsolateRow};
use aurora_render::{DrawnMode, Renderer};
use uuid::Uuid;

/// What a point asked of the renderer is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pick {
    /// A click: what it lands on is added, or removed.
    Toggle,
    /// The pointer: what it is over is named beside it.
    Hover,
    /// A click choosing the object a camera preset holds the camera by
    /// (`ui::viewfinder::presets`).
    Anchor,
}

/// Points waiting for their answer: past this, the oldest are given up.
const PENDING: usize = 6;

impl App {
    /// Every frame, before it is laid out.
    pub(super) fn photo_isolate_frame(&mut self, renderer: &mut Renderer) {
        self.photo_isolate_demo(renderer);
        let open = self.panels.snapshot;
        let agent = self.world.agent_id;
        let iso = &mut self.world.photo_isolation;
        iso.set_own(agent);
        iso.set_open(open);
        if !iso.picking && !self.snapshots.vf.preset_form.picking {
            self.snapshots.isolate_hover = None;
        }
        if !open {
            self.snapshots.isolate_point = None;
            for (ticket, _) in self.snapshots.isolate_picks.drain(..) {
                renderer.forget_drawn(ticket);
            }
            return;
        }
        // the answers of the points asked
        let mut i = 0;
        while i < self.snapshots.isolate_picks.len() {
            let (ticket, pick) = self.snapshots.isolate_picks[i];
            let Some(drawn) = renderer.take_drawn(ticket) else {
                i += 1;
                continue;
            };
            self.snapshots.isolate_picks.remove(i);
            // the face in front: a strand of hair before the head
            let root = drawn
                .blend
                .or(drawn.opaque)
                .and_then(|f| self.scene.record_object(f.record))
                .and_then(|(idx, _)| isolate::root_of(&self.world, idx))
                .map(|(_, id)| id);
            match pick {
                Pick::Hover => self.snapshots.isolate_hover = root,
                Pick::Toggle => {
                    if let Some(id) = root {
                        let added = self.world.photo_isolation.toggle(id);
                        log::info!("photo isolation: {id} {}", if added { "chosen" } else { "removed" });
                    }
                }
                Pick::Anchor => {
                    if let Some(id) = root {
                        log::info!("camera preset: anchor object {id}");
                        let form = &mut self.snapshots.vf.preset_form;
                        form.object = Some(id);
                        form.picking = false;
                    }
                }
            }
        }
        // this frame's point (a click before the pointer's hover)
        if let Some(((x, y), pick)) = self.snapshots.isolate_point.take()
            && (pick != Pick::Hover || !self.snapshots.isolate_picks.iter().any(|(_, p)| *p == Pick::Hover))
        {
            let picks = &mut self.snapshots.isolate_picks;
            if picks.len() >= PENDING {
                let (ticket, _) = picks.remove(0);
                renderer.forget_drawn(ticket);
            }
            picks.push((renderer.request_drawn(x, y, DrawnMode::Visible), pick));
        }
        // the names of the objects listed or under the pointer
        let wanted: Vec<Uuid> = self
            .world
            .photo_isolation
            .chosen()
            .iter()
            .copied()
            .chain(self.snapshots.isolate_hover)
            .collect();
        for id in wanted {
            let Some(o) = self.world.objects.index_of_uuid(&id).and_then(|i| self.world.objects.get(i)) else {
                continue;
            };
            let region = o.key.region;
            if !o.is_avatar()
                && !self.interactions.props.contains_key(&id)
                && let Some(command) = self.interactions.tip_request(region, id)
            {
                self.send(command);
            }
        }
        // choices that left the scene (a teleport, an avatar gone)
        if self.frame_count.is_multiple_of(120) {
            let objects = &self.world.objects;
            self.world.photo_isolation.retain(|id| objects.index_of_uuid(id).is_some());
        }
    }

    /// The name the panel gives an avatar or a linkset, and whether it is
    /// an avatar.
    pub(super) fn photo_isolate_name(&self, id: &Uuid) -> (String, bool) {
        let object = self.world.objects.index_of_uuid(id).and_then(|i| self.world.objects.get(i));
        if object.is_some_and(|o| o.is_avatar()) {
            let name = self
                .world
                .avatar_name(id)
                .unwrap_or_else(|| aurora_i18n::tr!("photo-isolate-avatar").to_owned());
            return (name, true);
        }
        let name = self
            .interactions
            .props
            .get(id)
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| {
                object
                    .map(|o| o.text.lines().next().unwrap_or("").to_owned())
                    .filter(|n| !n.is_empty())
            })
            .unwrap_or_else(|| aurora_i18n::tr!("photo-isolate-object").to_owned());
        (name, false)
    }

    /// The panel's rows (what is chosen) and the name under the pointer
    /// (with whether it is chosen).
    pub(super) fn photo_isolate_rows(&self) -> (Vec<IsolateRow>, Option<(String, bool)>) {
        let iso = &self.world.photo_isolation;
        let rows = iso
            .chosen()
            .iter()
            .map(|id| {
                let (name, avatar) = self.photo_isolate_name(id);
                IsolateRow { id: *id, name, avatar }
            })
            .collect();
        let hover = self
            .snapshots
            .isolate_hover
            .map(|id| (self.photo_isolate_name(&id).0, iso.is_chosen(&id)));
        (rows, hover)
    }

    /// AURORA_DEMO_ISOLATE=<own|pick>[-panel][-opaque], with the photo mode
    /// open (AURORA_DEMO_SNAPSHOT): at frame 640 `own` shows our avatar
    /// alone on an empty background (`-opaque` keeping the sky and the
    /// ground); `pick` chooses in the scene instead, the pointer at the
    /// middle of the window, 70 % down (our avatar in the viewfinder) from
    /// frame 640 and a click there at 650, then stops choosing at 680;
    /// `panel` opens the panel.
    fn photo_isolate_demo(&mut self, renderer: &Renderer) {
        if !self.demo || !(640..=680).contains(&self.frame_count) {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_ISOLATE") else {
            return;
        };
        let words: Vec<&str> = mode.split('-').collect();
        let frame = self.frame_count;
        let (w, h) = renderer.size();
        let at = (w as f32 / 2.0, h as f32 * 0.7);
        let iso = &mut self.world.photo_isolation;
        if frame == 640 {
            self.snapshots.vf.isolate = words.contains(&"panel");
            iso.set_transparent(!words.contains(&"opaque"));
            if words[0] == "pick" {
                iso.set_mode(isolate::Mode::Chosen);
                iso.set_picking(true);
            } else {
                iso.set_mode(isolate::Mode::Own);
                log::info!("isolation demo: our avatar alone");
            }
        }
        if words[0] == "pick" {
            if frame == 650 {
                self.snapshots.isolate_point = Some((at, Pick::Toggle));
            } else if frame < 670 {
                self.snapshots.isolate_point = Some((at, Pick::Hover));
            } else if frame == 680 && !words.contains(&"keep") {
                iso.set_picking(false);
                log::info!("isolation demo: choosing stopped, {} chosen", iso.chosen().len());
            }
        }
    }

    /// What the viewfinder's panel asks (`ppp`: pixels per point).
    pub(super) fn photo_isolate_out(&mut self, o: Isolate, ppp: f32) {
        let iso = &mut self.world.photo_isolation;
        match o {
            Isolate::Mode(mode) => {
                iso.set_mode(mode);
                // nothing chosen yet: straight to choosing
                iso.set_picking(mode == isolate::Mode::Chosen && iso.chosen().is_empty());
            }
            Isolate::Picking(on) => iso.set_picking(on),
            Isolate::Remove(id) => iso.remove(&id),
            Isolate::Transparent(on) => iso.set_transparent(on),
            Isolate::Click(at) => self.snapshots.isolate_point = Some(((at.x * ppp, at.y * ppp), Pick::Toggle)),
            Isolate::Hover(at) => {
                if self.snapshots.isolate_point.is_none() {
                    self.snapshots.isolate_point = Some(((at.x * ppp, at.y * ppp), Pick::Hover));
                }
            }
        }
    }
}
