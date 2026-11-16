//! What the interface may still offer on an object: the rule the cursor,
//! the hover tips and the one-click actions ask before showing anything
//! (the send boundary of `policy.rs` refuses the command anyway).

use super::super::World;
use glam::Vec3;
use uuid::Uuid;

impl World {
    /// RlvActions::canInteract (indra/newview/rlvactions.cpp): @interact
    /// leaves the HUDs alone, and @fartouch limits every other object to
    /// its distance, our own worn objects included (canTouch exempts them,
    /// this rule does not). `position` is the point under the cursor in the
    /// viewer's region-relative world, like `rlva_can_touch_at`; without one
    /// the object's own position is measured.
    pub fn rlva_can_interact_at(&self, index: usize, position: Option<Vec3>) -> bool {
        let r = &self.rlva;
        if !r.enabled() || self.hud_root(index).is_some() {
            return true;
        }
        if r.has("interact") {
            return false;
        }
        let max = r.distance_limit("fartouch", 1.5, Uuid::nil());
        // an object whose place is unknown is out of reach, never in reach
        !max.is_finite()
            || position
                .filter(|p| p.is_finite())
                .or_else(|| crate::scene::Scene::object_transform_unshifted(self, index, std::time::Instant::now()).map(|(p, _, _)| p))
                .is_some_and(|p| p.distance(self.agent.position) <= max)
    }

    /// Whether a right click on object `index` may open its menu
    /// (LLToolPie::handleRightClickPick). @interact keeps only our own avatar
    /// and the HUDs, as `rlva_can_interact_at` does; @fartouch keeps what
    /// LLToolSelect::handleObjectSelection selects: an object within its
    /// distance, what we wear, or one whose clicked `surface` is within
    /// 1.5 m, whatever the @fartouch distance is.
    pub fn rlva_can_designate(&self, index: usize, surface: Option<Vec3>) -> bool {
        let r = &self.rlva;
        if !r.enabled() || self.hud_root(index).is_some() || self.objects.get(index).is_some_and(|o| o.full_id == self.agent_id) {
            return true;
        }
        !r.has("interact")
            && (self.rlva_within_fartouch(index) || surface.is_some_and(|p| p.is_finite() && p.distance(self.agent.position) <= 1.5))
    }

    /// Whether a right click on the land may open its menu: @interact drops
    /// the pick (LLViewerWindow::cursorIntersect).
    pub fn rlva_can_designate_land(&self) -> bool {
        !self.rlva.enabled() || !self.rlva.has("interact")
    }
}

#[cfg(test)]
mod tests {
    use super::super::Command;
    use super::*;

    const A: Uuid = Uuid::from_u128(1);
    const PRIM: Uuid = Uuid::from_u128(201);
    const HUD: Uuid = Uuid::from_u128(202);

