//! Port of LLToolPie::final_click_action / cursorFromObject / handleLeftClickPick
//! (Firestorm indra/newview/lltoolpie.cpp, originally LGPL 2.1).

use crate::world::{World, objects::ObjKey};
use aurora_net::{NetCommand, build::ObjectProps};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Sit,
    Buy,
    Pay,
}

#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub action: Action,
    pub key: ObjKey,
    pub object: Uuid,
    pub root: Uuid,
}

/// Attachments and avatars do not expose these one-click actions. A child's
/// explicit action wins; Touch inherits the root unless the root is Disabled.
pub fn target(world: &World, idx: usize, props: &HashMap<Uuid, ObjectProps>) -> Option<Target> {
    let clicked = world.objects.get(idx)?;
    if clicked.pcode != aurora_prim::params::LL_PCODE_VOLUME {
        return None;
    }
    let mut root_idx = idx;
    for _ in 0..64 {
        let root = world.objects.get(root_idx)?;
        if root.is_avatar() {
            return None;
        }
        if root.parent_id == 0 {
            break;
        }
        root_idx = world.objects.parent_of(root)?;
    }
    let root = world.objects.get(root_idx)?;
    if root.parent_id != 0 {
        return None;
    }
    let code = if root.click_action == 8 || clicked.click_action != 0 {
        clicked.click_action
    } else {
        root.click_action
    };
    let action = match code {
        1 if !world.agent.is_sitting() => Action::Sit,
        2 if props.get(&root.full_id).is_none_or(|p| (1..=3).contains(&p.sale_type)) => Action::Buy,
        3 if (clicked.update_flags | root.update_flags) & (1 << 9) != 0 => Action::Pay,
        _ => return None,
    };
    let object = if action == Action::Buy { root } else { clicked };
    Some(Target {
        action,
        key: object.key,
        object: object.full_id,
        root: root.full_id,
    })
}

pub struct Dialog {
    pub target: Target,
    pub props: Option<ObjectProps>,
    pub amount: String,
    pub prices: Option<(i32, Vec<i32>)>,
    pub error: Option<String>,
    pub opened: Instant,
}

#[derive(Default)]
pub struct Interactions {
    pub props: HashMap<Uuid, ObjectProps>,
    requested: HashMap<Uuid, Instant>,
    pub dialog: Option<Dialog>,
    last_amount: i32,
}

impl Interactions {
    pub fn hover_request(&mut self, target: Target) -> Option<NetCommand> {
        if target.action != Action::Buy {
            return None;
        }
        let now = Instant::now();
        if self
            .requested
            .get(&target.root)
            .is_some_and(|t| now.duration_since(*t) < Duration::from_secs(5))
        {
            return None;
        }
        self.requested.insert(target.root, now);
        Some(NetCommand::RequestObjectProperties {
            handle: target.key.region,
            object: target.root,
        })
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
        let idx = world.objects.index_of(&d.target.key)?;
        if world.objects.get(idx)?.full_id != d.target.object {
            return None;
        }
        let current = target(world, idx, &self.props)?;
        if current.action != d.target.action || current.object != d.target.object || current.root != d.target.root {
            return None;
        }
        let p = d.props.as_ref()?;
        if let Some(latest) = self.props.get(&d.target.root)
            && (latest.owner_id != p.owner_id
                || (d.target.action == Action::Buy && (latest.sale_type != p.sale_type || latest.sale_price != p.sale_price)))
        {
            d.error = Some("L'offre ou le propriétaire a changé. Fermez puis réessayez.".into());
            return None;
        }
        let price = match d.target.action {
            Action::Buy => p.sale_price,
            Action::Pay => amount?,
            Action::Sit => return None,
        };
        if price < 0 || (d.target.action == Action::Pay && (price == 0 || d.prices.is_none())) {
            return None;
        }
        if world.balance.is_none_or(|b| price > b) {
            d.error = Some("Solde insuffisant ou encore inconnu.".into());
            return None;
        }
        let out = if d.target.action == Action::Buy {
            if !(1..=3).contains(&p.sale_type) {
                return None;
            }
            let folder = world
                .inventory
                .folders
                .values()
                .find(|f| !f.library && f.info.type_default == 6)
                .map(|f| f.info.id)
                .unwrap_or(world.inventory.root);
            if folder.is_nil() {
                d.error = Some("Le dossier Objets n'est pas encore disponible.".into());
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
        assert!(target(&w, child_idx, &props).is_none());
        w.objects.get_mut(child_idx).unwrap().click_action = 0;
        w.objects.get_mut(root_idx).unwrap().click_action = 8;
        assert!(target(&w, child_idx, &props).is_none());
        w.objects.get_mut(root_idx).unwrap().parent_id = 9000;
        assert!(target(&w, child_idx, &props).is_none());
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
    fn confirms_original_copy_and_contents_to_the_objects_folder() {
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
            assert!(
                matches!(state.confirm(&w, None), Some(NetCommand::BuyObject { local_id: 971, price: 10, sale_type: ty, folder, .. }) if ty == sale_type && !folder.is_nil())
            );
        }
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
