//! Phase 1: what can be checked without acting in-world. A: each rule is
//! issued through the real controller and the answer is read from the same
//! predicates the interface and the send boundary use, nothing being sent.
//! B: the replies to queries, read as they leave on the asked channel.
//! C: round trips through the simulator with the test's own worn objects.
use super::super::rlva::{LAYERS, POINTS};
use super::{Case, Cmd, Ctx, It, LINKED, Make, POINT_A, POINT_B, POINT_CONTROLLER, SUBFOLDERS, Skip, Step, Test, Who, World};
use crate::world::bridge::factory::attach_request;
use crate::world::names::NameScope;
use aurora_net::NetCommand;
use aurora_net::build::BuildCmd;
use std::time::Duration;
use uuid::Uuid;

pub(super) fn test(f: impl Fn(&World, &mut Ctx) -> Result<(), String> + 'static) -> Test {
    Box::new(f)
}

/// A test that holds when `f` does; `observed` is what a failure reports.
pub(super) fn holds(f: impl Fn(&World, &Ctx) -> bool + 'static, observed: &'static str) -> Test {
    Box::new(move |w, c| if f(w, c) { Ok(()) } else { Err(observed.to_owned()) })
}

pub(super) fn make(f: impl Fn(&World, &Ctx) -> Cmd + 'static) -> Make {
    Box::new(f)
}

pub(super) fn skip_if(f: impl Fn(&World, &Ctx) -> Option<String> + 'static) -> Step {
    Step::SkipIf(Box::new(f) as Skip)
}

/// The command as the user would send it.
pub(super) fn can(w: &World, command: &NetCommand) -> bool {
    w.rlva_allows_command(command, Uuid::nil())
}

/// A clothing layer the avatar wears, with its RLV name.
pub(super) fn worn_layer(w: &World) -> Option<(Uuid, &'static str)> {
    crate::world::appearance::worn_clothing(&w.inventory)
        .into_iter()
        .find_map(|(item, kind)| LAYERS.get(usize::from(kind)).map(|name| (item, *name)))
}

fn point_name(w: &World, point: u8) -> &str {
    w.avatar_lib
        .attach_names
        .get(&point)
        .map_or(POINTS[usize::from(point)], String::as_str)
}

/// Wears a test object, if it is not worn, and waits for it.
pub(super) fn wear(it: It, point: u8) -> [Step; 2] {
    [
        Step::Sent(
            "the test object is asked for",
            make(move |w, c| {
                if c.worn(w, it) {
                    return Cmd::Nothing;
                }
                match w.inventory.items.get(&c.item(it)) {
                    Some(item) => Cmd::send(NetCommand::RezAttachments(vec![attach_request(item, c.agent, point)])),
                    None => Cmd::Missing("the test object"),
                }
            }),
        ),
        Step::Expect("the test object is worn", holds(move |w, c| c.worn(w, it), "it did not arrive")),
    ]
}

/// Takes test objects off, those that are worn, and waits for them to go.
pub(super) fn take_off(its: &'static [It]) -> [Step; 2] {
    [
        Step::Sent(
            "the test objects are taken off",
            make(move |w, c| {
                let worn: Vec<Uuid> = its.iter().filter(|it| c.worn(w, **it)).map(|it| c.item(*it)).collect();
                if worn.is_empty() {
                    Cmd::Nothing
                } else {
                    Cmd::send(NetCommand::DetachAttachments(worn))
                }
            }),
        ),
        Step::Expect(
            "the test objects are off",
            holds(move |w, c| its.iter().all(|it| !c.worn(w, *it)), "still worn"),
        ),
    ]
}

/// Something that is not the test's sits on `point`.
fn foreign_on(w: &World, c: &Ctx, point: u8) -> bool {
    point != 0
        && w.worn_attachment_points()
            .iter()
            .any(|(item, at)| *at == point && !c.own_item(item))
}

/// A prim the placement tool would create; a probe of the rule, never sent
/// from phase 1.
fn probe_add(w: &World) -> NetCommand {
    NetCommand::Build(BuildCmd::Add(Box::new(super::cases_world::new_prim(w, w.agent.position))))
}

fn controller_key(w: &World, c: &Ctx) -> Option<(aurora_net::RegionHandle, u32)> {
    w.objects
        .index_of_uuid(&c.controller)
        .and_then(|i| w.objects.get(i))
        .map(|o| (o.key.region, o.key.local_id))
}

/// One row of the table of rules: `allowed` is the interface's own answer.
struct Rule {
    id: &'static str,
    title: &'static str,
    set: &'static str,
    clear: &'static str,
    /// What the rule refuses, as the report words it.
    refuses: &'static str,
    allowed: fn(&World, &Ctx) -> bool,
    /// Why the row cannot run here.
    needs: Option<fn(&World, &Ctx) -> Option<String>>,
}

/// `@fly=n` lands a flying avatar: phase 1 never issues it in flight.
fn grounded(w: &World, _: &Ctx) -> Option<String> {
    w.agent.flying.then(|| "the avatar is flying: @fly=n would land it".to_owned())
}

