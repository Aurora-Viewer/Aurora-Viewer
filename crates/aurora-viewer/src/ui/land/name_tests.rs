//! Parcel-name commits through the actual Aurora and classic windows.

use super::*;
use crate::world::land::powers;
use aurora_net::land::{LandCommand, OS_LEASE_PENDING};

const ORIGINAL: &str = "Original parcel";
const RENAMED: &str = "New parcel";

struct Harness {
    ctx: egui::Context,
    palette: Palette,
    land: LandUi,
    world: World,
    time: f64,
    classic: bool,
    open: bool,
}
impl Harness {
    fn new(classic: bool) -> Self {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.main_region = Some(10);
        world.land.select_at(10, glam::vec3(128.0, 128.0, 0.0), (256.0, 256.0));
        world.land.sel.as_mut().unwrap().parcel = Some(Arc::new(ParcelInfo {
            local_id: 7,
            name: "Original parcel".into(),
            owner_id: world.agent_id,
            status: aurora_net::land::OS_LEASED,
            area: 4096,
            region_allow_access_override: true,
            ..Default::default()
        }));
        let mut harness = Self {
            ctx,
            palette: theme.palette(),
            land: LandUi::default(),
            world,
            time: 0.0,
            classic,
            open: true,
        };
        for _ in 0..3 {
            harness.frame(vec![]);
        }
        harness.world.land.take_commands();
        harness
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.1;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1000.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |_| {
                self.land.show(
                    &self.ctx,
                    &self.palette,
                    &mut self.world,
                    &HashMap::new(),
                    &HashMap::new(),
                    None,
                    &mut Vec::new(),
                    true,
                    self.classic,
                    &mut self.open,
                );
            },
        );
        output.textures_delta.clear();
        output
    }
    fn label_rect(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|s| Self::label_rect(s, label)),
            _ => None,
        }
    }
    fn click(&mut self, label: &str) {
        let output = self.frame(vec![]);
        let pos = output
            .shapes
            .iter()
            .rev()
            .find_map(|s| Self::label_rect(&s.shape, label))
            .expect(label)
            .center();
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        self.frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }
    fn type_name(&mut self) {
        self.click("Original parcel");
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        self.frame(vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(vec![egui::Event::Text("New parcel".into())]);
        assert_eq!(self.land.name.text, "New parcel");
    }
}

impl Harness {
    fn parcel(&mut self) -> &mut ParcelInfo {
        Arc::make_mut(self.world.land.sel.as_mut().expect("selection").parcel.as_mut().expect("parcel"))
    }

    fn close(&mut self) {
        self.open = false;
        self.frame(vec![]);
    }

    fn group_owned(&mut self) {
        let group = Uuid::from_u128(9);
        let parcel = self.parcel();
        parcel.owner_id = group;
        parcel.group_id = group;
        parcel.is_group_owned = true;
        self.world.groups.groups = vec![GroupMembership {
            id: group,
            name: "Parcel group".into(),
            insignia: Uuid::nil(),
            powers: powers::LAND_CHANGE_IDENTITY,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }];
    }

    fn assert_discarded(&mut self) {
        assert!(
            !self
                .world
                .land
                .take_commands()
                .iter()
                .any(|command| matches!(command, aurora_net::NetCommand::Land(LandCommand::Update { .. })))
        );
        assert_eq!(self.land.name.text, ORIGINAL);
        assert_eq!(self.land.name.base, ORIGINAL);
        assert!(self.land.name.editing.is_none());
    }
}

#[test]
fn parcel_rename_rechecks_ownership_when_leaving_the_header() {
    let mut harness = Harness::new(false);
    harness.type_name();
    harness.parcel().owner_id = Uuid::from_u128(99);
    harness.click(aurora_i18n::tr!("ui-land-overview-manage-tab"));
    assert_eq!(harness.land.page, Page::Manage);
    assert_eq!(harness.parcel().name, ORIGINAL);
    harness.assert_discarded();
}

#[derive(Clone, Copy)]
enum PermissionLoss {
    Ownership,
    GroupPower,
    Lease,
}

impl PermissionLoss {
    fn apply(self, harness: &mut Harness) {
        match self {
            Self::Ownership => harness.parcel().owner_id = Uuid::from_u128(99),
            Self::GroupPower => harness.world.groups.groups[0].powers = 0,
            Self::Lease => harness.parcel().status = OS_LEASE_PENDING,
        }
    }
}

#[test]
fn parcel_rename_discards_a_pending_close_after_identity_rights_are_lost() {
    for classic in [false, true] {
        for loss in [PermissionLoss::Ownership, PermissionLoss::GroupPower, PermissionLoss::Lease] {
            let mut harness = Harness::new(classic);
            if matches!(loss, PermissionLoss::GroupPower) {
                harness.group_owned();
            }
            harness.type_name();
            loss.apply(&mut harness);
            harness.close();
            harness.assert_discarded();
        }
    }
}

#[test]
fn parcel_rename_rechecks_identity_rights_after_a_field_commits() {
    for classic in [false, true] {
        for loss in [PermissionLoss::Ownership, PermissionLoss::GroupPower, PermissionLoss::Lease] {
            let mut harness = Harness::new(classic);
            if matches!(loss, PermissionLoss::GroupPower) {
                harness.group_owned();
            }
            harness.type_name();
            let pending = harness.land.name.take_pending();
            loss.apply(&mut harness);
            harness.land.apply_name(&mut harness.world, pending);
            assert_eq!(harness.parcel().name, ORIGINAL);
            harness.assert_discarded();
        }
    }
}

#[test]
fn parcel_rename_with_enter_keeps_current_identity_rights_in_both_layouts() {
    for classic in [false, true] {
        let mut harness = Harness::new(classic);
        harness.type_name();
        harness.frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(harness.parcel().name, RENAMED);
    }
}

#[test]
fn parcel_rename_page_and_visitor_transitions_keep_valid_edits() {
    for visitor in [false, true] {
        let mut harness = Harness::new(false);
        harness.type_name();
        harness.click(if visitor {
            aurora_i18n::tr!("ui-land-overview-visitor")
        } else {
            aurora_i18n::tr!("ui-land-overview-manage-tab")
        });
        assert_eq!(harness.parcel().name, RENAMED);
    }
}

#[test]
fn parcel_rename_closing_keeps_valid_edits_in_both_layouts() {
    for classic in [false, true] {
        let mut harness = Harness::new(classic);
        harness.type_name();
        harness.close();
        assert!(harness.world.land.take_commands().iter().any(|command| matches!(
            command,
            aurora_net::NetCommand::Land(LandCommand::Update { update, .. }) if update.name == RENAMED
        )));
    }
}
