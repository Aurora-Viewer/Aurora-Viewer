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

#[test]
fn each_switch_combination_selects_its_subjects() {
    let avatar = Subject::Avatar {
        idx: 1,
        id: Uuid::from_u128(1),
    };
    let (object, _) = object(2);
    let land = Subject::Land {
        handle: 0,
        local: Vec3::ZERO,
        origin: (0, 0),
    };
    for enabled in [false, true] {
        for avatars in [false, true] {
            for all_objects in [false, true] {
                for land_on in [false, true] {
                    let settings = HoverTips {
                        enabled,
                        avatars,
                        all_objects,
                        land: land_on,
                        ..HoverTips::default()
                    };
                    let case = format!("enabled={enabled} avatars={avatars} all={all_objects} land={land_on}");
                    // the main switch shows avatars as before; "Avatars even
                    // when off" shows them alone
                    assert_eq!(any_enabled(&settings), enabled || avatars, "{case}");
                    assert_eq!(subject_enabled(avatar, &settings), enabled || avatars, "{case}");
                    assert_eq!(subject_enabled(object, &settings), enabled, "{case}");
                    assert_eq!(subject_enabled(land, &settings), enabled && land_on, "{case}");
                }
            }
        }
    }
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

const VENDOR: u32 = 34_001;
const OTHER: u32 = 34_002;
const OWNER: Uuid = Uuid::from_u128(0x0A11);

/// The demo world with RLVa on: a scripted vendor prim 1 m from the agent
/// and another avatar, with a group title, 2 m away. Returns their subjects
/// and the vendor's properties.
fn rlva_world() -> (World, Subject, Subject, HashMap<Uuid, ObjectProps>) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let handle = world.main_region.expect("demo region");
    let at = Vec3::new(100.0, 120.0, 30.0);
    let mut vendor = crate::demo::action_avatar(false);
    vendor.local_id = VENDOR;
    vendor.full_id = crate::demo::action_id(VENDOR);
    vendor.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    vendor.parent_id = 0;
    vendor.position = at;
    vendor.name_values.clear();
    let mut other = crate::demo::action_avatar(false);
    other.local_id = OTHER;
    other.full_id = crate::demo::action_id(OTHER);
    other.parent_id = 0;
    other.position = at + Vec3::X * 2.0;
    other.name_values = "FirstName STRING RW SV Test\nLastName STRING RW SV Resident\nTitle STRING RW SV Boss".to_owned();
    world.apply(aurora_net::NetEvent::ObjectUpdates {
        handle,
        objects: vec![vendor, other],
    });
    world.agent.position = at + Vec3::Y;
    let vendor = world.objects.index_of_uuid(&crate::demo::action_id(VENDOR)).expect("vendor");
    world.objects.get_mut(vendor).expect("vendor").update_flags = flag::HANDLE_TOUCH | flag::TAKES_MONEY;
    let other = world.objects.index_of_uuid(&crate::demo::action_id(OTHER)).expect("other avatar");
    let props = HashMap::from([(
        crate::demo::action_id(VENDOR),
        ObjectProps {
            object_id: crate::demo::action_id(VENDOR),
            owner_id: OWNER,
            owner_mask: perm::TRANSFER | perm::COPY,
            sale_type: 2,
            sale_price: 10,
            name: "Vendor".to_owned(),
            ..Default::default()
        },
    )]);
    world.rlva.configure(true);
    let object = subject_at(&world, Some(vendor), at).expect("an object subject");
    let avatar = subject_at(&world, Some(other), at + Vec3::X * 2.0).expect("an avatar subject");
    assert!(matches!(object, Subject::Object { .. }) && matches!(avatar, Subject::Avatar { .. }));
    (world, object, avatar, props)
}

fn rlva_restrict(world: &mut World, text: &str) {
    assert!(world.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Test object".to_owned(),
        source_id: Uuid::from_u128(1),
        owner_id: Uuid::from_u128(2),
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        region: None,
        position: Vec3::ZERO,
        message: text.to_owned(),
    }));
}

/// The tip of `subject` hovered at `point`, every switch on.
fn rlva_tip(world: &mut World, subject: Subject, props: &HashMap<Uuid, ObjectProps>, point: Vec3) -> Tip {
    let settings = HoverTips {
        enabled: true,
        all_objects: true,
        group_titles: true,
        ..HoverTips::default()
    };
    let parcels = HoverParcels::default();
    let sources = Sources {
        props,
        parcels: &parcels,
        land_impact: None,
        point,
        agent: world.agent.position,
    };
    build(world, subject, &sources, &settings).expect("a tip")
}

