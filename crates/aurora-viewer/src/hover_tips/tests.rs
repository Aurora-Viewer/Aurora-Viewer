use super::*;

const DELAY: Duration = Duration::from_millis(350);

fn object(n: u128) -> (Subject, Key) {
    let id = Uuid::from_u128(n);
    (
        Subject::Object {
            idx: n as usize,
            root: n as usize,
            root_id: id,
        },
        Key::Object(id),
    )
}

struct Run {
    timing: Timing,
    start: Instant,
}

impl Run {
    fn new() -> Self {
        Run {
            timing: Timing::default(),
            start: Instant::now(),
        }
    }

    /// One frame `ms` after the start, the pointer at `at`.
    fn frame(
        &mut self,
        ms: u64,
        at: (f32, f32),
        subject: Option<(Subject, Key)>,
        interrupted: bool,
        tip: Option<Rect>,
        follows: bool,
    ) -> Option<Shown> {
        self.timing.update(Frame {
            subject,
            pointer: Some(Pos2::new(at.0, at.1)),
            interrupted,
            tip_rect: tip,
            follows,
            delay: DELAY,
            now: self.start + Duration::from_millis(ms),
        })
    }
}

/// LLToolTipMgr::show: a tip only once the pointer rested the delay on
/// the same subject; a move restarts the wait.
#[test]
fn a_tip_waits_for_the_pointer_to_rest_on_its_subject() {
    let mut r = Run::new();
    let a = object(1);
    assert!(r.frame(0, (100.0, 100.0), Some(a), false, None, false).is_none());
    assert!(r.frame(200, (101.0, 100.0), Some(a), false, None, false).is_none());
    // the move at 200 ms restarted the wait
    assert!(r.frame(450, (101.0, 100.0), Some(a), false, None, false).is_none());
    let shown = r.frame(600, (101.0, 100.0), Some(a), false, None, false).expect("rested 400 ms");
    assert_eq!(shown.key, a.1);
    assert_eq!(shown.anchor, Pos2::new(101.0, 100.0));
    // it stays where it appeared while the pointer moves on its subject
    let again = r.frame(700, (140.0, 120.0), Some(a), false, None, false).expect("same subject");
    assert_eq!(again.anchor, Pos2::new(101.0, 100.0));
}

/// Another subject hides the tip at once and waits again; nothing under
/// the pointer (the sky) hides it.
#[test]
fn another_subject_or_none_hides_the_tip() {
    let mut r = Run::new();
    let (a, b) = (object(1), object(2));
    r.frame(0, (100.0, 100.0), Some(a), false, None, false);
    assert!(r.frame(400, (100.0, 100.0), Some(a), false, None, false).is_some());
    assert!(r.frame(450, (300.0, 100.0), Some(b), false, None, false).is_none());
    assert_eq!(r.frame(850, (300.0, 100.0), Some(b), false, None, false).map(|s| s.key), Some(b.1));
    assert!(r.frame(900, (500.0, 50.0), None, false, None, false).is_none());
}

/// The card stays while the pointer travels to it and rests on it, even
/// over another subject or a window (LLToolTipMgr's mouse near rect).
#[test]
fn the_pointer_may_travel_to_the_card_and_rest_on_it() {
    let mut r = Run::new();
    let (a, ground) = (object(1), object(9));
    r.frame(0, (100.0, 100.0), Some(a), false, None, false);
    r.frame(400, (100.0, 100.0), Some(a), false, None, false).expect("shown");
    let card = Rect::from_min_size(Pos2::new(116.0, 120.0), egui::vec2(300.0, 200.0));
    // between the pointer and the card, over the ground
    assert!(r.frame(450, (112.0, 115.0), Some(ground), false, Some(card), false).is_some());
    // on the card: the world hover finds nothing there
    assert!(r.frame(500, (250.0, 200.0), None, false, Some(card), false).is_some());
    // a press on its button keeps it for the button
    assert!(r.frame(520, (250.0, 200.0), None, true, Some(card), false).is_some());
    // away from it
    assert!(r.frame(600, (700.0, 500.0), Some(ground), false, Some(card), false).is_none());
}

/// A press, the wheel or a key hides the tip until the pointer leaves the
/// place (LLToolTipView::handleMouseDown, blockToolTips).
#[test]
fn a_press_hides_the_tip_until_the_pointer_leaves() {
    let mut r = Run::new();
    let a = object(1);
    r.frame(0, (100.0, 100.0), Some(a), false, None, false);
    r.frame(400, (100.0, 100.0), Some(a), false, None, false).expect("shown");
    let bubble = Rect::from_min_size(Pos2::new(116.0, 120.0), egui::vec2(200.0, 80.0));
    assert!(r.frame(450, (100.0, 100.0), Some(a), true, Some(bubble), false).is_none());
    // resting where it was: still blocked
    assert!(r.frame(1500, (105.0, 104.0), Some(a), false, None, false).is_none());
    // the pointer left: the wait starts again
    assert!(r.frame(1600, (600.0, 400.0), Some(a), false, None, false).is_none());
    assert!(r.frame(2000, (600.0, 400.0), Some(a), false, None, false).is_some());
}

