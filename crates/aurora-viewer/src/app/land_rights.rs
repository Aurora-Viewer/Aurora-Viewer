//! The land test of the actions that put something in the world or move the
//! avatar through it: placing a portal and passing through an object. The
//! rules are [`crate::world::land::agent_can_build`] and
//! [`crate::movement::passage::rights`]; this reads what the viewer knows and
//! asks for a parcel it does not know yet (selectParcelAt with the hover
//! tips' sequence, whose answers it keeps), so a later try can pass.
use super::*;
use crate::movement::passage::Refusal;
use crate::world::land::BuildRight;

impl App {
    /// allowAgentBuild at `local` of the main region.
    pub(super) fn build_right_at(&mut self, local: Vec3) -> BuildRight {
        let Some(handle) = self.world.main_region else {
            return BuildRight::Unknown;
        };
        let origin = aurora_net::handle_to_origin(handle);
        let known = self.hover_tips.parcels.at(handle, local, origin).map(|p| p.as_ref());
        let right = self.world.build_right_at(handle, local, known);
        if right == BuildRight::Unknown
            && let Some(cmd) = self.hover_tips.parcels.request(handle, local, origin, Instant::now())
        {
            self.send(cmd);
        }
        right
    }

    /// May we pass through `wall` (an object index) to `destination`: the
    /// object's rights read on its linkset root, the land where we stand
    /// (mAgentParcel) and where we would come out.
    pub(super) fn passage_rights(&mut self, wall: usize, destination: Vec3) -> Result<(), Refusal> {
        let root = crate::build::root_of(&self.world, wall);
        // a child whose root has not arrived yet tells nothing about the linkset
        let flags = self.world.objects.get(root).filter(|o| o.parent_id == 0).map(|o| o.update_flags);
        // the object alone lets us through: no parcel to ask for
        if crate::movement::passage::rights(flags, BuildRight::Denied, BuildRight::Denied).is_ok() {
            return Ok(());
        }
        let here = match self.world.build_right_here() {
            BuildRight::Unknown => self.build_right_at(self.world.agent.position),
            known => known,
        };
        let there = self.build_right_at(destination);
        crate::movement::passage::rights(flags, here, there)
    }

    /// The demo plaza becomes someone else's land closed to building.
    pub(super) fn demo_close_land(&mut self) {
        if let Some(parcel) = self.world.parcel.as_mut() {
            let parcel = Arc::make_mut(parcel);
            parcel.flags &= !(aurora_net::parcel_flags::CREATE_OBJECTS | aurora_net::parcel_flags::CREATE_GROUP_OBJECTS);
            parcel.owner_id = uuid::Uuid::from_u128(0xA0_7A55);
            parcel.is_group_owned = false;
        }
    }

    /// `AURORA_DEMO_MOVEMENT=passage-refused`: on closed land, facing
    /// someone else's object, the press is refused by the same rule and
    /// message as on a grid.
    pub(super) fn passage_refused_demo(&mut self) {
        self.demo_close_land();
        let ahead = self.world.agent.position + self.world.agent.forward() * 2.0;
        let here = self.world.build_right_here();
        let there = self.build_right_at(ahead);
        if let Err(refusal) = crate::movement::passage::rights(Some(0), here, there) {
            self.movement_ui.message = refusal.message();
            self.movement_ui.demo_reveal_message = true;
        }
    }
}