/// A HUD the avatar wears, whichever comes first.
fn worn_hud(w: &World) -> Option<usize> {
    w.objects.iter().map(|(i, _)| i).find(|i| w.hud_root(*i) == Some(*i))
}

fn wears_hud(w: &World, _: &Ctx) -> Option<String> {
    worn_hud(w).is_none().then(|| "the avatar wears no HUD".to_owned())
}

fn clothed(w: &World, _: &Ctx) -> Option<String> {
    worn_layer(w).is_none().then(|| "the avatar wears no clothing layer".to_owned())
}

const fn rule(
    id: &'static str,
    title: &'static str,
    set: &'static str,
    clear: &'static str,
    refuses: &'static str,
    allowed: fn(&World, &Ctx) -> bool,
) -> Rule {
    Rule {
        id,
        title,
        set,
        clear,
        refuses,
        allowed,
        needs: None,
    }
}

const RULES: &[Rule] = &[
    rule(
        "A01",
        "@rez",
        "@rez=n",
        "@rez=y",
        "creating and rezzing an object refused",
        |w, _| w.rlva_can_rez(None) && can(w, &probe_add(w)),
    ),
    rule(
        "A02",
        "@interact",
        "@interact=n",
        "@interact=y",
        "creating an object refused",
        |w, _| can(w, &probe_add(w)),
    ),
    // LLViewerWindow::cursorIntersect: a HUD's pick is dropped like the
    // world's, so its click and its menu go too
    Rule {
        needs: Some(wears_hud),
        ..rule(
            "A02b",
            "@interact on a HUD",
            "@interact=n",
            "@interact=y",
            "the click and the menu of a worn HUD refused",
            |w, _| worn_hud(w).is_some_and(|i| w.rlva_can_interact_at(i, None) && w.rlva_can_designate(i, None)),
        )
    },
    rule(
        "A03",
        "@edit with @rez",
        "@edit=n,rez=n",
        "@edit=y,rez=y",
        "the build tools closed",
        |w, _| w.rlva_can_build(),
    ),
    Rule {
        needs: Some(grounded),
        ..rule("A04", "@fly", "@fly=n", "@fly=y", "flying refused", |w, _| !w.rlva.has("fly"))
    },
    rule("A05", "@sit", "@sit=n", "@sit=y", "a sit request refused", |w, c| {
        let (handle, _) = controller_key(w, c).unwrap_or_default();
        can(
            w,
            &NetCommand::RequestSit {
                handle,
                target: c.controller,
                offset: glam::Vec3::ZERO,
            },
        )
    }),
    rule("A06", "@unsit", "@unsit=n", "@unsit=y", "standing up refused", |w, _| {
        can(w, &NetCommand::OneShotControl(aurora_net::control::STAND_UP))
    }),
    rule("A07", "@showinv", "@showinv=n", "@showinv=y", "the inventory hidden", |w, _| {
        !w.rlva.has("showinv")
    }),
    rule(
        "A08",
        "@showminimap",
        "@showminimap=n",
        "@showminimap=y",
        "the minimap hidden",
        |w, _| !w.rlva.has("showminimap"),
    ),
    rule(
        "A09",
        "@showworldmap",
        "@showworldmap=n",
        "@showworldmap=y",
        "the world map hidden",
        |w, _| !w.rlva.has("showworldmap"),
    ),
    rule("A10", "@showloc", "@showloc=n", "@showloc=y", "the location hidden", |w, _| {
        !w.rlva.has("showloc")
    }),
    rule(
        "A11",
        "@shownames",
        "@shownames=n",
        "@shownames=y",
        "another resident's name replaced",
        |w, c| w.rlva_can_name(c.other),
    ),
    rule(
        "A12",
        "@shownametags",
        "@shownametags=n",
        "@shownametags=y",
        "name tags hidden",
        |w, c| w.rlva.nametag_visible(c.other, c.agent, 5.0),
    ),
    rule("A14", "@buy", "@buy=n", "@buy=y", "buying refused", |w, _| !w.rlva.has("buy")),
    rule("A15", "@pay", "@pay=n", "@pay=y", "paying refused", |w, _| !w.rlva.has("pay")),
    rule("A16", "@setenv", "@setenv=n", "@setenv=y", "the environment locked", |w, _| {
        !w.rlva.has("setenv")
    }),
    rule(
        "A17",
        "@setgroup",
        "@setgroup=n",
        "@setgroup=y",
        "leaving the active group refused",
        |w, _| can(w, &NetCommand::LeaveGroup(w.groups.active)),
    ),
    rule(
        "A18",
        "@viewnote",
        "@viewnote=n",
        "@viewnote=y",
        "opening a notecard refused",
        |w, _| {
            !w.rlva.has("viewnote")
                && can(
                    w,
                    &NetCommand::PreviewInventoryItem(aurora_net::inventory::InvItem {
                        asset_type: 7,
                        inv_type: 7,
                        ..Default::default()
                    }),
                )
        },
    ),
    rule(
        "A19",
        "@viewscript",
        "@viewscript=n",
        "@viewscript=y",
        "opening a script refused",
        |w, _| {
            !w.rlva.has("viewscript")
                && can(
                    w,
                    &NetCommand::PreviewInventoryItem(aurora_net::inventory::InvItem {
                        asset_type: 10,
                        inv_type: 10,
                        ..Default::default()
                    }),
                )
        },
    ),
    rule(
        "A20",
        "@fartouch",
        "@fartouch:0.5=n",
        "@fartouch:0.5=y",
        "the selection distance limited",
        |w, _| w.rlva_select_limit().is_none(),
    ),
    rule(
        "A22",
        "@addattach",
        "@addattach=n",
        "@addattach=y",
        "wearing an object refused",
        |w, c| w.rlva_can_add(c.item(It::B), POINT_B, Uuid::nil()) && w.rlva_wear_locks().wear(&w.inventory, c.item(It::B), POINT_B).add,
    ),
    rule(
        "A23",
        "@remattach",
        "@remattach=n",
        "@remattach=y",
        "taking a worn object off refused",
        |w, c| w.rlva_can_remove(c.item(It::A), Uuid::nil()) && w.rlva_wear_locks().can_remove(c.item(It::A)),
    ),
    rule(
        "A24",
        "@addattach on one point",
        "@addattach:tail tip=n",
        "@addattach:tail tip=y",
        "wearing on that point refused",
        |w, c| w.rlva_can_add(c.item(It::B), POINT_B, Uuid::nil()),
    ),
    rule(
        "A25",
        "@remattach on one point",
        "@remattach:hind left foot=n",
        "@remattach:hind left foot=y",
        "taking off what that point holds refused",
        |w, c| w.rlva_can_remove(c.item(It::A), Uuid::nil()),
    ),
    Rule {
        needs: Some(clothed),
        ..rule(
            "A26",
            "@addoutfit",
            "@addoutfit=n",
            "@addoutfit=y",
            "wearing clothing refused",
            |w, _| worn_layer(w).is_some_and(|(item, _)| w.rlva_can_add(item, 0, Uuid::nil())),
        )
    },
    Rule {
        needs: Some(clothed),
        ..rule(
            "A27",
            "@remoutfit",
            "@remoutfit=n",
            "@remoutfit=y",
            "taking clothing off refused",
            |w, _| worn_layer(w).is_some_and(|(item, _)| w.rlva_can_remove(item, Uuid::nil())),
        )
    },
    Rule {
        needs: Some(clothed),
        ..rule(
            "A28",
            "@addoutfit on one layer",
            "@addoutfit:{layer}=n",
            "@addoutfit:{layer}=y",
            "wearing on that layer refused",
            |w, _| worn_layer(w).is_some_and(|(item, _)| w.rlva_can_add(item, 0, Uuid::nil())),
        )
    },
    Rule {
        needs: Some(clothed),
        ..rule(
            "A29",
            "@remoutfit on one layer",
            "@remoutfit:{layer}=n",
            "@remoutfit:{layer}=y",
            "taking that layer off refused",
            |w, _| worn_layer(w).is_some_and(|(item, _)| w.rlva_can_remove(item, Uuid::nil())),
        )
    },
    // RlvActions::canChangeToMouselook: what the camera asks before it
    // enters mouselook (#3357)
    rule(
        "A33",
        "@setcam_mouselook",
        "@setcam_mouselook=n",
        "@setcam_mouselook=y",
        "mouselook refused",
        |w, _| w.rlva.can_mouselook(),
    ),
    rule(
        "A34",
        "@camdistmin above zero",
        "@camdistmin:1=n",
        "@camdistmin:1=y",
        "mouselook refused",
        |w, _| w.rlva.can_mouselook(),
    ),
];

