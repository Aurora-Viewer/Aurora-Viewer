//! The world's hover tips each frame (`hover_tips.rs` says what they show
//! and when, `ui/hover_tips.rs` draws them): the subject the hover pick
//! found last frame, the requests its tip needs, the tip drawn and its
//! buttons.

use super::App;
use crate::hover_tips::{self as tips, Subject, TipAction};
use crate::settings::HoverTipForm;
use crate::theme::Palette;
use crate::ui::context::CtxAction;
use glam::Vec3;
use std::time::{Duration, Instant};

/// What the tips keep between frames.
#[derive(Default)]
pub(crate) struct HoverTipsState {
    pub timing: tips::Timing,
    pub parcels: tips::HoverParcels,
    /// Where the tip was drawn last frame (the pointer may rest on it).
    rect: Option<egui::Rect>,
    /// The world point of the shown subject (the land under it, the
    /// distance to an avatar).
    point: Vec3,
    /// The tip drawn last frame (the demo's log, tests).
    pub last: Option<tips::Tip>,
    /// The avatar under the pointer last frame and the point on it (the
    /// hover search leaves avatars out).
    pub avatar: Option<(usize, Vec3)>,
}

impl App {
    /// The tips are on: the saved setting, or the demo's scenario.
    pub(super) fn hover_tips_on(&self) -> bool {
        self.settings.hover_tips.enabled || (self.demo && crate::demo::hover_tips::scenario().is_some())
    }