    fn world() -> (World, usize, usize) {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            w.apply(event);
        }
        let region = w.main_region.expect("demo region");
        let own = w.objects.index_of_uuid(&w.agent_id).expect("own avatar");
        let own_local = w.objects.get(own).expect("own avatar").key.local_id;
        // a prim of the world, and one worn on a HUD point (35: Center)
        for (id, local, parent, point) in [(PRIM, 32001, 0, 0u8), (HUD, 32002, own_local, 35)] {
            let mut o = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: id,
                owner_id: w.agent_id,
                point,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            });
            o.local_id = local;
            o.parent_id = parent;
            if parent == 0 {
                o.state = 0;
                o.name_values.clear();
            }
            w.objects.upsert(region, o);
        }
        let prim = w.objects.index_of_uuid(&PRIM).expect("prim");
        let hud = w.objects.index_of_uuid(&HUD).expect("hud");
        w.objects.get_mut(prim).expect("prim").position = Vec3::new(120.0, 120.0, 30.0);
        w.agent.position = Vec3::new(120.0, 123.0, 30.0);
        w.rlva.configure(true);
        (w, prim, hud)
    }

    fn restrict(w: &mut World, text: &str) {
        w.rlva.process(A, Command::parse(text).expect("command"));
    }

    #[test]
    fn rlva_interact_leaves_only_the_huds() {
        let (mut w, prim, hud) = world();
        assert!(w.hud_root(hud).is_some(), "the fixture's second prim is worn on a HUD point");
        assert!(w.rlva_can_interact_at(prim, None) && w.rlva_can_interact_at(hud, None));
        restrict(&mut w, "interact=n");
        assert!(!w.rlva_can_interact_at(prim, None));
        assert!(
            !w.rlva_can_interact_at(prim, Some(w.agent.position)),
            "a near point changes nothing"
        );
        assert!(w.rlva_can_interact_at(hud, None));
        // the preference off: no rule at all
        w.rlva.configure(false);
        assert!(w.rlva_can_interact_at(prim, None));
    }

    #[test]
    fn rlva_fartouch_measures_the_point_under_the_cursor() {
        let (mut w, prim, hud) = world();
        // the prim's origin is 3 m away
        restrict(&mut w, "fartouch:2=n");
        assert!(!w.rlva_can_interact_at(prim, None));
        // a face of it reaching within the limit is in reach
        assert!(w.rlva_can_interact_at(prim, Some(w.agent.position + Vec3::X * 1.9)));
        assert!(!w.rlva_can_interact_at(prim, Some(w.agent.position + Vec3::X * 2.1)));
        // a point that is not a number falls back on the object's place
        assert!(!w.rlva_can_interact_at(prim, Some(Vec3::NAN)));
        assert!(w.rlva_can_interact_at(hud, Some(Vec3::splat(4000.0))), "a HUD has no distance");
        restrict(&mut w, "fartouch:2=y");
        // the bare restriction is 1.5 m
        restrict(&mut w, "fartouch=n");
        assert!(w.rlva_can_interact_at(prim, Some(w.agent.position + Vec3::X * 1.4)));
        assert!(!w.rlva_can_interact_at(prim, Some(w.agent.position + Vec3::X * 1.6)));
        restrict(&mut w, "fartouch=y");
        assert!(w.rlva_can_interact_at(prim, None));
    }

    /// #2749, #2750: what a right click, the selection and the Create tool
    /// keep under @interact and @fartouch.
    #[test]
    fn rlva_interact_and_fartouch_gate_menus_selection_and_placing() {
        let (mut w, prim, hud) = world();
        let me = w.objects.index_of_uuid(&w.agent_id).expect("own avatar");
        let at = w.agent.position;
        let far = at + Vec3::X * 10.0;
        let near = at + Vec3::X;
        let free = |w: &World| {
            assert!(w.rlva_can_designate(prim, Some(far)) && w.rlva_can_designate_land());
            assert!(crate::build::rlva_selectable(w, prim));
            assert!(w.rlva_can_place_at(far));
        };
        free(&w);

        // the prim's origin is 3 m away
        restrict(&mut w, "fartouch=n");
        assert!(!w.rlva_can_designate(prim, Some(far)), "out of reach, surface included");
        assert!(!w.rlva_can_designate(prim, None));
        assert!(w.rlva_can_designate(prim, Some(near)), "a surface within 1.5 m keeps the menu");
        restrict(&mut w, "fartouch=y");
        restrict(&mut w, "fartouch:2=n");
        assert!(
            !w.rlva_can_designate(prim, Some(at + Vec3::X * 1.9)),
            "the surface exception stays 1.5 m whatever the distance"
        );
        restrict(&mut w, "fartouch:2=y");
        restrict(&mut w, "fartouch:5=n");
        assert!(w.rlva_can_designate(prim, None), "within a longer @fartouch");
        restrict(&mut w, "fartouch:5=y");
        restrict(&mut w, "fartouch=n");
        assert!(w.rlva_can_designate(me, None) && w.rlva_can_designate(hud, None));
        assert!(w.rlva_can_designate_land(), "@fartouch leaves the land menu");
        assert!(crate::build::rlva_selectable(&w, prim), "@fartouch is measured by `selectable`");
        assert!(!w.rlva_can_place_at(far), "the Create tool places within reach");
        assert!(w.rlva_can_place_at(near));
        restrict(&mut w, "fartouch=y");
        free(&w);

        restrict(&mut w, "interact=n");
        assert!(!w.rlva_can_designate(prim, Some(near)), "at any distance");
        assert!(!w.rlva_can_designate_land());
        assert!(w.rlva_can_designate(me, None), "our own avatar keeps its menu");
        assert!(w.rlva_can_designate(hud, None), "a HUD keeps its menu");
        assert!(!crate::build::rlva_selectable(&w, prim), "a world object leaves the selection");
        assert!(crate::build::rlva_selectable(&w, hud), "a HUD stays selected");
        assert!(
            w.rlva_can_place_at(far),
            "@interact refuses the Create tool elsewhere (rlva_can_rez)"
        );
        restrict(&mut w, "interact=y");
        free(&w);

        // the preference off: no rule at all
        restrict(&mut w, "interact=n");
        w.rlva.configure(false);
        free(&w);
    }
}
