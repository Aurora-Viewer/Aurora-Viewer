use super::*;

/// A run of the tuner over a settings file, as the app does each frame.
struct Run {
    s: Settings,
    config: Config,
    tuner: Tuner,
    hold: Option<Hold>,
    now: Instant,
    avatars: u32,
    within: Option<Within>,
    changes: Vec<(Duration, Tuned)>,
    start: Instant,
}

impl Run {
    fn new(draw_distance: f32, max_avatars: u32) -> Self {
        let s = Settings {
            draw_distance,
            max_avatars,
            ..Settings::default()
        };
        let now = Instant::now();
        Self {
            s,
            config: Config {
                target_fps: 30,
                continuous: true,
                ..Config::default()
            },
            tuner: Tuner::start(now),
            hold: None,
            now,
            avatars: 40,
            within: None,
            changes: Vec::new(),
            start: now,
        }
    }

    /// One frame of `work_ms`, the clock moving on by as much.
    fn frame(&mut self, work_ms: f32, paused: bool) -> Decision {
        self.now += Duration::from_secs_f32(work_ms.max(1.0) / 1000.0);
        let applied = Tuned::read(&self.s);
        let user = self.hold.as_ref().map_or(applied, |h| h.user(&self.s));
        if let Some(h) = &mut self.hold {
            h.sync(&self.s);
        }
        let d = self.tuner.tick(
            &self.config,
            Frame {
                now: self.now,
                work_ms,
                cap_fps: 0.0,
                paused,
                avatars: self.avatars,
                within_distance: self.within,
            },
            applied,
            user,
        );
        if let Decision::Set(to) = d {
            let hold = self.hold.get_or_insert_with(|| Hold::new(&self.s));
            hold.write(&mut self.s, to);
            self.tuner.written(Tuned::read(&self.s));
            self.changes.push((self.now - self.start, Tuned::read(&self.s)));
        }
        d
    }

    /// Frames of `work_ms` for `secs` seconds.
    fn run(&mut self, work_ms: f32, secs: f32) {
        let end = self.now + Duration::from_secs_f32(secs);
        while self.now < end {
            self.frame(work_ms, false);
        }
    }

    fn live(&self) -> Tuned {
        Tuned::read(&self.s)
    }

    fn user(&self) -> Tuned {
        self.hold.as_ref().map_or(self.live(), |h| h.user(&self.s))
    }
}

/// The distance mode's count: `outer` within the distance, `inner` well
/// within it.
fn w(outer: u32, inner: u32) -> Option<Within> {
    Some(Within { outer, inner })
}

/// 30 frames per second: 33.3 ms. Too slow past 36.7, room under 30.
const SLOW: f32 = 50.0;
const ON: f32 = 33.0;
const FAST: f32 = 20.0;

#[test]
fn it_is_off_by_default_like_the_legacy_viewer() {
    let c = Config::default();
    assert_eq!(c.target_fps, 15, "TargetFPS");
    assert!(!c.continuous, "AutoTuneLock");
    assert!(c.keep_continuous, "KeepAutoTuneLock");
    assert_eq!(c.strategy, Strategy::AvatarsAndScene, "TuningFPSStrategy");
    assert!(!c.by_distance, "AutoTuneImpostorByDistEnabled");
    assert_eq!(c.impostor_distance, 64.0, "AutoTuneImpostorFarAwayDistance");
    assert_eq!(c.min_draw_distance, 32.0, "AutoTuneRenderFarClipMin");
    assert!(!c.runs_at_login());
    assert!(
        Config {
            continuous: true,
            ..c.clone()
        }
        .runs_at_login()
    );
    assert!(
        !Config {
            continuous: true,
            keep_continuous: false,
            ..c
        }
        .runs_at_login()
    );
    assert!(Settings::default().autotune_hold.is_none());
}

