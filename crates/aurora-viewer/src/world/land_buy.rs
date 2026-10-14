//! The rules of a land purchase: whether the selected land can be bought,
//! by whom, for how much, and what it will cost in tier and L$. Port of
//! LLFloaterBuyLandUI::updateParcelInfo and the figures of its refreshUI
//! (indra/newview/llfloaterbuyland.cpp, llparcelselection.cpp, originally
//! LGPL 2.1).

use super::land::powers;
use aurora_net::{GroupMembership, ParcelInfo, parcel_flags as pf};
use uuid::Uuid;

/// What a group may hold for each m² contributed (GROUP_LAND_BONUS_FACTOR).
pub const GROUP_LAND_BONUS_FACTOR: f32 = 1.1;
/// One parcel grid square, m² (PARCEL_GRID_STEP_METERS²).
const PARCEL_UNIT_AREA: i32 = 16;

/// Why the land cannot be bought (the floater's strings of the same names).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NoLandSelected,
    MultipleParcelsSelected,
    CantBuyForGroup,
    ParcelNotForSale,
    GroupAlreadyOwns,
    YouAlreadyOwn,
    SetToSellToOther,
    NoPublicLand,
    NotOwnedByYou,
}

impl Refusal {
    pub fn text(self) -> &'static str {
        match self {
            Refusal::NoLandSelected => aurora_i18n::tr!("buy-land-no-land-selected"),
            Refusal::MultipleParcelsSelected => aurora_i18n::tr!("buy-land-multiple-parcels-selected"),
            Refusal::CantBuyForGroup => aurora_i18n::tr!("buy-land-cant-buy-for-group"),
            Refusal::ParcelNotForSale => aurora_i18n::tr!("buy-land-parcel-not-for-sale"),
            Refusal::GroupAlreadyOwns => aurora_i18n::tr!("buy-land-group-already-owns"),
            Refusal::YouAlreadyOwn => aurora_i18n::tr!("buy-land-you-already-own"),
            Refusal::SetToSellToOther => aurora_i18n::tr!("buy-land-set-to-sell-to-other"),
            Refusal::NoPublicLand => aurora_i18n::tr!("buy-land-no-public-land"),
            Refusal::NotOwnedByYou => aurora_i18n::tr!("buy-land-not-owned-by-you"),
        }
    }
}

/// What the purchase needs of the selection's region.
#[derive(Debug, Clone, Copy)]
pub struct Region<'a> {
    pub name: &'a str,
    /// LLViewerRegion::getBillableFactor.
    pub billable_factor: f32,
    /// LLViewerRegion::getMaxTasks (0: not known yet).
    pub max_tasks: u32,
}

/// Who buys: the agent, or the agent's active group.
#[derive(Debug, Clone, Copy)]
pub struct Buyer<'a> {
    pub id: Uuid,
    pub groups: &'a [GroupMembership],
    pub active_group: Uuid,
    pub for_group: bool,
}

impl Buyer<'_> {
    fn has_power(&self, group: &Uuid, power: u64) -> bool {
        self.groups.iter().any(|g| g.id == *group && g.powers & power != 0)
    }

    /// LLAgent::hasPowerInActiveGroup.
    pub fn has_power_in_active_group(&self, power: u64) -> bool {
        !self.active_group.is_nil() && self.has_power(&self.active_group, power)
    }

    /// LLAgent::getGroupContribution: m² the agent gives the group.
    fn contribution(&self, group: &Uuid) -> i32 {
        self.groups.iter().find(|g| g.id == *group).map_or(0, |g| g.contribution)
    }
}

/// The land on offer, as the floater shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Offer {
    /// A parcel and its region are known (mParcelValid).
    pub valid: bool,
    /// Public land: claimed, not bought from someone.
    pub is_claim: bool,
    pub for_sale: bool,
    pub is_group_land: bool,
    /// m² the agent contributes to the selling group.
    pub group_contribution: i32,
    pub price: i32,
    pub actual_area: i32,
    /// What the area counts for the tier (premium or discounted land).
    pub billable_area: i32,
    pub supported_objects: i32,
    pub sold_with_objects: bool,
    /// "Region x,y" of the parcel's center.
    pub location: String,
    pub refusal: Option<Refusal>,
}

impl Offer {
    pub fn can_buy(&self) -> bool {
        self.valid && self.refusal.is_none()
    }

