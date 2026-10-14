use super::*;
use crate::world::notifications::Kind;
use uuid::Uuid;

fn info(list: &mut Notifications, title: &str) -> u64 {
    list.push(Kind::Info, title, "", Data::None)
}

fn at(now: f64) -> Moment {
    Moment { now, ..Moment::default() }
}

/// A new card unrolls (not counted yet), runs out, folds into the bell and
/// only then raises the count, with a bounce.
#[test]
fn a_card_unrolls_then_folds_into_the_bell() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let id = info(&mut list, "Lune paid you L$50");
    assert!(update(&list, &mut c, at(0.0), 30.0));
    assert_eq!(c.peek.iter().map(|p| p.id).collect::<Vec<_>>(), vec![id]);
    assert_eq!(bell_count(&list, &c), 0);
    // still out a moment before its time
    update(&list, &mut c, at(SHOW_INFO - 0.1), 30.0);
    assert!(c.peek[0].folding.is_none());
    // runs out: folds
    update(&list, &mut c, at(SHOW_INFO + 0.01), 30.0);
    assert!(c.peek[0].folding.is_some());
    // folded in: counted, the bell bounced and rang
    let t = SHOW_INFO + 0.02 + FOLD;
    update(&list, &mut c, at(t), 30.0);
    assert!(c.peek.is_empty());
    assert_eq!(bell_count(&list, &c), 1);
    assert_eq!(c.bump_at, Some(t));
    assert_eq!(c.ring_at, Some(t));
}

/// The region restart's card (show_toast="false") waits under the bell
/// without unrolling, and the bell counts it at once.
#[test]
fn a_quiet_card_never_unrolls() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let id = list.push(Kind::Alert, "Region restart", "", Data::RegionRestart);
    if let Some(n) = list.list.iter_mut().find(|n| n.id == id) {
        n.quiet = true;
    }
    update(&list, &mut c, at(0.0), 30.0);
    assert!(c.peek.is_empty());
    assert_eq!(bell_count(&list, &c), 1);
}

/// Hovering the panel stops the time; an offer waiting for an answer stays
/// longer than information.
#[test]
fn hovering_pauses_and_answers_stay_longer() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    list.push(
        Kind::Teleport,
        "Tess offers a teleport",
        "",
        Data::Lure {
            from: Uuid::from_u128(1),
            invite: 1,
            change_rating: false,
        },
    );
    update(&list, &mut c, at(0.0), 30.0);
    assert_eq!(c.peek[0].total, SHOW_ANSWER);
    let hovered = |now| Moment {
        now,
        hovered: true,
        ..Moment::default()
    };
    update(&list, &mut c, hovered(5.0), 30.0);
    update(&list, &mut c, hovered(60.0), 30.0);
    assert_eq!(c.peek[0].left, SHOW_ANSWER);
    assert!(c.peek[0].folding.is_none());
    update(&list, &mut c, at(61.0), 30.0);
    assert_eq!(c.peek[0].left, SHOW_ANSWER - 1.0);
}

/// Five at once: the newest three are shown, two wait behind "+ 2 more".
#[test]
fn a_burst_shows_three_and_counts_the_rest() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let ids: Vec<u64> = (0..5).map(|i| info(&mut list, &format!("n{i}"))).collect();
    update(&list, &mut c, at(0.0), 30.0);
    let (shown, behind) = visible(&c);
    assert_eq!(shown.iter().map(|p| p.id).collect::<Vec<_>>(), ids[2..].to_vec());
    assert_eq!(behind, 2);
}

/// Do not disturb pauses the bell: nothing unrolls nor rings, the count
/// still grows (with its bounce).
#[test]
fn do_not_disturb_pauses_the_bell() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    info(&mut list, "A group notice");
    let m = Moment {
        now: 1.0,
        dnd: true,
        ..Moment::default()
    };
    update(&list, &mut c, m, 30.0);
    assert!(c.peek.is_empty());
    assert_eq!(bell_count(&list, &c), 1);
    assert_eq!(c.bump_at, Some(1.0));
    assert_eq!(c.ring_at, None);
    // mouselook or the photo's full frame: the same
    let mut c = CenterUi::default();
    let m = Moment {
        now: 1.0,
        hidden: true,
        ..Moment::default()
    };
    update(&list, &mut c, m, 30.0);
    assert!(c.peek.is_empty());
}

/// A friend online is never in the center: no card, no count.
#[test]
fn a_friend_online_never_enters_the_center() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let id = list.push_presence(Uuid::from_u128(2), "Orion is online.", true);
    update(&list, &mut c, at(0.0), 30.0);
    assert!(c.peek.is_empty());
    assert_eq!(bell_count(&list, &c), 0);
    // its own small toast, gone after a few seconds
    assert_eq!(c.presence, vec![(id, 0.0)]);
    update(&list, &mut c, at(PRESENCE + 1.0), 30.0);
    assert!(c.presence.is_empty());
}

/// An answered card folds away at once and is not counted.
#[test]
fn an_answered_card_folds_away() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let id = info(&mut list, "n");
    update(&list, &mut c, at(0.0), 30.0);
    list.list.retain(|n| n.id != id);
    update(&list, &mut c, at(0.5), 30.0);
    assert!(c.peek[0].folding.is_some());
    update(&list, &mut c, at(0.5 + FOLD), 30.0);
    assert!(c.peek.is_empty());
    assert_eq!(bell_count(&list, &c), 0);
}

#[test]
fn days_are_named_today_and_yesterday() {
    let now = 1_780_000_000;
    assert_eq!(day_label(now, now), aurora_i18n::tr!("ui-chat-today"));
    assert_eq!(day_label(now - 86_400, now), aurora_i18n::tr!("ui-chat-yesterday"));
    assert!(day_label(now - 5 * 86_400, now).chars().any(|c| c.is_ascii_digit()));
}

/// A script menu stays rolled out until answered, without time running;
/// it keeps its place among the three shown, the other cards go behind
/// "+ N more" first.
#[test]
fn a_script_menu_stays_out_until_answered() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let menu = list.push(
        Kind::Script,
        "Fountain",
        "Pick a colour",
        Data::Dialog {
            object: Uuid::from_u128(9),
            object_name: "Fountain".into(),
            own_object: false,
            channel: -5,
            buttons: vec!["Violet".into()],
        },
    );
    let others: Vec<u64> = (0..4).map(|i| info(&mut list, &format!("n{i}"))).collect();
    update(&list, &mut c, at(0.0), 30.0);
    let (shown, behind) = visible(&c);
    assert_eq!(shown.iter().map(|p| p.id).collect::<Vec<_>>(), vec![menu, others[2], others[3]]);
    assert_eq!(behind, 2);
    // long after the others folded in, the menu is still out
    update(&list, &mut c, at(SHOW_ANSWER * 3.0), 30.0);
    update(&list, &mut c, at(SHOW_ANSWER * 3.0 + FOLD + 0.1), 30.0);
    assert_eq!(c.peek.iter().map(|p| (p.id, p.folding)).collect::<Vec<_>>(), vec![(menu, None)]);
    // answered: it folds away
    list.list.retain(|n| n.id != menu);
    update(&list, &mut c, at(SHOW_ANSWER * 4.0), 30.0);
    assert!(c.peek[0].folding.is_some());
}
