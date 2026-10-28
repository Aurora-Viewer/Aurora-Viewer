//! The physics layer editor in the app: opening, saving, "New physics"
//! (LLAgentWearables::createWearable with wear_and_edit_cb) and its demo
//! scenario (AURORA_DEMO_PHYSICS_EDIT, docs/TESTING.md).
use super::*;
use crate::physics_editor::{KIND, worn_physics};

/// AURORA_DEMO_PHYSICS_EDIT=drag: seconds after the editor opens when the
/// sliders start moving, and how long they take.
const DEMO_DRAG_START: f32 = 4.0;
const DEMO_DRAG_SECONDS: f32 = 2.0;

impl App {
    /// One appearance editor at a time, as in the Legacy Viewer's single
    /// edit panel: false (and the other one asks first) while the other has
    /// unsaved changes or a save in flight.
    pub(super) fn close_shape_editor_for_physics(&mut self) -> bool {
        if !self.shape_ui.open {
            return true;
        }
        if let Some(draft) = &self.shape_ui.draft
            && (draft.pending.is_some() || draft.dirty())
        {
            if draft.pending.is_none() {
                self.shape_ui.close_confirmation = true;
            }
            return false;
        }
        self.shape_ui.close(&mut self.world);
        true
    }

    /// The shape editor's side of [`Self::close_shape_editor_for_physics`].
    pub(super) fn close_physics_editor_for_shape(&mut self) -> bool {
        if !self.physics_ui.open {
            return true;
        }
        if let Some(draft) = &self.physics_ui.draft
            && (draft.pending.is_some() || draft.dirty())
        {
            if draft.pending.is_none() {
                self.physics_ui.close_confirmation = true;
            }
            return false;
        }
        self.physics_ui.close(&mut self.world);
        true
    }

    pub(super) fn open_physics_editor(&mut self, item: uuid::Uuid) {
        if !self.close_wearable_editor() {
            return;
        }
        if !self.close_shape_editor_for_physics() {
            self.appearance_ui.message = aurora_i18n::tr!("physics-other-editor").into();
            return;
        }
        if let Some(command) = self.physics_ui.open_item(&mut self.world, item) {
            // the Legacy Viewer keeps the camera for a physics layer
            // (disable_camera_switch)
            self.panels.appearance = false;
            if self.demo {
                let data = self.demo_documents.get(&item).cloned().map(Ok).unwrap_or_else(|| {
                    crate::world::inventory::wearable::create(KIND, &self.world.inventory.items[&item].name, self.world.agent_id)
                });
                let asset = self.world.inventory.items[&item].asset_id;
                self.physics_ui.loaded(&mut self.world, item, asset, &data);
            } else {
                self.send(command);
            }
        }
    }

    pub(super) fn physics_action(&mut self, action: ui::physics::Action) {
        match action {
            ui::physics::Action::Retry => {
                if let Some(item) = worn_physics(&self.world) {
                    self.open_physics_editor(item);
                }
            }
            ui::physics::Action::Save(name) => {
                if let Some(command) = self.physics_ui.save(&self.world, name) {
                    if self.demo {
                        self.demo_save_wearable(command);
                    } else {
                        self.send(command);
                    }
                }
            }
        }
    }

    /// A save of the shape or physics editor in the demo: the exact encoder
    /// and parser, the asset kept as the item's document, and the appearance
    /// a simulator would bake from it; no grid traffic.
    pub(super) fn demo_save_wearable(&mut self, command: NetCommand) {
        let NetCommand::SaveWearable {
            request,
            item,
            data,
            copy_name,
        } = command
        else {
            return;
        };
        let mut item = *item;
        if let Some(name) = copy_name {
            item.id = uuid::Uuid::new_v4();
            item.name = name;
        }
        item.asset_id = uuid::Uuid::new_v4();
        let failed = if crate::physics_editor::is_physics(&item) {
            aurora_i18n::tr!("physics-error")
        } else {
            aurora_i18n::tr!("shape-error")
        };
        let result = aurora_assets::wearable::Wearable::parse(&data)
            .map(|wearable| {
                self.demo_documents.insert(item.id, data);
                self.demo_bake(&wearable.params);
                item
            })
            .map_err(|_| failed.to_owned());
        self.on_app_event(NetEvent::WearableSaved { request, result });
    }

    /// The appearance a simulator bakes from a worn wearable's parameters,
    /// in the demo.
    fn demo_bake(&mut self, params: &std::collections::BTreeMap<i32, f32>) {
        let mut appearance = self.world.server_appearance().cloned().unwrap_or(aurora_net::AvatarAppearance {
            avatar_id: self.world.agent_id,
            texture_entry: None,
            visual_params: Vec::new(),
            cof_version: 0,
            appearance_version: 1,
            hover_height: 0.0,
            attachments: 0,
        });
        appearance.visual_params = self
            .world
            .avatar_lib
            .shape_params
            .edit_values(self.world.server_visual_params(), params);
        // a new bake comes with a new COF version (an older one is stale)
        appearance.cof_version += 1;
        self.world.apply(NetEvent::Appearance(appearance));
    }

    /// The save of a physics layer is done: wear it again so the server
    /// bakes it (as the shape editor does). False when the request is not
    /// the physics editor's.
    pub(super) fn physics_saved(&mut self, request: uuid::Uuid, result: &Result<aurora_net::inventory::InvItem, String>) -> bool {
        if self.physics_ui.draft.as_ref().is_none_or(|draft| draft.pending != Some(request)) {
            return false;
        }
        if let Some(item) = self.physics_ui.saved(&mut self.world, request, result.clone()) {
            self.apply_appearance_action(crate::world::appearance::Action::WearItem {
                item,
                replace: true,
                point: 0,
            });
        }
        true
    }

