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
    assert_eq!(
        c.presence,
        vec![presence::Toast {
            id,
            arrived: 0.0,
            shown: Some(0.0)
        }]
    );
    update(&list, &mut c, at(PRESENCE + 1.0), 30.0);
    assert!(c.presence.is_empty());
}

/// At the top right a friend's toast waits while a card is rolled out
/// under the bell, and shows once it has folded in, if it waited less than
/// `PRESENCE`; the other places do not wait.
#[test]
fn at_the_top_right_a_friend_waits_for_the_cards_to_fold() {
    let under_bell = |now| Moment {
        now,
        under_bell: true,
        ..Moment::default()
    };
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    info(&mut list, "Lune paid you L$50");
    update(&list, &mut c, under_bell(0.0), 30.0);
    let late = list.push_presence(Uuid::from_u128(2), "Orion is online.", true);
    update(&list, &mut c, under_bell(2.0), 30.0);
    let early = list.push_presence(Uuid::from_u128(3), "Lune is online.", true);
    update(&list, &mut c, under_bell(5.0), 30.0);
    assert_eq!(c.presence.len(), 2);
    assert!(c.presence.iter().all(|t| t.shown.is_none()), "{:?}", c.presence);
    // the first one waited too long: dropped before the card folds
    update(&list, &mut c, under_bell(2.0 + PRESENCE + 0.1), 30.0);
    assert_eq!(c.presence.iter().map(|t| t.id).collect::<Vec<_>>(), vec![early]);
    // the card folds in: the other one shows
    update(&list, &mut c, under_bell(SHOW_INFO + 0.01), 30.0);
    assert!(c.presence[0].shown.is_none(), "still folding");
    let free = SHOW_INFO + 0.01 + FOLD + 0.01;
    update(&list, &mut c, under_bell(free), 30.0);
    assert!(c.peek.is_empty());
    assert_eq!(c.presence[0].shown, Some(free));
    assert!(!c.presence.iter().any(|t| t.id == late));
    // never in the bell's list nor its count
    assert_eq!(bell_count(&list, &c), 1);
    // elsewhere it shows at once, the card out or not
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    info(&mut list, "Lune paid you L$50");
    list.push_presence(Uuid::from_u128(2), "Orion is online.", true);
    update(&list, &mut c, at(0.0), 30.0);
    assert_eq!(c.peek.len(), 1);
    assert_eq!(c.presence[0].shown, Some(0.0));
}

/// At the top right a friend's toast waits while the bell's list is open,
/// shows when it closes within `PRESENCE`, and is dropped after that; one
/// shown when the list opens leaves.
#[test]
fn at_the_top_right_a_friend_waits_for_the_list_to_close() {
    let m = |now, open| Moment {
        now,
        open,
        under_bell: true,
        ..Moment::default()
    };
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let shown = list.push_presence(Uuid::from_u128(2), "Orion is online.", true);
    update(&list, &mut c, m(0.0, false), 30.0);
    assert_eq!(c.presence[0].shown, Some(0.0));
    update(&list, &mut c, m(0.5, true), 30.0);
    assert!(!c.presence.iter().any(|t| t.id == shown), "under the open list");
    let id = list.push_presence(Uuid::from_u128(3), "Lune is online.", true);
    update(&list, &mut c, m(1.0, true), 30.0);
    update(&list, &mut c, m(2.0, false), 30.0);
    assert_eq!(
        c.presence.iter().map(|t| (t.id, t.shown)).collect::<Vec<_>>(),
        vec![(id, Some(2.0))]
    );
    let id = list.push_presence(Uuid::from_u128(4), "Tess is online.", true);
    update(&list, &mut c, m(10.0, true), 30.0);
    update(&list, &mut c, m(10.0 + PRESENCE + 0.1, false), 30.0);
    assert!(!c.presence.iter().any(|t| t.id == id), "waited too long");
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
            sender: ([127, 0, 0, 1], 13000).into(),
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

/// In do not disturb a script menu still rolls out and stays (#491), the
/// other cards wait in the bell.
#[test]
fn do_not_disturb_rolls_out_only_script_menus() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let early = info(&mut list, "before");
    update(&list, &mut c, at(0.0), 30.0);
    assert_eq!(c.peek.iter().map(|p| p.id).collect::<Vec<_>>(), vec![early]);
    info(&mut list, "Lune paid you L$50");
    let menu = list.push(
        Kind::Script,
        "Fountain",
        "Pick a colour",
        Data::Dialog {
            sender: ([127, 0, 0, 1], 13000).into(),
            object: Uuid::from_u128(9),
            object_name: "Fountain".into(),
            own_object: false,
            channel: -5,
            buttons: vec!["Violet".into()],
        },
    );
    update(&list, &mut c, Moment { dnd: true, ..at(0.1) }, 30.0);
    assert_eq!(c.peek.iter().map(|p| p.id).collect::<Vec<_>>(), vec![menu]);
}

