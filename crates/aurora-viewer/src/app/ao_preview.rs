//! The AO window's preview (ui/ao/preview.rs): while its stage is shown, a
//! hidden skeleton of our avatar plays the previewed entry
//! (world/ao_preview.rs), and the 3D stage of Inspect (`object_stage`) shows
//! our outfit posed by its joints, as Inspect shows an avatar
//! (app/inspect.rs `inspect_stage`). Closed where RLVa limits what may be
//! seen, as that stage is.
use super::*;
use crate::object_stage::Place;
use uuid::Uuid;

/// How often the parts of our own body shown on the stage are looked at
/// again (as Inspect does).
const BODY_PARTS: std::time::Duration = std::time::Duration::from_millis(500);

impl App {
    /// The preview's skeleton and stage for this frame, before the AO window
    /// is drawn: both go while the stage is not shown.
    pub(super) fn ao_preview_stage(&mut self, ctx: &egui::Context) {
        let shown = self.ao_ui.take_stage() && self.panels.ao && self.in_world();
        if !shown {
            if let Some(copy) = self.world.ao_preview() {
                self.scene.watched.remove(&copy);
                self.world.remove_ao_preview();
                self.ao_ui.stage.stage.clear(&self.scene);
                self.ao_ui.stage.restart();
                self.ao_preview_light = None;
            }
            return;
        }
        let now = std::time::Instant::now();
        let selected = self
            .settings
            .ao
            .accounts
            .get(&self.world.agent_id)
            .map_or(Uuid::nil(), |a| a.selected);
        let assets = self.ao_ui.preview_assets(self.ao_catalog.selected(selected), &self.ao_runtime);
        // what we play ourselves, less the AO's own and the stock animations
        // of the states (the preview's entry stands in for them): the face,
        // the tail, deformers and gestures stay under it
        let base: Vec<_> = self
            .world
            .animations_of(&self.world.agent_id)
            .iter()
            .filter(|a| !self.ao_runtime.plays(&a.id) && crate::ao::State::from_motion(a.id).is_none())
            .copied()
            .collect();
        let Some(copy) = self.world.update_ao_preview(base, &assets, now) else {
            return;
        };
        // posed every frame, wherever the camera looks, with our deformers
        self.scene.watched.insert(copy);
        let own = self.world.agent_id;
        self.scene.share_held_positions(&own, &copy);
        let w = &self.world;
        let Some(own) = w.objects.index_of_uuid(&w.agent_id) else {
            return;
        };
        // what we wear, not the HUDs, and our own body where it shows
        let mut roots: Vec<Uuid> = super::inspect::worn_roots(w, &w.agent_id)
            .into_iter()
            .filter(|id| {
                w.objects
                    .index_of_uuid(id)
                    .and_then(|i| w.objects.get(i))
                    .is_some_and(|o| !(31..=38).contains(&o.attachment_point()))
            })
            .collect();
        if self.ao_preview_body.is_none_or(|(at, _)| now.duration_since(at) > BODY_PARTS) {
            let parts = crate::object_stage::own::body_parts(w, &self.scene, own);
            self.ao_preview_body = Some((now, !parts.is_empty()));
        }
        if self.ao_preview_body.is_some_and(|(_, shows)| shows) {
            roots.insert(0, w.agent_id);
        }
        // a new entry stands on the floor again, framed afresh (followed,
        // the floor kept what a jump or a flight had moved)
        let changed = self.ao_ui.preview_changed(&assets);
        let w = &self.world;
        let group = |root: &Uuid| super::inspect::stage_group(w, root);
        let stage = &mut self.ao_ui.stage;
        stage.stage.update(w, &self.scene, &roots, group, Place::Worn(own), ctx, now);
        stage.worn = true;
        stage.single = false;
        stage.picking = false;
        stage.bar_face = false;
        stage.player = false;
        stage.upright = true;
        if changed {
            stage.reframe();
        }
        // the stage's ground, light or dark, shared with Inspect's: a change
        // made in the window is kept, as Inspect keeps it (app/inspect.rs)
        if self.ao_preview_light.is_some_and(|given| given != stage.light) {
            self.settings.stage_light = stage.light;
            if !self.demo {
                self.settings.save();
            }
        }
        stage.light = self.settings.stage_light;
        self.ao_preview_light = Some(stage.light);
        let joints = stage.wants_joints().then(|| {
            let model = stage.stage.model().ok();
            crate::object_stage::shown_joints(w, &copy, model.map(|m| &**m))
        });
        stage.set_joints(joints.flatten());
    }
}
