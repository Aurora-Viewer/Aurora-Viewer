//! LLPreviewNotecard::saveIfNeeded for object inventory (Firestorm, LGPL 2.1).

use super::{Session, emit};
use crate::types::NetEvent;
use uuid::Uuid;

impl Session<'_> {
    pub(super) fn save_task_notecard(&mut self, request: Uuid, handle: u64, object: Uuid, item: Uuid, data: Vec<u8>) {
        let cap = self
            .sim_for_handle(handle)
            .and_then(|addr| self.sims.get(&addr))
            .and_then(|sim| sim.caps.get("UpdateNotecardTaskInventory"))
            .cloned();
        let Some(cap) = cap else {
            emit(
                self.sh,
                NetEvent::TaskNotecardSaved {
                    request,
                    object,
                    item,
                    result: Err(aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into()),
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = crate::inventory::operations::save_task_notecard(&http, &cap, object, item, data).await;
            let _ = events.send(NetEvent::TaskNotecardSaved {
                request,
                object,
                item,
                result,
            });
        });
    }
}
