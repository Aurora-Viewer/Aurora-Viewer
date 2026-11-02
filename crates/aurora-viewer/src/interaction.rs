//! Port of LLToolPie::final_click_action / cursorFromObject / handleLeftClickPick
//! (Firestorm indra/newview/lltoolpie.cpp, originally LGPL 2.1).

use crate::world::{World, objects::ObjKey};
use aurora_net::{
    NetCommand,
    build::{ObjectProps, TaskItem, perm},
};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Touch,
    Grab,
    Sit,
    Buy,
    Pay,
    Open,
    Play,
    OpenMedia,
    Zoom,
    Disabled,
}

/// indra_constants.h: NONE and TOUCH are the same wire value.
pub mod code {
    pub const NONE: u8 = 0;
    pub const TOUCH: u8 = NONE;
    pub const SIT: u8 = 1;
    pub const BUY: u8 = 2;
    pub const PAY: u8 = 3;
    pub const OPEN: u8 = 4;
    pub const PLAY: u8 = 5;
    pub const OPEN_MEDIA: u8 = 6;
    pub const ZOOM: u8 = 7;
    pub const DISABLED: u8 = 8;
    pub const IGNORE: u8 = 9;
}

fn root(world: &World, idx: usize) -> Option<(usize, bool)> {
    let mut current = idx;
    for _ in 0..64 {
        let o = world.objects.get(current)?;
        if o.is_avatar() {
            return None;
        }
        if o.parent_id == 0 {
            return Some((current, false));
        }
        let parent = world.objects.parent_of(o)?;
        if world.objects.get(parent)?.is_avatar() {
            return Some((current, true));
        }
        current = parent;
    }
    None
}

pub fn final_action(world: &World, idx: usize) -> Option<u8> {
    let clicked = world.objects.get(idx)?;
    if clicked.is_client_preview() {
        return Some(code::DISABLED);
    }
    let (root_idx, attached) = root(world, idx)?;
    let root = world.objects.get(root_idx)?;
    if clicked.click_action == code::DISABLED {
        return Some(code::DISABLED);
    }
    if attached {
        return Some(code::TOUCH);
    }
    Some(if root.click_action == code::DISABLED || clicked.click_action != code::TOUCH {
        clicked.click_action
    } else {
        root.click_action
    })
}

fn use_action(world: &World, idx: usize) -> bool {
    let Some((r, attached)) = root(world, idx) else { return false };
    !attached
        && [idx, r].into_iter().any(|i| {
            world
                .objects
                .get(i)
                .is_some_and(|o| o.click_action != code::NONE && o.click_action != code::DISABLED)
        })
}

#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub action: Action,
    pub clicked: ObjKey,
    pub clicked_id: Uuid,
    pub key: ObjKey,
    pub object: Uuid,
    pub root: Uuid,
    /// Chosen from a menu (`menu_target`), not by the click action: the
    /// purchase is rechecked against the menu's rules, whatever the click
    /// action (LLFloaterBuy buys the selection).
    pub menu: bool,
}

/// Attachments and avatars do not expose these one-click actions. A child's
/// explicit action wins; Touch inherits the root unless the root is Disabled.
pub fn target(world: &World, idx: usize, props: &HashMap<Uuid, ObjectProps>) -> Option<Target> {
    let clicked = world.objects.get(idx)?;
    if clicked.pcode != aurora_prim::params::LL_PCODE_VOLUME || clicked.click_action == code::IGNORE {
        return None;
    }
    let (root_idx, _) = root(world, idx)?;
    let root = world.objects.get(root_idx)?;
    let code = final_action(world, idx)?;
    let flags = clicked.update_flags | root.update_flags;
    let action = match code {
        code::SIT if !world.agent.is_sitting() => Action::Sit,
        code::BUY if props.get(&root.full_id).is_none_or(|p| (1..=3).contains(&p.sale_type)) => Action::Buy,
        code::PAY if flags & (1 << 9) != 0 => Action::Pay,
        code::OPEN => Action::Open,
        code::PLAY => Action::Play,
        code::OPEN_MEDIA => Action::OpenMedia,
        code::ZOOM => Action::Zoom,
        // Disabled suppresses touch / inheritance. Firestorm still allows
        // physical grabbing when no one-click action is active.
        code::DISABLED if use_action(world, idx) || flags & 1 == 0 => Action::Disabled,
        _ if flags & 1 != 0 => Action::Grab,
        _ if clicked.click_action != code::DISABLED && flags & (1 << 7) != 0 => Action::Touch,
        _ => return None,
    };
    let object = if matches!(action, Action::Buy | Action::Open) {
        root
    } else {
        clicked
    };
    Some(Target {
        action,
        clicked: clicked.key,
        clicked_id: clicked.full_id,
        key: object.key,
        object: object.full_id,
        root: root.full_id,
        menu: false,
    })
}

