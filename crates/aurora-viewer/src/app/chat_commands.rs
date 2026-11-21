//! Running the chat bar commands (see [`crate::cmdline`]): what each branch
//! of Firestorm's cmd_line_chat does (indra/newview/chatbar_as_cmdline.cpp),
//! with Aurora's world, camera and network.

use std::time::Instant;

use aurora_net::{ChatType, NetCommand};
use glam::Vec3;
use uuid::Uuid;

use super::App;
use crate::cmdline::{self, Command};

/// key2name waits this long for a name before giving up.
const KEY_TO_NAME_TIMEOUT_SECS: u64 = 30;

/// A random number in `0..n` (Firestorm uses `rand() % n`).
fn random_below(n: u32) -> u32 {
    let r = (Uuid::new_v4().as_u128() >> 64) as u64;
    (r % u64::from(n.max(1))) as u32
}

/// FSCommon::send_message_to_script_channel (indra/newview/fscommon.cpp):
/// the viewer's own report, whispered on a script channel, nothing on
/// channel 0 or for an empty text.
fn script_channel_chat(message: &str, channel: i32) -> Option<NetCommand> {
    (channel != 0 && !message.is_empty()).then(|| NetCommand::Chat {
        message: message.to_owned(),
        channel,
        chat_type: ChatType::Whisper,
    })
}

impl App {
    /// FSCommon::send_message_to_script_channel builds its ChatFromViewer
    /// itself, past the chat path RLVa filters (send_chat_from_viewer):
    /// @sendchannel, @redirchat and the chat volume rules never touch the
    /// viewer's reports to the user's own scripts.
    pub(super) fn send_to_script_channel(&mut self, message: &str, channel: i32) {
        if let Some(command) = script_channel_chat(message, channel) {
            self.net.send(command);
        }
    }

    /// Local chat typed in the chat bar or the conversations window: runs it
    /// as a command when it is one (cmd_line_chat). `true` = handled, the
    /// line must not be sent.
    pub(super) fn chat_command(&mut self, text: &str) -> bool {
        let Some(cmd) = cmdline::parse(text.trim(), &self.settings.chat_commands, random_below) else {
            return false;
        };
        log::info!("chat command: {}", text.split_whitespace().next().unwrap_or_default());
        self.run_chat_command(cmd)
    }

    /// FSCommon::report_cmdline_result: a line in the local chat, also
    /// whispered on the announce channel when that option is on.
    fn report_command(&mut self, text: impl Into<String>) {
        let text = text.into();
        let s = &self.settings.chat_commands;
        if s.announce_to_channel {
            let channel = s.announce_channel;
            self.send_to_script_channel(&text, channel);
        }
        self.world.system_message(text);
    }