fn case(id: &'static str, title: &'static str, steps: Vec<Step>) -> Case {
    Case {
        id,
        title,
        phase: 1,
        gate: false,
        known: None,
        steps,
    }
}

fn rule_case(rule: &'static Rule) -> Case {
    let allowed = rule.allowed;
    let mut steps = Vec::new();
    if let Some(needs) = rule.needs {
        steps.push(skip_if(needs));
    }
    steps.extend([
        // (the avatar's own restraints: not a failure of the viewer)
        skip_if(move |w, c| {
            (!allowed(w, c) && c.foreign_rules(w)).then(|| "already refused by a restriction of another object".to_owned())
        }),
        Step::Now("allowed before the rule", holds(allowed, "already refused")),
        Step::Say(Who::Controller, rule.set),
        Step::Expect(rule.refuses, holds(move |w, c| !allowed(w, c), "still allowed")),
        Step::Say(Who::Controller, rule.clear),
        Step::Expect("allowed again once released", holds(allowed, "still refused")),
    ]);
    case(rule.id, rule.title, steps)
}

/// The last reply on the test's channel.
fn reply(c: &Ctx) -> Result<&str, String> {
    c.replies.last().map(String::as_str).ok_or_else(|| "no reply".to_owned())
}

fn answers(f: impl Fn(&World, &Ctx, &str) -> bool + 'static) -> Test {
    test(move |w, c| {
        let text = reply(c)?;
        if f(w, c, text) { Ok(()) } else { Err(format!("\"{text}\"")) }
    })
}

/// A query: said, then its reply checked.
fn query(id: &'static str, title: &'static str, text: &'static str, expected: &'static str, check: Test) -> Case {
    case(id, title, vec![Step::Say(Who::Controller, text), Step::Expect(expected, check)])
}