#[test]
fn rlva_leaves_no_tip_on_what_the_user_may_not_interact_with() {
    let (mut world, object, avatar, _) = rlva_world();
    let at = Vec3::new(100.0, 120.0, 30.0);
    let land = Subject::Land {
        handle: world.main_region.expect("demo region"),
        local: at,
        origin: (0, 0),
    };
    for subject in [object, avatar, land] {
        assert!(rlva_allows(&world, subject, at));
    }
    // @fartouch: the hovered point decides, the land keeps its tip
    rlva_restrict(&mut world, "@fartouch:0.5=n");
    assert!(!rlva_allows(&world, object, at));
    assert!(!rlva_allows(&world, avatar, at));
    assert!(rlva_allows(&world, object, world.agent.position + Vec3::X * 0.3));
    assert!(rlva_allows(&world, land, at));
    rlva_restrict(&mut world, "@fartouch:0.5=y");
    // @interact: no tip at all, the land's included
    rlva_restrict(&mut world, "@interact=n");
    for subject in [object, avatar, land] {
        assert!(!rlva_allows(&world, subject, world.agent.position));
    }
    // the preference off: every tip is back
    world.rlva.configure(false);
    for subject in [object, avatar, land] {
        assert!(rlva_allows(&world, subject, at));
    }
}

#[test]
fn rlva_hides_the_tip_of_an_avatar_whose_name_is_hidden() {
    let (mut world, _, avatar, props) = rlva_world();
    let at = Vec3::new(102.0, 120.0, 30.0);
    assert_eq!(rlva_tip(&mut world, avatar, &props, at).kicker.as_deref(), Some("Boss"));
    rlva_restrict(&mut world, "@shownames=n");
    assert!(!rlva_allows(&world, avatar, at));
    // an exception brings the tip back, without the group title
    rlva_restrict(&mut world, &format!("@shownames:{}=add", crate::demo::action_id(OTHER)));
    assert!(rlva_allows(&world, avatar, at));
    let tip = rlva_tip(&mut world, avatar, &props, at);
    assert_eq!(tip.kicker, None);
    assert!(!tip.legacy.contains("Boss"));
}

#[test]
fn rlva_takes_the_refused_buttons_off_an_avatars_tip() {
    let (mut world, _, avatar, props) = rlva_world();
    let at = Vec3::new(102.0, 120.0, 30.0);
    let other = crate::demo::action_id(OTHER);
    assert_eq!(
        rlva_tip(&mut world, avatar, &props, at).actions,
        vec![TipAction::Im(other), TipAction::PayResident(other)]
    );
    rlva_restrict(&mut world, "@pay=n");
    assert_eq!(rlva_tip(&mut world, avatar, &props, at).actions, vec![TipAction::Im(other)]);
    rlva_restrict(&mut world, "@startim=n");
    assert!(rlva_tip(&mut world, avatar, &props, at).actions.is_empty());
}

#[test]
fn rlva_takes_the_refused_buttons_off_an_objects_tip() {
    let (mut world, object, _, props) = rlva_world();
    let at = Vec3::new(100.0, 120.0, 30.0);
    let id = crate::demo::action_id(VENDOR);
    let all = vec![TipAction::Buy(id), TipAction::Pay(id), TipAction::Touch(VENDOR)];
    assert_eq!(rlva_tip(&mut world, object, &props, at).actions, all);
    rlva_restrict(&mut world, "@buy=n");
    assert_eq!(rlva_tip(&mut world, object, &props, at).actions, vec![TipAction::Touch(VENDOR)]);
    rlva_restrict(&mut world, "@buy=y");
    rlva_restrict(&mut world, "@touchall=n");
    assert_eq!(rlva_tip(&mut world, object, &props, at).actions, all[..2].to_vec());
    rlva_restrict(&mut world, "@touchall=y");
    // @fartouch at the hovered point: Touch goes where the point is too far
    rlva_restrict(&mut world, "@fartouch:1.5=n");
    assert_eq!(rlva_tip(&mut world, object, &props, at).actions, all);
    assert_eq!(rlva_tip(&mut world, object, &props, at - Vec3::Y).actions, all[..2].to_vec());
}

#[test]
fn rlva_hides_the_owner_and_the_position_of_an_objects_tip() {
    let (mut world, object, _, props) = rlva_world();
    let at = Vec3::new(100.0, 120.0, 30.0);
    let position = aurora_i18n::tr!("hover-tips-position");
    let has_position = |tip: &Tip| tip.rows.iter().any(|(label, _)| *label == position);
    let free = rlva_tip(&mut world, object, &props, at);
    assert!(has_position(&free));
    // @showloc: no position, in the card as in the Legacy text; the distance stays
    rlva_restrict(&mut world, "@showloc=n");
    let tip = rlva_tip(&mut world, object, &props, at);
    assert!(!has_position(&tip));
    assert!(tip.rows.iter().any(|(label, _)| *label == aurora_i18n::tr!("common-distance")));
    assert!(tip.legacy.lines().count() < free.legacy.lines().count());
    // @shownames: the owner is a stand-in, an exception changes nothing
    let anonym = world.rlva_anonymous(OWNER).display_name.clone();
    rlva_restrict(&mut world, "@shownames=n");
    assert_eq!(rlva_tip(&mut world, object, &props, at).owner, Some((anonym.clone(), false)));
    rlva_restrict(&mut world, &format!("@shownames:{OWNER}=add"));
    assert_eq!(rlva_tip(&mut world, object, &props, at).owner, Some((anonym, false)));
}
