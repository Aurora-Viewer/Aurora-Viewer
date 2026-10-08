//! AURORA_DEMO_CAMERA scenarios (demo mode): scripted mouse and key input
//! so the camera can be checked and captured without a grid. The camera is
//! logged at the frames marked `Log`.
//!
//! - `alt,x,y`: Alt+click at (x, y) (physical pixels) at frame 600, Ctrl+Alt
//!   drag (orbit) until 660, then the up arrow from 700 to 760 (back behind
//!   the avatar, FSResetCameraOnMovement)
//! - `pan,x,y` / `zoom,x,y`: the same with a Ctrl+Alt+Shift (pan) or an Alt
//!   (zoom closer) drag
//! - `ml`: into mouselook at frame 600, out at 700
//! - `wheel`: 3 wheel clicks out at 600, 12 in at 650 (into mouselook)
//! - `fly`: flying forward from frame 300 (camera lag)
//! - `sit`: sit on a turning seat with a sit camera at frame 300, stand up
//!   at 700 (demo::sit_events)

/// What the app does this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Cursor(f32, f32),
    Mods {
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
    Press,
    Release,
    /// Mouse motion in pixels (y down).
    Drag(f32, f32),
    /// Hold / release the up arrow.
    Walk(bool),
    ToggleMouselook,
    /// Wheel clicks, SL sign (positive zooms out).
    Wheel(f32),
    Fly,
    /// The seat scenario's simulator messages for this frame.
    SitEvents,
    Log,
}

pub fn steps(spec: &str, frame: u64) -> Vec<Step> {
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
    let mut out = Vec::new();
    match parts.first().copied().unwrap_or("") {
        mode @ ("alt" | "pan" | "zoom") => {
            let (Some(x), Some(y)) = (num(1), num(2)) else {
                return out;
            };
            let (ctrl, shift, drag) = match mode {
                "pan" => (true, true, Step::Drag(4.0, 2.0)),
                "zoom" => (false, false, Step::Drag(0.0, -3.0)),
                _ => (true, false, Step::Drag(6.0, 1.0)),
            };
            match frame {
                600 => out.extend([
                    Step::Cursor(x, y),
                    Step::Mods {
                        ctrl: false,
                        shift: false,
                        alt: true,
                    },
                    Step::Press,
                    Step::Log,
                ]),
                601..=660 => {
                    out.extend([Step::Mods { ctrl, shift, alt: true }, drag]);
                    if frame == 630 {
                        out.push(Step::Log);
                    }
                }
                661 => out.extend([
                    Step::Release,
                    Step::Mods {
                        ctrl: false,
                        shift: false,
                        alt: false,
                    },
                    Step::Log,
                ]),
                700 => out.extend([Step::Walk(true), Step::Log]),
                705 | 715 | 730 => out.push(Step::Log),
                760 => out.extend([Step::Walk(false), Step::Log]),
                _ => {}
            }
        }
        "ml" => match frame {
            600 | 700 => out.extend([Step::ToggleMouselook, Step::Log]),
            604 | 608 | 612 | 620 | 640 | 704 | 708 | 712 | 720 | 740 => out.push(Step::Log),
            _ => {}
        },
        "wheel" => match frame {
            600 => out.extend([Step::Wheel(3.0), Step::Log]),
            650 => out.extend([Step::Wheel(-12.0), Step::Log]),
            610 | 630 | 660 => out.push(Step::Log),
            _ => {}
        },
        "sit" => {
            if (250..=700).contains(&frame) {
                out.push(Step::SitEvents);
            }
            if matches!(frame, 290 | 302 | 320 | 360 | 400 | 500 | 600 | 700 | 702 | 720 | 760) {
                out.push(Step::Log);
            }
        }
        "fly" => match frame {
            300 => out.extend([Step::Fly, Step::Walk(true), Step::Log]),
            f if f > 300 && f.is_multiple_of(30) => out.push(Step::Log),
            _ => {}
        },
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_scenario_clicks_drags_then_walks() {
        assert!(steps("alt,400,300", 600).contains(&Step::Press));
        assert!(steps("alt,400,300", 620).contains(&Step::Drag(6.0, 1.0)));
        assert!(steps("alt,400,300", 661).contains(&Step::Release));
        assert!(steps("alt,400,300", 700).contains(&Step::Walk(true)));
        assert!(steps("alt", 600).is_empty());
    }
}
