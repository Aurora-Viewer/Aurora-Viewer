//! The region restart of the agent's region, from the simulator's notice to
//! the end of the alert: the rules of `world/region_restart.rs`, the alert
//! of `ui/region_restart.rs` (or Firestorm's box), the sound, the shakes,
//! the taskbar, the bell's pinned card, the nearby chat line and the channel
//! announcement (llviewermessage.cpp, llfloaterregionrestarting.cpp,
//! originally LGPL 2.1), and the `AURORA_DEMO_RESTART` scenario.

use super::App;
use crate::ui::region_restart::{self as alert, Action, BellInfo, Destinations, Landmark};
use crate::ui_sound::UiSound;
use crate::world::notifications::{Data, Kind, Response};
use crate::world::region_restart::{Cue, Level, Phase, Shake, view_shake};
use aurora_net::region_restart::Notice;
use aurora_net::{ChatType, NetCommand};
use std::time::Instant;
use uuid::Uuid;

/// LLAssetType::AT_LANDMARK and LLFolderType::FT_TRASH.
const ASSET_LANDMARK: i32 = 3;
const FOLDER_TRASH: i32 = 14;

/// What the app keeps of the restart besides its rules.
#[derive(Default)]
pub struct State {
    pub ui: alert::RestartUi,
    /// The pinned card of the bell.
    bell: Option<u64>,
    /// Firestorm's view shake, since then.
    view_shake_at: Option<Instant>,
    last_tick: Option<Instant>,
    /// The landmarks of the inventory, by its generation.
    landmarks: (Option<u64>, Vec<Landmark>),
    /// Firestorm's box: the landmark chosen in its list, and whether it is open.
    legacy_pick: Option<Uuid>,
    legacy_open: bool,
    /// AURORA_DEMO_RESTART.
    demo: Option<Demo>,
}

/// `AURORA_DEMO_RESTART=<seconds>[:manager][:fast][:still][:min][:lm][:bell][:toast][:dnd][:opensim][:legacy][:closed]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Demo {
    pub seconds: i32,
    pub manager: bool,
    /// The mockup's fast mode: 5 minutes in about 20 seconds.
    pub fast: bool,
    /// The count and the breath stand still (the mockup's still states).
    pub still: bool,
    pub minimized: bool,
    pub menu: bool,
    pub bell: bool,
    pub toast: bool,
    pub dnd: bool,
    pub opensim: bool,
    pub legacy: bool,
    pub closed: bool,
    started: bool,
}

impl Demo {
    pub fn parse(text: &str) -> Option<Demo> {
        let mut words = text.split(':');
        let seconds = words.next()?.trim().parse().ok()?;
        let mut d = Demo {
            seconds,
            ..Demo::default()
        };
        for w in words {
            match w.trim() {
                "manager" => d.manager = true,
                "fast" => d.fast = true,
                "still" => d.still = true,
                "min" => d.minimized = true,
                "lm" => d.menu = true,
                "bell" => d.bell = true,
                "toast" => d.toast = true,
                "dnd" => d.dnd = true,
                "opensim" => d.opensim = true,
                "legacy" => d.legacy = true,
                "closed" => d.closed = true,
                other => log::warn!("AURORA_DEMO_RESTART: unknown option {other:?}"),
            }
        }
        Some(d)
    }

    /// How fast the count runs in the mockup's fast mode: 5 min → 1 min in
    /// 6 s, 1 min → 30 s in 3 s, 30 s → 10 s in 4 s, then two seconds a
    /// second for the final count.
    pub fn rate(&self, seconds: i32) -> f64 {
        if !self.fast {
            return 1.0;
        }
        match seconds {
            s if s > 60 => 40.0,
            s if s > 30 => 10.0,
            s if s > 10 => 5.0,
            _ => 2.0,
        }
    }
}

/// The frame of the demo's notice (after the loading screen).
const DEMO_AT: u64 = 600;

impl State {
    pub fn take_demo(&mut self) -> Option<Demo> {
        self.demo.take()
    }

    pub fn put_demo(&mut self, demo: Option<Demo>) {
        self.demo = demo;
    }
}

