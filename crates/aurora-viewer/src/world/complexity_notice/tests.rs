use super::*;

const SHOWN: Context = Context {
    seconds: 10,
    mouselook: false,
};

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn counts(reporting: u32, over_limit: u32) -> Visibility {
    Visibility { reporting, over_limit }
}

/// Our avatar with its outfit formed.
fn own(complexity: u32, cof_version: i32) -> OwnAvatar {
    OwnAvatar {
        complexity,
        wearables_loaded: true,
        outfit_complete: true,
        cof_version,
        rez_status: 4,
    }
}

fn shown_over_limit(action: Option<Action>) -> Option<OverLimit> {
    match action {
        Some(Action::Show(n)) => n.over_limit,
        _ => None,
    }
}

#[test]
fn the_counts_wait_70_s_after_start() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    // 6 viewers report (ourselves among them), 1 of the 5 others does not show us
    assert_eq!(n.region_answer(counts(6, 1), t0 + secs(30), SHOWN), None);
    // the same counts again: still a change since the last notice (none yet), shown now
    let action = n.region_answer(counts(6, 1), t0 + secs(71), SHOWN);
    assert_eq!(
        action,
        Some(Action::Show(Notice {
            complexity: 0,
            over_limit: Some(OverLimit::NotEveryone),
            seconds: 10
        }))
    );
    // unchanged counts: nothing more
    assert_eq!(n.region_answer(counts(6, 1), t0 + secs(90), SHOWN), None);
}

#[test]
fn the_message_follows_the_share_not_showing_us() {
    for (reporting, over, expected) in [
        (11, 1, OverLimit::EveryoneNow),
        (11, 2, OverLimit::NotEveryone),
        (11, 5, OverLimit::OverHalf),
        (11, 8, OverLimit::Most),
        // everyone over the limit: we are not taken off the count
        (10, 10, OverLimit::Anyone),
    ] {
        let t0 = Instant::now();
        let mut n = Notifier::new(t0);
        let action = n.region_answer(counts(reporting, over), t0 + OVER_LIMIT_UPDATE_DELAY, SHOWN);
        assert_eq!(shown_over_limit(action), Some(expected), "{reporting} / {over}");
    }
}

#[test]
fn nobody_reporting_or_nobody_over_says_nothing() {
    let t0 = Instant::now();
    let t = t0 + secs(100);
    let mut n = Notifier::new(t0);
    assert_eq!(n.region_answer(counts(0, 0), t, SHOWN), None, "data not ready");
    assert_eq!(n.region_answer(counts(6, 0), t, SHOWN), None, "everyone sees us, and always did");
}

#[test]
fn a_new_notice_waits_the_pop_up_delay_unless_the_last_one_is_out() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    let t = t0 + secs(100);
    assert!(n.region_answer(counts(11, 2), t, SHOWN).is_some());
    // still out (10 s): a big change replaces it at once
    assert_eq!(
        shown_over_limit(n.region_answer(counts(11, 8), t + secs(5), SHOWN)),
        Some(OverLimit::Most)
    );
    // gone, within the 300 s: kept back
    assert_eq!(n.region_answer(counts(11, 2), t + secs(60), SHOWN), None);
    // after them
    assert_eq!(
        shown_over_limit(n.region_answer(counts(11, 3), t + secs(5) + POP_UP_DELAY, SHOWN)),
        Some(OverLimit::NotEveryone)
    );
}

#[test]
fn a_small_change_of_the_share_is_not_a_notice() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    let t = t0 + secs(100);
    // 20 of the 40 others: 50 %
    assert!(n.region_answer(counts(41, 20), t, SHOWN).is_some());
    // 20 of 42 (47.6 %): less than a tenth of a change
    assert_eq!(n.region_answer(counts(43, 20), t + secs(400), SHOWN), None);
    // back to zero: "Everyone can see you now"
    assert_eq!(
        shown_over_limit(n.region_answer(counts(43, 0), t + secs(800), SHOWN)),
        Some(OverLimit::EveryoneNow)
    );
}

#[test]
fn no_notice_with_the_setting_at_zero_or_in_mouselook() {
    let t0 = Instant::now();
    let t = t0 + secs(100);
    let off = Context {
        seconds: 0,
        mouselook: false,
    };
    let mut n = Notifier::new(t0);
    assert_eq!(n.region_answer(counts(11, 5), t, off), None);
    let mouselook = Context {
        seconds: 10,
        mouselook: true,
    };
    let mut n = Notifier::new(t0);
    assert_eq!(n.region_answer(counts(11, 5), t, mouselook), None);
    // the situation counts as told: the same counts after mouselook say nothing
    assert_eq!(n.region_answer(counts(11, 5), t + secs(400), SHOWN), None);
}

#[test]
fn the_outfit_we_arrive_with_is_not_announced() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    let loading = OwnAvatar {
        outfit_complete: false,
        rez_status: 2,
        ..own(30_000, 7)
    };
    assert_eq!(n.own_avatar(loading, t0 + secs(5), SHOWN), None);
    assert_eq!(n.own_avatar(own(80_000, 7), t0 + secs(10), SHOWN), None);
    assert_eq!(
        n.own_avatar(own(95_000, 7), t0 + secs(12), SHOWN),
        None,
        "still the outfit of the login"
    );
}