    fn run_chat_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::Ao(enabled) => {
                let account = self.settings.ao.accounts.entry(self.world.agent_id).or_default();
                let changed = account.enabled != enabled;
                account.enabled = enabled;
                if enabled {
                    self.ao_runtime.stands = true;
                    self.ao_runtime.paused = false;
                }
                self.settings.save();
                if changed {
                    self.report_command(if enabled {
                        aurora_i18n::tr!("ao-command-on")
                    } else {
                        aurora_i18n::tr!("ao-command-off")
                    });
                }
            }
            Command::AoSits(value) => {
                let selected = self
                    .settings
                    .ao
                    .accounts
                    .get(&self.world.agent_id)
                    .map_or(uuid::Uuid::nil(), |a| a.selected);
                if let Some(mut set) = self.ao_catalog.selected(selected).cloned() {
                    set.sits = value.unwrap_or(!set.sits);
                    match crate::ao::inventory::save_set(&self.world.inventory, &set) {
                        Ok(change) => self.apply_ao_action(crate::ui::ao::Action::Edit(change, None)),
                        Err(reason) => self.report_command(reason),
                    }
                }
            }
            Command::Report(lines) => {
                for l in lines {
                    self.report_command(l);
                }
            }
            Command::Unavailable(text) => self.report_command(text),
            Command::TeleportLocal { x, y, z } => {
                let z = z.unwrap_or(self.world.agent.position.z);
                self.teleport_in_region(Vec3::new(x, y, z));
            }
            Command::DrawDistance(amount) => {
                let d = amount.apply(self.settings.draw_distance);
                // the slider's range (the Legacy Viewer does not clamp here)
                self.settings.draw_distance = d.clamp(crate::settings::DRAW_DISTANCE_MIN, crate::settings::DRAW_DISTANCE_MAX);
                self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-draw-distance-set-to-m",
                    value0 = format!("{:.0}", self.settings.draw_distance)
                ));
            }
            Command::Bandwidth(kbps) => {
                self.settings.bandwidth_kbps = kbps;
                self.net.set_bandwidth(kbps as f32);
                self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-maximum-bandwidth-set-to-kbps",
                    value0 = kbps.to_string()
                ));
            }
            Command::TeleportToCamera => self.teleport_render_pos(self.camera.position),
            Command::Media { url, mime } => {
                // LLParcel::setMediaURL / setMediaType on our copy of the
                // parcel, then LLViewerParcelMedia::play.
                let Some(parcel) = self.world.parcel.as_mut() else {
                    return false;
                };
                let parcel = std::sync::Arc::make_mut(parcel);
                parcel.media_url = url;
                parcel.media.mime = mime;
                self.media.stop_parcel();
                self.media.play_parcel(&self.world, &self.settings.media);
            }
            Command::Music(url) => {
                // startInternetStreamWithAutoFade: until the parcel stream changes.
                self.parcel_music.play(url);
                self.remember_music_choice();
            }
            Command::KeyToName(id) => {
                self.world.social.avatar_names.want(&id);
                self.key_to_name.push((id, Instant::now()));
            }
            Command::Touch(id) => match self.world.objects.index_of_uuid(&id).and_then(|i| self.world.objects.get(i)) {
                // RlvActions::canTouch refused: nothing is sent nor reported
                Some(o) if !self.world.rlva_can_touch(o.key.region, o.key.local_id) => {}
                Some(o) => {
                    let local_id = o.key.local_id;
                    self.send(NetCommand::Touch { local_id });
                    self.report_command(aurora_i18n::tr!(
                        "app-chat-commands-touched-object-with-key",
                        id = format!("{}", id)
                    ));
                }
                None => self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-object-with-key-not-found",
                    id = format!("{}", id)
                )),
            },
            Command::SitOn(id) => match self.world.objects.index_of_uuid(&id).and_then(|i| self.world.objects.get(i)) {
                Some(o) => {
                    let sit = NetCommand::RequestSit {
                        handle: o.key.region,
                        target: id,
                        offset: Vec3::ZERO,
                    };
                    // cmd_line_chat asks RlvActions::canSit and, refused,
                    // says nothing at all
                    if !self.world.rlva_allows_command(&sit, Uuid::nil()) {
                        return true;
                    }
                    // (no @standtp origin: cmd_line_chat sends AgentRequestSit
                    // without RlvHandler::setSitSource)
                    self.world.rlva.sitting.no_sit_source = true;
                    self.send(sit);
                    self.world.rlva.sitting.no_sit_source = false;
                    self.report_command(aurora_i18n::tr!(
                        "app-chat-commands-sitting-on-object-with-key",
                        id = format!("{}", id)
                    ));
                }
                None => self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-object-with-key-not-found",
                    id = format!("{}", id)
                )),
            },
            Command::StandUp => {
                self.send(NetCommand::OneShotControl(aurora_net::control::STAND_UP));
                self.report_command(aurora_i18n::tr!("app-chat-commands-standing"));
            }
            Command::OfferTeleport(to) => {
                // "Join me!" stays in English: it is what the other avatar reads.
                // cmd_line_chat sends its StartLure itself, past every RLVa
                // check of the offer windows (canOfferTeleport, send_lures).
                self.net.send(NetCommand::OfferTeleport {
                    to: vec![to],
                    message: aurora_i18n::tr!("extra-join-me").into(),
                });
                let name = self.world.person_name(&to).unwrap_or_else(|| to.to_string());
                self.report_command(aurora_i18n::tr!("app-chat-commands-teleport-offered-to", name = name.to_string()));
            }
            Command::Ground { offset } => {
                let pos = self.world.agent.position;
                if let Some(ground) = self.world.ground_height(pos) {
                    self.teleport_in_region_if_local(Vec3::new(pos.x, pos.y, ground + offset));
                }
            }
            Command::Height(amount) => {
                let pos = self.world.agent.position;
                self.teleport_in_region_if_local(Vec3::new(pos.x, pos.y, amount.apply(pos.z)));
            }
            Command::TeleportHome => {
                self.request_user_teleport(NetCommand::TeleportHome, aurora_i18n::tr!("common-home").into(), false);
            }
            Command::RezPlatform(width) => self.rez_platform(width.unwrap_or(self.settings.chat_commands.platform_size)),
            Command::MapTo { region, pos } => {
                let pos = pos.unwrap_or_else(|| {
                    if self.settings.chat_commands.map_to_keep_pos {
                        let p = self.world.agent.position.round();
                        (p.x as i32, p.y as i32, p.z as i32)
                    } else {
                        (128, 128, 0)
                    }
                });
                self.teleport_to_location(region, Vec3::new(pos.0 as f32, pos.1 as f32, pos.2 as f32));
            }
            Command::TeleportToAvatar(name) => {
                // FSRadar::teleportToAvatar: the last known position, 2 m
                // higher (FSTeleportToOffsetLateral 0, …Vertical 2).
                if let Some((id, pos)) = self.find_nearby(&name)
                    && id != self.world.agent_id
                {
                    if pos.z <= 0.0 {
                        // AVATAR_UNKNOWN_Z_OFFSET: too high for coarse locations.
                        self.report_command(aurora_i18n::tr!(
                            "app-chat-commands-cannot-teleport-to-this-avatar-its-altitude-is-unknown"
                        ));
                    } else {
                        self.teleport_render_pos(pos + Vec3::Z * 2.0);
                    }
                }
            }
            Command::TrackPosition(pos) => {
                let (x, y) = crate::ui::minimap::to_global(&self.world, pos.round());
                self.world.map.track_location(x, y, pos.z.round(), false);
                let region = self.world.region_name();
                let p = pos.round();
                self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-tracking-2",
                    region = region.to_string(),
                    value0 = format!("{}", p.x),
                    value1 = format!("{}", p.y),
                    value2 = format!("{}", p.z)
                ));
            }
            Command::TrackAvatar(name) => match self.find_nearby(&name) {
                // Deviation: LLAvatarActions::track follows the avatar; Aurora
                // has no avatar tracking yet and marks where it is now.
                Some((id, pos)) => {
                    let (x, y) = crate::ui::minimap::to_global(&self.world, pos);
                    self.world.map.track_location(x, y, pos.z, false);
                    let name = cmdline_tracked_name(&self.world, id).unwrap_or(name);
                    self.report_command(aurora_i18n::tr!("app-chat-commands-tracking", name = name.to_string()));
                }
                None => self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-no-nearby-avatar-matches",
                    name = name.to_string()
                )),
            },
            Command::ClearChat => {
                self.world.clear_nearby_chat();
                self.world.nearby_read();
            }
            Command::CopyCamera => {
                // Focus 1 m ahead of the camera; Aurora's camera has no roll.
                let pos = self.camera.position;
                let text = cmdline::format_camera(pos, pos + self.camera.forward(), 0.0);
                self.egui_ctx.copy_text(text.clone());
                self.report_command(aurora_i18n::tr!(
                    "app-chat-commands-camera-position-copied-to-clipboard",
                    text = text.to_string()
                ));
            }
            Command::PasteCamera(spec) => self.paste_camera(spec),
        }
        true
    }

    /// Names typed for `tp2` / `trkpos`: nearby avatars in distance order,
    /// matched on their user name (cmdline_partial_name2key).
    fn find_nearby(&self, partial: &str) -> Option<(Uuid, Vec3)> {
        let nearby = crate::ui::people::nearby(&self.world);
        let names: Vec<(Uuid, String)> = nearby.iter().map(|(id, ..)| (*id, cmdline_user_name(&self.world, id))).collect();
        let id = cmdline::partial_name_match(partial, names.iter().map(|(id, n)| (*id, n.as_str())))?;
        nearby.iter().find(|e| e.0 == id).map(|e| (id, e.2))
    }

    /// teleportViaLocation to a point of the current region.
    /// `gtp` and `height`: cmd_line_chat asks RlvActions::canTeleportToLocal
    /// first and, refused, does nothing and says nothing.
    fn teleport_in_region_if_local(&mut self, pos: Vec3) {
        let global = self
            .world
            .main_origin()
            .map(|(x, y)| glam::DVec3::new(x as f64, y as f64, 0.0) + pos.as_dvec3());
        if global.is_some_and(|g| !self.world.rlva_can_teleport_local(g)) {
            return;
        }
        self.teleport_in_region(pos);
    }

    fn teleport_in_region(&mut self, pos: Vec3) {
        let region = self.world.region_name();
        self.teleport_to_location(region, pos);
    }

    /// teleportViaLocation to a render-space point, maybe in a neighbour.
    pub(super) fn teleport_render_pos(&mut self, pos: Vec3) {
        let size = self
            .world
            .main()
            .map_or((256.0, 256.0), |r| (r.heightmap.size_x as f32, r.heightmap.size_y as f32));
        if (0.0..size.0).contains(&pos.x) && (0.0..size.1).contains(&pos.y) {
            self.teleport_in_region(pos);
        } else {
            let (x, y) = crate::ui::minimap::to_global(&self.world, pos);
            self.request_user_teleport(
                crate::world::worldmap::teleport_command(x, y, pos.z),
                format!("({x:.0}, {y:.0}, {:.0})", pos.z),
                true,
            );
        }
    }

    /// cmdline_rezplat: a flat metal box 2.5 m under the avatar (where it
    /// will be in a third of a second).
    fn rez_platform(&mut self, width: f32) {
        use crate::build::MIN_PRIM_SCALE;
        // RlvActions::canRez: under @rez the command does nothing, silently
        if self.world.rlva.enabled() && self.world.rlva.has("rez") {
            return;
        }
        let limits = self.world.prim_limits();
        let Some(handle) = self.world.main_region else {
            return;
        };
        let agent = &self.world.agent;
        let pos = agent.position + agent.velocity * 0.333;
        let at = pos - Vec3::new(0.0, 0.0, 2.5);
        let size = limits.clamp(width);
        let p = aurora_net::build::NewPrim {
            handle,
            group_id: self.world.group_for_rez(self.settings.build.rez_under_land_group),
            pcode: aurora_prim::params::LL_PCODE_VOLUME,
            // LL_MCODE_METAL
            material: 1,
            // FLAGS_CREATE_SELECTED above 4096 m
            add_flags: if pos.z > 4096.0 { 0x02 } else { 0 },
            shape: crate::build::shapes::shapes()[0].raw(),
            ray_start: at,
            ray_end: at,
            ray_target: Uuid::nil(),
            ray_end_is_intersection: false,
            bypass_raycast: true,
            scale: Vec3::new(size, size, limits.clamp(MIN_PRIM_SCALE)),
            rotation: glam::Quat::IDENTITY,
            state: 0,
        };
        self.send(NetCommand::Build(aurora_net::build::BuildCmd::Add(Box::new(p))));
    }

    /// cmdline_apply_camera.
    fn paste_camera(&mut self, spec: cmdline::CameraSpec) {
        let focus = spec.focus.unwrap_or(spec.pos + (self.camera.target - self.camera.position));
        if self.world.agent.position.distance(spec.pos) > self.settings.draw_distance {
            self.report_command(aurora_i18n::tr!(
                "app-chat-commands-cannot-restore-the-camera-view-because-its-position-is"
            ));
            return;
        }
        self.camera.set_view(spec.pos, focus, &mut self.world.agent, &self.settings.camera);
        let text = cmdline::format_camera(spec.pos, focus, spec.roll.unwrap_or(0.0));
        self.report_command(aurora_i18n::tr!("app-chat-commands-camera-restored-to", text = text.to_string()));
    }

    /// key2name answers, once the names are known (key_to_name_callback).
    pub(super) fn poll_key_to_name(&mut self) {
        if self.key_to_name.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.key_to_name);
        for (id, since) in pending {
            if let Some(name) = self.world.social.avatar_names.complete(&id).or_else(|| self.world.person_name(&id)) {
                self.report_command(format!("{id}: ({name})"));
            } else if since.elapsed().as_secs() < KEY_TO_NAME_TIMEOUT_SECS {
                self.key_to_name.push((id, since));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The viewer's reports to a script channel are sent as they are, and
    /// past RLVa: a channel @sendchannel closes still gets them.
    #[test]
    fn rlva_viewer_reports_to_a_script_channel_skip_the_chat_rules() {
        assert!(script_channel_chat("hello", 0).is_none());
        assert!(script_channel_chat("", 5).is_none());
        let command = script_channel_chat("region_restart_in:60", -42).expect("a report");
        assert!(matches!(
            &command,
            NetCommand::Chat { message, channel: -42, chat_type: ChatType::Whisper } if message == "region_restart_in:60"
        ));
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.rlva.configure(true);
        world.rlva_console_input("@sendchannel=n");
        // the chat path RLVa filters would have dropped it
        assert!(world.rlva.outgoing_chat("region_restart_in:60", -42, ChatType::Whisper).is_empty());
    }
}

/// LLAvatarName::getUserName: "First Last", only "First" for a Resident.
/// Under `@shownames` the radar's stand-in, for everyone: the
/// FSRadarEntry::mUserName cmdline_partial_name2key
/// (indra/newview/chatbar_as_cmdline.cpp) matches is
/// FSRadarEntry::getRadarName there (indra/newview/fsradarentry.cpp), an
/// `@shownames:<uuid>` exception's too.
fn cmdline_user_name(world: &crate::world::World, id: &Uuid) -> String {
    if world.rlva.has("shownames") {
        return world.rlva_anonymous(*id).display_name.clone();
    }
    if let Some(n) = world.social.avatar_names.get(id)
        && !n.legacy_first.is_empty()
    {
        return if n.legacy_last.is_empty() || n.legacy_last == "Resident" {
            n.legacy_first.clone()
        } else {
            format!("{} {}", n.legacy_first, n.legacy_last)
        };
    }
    let legacy = world.legacy_name(id).unwrap_or_default();
    legacy.strip_suffix(" Resident").map(str::to_owned).unwrap_or(legacy)
}

/// The name `trkpos` reports for the avatar it tracks: FSRadarEntry::getName
/// (indra/newview/chatbar_as_cmdline.cpp), the radar's stand-in for everyone
/// under `@shownames`, an exception included.
fn cmdline_tracked_name(world: &crate::world::World, id: Uuid) -> Option<String> {
    if world.rlva.has("shownames") {
        Some(world.rlva_anonymous(id).display_name.clone())
    } else {
        world.person_name(&id)
    }
}

#[cfg(test)]
mod rlva_tests {
    use super::*;

    /// cmdline_partial_name2key and cmdline_tp2name
    /// (indra/newview/chatbar_as_cmdline.cpp) read the radar, whose names
    /// are all stand-ins under `@shownames`: an `@shownames:<uuid>`
    /// exception is matched and reported by their stand-in too.
    #[test]
    fn tp2_and_trkpos_name_everyone_by_the_radar_stand_in_under_shownames() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let id = Uuid::from_u128(0x7e57);
        let name = aurora_net::AvatarNameData {
            id,
            username: "jane.doe".into(),
            display_name: "Janey".into(),
            legacy_first: "Jane".into(),
            legacy_last: "Doe".into(),
            ..Default::default()
        };
        world.social.avatar_names.on_display_names(&[name], &[], None);
        world.rlva.configure(true);
        assert_eq!(cmdline_user_name(&world, &id), "Jane Doe");
        for command in ["@shownames=n".to_owned(), format!("@shownames:{id}=add")] {
            world.rlva_console_input(&command);
        }
        world.refresh_rlva_privacy();
        assert!(world.rlva_can_name(id), "the exception");
        let anonym = world.rlva_anonymous(id).display_name.clone();
        assert_ne!(anonym, "Jane Doe");
        assert_eq!(cmdline_user_name(&world, &id), anonym);
        assert_eq!(cmdline_tracked_name(&world, id), Some(anonym.clone()));
        let found = cmdline::partial_name_match(&anonym, [(id, cmdline_user_name(&world, &id).as_str())]);
        assert_eq!(found, Some(id));
        assert_eq!(
            cmdline::partial_name_match("jane", [(id, cmdline_user_name(&world, &id).as_str())]),
            None
        );
    }
}