impl App {
    fn classic_restart(&self) -> bool {
        self.settings.classic_restart_alert || self.region_restart.demo.as_ref().is_some_and(|d| d.legacy)
    }

    /// AlertMessage RegionRestartMinutes / RegionRestartSeconds, or
    /// OpenSim's `/RESTART_X_…` (attempt_standard_notification,
    /// process_alert_core).
    pub(super) fn on_region_restart(&mut self, notice: Notice) {
        let sound = if notice.opensim {
            UiSound::RestartOpenSim
        } else {
            UiSound::Restart
        };
        let region = notice.region.clone().unwrap_or_else(|| self.world.region_name());
        // RegionRestartMinutes / RegionRestartSeconds for the system, at
        // each notice (the alert flashes on its own when it opens)
        let told = crate::world::attention::Notice {
            title: aurora_i18n::tr!("region-restart-title").to_owned(),
            body: toast_text(&region, notice.seconds),
            open: crate::world::attention::Open::Notifications,
        };
        self.world.ask_attention(crate::world::attention::Cause::Notice, Some(told));
        if !self.settings.region_restart.show_alert {
            // FSUseNewRegionRestartNotification off: a toast at each notice
            // (RegionRestartMinutesToast / RegionRestartSecondsToast)
            let body = toast_text(&region, notice.seconds);
            self.world
                .notifications
                .push(Kind::Alert, aurora_i18n::tr!("region-restart-title"), body, Data::None);
            self.world.ui_sounds.push(sound);
            self.world
                .system_notice(aurora_i18n::tr!("region-restart-chat"), crate::world::notice_topics::Topic::Region);
            return;
        }
        let classic = self.classic_restart();
        let cues = self.world.region_restart.notice(&notice, &region, self.world.main_region, classic);
        self.region_restart_cues(&cues);
    }

    /// "Region restart cancelled.": the alert turns green and leaves, the
    /// channel hears it (fs_report_region_restart_to_channel(-1)).
    pub(super) fn on_region_restart_cancelled(&mut self) {
        if self.world.region_restart.cancelled() {
            self.drop_restart_bell();
            self.region_restart.legacy_open = false;
            self.world.system_notice(
                aurora_i18n::tr!("region-restart-chat-cancelled"),
                crate::world::notice_topics::Topic::Region,
            );
            self.announce_restart(None);
        } else {
            // no alert to close: the simulator's system message, as Firestorm
            self.world.notifications.push(
                Kind::Alert,
                aurora_i18n::tr!("world-mod-simulator-alert"),
                aurora_i18n::tr!("region-restart-chat-cancelled"),
                Data::None,
            );
        }
    }

    /// The session ended: when the region was restarting, the login screen
    /// says so and offers to reconnect. Returns the message to show.
    pub(super) fn restart_disconnect(&mut self) -> Option<String> {
        let a = self.world.region_restart.alert.as_ref()?;
        (a.phase == Phase::Live && a.seconds() <= 30).then(|| aurora_i18n::tr!("region-restart-disconnected", region = a.region.as_str()))
    }

