//! Visible native link actions, requested in addition to the Legacy
//! Viewer's LLUrlEntryAgent::getTooltip. Markers describe the parsed target,
//! including a labelled link, and are presentation only when copying chat.

use crate::slapp::{AgentVerb, AppLink};
use egui::{Color32, FontId, Response, TextFormat, Ui, text::LayoutJob};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Profile,
    Im,
    Pay,
    OfferTeleport,
    AddFriend,
    RemoveFriend,
    BlockResident,
    UnblockResident,
    BlockObject,
    UnblockObject,
    Report,
    Group,
    Parcel,
    Object,
    Inventory,
    Experience,
    Classified,
    Event,
    Map,
    Channel,
    Search,
    Blocked,
    Unsupported,
    Place,
    Teleport,
}

impl Action {
    pub fn app(link: &AppLink) -> Self {
        match link {
            AppLink::Agent { verb, .. } => match verb {
                AgentVerb::Im => Self::Im,
                AgentVerb::Pay => Self::Pay,
                AgentVerb::OfferTeleport => Self::OfferTeleport,
                AgentVerb::RequestFriend => Self::AddFriend,
                AgentVerb::RemoveFriend => Self::RemoveFriend,
                AgentVerb::Mute => Self::BlockResident,
                AgentVerb::Unmute => Self::UnblockResident,
                AgentVerb::Block(_) => Self::BlockObject,
                AgentVerb::Unblock(_) => Self::UnblockObject,
                AgentVerb::ReportAbuse => Self::Report,
            },
            AppLink::Group(_) => Self::Group,
            AppLink::Parcel(_) => Self::Parcel,
            AppLink::ObjectIm(_) => Self::Object,
            AppLink::Inventory { .. } | AppLink::InventoryShow => Self::Inventory,
            AppLink::Experience(_) => Self::Experience,
            AppLink::Classified(_) => Self::Classified,
            AppLink::Event(_) => Self::Event,
            AppLink::WorldMap { .. } => Self::Map,
            AppLink::Chat { .. } => Self::Channel,
            AppLink::Search { .. } => Self::Search,
            AppLink::Blocked(_) => Self::Blocked,
            AppLink::Unsupported => Self::Unsupported,
        }
    }

    pub fn place(teleport: bool) -> Self {
        if teleport { Self::Teleport } else { Self::Place }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Profile => "user-filled",
            Self::Im => "message-circle-filled",
            Self::Channel => "speakerphone",
            Self::Pay => "coin-filled",
            Self::OfferTeleport => "send-filled",
            Self::Teleport => "location-filled",
            Self::AddFriend => "user-plus",
            Self::RemoveFriend => "user-minus",
            Self::BlockResident | Self::BlockObject | Self::Blocked => "ban",
            Self::UnblockResident | Self::UnblockObject => "circle-check-filled",
            Self::Report => "flag-filled",
            Self::Group => "users-group",
            Self::Parcel | Self::Place => "map-pin-filled",
            Self::Object => "zoom-scan-filled",
            Self::Inventory => "briefcase-filled",
            Self::Experience => "layout-grid-filled",
            Self::Classified => "article-filled",
            Self::Event => "calendar-event-filled",
            Self::Map => "map",
            Self::Search => "search-filled",
            Self::Unsupported => "help-circle-filled",
        }
    }

    pub fn paint(self, painter: &egui::Painter, rect: egui::Rect, color: Color32) {
        if let Some(icon) = crate::ui::icons::global(self.icon()) {
            painter.image(
                icon.id(),
                rect.shrink(1.0),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                color,
            );
        }
    }
}

/// Reserve the icon without a word or a break before the link text.
/// Keep the text's own font metrics: a larger invisible glyph raises the
/// row height and shifts a link away from the surrounding sentence.
pub(super) fn reserve_icon(job: &mut LayoutJob, size: f32, lead: f32) {
    job.append(
        "M",
        lead,
        TextFormat {
            font_id: FontId::proportional(size),
            color: Color32::TRANSPARENT,
            ..Default::default()
        },
    );
}

/// One wrapping, clickable atom: marker and destination share interaction
/// and cannot separate into unrelated controls. Selection copies `text`.
pub(super) fn link(ui: &mut Ui, action: Action, text: &str, size: f32, color: Color32, background: Option<Color32>) -> Response {
    let behind = super::select::atom_begin(ui);
    let mut job = LayoutJob::default();
    reserve_icon(&mut job, size, 0.0);
    job.append(
        text,
        2.0,
        TextFormat {
            font_id: FontId::proportional(size),
            color,
            background: background.unwrap_or(Color32::TRANSPARENT),
            valign: ui.text_valign(),
            ..Default::default()
        },
    );
    let (origin, galley, response) = egui::Label::new(job).sense(egui::Sense::click()).layout_in_ui(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), text));
    if ui.is_rect_visible(response.rect) {
        let underline = if response.hovered() || response.has_focus() {
            egui::Stroke::new(1.0, color)
        } else {
            egui::Stroke::NONE
        };
        ui.painter()
            .add(egui::epaint::TextShape::new(origin, galley.clone(), color).with_underline(underline));
        if let Some(slot) = super::range_rects(&galley, origin, 0..1).first() {
            action.paint(
                ui.painter(),
                egui::Rect::from_center_size(slot.center(), egui::Vec2::splat(size)),
                color,
            );
        }
    }
    super::select::atom_end(ui, behind, text, response.rect);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