/// Target of a right-click menu entry (Pay, Buy, Open), whatever
/// the click action: Pay keeps the clicked prim when it or its root takes
/// money (enable_pay_object, LLFloaterPay::payViaObject); Buy and Open use the root.
pub fn menu_target(world: &World, idx: usize, action: Action) -> Option<Target> {
    let clicked = world.objects.get(idx)?;
    let (root_idx, _) = root(world, idx)?;
    let root = world.objects.get(root_idx)?;
    let object = match action {
        Action::Pay if (clicked.update_flags | root.update_flags) & (1 << 9) != 0 => clicked,
        Action::Pay => return None,
        _ => root,
    };
    Some(Target {
        action,
        clicked: clicked.key,
        clicked_id: clicked.full_id,
        key: object.key,
        object: object.full_id,
        root: root.full_id,
        menu: true,
    })
}

/// LLViewerObject::allowOpen: task inventory must be nonempty and editable.
pub fn allow_open(world: &World, idx: usize) -> bool {
    root(world, idx)
        .and_then(|(i, _)| world.objects.get(i))
        .is_some_and(|o| o.update_flags & (1 << 11) == 0 && o.update_flags & ((1 << 5) | (1 << 2)) != 0)
}

pub fn cursor_action(world: &World, idx: usize, props: &HashMap<Uuid, ObjectProps>) -> Option<Action> {
    let action = target(world, idx, props)?.action;
    if !use_action(world, idx) {
        return (action != Action::Disabled).then_some(action);
    }
    match final_action(world, idx)? {
        code::TOUCH => Some(action),
        code::SIT if action == Action::Sit => Some(action),
        code::BUY if action == Action::Buy => Some(action),
        code::PAY if action == Action::Pay => Some(action),
        code::OPEN if allow_open(world, idx) => Some(action),
        code::PLAY | code::OPEN_MEDIA if world.parcel.is_some() => Some(action),
        code::ZOOM => Some(action),
        _ => None,
    }
}

pub struct Dialog {
    pub target: Target,
    pub props: Option<ObjectProps>,
    pub amount: String,
    pub prices: Option<(i32, Vec<i32>)>,
    pub inventory: Option<Result<Vec<TaskItem>, String>>,
    pub error: Option<String>,
    pub opened: Instant,
}

#[derive(Default)]
pub struct Interactions {
    pub props: HashMap<Uuid, ObjectProps>,
    requested: HashMap<Uuid, Instant>,
    pub dialog: Option<Dialog>,
    pub contents: Option<Contents>,
    pub held: Option<Held>,
    pub media_url: Option<String>,
    last_amount: i32,
    /// L$ missing for the last confirmed payment or purchase: the app opens
    /// Buy L$ for them (`take_shortfall`).
    shortfall: Option<i32>,
}

pub struct Contents {
    pub target: Target,
    pub result: Option<Result<Vec<aurora_net::inventory::InvItem>, String>>,
    pub opened: Instant,
}
pub struct Held {
    pub target: Target,
    pub offset: glam::Vec3,
    pub position: glam::Vec3,
    pub surface: aurora_net::TouchSurface,
    /// The surface was read from the faces the GPU drew (a rigged mesh, a
    /// worn mesh, a worn prim with an action: scene/drawn.rs): the CPU
    /// cannot follow the cursor on it, so the faces drawn under the cursor
    /// are asked for again while it is held (`drawn`, `latest`).
    pub shown: bool,
    /// The request for the faces drawn under the cursor and the cursor it
    /// was made at, while the answer is on its way.
    pub drawn: Option<(u64, (f32, f32))>,
    /// When the faces drawn were last asked for.
    pub asked: Instant,
    /// The surface the newest answer gave and the cursor it was asked at
    /// (the press until the first answer).
    pub latest: (aurora_net::TouchSurface, (f32, f32)),
    pub cursor: (f32, f32),
    pub last: Instant,
    pub physical: bool,
}

impl Interactions {
    /// The L$ a refused confirmation was missing, once.
    pub fn take_shortfall(&mut self) -> Option<i32> {
        self.shortfall.take()
    }

