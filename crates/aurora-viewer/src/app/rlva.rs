//! RLVa preference and action integration, shared by every UI layout.
use super::own_motion::temprun_resync;
use super::*;
use uuid::Uuid;

/// The mini-map window across @showminimap, as the static `fPrevVisibile` of
/// RlvUIEnabler::onToggleShowMinimap (rlvui.cpp): hidden while the
/// restriction holds, shown again at its release only if it was shown when
/// the restriction came. The world map (onToggleShowWorldMap) and the
/// inventory (RlvBehaviourToggleHandler<RLV_BHVR_SHOWINV>::onCommandToggle,
/// rlvhandler.cpp) are closed and stay closed after the release.
#[derive(Default)]
pub(super) struct HiddenMinimap(Option<bool>);

impl HiddenMinimap {
    pub(super) fn apply(&mut self, restricted: bool, open: &mut bool) {
        match (restricted, self.0) {
            (true, None) => self.0 = Some(std::mem::take(open)),
            (true, Some(_)) => *open = false,
            (false, Some(was_open)) => {
                self.0 = None;
                *open |= was_open;
            }
            (false, None) => {}
        }
    }
}

/// The windows @showinv, @showminimap and @showworldmap hide never show,
/// whatever asked for them: run every frame, and again once the bars have
/// drawn, whose buttons and menus open a window in the frame it is drawn.
/// @edit hides the beacons window and refuses it, without a notice
/// (RlvBehaviourToggleHandler<RLV_BHVR_EDIT>::onCommandToggle,
/// rlvhandler.cpp: addGenericFloaterFilter("beacons")).
pub(super) fn keep_windows_closed(rlva: &crate::world::rlva::Rlva, panels: &mut ui::Panels) {
    panels.inventory &= !rlva.has("showinv");
    panels.minimap &= !rlva.has("showminimap");
    panels.world_map &= !rlva.has("showworldmap");
    panels.beacons &= !rlva.has("edit");
}

/// RlvBehaviourToggleHandler<RLV_BHVR_EDIT>::onCommandToggle: build mode is
/// left as @edit comes, not while it lasts.
fn leaves_build_mode(edit_before: bool, edit_now: bool) -> bool {
    edit_now && !edit_before
}

/// RlvUtil::notifyBlockedViewXXX (rlvcommon.h): what is said of a preview
/// RLVa keeps closed, by asset type (7 a notecard, 10 a script, else a
/// texture).
fn view_blocked_notice(asset_type: i32) -> String {
    match asset_type {
        7 => aurora_i18n::tr!("rlva-blocked-view-notecard").into(),
        10 => aurora_i18n::tr!("rlva-blocked-view-script").into(),
        _ => aurora_i18n::tr!("rlva-blocked-view-texture").into(),
    }
}

/// RlvActions::canHighlightTransparent (rlvactions.cpp) refuses the
/// highlight under @edit and @showloc, and the @viewtransparent toggle
/// handler (rlvhandler.cpp) turns it off: the three keep it off here.
fn can_highlight_transparent(rlva: &crate::world::rlva::Rlva) -> bool {
    !["viewtransparent", "edit", "showloc"].iter().any(|b| rlva.has(b))
}

/// A shortcut the restrictions refuse outright, with no notice:
/// RlvUIEnabler::filterFloaterGeneric (rlvui.cpp) validates the inventory,
/// mini-map and world map floaters without a message, and
/// LLViewHighlightTransparent (llviewermenu.cpp) leaves the highlight off.
fn refuses_shortcut(rlva: &crate::world::rlva::Rlva, action: Action) -> bool {
    match action {
        Action::Inventory | Action::NewInventoryWindow => rlva.has("showinv"),
        Action::Minimap => rlva.has("showminimap"),
        Action::WorldMap => rlva.has("showworldmap"),
        Action::ShowTransparency => !can_highlight_transparent(rlva),
        Action::Beacons => rlva.has("edit"),
        _ => false,
    }
}

impl App {
    /// RlvUIEnabler::filterFloaterViewXXX (rlvui.cpp): a preview that
    /// @viewnote, @viewscript or @viewtexture keeps closed says so. Returns
    /// whether it was refused.
    pub(super) fn rlva_preview_refused(&mut self, asset_type: i32) -> bool {
        let refused = !self.world.inventory.can_preview(asset_type);
        if refused {
            self.rlva_view_blocked(asset_type);
        }
        refused
    }

    /// RlvUtil::notifyBlockedViewXXX, also for the notecards and scripts of a
    /// locked attachment (LLTaskNotecardBridge::openItem, LLTaskLSLBridge::
    /// openItem, llpanelobjectinventory.cpp).
    pub(super) fn rlva_view_blocked(&mut self, asset_type: i32) {
        self.world
            .system_notice(view_blocked_notice(asset_type), crate::world::notice_topics::Topic::Warning);
    }

    /// A texture library preview, as `rlva_preview_refused` lets it open.
    pub(super) fn open_library_preview(&mut self, ctx: &egui::Context, item: Uuid) {
        let kind = self.world.inventory.items.get(&item).map(|it| it.asset_type);
        if kind.is_some_and(|kind| self.rlva_preview_refused(kind)) {
            return;
        }
        self.texture_library.open_preview(ctx, &self.world, item);
    }

