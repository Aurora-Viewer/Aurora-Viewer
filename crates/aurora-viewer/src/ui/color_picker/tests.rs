use super::*;
use egui::{Event as Input, Modifiers};

struct UiTest {
    ctx: egui::Context,
    picker: ColorPicker,
    prefs: Preferences,
    time: f64,
}

impl UiTest {
    fn new() -> Self {
        let mut test = Self {
            ctx: egui::Context::default(),
            picker: ColorPicker::default(),
            prefs: Preferences::default(),
            time: 0.0,
        };
        test.picker.open([0.5, 0.25, 1.0], false);
        for _ in 0..4 {
            test.frame(Vec::new());
        }
        test
    }
    fn frame(&mut self, events: Vec<Input>) -> (Option<Event>, egui::FullOutput) {
        self.time += 1.0 / 60.0;
        let mut result = None;
        let output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 760.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                result = result.or(self
                    .picker
                    .show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut self.prefs));
            },
        );
        (result, output)
    }
    fn button(pos: Pos2, pressed: bool) -> Input {
        Input::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Modifiers::NONE,
        }
    }
    fn click(&mut self, control: Control) -> (Option<Event>, egui::FullOutput) {
        let pos = self.picker.controls[&control].center();
        self.frame(vec![Input::PointerMoved(pos), Self::button(pos, true)]);
        self.frame(vec![Self::button(pos, false)])
    }
    fn text(&mut self, control: Control, text: &str) -> Option<Event> {
        self.click(control);
        self.frame(vec![
            Input::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            },
            Input::Text(text.to_owned()),
        ])
        .0
    }
}

#[test]
fn real_hex_field_validates_input_and_copy_uses_lsl_decimal_points() {
    let mut t = UiTest::new();
    t.click(Control::Mode(2));
    assert_eq!(t.text(Control::Hex, "#33AA66"), Some(Event::Preview([0.2, 170.0 / 255.0, 0.4])));
    assert_eq!(t.picker.draft.as_ref().unwrap().hex, "33AA66");
    let (_, output) = t.click(Control::CopyLsl);
    assert!(
        output
            .platform_output
            .commands
            .iter()
            .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(text) if text == "<0.200, 0.667, 0.400>"))
    );
    assert_eq!(t.text(Control::Hex, "GG0000"), None);
    assert_eq!(t.click(Control::Accept).0, None);
    assert!(t.picker.is_open());
    assert_eq!(t.click(Control::Cancel).0, Some(Event::Cancel));
    assert!(!t.picker.is_open());
}

#[test]
fn apply_off_defers_edits_and_enabling_it_previews_the_draft() {
    let mut t = UiTest::new();
    t.click(Control::Apply);
    assert!(!t.prefs.apply);
    assert_eq!(t.click(Control::Slot(18)).0, None);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, [1.0, 0.0, 0.0]);
    assert_eq!(t.click(Control::Apply).0, Some(Event::Preview([1.0, 0.0, 0.0])));
    t.click(Control::Apply);
    assert_eq!(t.click(Control::Accept).0, Some(Event::Accept([1.0, 0.0, 0.0])));
}

#[test]
fn palette_drag_saves_current_color_without_applying_another_color() {
    let mut t = UiTest::new();
    let rgb = t.picker.draft.as_ref().unwrap().rgb;
    let source = t.picker.controls[&Control::Current].center();
    let target = t.picker.controls[&Control::Slot(31)].center();
    t.frame(vec![Input::PointerMoved(source), UiTest::button(source, true)]);
    t.frame(vec![Input::PointerMoved(source + egui::vec2(15.0, 0.0))]);
    t.frame(vec![Input::PointerMoved(target)]);
    assert_eq!(t.frame(vec![UiTest::button(target, false)]).0, None);
    assert_eq!(t.prefs.palette[31], rgb);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, rgb);
}

#[test]
fn pipette_is_transient_and_escape_first_stops_it_then_cancels() {
    let mut t = UiTest::new();
    t.click(Control::Pipette);
    assert!(t.picker.pipette);
    assert!(t.picker.needs_sample((20.0, 30.0)));
    assert!(!t.picker.needs_sample((20.0, 30.0)));
    let id = Uuid::from_u128(999);
    t.picker.sample(Some((id, [0.1, 0.2, 0.3])));
    assert_eq!(t.frame(Vec::new()).0, Some(Event::Preview([0.1, 0.2, 0.3])));
    assert_eq!(t.picker.sampled, Some(id));
    let escape = || Input::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    assert_eq!(t.frame(vec![escape()]).0, None);
    assert!(!t.picker.pipette);
    assert_eq!(t.frame(vec![escape()]).0, Some(Event::Cancel));
}

#[test]
fn lsl_and_hsl_numeric_fields_commit_and_clamp_their_values() {
    let mut t = UiTest::new();
    let enter = || Input::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    t.click(Control::Mode(1));
    t.text(Control::Rgb(0), "0.125");
    t.frame(vec![enter()]);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb[0], 0.125);
    t.text(Control::Rgb(1), "2");
    t.frame(vec![enter()]);
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb[1], 1.0);
    t.text(Control::Hsl(2), "50");
    t.frame(vec![enter()]);
    assert_eq!(t.picker.draft.as_ref().unwrap().hsl[2], 0.5);
}

#[test]
fn changing_formats_keeps_float_precision_and_mixed_white_can_be_applied() {
    let mut t = UiTest::new();
    let original = t.picker.draft.as_ref().unwrap().rgb;
    t.click(Control::Mode(1));
    t.click(Control::Mode(2));
    t.click(Control::Mode(0));
    assert_eq!(t.picker.draft.as_ref().unwrap().rgb, original);
    t.picker.open([1.0; 3], true);
    t.frame(Vec::new());
    assert_eq!(t.click(Control::Slot(31)).0, Some(Event::Preview([1.0; 3])));
}
