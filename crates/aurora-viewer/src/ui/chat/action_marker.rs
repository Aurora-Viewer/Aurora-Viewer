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
            Self::Im | Self::Channel => "message-circle-filled",
            Self::Pay => "coin-filled",
            Self::OfferTeleport | Self::Teleport => "send-filled",
            Self::AddFriend => "user-plus",
            Self::RemoveFriend => "user-minus",
            Self::BlockResident | Self::BlockObject | Self::Blocked => "ban",
            Self::UnblockResident | Self::UnblockObject => "circle-check-filled",
            Self::Report => "flag-filled",
            Self::Group => "users-group",
            Self::Parcel | Self::Place => "location-filled",
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Profile => aurora_i18n::tr!("ui-chat-link-action-profile"),
            Self::Im => aurora_i18n::tr!("ui-chat-link-action-im"),
            Self::Pay => aurora_i18n::tr!("ui-chat-link-action-pay"),
            Self::OfferTeleport => aurora_i18n::tr!("ui-chat-link-action-offer-teleport"),
            Self::AddFriend => aurora_i18n::tr!("ui-chat-link-action-add-friend"),
            Self::RemoveFriend => aurora_i18n::tr!("ui-chat-link-action-remove-friend"),
            Self::BlockResident => aurora_i18n::tr!("ui-chat-link-action-block-resident"),
            Self::UnblockResident => aurora_i18n::tr!("ui-chat-link-action-unblock-resident"),
            Self::BlockObject => aurora_i18n::tr!("ui-chat-link-action-block-object"),
            Self::UnblockObject => aurora_i18n::tr!("ui-chat-link-action-unblock-object"),
            Self::Report => aurora_i18n::tr!("ui-chat-link-action-report"),
            Self::Group => aurora_i18n::tr!("ui-chat-link-action-group"),
            Self::Parcel => aurora_i18n::tr!("ui-chat-link-action-parcel"),
            Self::Object => aurora_i18n::tr!("ui-chat-link-action-object"),
            Self::Inventory => aurora_i18n::tr!("ui-chat-link-action-inventory"),
            Self::Experience => aurora_i18n::tr!("ui-chat-link-action-experience"),
            Self::Classified => aurora_i18n::tr!("ui-chat-link-action-classified"),
            Self::Event => aurora_i18n::tr!("ui-chat-link-action-event"),
            Self::Map => aurora_i18n::tr!("ui-chat-link-action-map"),
            Self::Channel => aurora_i18n::tr!("ui-chat-link-action-channel"),
            Self::Search => aurora_i18n::tr!("ui-chat-link-action-search"),
            Self::Blocked => aurora_i18n::tr!("ui-chat-link-action-blocked"),
            Self::Unsupported => aurora_i18n::tr!("ui-chat-link-action-unsupported"),
            Self::Place => aurora_i18n::tr!("ui-chat-link-action-place"),
            Self::Teleport => aurora_i18n::tr!("ui-chat-link-action-teleport"),
        }
    }

    /// An invisible glyph reserves the icon, as web trust badges do. The
    /// nonbreaking spaces keep the action with the start of its destination.
    pub fn append(self, job: &mut LayoutJob, size: f32, color: Color32, lead: f32) -> usize {
        job.append(
            "M",
            lead,
            TextFormat {
                font_id: FontId::proportional(size * 1.15),
                color: Color32::TRANSPARENT,
                ..Default::default()
            },
        );
        let label = format!("{}:\u{a0}", self.label().replace(' ', "\u{a0}"));
        let len = 1 + label.chars().count();
        job.append(
            &label,
            2.0,
            TextFormat {
                font_id: FontId::proportional((size * 0.85).max(10.0)),
                color: color.gamma_multiply(0.85),
                ..Default::default()
            },
        );
        len
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

/// One wrapping, clickable atom: marker and destination share interaction
/// and cannot separate into unrelated controls. Selection copies `text`.
pub(super) fn link(ui: &mut Ui, action: Action, text: &str, size: f32, color: Color32, background: Option<Color32>) -> Response {
    let behind = super::select::atom_begin(ui);
    let mut job = LayoutJob::default();
    action.append(&mut job, size, color, 0.0);
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: FontId::proportional(size),
            color,
            background: background.unwrap_or(Color32::TRANSPARENT),
            ..Default::default()
        },
    );
    let (origin, galley, response) = egui::Label::new(job).sense(egui::Sense::click()).layout_in_ui(ui);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, ui.is_enabled(), format!("{}: {text}", action.label())));
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
