//! AURORA_DEMO_CAMERA scenarios (demo mode): scripted mouse and key input
//! so the camera can be checked and captured without a grid. The camera is
//! logged at the frames marked `Log`.
//!
//! - `alt,x,y`: Alt+click at (x, y) (physical pixels) at frame 600, Ctrl+Alt
//!   drag (orbit) until 660, then the up arrow from 700 to 760 (back behind
//!   the avatar, FSResetCameraOnMovement)
//! - `pan,x,y` / `zoom,x,y`: the same with a Ctrl+Alt+Shift (pan) or an Alt
//!   (zoom closer) drag
//! - `crouch-alt,x,y` / `crouch-pan,x,y` / `crouch-zoom,x,y`: the same after
//!   toggling crouch at 560 and releasing its key at 570. The chosen view
//!   must remain after the mouse is released, until walking at 700.
//! - `close,x,y[,n]`: Alt+click at (x, y) at frame 600, Alt drag closer for n
//!   frames (60 by default), then the camera stays there: a close-up on a
//!   still camera, for comparing consecutive captures
//! - `drift,x,y[,n]`: the same close-up, then a very slow Ctrl+Alt orbit
//!   (a fiftieth of a pixel per frame) until frame 1200: edges that drift
//!   steadily when the picture is exact, for comparing consecutive captures
//! - `zoom-resident`: the People list's Zoom on the nearest standing resident
//!   at frame 600, who then walks south from 660 to 760 and west until 860:
//!   the camera keeps facing them all along
//! - `tag`: left press on our own name tag at frame 600, drag (steer the
//!   avatar, as when the avatar itself is held) until 660
//! - `ml`: into mouselook at frame 600, out at 700
//! - `ml-alt`: into mouselook at 600, Alt held from 620 (the cursor freed,
//!   no crosshair) with mouse motion from 625 to 640 (the view stays), Alt
//!   let go at 650 and the same motion from 655 to 670 (the view turns), out
//!   at 700
//! - `ground-ml-body-on|off`: ground sit at 520, enter at 600, stand at 760.
//! - `ml-eyes-on-body-on|ml-eyes-off-body-on`: animate the head with optional
//!   eye following; prefix `ground-` to exercise ground sitting.
//! - `wheel`: 3 wheel clicks out at 600, 12 in at 650 (into mouselook)
//! - `ml-zoom`: into mouselook at 600, right button held at 640 (no zoom
//!   yet: the normal angle stays until the wheel sets one), 3 wheel clicks in at
//!   650, released at 680 (the normal angle), held again at 720 (the
//!   remembered zoom at once), released at 760, a wheel click out at 800
//!   (leaves mouselook)
//! - `ml-zoom-alt`: into mouselook at 600, Alt + right click at 640: the
//!   context menu opens only with "Context menu with Alt + right click"
//! - `fly`: flying forward from frame 300 (camera lag)
//! - `sit`: sit on a turning seat with a sit camera at frame 300, stand up
//!   at 700 (demo::sit_events)
//! - `sit-ml` / `sit-ml-reclined`: the same seat, upright or tilted, delayed
//!   400 frames past the loading fade; first person with mouse look on both axes.
//! - `right-click,x,y` / `right-jitter,x,y`: right click with small motion / repeated
//!   back-and-forth motion: the camera stays still and the menu opens on release
//! - `right-drag,x,y` / `right-slow,x,y`: held right button with fast / slow motion:
//!   the camera orbits and no menu opens on release
//! - `scripted`: follow from 300, locked second source at 500, clear it at
//!   700 (first source resumes), clear the first at 850 (normal view).
//! - `scripted-sit`: sit at 300, root camera at 320, linked locked camera
//!   at 500, stand at 700 without a ClearFollowCamProperties packet.

