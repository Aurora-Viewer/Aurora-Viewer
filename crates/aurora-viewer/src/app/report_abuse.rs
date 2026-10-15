//! The "Report Abuse" window's work (LLFloaterReporter::show*, getObjectInfo,
//! onClickObjPicker / closePickTool, takeNewSnapshot, onClickSend; Firestorm
//! indra/newview/llfloaterreporter.cpp, originally LGPL 2.1): opening it on
//! an avatar, an object or nothing, the object picker, the screenshot, the
//! region's categories and the sending.

use super::context_menu::linkset_root;
use super::*;
use crate::report_abuse::{self as rules, ReportTarget};
use crate::ui::report_abuse::{ReportAction, Screenshot};
use uuid::Uuid;

/// The processor's name for the report's VersionString (gSysCPU), read
/// once off the main thread.
static CPU: std::sync::OnceLock<String> = std::sync::OnceLock::new();

impl App {
    pub(super) fn open_report_abuse(&mut self, target: ReportTarget) {
        if !matches!(self.screen, Screen::World) {
            return;
        }
        if CPU.get().is_none() {
            std::thread::spawn(|| {
                use sysinfo::{CpuRefreshKind, RefreshKind, System};
                let system = System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()));
                let _ = CPU.set(system.cpus().first().map(|c| c.brand().trim().to_owned()).unwrap_or_default());
            });
        }
        if self.report_abuse.picking {
            self.report_abuse.picking = false;
        }
        let region = self.world.region_name();
        let position = self.world.agent.position;
        let location = crate::slurl::make(&region, position);
        self.report_abuse.begin(region, position, location);
        self.send(NetCommand::RequestAbuseCategories {
            language: aurora_i18n::language().into(),
        });
        match target {
            ReportTarget::Nothing => {}
            ReportTarget::Avatar(id) => self.report_avatar(id),
            ReportTarget::Object(id) => self.report_object(id),
            ReportTarget::Chat { avatar, time, text } => {
                self.report_avatar(avatar);
                self.report_abuse.form.details = rules::chat_details(&time, &text);
                rules::clamp_chars(&mut self.report_abuse.form.details, rules::DETAILS_MAX_CHARS);
            }
            ReportTarget::Experience(id) => self.report_experience(id),
        }
        // showFromChat fills the details after postBuild kept them blank
        self.report_abuse.default_details = String::new();
    }

    fn report_avatar(&mut self, id: Uuid) {
        self.world.social.want_name(id);
        let name = self.world.social.name_of(&id);
        self.report_abuse.set_avatar(id, if name == "…" { "" } else { &name });
    }

    /// getExperienceInfo: the experience's owner (its agent_id in the
    /// experience cache) is the abuser, the details name the experience;
    /// details not read yet leave the abuser to choose.
    fn report_experience(&mut self, id: Uuid) {
        let owner = self.world.experiences.details.get(&id).filter(|d| !d.invalid()).map(|d| d.owner);
        if let Some(owner) = owner.filter(|o| !o.is_nil()) {
            self.report_avatar(owner);
        }
        self.report_abuse.form.details = rules::experience_details(id, owner.is_some());
    }

    /// getObjectInfo: an attachment stands for its root, the region and the
    /// position become the object's, an avatar is reported as itself, and
    /// an object's name and owner are asked of its region.
    fn report_object(&mut self, id: Uuid) {
        if id.is_nil() {
            return;
        }
        let Some(idx) = self.world.objects.index_of_uuid(&id) else {
            return;
        };
        let idx = match crate::ui::context::wearer(&self.world, idx) {
            Some(_) => linkset_root(&self.world, idx),
            None => idx,
        };
        let Some(o) = self.world.objects.get(idx) else {
            return;
        };
        let (key, full_id, avatar) = (o.key, o.full_id, o.is_avatar());
        self.report_abuse.object_id = full_id;
        if let Some(region) = self.world.regions.get(&key.region) {
            self.report_abuse.region.clone_from(&region.name);
        }
        if let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0) {
            self.report_abuse.position = pos - self.world.region_offset(key.region).unwrap_or(Vec3::ZERO);
        }
        if avatar {
            self.report_avatar(full_id);
        } else {
            self.report_abuse.awaiting_object = Some(full_id);
            self.send(NetCommand::RequestReportObjectProperties {
                handle: key.region,
                object: full_id,
            });
        }
    }

    /// LLSelectMgr::processObjectPropertiesFamily with
    /// COMPLAINT_REPORT_REQUEST: the object's name, its owner's user name.
    pub(super) fn report_abuse_properties(&mut self, props: &[aurora_net::build::ObjectProps]) {
        let Some(wanted) = self.report_abuse.awaiting_object else {
            return;
        };
        if let Some(p) = props.iter().find(|p| p.object_id == wanted) {
            self.world.social.want_name(p.owner_id);
            let owner = self.world.legacy_name(&p.owner_id).unwrap_or_default();
            self.report_abuse.set_object_properties(&p.name, p.owner_id, &owner);
        }
    }

    /// The object picker's click (LLToolObjPicker, then closePickTool): run
    /// by the left press while it is armed. Nothing yet while the depth
    /// under the cursor is asked for (the press is run again with it).
    pub(super) fn report_abuse_pick(&mut self) {
        let picked = self.menu_pick();
        if picked.is_none() && self.clicks.waiting() {
            return;
        }
        self.report_abuse.picking = false;
        // onClickObjPicker cleared the object and its owner
        self.report_abuse.form.object_name.clear();
        self.report_abuse.form.owner_name.clear();
        if let Some((Some(idx), _)) = picked
            && let Some(id) = self.world.objects.get(idx).map(|o| o.full_id)
        {
            self.report_object(id);
        }
    }

    /// The window, its screenshot and its answers, each frame in the world.
    pub(super) fn report_abuse_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        if let Some(rx) = &self.report_abuse.shot_job
            && let Ok(shot) = rx.try_recv()
        {
            self.report_abuse.shot_job = None;
            // (no picture: the report goes without, as when the snapshot fails)
            self.report_abuse.screenshot = shot.map(|(preview, j2c)| Screenshot {
                texture: ctx.load_texture("report-abuse-screenshot", preview, Default::default()),
                j2c,
            });
        }
        let grid = match self.settings.grid {
            crate::settings::GridChoice::SecondLife | crate::settings::GridChoice::SecondLifeBeta => "Second Life".to_owned(),
            _ => crate::world::chat_log::grid_name(&self.settings.grid, &self.settings.custom_login_uri, self.demo),
        };
        let mut actions = Vec::new();
        if self.demo {
            actions = self.report_abuse_demo(&grid);
        }
        if !self.report_abuse.open && self.report_abuse.notice.is_none() && self.report_abuse.thanks.is_none() {
            return;
        }
        self.report_abuse.refresh_names(&mut self.world);
        actions.extend(self.report_abuse.show(ctx, p, &mut self.world, &grid));
        for action in actions {
            match action {
                ReportAction::Pick => {
                    // the transient tool starts with a blank object
                    self.report_abuse.picking = true;
                    self.report_abuse.form.object_name.clear();
                    self.report_abuse.form.owner_name.clear();
                }
                ReportAction::CancelPick => self.report_abuse.picking = false,
                ReportAction::RefreshScreenshot => {
                    self.report_abuse.capture_at = Some(Instant::now() + rules::SCREENSHOT_DELAY);
                }
                ReportAction::Send => self.send_abuse_report(),
            }
        }
        if self.report_abuse.capture_at.is_some() {
            ctx.request_repaint();
        }
    }

    /// gatherReport, then the network (sendReportViaCaps or the legacy way).
    fn send_abuse_report(&mut self) {
        let r = &self.report_abuse;
        let beta = matches!(self.settings.grid, crate::settings::GridChoice::SecondLifeBeta);
        let (renderer, driver) = self
            .gfx
            .as_ref()
            .map(|g| (g.renderer.info.name.clone(), g.renderer.info.driver.clone()))
            .unwrap_or_default();
        let version = aurora_version::version();
        let report = aurora_net::abuse_report::AbuseReport {
            category: r.form.category,
            position: r.position,
            object_id: r.object_id,
            abuser_id: r.abuser_id,
            summary: rules::summary(&r.form, &self.world.region_name(), beta),
            details: rules::details(&r.form, version),
            version_string: rules::version_string(version, CPU.get().map_or("", |s| s.as_str()), &renderer, &driver),
            screenshot: r.screenshot.as_ref().and_then(|s| s.j2c.clone()),
        };
        // reset in case the next report also contains this text
        self.report_abuse.copyright_seen = false;
        self.report_abuse.sending = true;
        if self.demo {
            log::info!(
                "demo abuse report: category {} abuser {} object {} screenshot {} bytes; summary {:?}",
                report.category,
                report.abuser_id,
                report.object_id,
                report.screenshot.as_ref().map_or(0, Vec::len),
                report.summary
            );
        }
        self.send(NetCommand::SendAbuseReport(Box::new(report)));
    }

    /// HelpReportAbuseConfirm and the window closes; a failure stays on it.
    pub(super) fn report_abuse_sent(&mut self, result: Result<(), String>) {
        if !self.report_abuse.sending {
            return;
        }
        self.report_abuse.sending = false;
        match result {
            Ok(()) => {
                let mut text = aurora_i18n::tr!("report-abuse-confirm").to_owned();
                if self.demo {
                    text = format!("{text}\n\n{}", aurora_i18n::tr!("report-abuse-demo-sent"));
                }
                self.report_abuse.thanks = Some(text);
                self.report_abuse.open = false;
                self.report_abuse.picking = false;
            }
            Err(e) => self.report_abuse.notice = Some(e),
        }
    }

    /// The screenshot, without the window: asked for once its delay is over
    /// and nothing else uses the renderer's capture. Before the interface
    /// is drawn, like the bug report's.
    pub(super) fn report_abuse_capture(&mut self, renderer: &mut aurora_render::Renderer) {
        let due = self.report_abuse.capture_at.is_some_and(|at| Instant::now() >= at);
        if !due
            || !self.report_abuse.open
            // Bug report form (disabled, see main.rs):
            // || self.bug_report.capture_pending
            // || self.bug_report.capture_requested.is_some()
            || self.inventory_images.auto_capture_pending
            || self.inventory_images.capturing.is_some()
            || self.scene_capture_pending
            || renderer.capture_request
            || renderer.capture_scene
        {
            return;
        }
        self.report_abuse.capture_at = None;
        self.report_abuse.capture_pending = true;
        // takeNewSnapshot: the world and the interface, this window aside
        renderer.capture_request = true;
    }

    /// The captured frame goes to a thread to be scaled and encoded.
    pub(super) fn report_abuse_captured(&mut self, renderer: &mut aurora_render::Renderer) {
        if !self.report_abuse.capture_pending {
            return;
        }
        let Some((w, h, pixels)) = renderer.captured.take() else {
            return;
        };
        self.report_abuse.capture_pending = false;
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.report_abuse.shot_job = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(rules::prepare_screenshot(w, h, pixels));
        });
    }

    /// AURORA_DEMO_REPORT_ABUSE: the window opened from the Help menu
    /// (`menu`), on Nova (`avatar`), on an object (`object`), on a chat line
    /// (`chat`), with the object picker armed (`pick`); `send`, `missing`
    /// and `copyright` fill the form and press "Report Abuse" at frame 700;
    /// `experience` opens the profile of the demo group's experience (its
    /// "Report" button), `experience-report` the window opened from it.
    fn report_abuse_demo(&mut self, grid: &str) -> Vec<ReportAction> {
        let Ok(mode) = std::env::var("AURORA_DEMO_REPORT_ABUSE") else {
            return Vec::new();
        };
        let experience = crate::demo::group::GROUP_EXPERIENCE;
        match self.frame_count {
            // the experience's profile (its "Report" opens the window), or
            // the window opened from it once the details are in
            600 if matches!(mode.as_str(), "experience" | "experience-report") => {
                self.world.experiences.open_profile(experience);
                Vec::new()
            }
            640 if mode == "experience-report" => {
                self.open_report_abuse(ReportTarget::Experience(experience));
                Vec::new()
            }
            600 => {
                let nova = crate::demo::DEMO_NOVA;
                let target = match mode.as_str() {
                    "avatar" | "send" | "copyright" => ReportTarget::Avatar(nova),
                    "object" => {
                        let me = self.world.agent.position;
                        let nearest = self
                            .world
                            .objects
                            .iter()
                            .filter(|(_, o)| !o.is_avatar() && self.world.objects.parent_of(o).is_none())
                            .min_by(|a, b| (a.1.position - me).length().total_cmp(&(b.1.position - me).length()))
                            .map(|(_, o)| o.full_id);
                        nearest.map_or(ReportTarget::Nothing, ReportTarget::Object)
                    }
                    "chat" => ReportTarget::Chat {
                        avatar: nova,
                        time: "10/14/2026 12:00".into(),
                        text: aurora_i18n::tr!("common-demo-object").into(),
                    },
                    _ => ReportTarget::Nothing,
                };
                self.open_report_abuse(target);
                if mode == "pick" {
                    self.report_abuse.picking = true;
                }
                if matches!(mode.as_str(), "send" | "copyright") {
                    let f = &mut self.report_abuse.form;
                    if let Some((value, label)) = self.report_abuse.categories.iter().find(|c| c.0 == 45) {
                        f.category = *value;
                        f.category_label.clone_from(label);
                    }
                    f.summary = "Spam".into();
                    f.details = if mode == "copyright" {
                        "Sells copies of my copyrighted textures.".into()
                    } else {
                        "Repeats the same advert in the nearby chat every few seconds.".into()
                    };
                }
                Vec::new()
            }
            700 if matches!(mode.as_str(), "send" | "missing" | "copyright") => self.report_abuse.press_send(grid),
            _ => Vec::new(),
        }
    }
}
