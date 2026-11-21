//! `AURORA_DEMO_OUTSIDE`, with `AURORA_DEMO` only: a window outside the
//! viewer's that shows a test picture and a few controls, to see and
//! measure what such a window costs the 3D view and to try its pointer and
//! its keyboard (docs/TESTING.md).

use crate::theme::Palette;
use crate::ui::widgets;

/// Frame at which the window opens: the world is on screen.
pub(crate) const OPENS_AT: u64 = 300;
/// Frame of the scripted click: the window has been drawn and laid out.
const CLICKS_AT: u64 = OPENS_AT + 120;

/// What the scenario shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scenario {
    /// A picture that changes at every frame (the worst case: the window
    /// is drawn at every frame of the viewer); a still one otherwise.
    pub busy: bool,
    /// The window synchronized with its screen, which a window outside
    /// never is otherwise.
    pub sync: bool,
    /// A scripted pointer clicks the window's button once.
    pub click: bool,
}

/// `still` or `busy`, then `sync` and `click` after commas.
pub(crate) fn parse(value: &str) -> Option<Scenario> {
    let mut parts = value.split(',').map(str::trim);
    let busy = match parts.next()? {
        "still" | "1" => false,
        "busy" => true,
        _ => return None,
    };
    let mut scenario = Scenario {
        busy,
        sync: false,
        click: false,
    };
    for part in parts {
        let flag = match part {
            "sync" => &mut scenario.sync,
            "click" => &mut scenario.click,
            _ => return None,
        };
        // (named twice: a mistake, not a wish)
        if std::mem::replace(flag, true) {
            return None;
        }
    }
    Some(scenario)
}

pub(crate) fn id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("aurora outside demo")
}

/// A step of the scripted click, at a point of the window's pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Pointer {
    Move(egui::Pos2),
    Press,
    Release,
}

/// The scenario and what its controls hold.
pub(crate) struct Demo {
    pub scenario: Scenario,
    clicks: u32,
    on: bool,
    text: String,
    /// Where the button was drawn last.
    button: Option<egui::Rect>,
}

impl Demo {
    pub fn new(scenario: Scenario) -> Demo {
        Demo {
            scenario,
            clicks: 0,
            on: false,
            text: String::new(),
            button: None,
        }
    }

    /// The scripted pointer's step at `frame`: over the button, then down,
    /// then up, one frame each as a hand would.
    pub fn scripted(&self, frame: u64) -> Option<Pointer> {
        if !self.scenario.click {
            return None;
        }
        match frame.checked_sub(CLICKS_AT)? {
            0 => Some(Pointer::Move(self.button?.center())),
            1 => Some(Pointer::Press),
            2 => Some(Pointer::Release),
            _ => None,
        }
    }

    /// The test picture (the palette's colours, a bar and a number that
    /// move in a busy scenario), then a button that counts its clicks, a
    /// switch and a text field.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, frame: u64) {
        let area = ui.max_rect().shrink(16.0);
        let radius = ui.visuals().window_corner_radius;
        let swatches = [p.violet, p.violet_light, p.indigo, p.teal, p.navy, p.bar, p.field, p.raised];
        let side = (area.width() / swatches.len() as f32).min(72.0);
        let track = egui::Rect::from_min_size(area.min + egui::vec2(0.0, side + 12.0), egui::vec2(area.width(), 10.0));
        {
            let painter = ui.painter();
            for (i, color) in swatches.into_iter().enumerate() {
                let min = area.min + egui::vec2(side * i as f32, 0.0);
                painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(side - 8.0, side - 8.0)), radius, color);
            }
            painter.rect_filled(track, radius, p.field);
            let at = if self.scenario.busy { (frame % 240) as f32 / 240.0 } else { 0.5 };
            let knob = egui::Rect::from_min_size(track.min + egui::vec2((track.width() - 48.0) * at, 0.0), egui::vec2(48.0, 10.0));
            painter.rect_filled(knob, radius, p.violet);
            let number = if self.scenario.busy { frame } else { 0 };
            painter.text(
                track.left_bottom() + egui::vec2(0.0, 16.0),
                egui::Align2::LEFT_TOP,
                number,
                egui::FontId::proportional(28.0),
                p.ink,
            );
        }
        let controls = egui::Rect::from_min_max(egui::pos2(area.left(), track.bottom() + 64.0), area.max);
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(controls));
        ui.horizontal(|ui| {
            // (numbers only: nothing here to translate)
            let button = widgets::primary_button_sized(ui, p, &self.clicks.to_string(), egui::vec2(72.0, 28.0));
            self.button = Some(button.rect);
            if button.clicked() {
                self.clicks += 1;
                log::info!("AURORA_DEMO_OUTSIDE: the button was clicked ({})", self.clicks);
            }
            if widgets::switch(ui, p, &mut self.on).changed() {
                log::info!("AURORA_DEMO_OUTSIDE: the switch is {}", if self.on { "on" } else { "off" });
            }
            let field = ui.add(egui::TextEdit::singleline(&mut self.text).desired_width(220.0));
            // (what is typed stays out of the log)
            if field.changed() {
                log::info!("AURORA_DEMO_OUTSIDE: the field holds {} characters", self.text.chars().count());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_is_still_or_busy_with_its_options() {
        let scenario = |busy, sync, click| Some(Scenario { busy, sync, click });
        assert_eq!(parse("still"), scenario(false, false, false));
        assert_eq!(parse("1"), scenario(false, false, false));
        assert_eq!(parse("busy"), scenario(true, false, false));
        assert_eq!(parse(" busy , sync "), scenario(true, true, false));
        assert_eq!(parse("still,click"), scenario(false, false, true));
        assert_eq!(parse("busy,click,sync"), scenario(true, true, true));
        assert_eq!(parse(""), None);
        assert_eq!(parse("busy,fast"), None);
        assert_eq!(parse("busy,sync,sync"), None);
    }

    #[test]
    fn the_scripted_pointer_goes_over_the_button_then_clicks() {
        let mut demo = Demo::new(parse("still,click").expect("a scenario"));
        // (the button has not been drawn yet)
        assert_eq!(demo.scripted(CLICKS_AT), None);
        demo.button = Some(egui::Rect::from_min_size(egui::pos2(16.0, 170.0), egui::vec2(72.0, 28.0)));
        assert_eq!(demo.scripted(CLICKS_AT - 1), None);
        assert_eq!(demo.scripted(CLICKS_AT), Some(Pointer::Move(egui::pos2(52.0, 184.0))));
        assert_eq!(demo.scripted(CLICKS_AT + 1), Some(Pointer::Press));
        assert_eq!(demo.scripted(CLICKS_AT + 2), Some(Pointer::Release));
        assert_eq!(demo.scripted(CLICKS_AT + 3), None);
        // only when the scenario asks for it
        demo.scenario.click = false;
        assert_eq!(demo.scripted(CLICKS_AT), None);
    }
}