/// What the app does this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Cursor(f32, f32),
    /// The cursor on the middle of our own name tag.
    CursorOnOwnTag,
    Mods {
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
    /// Hold / release AltGr, which winit reports without Ctrl nor Alt.
    AltGraph(bool),
    Press,
    Release,
    RightButton(bool),
    /// Mouse motion in pixels (y down).
    Drag(f32, f32),
    /// Hold / release the up arrow.
    Walk(bool),
    /// Enable toggle crouch and hold / release its Down input.
    Crouch(bool),
    ToggleMouselook,
    GroundSit,
    /// Wheel clicks, SL sign (positive zooms out).
    Wheel(f32),
    Fly,
    /// The seat scenario's simulator messages for this frame.
    SitEvents,
    FollowEvents,
    /// People › Near you double click on the demo resident (Avatar.ZoomIn).
    ZoomResident,
    /// Slide the demo resident by (x, y) metres, as if it walked.
    MoveResident(f32, f32),
    Log,
}

pub fn steps(spec: &str, frame: u64) -> Vec<Step> {
    let frame = if spec.starts_with("sit-ml") {
        frame.saturating_sub(400)
    } else {
        frame
    };
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    let num = |i: usize| parts.get(i).and_then(|v| v.parse::<f32>().ok());
    let mut out = Vec::new();
    match parts.first().copied().unwrap_or("") {
        mode @ ("right-click" | "right-jitter" | "right-drag" | "right-slow") => {
            let (Some(x), Some(y)) = (num(1), num(2)) else {
                return out;
            };
            match frame {
                600 => out.extend([Step::Cursor(x, y), Step::RightButton(true), Step::Log]),
                601..=660 => {
                    let drag = match mode {
                        "right-click" if frame == 601 => Some((6.0, 6.0)),
                        "right-click" => None,
                        "right-jitter" if !frame.is_multiple_of(2) => Some((2.0, -1.0)),
                        "right-jitter" => Some((-2.0, 1.0)),
                        "right-slow" => Some((1.0, 0.0)),
                        _ => Some((6.0, 1.0)),
                    };
                    if let Some((dx, dy)) = drag {
                        out.push(Step::Drag(dx, dy));
                    }
                    if frame == 630 {
                        out.push(Step::Log);
                    }
                }
                661 => out.extend([Step::RightButton(false), Step::Log]),
                700 => out.push(Step::Log),
                _ => {}
            }
        }
        mode @ ("alt" | "altgr" | "pan" | "zoom" | "crouch-alt" | "crouch-pan" | "crouch-zoom") => {
            let (Some(x), Some(y)) = (num(1), num(2)) else {
                return out;
            };
            let (ctrl, shift, drag) = match mode {
                "pan" | "crouch-pan" => (true, true, Step::Drag(4.0, 2.0)),
                "zoom" | "crouch-zoom" => (false, false, Step::Drag(0.0, -3.0)),
                _ => (true, false, Step::Drag(6.0, 1.0)),
            };
            // `altgr`: the orbit of `alt` with AltGr alone held (alt_graph.rs)
            let altgr = mode == "altgr";
            let (ctrl, alt) = if altgr { (false, false) } else { (ctrl, true) };
            match frame {
                560 if mode.starts_with("crouch-") => out.push(Step::Crouch(true)),
                570 if mode.starts_with("crouch-") => out.push(Step::Crouch(false)),
                600 => out.extend([
                    Step::Cursor(x, y),
                    Step::Mods {
                        ctrl: false,
                        shift: false,
                        alt,
                    },
                    Step::AltGraph(altgr),
                    Step::Press,
                    Step::Log,
                ]),
                601..=660 => {
                    out.extend([Step::Mods { ctrl, shift, alt }, drag]);
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
                    Step::AltGraph(false),
                    Step::Log,
                ]),
                700 => out.extend([Step::Walk(true), Step::Log]),
                690 | 705 | 715 | 730 => out.push(Step::Log),
                760 => out.extend([Step::Walk(false), Step::Log]),
                _ => {}
            }
        }
        "close" => {
            let (Some(x), Some(y)) = (num(1), num(2)) else {
                return out;
            };
            let drags = num(3).map_or(60, |n| n.clamp(1.0, 600.0) as u64);
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
                f if f > 600 && f <= 600 + drags => out.push(Step::Drag(0.0, -3.0)),
                f if f == 601 + drags => out.extend([
                    Step::Release,
                    Step::Mods {
                        ctrl: false,
                        shift: false,
                        alt: false,
                    },
                    Step::Log,
                ]),
                _ => {}
            }
        }
        "drift" => {
            let (Some(x), Some(y)) = (num(1), num(2)) else {
                return out;
            };
            let drags = num(3).map_or(60, |n| n.clamp(1.0, 500.0) as u64);
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
                f if f > 600 && f <= 600 + drags => out.push(Step::Drag(0.0, -3.0)),
                f if f > 600 + drags && f <= 1200 => out.extend([
                    Step::Mods {
                        ctrl: true,
                        shift: false,
                        alt: true,
                    },
                    Step::Drag(0.02, 0.0),
                ]),
                1201 => out.extend([
                    Step::Release,
                    Step::Mods {
                        ctrl: false,
                        shift: false,
                        alt: false,
                    },
                    Step::Log,
                ]),
                _ => {}
            }
        }
        "zoom-resident" => {
            match frame {
                600 => out.push(Step::ZoomResident),
                // a 1.5 m walk sideways (south), then 1.2 m west, clear of
                // the plaza's panels
                660..=760 => out.push(Step::MoveResident(0.0, -0.015)),
                761..=860 => out.push(Step::MoveResident(-0.012, 0.0)),
                _ => {}
            }
            if matches!(frame, 600 | 630 | 700 | 760 | 820 | 900) {
                out.push(Step::Log);
            }
        }
        "tag" => match frame {
            600 => out.extend([
                Step::CursorOnOwnTag,
                Step::Mods {
                    ctrl: false,
                    shift: false,
                    alt: false,
                },
                Step::Press,
                Step::Log,
            ]),
            601..=660 => {
                out.push(Step::Drag(6.0, 0.0));
                if frame == 630 {
                    out.push(Step::Log);
                }
            }
            661 => out.extend([Step::Release, Step::Log]),
            _ => {}
        },
        mode @ ("ml-body-on"
        | "ml-body-off"
        | "ground-ml-body-on"
        | "ground-ml-body-off"
        | "ml-eyes-on-body-on"
        | "ml-eyes-off-body-on"
        | "ground-ml-eyes-on-body-on"
        | "ground-ml-eyes-off-body-on") => match frame {
            520 | 760 if mode.starts_with("ground-") => out.push(Step::GroundSit),
            600 | 700 => out.extend([Step::ToggleMouselook, Step::Log]),
            620..=640 => {
                out.push(Step::Drag(0.0, 8.0));
                if matches!(frame, 620 | 625) {
                    out.push(Step::Log);
                }
            }
            605 | 610 | 615 | 650 | 680 | 740 | 800 => out.push(Step::Log),
            _ => {}
        },
        "ml" => match frame {
            600 | 700 => out.extend([Step::ToggleMouselook, Step::Log]),
            604 | 608 | 612 | 620 | 640 | 704 | 708 | 712 | 720 | 740 => out.push(Step::Log),
            _ => {}
        },
        "ml-alt" => {
            let alt = |alt| Step::Mods {
                ctrl: false,
                shift: false,
                alt,
            };
            match frame {
                600 | 700 => out.extend([Step::ToggleMouselook, Step::Log]),
                620 => out.push(alt(true)),
                650 => out.push(alt(false)),
                625..=640 | 655..=670 => out.push(Step::Drag(8.0, 0.0)),
                _ => {}
            }
            if matches!(frame, 622 | 641 | 652 | 671) {
                out.push(Step::Log);
            }
        }
        "ml-zoom" => match frame {
            600 => out.extend([Step::ToggleMouselook, Step::Log]),
            640 | 720 => out.extend([Step::RightButton(true), Step::Log]),
            650 => out.extend([Step::Wheel(-3.0), Step::Log]),
            680 | 760 => out.extend([Step::RightButton(false), Step::Log]),
            800 => out.extend([Step::Wheel(1.0), Step::Log]),
            _ => {}
        },
        "ml-zoom-alt" => match frame {
            600 => out.extend([Step::ToggleMouselook, Step::Log]),
            640 => out.extend([
                Step::Mods {
                    ctrl: false,
                    shift: false,
                    alt: true,
                },
                Step::RightButton(true),
                Step::Log,
            ]),
            650 => out.extend([
                Step::RightButton(false),
                Step::Mods {
                    ctrl: false,
                    shift: false,
                    alt: false,
                },
                Step::Log,
            ]),
            700 => out.push(Step::Log),
            _ => {}
        },
        "wheel" => match frame {
            600 => out.extend([Step::Wheel(3.0), Step::Log]),
            650 => out.extend([Step::Wheel(-12.0), Step::Log]),
            610 | 630 | 660 => out.push(Step::Log),
            _ => {}
        },
        mode @ ("sit"
        | "scripted-sit"
        | "sit-ml"
        | "sit-ml-reclined"
        | "sit-ml-body-on"
        | "sit-ml-body-off"
        | "sit-ml-reclined-body-on"
        | "sit-ml-reclined-body-off") => {
            if (250..=700).contains(&frame) {
                out.push(Step::SitEvents);
            }
            if mode == "scripted-sit" && matches!(frame, 320 | 500) {
                out.push(Step::FollowEvents);
            }
            if mode.starts_with("sit-ml") {
                match frame {
                    360 | 660 => out.push(Step::ToggleMouselook),
                    480..=500 => out.push(Step::Drag(0.0, -5.0)),
                    550..=570 => out.push(Step::Drag(0.0, 10.0)),
                    620..=640 => out.push(Step::Drag(5.0, 0.0)),
                    460 | 520 | 590 | 650 | 680 => out.push(Step::Log),
                    _ => {}
                }
            }
            if matches!(frame, 290 | 302 | 320 | 360 | 400 | 500 | 600 | 700 | 702 | 720 | 760) {
                out.push(Step::Log);
            }
        }
        "scripted" => match frame {
            300 | 500 | 700 | 850 => out.extend([Step::FollowEvents, Step::Log]),
            380 | 580 => out.push(Step::Walk(true)),
            460 | 660 => out.push(Step::Walk(false)),
            480 | 640 | 800 | 920 => out.push(Step::Log),
            _ => {}
        },
        "fly" => match frame {
            300 => out.extend([Step::Fly, Step::Walk(true), Step::Log]),
            f if f > 300 && f.is_multiple_of(30) => out.push(Step::Log),
            _ => {}
        },
        _ => {}
    }
    out
}