    /// "New physics" of the outfit gallery: a layer with the default
    /// parameters in the Clothing folder, worn then edited.
    pub(super) fn new_physics(&mut self) {
        if let Some((parent, name)) = self.physics_ui.begin_create(&self.world) {
            self.appearance_ui.message = aurora_i18n::tr!("physics-creating").into();
            match crate::world::inventory::wearable::create_with_permissions(
                KIND,
                &name,
                self.world.agent_id,
                self.settings.inventory.creation_permissions.wearables.permissions(),
            ) {
                Ok(data) => self.send(NetCommand::CreateInventoryWearable {
                    parent,
                    kind: KIND,
                    name,
                    data,
                    permissions: None,
                }),
                Err(reason) => {
                    self.physics_ui.creation_failed();
                    self.appearance_ui.message = reason;
                }
            }
        }
    }

    /// An item the server created: the new physics layer is worn (true),
    /// to be edited once the outfit wears it.
    pub(super) fn physics_created(&mut self, item: &aurora_net::inventory::InvItem) -> bool {
        if !self.physics_ui.created(item) {
            return false;
        }
        if self.appearance_ui.message == aurora_i18n::tr!("physics-creating") {
            self.appearance_ui.message.clear();
        }
        self.world.inventory.add_items(vec![item.clone()]);
        self.apply_appearance_action(crate::world::appearance::Action::WearItem {
            item: item.id,
            replace: true,
            point: 0,
        });
        if self.demo {
            // the bake of the new layer: every motion at rest
            let defaults = crate::physics_editor::PARAMS.iter().map(|p| (p.id, p.default)).collect();
            self.demo_bake(&defaults);
        }
        true
    }

    pub(super) fn physics_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if let Some(item) = self.physics_ui.update(&mut self.world) {
            self.open_physics_editor(item);
        }
        let actions = ui::physics::show(ctx, p, &mut self.world, &mut self.physics_ui, self.settings.avatar_physics);
        self.demo_physics_drag();
        for action in actions {
            self.physics_action(action);
        }
    }

    /// AURORA_DEMO_PHYSICS_EDIT: `1` edits a worn physics layer (the lively
    /// one of AURORA_DEMO_PHYSICS); `new` runs "New physics" (created, worn,
    /// edited, nothing moves); `drag`: `new`, then every slider moves to the
    /// lively values as a hand would, a few seconds after the editor opens.
    pub(super) fn demo_physics_editor(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_PHYSICS_EDIT") else {
            return;
        };
        crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
        self.panels.perf = false;
        self.panels.inventory = false;
        self.panels.people = false;
        self.panels.chat = false;
        self.panels.minimap = false;
        if matches!(mode.as_str(), "new" | "drag") {
            self.new_physics();
            if mode == "drag" {
                self.physics_ui.demo_drag = Some(std::time::Instant::now());
            }
            return;
        }
        let item = aurora_net::inventory::InvItem {
            id: uuid::Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_0661),
            parent: self.world.inventory.root,
            name: aurora_i18n::tr!("physics-demo-name").into(),
            asset_type: 5,
            inv_type: 18,
            flags: u32::from(KIND),
            owner: self.world.agent_id,
            creator: self.world.agent_id,
            base_mask: 0x7fff_ffff,
            owner_mask: 0x7fff_ffff,
            next_owner_mask: 0x7fff_ffff,
            ..Default::default()
        };
        let mut wearable = crate::world::inventory::wearable::create(KIND, &item.name, self.world.agent_id)
            .ok()
            .and_then(|data| aurora_assets::wearable::Wearable::parse(&data).ok());
        if let Some(w) = &mut wearable {
            w.params.extend(crate::demo::physics::LAYER);
            if let Ok(data) = w.encode(&item.name) {
                self.demo_documents.insert(item.id, data);
            }
        }
        self.world.inventory.add_items(vec![item.clone()]);
        self.physics_ui.edit_once_worn(item.id);
        self.apply_appearance_action(crate::world::appearance::Action::WearItem {
            item: item.id,
            replace: true,
            point: 0,
        });
    }

    /// AURORA_DEMO_PHYSICS_EDIT=drag: the sliders, through the same draft
    /// and preview as a mouse drag.
    fn demo_physics_drag(&mut self) {
        let Some(start) = self.physics_ui.demo_drag else {
            return;
        };
        if self.physics_ui.draft.is_none() {
            // the clock starts once the new layer is open
            self.physics_ui.demo_drag = Some(std::time::Instant::now());
            return;
        }
        let t = ((start.elapsed().as_secs_f32() - DEMO_DRAG_START) / DEMO_DRAG_SECONDS).clamp(0.0, 1.0);
        if t <= 0.0 {
            return;
        }
        let Some(draft) = &mut self.physics_ui.draft else {
            return;
        };
        for (id, target) in crate::demo::physics::LAYER {
            if let Some(param) = crate::physics_editor::param_by_id(id) {
                crate::physics_editor::set(draft, param, param.default + (target - param.default) * t);
            }
        }
        self.physics_ui.changed(&mut self.world);
        if t >= 1.0 {
            log::info!("demo physics edit: sliders at the lively layer");
            self.physics_ui.demo_drag = None;
        }
    }
}