fn rules() -> Vec<Case> {
    let mut cases: Vec<Case> = RULES.iter().map(rule_case).collect();
    cases.push(case(
        "A13",
        "@shownearby is not @shownametags",
        vec![
            Step::Say(Who::Controller, "@shownearby=n"),
            Step::Expect("the nearby list hidden", holds(|w, _| w.rlva.has("shownearby"), "not hidden")),
            Step::Now(
                "name tags still shown",
                holds(|w, c| w.rlva.nametag_visible(c.other, c.agent, 5.0), "name tags hidden too"),
            ),
            Step::Say(Who::Controller, "@shownearby=y"),
            Step::Expect("the nearby list back", holds(|w, _| !w.rlva.has("shownearby"), "still hidden")),
        ],
    ));
    let touch = |w: &World, c: &Ctx| controller_key(w, c).is_some_and(|(handle, local)| w.rlva_can_touch(handle, local));
    cases.push(case(
        "A21",
        "@touchall with @touchme",
        vec![
            Step::Now("the controller can be touched", holds(touch, "already refused")),
            Step::Say(Who::Controller, "@touchall=n"),
            Step::Expect("touching refused", holds(move |w, c| !touch(w, c), "still allowed")),
            Step::Say(Who::Controller, "@touchme=add"),
            Step::Expect("the object that asked can be touched", holds(touch, "still refused")),
            Step::Say(Who::Controller, "@touchme=rem"),
            Step::Expect("touching refused again", holds(move |w, c| !touch(w, c), "still allowed")),
            Step::Say(Who::Controller, "@touchall=y"),
            Step::Expect("allowed again once released", holds(touch, "still refused")),
        ],
    ));
    cases.push(case(
        "A30",
        "point locks leave the other points",
        vec![
            Step::Say(Who::Controller, "@addattach:tail tip=n,remattach:hind left foot=n"),
            Step::Expect(
                "the locked points refuse",
                holds(
                    |w, c| !w.rlva_can_add(c.item(It::B), POINT_B, Uuid::nil()) && !w.rlva_can_remove(c.item(It::A), Uuid::nil()),
                    "still allowed",
                ),
            ),
            Step::Now(
                "another point takes the object, another object comes off",
                holds(
                    |w, c| w.rlva_can_add(c.item(It::B), 43, Uuid::nil()) && w.rlva_can_remove(c.item(It::Controller), Uuid::nil()),
                    "the other point is locked too",
                ),
            ),
        ],
    ));
    // the made-up resident is nobody's friend and stands nowhere: the rule
    // of each place is asked, no window is opened
    let named = |scope: fn(&Ctx) -> NameScope| move |w: &World, c: &Ctx| w.rlva_shows_name(scope(c), c.other);
    cases.push(case(
        "A31",
        "@shownames hides by place, and an exception names again (#3547)",
        vec![
            skip_if(|w, c| {
                (!w.rlva_can_name(c.other) && c.foreign_rules(w)).then(|| "already refused by a restriction of another object".to_owned())
            }),
            Step::Say(Who::Controller, "@shownames=n"),
            Step::Expect(
                "hidden in the world and as an object's owner",
                holds(
                    |w, c| !w.rlva_shows_name(NameScope::World, c.other) && !w.rlva_shows_name(NameScope::Owner, c.other),
                    "still named",
                ),
            ),
            Step::Now(
                "named in the friends list and a group's members",
                holds(named(|_| NameScope::Roster), "hidden there too"),
            ),
            Step::Now(
                "the notice of a message or of a teleport offer names its resident, an inventory offer's does not",
                holds(
                    |w, c| {
                        use crate::world::notifications::Kind;
                        w.rlva_shows_name(Kind::Message.names(), c.other)
                            && w.rlva_shows_name(Kind::Teleport.names(), c.other)
                            && !w.rlva_shows_name(Kind::Inventory.names(), c.other)
                    },
                    "the notices do not follow their kind",
                ),
            ),
            Step::Now(
                "a creator who owns the object is hidden",
                holds(|w, c| !w.rlva_shows_name(NameScope::Creator { owner: c.other }, c.other), "named"),
            ),
            Step::Now(
                "a creator who neither owns it nor is around is named",
                holds(named(|c| NameScope::Creator { owner: c.agent }), "hidden"),
            ),
            Step::Now(
                "oneself is named everywhere",
                holds(
                    |w, c| w.rlva_shows_name(NameScope::World, c.agent) && w.rlva_shows_name(NameScope::Owner, c.agent),
                    "hidden",
                ),
            ),
            Step::Say(Who::Controller, "@shownames:{other}=add"),
            Step::Expect(
                "an exception is named in the world and as an owner",
                holds(
                    |w, c| w.rlva_shows_name(NameScope::World, c.other) && w.rlva_shows_name(NameScope::Owner, c.other),
                    "still hidden",
                ),
            ),
            Step::Say(Who::Controller, "@shownames:{other}=rem"),
            Step::Expect(
                "hidden again without the exception",
                holds(|w, c| !w.rlva_shows_name(NameScope::World, c.other), "still named"),
            ),
            Step::Say(Who::Controller, "@shownames=y"),
            Step::Expect(
                "every name is back once released",
                holds(named(|_| NameScope::World), "still hidden"),
            ),
        ],
    ));
    // LLAgent::teleportViaLandmark (llagent.cpp), the rule a landmark
    // dropped in the world follows too, and LLWorldEnableTeleportHome
    // (llviewermenu.cpp): asked of the rules, nothing is sent.
    let home_entry_only = |w: &World, _: &Ctx| !w.rlva_home_entry_enabled() && w.rlva_can_teleport_home();
    cases.push(case(
        "A32",
        "@tplm and @tploc: landmarks and Teleport Home (#2729)",
        vec![
            skip_if(|w, c| {
                (!teleports_free(w, c) && c.foreign_rules(w)).then(|| "already refused by a restriction of another object".to_owned())
            }),
            Step::Now("allowed before the rules", holds(teleports_free, "already refused")),
            Step::Say(Who::Controller, "@tplm=n"),
            Step::Expect(
                "a teleport by landmark refused",
                holds(|w, _| !w.rlva_can_teleport_landmark(), "still allowed"),
            ),
            Step::Now(
                "the Teleport Home entry greyed, the Home buttons left",
                holds(home_entry_only, "the entry is not greyed, or the buttons are"),
            ),
            Step::Say(Who::Controller, "@tplm=y,tploc=n"),
            Step::Expect(
                "@tploc alone leaves the landmarks",
                holds(|w, _| w.rlva.has("tploc") && w.rlva_can_teleport_landmark(), "landmarks refused"),
            ),
            Step::Now(
                "the Teleport Home entry greyed under @tploc too",
                holds(home_entry_only, "the entry is not greyed, or the buttons are"),
            ),
            Step::Say(Who::Controller, "@tplm=n"),
            Step::Expect("no way home under both", holds(|w, _| !w.rlva_can_teleport_home(), "still allowed")),
            Step::Say(Who::Controller, "@tplm=y,tploc=y"),
            Step::Expect("allowed again once released", holds(teleports_free, "still refused")),
        ],
    ));
    cases
}