pub fn sit_events(spec: &str, frame: u64) -> Vec<aurora_net::NetEvent> {
    let frame = if spec.starts_with("sit-ml") {
        frame.saturating_sub(400)
    } else {
        frame
    };
    let mut events = crate::demo::sit_events(frame);
    if spec.starts_with("sit-ml-reclined") {
        for event in &mut events {
            if let aurora_net::NetEvent::ObjectUpdates { objects, .. } = event {
                for object in objects {
                    if object.local_id == 950 {
                        object.rotation *= glam::Quat::from_rotation_y(1.1);
                    }
                }
            }
        }
    }
    events
}

pub fn sit_follow_events(frame: u64, handle: aurora_net::RegionHandle, subject: glam::Vec3) -> Vec<aurora_net::NetEvent> {
    let mut events = follow_events(if frame == 320 { 300 } else { frame }, handle, subject);
    // The actual seat and linked prim of demo::sit_events. In particular,
    // standing at 700 must release these locally, without a clear packet.
    events.retain_mut(|event| {
        if let aurora_net::NetEvent::SetFollowCam { object, .. } = event {
            *object = if frame == 320 {
                crate::demo::action_id(950)
            } else {
                uuid::Uuid::from_u128(951)
            };
            true
        } else {
            false
        }
    });
    events
}

