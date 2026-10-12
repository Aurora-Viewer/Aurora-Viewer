//! Offline gestures use the actual controls' rectangles, including drag/drop.
use super::{ColorPicker, Control};
use egui::{Event, Modifiers, Pos2};

pub fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_COLOR_PICKER").ok()).as_deref()
}

pub fn events(ctx: &egui::Context, picker: &ColorPicker, frame: u64) -> Vec<Event> {
    let Some(mode) = mode().filter(|mode| picker.is_open() || *mode == "toggle") else {
        return Vec::new();
    };
    let button = |pos, pressed| Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Modifiers::NONE,
    };
    let click = |control, start| {
        picker
            .controls
            .get(&control)
            .map(|rect| {
                if frame == start {
                    vec![Event::PointerMoved(rect.center()), button(rect.center(), true)]
                } else if frame == start + 1 {
                    vec![button(rect.center(), false)]
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default()
    };
    if mode == "toggle" {
        return match frame {
            700 | 701 => click(Control::Origin, 700),
            740 | 741 => click(Control::Origin, 740),
            780 | 781 => click(Control::Origin, 780),
            _ => Vec::new(),
        };
    }
    if matches!(mode, "follow" | "link") {
        let start_id = egui::Id::new(("demo_color_picker_drag_start", mode));
        if frame == 700 {
            let control = if mode == "follow" { Control::ParentTitle } else { Control::Title };
            if let Some(rect) = picker.controls.get(&control) {
                let start = rect.min + egui::vec2(70.0, 20.0);
                ctx.data_mut(|d| d.insert_temp(start_id, start));
                return vec![Event::PointerMoved(start), button(start, true)];
            }
        }
        if let Some(start) = ctx.data(|d| d.get_temp::<Pos2>(start_id)) {
            let offset = if mode == "follow" {
                let fraction = if frame <= 800 {
                    frame.saturating_sub(700) as f32 / 100.0
                } else {
                    1.0 - frame.saturating_sub(820) as f32 / 100.0
                }
                .clamp(0.0, 1.0);
                egui::vec2(fraction * 800.0, fraction * 40.0)
            } else {
                let fraction = (frame.saturating_sub(700) as f32 / 80.0).clamp(0.0, 1.0);
                egui::vec2(fraction * 240.0, fraction * 80.0)
            };
            let end = if mode == "follow" { 921 } else { 851 };
            if (701..end).contains(&frame) {
                return vec![Event::PointerMoved(start + offset)];
            }
            if frame == end {
                return vec![button(start + offset, false)];
            }
        }
        return Vec::new();
    }
    if frame == 680 || frame == 681 {
        return match mode {
            "lsl" => click(Control::Mode(1), 680),
            "hex" | "invalid" => click(Control::Mode(2), 680),
            "no-apply" => click(Control::Apply, 680),
            "pipette" => click(Control::Pipette, 680),
            _ => Vec::new(),
        };
    }
    if matches!(mode, "hex" | "invalid") {
        if matches!(frame, 700 | 701) {
            return click(Control::Hex, 700);
        }
        if frame == 702 {
            return vec![
                Event::Key {
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
                Event::Text(if mode == "hex" { "33AA66" } else { "GG0000" }.to_owned()),
            ];
        }
    }
    if matches!(mode, "drag" | "cancel" | "no-apply" | "palette") {
        if matches!(frame, 700 | 701) {
            return click(Control::Lightness, 700);
        }
        if let Some(grid) = picker.controls.get(&Control::Grid) {
            let fraction = ((frame.saturating_sub(720)) as f32 / 100.0).clamp(0.0, 1.0);
            let pos = Pos2::new(egui::lerp(grid.x_range(), 0.1 + fraction * 0.6), egui::lerp(grid.y_range(), 0.2));
            match frame {
                720 => return vec![Event::PointerMoved(pos), button(pos, true)],
                721..=820 => return vec![Event::PointerMoved(pos)],
                821 => return vec![button(pos, false)],
                _ => {}
            }
        }
        if mode == "palette"
            && let (Some(current), Some(slot)) = (picker.controls.get(&Control::Current), picker.controls.get(&Control::Slot(31)))
        {
            return match frame {
                840 => vec![Event::PointerMoved(current.center()), button(current.center(), true)],
                841 => vec![Event::PointerMoved(current.center() + egui::vec2(15.0, 0.0))],
                842..=844 => vec![Event::PointerMoved(slot.center())],
                845 => vec![button(slot.center(), false)],
                _ => Vec::new(),
            };
        }
        if matches!(frame, 850 | 851) {
            return click(if mode == "cancel" { Control::Cancel } else { Control::Accept }, 850);
        }
    }
    Vec::new()
}