    pub(super) fn camera_aspect(&self) -> f32 {
        self.gfx
            .as_ref()
            .map(|g| g.renderer.size())
            .filter(|&(w, h)| w > 0 && h > 0)
            .map_or(1.0, |(w, h)| w as f32 / h as f32)
    }
    pub(super) fn rlva_overlay_blocks(&self) -> bool {
        let size = self.gfx.as_ref().map(|g| g.renderer.size());
        let Some((w, h)) = size.filter(|&(w, h)| w > 0 && h > 0) else {
            return false;
        };
        let (u, v) = (self.cursor_pos.0 / w as f32, self.cursor_pos.1 / h as f32);
        self.world
            .rlva_overlay_frames(Instant::now())
            .iter()
            .any(|frame| frame.blocks_touch && self.ui_image_masks.get(&frame.texture).is_none_or(|mask| mask.hit(u, v)))
    }

    /// LLStartUp's state for RLVa: started when the login begins, its
    /// commands retained until the world is shown (STATE_STARTED).
    fn rlva_phase(&self) -> crate::world::rlva::startup::Phase {
        use crate::world::rlva::startup::Phase;
        match self.screen {
            super::Screen::Login => Phase::Login,
            super::Screen::Loading { .. } => Phase::Loading,
            super::Screen::World => Phase::Started,
        }
    }

    /// RLVa on or off and its settings, before a command is checked: the
    /// ones read once keep the values RLVa started with.
    pub(super) fn rlva_sync_settings(&mut self) {
        self.world.rlva_configure(self.settings.rlva_enabled);
        let phase = self.rlva_phase();
        self.world.rlva.apply_preferences(&self.settings.rlva, phase);
    }

