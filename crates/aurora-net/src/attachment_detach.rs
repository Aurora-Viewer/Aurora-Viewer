//! ObjectDetach following LLSelectMgr::sendDetach (llselectmgr.cpp, LGPL 2.1).
//! Simulator-local roots can be detached without an inventory record, for
//! temporary attachments and recovery of ordinary attachments with stale items.
use aurora_msg::msgs::{ObjectDetach, object_detach};
use uuid::Uuid;

pub(crate) fn packet(agent: Uuid, session: Uuid, local_id: u32) -> ObjectDetach {
    let mut packet = ObjectDetach::default();
    packet.agent_data.agent_id = agent;
    packet.agent_data.session_id = session;
    packet.object_data = vec![object_detach::ObjectData { object_local_id: local_id }];
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_msg::Msg;

    #[test]
    fn rlva_temporary_detach_encodes_the_object_local_id_without_an_inventory_item() {
        let packet = packet(Uuid::from_u128(1), Uuid::from_u128(2), 999);
        let mut writer = aurora_msg::Writer::default();
        packet.encode_body(&mut writer);
        let decoded = ObjectDetach::decode(&writer.buf).unwrap();
        assert_eq!(decoded.agent_data.agent_id, Uuid::from_u128(1));
        assert_eq!(decoded.agent_data.session_id, Uuid::from_u128(2));
        assert_eq!(decoded.object_data.len(), 1);
        assert_eq!(decoded.object_data[0].object_local_id, 999);
    }
}
