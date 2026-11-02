//! General tab (LLPanelPermissions, indra/newview/llpanelpermissions.cpp,
//! originally LGPL 2.1): name, description, creator / owner / previous
//! owner, group, share / deed, click action, sale, object key, search,
//! permissions of everyone and of the next owner.

use super::common::{check, combo, label, row, small, spin_i, text};
use super::{Confirm, LABEL_W, Request};
use crate::build::BuildTool;
use crate::build::edits::{CLICK_ACTION_BUY, CLICK_ACTION_TOUCH, FLAGS_INCLUDE_IN_SEARCH};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::build::{ObjectProps, perm, perm_field};
use egui::RichText;
use uuid::Uuid;

/// Group powers (roles_constants.h).
const GP_OBJECT_DEED: u64 = 1 << 36;
const GP_OBJECT_MANIPULATE: u64 = 1 << 38;
const GP_OBJECT_SET_SALE: u64 = 1 << 39;
/// FS_NOT / ORIGINAL / COPY / CONTENTS.
const SALE_COPY: u8 = 2;

/// "clickaction" combo, in its display order (CLICK_ACTION_* codes).
fn click_actions() -> [(&'static str, u8); 8] {
    [
        (aurora_i18n::tr!("build-ui-general-touch-default"), 0),
        (aurora_i18n::tr!("build-ui-general-sit-on-object"), 1),
        (aurora_i18n::tr!("common-buy-object"), 2),
        (aurora_i18n::tr!("common-pay-object"), 3),
        (aurora_i18n::tr!("common-open"), 4),
        (aurora_i18n::tr!("common-zoom-2"), 7),
        (aurora_i18n::tr!("build-ui-general-ignore-object"), 9),
        (aurora_i18n::tr!("common-none"), 8),
    ]
}

/// "sale type" combo: Copy, Contents, Original.
fn sale_types() -> [(&'static str, u8); 3] {
    [
        (aurora_i18n::tr!("common-copy"), 2),
        (aurora_i18n::tr!("build-ui-general-contents"), 3),
        (aurora_i18n::tr!("common-original"), 1),
    ]
}

/// LLSelectMgr::selectGetPerm over the roots: (all set, all clear) bits.
fn perm_state(list: &[&ObjectProps], mask: impl Fn(&ObjectProps) -> u32) -> (u32, u32) {
    let mut on = u32::MAX;
    let mut off = u32::MAX;
    for p in list {
        let m = mask(p);
        on &= m;
        off &= !m;
    }
    (on, off)
}

/// (value, tentative) of one bit.
fn bit(state: (u32, u32), b: u32) -> (bool, bool) {
    if state.0 & b == b {
        (true, false)
    } else if state.1 & b == b {
        (false, false)
    } else {
        (true, true)
    }
}

/// A clickable name (agent or group SLURL of the panel).
fn name_link(ui: &mut egui::Ui, p: &Palette, name: &str) -> bool {
    ui.add(egui::Label::new(RichText::new(name).size(12.0).color(p.violet_light)).sense(egui::Sense::click()))
        .on_hover_text(aurora_i18n::tr!("build-ui-general-open-profile"))
        .clicked()
}

fn person_link(ui: &mut egui::Ui, p: &Palette, world: &World, id: Uuid) -> bool {
    if !world.rlva_can_name(id) {
        text(ui, p, aurora_i18n::tr!("scene-object-hidden-person"));
        false
    } else {
        name_link(ui, p, &world.social.name_of(&id))
    }
}

fn group_name(world: &mut World, id: Uuid) -> String {
    world
        .land
        .group_name(&id, &world.groups.groups)
        .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into())
}

/// The shared General editor and its detached property dialogs must use
/// current objects and full properties. LLViewerObject::isPermanentEnforced
/// prevents editing permanent objects in a neighbouring region.
fn property_policy(tool: &BuildTool, world: &World, targets: &[usize]) -> bool {
    !targets.is_empty()
        && targets.iter().all(|&index| {
            world.objects.get(index).is_some_and(|object| {
                !object.is_avatar()
                    && !object.is_client_preview()
                    && world.rlva_can_edit(index)
                    && (object.update_flags & crate::ui::context::flags::AFFECTS_NAVMESH == 0
                        || world.main_region == Some(object.key.region))
                    && tool.props.get(&object.full_id).is_some_and(|props| props.full)
            })
        })
}

pub(super) fn can_set_group(tool: &BuildTool, world: &World) -> bool {
    can_set_group_targets(tool, world, &tool.prop_targets(world))
}

fn can_set_group_targets(tool: &BuildTool, world: &World, targets: &[usize]) -> bool {
    property_policy(tool, world, targets)
        && targets.iter().all(|&index| {
            crate::build::root_of(world, index) == index
                && world
                    .objects
                    .get(index)
                    .and_then(|object| tool.props.get(&object.full_id))
                    .is_some_and(|props| props.owner_id == world.agent_id)
        })
}

pub(super) fn can_deed(tool: &BuildTool, world: &World, group: Uuid) -> bool {
    can_deed_targets(tool, world, &tool.prop_targets(world), group)
}

fn can_deed_targets(tool: &BuildTool, world: &World, targets: &[usize], group: Uuid) -> bool {
    can_set_group_targets(tool, world, targets)
        && !group.is_nil()
        && !tool.attachment_selected(world)
        && world
            .groups
            .group(&group)
            .is_some_and(|membership| membership.powers & GP_OBJECT_DEED != 0)
        && targets.iter().all(|&index| {
            world
                .objects
                .get(index)
                .and_then(|object| tool.props.get(&object.full_id))
                .is_some_and(|props| {
                    props.group_id == group && props.owner_mask & perm::TRANSFER != 0 && props.group_mask & perm::MOVE != 0
                })
        })
}

fn valid_sale(for_sale: bool, sale_type: u8, owner_mask: u32) -> bool {
    !for_sale || owner_mask & perm::TRANSFER != 0 && (sale_type != SALE_COPY || owner_mask & perm::COPY != 0)
}

fn pathfinding_attributes(world: &World, targets: &[usize]) -> &'static str {
    let all = |mask: u32, set: bool| {
        targets.iter().all(|&index| {
            world
                .objects
                .get(index)
                .is_some_and(|object| (object.update_flags & mask != 0) == set)
        })
    };
    let permanent = crate::ui::context::flags::AFFECTS_NAVMESH;
    let character = 1 << 13; // FLAGS_CHARACTER (llprimitive/object_flags.h).
    if all(permanent | character, false) {
        aurora_i18n::tr!("common-none")
    } else if all(permanent, true) {
        aurora_i18n::tr!("scene-object-pathfinding-permanent")
    } else if all(character, true) {
        aurora_i18n::tr!("scene-object-pathfinding-character")
    } else {
        aurora_i18n::tr!("status-multiple")
    }
}