#[test]
fn the_config_is_sanitized() {
    let mut c = Config {
        target_fps: 0,
        impostor_distance: f32::NAN,
        min_draw_distance: 9999.0,
        ..Config::default()
    };
    c.sanitize();
    assert_eq!((c.target_fps, c.impostor_distance, c.min_draw_distance), (1, 64.0, 256.0));
    c.target_fps = 5000;
    c.impostor_distance = 1.0;
    c.min_draw_distance = -3.0;
    c.sanitize();
    assert_eq!((c.target_fps, c.impostor_distance, c.min_draw_distance), (300, 16.0, 32.0));
}

#[test]
fn nothing_changes_before_the_resume_wait_and_enough_frames() {
    let mut r = Run::new(256.0, 16);
    r.run(SLOW, RESUME_WAIT.as_secs_f32() - 0.1);
    assert!(r.changes.is_empty());
    assert_eq!(r.tuner.status, Status::Waiting);
}

#[test]
fn too_slow_lowers_the_avatars_first_then_the_draw_distance_down_to_the_floors() {
    let mut r = Run::new(256.0, 16);
    r.run(SLOW, 120.0);
    // ours and the five nearest stay in full, the distance at its minimum
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 32.0,
            max_avatars: AVATAR_FLOOR
        }
    );
    assert_eq!(r.tuner.status, Status::CannotReach, "never reached: it stops at the floors");
    // the avatars went first, one at a time
    let first_dd = r
        .changes
        .iter()
        .position(|(_, t)| t.draw_distance < 256.0)
        .expect("draw distance lowered");
    assert!(r.changes[..first_dd].iter().all(|(_, t)| t.draw_distance == 256.0));
    assert_eq!(r.changes[first_dd - 1].1.max_avatars, AVATAR_FLOOR);
    // 10 m steps, a change interval apart at least
    for w in r.changes.windows(2) {
        assert!(w[1].0 - w[0].0 >= CHANGE_INTERVAL, "{w:?}");
        assert!(w[0].1.draw_distance - w[1].1.draw_distance <= DD_STEP);
    }
    // the user's values are untouched
    assert_eq!(
        r.user(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
    // never reached: no more changes, no loop
    let n = r.changes.len();
    r.run(SLOW, 60.0);
    assert_eq!(r.changes.len(), n);
}

#[test]
fn a_crowd_smaller_than_the_limit_goes_straight_to_what_changes_something() {
    let mut r = Run::new(256.0, 16);
    r.avatars = 9;
    r.run(SLOW, 6.5);
    // 16 in full with 9 around is 9 in full: the first step draws 8
    assert_eq!(r.changes[0].1.max_avatars, 8);
}

#[test]
fn the_avatars_only_strategy_never_touches_the_draw_distance() {
    let mut r = Run::new(256.0, 16);
    r.config.strategy = Strategy::AvatarsOnly;
    r.run(SLOW, 60.0);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: AVATAR_FLOOR
        }
    );
    assert_eq!(r.tuner.status, Status::CannotReach);
}

#[test]
fn the_minimum_draw_distance_is_kept_and_never_raises_a_shorter_users() {
    let mut r = Run::new(256.0, 16);
    r.config.min_draw_distance = 128.0;
    r.run(SLOW, 120.0);
    assert_eq!(r.live().draw_distance, 128.0);
    // a user distance under the minimum is never raised to it
    let mut r = Run::new(64.0, 16);
    r.config.min_draw_distance = 128.0;
    r.run(SLOW, 120.0);
    assert_eq!(r.live().draw_distance, 64.0);
}

#[test]
fn at_the_target_nothing_changes() {
    let mut r = Run::new(256.0, 16);
    r.run(ON, 60.0);
    assert!(r.changes.is_empty());
    assert_eq!(r.tuner.status, Status::OnTarget);
}

#[test]
fn a_sudden_spike_changes_nothing() {
    let mut r = Run::new(256.0, 16);
    r.run(ON, 8.0);
    // a few long frames (a texture upload, a region crossing): the median holds
    for _ in 0..10 {
        r.frame(400.0, false);
    }
    r.run(ON, 8.0);
    assert!(r.changes.is_empty(), "{:?}", r.changes);
}

