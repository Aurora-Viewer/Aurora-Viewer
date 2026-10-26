//! RLVa preference and action integration, shared by every UI layout.
use super::*;
use uuid::Uuid;

impl App {
    pub(super) fn camera_aspect(&self) -> f32 {
        self.gfx
            .as_ref()
            .map(|g| g.window.inner_size())
            .filter(|size| size.width > 0 && size.height > 0)
            .map_or(1.0, |size| size.width as f32 / size.height as f32)
    }
    pub(super) fn rlva_overlay_blocks(&self) -> bool {
        let size = self.gfx.as_ref().map(|g| g.window.inner_size());
        let Some(size) = size.filter(|s| s.width > 0 && s.height > 0) else {
            return false;
        };
        let (u, v) = (self.cursor_pos.0 / size.width as f32, self.cursor_pos.1 / size.height as f32);
        self.world
            .rlva_overlay_frames(Instant::now())
            .iter()
            .any(|frame| frame.blocks_touch && self.ui_image_masks.get(&frame.texture).is_none_or(|mask| mask.hit(u, v)))
    }

    pub(super) fn update_rlva(&mut self) {
        self.demo_rlva();
        if std::mem::take(&mut self.world.rlva.im.remember) {
            self.settings.rlva.im_queries_auto_share = true;
            if !self.demo {
                self.settings.save();
            }
        }
        self.world.rlva_configure(self.settings.rlva_enabled);
        self.world.rlva.preferences.clone_from(&self.settings.rlva);
        if !self.settings.rlva_enabled || !self.settings.rlva.im_queries {
            self.world
                .notifications
                .list
                .retain(|notification| !matches!(notification.data, crate::world::notifications::Data::RlvaQuery { .. }));
        }
        for id in std::mem::take(&mut self.world.rlva.im.close) {
            self.chat_ui.forget(id);
            if self.chat_ui.selected == Some(id) {
                self.chat_ui.selected = None;
                self.chat_ui.show_profile = false;
            }
        }
        if !self.settings.rlva_enabled {
            self.named_teleport.cancel_scripts();
            self.world.maturity.cancel_script_teleports();
        }
        self.create_landmark_ui.apply_rlva_privacy(self.world.rlva.has("showloc"));
        self.media.enforce_focus(&self.world);
        if !self.settings.rlva_enabled && !self.appearance_ui.pending_actor.is_nil() {
            if let Some((_, sync)) = self.appearance_ui.pending.as_mut() {
                *sync = false;
            }
            self.appearance_ui.attachments.clear();
            self.appearance_ui.pending_actor = Uuid::nil();
        }
        self.world.rlva_controllers_tick(Instant::now());
        let login_locked = self.world.rlva.has("tploc") || (self.world.agent.is_sitting() && self.world.rlva.has("unsit"));
        self.rlva_login.update(
            self.settings.rlva_enabled,
            self.world.agent_id,
            self.world.movement_complete,
            login_locked,
        );
        self.world.eep.rlva_locked = self.world.rlva.has("setenv");
        self.world.inventory.rlva_preview_mask = self.world.rlva.preview_mask();
        self.script_editors.rlva_hidden = self.world.rlva.has("viewscript");
        self.world.rlva.appearance_busy = self.appearance_ui.pending.is_some();
        if !self.world.rlva.appearance_busy && self.world.rlva.appearance.is_empty() {
            self.world.rlva.self_detach = None;
        }
        self.world.rlva_watchdog_tick(Instant::now());
        if let Some(command) = self.world.rlva_offer_tick(Instant::now()) {
            if let NetCommand::EditInventory { request, .. } = &command {
                self.world_edits.insert(*request);
            }
            self.send_from(command, crate::world::rlva::offers::ACTOR);
        }
        if let Some(command) = self.world.rlva_rename_tick(Instant::now()) {
            if let NetCommand::EditInventory { request, .. } = &command {
                self.world_edits.insert(*request);
            }
            self.send(command);
        }
        self.world.rlva_wardrobe_tick();
        for command in std::mem::take(&mut self.world.rlva.recovery) {
            self.send_from(command, crate::world::rlva::watchdog::ACTOR);
        }
        self.world
            .rlva
            .collect(Instant::now(), |id| self.world.objects.index_of_uuid(&id).is_some());
        if !self.world.rlva.appearance_busy && self.world.rlva.appearance.is_empty() {
            self.world.rlva.self_detach = None;
        }
        self.world.rlva_update_stand();
        for (actor, command) in self.world.rlva_sitting_tick() {
            self.send_from(command, actor);
        }
        for (actor, command) in self.world.rlva_group_tick() {
            self.send_from(command, actor);
        }
        if let Some((handle, position)) = self.world.rlva.stand_return {
            self.send(NetCommand::TeleportTo {
                handle,
                position,
                look_at: self.world.agent.forward(),
            });
            self.world.rlva.stand_return = None;
        }
        let commands = self.world.rlva_pending();
        self.rlva_environment_asset();
        self.rlva_camera_commands();
        if self.appearance_ui.pending.is_none() && !self.world.rlva.appearance.is_empty() {
            let (actor, action) = self.world.rlva.appearance.remove(0);
            self.apply_appearance_action_from(action, actor);
        }
        for (actor, command) in commands {
            self.send_rlva(command, actor);
        }
        // Protocol replies are not user chat and must bypass chat redirection
        // and sendchannel, as RlvUtil::sendChatReply does.
        for command in std::mem::take(&mut self.world.rlva.out) {
            if matches!(command, NetCommand::ChatReply { .. } | NetCommand::ImReply { .. }) {
                self.net.send(command);
            } else {
                self.send(command);
            }
        }
        if self.world.rlva.has("showinv") {
            self.panels.inventory = false;
            self.inventory_ui.close_additional_window();
        }
        if self.world.rlva.has("showminimap") {
            self.panels.minimap = false;
        }
        if self.world.rlva.has("showworldmap") {
            self.panels.world_map = false;
        }
        if self.world.rlva.has("alwaysrun") && self.always_run {
            self.set_always_run(false);
        }
        if self.world.rlva.has("temprun") {
            self.temp_run = None;
        }
        let invalid: Vec<_> = self
            .build
            .selection
            .iter()
            .filter(|key| self.world.objects.index_of(key).is_none_or(|i| !self.world.rlva_can_edit(i)))
            .copied()
            .collect();
        if !invalid.is_empty() {
            self.build.deselect(&self.world, &invalid);
        }
        let locked_hud = self.world.rlva_has_locked_hud();
        if locked_hud {
            self.settings.show_huds = true;
        }
        if self.world.rlva.has("viewwireframe") || locked_hud {
            self.settings.debug.wireframe = false;
        }
        if self.world.rlva.has("viewtransparent") || self.world.rlva.has("edit") || self.world.rlva.has("showloc") {
            self.settings.debug.show_alpha = false;
        }
    }

