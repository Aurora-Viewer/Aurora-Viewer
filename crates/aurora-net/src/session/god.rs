//! Admin status (god mode): the request and the region's answer. Port of
//! LLAgent::requestEnterGodMode / requestLeaveGodMode (indra/newview/
//! llagent.cpp) and process_grant_godlike_powers (llviewermessage.cpp,
//! originally LGPL 2.1). The region decides: OpenSim grants it to the
//! region's owner by default, to grid gods (user level 200 or more) and, by
//! its configuration, to estate managers (GodController.cs); Second Life only
//! to its staff. The viewer only asks and shows the level it is given.

use super::{Session, emit};
use crate::types::NetEvent;
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg};

impl Session<'_> {
    /// RequestGodlikePowers: `godlike` asks for the region's god level,
    /// false gives it back. The token is null from a viewer.
    pub(super) fn request_godlike(&mut self, godlike: bool) {
        let mut m = RequestGodlikePowers::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.request_block.godlike = godlike;
        self.send_main(&m, true);
    }

    pub(super) fn dispatch_god(&mut self, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if pkt.id != GrantGodlikePowers::ID {
            return Ok(false);
        }
        let m: GrantGodlikePowers = pkt.decode()?;
        // only ours (the token is the simulator's, ignored by viewers)
        if m.agent_data.agent_id == self.agent_id() {
            emit(self.sh, NetEvent::GodLevel(m.grant_data.god_level));
        }
        Ok(true)
    }
}