#[test]
fn a_sudden_lasting_drop_lowers_then_gives_back_when_it_ends() {
    let mut r = Run::new(256.0, 16);
    r.run(ON, 8.0);
    r.run(SLOW, 10.0);
    assert!(r.live().max_avatars < 16);
    // the load goes away: back to the user's values, never above
    r.run(FAST, 300.0);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
    for (_, t) in &r.changes {
        assert!(t.draw_distance <= 256.0 && t.max_avatars <= 16);
    }
}

#[test]
fn a_pause_waits_and_measures_again() {
    let mut r = Run::new(256.0, 16);
    r.run(ON, 8.0);
    // the window goes to the background, a teleport: no tuning, then 5 s
    for _ in 0..100 {
        assert_eq!(r.frame(SLOW, true), Decision::Keep);
    }
    assert_eq!(r.tuner.status, Status::Waiting);
    r.run(SLOW, RESUME_WAIT.as_secs_f32() + 0.5);
    assert!(r.changes.is_empty(), "nothing before the wait and fresh frames");
    r.run(SLOW, 2.0);
    assert!(!r.changes.is_empty());
}

#[test]
fn an_up_and_down_loop_backs_off_to_one_try_every_two_minutes() {
    // a scene right at the edge: fast with a step given back, slow with it
    let mut r = Run::new(256.0, 16);
    r.avatars = 40;
    let mut last_level = 16;
    let end = r.now + Duration::from_secs(1800);
    while r.now < end {
        let ms = if r.live().max_avatars >= 12 { SLOW } else { FAST };
        r.frame(ms, false);
        last_level = r.live().max_avatars;
    }
    assert!(last_level <= 12);
    // the changes thin out: the wait before a step up doubles at each bounce
    let ups: Vec<Duration> = r
        .changes
        .windows(2)
        .filter(|w| w[1].1.max_avatars > w[0].1.max_avatars)
        .map(|w| w[1].0)
        .collect();
    assert!(ups.len() >= 3, "{ups:?}");
    let gaps: Vec<Duration> = ups.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.windows(2).all(|g| g[1] >= g[0]), "{gaps:?}");
    assert!(*gaps.last().expect("gaps") >= MAX_UP_WAIT);
    // at most one round trip per two minutes at the end
    let last_ten_minutes = r.changes.iter().filter(|(t, _)| *t > Duration::from_secs(1200)).count();
    assert!(last_ten_minutes <= 12, "{last_ten_minutes}");
}

#[test]
fn without_continuous_it_stops_once_the_target_is_held_and_keeps_the_values() {
    let mut r = Run::new(256.0, 16);
    r.config.continuous = false;
    // slow until a few avatars are impostors, then on target
    let mut finished = false;
    let end = r.now + Duration::from_secs(120);
    while r.now < end && !finished {
        let ms = if r.live().max_avatars > 10 { SLOW } else { ON };
        finished = r.frame(ms, false) == Decision::Finish;
    }
    assert!(finished);
    assert_eq!(r.tuner.status, Status::Finished);
    let kept = r.live();
    assert_eq!(kept.max_avatars, 10);
    // stopped: nothing more changes, slow frames or not
    r.run(SLOW, 30.0);
    assert_eq!(r.live(), kept);
    // the user's values are still apart, and come back
    let hold = r.hold.take().expect("held");
    hold.restore(&mut r.s);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
}

#[test]
fn turning_it_off_gives_the_users_values_back() {
    let mut r = Run::new(256.0, 16);
    r.run(SLOW, 60.0);
    assert_ne!(
        r.live(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
    r.hold.take().expect("held").restore(&mut r.s);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
}

#[test]
fn a_change_by_the_user_while_it_runs_is_theirs() {
    let mut r = Run::new(256.0, 16);
    r.run(SLOW, 40.0);
    // the user sets 128 m and 4 avatars in Preferences
    r.s.draw_distance = 128.0;
    r.s.max_avatars = 4;
    r.frame(ON, false);
    assert_eq!(
        r.user(),
        Tuned {
            draw_distance: 128.0,
            max_avatars: 4
        }
    );
    // tuning goes on from there, never above it
    r.run(FAST, 300.0);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 128.0,
            max_avatars: 4
        }
    );
    r.hold.take().expect("held").restore(&mut r.s);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 128.0,
            max_avatars: 4
        }
    );
}