    fn region_restart_cues(&mut self, cues: &[Cue]) {
        let now = self.egui_ctx.input(|i| i.time);
        let classic = self.classic_restart();
        let reduce = self.settings.reduce_motion;
        let Some(a) = self.world.region_restart.alert.as_ref() else {
            return;
        };
        let (opensim, region, minimized, seconds) = (a.opensim, a.region.clone(), a.minimized, a.seconds());
        for cue in cues {
            match *cue {
                Cue::Open => {
                    self.region_restart.ui.opened(now);
                    self.region_restart.legacy_open = true;
                    self.region_restart.legacy_pick = None;
                    // LLWindow::flashIcon(5.f) when the box opens
                    if self.window_behind() {
                        self.flash_taskbar();
                    }
                    // the notification of the well, without a toast
                    // (show_toast="false"), pinned while the count runs
                    self.drop_restart_bell();
                    let id = self.world.notifications.push(
                        Kind::Alert,
                        aurora_i18n::tr!("region-restart-title"),
                        toast_text(&region, seconds),
                        Data::RegionRestart,
                    );
                    if let Some(n) = self.world.notifications.list.iter_mut().find(|n| n.id == id) {
                        n.quiet = true;
                    }
                    self.region_restart.bell = Some(id);
                }
                Cue::Sound => self
                    .world
                    .ui_sounds
                    .push(if opensim { UiSound::RestartOpenSim } else { UiSound::Restart }),
                Cue::Shake if !reduce => {
                    let shake = self.settings.region_restart.shake;
                    if classic || shake == Shake::View {
                        // FIRE-12900: FSNoScreenShakeOnRegionRestart
                        if shake != Shake::None {
                            self.region_restart.view_shake_at = Some(Instant::now());
                        }
                    } else if shake == Shake::Alert && !cues.contains(&Cue::Open) && !minimized {
                        self.region_restart.ui.shake(now);
                    }
                }
                Cue::Shake => {}
                Cue::Notice(seconds) => {
                    // FIRE-6307: a line in the nearby chat at each notice
                    self.world
                        .system_notice(aurora_i18n::tr!("region-restart-chat"), crate::world::notice_topics::Topic::Region);
                    self.announce_restart(Some(seconds));
                    if let Some(id) = self.region_restart.bell
                        && let Some(n) = self.world.notifications.list.iter_mut().find(|n| n.id == id)
                    {
                        n.body = toast_text(&region, seconds);
                    }
                }
                Cue::Step(Level::Zero) => {
                    self.world.system_notice(
                        aurora_i18n::tr!("region-restart-chat-zero"),
                        crate::world::notice_topics::Topic::Region,
                    );
                    self.drop_restart_bell();
                }
                Cue::Step(_) | Cue::Back => {}
            }
        }
    }

    /// FSReportRegionRestartToChat: `region_restart_in:<s>` (or the
    /// cancellation) whispered on FSRegionRestartAnnounceChannel.
    fn announce_restart(&mut self, seconds: Option<i32>) {
        let s = &self.settings.region_restart;
        if s.show_alert && s.announce && s.channel != 0 {
            let channel = s.channel;
            self.send(NetCommand::Chat {
                message: aurora_net::region_restart::announcement(seconds),
                channel,
                chat_type: ChatType::Whisper,
            });
        }
    }

    /// The pinned card leaves the bell (for its history).
    fn drop_restart_bell(&mut self) {
        if let Some(id) = self.region_restart.bell.take() {
            self.world.notifications.note(id, crate::world::notification_history::Outcome::Read);
            self.world.notifications.list.retain(|n| n.id != id);
        }
    }