/// Standalone Object > Profile uses the same group picker and deed question
/// as General, with its own IDs and a pinned object selection.
pub(crate) fn property_dialogs(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    super::floaters::group_picker(ctx, p, tool, world, "object_profile_group_picker");
    if let Some(Confirm::Deed(group)) = tool.ui.confirm.clone()
        && let Some(yes) = super::floaters::confirm_answer(ctx, p, &Confirm::Deed(group), egui::Id::unique("object_profile_deed"))
    {
        tool.ui.confirm = None;
        if yes && can_deed(tool, world, group) {
            tool.deed(world, group);
        }
    }
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, permissions: bool) {
    let legacy = super::legacy::active(ui);
    let identity_width = if legacy { 90.0 } else { LABEL_W };
    // the roots, or the selected prims when no root is (LLPanelPermissions::refresh)
    let mut targets: Vec<usize> = tool
        .sel_prims(world)
        .into_iter()
        .filter(|&i| crate::build::root_of(world, i) == i)
        .collect();
    let root_selected = !targets.is_empty();
    if !root_selected {
        targets = tool.sel_prims(world);
    }
    let ids: Vec<Uuid> = targets.iter().filter_map(|&i| world.objects.get(i).map(|o| o.full_id)).collect();
    let props: Vec<ObjectProps> = ids.iter().filter_map(|id| tool.props.get(id).cloned()).collect();
    if !targets.is_empty()
        && targets
            .iter()
            .all(|&idx| world.objects.get(idx).is_some_and(|object| object.is_client_preview()))
    {
        for properties in &props {
            ui.label(&properties.name);
        }
        small(ui, p, aurora_i18n::tr!("blender-live-local-only"));
        return;
    }
    if props.len() != ids.len() || props.is_empty() || props.iter().any(|p| !p.full) {
        small(ui, p, aurora_i18n::tr!("build-ui-general-waiting-for-simulator-properties"));
        return;
    }
    let refs: Vec<&ObjectProps> = props.iter().collect();
    let first = &props[0];
    let count = props.len();
    let me = world.agent_id;
    let same = |f: &dyn Fn(&ObjectProps) -> Uuid| props.iter().all(|x| f(x) == f(first));
    let owners_identical = same(&|x| x.owner_id);
    let group_owned = same(&|x| x.group_id) && !first.group_id.is_nil() && (first.owner_id.is_nil() || first.owner_id == first.group_id);
    let group_power = |power: u64| world.groups.group(&first.group_id).is_some_and(|g| g.powers & power != 0);
    let self_owned = owners_identical && first.owner_id == me;
    let allowed = property_policy(tool, world, &targets);
    let can_manipulate = allowed && (self_owned || (owners_identical && group_owned && group_power(GP_OBJECT_MANIPULATE)));
    let can_sell = allowed && (self_owned || (owners_identical && group_owned && group_power(GP_OBJECT_SET_SALE)));
    // Editing and changing ownership permissions are separate rights
    // (LLPanelPermissions::refresh / LLViewerObject::permModify).
    let modify = allowed && targets.iter().all(|&idx| tool.can_modify(world, idx));
    let all_volume = targets.iter().all(|&index| {
        world
            .objects
            .get(index)
            .is_some_and(|object| object.pcode == aurora_prim::params::LL_PCODE_VOLUME)
    });

    let group = perm_state(&refs, |x| x.group_mask);
    let owner = perm_state(&refs, |x| x.owner_mask);
    let share_bits = perm::COPY | perm::MODIFY | perm::MOVE;
    let (share, share_tentative) = if group.0 & share_bits == share_bits {
        (true, false)
    } else if group.1 & share_bits == share_bits {
        (false, false)
    } else {
        (true, true)
    };
    let can_transfer = owner.0 & perm::TRANSFER != 0;
    if !permissions {
        // ---- name, description
        for (field, title, limit, description) in [
            ("name", aurora_i18n::tr!("common-name-2"), 63, false),
            ("desc", aurora_i18n::tr!("common-description"), 127, true),
        ] {
            let current = if count == 1 {
                if description {
                    first.description.clone()
                } else {
                    first.name.clone()
                }
            } else {
                String::new()
            };
            let mut v = tool.ui.edits.get(field).cloned().unwrap_or_else(|| current.clone());
            row(ui, p, title, if legacy { 100.0 } else { LABEL_W - 30.0 }, |ui| {
                let hint = if count > 1 {
                    aurora_i18n::tr!("build-ui-general-multiple-selection")
                } else {
                    ""
                };
                let width = ui.available_width();
                // read only without the modify right, still selectable
                let r = crate::ui::widgets::text_field(ui, modify, &mut v, |b, _| {
                    egui::TextEdit::singleline(b).desired_width(width).char_limit(limit).hint_text(hint)
                });
                if description {
                    r.clone().on_hover_text(aurora_i18n::tr!("build-ui-general-127-bytes-maximum"));
                } else {
                    r.clone()
                        .on_hover_text(aurora_i18n::tr!("build-ui-general-63-characters-maximum-printable-ascii-excluding"));
                }
                if r.changed() {
                    // validateASCIIPrintableNoPipe
                    v.retain(|c| (' '..='~').contains(&c) && c != '|');
                    tool.ui.edits.insert(field, v.clone());
                }
                if r.lost_focus() {
                    tool.ui.edits.remove(field);
                    if modify && v != current && (description || !v.trim().is_empty()) {
                        tool.set_name(world, &v, description);
                    }
                }
            });
        }
        ui.add_space(4.0);

        // ---- creator, owner, previous owner, group
        for id in [first.creator_id, first.owner_id, first.last_owner_id] {
            if !id.is_nil() && id != first.group_id && world.rlva_can_name(id) {
                world.social.want_name(id);
            }
        }
        row(ui, p, aurora_i18n::tr!("common-creator-2"), identity_width, |ui| {
            if !same(&|x| x.creator_id) {
                text(ui, p, aurora_i18n::tr!("status-multiple"));
            } else if first.creator_id.is_nil() {
                text(ui, p, aurora_i18n::tr!("status-nobody"));
            } else if person_link(ui, p, world, first.creator_id) {
                tool.ui.requests.push(Request::Profile(first.creator_id));
            }
        });
        row(ui, p, aurora_i18n::tr!("common-owner-2"), identity_width, |ui| {
            if !owners_identical {
                text(ui, p, aurora_i18n::tr!("status-multiple"));
            } else if group_owned {
                let name = group_name(world, first.group_id);
                if name_link(ui, p, &name) {
                    tool.ui.requests.push(Request::GroupProfile(first.group_id));
                }
            } else if person_link(ui, p, world, first.owner_id) {
                tool.ui.requests.push(Request::Profile(first.owner_id));
            }
        });
        row(ui, p, aurora_i18n::tr!("build-ui-general-previous-owner"), identity_width, |ui| {
            if !same(&|x| x.last_owner_id) || first.last_owner_id.is_nil() {
                return;
            }
            if first.last_owner_id == first.group_id {
                if name_link(ui, p, &group_name(world, first.group_id)) {
                    tool.ui.requests.push(Request::GroupProfile(first.group_id));
                }
            } else if person_link(ui, p, world, first.last_owner_id) {
                tool.ui.requests.push(Request::Profile(first.last_owner_id));
            }
        });
        row(ui, p, aurora_i18n::tr!("common-group-2"), identity_width, |ui| {
            if same(&|x| x.group_id) && !first.group_id.is_nil() {
                let name = group_name(world, first.group_id);
                if name_link(ui, p, &name) {
                    tool.ui.requests.push(Request::GroupProfile(first.group_id));
                }
            } else {
                text(ui, p, "");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if super::common::icon_button(
                    ui,
                    p,
                    "users-group",
                    aurora_i18n::tr!("build-ui-general-set-group"),
                    can_set_group_targets(tool, world, &targets),
                ) {
                    tool.ui.group_picker_open = true;
                }
            });
        });

        // ---- share with group, deed
        ui.horizontal(|ui| {
            ui.add_space(identity_width);
            if let Some(v) = check(ui, p, share, share_tentative, aurora_i18n::tr!("common-share"), can_manipulate) {
                tool.set_perm(world, perm_field::GROUP, v, share_bits);
            }
            let deed_ok = can_deed_targets(tool, world, &targets, first.group_id);
            if ui
                .add_enabled(
                    deed_ok,
                    egui::Button::new(RichText::new(aurora_i18n::tr!("common-deed")).size(12.0)),
                )
                .on_hover_text(aurora_i18n::tr!("build-ui-general-deed-this-object-to-group"))
                .clicked()
            {
                tool.ui.confirm = Some(Confirm::Deed(first.group_id));
            }
        });

        if !legacy {
            small(
                ui,
                p,
                if modify {
                    aurora_i18n::tr!("build-ui-general-you-can-edit-this-object")
                } else {
                    aurora_i18n::tr!("build-ui-general-you-cannot-edit-this-object")
                },
            );
            ownership_permissions(ui, p, tool, world, &props, can_manipulate, can_sell);
            crate::ui::widgets::divider(ui, p);
        }

        // ---- click action
        let prims = tool.sel_prims(world);
        let (click, _) = super::common::same(prims.iter().filter_map(|&i| world.objects.get(i).map(|o| o.click_action)));
        // PLAY / OPEN_MEDIA show as "Touch"
        let click = click.map(|c| if click_actions().iter().any(|(_, v)| *v == c) { c } else { 0 });
        row(ui, p, aurora_i18n::tr!("build-ui-general-click-to"), LABEL_W, |ui| {
            if let Some(a) = combo(ui, "click_action", click, &click_actions(), 150.0, modify && all_volume) {
                if a == CLICK_ACTION_BUY && !props.iter().all(|x| x.sale_type != 0) {
                    // CantSetBuyObject
                    tool.status = aurora_i18n::tr!("build-ui-general-this-object-is-not-for-sale-buy-object-cannot").into();
                } else {
                    tool.set_click_action(world, a);
                }
            }
        });

        // ---- sale
        let for_sale_n = props.iter().filter(|x| x.sale_type != 0).count();
        let sale_mixed = for_sale_n != 0 && for_sale_n != count;
        let price_mixed = !props.iter().all(|x| x.sale_price == first.sale_price);
        let sale_type_now = props.iter().find(|x| x.sale_type != 0).map(|x| x.sale_type).unwrap_or(SALE_COPY);
        let sale_ok = !tool.attachment_selected(world) && can_sell;
        let (mut for_sale, mut sale_type, mut price) =
            tool.ui
                .sale_draft
                .unwrap_or((for_sale_n > 0, sale_type_now, first.sale_price.max(0)));
        let mut draft_changed = false;
        let mut commit_now = false;
        egui::Frame::new()
            .stroke(egui::Stroke::new(
                1.0,
                if tool.ui.sale_draft.is_some() { p.violet } else { p.raised },
            ))
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(4)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some(v) = check(
                        ui,
                        p,
                        for_sale,
                        sale_mixed,
                        aurora_i18n::tr!("common-for-sale"),
                        sale_ok && (can_transfer || for_sale_n > 0),
                    ) {
                        for_sale = v;
                        draft_changed = true;
                        // unticking commits at once (onCommitSaleInfo)
                        commit_now = !v;
                    }
                    if let Some(t) = combo(
                        ui,
                        "sale_type",
                        Some(sale_type),
                        &sale_types(),
                        110.0,
                        sale_ok && can_transfer && for_sale && !price_mixed,
                    ) {
                        sale_type = t;
                        draft_changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    label(ui, p, "L$");
                    ui.add_enabled_ui(sale_ok && can_transfer && for_sale && !sale_mixed, |ui| {
                        let e = spin_i(ui, &mut price, 0..=999_999_999, 100.0);
                        if e.changed {
                            draft_changed = true;
                        }
                        // a typed price commits when "For sale" is ticked
                        if e.commit && for_sale {
                            commit_now = true;
                        }
                    });
                    if ui
                        .add_enabled(
                            sale_ok && valid_sale(for_sale, sale_type, owner.0) && (tool.ui.sale_draft.is_some() || draft_changed),
                            egui::Button::new(RichText::new(aurora_i18n::tr!("common-apply")).size(12.0)),
                        )
                        .clicked()
                    {
                        commit_now = true;
                    }
                });
            });
        if draft_changed {
            tool.ui.sale_draft = Some((for_sale, sale_type, price));
        }
        if commit_now && sale_ok && valid_sale(for_sale, sale_type, owner.0) {
            // LLPanelPermissions::setAllSaleInfo
            let st = if for_sale && price >= 0 { sale_type } else { 0 };
            let was_for_sale = for_sale_n > 0;
            tool.set_sale(world, st, price.max(0));
            tool.ui.sale_draft = None;
            // FIRE-5273: Buy / Touch follow the sale state when they were Buy or Touch
            let all_touch_or_buy = prims.iter().all(|&i| {
                world
                    .objects
                    .get(i)
                    .is_some_and(|o| o.click_action == CLICK_ACTION_BUY || o.click_action == CLICK_ACTION_TOUCH)
            });
            if modify && all_touch_or_buy && was_for_sale != (st != 0) {
                tool.set_click_action(world, if st != 0 { CLICK_ACTION_BUY } else { CLICK_ACTION_TOUCH });
            }
        }

        // ---- key, search
        ui.horizontal(|ui| {
            if legacy && ui.button(aurora_i18n::tr!("build-legacy-copy-keys")).clicked() {
                tool.ui.requests.push(Request::CopyKeys);
            }
            let roots = tool.roots(world);
            let (search, search_mixed) = super::common::same(
                roots
                    .iter()
                    .filter_map(|&r| world.objects.get(r).map(|o| o.update_flags & FLAGS_INCLUDE_IN_SEARCH != 0)),
            );
            if let Some(v) = check(
                ui,
                p,
                search.unwrap_or(false),
                search_mixed,
                aurora_i18n::tr!("common-show-in-search"),
                can_sell && all_volume,
            ) {
                tool.set_include_in_search(world, v);
            }
        });
        if legacy {
            small(
                ui,
                p,
                if modify {
                    aurora_i18n::tr!("build-ui-general-you-can-edit-this-object")
                } else {
                    aurora_i18n::tr!("build-ui-general-you-cannot-edit-this-object")
                },
            );
            ownership_permissions(ui, p, tool, world, &props, can_manipulate, can_sell);
        }
    }
    if permissions {
        // ---- modify info (text modify info 1..6)
        let info = if !can_manipulate && !root_selected {
            aurora_i18n::tr!("build-ui-general-select-the-whole-object")
        } else {
            match (modify, count == 1) {
                (true, true) => aurora_i18n::tr!("build-ui-general-you-can-edit-this-object"),
                (true, false) => aurora_i18n::tr!("build-ui-general-you-can-edit-these-objects"),
                (false, true) => aurora_i18n::tr!("build-ui-general-you-cannot-edit-this-object"),
                (false, false) => aurora_i18n::tr!("build-ui-general-you-cannot-edit-these-objects"),
            }
        };
        ui.label(RichText::new(info).size(12.0).color(if modify { p.violet_light } else { p.muted }));

        ownership_permissions(ui, p, tool, world, &props, can_manipulate, can_sell);
    } else {
        row(
            ui,
            p,
            aurora_i18n::tr!("build-ui-general-pathfinding-attributes"),
            LABEL_W + 30.0,
            |ui| {
                text(ui, p, pathfinding_attributes(world, &targets));
            },
        );
    }
}