pub fn follow_events(frame: u64, handle: aurora_net::RegionHandle, subject: glam::Vec3) -> Vec<aurora_net::NetEvent> {
    use aurora_net::NetEvent;
    let first = uuid::Uuid::from_u128(0xca0001);
    let second = uuid::Uuid::from_u128(0xca0002);
    match frame {
        300 => vec![NetEvent::SetFollowCam {
            handle,
            object: first,
            properties: vec![(12, 1.0), (7, 6.0), (0, 20.0), (4, 1.0), (10, 0.0), (11, 0.0)],
        }],
        500 => {
            let eye = subject + glam::Vec3::new(-8.0, -6.0, 5.0);
            let at = subject + glam::Vec3::Z;
            vec![NetEvent::SetFollowCam {
                handle,
                object: second,
                properties: vec![
                    (12, 1.0),
                    (14, eye.x),
                    (15, eye.y),
                    (16, eye.z),
                    (18, at.x),
                    (19, at.y),
                    (20, at.z),
                    (21, 1.0),
                    (22, 1.0),
                ],
            }]
        }
        700 => vec![NetEvent::ClearFollowCam { object: second }],
        850 => vec![NetEvent::ClearFollowCam { object: first }],
        _ => Vec::new(),
    }
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

    #[test]
    fn altgr_scenario_orbits_with_altgr_alone_held() {
        let press = steps("altgr,400,300", 600);
        assert!(press.contains(&Step::AltGraph(true)) && press.contains(&Step::Press));
        // winit reports neither Ctrl nor Alt while AltGr is held
        let drag = steps("altgr,400,300", 620);
        let mods = Step::Mods {
            ctrl: false,
            shift: false,
            alt: false,
        };
        assert!(drag.contains(&mods) && drag.contains(&Step::Drag(6.0, 1.0)));
        assert!(steps("altgr,400,300", 661).contains(&Step::AltGraph(false)));
        // the orbit of `alt` otherwise (Ctrl + Alt during the drag)
        let alt = steps("alt,400,300", 620);
        assert!(alt.contains(&Step::Mods {
            ctrl: true,
            shift: false,
            alt: true,
        }));
    }

    #[test]
    fn close_scenario_zooms_then_leaves_the_camera_still() {
        assert!(steps("close,400,300", 600).contains(&Step::Press));
        assert_eq!(steps("close,400,300", 660), vec![Step::Drag(0.0, -3.0)]);
        assert!(steps("close,400,300", 661).contains(&Step::Release));
        assert!(steps("close,400,300", 700).is_empty());
        // the number of zoom frames is the camera's final distance
        assert_eq!(steps("close,400,300,20", 620), vec![Step::Drag(0.0, -3.0)]);
        assert!(steps("close,400,300,20", 621).contains(&Step::Release));
    }

    #[test]
    fn drift_scenario_zooms_then_orbits_slowly() {
        assert!(steps("drift,400,300", 600).contains(&Step::Press));
        assert_eq!(steps("drift,400,300", 630), vec![Step::Drag(0.0, -3.0)]);
        assert!(steps("drift,400,300", 900).contains(&Step::Drag(0.02, 0.0)));
        assert!(steps("drift,400,300", 1201).contains(&Step::Release));
        assert!(steps("drift,400,300", 1300).is_empty());
    }

    #[test]
    fn ml_alt_scenario_moves_the_mouse_with_and_without_alt() {
        let alt = |alt| Step::Mods {
            ctrl: false,
            shift: false,
            alt,
        };
        assert!(steps("ml-alt", 600).contains(&Step::ToggleMouselook));
        assert_eq!(steps("ml-alt", 620), vec![alt(true)]);
        assert_eq!(steps("ml-alt", 630), vec![Step::Drag(8.0, 0.0)]);
        assert_eq!(steps("ml-alt", 650), vec![alt(false)]);
        assert_eq!(steps("ml-alt", 660), vec![Step::Drag(8.0, 0.0)]);
        assert!(steps("ml-alt", 700).contains(&Step::ToggleMouselook));
    }

    #[test]
    fn tag_scenario_presses_on_own_tag_then_drags() {
        let start = steps("tag", 600);
        assert_eq!(start.first(), Some(&Step::CursorOnOwnTag));
        assert!(start.contains(&Step::Press));
        assert!(steps("tag", 620).contains(&Step::Drag(6.0, 0.0)));
        assert!(steps("tag", 661).contains(&Step::Release));
    }
}