    /// The arrival of a teleport that keeps the camera's direction (the
    /// @standtp return): the Legacy Viewer detached the camera when it asked
    /// (LLAgent::doTeleportViaLocationLookAt, setFocusOnAvatar(false)), so
    /// the arrival's resetView turns the avatar to where the camera looks
    /// (LLAgentCamera::setFocusOnAvatar, process_teleport_local; the
    /// region-change arrival takes the camera's at-axis,
    /// process_agent_movement_complete). An arrival elsewhere (another
    /// teleport, a failed one) drops it.
    pub(super) fn keep_look_at_on_arrival(&mut self) {
        let Some(target) = self.tp_keeps_look_at.take() else { return };
        if self.world.agent.position.truncate().distance(target.truncate()) > 2.0 {
            return;
        }
        let look = self.camera.forward();
        self.camera.fold_orbit(&mut self.world.agent);
        self.world.agent.set_look_at(glam::Vec3::new(look.x, look.y, 0.0));
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
        let phase = self.rlva_phase();
        self.world.rlva_startup(phase, &self.settings.rlva, Instant::now());
        // RLVa off in session (Aurora only) drops the open @list / @except
        // questions; the IM queries option does not: it only stops new ones
        // (RlvHandler::processIMQuery), and a question already shown still
        // answers (RlvHandler::onIMQueryListResponse)
        if !self.settings.rlva_enabled {
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
        self.world.rlva_sync_environment_lock();
        self.rlva_environment_windows();
        self.rlva_camera_toggles();
        self.world.inventory.rlva_preview_mask = self.world.rlva.preview_mask();
        // (@viewnote, @viewscript and @viewtexture refuse to open a preview,
        // RlvUIEnabler::filterFloaterViewXXX in rlvui.cpp; one already open
        // stays open and usable)
        // An outfit change is over once its detaches have landed too: the
        // COF write is acknowledged before the objects leave the avatar.
        self.world.rlva.appearance_busy = self.appearance_ui.pending.is_some() || self.world.rlva_detach_in_flight(Instant::now());
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
        for (actor, command) in self.world.rlva_sitting_tick() {
            self.send_from(command, actor);
        }
        for (actor, command) in self.world.rlva_group_tick() {
            self.send_from(command, actor);
        }
        if let Some(command) = self.world.rlva_stand_return_command(self.camera.forward()) {
            if let NetCommand::TeleportTo { position, .. } = &command {
                self.tp_keeps_look_at = Some(*position);
            }
            self.send(command);
            self.world.rlva.stand_return = None;
        }
        // RlvUIEnabler::onToggleMovement (rlvui.cpp): @fly lands a flight
        // under way (gAgent.setFlying(false)), not only refuses a takeoff.
        // Before the forces: the rules came with them, a @fly:true=force
        // after its own @fly=n takes off once the lock has landed us.
        self.world.rlva_movement_tick();
        let commands = self.world.rlva_pending();
        self.rlva_environment_asset();
        self.rlva_camera_commands();
        if !self.world.rlva.appearance_busy && self.appearance_ui.pending.is_none() && !self.world.rlva.appearance.is_empty() {
            let (actor, action) = self.world.rlva.appearance.remove(0);
            if !self.world.rlva_force_settled(&action, Instant::now()) {
                self.apply_appearance_action_from(action, actor);
            }
        }
        for (actor, command) in commands {
            self.send_rlva(command, actor);
        }
        // Protocol replies are not user chat and must bypass chat redirection
        // and sendchannel, as RlvUtil::sendChatReply does.
        for command in std::mem::take(&mut self.world.rlva.out) {
            if let NetCommand::ChatReply { message, channel } = &command {
                self.rlva_selftest.runner.reply(*channel, message);
            }
            if matches!(command, NetCommand::ChatReply { .. } | NetCommand::ImReply { .. }) {
                self.net.send(command);
            } else {
                self.send(command);
            }
        }
        self.rlva_minimap
            .apply(self.world.rlva.has("showminimap"), &mut self.panels.minimap);
        keep_windows_closed(&self.world.rlva, &mut self.panels);
        if self.world.rlva.has("showinv") {
            self.inventory_ui.close_additional_window();
        }
        if self.world.rlva.has("alwaysrun") && self.always_run {
            self.set_always_run(false);
        }
        // RlvUIEnabler::onToggleMovement: clearTempRun tells the simulator,
        // which would otherwise keep running with nothing left to stop it.
        let temprun = self.world.rlva.has("temprun");
        let ended = temprun && self.temp_run.take().is_some();
        let changed = temprun != std::mem::replace(&mut self.rlva_temprun, temprun);
        if temprun_resync(ended, changed, self.autopilot.as_ref().is_some_and(|a| a.run)) {
            self.send(NetCommand::SetAlwaysRun(self.walk_run()));
        }
        // RlvBehaviourToggleHandler<RLV_BHVR_EDIT>::onCommandToggle
        // (rlvhandler.cpp): @edit leaves build mode as it comes
        // (LLToolMgr::leaveBuildMode); the tools may open again after, as
        // far as RlvActions::canBuild allows.
        let edit = self.world.rlva.has("edit");
        if leaves_build_mode(std::mem::replace(&mut self.rlva_edit, edit), edit) && self.build.open {
            self.build.close(&mut self.world, &mut self.settings.build);
        }
        if self
            .interactions
            .contents
            .as_ref()
            .is_some_and(|c| !self.world.rlva_keeps_contents_open(c.target.object))
        {
            self.interactions.contents = None;
        }
        // LLSelectMgr::deselectAllIfTooFar: what RLVa keeps from edits
        // leaves the selection, the temporary one of a menu excepted
        let menu = self.build.menu_selection;
        let invalid: Vec<_> = self
            .build
            .selection
            .iter()
            .filter(|key| {
                self.world.objects.index_of(key).is_none_or(|i| {
                    if menu {
                        !crate::build::rlva_keeps_menu_selection(&self.world, i)
                    } else {
                        !crate::build::rlva_selectable(&self.world, i)
                    }
                })
            })
            .copied()
            .collect();
        if !invalid.is_empty() {
            self.build.drop_selected(&self.world, &invalid);
        }
        let locked_hud = self.world.rlva_has_locked_hud();
        if locked_hud {
            self.settings.show_huds = true;
        }
        self.world.rlva_limit_hud_zoom(locked_hud);
        if self.world.rlva.has("viewwireframe") || locked_hud {
            self.settings.debug.wireframe = false;
        }
        if !self.rlva_can_highlight_transparent() {
            self.settings.debug.show_alpha = false;
        }
    }

    pub(super) fn rlva_can_highlight_transparent(&self) -> bool {
        can_highlight_transparent(&self.world.rlva)
    }

    pub(super) fn rlva_refuses_shortcut(&self, action: Action) -> bool {
        refuses_shortcut(&self.world.rlva, action)
    }

    fn demo_rlva(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_RLVA") else {
            return;
        };
        // (the self-test's own scenario: app/rlva_selftest.rs)
        if mode == "selftest" {
            return;
        }
        // Native updates may run twice before the next rendered frame.
        // A fixture's force action must be delivered only once per frame.
        if self.demo_rlva_frame == Some(self.frame_count) {
            return;
        }
        self.demo_rlva_frame = Some(self.frame_count);
        if self.frame_count == 360 {
            self.settings.rlva_enabled = false;
            self.panels.settings = true;
            self.options_ui.page = ui::options::Page::Rlva;
            self.options_ui.tab = 0;
        }
        if self.frame_count == 700 {
            self.settings.rlva_enabled = mode != "disabled";
            self.world.rlva_configure(self.settings.rlva_enabled);
            let scenario = mode.strip_suffix("-released").unwrap_or(&mode);
            if scenario == "baseline"
                || scenario == "ground-wear"
                || scenario.starts_with("sphere-")
                || scenario.starts_with("camera-")
                || matches!(
                    scenario,
                    "overlay" | "textures" | "textures-grey" | "silhouette" | "self" | "head" | "resolution" | "interact" | "fartouch"
                )
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
            } else if scenario == "interact" || scenario == "fartouch" {
                self.panels.settings = false;
                format!("@{scenario}=n")
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
            } else if let Some(limit) = scenario.strip_prefix("camera-zoom-") {
                // the camera starts 7.6 m from the focus point, which is
                // over the avatar: about 6 m from where the avatar's own
                // view would put it (the origin) and 7 m from the head
                self.panels.settings = false;
                match limit {
                    "max" => "@setcam_origindistmax:8=n".to_owned(),
                    // the origin limit alone holds the camera: the
                    // avatar's is left out unless its two ends are equal
                    "both" => "@setcam_avdistmax:5=n,setcam_origindistmax:8=n".to_owned(),
                    // the avatar's limits made equal hold it alone
                    "locked" => "@setcam_avdistmin:5=n,setcam_avdistmax:5=n,setcam_origindistmax:8=n".to_owned(),
                    _ => "@setcam_origindistmin:5=n".to_owned(),
                }
            } else if matches!(scenario, "offset-zoom" | "no-mouselook" | "avatar-zoom") {
                // the avatar's own view, with nothing over it
                self.panels.settings = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                match scenario {
                    // 1.5 times a 6.08 m offset: 9.12 m behind the avatar
                    "offset-zoom" => "@setcam_eyeoffset:-6/0/1=n,setcam_eyeoffsetscale:1.5=n".to_owned(),
                    "no-mouselook" => "@camdistmin:1=n".to_owned(),
                    _ => "@camdistmax:5=n".to_owned(),
                }
            } else if let Some(run) = scenario.strip_prefix("run-") {
                self.panels.settings = false;
                match run {
                    "temprun" => "@temprun=n".to_owned(),
                    "alwaysrun" => "@alwaysrun=n".to_owned(),
                    "both" => "@temprun=n,alwaysrun=n".to_owned(),
                    _ => "@clear".to_owned(),
                }
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
            } else if scenario == "ground-wear" {
                self.panels.settings = false;
                "@detach=n".to_owned()
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
                self.options_ui.rlva_tools.tab = ui::rlva::Tab::Console;
                self.settings.rlva.debug_commands = true;
                "@jump=n,detach=n,fly=n,fly=n,fly=y".to_owned()
            } else if let Some(tab) = match scenario {
                "tools-restrictions" => Some(ui::rlva::Tab::Restrictions),
                "tools-exceptions" => Some(ui::rlva::Tab::Exceptions),
                "tools-modifiers" => Some(ui::rlva::Tab::Modifiers),
                _ => None,
            } {
                self.panels.settings = false;
                self.panels.chat = false;
                self.panels.inventory = false;
                self.options_ui.rlva_tools.open = true;
                self.options_ui.rlva_tools.tab = tab;
                let (tess, loup) = (crate::demo::DEMO_TESS, crate::demo::DEMO_LOUP);
                format!(
                    "@jump=n,detach=n,recvim=n,recvim:{tess}=add,sendim:{loup}=add,tplure:{tess}=add,shownametags:{loup}=add,\
                     recvim:20;50=n,fartouch:2.5=n,sittp=n,setcam_avdistmax:10=n,setcam_textures=n"
                )
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
            } else if matches!(scenario, "textures" | "textures-grey" | "silhouette" | "self" | "head") {
                self.panels.settings = false;
                match scenario {
                    "textures" => format!("@setcam_textures:{}=n", crate::demo::TEX_GRADIENT),
                    // Empty @setcam_textures is IMG_DEFAULT, the local flat grey.
                    "textures-grey" => "@setcam_textures=n".to_owned(),
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
        if let Some(limit) = mode.strip_suffix("-released").unwrap_or(&mode).strip_prefix("camera-zoom-") {
            // five wheel clicks past the distance limit, then one back
            let out = if limit == "min" { -1.0 } else { 1.0 };
            let clicks = match self.frame_count {
                n @ 740..=788 if n % 12 == 8 => out,
                810 => -out,
                _ => 0.0,
            };
            if clicks != 0.0 {
                self.camera
                    .scroll(clicks, false, false, &mut self.world.agent, &mut self.settings.camera);
            }
            if matches!(self.frame_count, 730 | 800 | 840) {
                let (head, origin) = self.camera.rlva_distances();
                log::info!(
                    "RLVa demo: {mode}, frame {}, camera to its origin {origin:.3} m, to the head {head:.3} m, to focus {:.3} m",
                    self.frame_count,
                    self.camera.position.distance(self.camera.target),
                );
            }
        }
        if mode == "offset-zoom" {
            // four wheel clicks out, then four back in, through the window's
            // own wheel path (#3372)
            match self.frame_count {
                n @ 740..=776 if n % 12 == 8 => self.camera_wheel(1.0),
                n @ 800..=836 if n % 12 == 8 => self.camera_wheel(-1.0),
                _ => {}
            }
        }
        if mode == "no-mouselook" {
            // the M key, then the wheel all the way in (#3357)
            match self.frame_count {
                740 => self.toggle_mouselook(),
                n @ 752..=860 if n % 6 == 2 => self.camera_wheel(-1.0),
                _ => {}
            }
        }
        if mode == "avatar-zoom" {
            // four wheel clicks out, two of them past the limit, then one
            // back in (#3356)
            match self.frame_count {
                n @ 740..=776 if n % 12 == 8 => self.camera_wheel(1.0),
                800 => self.camera_wheel(-1.0),
                _ => {}
            }
        }
        if matches!(mode.as_str(), "offset-zoom" | "no-mouselook" | "avatar-zoom") && matches!(self.frame_count, 730 | 741 | 790 | 880) {
            log::info!(
                "RLVa demo: {mode}, frame {}, camera to focus {:.3} m, to the head {:.3} m, mouselook={}, cursor held={}",
                self.frame_count,
                self.camera.position.distance(self.camera.target),
                self.camera.rlva_distances().0,
                self.camera.mouselook(),
                self.cursor_hold.held()
            );
        }
        if mode.starts_with("run-") {
            // A double tap through the real input path, held until frame 750;
            // `run-autopilot` runs to a point and @temprun comes and goes;
            // `run-autopilot-always` has Always run on and @alwaysrun comes.
            let autopilot = mode.starts_with("run-autopilot");
            let always = mode == "run-autopilot-always";
            match self.frame_count {
                715 if always => self.set_always_run(true),
                720 if autopilot => self.walk_to(self.world.agent.position + Vec3::X * 25.0, true),
                730 if always => self.rlva_demo_message("@alwaysrun=n".to_owned()),
                730 if autopilot => self.rlva_demo_message("@temprun=n".to_owned()),
                745 if autopilot && !always => self.rlva_demo_message("@temprun=y".to_owned()),
                720 => self.walk_tap(0, true),
                722 if !autopilot => {
                    // over NUDGE_TIME is no double tap: a slow frame, not a refusal
                    let gap = self.last_tap.map_or(0.0, |(_, at)| at.elapsed().as_secs_f32());
                    log::info!("RLVa demo: {mode} tap gap {gap:.3} s");
                    self.walk_tap(0, true);
                }
                721 | 750 if !autopilot => self.walk_tap(0, false),
                730 if mode == "run-late" => self.rlva_demo_message("@temprun=n".to_owned()),
                740 | 749 => log::info!("RLVa demo: {mode} temp_run={}, running={}", self.temp_run.is_some(), self.running()),
                _ => {}
            }
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
            self.world.rlva.close_console(self.world.agent_id);
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
                    line.text == aurora_i18n::tr!("rlva-im-blocked-receive")
                        || line.text
                            == crate::world::rlva::legacy_strings::remote(crate::world::rlva::legacy_strings::Remote::BlockedSendIm)
                })
            });
            log::info!(
                "RLVa communication demo: new_session_blocked={new_session_blocked}, local_placeholders={blocked}, existing_session_allowed={}",
                self.world.rlva_can_start_im(from)
            );
        }
        // An object worn from the ground: it attaches with an item the
        // inventory has not heard of, and keeps its own object id.
        let ground_item = Uuid::from_u128(0x726c7667);
        if mode == "ground-wear" && self.frame_count == 760 {
            let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: ground_item,
                owner_id: self.world.agent_id,
                point: 3,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            });
            object.full_id = Uuid::from_u128(0x726c7668);
            self.world.apply(NetEvent::ObjectUpdates {
                handle: self.world.main_region.unwrap_or_default(),
                objects: vec![object],
            });
        }
        if mode == "ground-wear" && self.frame_count == 860 {
            log::info!(
                "RLVa ground wear demo: detach={}, in_inventory={}, worn={}, rejected={}",
                self.world.rlva.has("detach"),
                self.world.inventory.items.contains_key(&ground_item),
                self.world.worn_attachment(&ground_item).is_some(),
                self.world.rlva.watchdog.rejected.contains(&ground_item),
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
                region: None,
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
            region: None,
            position: Vec3::ZERO,
            message,
        }));
    }

    fn send_rlva(&mut self, command: NetCommand, actor: Uuid) {
        self.send_from(command, actor);
    }

    /// RlvBehaviourToggleHandler<RLV_BHVR_SETENV> (rlvhandler.cpp) closes
    /// the environment floaters and keeps them from opening while
    /// `@setenv=n` holds: Personal Lighting (env_adjust_snapshot), the sky
    /// and water editors, the day cycle editor and My Environments. Aurora
    /// Weather lights the scene the same way, and goes with them.
    pub(super) fn rlva_environment_windows_locked(&self) -> bool {
        self.world.rlva.has("setenv")
    }

    fn rlva_environment_windows(&mut self) {
        if !self.rlva_environment_windows_locked() {
            return;
        }
        self.panels.personal_lighting = false;
        self.env_ui.weather_open = false;
        if self.env_editor.open {
            self.env_editor.close();
            self.world.eep.clear_edit();
        }
        if self.day_editor.open {
            self.day_editor.close();
            self.world.eep.clear_edit();
        }
    }

    /// RLVa's camera toggles, once a frame: `@setcam_unlock` arriving resets
    /// the view (RlvBehaviourToggleHandler<RLV_BHVR_SETCAM_UNLOCK>:
    /// handle_reset_view), RLVa's view taking over puts the zoom back and the
    /// focus on the avatar (setCameraOverride → switchCameraPreset), and the
    /// user's field of view comes back when the last fov limit goes
    /// (RlvBehaviourSetCamFovHandler, rlvhandler.cpp).
    fn rlva_camera_toggles(&mut self) {
        if !self.in_world() {
            return;
        }
        let (unlock, view) = self.world.rlva.camera_toggles();
        if view {
            self.camera.read_rlva(&self.world.rlva);
            self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
        }
        if unlock {
            self.camera.read_rlva(&self.world.rlva);
            self.reset_camera_view();
        }
        if self.world.rlva.track_fov_limits(&mut self.settings.camera.fov) && !self.demo {
            self.settings.save();
        }
    }

    fn rlva_environment_asset(&mut self) {
        use crate::scene::settings::SettingsState;
        use crate::world::{eep::Settings as Environment, env_select::SettingsKind};
        let Some((actor, command)) = self.world.rlva.environment_request.take() else {
            return;
        };
        if !self.world.rlva.enabled() || self.world.rlva.has_except("setenv", actor) {
            return;
        }
        let kind = if command.behaviour == "setenv_daycycle" {
            SettingsKind::Day
        } else {
            SettingsKind::Sky
        };
        let asset = match rlva_environment_lookup(&self.world.inventory, &command.behaviour, &command.option, kind) {
            EnvironmentLookup::Found(asset) => Some(asset),
            EnvironmentLookup::Missing => None,
            EnvironmentLookup::Loading(folders) => {
                for id in folders.into_iter().take(16) {
                    self.world.inventory.request(id);
                }
                self.world.rlva.environment_request = Some((actor, command));
                return;
            }
        };
        let Some(asset) = asset.filter(|id| !id.is_nil()) else {
            return;
        };
        match self.scene.settings.get(asset) {
            SettingsState::Pending => self.world.rlva.environment_request = Some((actor, command)),
            SettingsState::Failed => {}
            // LLEnvironment::setEnvironment(target, settings) by its type:
            // the user's ENV_LOCAL without a lock, ENV_EDIT under it, where a
            // fixed half keeps the instance's day and a day clears both
            // fixed halves (DayInstance::clear + setDay)
            SettingsState::Ready(settings) if !command.environment_locked => {
                self.world.eep.rlva_local_settings(asset, settings.as_ref());
            }
            SettingsState::Ready(settings) => {
                let rlva = &mut self.world.rlva;
                match settings.as_ref() {
                    Environment::Sky(sky) => {
                        rlva.environment = Some((actor, *sky));
                        rlva.environment_day_sky_fixed = rlva.environment_day.is_some();
                    }
                    Environment::Water(water) => rlva.environment_water = Some(*water),
                    Environment::Day(day) => {
                        rlva.environment_day = Some(std::sync::Arc::new(day.clone()));
                        rlva.environment_day_sky_fixed = false;
                        rlva.environment_water = None;
                        if rlva.environment.is_none() {
                            let sky = self.world.eep.current().0;
                            self.world.rlva.environment = Some((actor, sky));
                        }
                    }
                }
            }
        }
    }

    pub(super) fn rlva_camera_commands(&mut self) {
        use crate::world::rlva::{
            Param,
            camera::{FocusRequest, FocusTarget},
        };
        for (actor, c) in std::mem::take(&mut self.world.rlva.viewer) {
            // @version*, @getstatus* and @getcommand queued behind these to
            // keep the replies in order, with the rules they were received
            // under
            if let Param::Reply(channel) = c.param {
                let live = self.world.rlva.rewind(&c);
                let text = self.world.rlva.inline_reply(actor, &c);
                self.world.rlva.resume(live);
                if let Some(text) = text {
                    self.world.rlva.reply(channel, &c.behaviour, text);
                    continue;
                }
            }
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
                    self.world.rlva.reply(channel, &c.behaviour, text);
                }
                continue;
            }
            if self.world.rlva.debug_force(actor, &c) {
                continue;
            }
            if c.behaviour == "getheightoffset" {
                if let Param::Reply(channel) = c.param {
                    // an option fails the query, which still answers, with
                    // nothing (RlvReplyHandler<RLV_BHVR_GETHEIGHTOFFSET>)
                    let text = if c.option.is_empty() {
                        format!("{:.2}", self.world.hover_height.value() * 100.0)
                    } else {
                        String::new()
                    };
                    self.world.rlva.reply(channel, &c.behaviour, text);
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
                    self.world.rlva.reply(channel, &c.behaviour, reply);
                }
                continue;
            }
            if self.world.rlva.camera_force(actor, &c) {
                continue;
            }
            match c.behaviour.as_str() {
                "setcam_mode" => {
                    self.camera.read_rlva(&self.world.rlva);
                    match c.option.as_str() {
                        // (refused by the camera where RLVa forbids mouselook)
                        "mouselook" => self.camera.change_to_mouselook(&mut self.world.agent, &self.settings.camera, true),
                        "thirdperson" => self.camera.change_to_default(&mut self.world.agent, &self.settings.camera),
                        // handle_reset_view, as Escape (ResetViewTurnsAvatar)
                        "reset" | "" => self.reset_camera_view(),
                        _ => {}
                    }
                }
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
                            let Some((position, rotation)) =
                                rlva_focus_transform(crate::scene::Scene::object_transform_unshifted(&self.world, i, now))
                            else {
                                continue;
                            };
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

/// Where `@setcam_focus` puts the focus for an object: its position as
/// the Legacy Viewer reads it (RlvForceHandler<RLV_BHVR_SETCAM_FOCUS>,
/// rlvhandler.cpp: getPositionAgent), a HUD attachment included, whose
/// position is then in the HUD's own frame (no refusal there).
pub(super) fn rlva_focus_transform(transform: Option<(Vec3, glam::Quat, bool)>) -> Option<(Vec3, glam::Quat)> {
    transform
        .filter(|(position, _, _)| position.is_finite())
        .map(|(position, rotation, _hud)| (position, rotation))
}

/// What `@setenv_asset`, `preset` and `daycycle` name.
#[derive(Debug, PartialEq)]
pub(super) enum EnvironmentLookup {
    Found(Uuid),
    Missing,
    /// Library folders still to load before the answer is known.
    Loading(Vec<Uuid>),
}

/// registerSetEnvFn("asset"/"preset"/"daycycle") (rlvenvironment.cpp): an
/// id, or for preset and daycycle a settings item of that kind named so
/// (case-insensitive) in the first Library folder called "Environments",
/// at any depth (rlvGetLibraryEnvironmentsFolder, LLNameCategoryCollector);
/// `asset` takes an id only (registerSetEnvFn<LLUUID>). The Legacy Viewer
/// reads the Library it fetched in full at login; the Library here loads
/// its folders when asked, so the folders not loaded yet are returned to
/// be fetched first.
pub(super) fn rlva_environment_lookup(
    inventory: &crate::world::inventory::Inventory,
    behaviour: &str,
    option: &str,
    kind: crate::world::env_select::SettingsKind,
) -> EnvironmentLookup {
    use crate::world::{env_select::SettingsKind, inventory::FetchState};
    // LLUUID::set (rlvenvironment.cpp): a key in the Legacy forms only
    if let Some(id) = crate::world::rlva::numeric::uuid_set(option).filter(|id| !id.is_nil()) {
        return EnvironmentLookup::Found(id);
    }
    if behaviour == "setenv_asset" {
        return EnvironmentLookup::Missing;
    }
    let root = inventory.lib_root;
    let environments = inventory
        .folders
        .values()
        .filter(|f| f.library && f.info.id != root && f.info.name.eq_ignore_ascii_case("Environments"))
        .map(|f| f.info.id)
        .min();
    // the whole Library while the folder is not found, the folder after
    let mut stack = vec![environments.unwrap_or(root)];
    let mut seen = std::collections::HashSet::new();
    let mut loading = Vec::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(folder) = inventory.folders.get(&id) else {
            continue;
        };
        if folder.state != FetchState::Fetched {
            loading.push(id);
        }
        stack.extend(folder.children.iter().copied());
        if environments.is_none() {
            continue;
        }
        let found = folder
            .items
            .iter()
            .filter_map(|id| inventory.items.get(id))
            .find(|it| it.asset_type == 56 && SettingsKind::from_flags(it.flags) == Some(kind) && it.name.eq_ignore_ascii_case(option));
        if let Some(item) = found {
            return EnvironmentLookup::Found(item.asset_id);
        }
    }
    if loading.is_empty() {
        EnvironmentLookup::Missing
    } else {
        EnvironmentLookup::Loading(loading)
    }
}

