//! Visual toolbar editing, deliberately available throughout the toybox session
//! (LLToolBarView::draw only exposes destinations during a drag).

use super::{Bar, ToolbarSettings, ToolbarUi, drawers, structure};
use crate::{
    theme::Palette,
    ui::{bars, icons::Icons, skin::ToolButton, widgets},
};
use egui::{Id, Rect};

pub fn group(buttons: &[ToolButton], command: &str) -> Option<std::ops::Range<usize>> {
    let at = buttons.iter().position(|button| button.command == command)?;
    if structure::is_separator(command) {
        return None;
    }
    let start = buttons[..at]
        .iter()
        .rposition(|b| structure::is_separator(&b.command))
        .map_or(0, |i| i + 1);
    let end = buttons[at + 1..]
        .iter()
        .position(|b| structure::is_separator(&b.command))
        .map_or(buttons.len(), |i| at + 1 + i);
    Some(start..end)
}

pub fn split_allowed(buttons: &[ToolButton], command: &str, before: bool) -> bool {
    let Some(at) = buttons.iter().position(|button| button.command == command) else {
        return false;
    };
    !structure::is_separator(command)
        && if before {
            at > 0 && !structure::is_separator(&buttons[at - 1].command)
        } else {
            at + 1 < buttons.len() && !structure::is_separator(&buttons[at + 1].command)
        }
}

pub fn fold_allowed(buttons: &[ToolButton], command: &str) -> bool {
    group(buttons, command).is_some_and(|range| !buttons[range].iter().any(|b| drawers::is_drawer(&b.command)))
}

pub fn merge_range(buttons: &[ToolButton], command: &str, before: bool) -> Option<std::ops::Range<usize>> {
    let at = buttons.iter().position(|button| button.command == command)?;
    let pivot = if structure::is_separator(command) {
        at
    } else {
        let range = group(buttons, command)?;
        if before {
            range.start.checked_sub(1)?
        } else {
            if range.end == buttons.len() {
                return None;
            }
            range.end
        }
    };
    let mut start = pivot;
    let mut end = pivot + 1;
    while start > 0 && structure::is_separator(&buttons[start - 1].command) {
        start -= 1;
    }
    while end < buttons.len() && structure::is_separator(&buttons[end].command) {
        end += 1;
    }
    (start > 0 && end < buttons.len()).then_some(start..end)
}

pub fn region(ui: &egui::Ui, p: &Palette, bar: Bar, rect: Rect, _empty: bool) {
    if !ToolbarUi::editing(ui.ctx()) {
        return;
    }
    let radius = ui.visuals().widgets.inactive.corner_radius;
    // Translucent editing guides distinguish edge bars from blocks.
    ui.painter().rect_filled(rect, radius, p.violet_light.gamma_multiply(0.12));
    ui.painter().extend(egui::Shape::dashed_line(
        &[
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ],
        egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.45)),
        6.0,
        5.0,
    ));
    {
        let name = match bar {
            Bar::Top => aurora_i18n::tr!("toolbar-edit-top"),
            Bar::Bottom => aurora_i18n::tr!("toolbar-edit-bottom"),
            Bar::Left => aurora_i18n::tr!("toolbar-edit-left"),
            Bar::Right => aurora_i18n::tr!("toolbar-edit-right"),
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(10.0),
            p.muted,
        );
    }
}

/// Keep the real readout/widget paint while editing owns its gestures.
pub(crate) fn preview(ui: &mut egui::Ui) {
    ui.visuals_mut().disabled_alpha = 1.0;
    ui.disable();
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn native_target(
    ui: &mut egui::Ui,
    p: &Palette,
    toolbar: &mut ToolbarUi,
    button: &ToolButton,
    bar: Bar,
    style: super::Style,
    locked: bool,
    rect: Rect,
) {
    let response = ui.interact(rect, bars::tool_button_id(&button.command), egui::Sense::click_and_drag());
    let response = match button.command.as_str() {
        "aurora_menu" => ui.interact(rect, Id::unique("bars_island_menu"), egui::Sense::click_and_drag()),
        "report_bug" => ui.interact(rect, Id::unique("bars_island_bug"), egui::Sense::click_and_drag()),
        _ => response,
    };
    toolbar.button(&response, bar, style, button, p, locked);
    if response.clicked() || response.secondary_clicked() {
        toolbar.selected = Some(button.command.clone());
    }
    let settings = super::appearance::settings(ui.ctx());
    source_menu(&response, p, toolbar, &settings, &button.command);
}

#[allow(clippy::too_many_arguments)]
pub fn cell(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    toolbar: &mut ToolbarUi,
    button: &ToolButton,
    bar: Bar,
    style: super::Style,
    locked: bool,
    mut look: bars::ButtonLook,
) -> egui::Response {
    look.editing = true;
    if structure::is_block(&button.command) {
        look.mode = if look.size.x >= 100.0 {
            super::ButtonMode::IconsWithText
        } else {
            super::ButtonMode::IconsOnly
        };
    }
    let selected = toolbar.selected.as_deref() == Some(&button.command);
    let response = bars::tool_button(
        ui,
        p,
        icons,
        &button.command,
        &button.icon,
        super::label(button),
        super::label(button),
        selected,
        true,
        look,
        0,
        0.0,
        false,
    );
    let response = match button.command.as_str() {
        "aurora_menu" => ui.interact(response.rect, Id::unique("bars_island_menu"), egui::Sense::click_and_drag()),
        "report_bug" => ui.interact(response.rect, Id::unique("bars_island_bug"), egui::Sense::click_and_drag()),
        _ => response,
    };
    toolbar.button(&response, bar, style, button, p, locked);
    if response.clicked() || response.secondary_clicked() {
        toolbar.selected = Some(button.command.clone());
    }
    response
}

pub fn source_menu(response: &egui::Response, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, command: &str) {
    if command != "aurora_menu"
        && !super::commands::catalogue(&response.ctx)
            .find(command)
            .is_some_and(|entry| entry.group)
    {
        return;
    }
    crate::ui::menu::dropdown_placed(
        response,
        p,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        Some((egui::RectAlign::BOTTOM_START, 8.0)),
        |ui| {
            let parent = if command == "aurora_menu" { "" } else { command };
            crate::ui::menu::searchable(ui, p, |ui| super::commands::show(ui, p, toolbar, settings, parent));
        },
    );
}

pub(super) fn header(ui: &mut egui::Ui, p: &Palette, settings: &ToolbarSettings) {
    ui.horizontal_wrapped(|ui| {
        widgets::glyph(ui, super::drawers::CUSTOMIZE_ICON, p.violet_light, 16.0);
        ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-interface-edit")).strong());
    });
    if settings.locked {
        ui.label(egui::RichText::new(aurora_i18n::tr!("toolbar-edit-locked")).small().color(p.muted));
    }
}