fn teleports_free(w: &World, _: &Ctx) -> bool {
    w.rlva_can_teleport_landmark() && w.rlva_home_entry_enabled() && w.rlva_can_teleport_home()
}

fn replies() -> Vec<Case> {
    vec![
        query(
            "B01",
            "@versionnew",
            "@versionnew={ch}",
            "the RLV version line",
            answers(|_, _, text| text.starts_with("RestrainedLove viewer v") && text.contains("RLVa")),
        ),
        case(
            "B02",
            "@getstatus lists the rules in the order issued",
            vec![
                skip_if(grounded),
                Step::Say(Who::Controller, "@fly=n"),
                Step::Say(Who::Controller, "@alwaysrun=n"),
                Step::Say(Who::Controller, "@showminimap=n"),
                Step::Say(Who::Controller, "@getstatus={ch}"),
                Step::Expect(
                    "/fly/alwaysrun/showminimap",
                    answers(|_, _, text| text == "/fly/alwaysrun/showminimap"),
                ),
            ],
        ),
        case(
            "B03",
            "@notify tells a rule set and a filtered @clear",
            vec![
                skip_if(grounded),
                Step::Say(Who::Controller, "@notify:{ch}=add"),
                Step::Say(Who::Controller, "@fly=n"),
                Step::Expect("/fly=n", answers(|_, _, text| text == "/fly=n")),
                Step::Say(Who::Controller, "@clear=fly"),
                Step::Expect(
                    "/clear:fly",
                    test(|_, c| {
                        if c.replies.iter().any(|r| r == "/clear:fly") {
                            Ok(())
                        } else {
                            Err(format!("{:?}", c.replies))
                        }
                    }),
                ),
            ],
        ),
        query(
            "B04",
            "@getattach counts the worn test objects",
            "@getattach={ch}",
            "1 on Hind Left Foot and Hind Right Foot",
            answers(|_, _, text| {
                let at = |p: u8| text.as_bytes().get(usize::from(p)).copied();
                text.len() == POINTS.len() && at(POINT_A) == Some(b'1') && at(POINT_CONTROLLER) == Some(b'1') && at(0) == Some(b'0')
            }),
        ),
        case(
            "B05",
            "@getattach leaves the viewer's bridge out",
            vec![
                skip_if(|w, c| {
                    let bridge = crate::world::bridge::POINT;
                    if c.bridge.is_nil() || w.worn_attachment(&c.bridge).is_none() {
                        Some("no bridge worn".to_owned())
                    } else if w
                        .worn_attachment_points()
                        .iter()
                        .any(|(item, at)| *at == bridge && *item != c.bridge)
                    {
                        Some("something else is worn on Center 2".to_owned())
                    } else {
                        None
                    }
                }),
                Step::Say(Who::Controller, "@getattach={ch}"),
                Step::Expect(
                    "0 on Center 2",
                    answers(|_, _, text| text.as_bytes().get(usize::from(crate::world::bridge::POINT)) == Some(&b'0')),
                ),
            ],
        ),
        case(
            "B06",
            "@getattachnames and @getremattachnames",
            vec![
                Step::Say(Who::Controller, "@getattachnames={ch}"),
                Step::Expect(
                    "both test points named",
                    answers(|w, _, text| {
                        let names: Vec<&str> = text.split(',').collect();
                        names.contains(&point_name(w, POINT_A)) && names.contains(&point_name(w, POINT_CONTROLLER))
                    }),
                ),
                Step::Say(Who::Controller, "@remattach:hind left foot=n"),
                Step::Say(Who::Controller, "@getremattachnames={ch}"),
                Step::Expect(
                    "the locked point left out, the other named",
                    answers(|w, _, text| {
                        let names: Vec<&str> = text.split(',').collect();
                        !names.contains(&point_name(w, POINT_A)) && names.contains(&point_name(w, POINT_CONTROLLER))
                    }),
                ),
            ],
        ),
        query(
            "B07",
            "@getpath of the controller",
            "@getpath={ch}",
            "the test folder",
            answers(|_, _, text| text == super::FOLDER),
        ),
        query(
            "B08",
            "@getinv of the test folder",
            "@getinv:{f}={ch}",
            "its six folders",
            answers(|_, _, text| {
                let mut names: Vec<&str> = text.split(',').collect();
                names.sort_unstable();
                let mut expected = SUBFOLDERS.to_vec();
                expected.sort_unstable();
                names == expected
            }),
        ),
        case(
            "B09",
            "@getinvworn of the test folder",
            steps(vec![
                take_off(&[It::B, It::Tagged, It::Probe, It::Plain]).into(),
                vec![
                    Step::Say(Who::Controller, "@getinvworn:{f}={ch}"),
                    Step::Expect(
                        "locked|30 (all worn) and free|10 (none worn)",
                        answers(|_, _, text| {
                            let parts: Vec<&str> = text.split(',').collect();
                            parts.contains(&"locked|30") && parts.contains(&"free|10")
                        }),
                    ),
                ],
            ]),
        ),
        query(
            "B10",
            "@getgroup with an option",
            "@getgroup:x={ch}",
            "the active group, or none",
            // (the reply is a group's name: a failure does not quote it)
            test(|w, c| {
                let expected = w
                    .groups
                    .groups
                    .iter()
                    .find(|g| g.id == w.groups.active)
                    .map_or("none", |g| g.name.as_str());
                if reply(c)? == expected {
                    Ok(())
                } else {
                    Err("another answer".to_owned())
                }
            }),
        ),
        case(
            "B11",
            "@getsitid with an option",
            vec![
                skip_if(|w, _| w.agent.seated.then(|| "the avatar is sitting".to_owned())),
                Step::Say(Who::Controller, "@getsitid:x={ch}"),
                Step::Expect("the null key", answers(|_, _, text| text == Uuid::nil().to_string())),
            ],
        ),
        query(
            "B12",
            "@getcam_avdistmin with an option",
            "@getcam_avdistmin:x={ch}",
            "an empty reply",
            answers(|_, _, text| text.is_empty()),
        ),
        case(
            "B13",
            "@fly:0=force while not flying",
            vec![
                skip_if(|w, _| w.agent.flying.then(|| "the avatar is flying".to_owned())),
                Step::Say(Who::Controller, "@fly:0=force"),
                Step::Pause(Duration::from_millis(500)),
                Step::Now("still on the ground", holds(|w, _| !w.agent.flying, "the avatar flies")),
            ],
        ),
    ]
}