/// Aurora Weather lights and changes the scene (rain, snow, fog, the
/// lightning), so @setenv holds it like the rest of the environment
/// (RlvEnvironment, indra/newview/rlvenvironment.cpp: the scene stays the
/// holder's): the weather in effect when the lock came stays, whatever the
/// Weather window chooses meanwhile, and the choice applies at the release.
pub(super) fn rlva_hold_weather(
    now: &mut crate::weather::WeatherSettings,
    chosen: crate::weather::WeatherSettings,
    environment_locked: bool,
) {
    if !environment_locked {
        *now = chosen;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restricted(commands: &str) -> crate::world::rlva::Rlva {
        let mut rlva = crate::world::rlva::Rlva::default();
        rlva.configure(true);
        assert!(rlva.receive(&aurora_net::ChatMessage {
            from_name: "Fixture".into(),
            source_id: Uuid::from_u128(1),
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: Vec3::ZERO,
            message: commands.into(),
        }));
        rlva
    }

    /// RlvUIEnabler::onToggleShowMinimap: `fPrevVisibile`.
    #[test]
    fn rlva_minimap_comes_back_at_the_release_only_if_it_was_open() {
        for was_open in [true, false] {
            let mut hidden = HiddenMinimap::default();
            let mut open = was_open;
            hidden.apply(false, &mut open);
            assert_eq!(open, was_open, "no restriction: the window is the user's");
            hidden.apply(true, &mut open);
            assert!(!open);
            // whatever opens it while the restriction holds, it closes
            open = true;
            hidden.apply(true, &mut open);
            assert!(!open);
            hidden.apply(false, &mut open);
            assert_eq!(open, was_open);
            // the release is remembered once: closing it afterwards holds
            open = false;
            hidden.apply(false, &mut open);
            assert!(!open);
        }
    }

    #[test]
    fn rlva_hidden_windows_open_by_no_path_and_their_shortcuts_do_nothing() {
        let all = |open: bool| {
            let mut panels = ui::Panels::from_settings(&crate::settings::Settings::default());
            (panels.inventory, panels.minimap, panels.world_map) = (open, open, open);
            panels
        };
        let free = crate::world::rlva::Rlva::default();
        let mut panels = all(true);
        keep_windows_closed(&free, &mut panels);
        assert!(panels.inventory && panels.minimap && panels.world_map);
        for (command, shortcuts) in [
            ("@showinv=n", &[Action::Inventory, Action::NewInventoryWindow][..]),
            ("@showminimap=n", &[Action::Minimap][..]),
            ("@showworldmap=n", &[Action::WorldMap][..]),
        ] {
            let rlva = restricted(command);
            let mut panels = all(true);
            keep_windows_closed(&rlva, &mut panels);
            assert_eq!(
                (panels.inventory, panels.minimap, panels.world_map),
                (command != "@showinv=n", command != "@showminimap=n", command != "@showworldmap=n"),
                "{command}"
            );
            // it closes, it never opens
            let mut panels = all(false);
            keep_windows_closed(&rlva, &mut panels);
            assert!(!panels.inventory && !panels.minimap && !panels.world_map);
            for action in [Action::Inventory, Action::NewInventoryWindow, Action::Minimap, Action::WorldMap] {
                assert_eq!(
                    refuses_shortcut(&rlva, action),
                    shortcuts.contains(&action),
                    "{command}: {action:?}"
                );
                assert!(!refuses_shortcut(&free, action));
            }
        }
    }

    /// RlvActions::canHighlightTransparent and the @viewtransparent toggle.
    #[test]
    fn rlva_transparency_highlight_is_refused_under_edit_showloc_and_viewtransparent() {
        let free = crate::world::rlva::Rlva::default();
        assert!(can_highlight_transparent(&free) && !refuses_shortcut(&free, Action::ShowTransparency));
        for command in ["@edit=n", "@showloc=n", "@viewtransparent=n"] {
            let rlva = restricted(command);
            assert!(!can_highlight_transparent(&rlva), "{command}");
            assert!(refuses_shortcut(&rlva, Action::ShowTransparency), "{command}");
        }
        let other = restricted("@showinv=n");
        assert!(can_highlight_transparent(&other));
    }

    /// RlvBehaviourToggleHandler<RLV_BHVR_EDIT>::onCommandToggle
    /// (rlvhandler.cpp): @edit hides the beacons window and filters it
    /// without a notice, and build mode is left once, as @edit comes.
    #[test]
    fn rlva_edit_closes_the_beacons_and_leaves_build_mode_as_it_comes() {
        let free = crate::world::rlva::Rlva::default();
        let rlva = restricted("@edit=n");
        let mut panels = ui::Panels::from_settings(&crate::settings::Settings::default());
        panels.beacons = true;
        keep_windows_closed(&free, &mut panels);
        assert!(panels.beacons && !refuses_shortcut(&free, Action::Beacons));
        keep_windows_closed(&rlva, &mut panels);
        assert!(!panels.beacons && refuses_shortcut(&rlva, Action::Beacons));
        assert!(!refuses_shortcut(&restricted("@editworld=n"), Action::Beacons));
        assert!(leaves_build_mode(false, true), "@edit comes");
        assert!(!leaves_build_mode(true, true), "build mode opened while it lasts stays");
        assert!(!leaves_build_mode(true, false) && !leaves_build_mode(false, false));
    }

    /// RlvUtil::notifyBlockedViewXXX: one notice per kind of preview.
    #[test]
    fn rlva_refused_previews_name_their_kind() {
        assert_eq!(view_blocked_notice(7), aurora_i18n::tr!("rlva-blocked-view-notecard"));
        assert_eq!(view_blocked_notice(10), aurora_i18n::tr!("rlva-blocked-view-script"));
        assert_eq!(view_blocked_notice(0), aurora_i18n::tr!("rlva-blocked-view-texture"));
    }

    #[test]
    fn rlva_setenv_holds_the_aurora_weather_in_effect() {
        let before = crate::weather::WeatherSettings::default();
        let mut chosen = before.clone();
        chosen.thunder = !before.thunder;
        let mut now = before.clone();
        rlva_hold_weather(&mut now, chosen.clone(), true);
        assert_eq!(now, before, "held: the window's choice waits");
        rlva_hold_weather(&mut now, chosen.clone(), false);
        assert_eq!(now, chosen, "released: the choice applies");
    }

    /// registerSetEnvFn (rlvenvironment.cpp) and
    /// rlvGetLibraryEnvironmentsFolder: an id for `asset`, a name in the
    /// Library's "Environments" folder, found at any depth, for preset and
    /// daycycle; nothing outside that folder.
    #[test]
    fn rlva_environment_names_come_from_the_library_environments_folder_only() {
        use crate::world::env_select::SettingsKind;
        use crate::world::inventory::{FetchState, Folder, Inventory};
        let [root, objects, environments, inside, outside, asset] = [1, 2, 3, 4, 5, 6].map(Uuid::from_u128);
        let folder = |id, name: &str, children: Vec<Uuid>, items: Vec<Uuid>| Folder {
            info: aurora_net::inventory::InvFolder {
                id,
                name: name.to_owned(),
                ..Default::default()
            },
            children,
            items,
            state: FetchState::Fetched,
            library: true,
        };
        let item = |id, asset_id| aurora_net::inventory::InvItem {
            id,
            asset_id,
            asset_type: 56,
            name: "Midday".to_owned(),
            ..Default::default()
        };
        let mut inventory = Inventory {
            lib_root: root,
            ..Default::default()
        };
        inventory
            .folders
            .insert(root, folder(root, "Library", vec![objects], vec![outside]));
        inventory
            .folders
            .insert(objects, folder(objects, "Objects", vec![environments], vec![]));
        inventory.items.insert(outside, item(outside, Uuid::from_u128(99)));
        let lookup = rlva_environment_lookup;
        assert_eq!(
            lookup(&inventory, "setenv_preset", "Midday", SettingsKind::Sky),
            EnvironmentLookup::Missing,
            "no Environments folder: not the Library root"
        );
        inventory
            .folders
            .insert(environments, folder(environments, "environments", vec![], vec![inside]));
        inventory.items.insert(inside, item(inside, asset));
        assert_eq!(
            lookup(&inventory, "setenv_preset", "MIDDAY", SettingsKind::Sky),
            EnvironmentLookup::Found(asset),
            "any depth, any case"
        );
        assert_eq!(
            lookup(&inventory, "setenv_daycycle", "Midday", SettingsKind::Day),
            EnvironmentLookup::Missing,
            "a sky is no day"
        );
        assert_eq!(
            lookup(&inventory, "setenv_asset", "Midday", SettingsKind::Sky),
            EnvironmentLookup::Missing,
            "asset takes an id only"
        );
        assert_eq!(
            lookup(&inventory, "setenv_asset", &asset.to_string(), SettingsKind::Sky),
            EnvironmentLookup::Found(asset)
        );
        // a folder not loaded yet is fetched before the answer
        inventory.folders.get_mut(&environments).expect("folder").state = FetchState::Unknown;
        assert_eq!(
            lookup(&inventory, "setenv_preset", "Sunset", SettingsKind::Sky),
            EnvironmentLookup::Loading(vec![environments])
        );
    }

    /// RlvForceHandler<RLV_BHVR_SETCAM_FOCUS> takes any object it finds, a
    /// HUD attachment included.
    #[test]
    fn rlva_setcam_focus_accepts_a_hud_object() {
        let at = glam::Vec3::new(0.1, 0.2, 0.3);
        assert_eq!(
            rlva_focus_transform(Some((at, glam::Quat::IDENTITY, true))),
            Some((at, glam::Quat::IDENTITY))
        );
        assert_eq!(rlva_focus_transform(Some((glam::Vec3::NAN, glam::Quat::IDENTITY, false))), None);
        assert_eq!(rlva_focus_transform(None), None);
    }
}