    /// Each frame: the demo, the count, the region left and the view shake.
    pub(super) fn update_region_restart(&mut self) {
        let now = Instant::now();
        let real = self
            .region_restart
            .last_tick
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64())
            .min(0.25);
        self.region_restart.last_tick = Some(now);
        if self.demo {
            self.demo_region_restart();
        }
        let counting = self.world.region_restart.alert.as_ref().is_some_and(|a| a.phase == Phase::Live);
        if let Some(handle) = self.world.main_region.filter(|_| counting) {
            let arrival = self.world.region_name();
            let before = self.world.region_restart.alert.as_ref().map(|a| a.phase.clone());
            self.world.region_restart.region_changed(handle, &arrival);
            let after = self.world.region_restart.alert.as_ref().map(|a| a.phase.clone());
            if before != after {
                // LLFloaterRegionRestarting::regionChange: the box closes
                self.drop_restart_bell();
                self.region_restart.legacy_open = false;
            }
        }
        let rate = match (&self.region_restart.demo, self.world.region_restart.alert.as_ref()) {
            (Some(d), _) if d.still => 0.0,
            (Some(d), Some(a)) => d.rate(a.seconds()),
            _ => 1.0,
        };
        let cues = self.world.region_restart.advance(real * rate, real);
        if !cues.is_empty() {
            self.region_restart_cues(&cues);
        }
        // the view shake: a pan of the eye only
        self.camera.shake = glam::Vec3::ZERO;
        if let Some(t) = self.region_restart.view_shake_at {
            match view_shake(now.duration_since(t).as_secs_f64()) {
                Some((x, y)) => self.camera.shake = self.camera.pan_offset(x, y),
                None => self.region_restart.view_shake_at = None,
            }
        }
        if self.world.region_restart.alert.is_none() {
            self.drop_restart_bell();
        }
    }

    /// The live count of the bell's card (Aurora's alert only).
    pub(super) fn restart_bell_info(&self, p: &crate::theme::Palette) -> Option<BellInfo> {
        let a = self.world.region_restart.alert.as_ref()?;
        (a.live() && !self.classic_restart()).then(|| BellInfo {
            region: a.region.clone(),
            seconds: a.seconds(),
            color: alert::color(p, a),
        })
    }

    /// An answer of the bell's card: Home, or "Show the alert". True when it
    /// was the restart's card.
    pub(super) fn region_restart_bell_answer(&mut self, id: u64, r: &Response) -> bool {
        if self.region_restart.bell != Some(id) {
            return false;
        }
        match r {
            Response::Accept => {
                self.notif_ui.open = false;
                self.region_restart_action(Action::Home);
            }
            Response::Open => {
                self.notif_ui.open = false;
                self.world.region_restart.show();
                let now = self.egui_ctx.input(|i| i.time);
                self.region_restart.ui.opened(now);
            }
            _ => {}
        }
        true
    }

    /// The landmarks of the inventory, out of the trash and the library,
    /// with where each leads once its asset is known.
    fn restart_landmarks(&mut self) -> Vec<Landmark> {
        let inv = &self.world.inventory;
        if self.region_restart.landmarks.0 != Some(inv.generation) {
            let list = inv
                .items
                .values()
                .filter(|it| it.asset_type == ASSET_LANDMARK && !it.asset_id.is_nil())
                .filter(|it| inv.folders.get(&it.parent).is_some_and(|f| !f.library) && !inv.in_folder_type(it.id, FOLDER_TRASH))
                .map(|it| Landmark {
                    asset: it.asset_id,
                    name: it.name.clone(),
                    detail: String::new(),
                    created: it.created_at,
                })
                .collect();
            self.region_restart.landmarks = (Some(inv.generation), list);
        }
        let mut list = self.region_restart.landmarks.1.clone();
        // where the listed ones lead (their asset asked once)
        let shown: Vec<Uuid> = alert::listed(&list, self.region_restart.ui.query())
            .iter()
            .map(|l| l.asset)
            .collect();
        for asset in shown {
            self.world.landmarks.resolve(asset);
        }
        for l in &mut list {
            if let crate::world::landmarks::Resolved::Ready { global, region_pos, .. } = self.world.landmarks.peek(l.asset)
                && let Some((_, sim)) = self.world.map.sim_at_global(global.x, global.y)
            {
                l.detail =
                    crate::world::worldmap::location_label(&sim.name, f64::from(region_pos.x), f64::from(region_pos.y), region_pos.z);
            }
        }
        list
    }

    /// The closest neighbour region on the map that is up: its grid slot
    /// and name.
    fn restart_neighbour(&self) -> Option<((u32, u32), String)> {
        let handle = self.world.main_region?;
        let (ox, oy) = aurora_net::handle_to_origin(handle);
        let (gx, gy) = (ox / 256, oy / 256);
        let me = self.world.agent.position;
        let mut best: Option<(f32, (u32, u32), String)> = None;
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (x, y) = (gx as i64 + dx, gy as i64 + dy);
                if x < 0 || y < 0 {
                    continue;
                }
                let Some((key, sim)) = self.world.map.sim_at(x as u32, y as u32) else {
                    continue;
                };
                if sim.is_down() || key == (gx, gy) {
                    continue;
                }
                // how far the border toward it is from us
                let bx = if dx < 0 {
                    me.x
                } else if dx > 0 {
                    256.0 - me.x
                } else {
                    0.0
                };
                let by = if dy < 0 {
                    me.y
                } else if dy > 0 {
                    256.0 - me.y
                } else {
                    0.0
                };
                let d = (bx * bx + by * by).sqrt();
                if best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, key, sim.name.clone()));
                }
            }
        }
        best.map(|(_, key, name)| (key, name))
    }

    /// The most recent place of the teleport history outside this region.
    fn restart_last_place(&self) -> Option<(String, glam::DVec3)> {
        let handle = self.world.main_region?;
        let (ox, oy) = aurora_net::handle_to_origin(handle);
        let here = |g: &glam::DVec3| (g.x / 256.0).floor() as u32 == ox / 256 && (g.y / 256.0).floor() as u32 == oy / 256;
        self.world
            .tp_storage
            .items
            .iter()
            .rev()
            .find(|h| !here(&h.global))
            .map(|h| (h.title.clone(), h.global))
    }

    /// Draw the alert (or Firestorm's box) and act on its clicks.
    pub(super) fn show_region_restart(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(a) = self.world.region_restart.alert.clone() else {
            self.region_restart.ui.layer = None;
            return;
        };
        let landmarks = self.restart_landmarks();
        if self.classic_restart() {
            self.region_restart.ui.layer = None;
            if !self.region_restart.legacy_open || a.phase != Phase::Live {
                return;
            }
            let mut open = true;
            let actions = alert::legacy_box(ctx, p, &a, &landmarks, &mut self.region_restart.legacy_pick, &mut open);
            self.region_restart.legacy_open = open;
            for action in actions {
                self.region_restart_action(action);
            }
            return;
        }
        let demo = self.region_restart.demo.clone();
        let region = self
            .world
            .main_region
            .and_then(|h| self.world.regions.get(&h))
            .and_then(|r| r.info.as_ref());
        // LLViewerRegion::canManageEstate
        let manager =
            region.is_some_and(|i| i.is_estate_manager || i.owner == self.world.agent_id) || demo.as_ref().is_some_and(|d| d.manager);
        let destinations = Destinations {
            landmarks,
            neighbour: self.restart_neighbour().map(|(_, n)| n),
            last_place: self.restart_last_place().map(|(t, _)| t),
        };
        let view = alert::View {
            alert: &a,
            manager,
            pill_only: self.world.status.dnd || self.camera.mouselook() || self.photo_full_frame(),
            still: self.settings.reduce_motion,
            frozen: demo.as_ref().is_some_and(|d| d.still),
            destinations: &destinations,
        };
        let actions = alert::show(ctx, p, &mut self.region_restart.ui, &view);
        for action in actions {
            self.region_restart_action(action);
        }
    }

    fn region_restart_action(&mut self, action: Action) {
        match action {
            Action::Home => {
                self.request_user_teleport(NetCommand::TeleportHome, aurora_i18n::tr!("common-home").into(), false);
            }
            Action::Landmark(asset) => {
                // gAgent.teleportViaLandmark, without the confirmation of
                // the landmark lists (LLFloaterRegionRestarting::onTeleportClicked)
                self.request_user_teleport(
                    NetCommand::TeleportLandmark(asset),
                    aurora_i18n::tr!("common-landmark").into(),
                    false,
                );
            }
            Action::Neighbour => {
                if let Some(((x, y), _)) = self.restart_neighbour() {
                    let z = self.world.agent.position.z;
                    self.world
                        .map
                        .track_location(f64::from(x) * 256.0 + 128.0, f64::from(y) * 256.0 + 128.0, z, true);
                }
            }
            Action::LastPlace => {
                if let Some((_, g)) = self.restart_last_place() {
                    self.world.map.track_location(g.x, g.y, g.z as f32, true);
                }
            }
            Action::AllLandmarks => {
                self.world.place_details.close_panel();
                self.places_ui.open_tab(crate::ui::places::TAB_LANDMARKS);
                self.panels.places = true;
            }
            Action::Minimize => self.world.region_restart.minimize(true),
            Action::Expand => self.world.region_restart.minimize(false),
            Action::Close => {
                self.world.region_restart.close();
                if self.world.region_restart.alert.is_none() {
                    self.drop_restart_bell();
                }
            }
            Action::CancelRestart => {
                if self.demo {
                    // the demo's simulator cancels at once
                    self.on_region_restart_cancelled();
                } else {
                    self.send(NetCommand::CancelRegionRestart);
                }
            }
            // the window where restarts are scheduled and cancelled
            Action::RegionEstate => crate::ui::region::request_open(&self.egui_ctx),
        }
    }

    /// The alert above the toasts, below the list opened from the bell (the
    /// mockup's order: dialogs, menus, the open center, the alert, toasts,
    /// windows, the world).
    pub(super) fn order_region_restart(&self, ctx: &egui::Context, aurora: bool) {
        let Some(alert) = self.region_restart.ui.layer else {
            return;
        };
        let layer = |name: &str| egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(name));
        ctx.memory_mut(|m| {
            let areas = m.areas_mut();
            if aurora {
                let center = layer("notification_center");
                if self.notif_ui.open {
                    areas.set_sublayer(alert, center);
                } else {
                    areas.set_sublayer(center, alert);
                }
            } else if self.notif_ui.open {
                areas.set_sublayer(alert, layer("notification_list"));
            } else {
                areas.set_sublayer(layer("notification_toasts"), alert);
            }
        });
    }

    /// AURORA_DEMO_RESTART: the demo region announces its restart.
    fn demo_region_restart(&mut self) {
        if self.region_restart.demo.is_none() {
            let Some(d) = std::env::var("AURORA_DEMO_RESTART").ok().and_then(|v| Demo::parse(&v)) else {
                return;
            };
            self.region_restart.demo = Some(d);
        }
        let Some(d) = self.region_restart.demo.clone() else {
            return;
        };
        if d.started || self.frame_count < DEMO_AT {
            return;
        }
        if let Some(demo) = self.region_restart.demo.as_mut() {
            demo.started = true;
        }
        if d.dnd {
            self.world.set_dnd(true);
        }
        self.on_region_restart(Notice {
            seconds: d.seconds,
            region: (!d.opensim).then(|| self.world.region_name()),
            opensim: d.opensim,
        });
        // the mockup's states: a count started at 5 minutes, its bar
        // emptied that far
        if let Some(a) = self.world.region_restart.alert.as_mut() {
            a.total = a.total.max(300.0);
        }
        if d.minimized {
            self.world.region_restart.minimize(true);
        }
        if d.closed {
            self.world.region_restart.close();
        }
        if d.menu {
            self.region_restart.ui.open_menu = true;
        }
        if d.bell {
            self.notif_ui.open = true;
        }
        if d.toast {
            let agent = self.world.agent_id;
            let event = aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
                from_agent_id: crate::demo::DEMO_NOVA,
                from_name: "Nova Exemple".into(),
                to_agent_id: agent,
                dialog: 0,
                session_id: Uuid::nil(),
                message: aurora_i18n::tr!("demo-restart-toast").into(),
                offline: false,
                from_group: false,
                binary_bucket: Vec::new(),
                timestamp: 0,
            });
            if let Some(e) = self.world.apply(event) {
                self.on_app_event(e);
            }
        }
        // the screenshots show the alert alone
        self.panels.perf = false;
        self.panels.chat = false;
        self.panels.people = false;
    }
}