    pub fn on_purchase_inventory(&mut self, object: Uuid, result: &Result<Vec<TaskItem>, String>) {
        if let Some(d) = &mut self.dialog
            && d.target.action == Action::Buy
            && d.target.root == object
        {
            d.inventory = Some(result.clone());
        }
    }
    pub fn on_contents(&mut self, object: Uuid, result: Result<Vec<aurora_net::inventory::InvItem>, String>) {
        if let Some(c) = &mut self.contents
            && c.target.object == object
        {
            c.result = Some(result);
        }
    }
    pub fn hover_request(&mut self, target: Target) -> Option<NetCommand> {
        if !matches!(target.action, Action::Buy | Action::Open) {
            return None;
        }
        self.tip_request(target.key.region, target.root)
    }

    /// ObjectPropertiesFamily of a hovered root (LLSelectMgr::setHoverObject
    /// asks it as the pointer comes on it): at most every 5 s while it has
    /// not answered, every 30 s once it has.
    pub fn tip_request(&mut self, handle: aurora_net::RegionHandle, root: Uuid) -> Option<NetCommand> {
        let now = Instant::now();
        let wait = Duration::from_secs(if self.props.contains_key(&root) { 30 } else { 5 });
        if self.requested.get(&root).is_some_and(|t| now.duration_since(*t) < wait) {
            return None;
        }
        self.requested.insert(root, now);
        Some(NetCommand::RequestObjectProperties { handle, object: root })
    }

    pub fn open(&mut self, target: Target) -> Vec<NetCommand> {
        self.dialog = Some(Dialog {
            target,
            props: None,
            amount: if self.last_amount > 0 {
                self.last_amount.to_string()
            } else {
                String::new()
            },
            prices: None,
            inventory: None,
            error: None,
            opened: Instant::now(),
        });
        let mut out = vec![NetCommand::RequestObjectProperties {
            handle: target.key.region,
            object: target.root,
        }];
        if target.action == Action::Pay {
            out.push(NetCommand::RequestPayPrice {
                handle: target.key.region,
                object: target.object,
            });
        } else if target.action == Action::Buy {
            out.push(NetCommand::RequestTaskInventory {
                handle: target.key.region,
                local_id: target.key.local_id,
                object: target.root,
            });
        }
        out
    }

    pub fn on_properties(&mut self, props: &[ObjectProps]) {
        for p in props {
            self.props.insert(p.object_id, p.clone());
            if let Some(d) = &mut self.dialog
                && d.target.root == p.object_id
                && d.props.is_none()
            {
                d.props = Some(p.clone());
            }
        }
    }

    pub fn on_prices(&mut self, object: Uuid, default: i32, buttons: Vec<i32>) {
        if let Some(d) = &mut self.dialog
            && d.target.action == Action::Pay
            && d.target.object == object
        {
            // llSetPayPrice: -1 hides the entry, -2 keeps its default; other
            // values select their absolute value (LLFloaterPay::processPayPriceReply).
            if default != -1 && default != -2 {
                d.amount = default.saturating_abs().to_string();
            }
            d.prices = Some((default, buttons.into_iter().take(4).collect()));
        }
    }