#[test]
fn the_hold_never_writes_above_the_users_values_nor_outside_the_bounds() {
    let mut s = Settings {
        draw_distance: 200.0,
        max_avatars: 10,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    for (dd, n) in [(1e9, 1000), (-5.0, 0), (f32::NAN, 7), (f32::INFINITY, 3), (31.0, 11)] {
        h.write(
            &mut s,
            Tuned {
                draw_distance: dd,
                max_avatars: n,
            },
        );
        assert!((DD_FLOOR..=200.0).contains(&s.draw_distance), "{dd} -> {}", s.draw_distance);
        assert!((1..=10).contains(&s.max_avatars), "{n} -> {}", s.max_avatars);
    }
    // a user distance under Aurora's floor is not raised either
    let mut s = Settings {
        draw_distance: 20.0,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    h.write(
        &mut s,
        Tuned {
            draw_distance: 10.0,
            max_avatars: 1,
        },
    );
    assert_eq!(s.draw_distance, 20.0);
}

#[test]
fn the_distance_mode_draws_in_full_only_the_avatars_in_range_never_more_than_the_users() {
    let mut r = Run::new(256.0, 16);
    r.config.by_distance = true;
    r.within = w(9, 9);
    r.run(ON, 7.0);
    assert_eq!(r.live().max_avatars, 9);
    // more in range than the user's limit: the user's limit
    r.within = w(60, 60);
    r.run(ON, 3.0);
    assert_eq!(r.live().max_avatars, 16);
    // nobody in range: ours and the five nearest still
    r.within = w(1, 1);
    r.run(ON, 3.0);
    assert_eq!(r.live().max_avatars, AVATAR_FLOOR);
    // with frames too slow the lever goes under the range's count
    r.within = w(12, 12);
    r.run(SLOW, 20.0);
    assert_eq!(r.live().max_avatars, AVATAR_FLOOR);
}

#[test]
fn the_distance_mode_does_not_flip_for_someone_at_the_edge() {
    let mut r = Run::new(256.0, 16);
    r.config.by_distance = true;
    r.within = w(8, 8);
    r.run(ON, 7.0);
    assert_eq!(r.live().max_avatars, 8);
    let before = r.changes.len();
    // someone standing at the edge: in and out of the distance at every
    // frame, never well inside it
    for i in 0..1800 {
        r.within = if i % 2 == 0 { w(8, 8) } else { w(9, 8) };
        r.frame(ON, false);
    }
    assert_eq!(r.changes.len(), before, "{:?}", &r.changes[before..]);
    // they walk in: one rise; out past the distance: one drop
    r.within = w(9, 9);
    r.run(ON, 2.0);
    assert_eq!(r.live().max_avatars, 9);
    r.within = w(8, 8);
    r.run(ON, 2.0);
    assert_eq!(r.live().max_avatars, 8);
    assert_eq!(r.changes.len(), before + 2);
    // no count made with the current distance yet: it stays as it is
    r.within = None;
    r.run(ON, 5.0);
    assert_eq!(r.live().max_avatars, 8);
}

#[test]
fn the_target_is_held_under_the_frame_rate_cap() {
    let c = Config {
        target_fps: 120,
        ..Config::default()
    };
    assert_eq!(effective_target(&c, 60.0), 60.0);
    assert_eq!(effective_target(&c, 0.0), 120.0);
    assert_eq!(effective_target(&Config { target_fps: 0, ..c }, 0.0), 1.0);
}

#[test]
fn the_shown_values_are_the_users_while_autotune_holds_them() {
    let mut s = Settings {
        draw_distance: 256.0,
        max_avatars: 16,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    h.write(
        &mut s,
        Tuned {
            draw_distance: 100.0,
            max_avatars: 8,
        },
    );
    assert_eq!(
        h.shown(&s),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
    // a value someone else wrote (the photo quality, a level) is shown as is
    s.draw_distance = 512.0;
    assert_eq!(
        h.shown(&s),
        Tuned {
            draw_distance: 512.0,
            max_avatars: 16
        }
    );
}

#[test]
fn the_photo_quality_and_the_steps_keep_the_users_values() {
    use crate::graphics_presets::Values;
    // tuned, then the photo quality raises everything
    let mut s = Settings {
        draw_distance: 256.0,
        max_avatars: 16,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    h.write(
        &mut s,
        Tuned {
            draw_distance: 100.0,
            max_avatars: 8,
        },
    );
    s.autotune_hold = Some(h);
    s.photo_held = Some(crate::snapshot::quality::boost(&mut s));
    assert_eq!((s.draw_distance, s.max_avatars), (512.0, 100), "boosted");
    // the photo window took the user's values, not the tuned ones
    assert_eq!(s.photo_held.as_ref().map(|p| p.user().max_avatars), Some(16));
    // the hold adopts nothing of the boost, writes nothing under it
    let mut h2 = h;
    h2.sync(&s);
    assert_eq!(
        h2.user(&s),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
    h2.write(
        &mut s,
        Tuned {
            draw_distance: 50.0,
            max_avatars: 2,
        },
    );
    assert_eq!((s.draw_distance, s.max_avatars), (512.0, 100));
    // saved while both hold: the user's
    let saved = s.to_saved();
    assert_eq!((saved.draw_distance, saved.max_avatars), (256.0, 16));
    // turned off under the photo: the photo gives the user's back at its end
    let mut off = s.clone();
    off.autotune_hold.take().expect("held").restore(&mut off);
    assert_eq!((off.draw_distance, off.max_avatars), (512.0, 100));
    let held = off.photo_held.take().expect("photo");
    held.restore(&mut off);
    assert_eq!((off.draw_distance, off.max_avatars), (256.0, 16));
    // the steps: they climb to the tuned distance; off, to the user's
    let mut s = Settings {
        draw_distance: 256.0,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    let max_avatars = s.max_avatars;
    h.write(
        &mut s,
        Tuned {
            draw_distance: 100.0,
            max_avatars,
        },
    );
    s.autotune_hold = Some(h);
    let (steps, value) = crate::settings::draw_stepping::start(None, s.draw_distance, Instant::now());
    s.draw_stepping = Some(steps);
    s.draw_distance = value;
    assert_eq!(s.to_saved().draw_distance, 256.0, "saved during the steps: the user's");
    assert_eq!(Values::read(&s).draw_distance, value);
    s.autotune_hold.take().expect("held").restore(&mut s);
    assert_eq!(s.draw_stepping.map(|st| st.user), Some(256.0));
    assert_eq!(s.to_saved().draw_distance, 256.0);
}

#[test]
fn the_settings_file_holds_the_users_values_while_it_runs() {
    let mut r = Run::new(256.0, 16);
    r.run(SLOW, 60.0);
    r.s.autotune_hold = r.hold;
    let saved = r.s.to_saved();
    assert_eq!((saved.draw_distance, saved.max_avatars), (256.0, 16));
    assert!(saved.autotune_hold.is_none());
    // as the level and a saved profile see them
    let v = crate::graphics_presets::Values::read(&r.s);
    assert_eq!((v.draw_distance, v.max_avatars), (256.0, 16));
    // and the JSON written has no trace of the hold
    let json = serde_json::to_value(&saved).expect("json");
    assert!(json.get("autotune_hold").is_none());
    assert_eq!(json["draw_distance"], 256.0);
}

/// A scene right at the edge: slow with 12 avatars in full or more.
fn edge(r: &Run, slow: f32, fast: f32) -> f32 {
    if r.live().max_avatars >= 12 { slow } else { fast }
}

/// The times of the step ups of a run.
fn ups(r: &Run) -> Vec<Duration> {
    r.changes
        .windows(2)
        .filter(|w| w[1].1.max_avatars > w[0].1.max_avatars)
        .map(|w| w[1].0)
        .collect()
}

#[test]
fn the_wait_comes_back_down_once_the_load_is_gone() {
    let mut r = Run::new(256.0, 16);
    let end = r.now + Duration::from_secs(900);
    while r.now < end {
        let ms = edge(&r, SLOW, FAST);
        r.frame(ms, false);
    }
    assert_eq!(r.tuner.up_wait, MAX_UP_WAIT, "backed off");
    // the load goes away for good: each step up that holds halves the wait
    let from = r.changes.len();
    r.run(FAST, 600.0);
    assert_eq!(
        r.live(),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 16
        }
    );
    let times: Vec<Duration> = r.changes[from..].iter().map(|(t, _)| *t).collect();
    let gaps: Vec<Duration> = times.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.windows(2).all(|g| g[1] <= g[0]), "{gaps:?}");
    assert!(*gaps.last().expect("gaps") < Duration::from_secs(30), "{gaps:?}");
}

#[test]
fn a_bounce_is_seen_at_a_low_target_too() {
    // 3 fps: a measurement takes longer than the bounce window
    let mut r = Run::new(256.0, 16);
    r.config.target_fps = 3;
    let end = r.now + Duration::from_secs(3600);
    while r.now < end {
        let ms = edge(&r, 500.0, 200.0);
        r.frame(ms, false);
    }
    let ups = ups(&r);
    assert!(ups.len() >= 3, "{ups:?}");
    let gaps: Vec<Duration> = ups.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(gaps.windows(2).all(|g| g[1] >= g[0]), "{gaps:?}");
    assert!(*gaps.last().expect("gaps") >= MAX_UP_WAIT, "{gaps:?}");
}

#[test]
fn a_change_of_the_avatars_during_the_steps_is_the_users() {
    let mut s = Settings {
        draw_distance: 256.0,
        max_avatars: 16,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    h.write(
        &mut s,
        Tuned {
            draw_distance: 100.0,
            max_avatars: 8,
        },
    );
    let (steps, value) = crate::settings::draw_stepping::start(None, s.draw_distance, Instant::now());
    s.draw_stepping = Some(steps);
    s.draw_distance = value;
    // the user sets 10 avatars in Preferences during the steps
    s.max_avatars = 10;
    h.sync(&s);
    assert_eq!(
        h.user(&s),
        Tuned {
            draw_distance: 256.0,
            max_avatars: 10
        }
    );
    h.restore(&mut s);
    assert_eq!(s.max_avatars, 10);
    assert_eq!(s.draw_stepping.map(|st| st.user), Some(256.0));
}

#[test]
fn a_level_chosen_while_it_runs_is_the_users_even_at_the_tuned_values() {
    let mut s = Settings {
        draw_distance: 256.0,
        max_avatars: 16,
        ..Settings::default()
    };
    let mut h = Hold::new(&s);
    let tuned = Tuned {
        draw_distance: 64.0,
        max_avatars: 6,
    };
    h.write(&mut s, tuned);
    s.autotune_hold = Some(h);
    // under the photo quality, nothing is taken
    let mut photo = s.clone();
    photo.photo_held = Some(crate::snapshot::quality::boost(&mut photo));
    Hold::chosen(&mut photo);
    assert_eq!(photo.autotune_hold, Some(h));
    // a level whose values are the tuned ones: they are the user's now
    Hold::chosen(&mut s);
    let h = s.autotune_hold.take().expect("held");
    assert_eq!(h.user(&s), tuned);
    h.restore(&mut s);
    assert_eq!(Tuned::read(&s), tuned);
}