    fn demo_rlva(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_RLVA") else {
            return;
        };
        // Native updates may run twice before the next rendered frame.
        // A fixture's force action must be delivered only once per frame.
        if self.demo_rlva_frame == Some(self.frame_count) {
            return;
        }
        self.demo_rlva_frame = Some(self.frame_count);
        if self.frame_count == 360 {
            self.settings.rlva_enabled = false;
            self.panels.settings = true;
            self.options_ui.page = ui::options::Page::Protections;
            self.options_ui.tab = 0;
        }
        if self.frame_count == 700 {
            self.settings.rlva_enabled = mode != "disabled";
            self.world.rlva_configure(self.settings.rlva_enabled);
            let scenario = mode.strip_suffix("-released").unwrap_or(&mode);
            if scenario == "baseline"
                || scenario.starts_with("sphere-")
                || scenario.starts_with("camera-")
                || matches!(scenario, "overlay" | "textures" | "silhouette" | "self" | "head" | "resolution")
            {
                // Keep the avatar and the world visible in offline visual
                // comparisons; a conversation otherwise covers the subject.
                self.panels.chat = false;
                self.panels.inventory = false;
                let focus = self.world.agent.position + Vec3::Z;
                self.camera.set_view(
                    focus + Vec3::new(-4.0, -6.0, 2.5),
                    focus,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
            }
            let message = if scenario == "baseline" {
                self.panels.settings = false;
                "@clear".to_owned()
            } else if scenario == "camera-prefs" {
                self.options_ui.page = ui::options::Page::Camera;
                self.options_ui.tab = 0;
                "@setcam=n,setcam_eyeoffset:-4/0/1=n,setcam_fov:0.8=force".to_owned()
            } else if scenario == "camera-focus" {
                self.panels.settings = false;
                format!(
                    "@setcam_focus:{};4;0/-1/0=force,setcam_fovmin:0.6=n,setcam_fovmax:0.9=n",
                    crate::demo::DEMO_LOUP
                )
            } else if scenario == "rename" {
                self.settings.rlva.shared_wear = false;
                self.settings.rlva.auto_rename = true;
                self.world.rlva_demo_naming_seed();
                "@clear".to_owned()
            } else if scenario == "hud-lock" {
                self.panels.settings = false;
                self.panels.inventory = false;
                self.panels.chat = false;
                self.settings.show_huds = false;
                self.settings.debug.wireframe = true;
                self.world.apply(NetEvent::ObjectUpdates {
                    handle: self.world.main_region.unwrap_or_default(),
                    objects: vec![crate::demo::hud::object(31, false)],
                });
                "@remattach:center 2=n".to_owned()
            } else if scenario == "communication" || scenario == "temporary" {
                self.panels.settings = false;
                self.panels.inventory = false;
                self.panels.chat = true;
                self.chat_ui.selected = (scenario == "communication").then_some(crate::demo::DEMO_TESS);
                if scenario == "communication" {
                    "@recvim=n,sendim=n,startim=n".to_owned()
                } else {
                    "@clear".to_owned()
                }
            } else if scenario == "im-query" {
                self.panels.settings = false;
                self.panels.inventory = false;
                self.panels.chat = true;
                "@fly=n,recvim=n,sendim=n".to_owned()
            } else if scenario == "sitting" {
                self.panels.settings = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                let id = Uuid::from_u128(0x726c766173656174);
                let mut seat = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                    item_id: id,
                    owner_id: self.world.agent_id,
                    point: 1,
                    add: true,
                    flags: 0,
                    group_mask: 0,
                    everyone_mask: 0,
                    next_owner_mask: 0,
                    name: String::new(),
                    desc: String::new(),
                });
                seat.local_id = 160000;
                seat.parent_id = 0;
                seat.state = 0;
                seat.name_values.clear();
                seat.position = self.world.agent.position + Vec3::new(2.0, 0.0, -0.6);
                seat.scale = Vec3::new(0.7, 0.7, 0.4);
                self.world.apply(NetEvent::ObjectUpdates {
                    handle: self.world.main_region.unwrap_or_default(),
                    objects: vec![seat],
                });
                self.camera.change_to_default(&mut self.world.agent, &self.settings.camera);
                format!("@notify:123=n,sit:{id}=force")
            } else if scenario == "tools" || scenario == "tools-closed" {
                self.panels.settings = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                self.options_ui.rlva_tools.open = true;
                self.options_ui.rlva_tools.tab = 3;
                self.settings.rlva.debug_commands = true;
                "@jump=n,detach=n,fly=n,fly=n,fly=y".to_owned()
            } else if scenario == "named-blocked" {
                let request = Uuid::from_u128(0x726c76616e616d65);
                self.request_user_teleport(
                    NetCommand::TeleportToRegion {
                        request,
                        name: "Aurora Deferred Demo".into(),
                        position: Vec3::new(128.0, 128.0, 30.0),
                        look_at: Vec3::X,
                    },
                    "Aurora Deferred Demo".into(),
                    true,
                );
                "@tploc=n".to_owned()
            } else if scenario == "maturity-blocked" {
                self.world.maturity.request(2, 0);
                self.request_user_teleport(NetCommand::TeleportHome, aurora_i18n::tr!("common-home").into(), false);
                "@tplm=n".to_owned()
            } else if scenario == "chat-home" {
                self.settings.chat_commands.enabled = true;
                self.settings.chat_commands.teleport_home = "tph".into();
                "@tplm=n".to_owned()
            } else if matches!(scenario, "chat" | "chat-silent") {
                self.panels.settings = false;
                self.panels.inventory = false;
                self.panels.chat = true;
                self.chat_ui.selected = None;
                self.settings.rlva.show_ellipsis = scenario == "chat";
                "@recvchat=n,recvemote=n".to_owned()
            } else if scenario.starts_with("version") {
                if scenario == "version-compat" {
                    self.world.rlva_demo_version_seed();
                    self.settings.rlva.compatibility_list = format!("creator:{}", Uuid::from_u128(0x726c766163726561));
                }
                "@notify:123=n".to_owned()
            } else if scenario.starts_with("experience") {
                self.world
                    .rlva_demo_experience(if scenario == "experience-allowed" { 21 } else { 13 }, false);
                "@notify:123=n".to_owned()
            } else if scenario.starts_with("shared-offer") {
                self.world.rlva_demo_shared_offer(false);
                "@notify:123;inv_offer=n".to_owned()
            } else if scenario.starts_with("privacy") || scenario == "nearby" {
                self.panels.settings = false;
                self.panels.inventory = false;
                self.panels.chat = true;
                if scenario == "nearby" {
                    self.panels.people = true;
                    self.panels.people_tab = 0;
                    "@shownearby=n".to_owned()
                } else if scenario == "privacy-except" {
                    let speaker = self
                        .world
                        .chat
                        .iter()
                        .rev()
                        .find(|line| !line.source.is_nil() && line.source != self.world.agent_id)
                        .map_or(crate::demo::DEMO_LOUP, |line| line.source);
                    format!("@shownames_sec=n,shownames_sec:{speaker}=n,showloc=n")
                } else {
                    "@shownames=n,showloc=n".to_owned()
                }
            } else if let Some(effect) = scenario.strip_prefix("sphere-") {
                let (number, params, strength) = match effect {
                    "blur" => (1, "0/0/0/0", 8.0),
                    "variable" => (2, "15/0/0/0", 4.0),
                    "chromatic" => (3, "12/0/-12/0", 1.0),
                    "pixel" => (4, "16/16/0/0", 1.0),
                    _ => (0, "0/0/0/0", 0.9),
                };
                self.panels.settings = false;
                format!(
                    "@setsphere=n,setsphere_mode:{number}=force,setsphere_origin:1=force,setsphere_distmin:2=force,setsphere_distmax:15=force,setsphere_valuemin:0=force,setsphere_valuemax:{strength}=force,setsphere_param:{params}=force"
                )
            } else if scenario == "resolution" {
                self.panels.settings = false;
                "@setdebug_renderresolutiondivisor:12=force".to_owned()
            } else if scenario == "overlay" {
                self.panels.settings = false;
                format!(
                    "@setoverlay=n,setoverlay_texture:{}=force,setoverlay_alpha:0.7=force,setoverlay_touch=n",
                    crate::demo::TEX_HOLES
                )
            } else if matches!(scenario, "textures" | "silhouette" | "self" | "head") {
                self.panels.settings = false;
                match scenario {
                    "textures" => format!("@setcam_textures:{}=n", crate::demo::TEX_GRADIENT),
                    "silhouette" => "@camavdist:0=n".to_owned(),
                    "self" => "@showself=n".to_owned(),
                    _ => "@showselfhead=n".to_owned(),
                }
            } else {
                "@fly=n,jump=n,unsit=n,sendchat=n,pay=n".to_owned()
            };
            self.rlva_demo_message(message);
            log::info!(
                "RLVa demo: mode={mode}, enabled={}, fly={}, jump={}, unsit={}, sendchat={}",
                self.world.rlva.enabled(),
                self.world.rlva.has("fly"),
                self.world.rlva.has("jump"),
                self.world.rlva.has("unsit"),
                self.world.rlva.has("sendchat")
            );
        }
        if mode == "sitting" && self.frame_count == 740 {
            self.rlva_demo_message("@sitground=force".to_owned());
        }
        if mode == "rename" && self.frame_count == 740 {
            self.world.rlva_demo_naming_attach();
        }
        if mode == "rename" && self.frame_count == 810 {
            self.world.rlva_demo_naming_report();
        }
        if mode == "shared-offer-accepted" && self.frame_count == 740 {
            let id = self.world.notifications.list.iter().find(|notification| matches!(notification.data, crate::world::notifications::Data::Inventory { tx, .. } if tx == Uuid::from_u128(0x726c76616f66667478))).map(|notification| notification.id);
            if let Some(id) = id {
                let (commands, _) = self.world.respond_notification(id, crate::world::notifications::Response::Accept);
                for command in commands {
                    self.send(command);
                }
                self.world.rlva_demo_shared_offer(true);
            }
        }
        if mode == "shared-offer-accepted" && self.frame_count == 810 {
            self.world.rlva_demo_shared_offer_report();
        }
        if mode.starts_with("experience") && self.frame_count == 740 {
            self.world.rlva_demo_experience(0, true);
        }
        if mode.starts_with("experience") && self.frame_count == 810 {
            self.world.rlva_demo_experience_report();
        }
        if matches!(mode.as_str(), "chat" | "chat-silent") && self.frame_count == 740 {
            self.world.rlva_demo_chat();
        }
        if mode == "chat-home" && self.frame_count == 740 {
            let handled = self.chat_command("tph");
            log::info!(
                "RLVa home shortcut demo: handled={handled}, requested={}, pending_ui={}",
                self.world.teleporting,
                self.tp_dest.is_some()
            );
        }
        if matches!(mode.as_str(), "tools" | "tools-closed") && self.frame_count == 740 {
            self.world.rlva_console_input("@fly=n,fly=n,fly=y,fly=n,version=0,getstatusall=0");
        }
        if mode == "named-blocked" && self.frame_count == 740 {
            let (x, y) = self.world.main_region.map(aurora_net::handle_to_origin).unwrap_or((0, 0));
            self.on_app_event(NetEvent::TeleportRegionResolved {
                request: Uuid::from_u128(0x726c76616e616d65),
                result: Ok(aurora_net::origin_to_handle(x.saturating_add(4096), y.saturating_add(4096))),
            });
        }
        if mode == "maturity-blocked" && self.frame_count == 740 {
            self.world.apply(NetEvent::MaturityPreference(Some(2)));
        }
        if matches!(mode.as_str(), "named-blocked" | "maturity-blocked") && self.frame_count == 810 {
            log::info!(
                "RLVa deferred teleport demo: mode={mode}, requested={}, pending_ui={}, failed={}",
                self.world.teleporting,
                self.tp_dest.is_some(),
                self.world.tp_failed
            );
        }
        if mode == "tools-closed" && self.frame_count == 810 {
            self.world.rlva.close_console();
            self.options_ui.rlva_tools.open = false;
            log::info!(
                "RLVa console demo: closed, fly={}, other_jump={}",
                self.world.rlva.has("fly"),
                self.world.rlva.has("jump")
            );
        }
        if mode == "tools" && self.frame_count == 810 {
            log::info!(
                "RLVa console demo: entries={}, replies={}, network_replies={}",
                self.world.rlva.diagnostics.entries.len(),
                self.world
                    .rlva
                    .diagnostics
                    .entries
                    .iter()
                    .filter(|entry| entry.outcome == crate::world::rlva::diagnostics::Outcome::Reply)
                    .count(),
                self.world
                    .rlva
                    .out
                    .iter()
                    .filter(|command| matches!(command, NetCommand::ChatReply { channel: 0, .. }))
                    .count()
            );
        }
        if mode.starts_with("version") && self.frame_count == 740 {
            self.rlva_demo_message("@version=123,versionnew=124,versionnum=125,versionnum:impl=126".into());
            self.world.rlva_demo_version_report();
        }
        if mode == "im-query" && self.frame_count == 740 {
            self.world.apply(NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: crate::demo::DEMO_TESS,
                from_name: "Tess Example".into(),
                to_agent_id: self.world.agent_id,
                dialog: 0,
                session_id: Uuid::nil(),
                message: "@list".into(),
                offline: false,
                from_group: false,
                binary_bucket: vec![],
                timestamp: 0,
            }));
        }
        if mode == "communication" && self.frame_count == 740 {
            let unopened = Uuid::from_u128(0x726c76616e6577696d);
            self.on_ctx_action(ui::context::CtxAction::Im(unopened));
            let new_session_blocked = !self.world.social.has_session(&unopened);
            let from = crate::demo::DEMO_TESS;
            self.world.apply(NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: from,
                from_name: "Tess Example".into(),
                to_agent_id: self.world.agent_id,
                dialog: 0,
                session_id: Uuid::from_u128(self.world.agent_id.as_u128() ^ from.as_u128()),
                message: "Private demo message".into(),
                offline: false,
                from_group: false,
                binary_bucket: vec![],
                timestamp: 0,
            }));
            self.world.own_im(from, "Private demo reply");
            self.send(NetCommand::SendIm {
                to: from,
                message: "Private demo reply".into(),
            });
            let blocked = self.world.social.ims.iter().find(|im| im.other == from).is_some_and(|im| {
                im.lines.iter().rev().take(2).all(|line| {
                    line.text == aurora_i18n::tr!("rlva-im-blocked-receive") || line.text == aurora_i18n::tr!("rlva-im-blocked-send")
                })
            });
            log::info!(
                "RLVa communication demo: new_session_blocked={new_session_blocked}, local_placeholders={blocked}, existing_session_allowed={}",
                self.world.rlva_can_start_im(from)
            );
        }
        if mode == "temporary" && self.frame_count == 740 {
            self.settings.rlva.temporary_controllers = false;
            self.world.rlva.preferences.temporary_controllers = false;
            let source = Uuid::from_u128(0x726c766174656d70);
            let object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: source,
                owner_id: self.world.agent_id,
                point: 1,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            });
            self.world.apply(NetEvent::ObjectUpdates {
                handle: self.world.main_region.unwrap_or_default(),
                objects: vec![object],
            });
            self.world.apply(NetEvent::Chat(aurora_net::ChatMessage {
                from_name: "Temporary controller".into(),
                source_id: source,
                owner_id: self.world.agent_id,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: Vec3::ZERO,
                message: "@jump=n,version=123".into(),
            }));
            log::info!(
                "RLVa temporary demo: option_off_jump={}, source_allowed={}",
                self.world.rlva.has("jump"),
                self.world.rlva_controller_chat_allowed(source)
            );
        }
        if mode == "hud-lock" && self.frame_count == 810 {
            log::info!(
                "RLVa HUD demo: locked={}, shown={}, wireframe={}",
                self.world.rlva_has_locked_hud(),
                self.huds_shown(),
                self.settings.debug.wireframe
            );
        }
        if mode == "sitting" && self.frame_count == 810 {
            log::info!(
                "RLVa demo: ground_sit={}, object_seated={}",
                self.world.agent.ground_sit,
                self.world.agent.seated
            );
        }
        if (mode == "released" || mode.ends_with("-released")) && self.frame_count == 760 {
            self.settings.rlva_enabled = false;
            self.world.rlva_configure(false);
            log::info!(
                "RLVa demo: disabled, all restrictions released={}",
                !self.world.rlva.has("fly") && !self.world.rlva.has("sendchat")
            );
        }
    }

    pub(super) fn rlva_demo_message(&mut self, message: String) {
        self.world.apply(NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "RLVa demo object".to_owned(),
            source_id: Uuid::from_u128(0x726c7661),
            owner_id: self.world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: Vec3::ZERO,
            message,
        }));
    }

    fn send_rlva(&mut self, command: NetCommand, actor: Uuid) {
        self.send_from(command, actor);
    }

    fn rlva_environment_asset(&mut self) {
        use crate::scene::settings::SettingsState;
        use crate::world::{eep::Settings as Environment, env_select::SettingsKind, inventory::FetchState};
        let Some((actor, command)) = self.world.rlva.environment_request.take() else {
            return;
        };
        if !self.world.rlva.enabled() || self.world.rlva.has_except("setenv", actor) {
            return;
        }
        let asset = if let Ok(id) = Uuid::parse_str(&command.option) {
            Some(id)
        } else {
            let root = self.world.inventory.lib_root;
            let environments = self
                .world
                .inventory
                .folders
                .values()
                .find(|f| f.library && f.info.parent == root && f.info.name.eq_ignore_ascii_case("Environments"))
                .map(|f| f.info.id);
            let mut stack = environments.map_or_else(|| vec![root], |id| vec![id]);
            let mut seen = std::collections::HashSet::new();
            let mut loading = Vec::new();
            let mut asset = None;
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(folder) = self.world.inventory.folders.get(&id) else {
                    continue;
                };
                if folder.state != FetchState::Fetched {
                    loading.push(id);
                }
                if environments.is_some() {
                    stack.extend(folder.children.iter().copied());
                }
                let kind = if command.behaviour == "setenv_daycycle" {
                    SettingsKind::Day
                } else {
                    SettingsKind::Sky
                };
                asset = asset.or_else(|| {
                    folder
                        .items
                        .iter()
                        .filter_map(|id| self.world.inventory.items.get(id))
                        .find(|it| {
                            it.asset_type == 56
                                && SettingsKind::from_flags(it.flags) == Some(kind)
                                && it.name.eq_ignore_ascii_case(&command.option)
                        })
                        .map(|it| it.asset_id)
                });
            }
            if asset.is_none() && !loading.is_empty() {
                for id in loading.into_iter().take(16) {
                    self.world.inventory.request(id);
                }
                self.world.rlva.environment_request = Some((actor, command));
                return;
            }
            asset
        };
        let Some(asset) = asset.filter(|id| !id.is_nil()) else {
            return;
        };
        match self.scene.settings.get(asset) {
            SettingsState::Pending => self.world.rlva.environment_request = Some((actor, command)),
            SettingsState::Failed => {}
            SettingsState::Ready(settings) => {
                let (sky, water) = self.world.rlva_environment_frames(self.world.eep.current());
                self.world.rlva.environment = Some((actor, sky));
                match settings.as_ref() {
                    Environment::Sky(sky) => {
                        self.world.rlva.environment = Some((actor, *sky));
                        self.world.rlva.environment_day = None;
                        self.world.rlva.environment_water = Some(water);
                    }
                    Environment::Water(water) => self.world.rlva.environment_water = Some(*water),
                    Environment::Day(day) => {
                        self.world.rlva.environment_day = Some(std::sync::Arc::new(day.clone()));
                        self.world.rlva.environment_water = None;
                    }
                }
            }
        }
    }

    fn rlva_camera_commands(&mut self) {
        use crate::world::rlva::{
            Param,
            camera::{FocusRequest, FocusTarget},
        };
        for (actor, c) in std::mem::take(&mut self.world.rlva.viewer) {
            if c.behaviour.starts_with("getdebug_") {
                if let Param::Reply(channel) = c.param {
                    let text = match c.behaviour.as_str() {
                        "getdebug_avatarsex" => self.world.rlva.pseudo_sex.as_deref().map_or_else(
                            || {
                                self.world
                                    .visual_params(&self.world.agent_id)
                                    .map(|values| {
                                        u8::from(self.world.avatar_lib.shape_params.weights(values).get(&80).copied().unwrap_or(0.0) > 0.5)
                                            .to_string()
                                    })
                                    .unwrap_or_default()
                            },
                            str::to_owned,
                        ),
                        "getdebug_aspectratio" => {
                            let rect = self.egui_ctx.content_rect();
                            format!("{:.3}", rect.width() / rect.height().max(1.0))
                        }
                        "getdebug_renderresolutiondivisor" => self.world.rlva.resolution_divisor.unwrap_or(1).to_string(),
                        "getdebug_windlightuseatmosshaders" => "1".to_owned(),
                        "getdebug_restrainedloveforbidgivetorlv" => u8::from(!self.settings.rlva.allow_shared_offers).to_string(),
                        "getdebug_restrainedlovenosetenv" => u8::from(!self.settings.rlva.allow_environment_locks).to_string(),
                        _ => String::new(),
                    };
                    self.world.rlva.reply(channel, text);
                }
                continue;
            }
            if self.world.rlva.debug_force(actor, &c) {
                continue;
            }
            if c.behaviour == "getheightoffset" {
                if c.option.is_empty()
                    && let Param::Reply(channel) = c.param
                {
                    self.world
                        .rlva
                        .reply(channel, format!("{:.2}", self.world.hover_height.value() * 100.0));
                }
                continue;
            }
            if c.behaviour == "adjustheight" {
                if self.world.hover_height_available()
                    && let Some(height) = crate::world::rlva::camera::height_offset(&c.option, self.pelvis_to_foot())
                {
                    self.settings.auto_hover_height = false;
                    self.world.hover_height.preview(height);
                    self.world.hover_height.commit();
                }
                continue;
            }
            if let Param::Reply(channel) = c.param {
                let fov = self.world.rlva.camera_settings(&self.settings.camera, self.camera_aspect()).fov;
                if let Some(reply) = self
                    .world
                    .rlva
                    .camera_reply(&c, self.camera.position.distance(self.world.agent.position), fov)
                {
                    self.world.rlva.reply(channel, reply);
                }
                continue;
            }
            if self.world.rlva.camera_force(actor, &c) {
                continue;
            }
            match c.behaviour.as_str() {
                "setcam_mode" => match c.option.as_str() {
                    "mouselook"
                        if self.world.rlva.camera_value("setcam_mouselook").is_none()
                            && self.world.rlva.camera_limit("setcam_avdistmin", 0.0, true) <= 0.0 =>
                    {
                        self.camera.change_to_mouselook(&mut self.world.agent, &self.settings.camera, true)
                    }
                    "thirdperson" | "thirdview" => self.camera.change_to_default(&mut self.world.agent, &self.settings.camera),
                    "reset" | "" => self
                        .camera
                        .reset_view(&mut self.world.agent, &self.settings.camera, true, true, false, false),
                    _ => {}
                },
                "setcam_focus" => {
                    let Some(request) = FocusRequest::parse(&c.option) else {
                        continue;
                    };
                    let now = Instant::now();
                    let (focus, object) = match request.target {
                        FocusTarget::Position(position) if self.world.main_region.is_some() => (position, None),
                        FocusTarget::Position(_) => continue,
                        FocusTarget::Object(id) => {
                            let Some((i, o)) = self
                                .world
                                .objects
                                .index_of_uuid(&id)
                                .and_then(|i| self.world.objects.get(i).map(|o| (i, o)))
                            else {
                                continue;
                            };
                            let Some((position, rotation, hud)) = crate::scene::Scene::object_transform_unshifted(&self.world, i, now)
                            else {
                                continue;
                            };
                            if hud {
                                continue;
                            }
                            let geom = crate::camera::FocusObject {
                                position,
                                rotation,
                                scale: o.scale,
                                avatar: o.is_avatar(),
                                mesh: o.volume.is_mesh(),
                            };
                            let focus = if o.is_avatar() {
                                self.scene.head_world(&self.world, &id, now).unwrap_or(position)
                            } else {
                                position
                            };
                            (focus, Some((id, geom)))
                        }
                    };
                    let Some(camera) = request.camera(focus, object.map_or(Vec3::ZERO, |(_, o)| o.scale), self.camera.position) else {
                        continue;
                    };
                    self.camera
                        .set_script_focus(camera, focus, object, &mut self.world.agent, &self.settings.camera);
                }
                _ => {}
            }
        }
    }
}