    /// Recheck object identity/flags and sale before producing a transaction.
    /// ObjectBuy carries the confirmed sale info; the simulator rejects a changed price.
    pub fn confirm(&mut self, world: &World, amount: Option<i32>) -> Option<NetCommand> {
        let d = self.dialog.as_mut()?;
        let idx = world.objects.index_of(&d.target.clicked)?;
        if world.objects.get(idx)?.full_id != d.target.clicked_id {
            return None;
        }
        // LLFloaterPay::give pays the selected prim regardless of its normal
        // click action, and LLFloaterBuy buys the selection: a payment, or a
        // purchase chosen from a menu, rechecks its eligibility itself.
        let current = if d.target.action == Action::Pay || d.target.menu {
            menu_target(world, idx, d.target.action)?
        } else {
            target(world, idx, &self.props)?
        };
        if current.action != d.target.action || current.object != d.target.object || current.root != d.target.root {
            return None;
        }
        let p = d.props.as_ref()?;
        if let Some(latest) = self.props.get(&d.target.root)
            && (latest.owner_id != p.owner_id
                || (d.target.action == Action::Buy && (latest.sale_type != p.sale_type || latest.sale_price != p.sale_price)))
        {
            d.error = Some(aurora_i18n::tr!("interaction-the-offer-or-owner-has-changed-close-and-try").into());
            return None;
        }
        let price = match d.target.action {
            Action::Buy => p.sale_price,
            Action::Pay => amount?,
            _ => return None,
        };
        if price < 0 || (d.target.action == Action::Pay && (price == 0 || d.prices.is_none())) {
            return None;
        }
        // give_money / can_afford_transaction: short of L$, the Buy L$ window
        // opens for what is missing (an unknown balance counts as 0, like
        // the status bar's); the dialog stays open to try again
        let balance = world.balance.unwrap_or(0);
        if price > balance {
            self.shortfall = Some(price - balance.max(0));
            return None;
        }
        let out = if d.target.action == Action::Buy {
            if !(1..=3).contains(&p.sale_type) {
                return None;
            }
            if p.sale_type == 3
                && !d.inventory.as_ref().is_some_and(|r| {
                    r.as_ref()
                        .is_ok_and(|items| items.iter().any(|item| purchase_item(item, world.agent_id, 3)))
                })
            {
                return None;
            }
            let folder = if p.sale_type == 3 {
                world.inventory.root
            } else {
                world
                    .inventory
                    .folders
                    .values()
                    .find(|f| !f.library && f.info.type_default == 6)
                    .map(|f| f.info.id)
                    .unwrap_or(world.inventory.root)
            };
            if folder.is_nil() {
                d.error = Some(aurora_i18n::tr!("interaction-the-objects-folder-is-not-available-yet").into());
                return None;
            }
            NetCommand::BuyObject {
                handle: d.target.key.region,
                local_id: d.target.key.local_id,
                folder,
                sale_type: p.sale_type,
                price,
            }
        } else {
            let (default, buttons) = d.prices.as_ref()?;
            if *default == -1 && !buttons.contains(&price) {
                return None;
            }
            self.last_amount = price;
            NetCommand::PayObject {
                handle: d.target.key.region,
                object: d.target.object,
                amount: price,
                description: p.name.clone(),
            }
        };
        self.dialog = None;
        Some(out)
    }
}