/// RegionRestartMinutesToast / RegionRestartSecondsToast, in whole minutes
/// above one.
fn toast_text(region: &str, seconds: i32) -> String {
    if seconds >= 60 && seconds % 60 == 0 {
        aurora_i18n::tr!("region-restart-toast-minutes", region = region, count = seconds / 60)
    } else {
        aurora_i18n::tr!("region-restart-toast-seconds", region = region, count = seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_reads_its_options() {
        let d = Demo::parse("300:fast:manager").expect("demo");
        assert_eq!((d.seconds, d.fast, d.manager, d.still), (300, true, true, false));
        assert_eq!(Demo::parse("x"), None);
        let d = Demo::parse("52:still:min").expect("demo");
        assert!(d.still && d.minimized);
    }

    /// The mockup's fast mode plays 5 minutes in about 18 seconds.
    #[test]
    fn the_fast_demo_is_about_twenty_seconds() {
        let d = Demo::parse("300:fast").expect("demo");
        let (mut left, mut t) = (300.0_f64, 0.0);
        while left > 0.0 {
            left -= d.rate(left.ceil() as i32) / 60.0;
            t += 1.0 / 60.0;
        }
        assert!((16.0..21.0).contains(&t), "{t}");
        assert_eq!(Demo::parse("300").expect("demo").rate(100), 1.0);
    }
}