fn steps(parts: Vec<Vec<Step>>) -> Vec<Step> {
    parts.into_iter().flatten().collect()
}

/// The detach request as it left names B only.
fn only_b_left() -> Test {
    test(|_, c| match &c.last {
        Some((_, NetCommand::DetachAttachments(items))) if items.as_slice() == [c.item(It::B)] => Ok(()),
        Some((_, NetCommand::DetachAttachments(items))) => Err(format!("{} object(s) in the request", items.len())),
        _ => Err("no detach request was seen".to_owned()),
    })
}

fn detach_both() -> Make {
    make(|_, c| Cmd::send(NetCommand::DetachAttachments(vec![c.item(It::A), c.item(It::B)])))
}

/// B was replaced on Tail Tip by the tagged object.
fn tagged_replaced_b() -> Test {
    test(|w, c| {
        if !c.worn(w, It::Tagged) {
            Err("the tagged object is not worn".to_owned())
        } else if c.point(w, It::Tagged) != POINT_B {
            Err(format!(
                "worn on {}",
                POINTS[usize::from(c.point(w, It::Tagged)).min(POINTS.len() - 1)]
            ))
        } else if c.worn(w, It::B) {
            Err("the object that held the point is still worn".to_owned())
        } else {
            Ok(())
        }
    })
}