/// "Mark all read" empties the bell's count: the information leaves, a
/// script menu and an offer stay in "To do", read.
#[test]
fn mark_all_read_clears_the_count_and_keeps_what_waits() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    info(&mut list, "Lune paid you L$50");
    let menu = list.push(
        Kind::Script,
        "Fountain",
        "Pick a colour",
        Data::Dialog {
            sender: ([127, 0, 0, 1], 13000).into(),
            object: Uuid::from_u128(9),
            object_name: "Fountain".into(),
            own_object: false,
            channel: -5,
            buttons: vec!["Violet".into()],
        },
    );
    let offer = list.push(
        Kind::Group,
        "Join Aurora Builders?",
        "",
        Data::Group {
            group: Uuid::from_u128(7),
            tx: Uuid::from_u128(8),
            offline: false,
        },
    );
    update(&list, &mut c, at(0.0), 30.0);
    // the list is open: no card out, everything counted
    update(&list, &mut c, Moment { open: true, ..at(0.1) }, 30.0);
    assert_eq!(bell_count(&list, &c), 3);
    list.read_all();
    assert_eq!(list.listed().map(|n| n.id).collect::<Vec<_>>(), vec![menu, offer]);
    assert!(list.list.iter().all(|n| n.read));
    assert_eq!(bell_count(&list, &c), 0);
}

/// The windows a scripted object sends besides llDialog: llTextBox,
/// llRequestPermissions, llLoadURL and an object's llGiveInventory.
pub(crate) fn scripted_windows(list: &mut Notifications) -> Vec<u64> {
    let object = Uuid::from_u128(9);
    vec![
        list.push(
            Kind::Script,
            "Sign",
            "Your name?",
            Data::TextBox {
                sender: ([127, 0, 0, 1], 13000).into(),
                object,
                channel: -6,
            },
        ),
        list.push(
            Kind::Permissions,
            "Dance ball",
            "Animate your avatar",
            Data::Permissions {
                task: object,
                item: Uuid::from_u128(10),
                questions: 0x10,
                region: 1,
                experience: None,
            },
        ),
        list.push(Kind::Url, "Kiosk", "Our shop", Data::Url("https://example.invalid/".into())),
        list.push(
            Kind::Inventory,
            "Gift dispenser",
            "A notecard",
            Data::Inventory {
                from: Uuid::from_u128(11),
                tx: Uuid::from_u128(12),
                asset_type: 7,
                object: Uuid::nil(),
                task: true,
                task_offer: None,
            },
        ),
    ]
}

/// Every window of a scripted object stays rolled out until answered, not
/// only llDialog: no time runs, the information arriving after it does not
/// push it away, and it is back once the list closes, the view clears or
/// do not disturb ends, even read in the list meanwhile.
#[test]
fn every_scripted_window_stays_out_until_answered() {
    let mut list = Notifications::default();
    let mut c = CenterUi::default();
    let windows = scripted_windows(&mut list);
    for i in 0..4 {
        info(&mut list, &format!("n{i}"));
    }
    update(&list, &mut c, at(0.0), 30.0);
    let pinned = |c: &CenterUi| {
        c.peek
            .iter()
            .filter(|p| p.pinned && p.folding.is_none())
            .map(|p| p.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(pinned(&c), windows);
    // the newest three shown, the fourth waits behind "+ N more", the
    // information after them too
    let (shown, _) = visible(&c);
    assert_eq!(shown.iter().map(|p| p.id).collect::<Vec<_>>(), windows[1..].to_vec());
    // long after: still out
    update(&list, &mut c, at(SHOW_ANSWER * 5.0), 30.0);
    update(&list, &mut c, at(SHOW_ANSWER * 5.0 + FOLD + 0.1), 30.0);
    assert_eq!(pinned(&c), windows);
    assert_eq!(c.peek.len(), windows.len());
    // the list opened (everything read), then closed: back out
    update(&list, &mut c, Moment { open: true, ..at(100.0) }, 30.0);
    assert!(c.peek.is_empty());
    list.read_all();
    update(&list, &mut c, at(100.1), 30.0);
    assert_eq!(pinned(&c), windows);
    // mouselook, then back
    update(&list, &mut c, Moment { hidden: true, ..at(101.0) }, 30.0);
    assert!(c.peek.is_empty());
    update(&list, &mut c, at(101.1), 30.0);
    assert_eq!(pinned(&c), windows);
    // do not disturb keeps all but the text box in, until it ends
    update(&list, &mut c, Moment { dnd: true, ..at(102.0) }, 30.0);
    assert_eq!(pinned(&c), vec![windows[0]]);
    update(&list, &mut c, at(102.1), 30.0);
    assert_eq!(pinned(&c), windows);
    // answered: it folds away and stays away
    list.list.retain(|n| n.id != windows[1]);
    update(&list, &mut c, at(103.0), 30.0);
    update(&list, &mut c, at(103.0 + FOLD + 0.1), 30.0);
    assert_eq!(pinned(&c), vec![windows[0], windows[2], windows[3]]);
}

/// A scripted window that arrives while the list is open or in mouselook
/// comes out once they end.
#[test]
fn a_scripted_window_kept_in_comes_out_afterwards() {
    for kept in [Moment { open: true, ..at(0.0) }, Moment { hidden: true, ..at(0.0) }] {
        let mut list = Notifications::default();
        let mut c = CenterUi::default();
        let windows = scripted_windows(&mut list);
        update(&list, &mut c, kept, 30.0);
        assert!(c.peek.is_empty());
        update(&list, &mut c, at(0.1), 30.0);
        assert_eq!(c.peek.iter().map(|p| p.id).collect::<Vec<_>>(), windows);
    }
}
