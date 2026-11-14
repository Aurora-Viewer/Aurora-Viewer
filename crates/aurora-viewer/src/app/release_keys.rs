//! Advanced › Release Keys: the region lets go of every control the scripts
//! took (llTakeControls), for a broken vehicle, weapon or HUD that keeps the
//! arrow keys. Port of LLToolsReleaseKeys and LLToolsEnableReleaseKeys
//! (indra/newview/llviewermenu.cpp) and LLAgent::forceReleaseControls
//! (indra/newview/llagent.cpp), originally LGPL 2.1. The viewer keeps its
//! counts as they are: the region answers with ScriptControlChange releases.
use super::*;

/// The scenario's vehicle: the arrow keys and Page Up / Down, taken and not
/// passed on, as a vehicle script does.
const DEMO_TAKEN: u32 = aurora_net::control::AT_POS
    | aurora_net::control::AT_NEG
    | aurora_net::control::LEFT_POS
    | aurora_net::control::LEFT_NEG
    | aurora_net::control::UP_POS
    | aurora_net::control::UP_NEG
    | aurora_net::control::YAW_POS
    | aurora_net::control::YAW_NEG;

/// The frame `AURORA_DEMO_RELEASE_KEYS` takes the controls at (the region is
/// in, before `AURORA_DEMO_MENU_SEARCH` opens the menu at 540).
const DEMO_FRAME: u64 = 400;

/// Whether the entry is enabled (Tools.EnableReleaseKeys): a script holds a
/// control (anyControlGrabbed), and RLVa does not lock an attachment
/// against removal: letting go of the controls would free the resident from
/// a device they cannot take off.
pub(crate) fn allowed(any_grabbed: bool, attachment_locked: bool) -> bool {
    any_grabbed && !attachment_locked
}

/// `allowed` from the world's state; the RLVa check runs only while a
/// script holds a control.
pub(crate) fn enabled(world: &crate::world::World) -> bool {
    world.script_controls.any() && allowed(true, world.rlva_has_locked_attachment())
}

impl App {
    /// Tools.ReleaseKeys: refused while RLVa locks an attachment, as the
    /// entry is greyed then; sent even when no control is held, as the
    /// Legacy Viewer does.
    pub(super) fn release_keys(&mut self) {
        if self.world.rlva_has_locked_attachment() {
            log::info!("release keys: refused, RLVa locks an attachment");
            return;
        }
        log::info!("release keys: asking the region to release the script controls");
        self.send(NetCommand::ForceScriptControlRelease);
    }

    /// `AURORA_DEMO_RELEASE_KEYS=1`: a vehicle script takes the movement keys.
    pub(super) fn release_keys_demo_steps(&mut self) {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !self.demo
            || self.frame_count != DEMO_FRAME
            || !*ON.get_or_init(|| std::env::var("AURORA_DEMO_RELEASE_KEYS").is_ok_and(|v| v == "1"))
        {
            return;
        }
        log::info!("demo release keys: a script takes the movement keys");
        self.world.apply(NetEvent::ScriptControlChange(vec![(true, DEMO_TAKEN, false)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_needs_a_held_control_and_no_rlva_attachment_lock() {
        assert!(!allowed(false, false), "nothing held: greyed out");
        assert!(allowed(true, false));
        assert!(!allowed(true, true), "RLVa locks an attachment: greyed out");
        assert!(!allowed(false, true));
    }

    #[test]
    fn the_demo_release_frees_every_control_the_scenario_took() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        assert!(!enabled(&world));
        world.apply(NetEvent::ScriptControlChange(vec![(true, DEMO_TAKEN, false)]));
        // a weapon also takes the mouselook button, passed on
        world.apply(NetEvent::ScriptControlChange(vec![(
            true,
            aurora_net::control::ML_LBUTTON_DOWN,
            true,
        )]));
        assert!(enabled(&world));
        for event in crate::demo::demo_reply(&NetCommand::ForceScriptControlRelease) {
            world.apply(event);
        }
        assert!(!world.script_controls.any());
        assert!(!enabled(&world));
    }

    #[test]
    fn rlva_remove_locks_grey_the_entry_out() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.apply(NetEvent::ScriptControlChange(vec![(true, DEMO_TAKEN, false)]));
        world.rlva_configure(true);
        let source = uuid::Uuid::from_u128(7);
        let say = |world: &mut crate::world::World, text: &str| {
            let chat = aurora_net::ChatMessage {
                from_name: "RLVa test object".to_owned(),
                source_id: source,
                owner_id: uuid::Uuid::nil(),
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                region: None,
                position: glam::Vec3::ZERO,
                message: text.into(),
            };
            assert!(world.rlva.receive(&chat));
        };
        // an add lock leaves the attachments removable
        say(&mut world, "@addattach=n");
        assert!(enabled(&world));
        // a point locked against removal, worn or not
        say(&mut world, "@remattach:chest=n");
        assert!(!enabled(&world));
        say(&mut world, "@clear");
        say(&mut world, "@detach:skull=n");
        assert!(!enabled(&world));
        say(&mut world, "@clear");
        assert!(enabled(&world));
        // RLVa off: no lock applies
        say(&mut world, "@remattach=n");
        world.rlva_configure(false);
        assert!(enabled(&world));
    }
}