    /// The tips of this frame, from the hover of the last one; the action
    /// of a card's button. Nothing while they are off, in mouselook or over
    /// a held photo (`shown` false).
    pub(super) fn draw_hover_tips(&mut self, ctx: &egui::Context, p: &Palette, shown: bool) -> Option<CtxAction> {
        // AURORA_DEMO_HOVER_TIPS: the scenario's tips, the saved ones kept
        let (settings, legacy) = self
            .demo
            .then(crate::demo::hover_tips::scenario)
            .flatten()
            .unwrap_or_else(|| (self.settings.hover_tips.clone(), self.settings.classic_hover_tips));
        if !settings.enabled || !shown {
            self.hover_tips.timing.hide();
            self.hover_tips.rect = None;
            self.hover_tips.last = None;
            return None;
        }
        let now = Instant::now();
        let hovered = match self.hover_tips.avatar {
            Some((idx, at)) => tips::subject_at(&self.world, Some(idx), at).map(|s| (s, at)),
            None => self
                .look_hover
                .and_then(|(idx, point)| tips::subject_at(&self.world, idx, point).map(|s| (s, point))),
        };
        let subject = hovered.map(|(s, _)| (s, s.key(&self.hover_tips.parcels)));
        // the parcel under a bare ground point, or under the subject for the
        // readout's second line (setHoverParcel)
        let readout = !legacy && settings.form == HoverTipForm::Readout;
        if settings.land
            && let Some((s, point)) = hovered
            && (matches!(s, Subject::Land { .. }) || readout)
            && let Some(Subject::Land { handle, local, origin }) = tips::subject_at(&self.world, None, point)
            && let Some(cmd) = self.hover_tips.parcels.request(handle, local, origin, now)
        {
            self.send(cmd);
        }
        let interrupted = ctx.input(|i| {
            i.pointer.any_pressed()
                || i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::MouseWheel { .. }))
        });
        // the demo's resting cursor (AURORA_DEMO_HOVER) moves only the
        // world's, egui has no pointer then
        let ppp = ctx.pixels_per_point();
        let pointer = ctx
            .input(|i| i.pointer.latest_pos())
            .or_else(|| self.demo.then(|| egui::pos2(self.cursor_pos.0 / ppp, self.cursor_pos.1 / ppp)));
        let frame = tips::Frame {
            subject,
            pointer,
            interrupted,
            tip_rect: self.hover_tips.rect,
            follows: readout,
            delay: Duration::from_secs_f32(settings.delay),
            now,
        };
        let Some(shown) = self.hover_tips.timing.update(frame) else {
            self.hover_tips.rect = None;
            self.hover_tips.last = None;
            return None;
        };
        if let Some((s, point)) = hovered
            && s.key(&self.hover_tips.parcels) == shown.key
        {
            self.hover_tips.point = point;
        }
        // the subject must still be what it was (a slot freed and reused)
        let alive = match shown.subject {
            Subject::Object { root, root_id, .. } => self.world.objects.get(root).is_some_and(|o| o.full_id == root_id),
            Subject::Avatar { idx, id } => self.world.objects.get(idx).is_some_and(|o| o.full_id == id),
            Subject::Land { .. } => true,
        };
        if !alive {
            self.hover_tips.timing.hide();
            return None;
        }
        // ObjectPropertiesFamily of the hovered root (LLSelectMgr::setHoverObject)
        // and its land impact for the card (getSelectedLinksetCost)
        let mut land_impact = None;
        if let Subject::Object { root, root_id, .. } = shown.subject
            && let Some(handle) = self.world.objects.get(root).map(|o| o.key.region)
        {
            if let Some(cmd) = self.interactions.tip_request(handle, root_id) {
                self.send(cmd);
            }
            land_impact = self.build.hover_land_impact(handle, root_id);
            self.flush_build();
        }
        let point = self.hover_tips.point;
        let sources = tips::Sources {
            props: &self.interactions.props,
            parcels: &self.hover_tips.parcels,
            land_impact,
            point,
            agent: self.world.agent.position,
        };
        let tip = tips::build(&mut self.world, shown.subject, &sources, &settings);
        // the readout's second line: the parcel the subject stands on
        let land = if readout && settings.land && !matches!(shown.subject, Subject::Land { .. }) {
            tips::subject_at(&self.world, None, point).and_then(|s| tips::build(&mut self.world, s, &sources, &settings))
        } else {
            None
        };
        let drawn = crate::ui::hover_tips::show(ctx, p, tip.as_ref(), land.as_ref(), settings.form, legacy, shown.anchor);
        if self.demo
            && tip.is_some()
            && tip.as_ref().map(|t| &t.title) != self.hover_tips.last.as_ref().map(|t| &t.title)
            && let Some(t) = &tip
        {
            log::info!(
                "demo hover tip: {:?} {:?} owner={:?} sale={:?} rows={:?} flags={:?} actions={:?} land={:?}",
                t.kind,
                t.title,
                t.owner,
                t.sale,
                t.rows,
                t.flags,
                t.actions,
                land.as_ref().map(|l| &l.title),
            );
        }
        self.hover_tips.rect = drawn.as_ref().map(|d| d.rect);
        self.hover_tips.last = tip;
        let action = drawn.and_then(|d| d.action)?;
        // a button used: the tip goes until the pointer leaves it (a press)
        self.hover_tips.timing.hide();
        Some(match action {
            TipAction::Buy(id) => CtxAction::Buy(id),
            TipAction::Pay(id) => CtxAction::Pay(id),
            TipAction::Touch(local_id) => CtxAction::Touch(local_id),
            TipAction::Inspect(id) => CtxAction::Inspect(id),
            TipAction::Profile(id) => CtxAction::Profile(id),
            TipAction::Im(id) => CtxAction::Im(id),
            TipAction::PayResident(id) => CtxAction::PayResident(id),
            TipAction::AboutLand(at) => CtxAction::AboutLand(at),
            TipAction::BuyLand(at) => CtxAction::BuyLand(at),
        })
    }

    /// Ctrl+Shift+T (View.ShowHoverTips): the tips on or off, said in the
    /// nearby chat like the other toggles of a shortcut.
    pub(super) fn toggle_hover_tips(&mut self) {
        let tips = &mut self.settings.hover_tips;
        tips.enabled = !tips.enabled;
        self.world.system_message(if tips.enabled {
            aurora_i18n::tr!("app-hover-tips-on")
        } else {
            aurora_i18n::tr!("app-hover-tips-off")
        });
        self.settings.save();
    }

    /// ParcelProperties of the hovered parcel.
    pub(super) fn on_hover_parcel(&mut self, handle: aurora_net::RegionHandle, parcel: std::sync::Arc<aurora_net::ParcelInfo>) {
        self.hover_tips.parcels.on_parcel(handle, parcel);
    }
}