/// The readout follows the subject at once once it is shown.
#[test]
fn the_readout_follows_the_pointer_without_a_new_delay() {
    let mut r = Run::new();
    let (a, b) = (object(1), object(2));
    r.frame(0, (100.0, 100.0), Some(a), false, None, true);
    assert!(r.frame(200, (101.0, 100.0), Some(a), false, None, true).is_none());
    // moving on the same subject does not restart its wait
    assert!(r.frame(400, (102.0, 100.0), Some(a), false, None, true).is_some());
    assert_eq!(r.frame(420, (300.0, 100.0), Some(b), false, None, true).map(|s| s.key), Some(b.1));
    assert!(r.frame(440, (300.0, 50.0), None, false, None, true).is_none());
}

fn parcel(flags: u32) -> ParcelInfo {
    ParcelInfo {
        flags,
        ..Default::default()
    }
}

/// handleTooltipLand's restrictions, in the About Land order.
#[test]
fn the_land_restrictions_follow_the_legacy_order() {
    let all = pf::CREATE_OBJECTS | pf::ALLOW_TERRAFORM | pf::ALLOW_FLY | pf::ALLOW_OTHER_SCRIPTS;
    assert!(land_flags(&parcel(all)).is_empty());
    assert_eq!(
        land_flags(&parcel(pf::ALLOW_DAMAGE)),
        vec![Flag::NoBuild, Flag::NoTerraform, Flag::Damage, Flag::NoFly, Flag::NoScripts]
    );
    assert_eq!(
        land_flags(&parcel(
            pf::CREATE_GROUP_OBJECTS | pf::ALLOW_GROUP_SCRIPTS | pf::ALLOW_TERRAFORM | pf::ALLOW_FLY
        )),
        vec![Flag::GroupBuild, Flag::GroupScripts]
    );
}

/// The card's chips leave out what its buttons do.
#[test]
fn only_pay_and_touch_have_buttons() {
    assert!(Flag::Pay.has_action() && Flag::Touch.has_action());
    for f in [Flag::Script, Flag::Physics, Flag::Phantom, Flag::NoFly, Flag::Damage] {
        assert!(!f.has_action(), "{f:?}");
    }
}

/// for_sale_selection: a copy is only sold when the owner may copy.
#[test]
fn an_object_is_for_sale_when_it_can_be_delivered() {
    let p = |sale_type, owner_mask| ObjectProps {
        sale_type,
        owner_mask,
        ..Default::default()
    };
    assert!(!for_sale(&p(0, perm::TRANSFER | perm::COPY)));
    assert!(for_sale(&p(2, perm::TRANSFER | perm::COPY)));
    assert!(!for_sale(&p(2, perm::TRANSFER)), "a copy needs copy rights");
    assert!(for_sale(&p(1, perm::TRANSFER)), "an original does not");
    assert!(!for_sale(&p(1, perm::COPY)), "nothing without transfer");
}

/// The avatars' capsule: met from the side, from above, missed beside it,
/// above its head and behind the camera.
#[test]
fn a_ray_meets_an_avatar_capsule() {
    use crate::scene::ray_capsule;
    let c = Vec3::new(10.0, 0.0, 30.0);
    let hit = |o: Vec3, d: Vec3| ray_capsule(o, d.normalize(), c, 0.35, -1.25, 1.15);
    let t = hit(Vec3::new(0.0, 0.0, 30.5), Vec3::X).expect("from the side");
    assert!((t - 9.65).abs() < 1e-4, "{t}");
    let t = hit(Vec3::new(10.0, 0.0, 40.0), -Vec3::Z).expect("from above");
    assert!((t - 8.85).abs() < 1e-4, "{t}");
    assert!(hit(Vec3::new(0.0, 1.0, 30.5), Vec3::X).is_none(), "beside it");
    assert!(hit(Vec3::new(0.0, 0.0, 32.0), Vec3::X).is_none(), "above the head");
    assert!(hit(Vec3::new(20.0, 0.0, 30.5), Vec3::X).is_none(), "behind the ray");
}

/// setHoverParcel: one request per cell until the parcel answers, none
/// once the point lies on a parcel known.
#[test]
fn the_hovered_parcel_is_asked_once_and_kept() {
    let mut parcels = HoverParcels::default();
    let (handle, origin) = (7, (256_000, 256_000));
    let now = Instant::now();
    let at = Vec3::new(10.0, 10.0, 22.0);
    let Some(aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::ParcelRequest {
        sequence,
        snap,
        west,
        south,
        east,
        north,
        ..
    })) = parcels.request(handle, at, origin, now)
    else {
        panic!("a ParcelPropertiesRequest");
    };
    assert_eq!(sequence, aurora_net::land::HOVERED_PARCEL_SEQ_ID);
    assert!(!snap);
    assert_eq!((west, south, east, north), (8.0, 8.0, 12.0, 12.0));
    assert!(parcels.request(handle, at, origin, now).is_none(), "not twice");
    // a parcel of the 16 first cells of the region's first row of cells
    let mut bitmap = vec![0u8; 64 * 64 / 8];
    for x in 0..16 {
        for y in 0..16 {
            let i = y * 64 + x;
            bitmap[i / 8] |= 1 << (i % 8);
        }
    }
    parcels.on_parcel(
        handle,
        Arc::new(ParcelInfo {
            local_id: 3,
            bitmap,
            ..Default::default()
        }),
    );
    assert_eq!(parcels.at(handle, at, origin).map(|p| p.local_id), Some(3));
    assert!(parcels.request(handle, Vec3::new(60.0, 40.0, 0.0), origin, now).is_none());
    assert!(
        parcels.request(handle, Vec3::new(70.0, 40.0, 0.0), origin, now).is_some(),
        "another parcel"
    );
}
