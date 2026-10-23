use super::*;
use crate::blender_live::shape::{self, Action, Key, Outgoing, Pending};
use std::sync::mpsc;
use std::time::{Duration, Instant};

impl App {
    pub(super) fn demo_blender_shape(&mut self, readonly: bool) {
        crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
        let Some(id) = crate::shape_editor::worn_shape(&self.world) else {
            return;
        };
        let data = crate::world::inventory::wearable::create(0, "Test Shape", self.world.agent_id).and_then(|data| {
            let mut wearable = aurora_assets::wearable::Wearable::parse(&data).map_err(str::to_owned)?;
            wearable.params.insert(80, 1.0);
            wearable.params.insert(33, 1.5);
            wearable.params.insert(105, 0.8);
            wearable.encode("Test Shape").map_err(str::to_owned)
        });
        if let Ok(data) = data {
            if let Ok(wearable) = aurora_assets::wearable::Wearable::parse(&data) {
                let visual_params = self.world.avatar_lib.shape_params.edit_values(&[], &wearable.params);
                self.world.apply(NetEvent::Appearance(aurora_net::AvatarAppearance {
                    avatar_id: self.world.agent_id,
                    visual_params,
                    texture_entry: None,
                    cof_version: 1,
                    appearance_version: 1,
                    hover_height: 0.0,
                    attachments: 0,
                }));
            }
            if let Some(item) = self.world.inventory.items.get_mut(&id) {
                item.asset_id = uuid::Uuid::from_u128(0xA0B1_5A9E_0010);
                if readonly {
                    item.owner_mask &= !aurora_net::build::perm::MODIFY;
                }
            }
            self.demo_documents.insert(id, data);
        }
    }

    pub(super) fn send_blender_shape(&mut self, action: Action) {
        let connection = if action == Action::Apply {
            self.blender_live.shape_apply_connection()
        } else {
            self.blender_live.shape_connection()
        };
        let item = match shape::editable(&self.world).and_then(|item| connection.map(|_| item)) {
            Ok(item) => item,
            Err(error) => {
                self.blender_live.shape.fail(error);
                return;
            }
        };
        if self.blender_live.shape.busy() {
            return;
        }
        let state = &mut self.blender_live.shape;
        state.message.clear();
        state.action = action;
        state.error = false;
        state.loading = Some(Key::of(&item));
        state.started = Some(Instant::now());
        if self.demo {
            let result = self
                .demo_documents
                .get(&item.id)
                .cloned()
                .ok_or_else(|| aurora_i18n::tr!("blender-shape-invalid").to_owned());
            self.blender_shape_loaded(item.id, item.asset_id, &result);
        } else {
            self.send(NetCommand::PreviewInventoryItem(item));
        }
    }

    pub(super) fn blender_shape_loaded(&mut self, item: uuid::Uuid, asset: uuid::Uuid, result: &Result<Vec<u8>, String>) -> bool {
        let key = Key { item, asset };
        if self.blender_live.shape.loading != Some(key) {
            return false;
        }
        self.blender_live.shape.loading = None;
        if !key.valid(&self.world) {
            self.blender_live.shape.fail(aurora_i18n::tr!("blender-shape-changed").into());
            return true;
        }
        let bytes = match result {
            Ok(bytes) => bytes.clone(),
            Err(error) => {
                self.blender_live
                    .shape
                    .fail(aurora_i18n::tr!("blender-shape-failed", error = error.as_str()));
                return true;
            }
        };
        let id = uuid::Uuid::new_v4();
        let library = self.world.avatar_lib.clone();
        let name = self.world.inventory.items[&item].name.clone();
        let (tx, rx) = mpsc::channel();
        self.blender_live.shape.pending = Some(Pending { key, id, result: rx });
        let ctx = self.egui_ctx.clone();
        let action = self.blender_live.shape.action;
        self.scene.jobs.execute(move || {
            let result = shape::build(&library, &bytes, id, &name).map(|mut data| {
                std::sync::Arc::make_mut(&mut data)["action"] = action.wire().into();
                data
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        true
    }

    pub(super) fn update_blender_shape(&mut self) {
        if self.demo
            && !self.blender_live.shape.demo_sent
            && matches!(
                std::env::var("AURORA_DEMO_BLENDER_LIVE").as_deref(),
                Ok("shape" | "shape-readonly" | "shape-apply")
            )
            && self.blender_live.shape_connection().is_ok()
        {
            self.blender_live.shape.demo_sent = true;
            let action = if std::env::var("AURORA_DEMO_BLENDER_LIVE").as_deref() == Ok("shape-apply") {
                Action::Apply
            } else {
                Action::Create
            };
            self.send_blender_shape(action);
        }
        let state = &self.blender_live.shape;
        if !state.busy() {
            return;
        }
        let key = state
            .loading
            .or_else(|| state.pending.as_ref().map(|pending| pending.key))
            .or(state.active.map(|active| active.0));
        let shared = if state.action == Action::Apply {
            self.blender_live.shape_apply_connection()
        } else {
            self.blender_live.shape_connection()
        };
        let failure = if key.is_some_and(|key| !key.valid(&self.world)) {
            Some(aurora_i18n::tr!("blender-shape-changed").to_owned())
        } else if state.started.is_some_and(|start| start.elapsed() > Duration::from_secs(45)) {
            Some(aurora_i18n::tr!("blender-shape-timeout").to_owned())
        } else {
            shared.as_ref().err().cloned()
        };
        if let Some(error) = failure {
            if let Ok(shared) = shared {
                shared.shape.lock().take();
                if let Some((_, id)) = self.blender_live.shape.active {
                    *shared.shape_cancel.lock() = Some(id);
                }
            }
            self.blender_live.shape.fail(error);
            return;
        }
        let Ok(shared) = shared else { return };
        let state = &mut self.blender_live.shape;
        if let Some((id, error)) = shared.shape_ack.lock().take()
            && state.active.is_some_and(|active| active.1 == id)
        {
            state.active = None;
            state.started = None;
            state.error = error.is_some();
            state.message = error.map_or_else(
                || match state.action {
                    Action::Create => aurora_i18n::tr!("blender-shape-success").to_owned(),
                    Action::Apply => aurora_i18n::tr!("blender-shape-applied").to_owned(),
                },
                |error| aurora_i18n::tr!("blender-shape-failed", error = error),
            );
        }
        if let Some(pending) = state.pending.take() {
            match pending.result.try_recv() {
                Ok(Ok(data)) => {
                    // Recheck immediately before publishing; assets/COF/permissions
                    // may have changed while the worker built the reference body.
                    if !pending.key.valid(&self.world) {
                        state.fail(aurora_i18n::tr!("blender-shape-changed").into());
                    } else {
                        state.active = Some((pending.key, pending.id));
                        state.started = Some(Instant::now());
                        *shared.shape.lock() = Some(Outgoing { id: pending.id, data });
                    }
                }
                Ok(Err(error)) => state.fail(aurora_i18n::tr!("blender-shape-failed", error = error)),
                Err(mpsc::TryRecvError::Empty) => state.pending = Some(pending),
                Err(mpsc::TryRecvError::Disconnected) => state.fail(aurora_i18n::tr!("blender-shape-timeout").into()),
            }
        }
    }
}