    /// The L$ price per m², one decimal in the floater.
    pub fn price_per_m2(&self) -> f32 {
        if self.actual_area > 0 {
            self.price as f32 / self.actual_area as f32
        } else {
            0.0
        }
    }
}

/// ll_round: half up.
fn round(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

/// LLFloaterBuyLandUI::updateParcelInfo. `multiple`: the selection covers
/// several parcels (PARCEL_RESULT_MULTIPLE).
pub fn assess(parcel: Option<&ParcelInfo>, multiple: bool, region: Option<&Region>, buyer: &Buyer) -> Offer {
    let mut o = Offer::default();
    let (Some(p), Some(region)) = (parcel, region) else {
        o.refusal = Some(Refusal::NoLandSelected);
        return o;
    };
    o.valid = true;
    if multiple {
        o.refusal = Some(Refusal::MultipleParcelsSelected);
        return o;
    }
    // LLParcel::isPublic
    o.is_claim = p.owner_id.is_nil();
    if o.is_claim {
        // LLParcelSelection::getClaimableArea
        o.actual_area = p.public_count * PARCEL_UNIT_AREA;
        o.for_sale = true;
        o.price = o.actual_area.saturating_mul(p.claim_price);
    } else {
        o.actual_area = p.area;
        o.for_sale = p.flags & pf::FOR_SALE != 0;
        o.is_group_land = p.is_group_owned;
        o.price = if o.for_sale { p.sale_price } else { 0 };
        if o.is_group_land {
            o.group_contribution = buyer.contribution(&p.group_id);
        }
    }
    o.billable_area = round(region.billable_factor * o.actual_area as f32);
    o.supported_objects = round(p.max_prims as f32 * p.prim_bonus);
    // never more than the region's limit, whatever the parcel's bonus
    if region.max_tasks > 0 {
        o.supported_objects = o.supported_objects.min(i32::try_from(region.max_tasks).unwrap_or(i32::MAX));
    }
    o.sold_with_objects = p.flags & pf::SELL_PARCEL_OBJECTS != 0;
    // LLParcel::getCenterpoint
    let cx = (p.aabb_min.x + p.aabb_max.x) * 0.5;
    let cy = (p.aabb_min.y + p.aabb_max.y) * 0.5;
    o.location = format!("{} {},{}", region.name, cx as i32, cy as i32);

    if buyer.for_group && !buyer.has_power_in_active_group(powers::LAND_DEED) {
        o.refusal = Some(Refusal::CantBuyForGroup);
        return o;
    }
    if !o.is_claim {
        let new_owner = if buyer.for_group { buyer.active_group } else { buyer.id };
        if !o.for_sale || (o.price == 0 && p.auth_buyer.is_nil()) {
            o.refusal = Some(Refusal::ParcelNotForSale);
        } else if p.owner_id == new_owner {
            o.refusal = Some(if buyer.for_group {
                Refusal::GroupAlreadyOwns
            } else {
                Refusal::YouAlreadyOwn
            });
        } else if !p.auth_buyer.is_nil() && p.auth_buyer != buyer.id {
            // maybe the parcel is set for sale to a group we are an officer of
            let authorized_group =
                buyer.has_power(&p.auth_buyer, powers::LAND_DEED) && buyer.has_power(&p.auth_buyer, powers::LAND_SET_SALE_INFO);
            if !authorized_group {
                o.refusal = Some(Refusal::SetToSellToOther);
            }
        }
    } else if o.actual_area == 0 {
        o.refusal = Some(Refusal::NoPublicLand);
    } else if p.other_count != 0 {
        // policy: someone else's land must not be selected
        o.refusal = Some(Refusal::NotOwnedByYou);
    }
    o
}

/// The L$ side of the purchase (refreshUI's section three).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Funds {
    pub balance: i32,
    /// L$ bought with the land.
    pub currency_buy: i32,
    pub price: i32,
}

impl Funds {
    pub fn have_enough(&self) -> bool {
        self.balance >= self.price
    }

    /// What the balance will be once the L$ are bought and the land paid.
    pub fn final_balance(&self) -> i64 {
        i64::from(self.balance) + i64::from(self.currency_buy) - i64::from(self.price)
    }

    pub fn will_have_enough(&self) -> bool {
        self.final_balance() >= 0
    }