fn round_trips() -> Vec<Case> {
    let no_foreign_tail = || {
        skip_if(|w, c| {
            foreign_on(w, c, POINT_B).then(|| "the avatar wears something of its own on Tail Tip: a test avatar is recommended".to_owned())
        })
    };
    vec![
        case(
            "C1",
            "a point lock is filtered out of a detach request (#3633, #3253)",
            steps(vec![
                wear(It::B, POINT_B).into(),
                vec![
                    Step::Say(Who::Controller, "@detach:hind left foot=n"),
                    Step::Expect(
                        "the point is locked",
                        holds(|w, c| !w.rlva_can_remove(c.item(It::A), Uuid::nil()), "not locked"),
                    ),
                    Step::Sent("one request for both objects leaves, without a refusal notice", detach_both()),
                    Step::Now("it names the unlocked object only", only_b_left()),
                    Step::Expect("the unlocked object is gone", holds(|w, c| !c.worn(w, It::B), "still worn")),
                    Step::Now(
                        "the locked object is still worn",
                        holds(|w, c| c.worn(w, It::A), "it was taken off"),
                    ),
                ],
            ]),
        ),
        case(
            "C2a",
            "@attachall replaces what the named point holds (#3635)",
            steps(vec![
                vec![no_foreign_tail()],
                take_off(&[It::Tagged]).into(),
                wear(It::B, POINT_B).into(),
                vec![
                    Step::Say(Who::Controller, "@attachall:{f}/tagged=force"),
                    Step::Expect("the tagged object on Tail Tip, the former one off", tagged_replaced_b()),
                ],
            ]),
        ),
        case(
            "C2b",
            "@attach replaces what the named point holds (#3635)",
            steps(vec![
                vec![no_foreign_tail()],
                take_off(&[It::Tagged]).into(),
                wear(It::B, POINT_B).into(),
                vec![
                    Step::Say(Who::Controller, "@attach:{f}/tagged=force"),
                    Step::Expect("the tagged object on Tail Tip, the former one off", tagged_replaced_b()),
                ],
                take_off(&[It::Tagged]).into(),
            ]),
        ),
        case(
            "C2c",
            "@attachall of an object with no known point replaces what its point holds (#3635)",
            steps(vec![
                take_off(&[It::Plain]).into(),
                wear(It::Probe, 0).into(),
                vec![
                    Step::Now(
                        "the simulator chose a point",
                        test(|w, c| {
                            c.free_point = c.point(w, It::Probe);
                            if c.free_point == 0 { Err("no point".to_owned()) } else { Ok(()) }
                        }),
                    ),
                    skip_if(|w, c| {
                        foreign_on(w, c, c.free_point)
                            .then(|| "the avatar wears something of its own on the simulator's default point".to_owned())
                    }),
                    Step::Say(Who::Controller, "@attachall:{f}/nopoint=force"),
                    Step::Expect(
                        "the object on the same point, the former one off",
                        test(|w, c| {
                            if !c.worn(w, It::Plain) {
                                Err("the object is not worn".to_owned())
                            } else if c.point(w, It::Plain) != c.free_point {
                                // the simulator put it elsewhere: nothing to replace there
                                Err("the simulator put it on another point than the first object".to_owned())
                            } else if c.worn(w, It::Probe) {
                                Err("the object that held the point is still worn".to_owned())
                            } else {
                                Ok(())
                            }
                        }),
                    ),
                ],
                take_off(&[It::Plain, It::Probe]).into(),
            ]),
        ),
        case(
            "C3a",
            "@detachthis keeps the folder's object on, others come off",
            steps(vec![
                wear(It::B, POINT_B).into(),
                vec![
                    Step::Say(Who::Controller, "@detachthis:{f}/locked=n"),
                    Step::Expect(
                        "the folder's object is locked",
                        holds(|w, c| !w.rlva_can_remove(c.item(It::A), Uuid::nil()), "not locked"),
                    ),
                    Step::Now(
                        "the controller's is not",
                        holds(|w, c| w.rlva_can_remove(c.item(It::Controller), Uuid::nil()), "locked too"),
                    ),
                    Step::Sent("one request for both objects leaves", detach_both()),
                    Step::Now("it names the other folder's object only", only_b_left()),
                    Step::Expect("that object is gone", holds(|w, c| !c.worn(w, It::B), "still worn")),
                    Step::Now(
                        "the locked folder's object is still worn",
                        holds(|w, c| c.worn(w, It::A), "it was taken off"),
                    ),
                ],
            ]),
        ),
        case(
            "C3b",
            "@detachallthis locks the folder's object",
            vec![
                Step::Say(Who::Controller, "@detachallthis:{f}/locked=n"),
                Step::Expect(
                    "the folder's object is locked",
                    holds(|w, c| !w.rlva_can_remove(c.item(It::A), Uuid::nil()), "not locked"),
                ),
                Step::Refused(
                    "taking it off is refused",
                    make(|_, c| Cmd::send(NetCommand::DetachAttachments(vec![c.item(It::A)]))),
                ),
                Step::Pause(Duration::from_millis(500)),
                Step::Now("it is still worn", holds(|w, c| c.worn(w, It::A), "it was taken off")),
            ],
        ),
        case(
            "C3c",
            "@attachthis refuses wearing the folder's object, others are allowed",
            steps(vec![
                take_off(&[It::B, It::Tagged]).into(),
                vec![
                    Step::Say(Who::Controller, "@attachthis:{f}/free=n"),
                    Step::Expect(
                        "the folder's object cannot be worn",
                        holds(|w, c| !w.rlva_can_add(c.item(It::B), POINT_B, Uuid::nil()), "still allowed"),
                    ),
                    Step::Now(
                        "another folder's object can",
                        holds(|w, c| w.rlva_can_add(c.item(It::Tagged), POINT_B, Uuid::nil()), "refused too"),
                    ),
                    Step::Refused(
                        "wearing it is refused",
                        make(|w, c| match w.inventory.items.get(&c.item(It::B)) {
                            Some(item) => Cmd::send(NetCommand::RezAttachments(vec![attach_request(item, c.agent, POINT_B)])),
                            None => Cmd::Missing("the test object"),
                        }),
                    ),
                    Step::Pause(Duration::from_millis(500)),
                    Step::Now("it is not worn", holds(|w, c| !c.worn(w, It::B), "it was worn")),
                ],
            ]),
        ),
        case(
            "C4",
            "an object linked from a locked folder is locked",
            steps(vec![
                wear(It::B, POINT_B).into(),
                vec![
                    Step::Now(
                        "the folder holds a link to the object",
                        holds(
                            |w, c| {
                                w.inventory.folders.get(&c.folders[LINKED]).is_some_and(|f| {
                                    f.items
                                        .iter()
                                        .filter_map(|id| w.inventory.items.get(id))
                                        .any(|i| i.asset_type == 24 && i.asset_id == c.item(It::B))
                                })
                            },
                            "no link",
                        ),
                    ),
                    Step::Say(Who::Controller, "@detachthis:{f}/linked=n"),
                    Step::Soon(
                        "the linked object cannot be taken off",
                        holds(|w, c| !w.rlva_can_remove(c.item(It::B), Uuid::nil()), "it can be taken off"),
                    ),
                    Step::Refused(
                        "taking it off is refused",
                        make(|_, c| Cmd::send(NetCommand::DetachAttachments(vec![c.item(It::B)]))),
                    ),
                ],
            ]),
        ),
        case(
            "C5",
            "taking the controller off releases its rules",
            steps(vec![
                vec![
                    skip_if(grounded),
                    Step::Say(Who::Second, "@notify:{ch}=add"),
                    Step::Say(Who::Controller, "@fly=n"),
                    Step::Expect("the rule is set", holds(|w, _| w.rlva.has("fly"), "not set")),
                    Step::Sent(
                        "the controller is taken off",
                        make(|_, c| Cmd::send(NetCommand::DetachAttachments(vec![c.item(It::Controller)]))),
                    ),
                    Step::Expect(
                        "the controller is off",
                        test(|w, c| {
                            if c.worn(w, It::Controller) {
                                return Err("still worn".to_owned());
                            }
                            c.controller = Uuid::nil();
                            Ok(())
                        }),
                    ),
                    Step::Expect("its rule is gone", holds(|w, _| !w.rlva.has("fly"), "the rule stays")),
                    Step::Expect(
                        "the listener is told /clear",
                        test(|_, c| {
                            if c.replies.iter().any(|r| r == "/clear") {
                                Ok(())
                            } else {
                                Err(format!("{:?}", c.replies))
                            }
                        }),
                    ),
                ],
                wear(It::Controller, POINT_CONTROLLER).into(),
                vec![
                    Step::Now(
                        "the controller is worn again",
                        test(|w, c| {
                            c.controller = w
                                .worn_attachment(&c.item(It::Controller))
                                .and_then(|i| w.objects.get(i))
                                .map_or(Uuid::nil(), |o| o.full_id);
                            if c.controller.is_nil() {
                                Err("not worn".to_owned())
                            } else {
                                Ok(())
                            }
                        }),
                    ),
                    Step::Hello(Who::Controller),
                    Step::Now("no rule came back with it", holds(|w, _| !w.rlva.has("fly"), "the rule is back")),
                ],
            ]),
        ),
        // RlvForceWear::forceDetach (rlvhelper.cpp) reads the locks when the
        // force is processed: the release that follows it in the same
        // message comes too late for it.
        case(
            "C6",
            "a forced removal between a lock and its release in one message obeys the lock",
            steps(vec![
                vec![no_foreign_tail()],
                wear(It::B, POINT_B).into(),
                vec![
                    Step::Say(
                        Who::Controller,
                        "@remattach:tail tip=n,remattach:tail tip=force,remattach:tail tip=y",
                    ),
                    Step::Expect(
                        "the lock is released",
                        holds(|w, c| w.rlva_can_remove(c.item(It::B), Uuid::nil()), "still locked"),
                    ),
                    Step::Pause(Duration::from_secs(2)),
                    Step::Now("the object is still worn", holds(|w, c| c.worn(w, It::B), "it was taken off")),
                    Step::Say(Who::Controller, "@remattach:tail tip=force"),
                    Step::Expect("without the lock it comes off", holds(|w, c| !c.worn(w, It::B), "still worn")),
                ],
            ]),
        ),
    ]
}

pub(super) fn phase1() -> Vec<Case> {
    let mut cases = rules();
    cases.sort_by_key(|c| c.id);
    cases.extend(replies());
    cases.extend(round_trips());
    cases.extend(super::cases_move::phase1());
    cases
}
