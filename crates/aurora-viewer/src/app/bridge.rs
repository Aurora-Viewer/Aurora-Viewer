//! Worn bridge lifecycle and the single Movelock action used by all inputs.
use super::*;
use uuid::Uuid;

impl App {
    pub(super) fn release_bridge(&mut self) {
        self.release_native_movelock();
        if let Some(command) = self.bridge_runtime.release() {
            self.send(command);
        }
    }
    pub(super) fn toggle_move_lock(&mut self) {
        if !self.in_world() {
            return;
        }
        let prefs = self.settings.bridge.entry(self.world.agent_id).or_default();
        prefs.enabled = !prefs.enabled;
        if prefs.enabled && self.bridge_runtime.error.is_some() {
            self.bridge_runtime.retry(Instant::now());
        }
        if !self.demo {
            self.settings.save();
        }
        self.update_native_movelock();
        self.update_bridge();
    }
    pub(super) fn update_bridge(&mut self) {
        let folder_names = self.demo && matches!(self.ao_demo.as_deref(), Some("folder-names" | "folder-names-wait"));
        if self.demo && self.frame_count < 300 {
            return;
        }
        if self.demo
            && self.frame_count == 300
            && (std::env::var_os("AURORA_DEMO_MOVELOCK").is_some()
                || std::env::var_os("AURORA_DEMO_MOVEMENT").is_some()
                || std::env::var_os("AURORA_DEMO_BRIDGE_QUERY").is_some()
                || crate::ao::water_demo::mode().is_some()
                || folder_names)
        {
            crate::world::bridge::demo::seed(&mut self.world);
            if self.ao_demo.is_none() {
                crate::ao::demo::inventory_services(&mut self.world, true);
            }
            self.movelock_ui.open =
                crate::ao::water_demo::mode().is_none() && !folder_names && std::env::var_os("AURORA_DEMO_BRIDGE_QUERY").is_none();
            if std::env::var("AURORA_DEMO_BRIDGE_QUERY").is_ok_and(|m| m == "radar") {
                crate::world::bridge::demo::seed_queries(&mut self.world);
                self.panels.people = true;
                self.panels.people_tab = 0;
            }
            self.panels.perf = false;
            self.settings.bridge.entry(self.world.agent_id).or_default().enabled =
                std::env::var("AURORA_DEMO_MOVELOCK").is_ok_and(|mode| mode != "off");
            if std::env::var("AURORA_DEMO_MOVELOCK").is_ok_and(|mode| mode.starts_with("native")) {
                self.settings.bridge.entry(self.world.agent_id).or_default().mode = crate::world::bridge::LockMode::NativeSit;
                crate::ao::demo::assets(&mut self.scene);
                crate::ao::demo::native_anchor_assets(&mut self.scene);
                self.panels.chat = false;
                self.panels.inventory = false;
                self.panels.people = false;
                crate::ui::widgets::place_floater(&self.egui_ctx, "movelock", egui::pos2(60.0, 90.0), egui::vec2(470.0, 450.0));
                let selected = crate::ao::demo::seed(&mut self.world.inventory, self.world.agent_id);
                self.settings.ao.accounts.insert(
                    self.world.agent_id,
                    crate::ao::Account {
                        selected,
                        enabled: std::env::var("AURORA_DEMO_MOVELOCK").is_ok_and(|mode| mode != "native-missing"),
                    },
                );
            }
        }
        let online = self.in_world()
            && (!self.demo
                || std::env::var_os("AURORA_DEMO_MOVELOCK").is_some()
                || std::env::var_os("AURORA_DEMO_MOVEMENT").is_some()
                || std::env::var_os("AURORA_DEMO_BRIDGE_QUERY").is_some()
                || crate::ao::water_demo::mode().is_some()
                || folder_names);
        if !online {
            return;
        }
        let prefs = self.settings.bridge.entry(self.world.agent_id).or_default();
        let effective = self.native_anchor.bridge_preferences(*prefs);
        self.bridge_runtime.update(&mut self.world, &effective, online, Instant::now());
        self.egui_ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique("movelock_requested"), prefs.enabled));
        if self.demo {
            let mode = std::env::var("AURORA_DEMO_MOVELOCK").unwrap_or_default();
            self.bridge_demo.startup_stalled = mode == "timeout";
            self.bridge_demo.invalid_endpoint = mode == "endpoint";
            self.bridge_demo.halt = mode == "halted";
            let prefs = self.settings.bridge.get(&self.world.agent_id).copied().unwrap_or_default();
            self.bridge_demo.state(
                &mut self.bridge_runtime,
                mode == "moving"
                    || self.input.forward
                    || self.input.back
                    || self.input.strafe_left
                    || self.input.strafe_right
                    || self.input.up
                    || self.input.down,
                mode == "seated" || self.world.agent.is_sitting(),
                &prefs,
            );
        }
        for command in std::mem::take(&mut self.bridge_runtime.commands) {
            self.send(command);
        }
        let endpoint = self.sync_bridge_queries();
        let (command, message) = self.bridge_services.update(endpoint.as_ref(), Instant::now());
        if let Some(command) = command {
            self.send(command);
        }
        if let Some(message) = message {
            self.world.system_message(message);
        }
        if self.demo
            && self.frame_count == 700
            && std::env::var("AURORA_DEMO_BRIDGE_QUERY").is_ok_and(|m| matches!(m.as_str(), "scripts" | "object" | "error"))
        {
            let object = self
                .world
                .objects
                .iter()
                .find(|(_, o)| !o.is_avatar() && Some(o.key.region) == self.world.main_region)
                .map(|(_, o)| o.full_id);
            if let Some(object) = object {
                if std::env::var("AURORA_DEMO_BRIDGE_QUERY").is_ok_and(|m| m == "object") {
                    self.on_ctx_action(crate::ui::context::CtxAction::ObjectInfo(object));
                } else {
                    self.inspect_scripts(object);
                }
            }
        }
    }
    pub(super) fn sync_bridge_queries(&mut self) -> Option<aurora_net::bridge::Endpoint> {
        let endpoint = if self.in_world() && !self.world.teleporting {
            self.bridge_runtime.query_endpoint()
        } else {
            None
        };
        if let Some(message) = self
            .bridge_services
            .sync(self.world.agent_id, self.world.main_region, endpoint.as_ref())
        {
            self.world.system_message(message);
        }
        endpoint
    }
    pub(super) fn inspect_scripts(&mut self, object: Uuid) {
        self.inspect_bridge_information(object, false);
    }
    pub(super) fn inspect_object_information(&mut self, object: Uuid) {
        self.inspect_bridge_information(object, true);
    }
    fn inspect_bridge_information(&mut self, object: Uuid, extended: bool) {
        if !self.in_world() {
            return;
        }
        let valid = self
            .world
            .objects
            .index_of_uuid(&object)
            .and_then(|i| self.world.objects.get(i))
            .is_some_and(|o| Some(o.key.region) == self.world.main_region);
        if !valid {
            self.world.system_notice(
                if extended {
                    aurora_i18n::tr!("bridge-object-info-region")
                } else {
                    aurora_i18n::tr!("bridge-script-info-region")
                },
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        let _ = self.sync_bridge_queries();
        let queued = if extended {
            self.bridge_services
                .inspect_object(object, Instant::now(), &self.world.avatar_lib.attach_names)
        } else {
            self.bridge_services.inspect(object, Instant::now())
        };
        if queued {
            if self.bridge_runtime.error.is_some() {
                self.bridge_runtime.retry(Instant::now());
            }
            self.world.system_notice(
                if extended {
                    aurora_i18n::tr!("bridge-object-info-loading")
                } else {
                    aurora_i18n::tr!("bridge-script-info-loading")
                },
                crate::world::notice_topics::Topic::Items,
            );
        }
    }
    pub(super) fn bridge_demo_command(&mut self, command: &NetCommand) -> bool {
        let mode = std::env::var("AURORA_DEMO_MOVELOCK").unwrap_or_default();
        let moving = mode == "moving"
            || self.input.forward
            || self.input.back
            || self.input.strafe_left
            || self.input.strafe_right
            || self.input.up
            || self.input.down;
        let Some(events) = self.bridge_demo.reply(
            &self.bridge_runtime,
            &self.world,
            command,
            moving,
            mode == "seated" || self.world.agent.is_sitting(),
            mode == "error",
        ) else {
            return false;
        };
        for event in events {
            let now = Instant::now();
            if let Some(event) = self.bridge_runtime.receive(&mut self.world, event, now, now) {
                self.on_app_event(event);
            }
        }
        true
    }
    pub(super) fn movelock_window(&mut self, ctx: &egui::Context, p: &Palette) {
        if std::mem::take(&mut self.toolbar_ui.movelock_options) {
            self.movelock_ui.open = true;
        }
        let prefs = self.settings.bridge.entry(self.world.agent_id).or_default();
        let fallback = prefs.enabled && prefs.mode == crate::world::bridge::LockMode::NativeSit && self.native_anchor.bridge_fallback();
        let status = if prefs.enabled && prefs.mode == crate::world::bridge::LockMode::NativeSit {
            if fallback {
                aurora_i18n::tr!("movelock-sit-needs-stand")
            } else {
                self.native_anchor.status.label()
            }
        } else {
            prefs.state()
        };
        let was_enabled = prefs.enabled;
        let shortcut = self
            .settings
            .keybinds
            .get(Action::MoveLock)
            .iter()
            .flatten()
            .map(|b| b.label())
            .collect::<Vec<_>>()
            .join(" / ");
        self.toolbar_ui.movelock_active = prefs.enabled;
        self.toolbar_ui.movelock_status = format!("{} ({shortcut})\n{status}", aurora_i18n::tr!("movelock-title"));
        if prefs.enabled && prefs.indicator {
            let label = if fallback {
                aurora_i18n::tr!("movelock-short-ao")
            } else if prefs.mode == crate::world::bridge::LockMode::NativeSit {
                self.native_anchor.status.short_label()
            } else {
                aurora_i18n::tr!("movelock-short-on")
            };
            self.movelock_ui.indicator(ctx, p, label, status);
        }
        let diagnostic = (prefs.mode == crate::world::bridge::LockMode::Bridge || fallback)
            .then(|| self.bridge_runtime.error.as_deref().or(self.bridge_runtime.progress()))
            .flatten();
        let (changed, repair) = self.movelock_ui.show(ctx, p, prefs, status, diagnostic, &shortcut);
        if repair || (prefs.enabled && !was_enabled && self.bridge_runtime.error.is_some()) {
            // Repair preserves the requested mode.
            self.bridge_runtime.retry(Instant::now());
        }
        if changed && !self.demo {
            self.settings.save();
        }
    }
}
