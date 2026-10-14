//! LLGestureMgr activation and sound protocol (Firestorm, LGPL 2.1).
//! Asset loading and parsing stay on the network/worker runtimes.
use super::{Session, emit};
use crate::types::NetEvent;
use aurora_msg::msgs::{ActivateGestures, DeactivateGestures, SoundTrigger, activate_gestures, deactivate_gestures};
use glam::Vec3;
use uuid::Uuid;

impl Session<'_> {
    pub(super) fn load_gesture(&self, item: Uuid, asset: Uuid) {
        let Some(cap) = self.main_cap("ViewerAsset") else {
            emit(
                self.sh,
                NetEvent::GestureLoaded {
                    item,
                    asset,
                    result: Err(aurora_i18n::tr!("gesture-load-unavailable").into()),
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = match crate::inventory::operations::preview(&http, &cap, "gesture_id", asset).await {
                Ok(data) => tokio::task::spawn_blocking(move || {
                    aurora_assets::gesture::Gesture::decode(&data).map_err(|_| aurora_i18n::tr!("gesture-invalid").to_owned())
                })
                .await
                .unwrap_or_else(|_| Err(aurora_i18n::tr!("gesture-invalid").into())),
                Err(error) => Err(error),
            };
            events.send(NetEvent::GestureLoaded { item, asset, result });
        });
    }

    pub(super) fn set_gesture_active(&mut self, item: Uuid, asset: Uuid, active: bool) {
        if item.is_nil() {
            return;
        }
        if active {
            let mut message = ActivateGestures::default();
            message.agent_data.agent_id = self.agent_id();
            message.agent_data.session_id = self.session_id();
            message.data.push(activate_gestures::Data {
                item_id: item,
                asset_id: asset,
                gesture_flags: 0,
            });
            self.send_main(&message, true);
        } else {
            let mut message = DeactivateGestures::default();
            message.agent_data.agent_id = self.agent_id();
            message.agent_data.session_id = self.session_id();
            message.data.push(deactivate_gestures::Data {
                item_id: item,
                gesture_flags: 0,
            });
            self.send_main(&message, true);
        }
    }

    pub(super) fn gesture_sound(&mut self, asset: Uuid, position: Vec3) {
        if asset.is_nil() || !position.is_finite() {
            return;
        }
        let Some(sim) = self.main.and_then(|addr| self.sims.get(&addr)) else {
            return;
        };
        let mut message = SoundTrigger::default();
        message.sound_data.sound_id = asset;
        message.sound_data.handle = sim.handle;
        message.sound_data.position = position;
        message.sound_data.gain = 1.0;
        self.send_main(&message, false);
    }
}