#[test]
fn an_outfit_change_announces_each_new_complexity() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    assert_eq!(n.own_avatar(own(80_000, 7), t0, SHOWN), None);
    // a new outfit version: its complexity is told, each time it changes
    let action = n.own_avatar(own(120_000, 8), t0 + secs(20), SHOWN);
    assert_eq!(
        action,
        Some(Action::Show(Notice {
            complexity: 120_000,
            over_limit: None,
            seconds: 10
        }))
    );
    assert_eq!(n.own_avatar(own(120_000, 8), t0 + secs(22), SHOWN), None, "no change");
    assert!(n.own_avatar(own(150_000, 8), t0 + secs(24), SHOWN).is_some());
    // the counts then wait 70 s, the notice of our complexity is out
    assert_eq!(n.region_answer(counts(11, 5), t0 + secs(60), SHOWN), None);
    let action = n.region_answer(counts(11, 5), t0 + secs(24) + OVER_LIMIT_UPDATE_DELAY, SHOWN);
    assert_eq!(
        action,
        Some(Action::Show(Notice {
            complexity: 150_000,
            over_limit: Some(OverLimit::OverHalf),
            seconds: 10
        }))
    );
}

#[test]
fn a_complexity_moving_on_its_own_shows_once_per_pop_up_delay() {
    // #3381: our complexity moved every refresh, a notice each time
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    assert_eq!(n.own_avatar(own(80_000, 7), t0, SHOWN), None);
    assert!(n.own_avatar(own(120_000, 8), t0 + secs(2), SHOWN).is_some());
    // the outfit forming: the notice still out takes the new values
    assert!(n.own_avatar(own(125_000, 8), t0 + secs(8), SHOWN).is_some());
    assert!(n.own_avatar(own(130_000, 8), t0 + secs(14), SHOWN).is_some());
    // the notice gone (10 s): nothing more until the pop-up delay is over
    let mut t = t0 + secs(26);
    let mut complexity = 130_000;
    while t < t0 + secs(2) + POP_UP_DELAY {
        complexity += 1_000;
        assert_eq!(n.own_avatar(own(complexity, 8), t, SHOWN), None, "{t:?}");
        t += secs(2);
    }
    // then the complexity of then, once
    let action = n.own_avatar(own(complexity, 8), t, SHOWN);
    assert_eq!(
        action,
        Some(Action::Show(Notice {
            complexity,
            over_limit: None,
            seconds: 10
        }))
    );
    assert_eq!(n.own_avatar(own(complexity, 8), t + secs(2), SHOWN), None, "no change");
}

#[test]
fn a_burst_ends_after_its_time_even_with_the_notice_out() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    assert_eq!(n.own_avatar(own(80_000, 7), t0, SHOWN), None);
    let mut t = t0 + secs(2);
    let mut complexity = 80_000;
    while t < t0 + secs(2) + OWN_BURST {
        complexity += 1_000;
        assert!(n.own_avatar(own(complexity, 8), t, SHOWN).is_some(), "{t:?}");
        t += secs(2);
    }
    assert_eq!(n.own_avatar(own(complexity + 1_000, 8), t, SHOWN), None);
}

#[test]
fn a_drop_of_the_rezzed_status_is_an_outfit_change() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    assert_eq!(n.own_avatar(own(80_000, 7), t0, SHOWN), None);
    let rebaking = OwnAvatar {
        rez_status: 1,
        ..own(80_000, 7)
    };
    assert_eq!(n.own_avatar(rebaking, t0 + secs(2), SHOWN), None, "same complexity");
    assert!(n.own_avatar(own(64_000, 7), t0 + secs(4), SHOWN).is_some());
}

#[test]
fn people_not_seeing_us_make_complexity_changes_show() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    assert_eq!(n.own_avatar(own(80_000, 7), t0, SHOWN), None);
    let _ = n.region_answer(counts(11, 5), t0 + secs(80), SHOWN);
    assert!(n.own_avatar(own(90_000, 7), t0 + secs(85), SHOWN).is_some());
}

#[test]
fn a_change_of_kind_takes_the_other_notice_away() {
    let t0 = Instant::now();
    let mut n = Notifier::new(t0);
    let t = t0 + secs(100);
    assert!(n.region_answer(counts(11, 5), t, SHOWN).is_some());
    // our complexity changes in mouselook while the counts' notice is out
    let mouselook = Context {
        seconds: 10,
        mouselook: true,
    };
    assert_eq!(n.own_avatar(own(80_000, 7), t + secs(1), mouselook), Some(Action::Cancel));
}

#[test]
fn the_setting_at_zero_ignores_our_complexity() {
    let t0 = Instant::now();
    let off = Context {
        seconds: 0,
        mouselook: false,
    };
    let mut n = Notifier::new(t0);
    assert_eq!(n.own_avatar(own(80_000, 7), t0, off), None);
    assert_eq!(n.own_avatar(own(150_000, 8), t0 + secs(20), off), None);
}