fn ownership_permissions(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    props: &[ObjectProps],
    can_manipulate: bool,
    can_sell: bool,
) {
    let refs: Vec<_> = props.iter().collect();
    let owner = perm_state(&refs, |x| x.owner_mask);
    let base = perm_state(&refs, |x| x.base_mask);
    let can_transfer = owner.0 & perm::TRANSFER != 0;
    // ---- everyone / next owner
    let everyone = perm_state(&refs, |x| x.everyone_mask);
    let next = perm_state(&refs, |x| x.next_owner_mask);
    label(ui, p, aurora_i18n::tr!("build-ui-general-anyone"));
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let (v, t) = bit(everyone, perm::MOVE);
        if let Some(n) = check(
            ui,
            p,
            v,
            t,
            aurora_i18n::tr!("common-move"),
            can_manipulate && owner.0 & perm::MOVE != 0,
        ) {
            tool.set_perm(world, perm_field::EVERYONE, n, perm::MOVE);
        }
        let (v, t) = bit(everyone, perm::COPY);
        let copy_ok = owner.0 & perm::COPY != 0 && owner.0 & perm::TRANSFER != 0;
        if let Some(n) = check(
            ui,
            p,
            v,
            t || !copy_ok,
            aurora_i18n::tr!("common-copy-2"),
            can_manipulate && copy_ok,
        ) {
            tool.set_perm(world, perm_field::EVERYONE, n, perm::COPY);
        }
    });
    label(ui, p, aurora_i18n::tr!("build-ui-general-next-owner"));
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let next_ok = can_sell && owner.0 & perm::TRANSFER != 0;
        let (v, t) = bit(next, perm::MODIFY);
        if let Some(n) = check(ui, p, v, t, aurora_i18n::tr!("common-edit"), next_ok && base.0 & perm::MODIFY != 0) {
            tool.set_perm(world, perm_field::NEXT_OWNER, n, perm::MODIFY);
        }
        let (v, t) = bit(next, perm::COPY);
        let can_copy = owner.0 & perm::COPY != 0;
        if let Some(n) = check(
            ui,
            p,
            v,
            t || !can_copy,
            aurora_i18n::tr!("common-copy-2"),
            next_ok && base.0 & perm::COPY != 0,
        ) {
            tool.set_perm(world, perm_field::NEXT_OWNER, n, perm::COPY);
        }
        let (v, t) = bit(next, perm::TRANSFER);
        // enabled by the next owner's copy bit (sic, LLPanelPermissions)
        if let Some(n) = check(
            ui,
            p,
            v,
            t || !can_transfer,
            aurora_i18n::tr!("common-transfer"),
            next_ok && next.0 & perm::COPY != 0,
        ) {
            tool.set_perm(world, perm_field::NEXT_OWNER, n, perm::TRANSFER);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perm_bits_mixed() {
        let a = ObjectProps {
            everyone_mask: perm::MOVE,
            ..Default::default()
        };
        let b = ObjectProps::default();
        let st = perm_state(&[&a, &b], |x| x.everyone_mask);
        assert_eq!(bit(st, perm::MOVE), (true, true));
        assert_eq!(bit(st, perm::COPY), (false, false));
        let st = perm_state(&[&a, &a], |x| x.everyone_mask);
        assert_eq!(bit(st, perm::MOVE), (true, false));
    }

    #[test]
    fn detached_property_dialogs_require_current_full_owner_permissions_and_group_powers() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let (index, object) = world
            .objects
            .iter()
            .find(|(_, object)| !object.is_avatar() && object.parent_id == 0)
            .unwrap();
        let (id, key) = (object.full_id, object.key);
        let group = Uuid::from_u128(0xfeed_0101);
        world.agent_id = Uuid::from_u128(700);
        world.main_region = Some(key.region);
        world.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Property group".into(),
            insignia: Uuid::nil(),
            powers: GP_OBJECT_DEED,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        let mut tool = BuildTool {
            selection: vec![key],
            ..Default::default()
        };
        tool.props.insert(
            id,
            ObjectProps {
                object_id: id,
                full: true,
                owner_id: world.agent_id,
                group_id: group,
                owner_mask: perm::TRANSFER,
                group_mask: perm::MOVE,
                ..Default::default()
            },
        );
        assert!(can_set_group(&tool, &world));
        assert!(can_deed(&tool, &world, group));
        world.groups.groups.last_mut().unwrap().powers = 0;
        assert!(!can_deed(&tool, &world, group));
        tool.props.get_mut(&id).unwrap().owner_id = Uuid::from_u128(701);
        assert!(!can_set_group(&tool, &world));
        assert!(!can_deed(&tool, &world, group));
        tool.props.get_mut(&id).unwrap().owner_id = world.agent_id;
        tool.props.get_mut(&id).unwrap().full = false;
        assert!(!can_set_group(&tool, &world));
        tool.props.get_mut(&id).unwrap().full = true;
        world.objects.get_mut(index).unwrap().update_flags |= crate::ui::context::flags::AFFECTS_NAVMESH;
        world.main_region = Some(key.region.wrapping_add(1));
        assert!(!property_policy(&tool, &world, &tool.prop_targets(&world)));
        assert_eq!(
            pathfinding_attributes(&world, &[index]),
            aurora_i18n::tr!("scene-object-pathfinding-permanent")
        );
        world.objects.get_mut(index).unwrap().update_flags = 1 << 13;
        assert_eq!(
            pathfinding_attributes(&world, &[index]),
            aurora_i18n::tr!("scene-object-pathfinding-character")
        );
    }
}