    /// L$ missing without buying any (the amount proposed to buy).
    pub fn shortfall(&self) -> i32 {
        self.price.saturating_sub(self.balance).max(0)
    }
}

/// m² of contribution that cover the land for a group seller: the
/// "Remove [AMOUNT] m² of contribution" offered.
pub fn min_contribution(billable_area: i32) -> i32 {
    (billable_area as f32 / GROUP_LAND_BONUS_FACTOR).ceil() as i32
}

/// The land use line: how the parcel counts against the tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaNote {
    NoParcel,
    /// Charged at its area.
    Plain(i32),
    /// Charged as more than its area.
    Premium(i32),
    /// Charged as less.
    Discounted(i32),
}

pub fn area_note(o: &Offer) -> AreaNote {
    if !o.valid {
        AreaNote::NoParcel
    } else if o.billable_area == o.actual_area {
        AreaNote::Plain(o.actual_area)
    } else if o.billable_area > o.actual_area {
        AreaNote::Premium(o.billable_area)
    } else {
        AreaNote::Discounted(o.billable_area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Uuid = Uuid::from_u128(1);
    const SELLER: Uuid = Uuid::from_u128(2);
    const GROUP: Uuid = Uuid::from_u128(3);
    const OTHER: Uuid = Uuid::from_u128(4);

    fn region() -> Region<'static> {
        Region {
            name: "Aurora",
            billable_factor: 1.0,
            max_tasks: 15000,
        }
    }

    fn for_sale(price: i32) -> ParcelInfo {
        ParcelInfo {
            owner_id: SELLER,
            area: 1024,
            flags: pf::FOR_SALE | pf::SELL_PARCEL_OBJECTS,
            sale_price: price,
            max_prims: 234,
            prim_bonus: 1.0,
            aabb_min: glam::Vec3::new(128.0, 192.0, 0.0),
            aabb_max: glam::Vec3::new(160.0, 224.0, 0.0),
            ..Default::default()
        }
    }

    fn me(groups: &[GroupMembership]) -> Buyer<'_> {
        Buyer {
            id: ME,
            groups,
            active_group: GROUP,
            for_group: false,
        }
    }

    fn officer(powers: u64) -> GroupMembership {
        GroupMembership {
            id: GROUP,
            name: "Group".into(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 600,
        }
    }

    #[test]
    fn parcel_for_sale() {
        let o = assess(Some(&for_sale(4500)), false, Some(&region()), &me(&[]));
        assert!(o.can_buy());
        assert_eq!(
            (o.price, o.actual_area, o.billable_area, o.supported_objects),
            (4500, 1024, 1024, 234)
        );
        assert!(o.sold_with_objects);
        assert_eq!(o.location, "Aurora 144,208");
        assert!((o.price_per_m2() - 4.39).abs() < 0.01);
    }

    #[test]
    fn refusals_in_firestorms_order() {
        let r = |p: Option<&ParcelInfo>, multiple: bool, b: &Buyer| assess(p, multiple, Some(&region()), b).refusal;
        let none: [GroupMembership; 0] = [];
        assert_eq!(r(None, false, &me(&none)), Some(Refusal::NoLandSelected));
        assert_eq!(
            assess(Some(&for_sale(10)), false, None, &me(&none)).refusal,
            Some(Refusal::NoLandSelected)
        );
        assert_eq!(r(Some(&for_sale(10)), true, &me(&none)), Some(Refusal::MultipleParcelsSelected));
        let mut not_for_sale = for_sale(10);
        not_for_sale.flags = 0;
        assert_eq!(r(Some(&not_for_sale), false, &me(&none)), Some(Refusal::ParcelNotForSale));
        // free, but to nobody in particular: not for sale
        assert_eq!(r(Some(&for_sale(0)), false, &me(&none)), Some(Refusal::ParcelNotForSale));
        let mut mine = for_sale(10);
        mine.owner_id = ME;
        assert_eq!(r(Some(&mine), false, &me(&none)), Some(Refusal::YouAlreadyOwn));
        let mut to_other = for_sale(10);
        to_other.auth_buyer = OTHER;
        assert_eq!(r(Some(&to_other), false, &me(&none)), Some(Refusal::SetToSellToOther));
        // free to us alone
        let mut gift = for_sale(0);
        gift.auth_buyer = ME;
        assert_eq!(r(Some(&gift), false, &me(&none)), None);
    }

    #[test]
    fn sale_to_a_group_we_officer() {
        let mut p = for_sale(10);
        p.auth_buyer = GROUP;
        let deed_only = [officer(powers::LAND_DEED)];
        assert_eq!(
            assess(Some(&p), false, Some(&region()), &me(&deed_only)).refusal,
            Some(Refusal::SetToSellToOther)
        );
        let both = [officer(powers::LAND_DEED | powers::LAND_SET_SALE_INFO)];
        assert_eq!(assess(Some(&p), false, Some(&region()), &me(&both)).refusal, None);
    }

    #[test]
    fn buying_for_the_group() {
        let none = [officer(powers::NONE)];
        let mut b = me(&none);
        b.for_group = true;
        assert_eq!(
            assess(Some(&for_sale(10)), false, Some(&region()), &b).refusal,
            Some(Refusal::CantBuyForGroup)
        );
        let deed = [officer(powers::LAND_DEED)];
        let mut b = me(&deed);
        b.for_group = true;
        assert_eq!(assess(Some(&for_sale(10)), false, Some(&region()), &b).refusal, None);
        let mut owned = for_sale(10);
        owned.owner_id = GROUP;
        assert_eq!(
            assess(Some(&owned), false, Some(&region()), &b).refusal,
            Some(Refusal::GroupAlreadyOwns)
        );
    }

    #[test]
    fn group_land_and_contribution() {
        let mut p = for_sale(10);
        p.is_group_owned = true;
        p.group_id = GROUP;
        let g = [officer(powers::NONE)];
        let o = assess(Some(&p), false, Some(&region()), &me(&g));
        assert!(o.is_group_land);
        assert_eq!(o.group_contribution, 600);
        assert_eq!(min_contribution(512), 466);
    }

    #[test]
    fn premium_land_counts_more() {
        let r = Region {
            billable_factor: 1.5,
            ..region()
        };
        let o = assess(Some(&for_sale(10)), false, Some(&r), &me(&[]));
        assert_eq!(o.billable_area, 1536);
        assert_eq!(area_note(&o), AreaNote::Premium(1536));
        let r = Region {
            billable_factor: 0.5,
            ..region()
        };
        assert_eq!(
            area_note(&assess(Some(&for_sale(10)), false, Some(&r), &me(&[]))),
            AreaNote::Discounted(512)
        );
        assert_eq!(area_note(&Offer::default()), AreaNote::NoParcel);
    }

    #[test]
    fn objects_capped_by_the_region() {
        let mut p = for_sale(10);
        p.max_prims = 20000;
        p.prim_bonus = 2.0;
        assert_eq!(assess(Some(&p), false, Some(&region()), &me(&[])).supported_objects, 15000);
    }

    #[test]
    fn public_land_is_claimed() {
        let p = ParcelInfo {
            public_count: 4,
            claim_price: 1,
            ..Default::default()
        };
        let o = assess(Some(&p), false, Some(&region()), &me(&[]));
        assert!(o.is_claim && o.can_buy());
        assert_eq!((o.actual_area, o.price), (64, 64));
        let empty = ParcelInfo::default();
        assert_eq!(
            assess(Some(&empty), false, Some(&region()), &me(&[])).refusal,
            Some(Refusal::NoPublicLand)
        );
        let mixed = ParcelInfo {
            public_count: 4,
            other_count: 1,
            ..Default::default()
        };
        assert_eq!(
            assess(Some(&mixed), false, Some(&region()), &me(&[])).refusal,
            Some(Refusal::NotOwnedByYou)
        );
    }

    #[test]
    fn funds() {
        let f = Funds {
            balance: 1200,
            currency_buy: 0,
            price: 4500,
        };
        assert!(!f.have_enough() && !f.will_have_enough());
        assert_eq!(f.shortfall(), 3300);
        let f = Funds { currency_buy: 3300, ..f };
        assert!(f.will_have_enough());
        assert_eq!(f.final_balance(), 0);
        let rich = Funds {
            balance: 12000,
            currency_buy: 0,
            price: 4500,
        };
        assert!(rich.have_enough());
        assert_eq!((rich.shortfall(), rich.final_balance()), (0, 7500));
    }
}
