//! `AURORA_DEMO_OUTSIDE`, with `AURORA_DEMO` only: a window outside the
//! viewer's that shows a test picture, to see and measure what such a
//! window costs the 3D view (docs/TESTING.md).

use crate::theme::Palette;

/// Frame at which the window opens: the world is on screen.
pub(crate) const OPENS_AT: u64 = 300;

/// What the scenario shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scenario {
    /// A picture that changes at every frame (the worst case: the window
    /// is drawn at every frame of the viewer); a still one otherwise.
    pub busy: bool,
    /// The window synchronized with its screen, which a window outside
    /// never is otherwise.
    pub sync: bool,
}

/// `still` or `busy`, then `,sync`.
pub(crate) fn parse(value: &str) -> Option<Scenario> {
    let mut parts = value.split(',').map(str::trim);
    let busy = match parts.next()? {
        "still" | "1" => false,
        "busy" => true,
        _ => return None,
    };
    let sync = match parts.next() {
        None => false,
        Some("sync") => true,
        Some(_) => return None,
    };
    parts.next().is_none().then_some(Scenario { busy, sync })
}

pub(crate) fn id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("aurora outside demo")
}

/// The test picture: the palette's colours, and in a busy scenario a bar
/// that moves and the frame's number.
pub(crate) fn show(ui: &mut egui::Ui, p: &Palette, scenario: Scenario, frame: u64) {
    let area = ui.max_rect().shrink(16.0);
    let painter = ui.painter();
    let radius = ui.visuals().window_corner_radius;
    let swatches = [p.violet, p.violet_light, p.indigo, p.teal, p.navy, p.bar, p.field, p.raised];
    let side = (area.width() / swatches.len() as f32).min(72.0);
    for (i, color) in swatches.into_iter().enumerate() {
        let min = area.min + egui::vec2(side * i as f32, 0.0);
        painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(side - 8.0, side - 8.0)), radius, color);
    }
    let track = egui::Rect::from_min_size(area.min + egui::vec2(0.0, side + 12.0), egui::vec2(area.width(), 10.0));
    painter.rect_filled(track, radius, p.field);
    let at = if scenario.busy { (frame % 240) as f32 / 240.0 } else { 0.5 };
    let knob = egui::Rect::from_min_size(track.min + egui::vec2((track.width() - 48.0) * at, 0.0), egui::vec2(48.0, 10.0));
    painter.rect_filled(knob, radius, p.violet);
    let number = if scenario.busy { frame } else { 0 };
    painter.text(
        track.left_bottom() + egui::vec2(0.0, 16.0),
        egui::Align2::LEFT_TOP,
        number,
        egui::FontId::proportional(28.0),
        p.ink,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_is_still_or_busy_and_may_be_synchronized() {
        let scenario = |busy, sync| Some(Scenario { busy, sync });
        assert_eq!(parse("still"), scenario(false, false));
        assert_eq!(parse("1"), scenario(false, false));
        assert_eq!(parse("busy"), scenario(true, false));
        assert_eq!(parse(" busy , sync "), scenario(true, true));
        assert_eq!(parse("still,sync"), scenario(false, true));
        assert_eq!(parse(""), None);
        assert_eq!(parse("busy,fast"), None);
        assert_eq!(parse("busy,sync,more"), None);
    }
}
