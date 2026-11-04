//! Advanced › Request / Leave Admin Status (Ctrl+Alt+G): the request to the
//! region (handle_god_mode / handle_leave_god_mode, indra/newview/
//! llviewermenu.cpp, originally LGPL 2.1). The region answers with the god
//! level it grants (GrantGodlikePowers, `World::set_god_level`), or not at
//! all when it refuses.
use super::*;

/// The level the demo grants: OpenSim's for a region owner or a grid god
/// (GodController.cs, 200 and more is a god).
const DEMO_GOD_LEVEL: u8 = 200;

impl App {
    pub(super) fn request_admin_status(&mut self, godlike: bool) {
        log::info!("admin status: {} asked", if godlike { "god mode" } else { "leaving god mode" });
        if self.demo {
            // no region to answer: the demo grants it at once
            self.world.set_god_level(if godlike { DEMO_GOD_LEVEL } else { 0 });
            return;
        }
        self.send(NetCommand::RequestGodlikePowers { godlike });
    }

    /// AURORA_DEMO_GOD=1: admin status asked at frame 400, the crown in the
    /// navigation bar and the notice in the chat.
    pub(super) fn demo_admin_status(&mut self) {
        if self.demo && self.frame_count == 400 && std::env::var_os("AURORA_DEMO_GOD").is_some() {
            self.request_admin_status(true);
        }
    }
}