/// LLFloaterBuy / LLFloaterBuyContents::inventoryChanged: only transferable
/// items are delivered; a contents sale also requires the seller to copy them.
pub fn purchase_item(item: &TaskItem, buyer: Uuid, sale_type: u8) -> bool {
    !item.is_folder
        && item.asset_type >= 0
        && ((!item.group_owned && item.owner_id == buyer) || item.owner_mask & perm::TRANSFER != 0)
        && (sale_type != 3 || item.owner_mask & perm::COPY != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events() {
            w.apply(ev);
        }
        for ev in crate::demo::action_events() {
            w.apply(ev);
        }
        w
    }
    #[test]
    fn inheritance_overrides_disabled_and_attachments() {
        let mut w = world();
        let root_idx = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child_idx = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        let props = HashMap::new();
        assert_eq!(target(&w, child_idx, &props).unwrap().action, Action::Buy);
        w.objects.get_mut(child_idx).unwrap().click_action = 3;
        w.objects.get_mut(child_idx).unwrap().update_flags = 1 << 9;
        assert_eq!(target(&w, child_idx, &props).unwrap().action, Action::Pay);
        w.objects.get_mut(child_idx).unwrap().click_action = 8;
        assert_eq!(target(&w, child_idx, &props).unwrap().action, Action::Disabled);
        w.objects.get_mut(child_idx).unwrap().click_action = 0;
        w.objects.get_mut(root_idx).unwrap().click_action = 8;
        assert!(target(&w, child_idx, &props).is_none());
        w.objects.get_mut(root_idx).unwrap().parent_id = 9000;
        assert!(target(&w, child_idx, &props).is_none());
    }
    #[test]
    fn world_click_other_residents_attachment_needs_touch_but_no_edit_rights() {
        let mut w = world();
        let (wearer, owner) = w
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != w.agent_id)
            .map(|(_, o)| (o.key.local_id, o.full_id))
            .expect("another demo resident");
        let root_idx = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child_idx = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        let root = w.objects.get_mut(root_idx).unwrap();
        root.parent_id = wearer;
        root.owner_id = owner;
        root.update_flags = 1 << 7;
        let props = HashMap::new();
        for idx in [root_idx, child_idx] {
            let picked = w.objects.get(idx).unwrap();
            let result = target(&w, idx, &props).expect("scripted attachment");
            assert_eq!(result.action, Action::Touch);
            assert_eq!(result.clicked, picked.key);
            assert_eq!(result.key, picked.key, "ObjectGrab and ObjectDeGrab address the clicked prim");
            assert_eq!(result.root, crate::demo::action_id(971));
            assert!(!crate::build::selectable(&w, idx));
        }
        w.objects.get_mut(root_idx).unwrap().update_flags = 0;
        assert!(target(&w, child_idx, &props).is_none());
    }
    #[test]
    fn every_wire_action_and_default_touch_physics_and_open_permissions() {
        let mut w = world();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let props = HashMap::new();
        w.objects.get_mut(idx).unwrap().update_flags = (1 << 7) | (1 << 9) | (1 << 5);
        for (code, expected) in [
            (0, Action::Touch),
            (1, Action::Sit),
            (2, Action::Buy),
            (3, Action::Pay),
            (4, Action::Open),
            (5, Action::Play),
            (6, Action::OpenMedia),
            (7, Action::Zoom),
            (8, Action::Disabled),
        ] {
            w.objects.get_mut(idx).unwrap().click_action = code;
            assert_eq!(target(&w, idx, &props).unwrap().action, expected);
            assert_eq!(cursor_action(&w, idx, &props), (code != 8).then_some(expected));
        }
        w.objects.get_mut(idx).unwrap().click_action = code::IGNORE;
        assert!(target(&w, idx, &props).is_none());
        w.objects.get_mut(idx).unwrap().click_action = code::NONE;
        w.objects.get_mut(idx).unwrap().update_flags = 1;
        assert_eq!(target(&w, idx, &props).unwrap().action, Action::Grab);
        w.objects.get_mut(idx).unwrap().click_action = code::DISABLED;
        assert_eq!(target(&w, idx, &props).unwrap().action, Action::Grab);
        assert_eq!(cursor_action(&w, idx, &props), Some(Action::Grab));
        w.objects.get_mut(idx).unwrap().click_action = code::NONE;
        w.objects.get_mut(idx).unwrap().update_flags = 0;
        assert!(target(&w, idx, &props).is_none());
        w.objects.get_mut(idx).unwrap().click_action = code::OPEN;
        assert_eq!(target(&w, idx, &props).unwrap().action, Action::Open);
        assert!(cursor_action(&w, idx, &props).is_none());
        w.objects.get_mut(idx).unwrap().update_flags = (1 << 5) | (1 << 11);
        assert!(!allow_open(&w, idx));
        w.objects.get_mut(idx).unwrap().update_flags = 1 << 2;
        assert!(allow_open(&w, idx));
        w.objects.get_mut(idx).unwrap().click_action = code::PLAY;
        w.parcel = None;
        assert!(cursor_action(&w, idx, &props).is_none());
    }
    #[test]
    fn disabled_root_does_not_disable_children_and_ignore_is_per_prim() {
        let mut w = world();
        let root = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        let props = HashMap::new();
        w.objects.get_mut(root).unwrap().click_action = code::DISABLED;
        w.objects.get_mut(child).unwrap().update_flags = 1 << 7;
        assert_eq!(target(&w, child, &props).unwrap().action, Action::Touch);
        w.objects.get_mut(root).unwrap().click_action = code::IGNORE;
        assert_eq!(target(&w, child, &props).unwrap().action, Action::Touch);
        assert!(cursor_action(&w, child, &props).is_none());
        w.objects.get_mut(root).unwrap().click_action = code::BUY;
        w.objects.get_mut(child).unwrap().click_action = code::IGNORE;
        assert!(target(&w, child, &props).is_none());
        w.objects.get_mut(root).unwrap().parent_id = 9000;
        w.objects.get_mut(child).unwrap().click_action = code::SIT;
        assert_eq!(target(&w, child, &props).unwrap().action, Action::Touch);
    }
    #[test]
    fn no_sale_no_money_and_price_reply_for_another_object() {
        let mut w = world();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(972)).unwrap();
        let mut state = Interactions::default();
        let t = target(&w, idx, &state.props).unwrap();
        state.open(t);
        state.on_prices(Uuid::nil(), 20, vec![20]);
        assert!(state.dialog.as_ref().unwrap().prices.is_none());
        w.objects.get_mut(idx).unwrap().update_flags = 0;
        assert!(target(&w, idx, &state.props).is_none());
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        state.on_properties(&[ObjectProps {
            object_id: crate::demo::action_id(971),
            sale_type: 0,
            ..Default::default()
        }]);
        assert!(target(&w, idx, &state.props).is_none());
    }
    #[test]
    fn confirmation_is_required_and_sends_the_clicked_prim() {
        let mut w = world();
        w.balance = Some(100);
        let mut state = Interactions::default();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(972)).unwrap();
        let t = target(&w, idx, &state.props).unwrap();
        let requests = state.open(t);
        assert!(
            requests
                .iter()
                .all(|c| matches!(c, NetCommand::RequestPayPrice { .. } | NetCommand::RequestObjectProperties { .. }))
        );
        assert!(state.confirm(&w, Some(20)).is_none());
        state.on_properties(&[ObjectProps {
            object_id: t.root,
            name: "Test".into(),
            ..Default::default()
        }]);
        state.on_prices(t.object, -1, vec![20]);
        assert!(state.confirm(&w, Some(30)).is_none());
        assert!(state.confirm(&w, Some(200)).is_none());
        assert!(matches!(state.confirm(&w, Some(20)), Some(NetCommand::PayObject { object, amount: 20, .. }) if object == t.object));
        assert!(state.confirm(&w, Some(20)).is_none());
    }
    #[test]
    fn menu_payments_do_not_require_a_pay_click_action() {
        let mut w = world();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(972)).unwrap();
        for action in 0..=9 {
            w.objects.get_mut(idx).unwrap().click_action = action;
            let mut state = Interactions::default();
            let t = menu_target(&w, idx, Action::Pay).expect("the object takes money");
            state.open(t);
            state.on_properties(&[ObjectProps {
                object_id: t.root,
                name: "Tip jar".into(),
                ..Default::default()
            }]);
            state.on_prices(t.object, 10, vec![1, 5, 10, 20]);
            assert!(
                matches!(state.confirm(&w, Some(10)), Some(NetCommand::PayObject { object, amount: 10, .. }) if object == t.clicked_id),
                "context-menu payment with click action {action}"
            );
            assert!(state.dialog.is_none());
            assert!(state.confirm(&w, Some(10)).is_none(), "a payment cannot be sent twice");
        }
    }
    #[test]
    fn linked_menu_payments_query_and_pay_the_clicked_prim() {
        let mut w = world();
        let root = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        for (root_takes_money, child_takes_money) in [(true, false), (false, true), (true, true)] {
            w.objects.get_mut(root).unwrap().update_flags = u32::from(root_takes_money) << 9;
            w.objects.get_mut(child).unwrap().update_flags = u32::from(child_takes_money) << 9;
            let mut state = Interactions::default();
            let t = menu_target(&w, child, Action::Pay).unwrap();
            assert_eq!(t.object, crate::demo::action_id(974));
            let requests = state.open(t);
            assert!(
                requests
                    .iter()
                    .any(|c| matches!(c, NetCommand::RequestPayPrice { object, .. } if *object == t.clicked_id))
            );
            assert!(
                requests
                    .iter()
                    .any(|c| matches!(c, NetCommand::RequestObjectProperties { object, .. } if *object == t.root))
            );
            state.on_properties(&[ObjectProps {
                object_id: t.root,
                name: "Linked tip jar".into(),
                ..Default::default()
            }]);
            state.on_prices(t.root, 10, vec![10]);
            assert!(state.confirm(&w, Some(10)).is_none(), "only the clicked prim's prices apply");
            state.on_prices(t.object, -1, vec![10]);
            assert!(
                matches!(state.confirm(&w, Some(10)), Some(NetCommand::PayObject { object, handle, amount: 10, description })
                if object == t.clicked_id && handle == t.clicked.region && description == "Linked tip jar")
            );
        }
        w.objects.get_mut(root).unwrap().update_flags = 0;
        w.objects.get_mut(child).unwrap().update_flags = 0;
        assert!(menu_target(&w, child, Action::Pay).is_none());
    }
    #[test]
    fn menu_payments_recheck_identity_money_flags_and_owner() {
        let mut w = world();
        let root = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        for change in ["identity", "root", "money", "owner", "balance"] {
            let original = w.objects.get(child).unwrap();
            let (original_id, original_parent) = (original.full_id, original.parent_id);
            let root_flags = w.objects.get(root).unwrap().update_flags;
            w.objects.get_mut(root).unwrap().update_flags |= 1 << 9;
            let mut state = Interactions::default();
            let t = menu_target(&w, child, Action::Pay).unwrap();
            state.open(t);
            let mut props = ObjectProps {
                object_id: t.root,
                owner_id: Uuid::from_u128(1),
                ..Default::default()
            };
            state.on_properties(&[props.clone()]);
            state.on_prices(t.object, 10, vec![10]);
            match change {
                "identity" => w.objects.get_mut(child).unwrap().full_id = Uuid::nil(),
                "root" => w.objects.get_mut(child).unwrap().parent_id = 972,
                "money" => w.objects.get_mut(root).unwrap().update_flags &= !(1 << 9),
                "owner" => {
                    props.owner_id = Uuid::from_u128(2);
                    state.on_properties(&[props]);
                }
                "balance" => w.balance = Some(5),
                _ => unreachable!(),
            }
            assert!(state.confirm(&w, Some(10)).is_none(), "payment must reject changed {change}");
            assert!(state.dialog.is_some());
            w.objects.get_mut(child).unwrap().full_id = original_id;
            w.objects.get_mut(child).unwrap().parent_id = original_parent;
            w.objects.get_mut(root).unwrap().update_flags = root_flags;
        }
    }
    #[test]
    fn buy_root_quote_cannot_change_after_opening() {
        let w = world();
        let mut state = Interactions::default();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        let t = target(&w, idx, &state.props).unwrap();
        assert_eq!(t.object, crate::demo::action_id(971));
        state.open(t);
        let mut p = ObjectProps {
            object_id: t.root,
            sale_type: 2,
            sale_price: 10,
            ..Default::default()
        };
        state.on_properties(&[p.clone()]);
        p.sale_price = 100;
        state.on_properties(&[p]);
        assert_eq!(state.dialog.as_ref().unwrap().props.as_ref().unwrap().sale_price, 10);
        assert!(state.confirm(&w, None).is_none());
        assert!(state.dialog.as_ref().unwrap().error.is_some());
    }
    #[test]
    fn buy_on_a_child_overrides_a_different_root_action_and_rechecks_the_child() {
        let mut w = world();
        let root = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        w.objects.get_mut(root).unwrap().click_action = code::NONE;
        w.objects.get_mut(child).unwrap().click_action = code::BUY;
        let mut state = Interactions::default();
        let t = target(&w, child, &state.props).unwrap();
        state.open(t);
        state.on_properties(&[ObjectProps {
            object_id: t.root,
            sale_type: 2,
            sale_price: 10,
            ..Default::default()
        }]);
        assert!(matches!(state.confirm(&w, None), Some(NetCommand::BuyObject { local_id: 971, .. })));
        state.open(t);
        state.on_properties(&[ObjectProps {
            object_id: t.root,
            sale_type: 2,
            sale_price: 10,
            ..Default::default()
        }]);
        w.objects.get_mut(child).unwrap().click_action = code::DISABLED;
        assert!(state.confirm(&w, None).is_none());
    }
    /// LLFloaterBuy buys the selection: Buy from the menu, the hover tip or
    /// Area Search completes whatever the click action (a boxed product
    /// keeps the default Touch), still rechecking identity and sale.
    #[test]
    fn menu_purchases_do_not_require_a_buy_click_action() {
        let mut w = world();
        let root = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        for action in [code::TOUCH, code::SIT, code::PAY, code::OPEN, code::DISABLED] {
            w.objects.get_mut(root).unwrap().click_action = action;
            w.objects.get_mut(child).unwrap().click_action = action;
            let mut state = Interactions::default();
            let t = menu_target(&w, child, Action::Buy).unwrap();
            assert_eq!(t.object, crate::demo::action_id(971));
            state.open(t);
            let props = ObjectProps {
                object_id: t.root,
                sale_type: 2,
                sale_price: 10,
                ..Default::default()
            };
            state.on_properties(std::slice::from_ref(&props));
            assert!(
                matches!(
                    state.confirm(&w, None),
                    Some(NetCommand::BuyObject {
                        local_id: 971,
                        price: 10,
                        ..
                    })
                ),
                "menu purchase with click action {action}"
            );
            // the price changed since the dialog opened
            state.open(t);
            state.on_properties(std::slice::from_ref(&props));
            state.on_properties(&[ObjectProps { sale_price: 20, ..props }]);
            assert!(state.confirm(&w, None).is_none());
        }
    }

    /// give_money / can_afford_transaction: short of L$, nothing is sent
    /// and the missing amount is handed to Buy L$, once.
    #[test]
    fn a_short_balance_asks_for_the_missing_l() {
        let mut w = world();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        for (balance, missing) in [(Some(4), 6), (Some(-3), 10), (None, 10)] {
            w.balance = balance;
            let mut state = Interactions::default();
            let t = target(&w, idx, &state.props).unwrap();
            state.open(t);
            state.on_properties(&[ObjectProps {
                object_id: t.root,
                sale_type: 2,
                sale_price: 10,
                ..Default::default()
            }]);
            assert!(state.confirm(&w, None).is_none());
            assert_eq!(state.take_shortfall(), Some(missing), "balance {balance:?}");
            assert_eq!(state.take_shortfall(), None);
            assert!(state.dialog.is_some());
            w.balance = Some(10);
            assert!(matches!(state.confirm(&w, None), Some(NetCommand::BuyObject { price: 10, .. })));
            assert_eq!(state.take_shortfall(), None);
        }
    }

    #[test]
    fn confirms_original_copy_to_objects_and_contents_to_inventory_root() {
        let w = world();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        for sale_type in 1..=3 {
            let mut state = Interactions::default();
            let t = target(&w, idx, &state.props).unwrap();
            state.open(t);
            state.on_properties(&[ObjectProps {
                object_id: t.root,
                sale_type,
                sale_price: 10,
                ..Default::default()
            }]);
            if sale_type == 3 {
                assert!(state.confirm(&w, None).is_none());
                state.on_purchase_inventory(
                    t.root,
                    &Ok(vec![TaskItem {
                        asset_type: 6,
                        owner_mask: perm::COPY | perm::TRANSFER,
                        ..Default::default()
                    }]),
                );
            }
            let expected_folder = if sale_type == 3 {
                w.inventory.root
            } else {
                w.inventory
                    .folders
                    .values()
                    .find(|f| !f.library && f.info.type_default == 6)
                    .map(|f| f.info.id)
                    .unwrap_or(w.inventory.root)
            };
            assert!(
                matches!(state.confirm(&w, None), Some(NetCommand::BuyObject { local_id: 971, price: 10, sale_type: ty, folder, .. }) if ty == sale_type && folder == expected_folder)
            );
        }
    }

    #[test]
    fn purchase_preview_filters_items_by_delivery_permissions() {
        let buyer = Uuid::from_u128(1);
        let mut item = TaskItem {
            asset_type: 7,
            owner_id: Uuid::from_u128(2),
            owner_mask: perm::TRANSFER,
            ..Default::default()
        };
        assert!(purchase_item(&item, buyer, 1));
        assert!(purchase_item(&item, buyer, 2));
        assert!(!purchase_item(&item, buyer, 3));
        item.owner_mask |= perm::COPY;
        assert!(purchase_item(&item, buyer, 3));
        item.owner_mask = perm::COPY;
        assert!(!purchase_item(&item, buyer, 2));
        item.owner_id = buyer;
        assert!(purchase_item(&item, buyer, 3));
        item.group_owned = true;
        assert!(!purchase_item(&item, buyer, 3));
        item.owner_mask |= perm::TRANSFER;
        item.is_folder = true;
        assert!(!purchase_item(&item, buyer, 3));
        item.is_folder = false;
        item.asset_type = -1;
        assert!(!purchase_item(&item, buyer, 2));
    }

    #[test]
    fn purchase_inventory_is_requested_on_the_root_and_empty_contents_cannot_be_bought() {
        let w = world();
        let mut state = Interactions::default();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        let t = target(&w, idx, &state.props).unwrap();
        assert!(
            state
                .open(t)
                .iter()
                .any(|cmd| matches!(cmd, NetCommand::RequestTaskInventory {object, local_id: 971, ..} if *object == t.root))
        );
        state.on_purchase_inventory(t.clicked_id, &Ok(Vec::new()));
        assert!(state.dialog.as_ref().unwrap().inventory.is_none());
        state.on_properties(&[ObjectProps {
            object_id: t.root,
            sale_type: 3,
            sale_price: 10,
            ..Default::default()
        }]);
        state.on_purchase_inventory(t.root, &Ok(Vec::new()));
        assert!(state.confirm(&w, None).is_none());
        state.on_purchase_inventory(t.root, &Err(aurora_i18n::tr!("ui-media-unavailable").into()));
        assert!(state.confirm(&w, None).is_none());
    }
    #[test]
    fn sit_disappears_while_seated_and_full_updates_keep_the_action() {
        let mut w = world();
        let idx = w.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let props = HashMap::new();
        assert_eq!(target(&w, idx, &props).unwrap().action, Action::Sit);
        w.agent
            .on_server_update(glam::Vec3::ZERO, glam::Vec3::ZERO, glam::Vec3::ZERO, true, true);
        assert!(target(&w, idx, &props).is_none());
        for ev in crate::demo::action_events() {
            w.apply(ev);
        }
        assert_eq!(w.objects.get(idx).unwrap().click_action, 1);
    }
}
