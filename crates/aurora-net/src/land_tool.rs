//! The website side of a land purchase: the XML-RPC calls
//! `preflightBuyLandPrep` and `buyLandPrep` on `<helper>landtool.php`, which
//! say (then do) what the account needs before the ParcelBuy: a membership
//! upgrade, a higher land use fee, L$. Port of the transactions of
//! LLFloaterBuyLandUI (indra/newview/llfloaterbuyland.cpp: updateWebSiteInfo,
//! finishWebSiteInfo, runWebSitePrep, finishWebSitePrep; originally LGPL 2.1).

use crate::currency::{CurrencyError, Password};
use aurora_llsd::{Llsd, Map};
use uuid::Uuid;

/// The helper page of the land purchases.
pub(crate) const PAGE: &str = "landtool.php";

/// Answer of `preflightBuyLandPrep` (finishWebSiteInfo).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LandToolInfo {
    /// The account must change plan to own land (`membership.upgrade`), how
    /// the site puts it, and the plans offered (id, description).
    pub membership_upgrade: bool,
    pub membership_action: String,
    pub levels: Vec<(String, String)>,
    /// The monthly land use fee goes up (`landUse.upgrade`), and how.
    pub land_use_upgrade: bool,
    pub land_use_action: String,
    /// Cost of the L$ to buy: US cents (old helpers) or text with its
    /// currency (newer ones).
    pub usd_cents: Option<i32>,
    pub local_cost: Option<String>,
    /// Confirmation wanted before the purchase: "click", "password", or
    /// anything else for none (LLConfirmationManager).
    pub confirm: String,
}

/// What `buyLandPrep` is sent (runWebSitePrep).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LandToolBuy {
    /// Id of the plan chosen, "noChange" without an upgrade.
    pub level_id: String,
    /// 0 for a group purchase: the group's land, not the buyer's.
    pub billable_area: i32,
    pub currency_buy: i32,
    /// The estimate accepted for those L$ (0 / empty without one).
    pub usd_cents: i32,
    pub local_cost: String,
    pub confirm: String,
    pub password: Password,
}

impl std::fmt::Debug for LandToolBuy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LandToolBuy")
            .field("level_id", &self.level_id)
            .field("billable_area", &self.billable_area)
            .field("currency_buy", &self.currency_buy)
            .field("confirm", &self.confirm)
            .finish_non_exhaustive()
    }
}

/// Fields of both calls, in Firestorm's order.
fn base_params(agent_id: Uuid, secure_session_id: Uuid) -> Map {
    let mut p = Map::new();
    p.insert("agentId".into(), agent_id.to_string().into());
    p.insert("secureSessionId".into(), secure_session_id.to_string().into());
    p.insert("language".into(), aurora_i18n::language().into());
    p
}

pub(crate) fn preflight_params(agent_id: Uuid, secure_session_id: Uuid, billable_area: i32, currency_buy: i32) -> Llsd {
    let mut p = base_params(agent_id, secure_session_id);
    p.insert("billableArea".into(), billable_area.into());
    p.insert("currencyBuy".into(), currency_buy.into());
    Llsd::Map(p)
}

pub(crate) fn buy_params(agent_id: Uuid, secure_session_id: Uuid, b: &LandToolBuy) -> Llsd {
    let mut p = base_params(agent_id, secure_session_id);
    p.insert("levelId".into(), b.level_id.clone().into());
    p.insert("billableArea".into(), b.billable_area.into());
    p.insert("currencyBuy".into(), b.currency_buy.into());
    p.insert("estimatedCost".into(), b.usd_cents.into());
    p.insert("estimatedLocalCost".into(), b.local_cost.clone().into());
    p.insert("confirm".into(), b.confirm.clone().into());
    if !b.password.0.is_empty() {
        p.insert("password".into(), b.password.0.clone().into());
    }
    Llsd::Map(p)
}

pub(crate) fn parse_preflight(result: &Llsd) -> Result<LandToolInfo, CurrencyError> {
    crate::currency::refusal(result)?;
    let membership = &result["membership"];
    let land_use = &result["landUse"];
    let currency = &result["currency"];
    Ok(LandToolInfo {
        membership_upgrade: membership["upgrade"].as_bool(),
        membership_action: membership["action"].to_string_value(),
        levels: membership["levels"]
            .as_array()
            .iter()
            .map(|l| (l["id"].to_string_value(), l["description"].to_string_value()))
            .collect(),
        land_use_upgrade: land_use["upgrade"].as_bool(),
        land_use_action: land_use["action"].to_string_value(),
        usd_cents: currency.has("estimatedCost").then(|| currency["estimatedCost"].as_i32()),
        local_cost: currency
            .has("estimatedLocalCost")
            .then(|| currency["estimatedLocalCost"].to_string_value()),
        confirm: result["confirm"].to_string_value(),
    })
}

pub(crate) fn parse_buy(result: &Llsd) -> Result<(), CurrencyError> {
    crate::currency::refusal(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xmlrpc;

    const AGENT: Uuid = Uuid::from_u128(0x1111);
    const SESSION: Uuid = Uuid::from_u128(0x2222);

    fn response(body: &str) -> Llsd {
        xmlrpc::parse_response(format!("<?xml version=\"1.0\"?><methodResponse><params><param><value><struct>{body}</struct></value></param></params></methodResponse>").as_bytes())
            .expect("well-formed test response")
    }

    #[test]
    fn preflight_request_has_firestorms_fields() {
        let p = preflight_params(AGENT, SESSION, 512, 300);
        assert_eq!(p["agentId"].as_str(), AGENT.to_string());
        assert_eq!(p["secureSessionId"].as_str(), SESSION.to_string());
        assert_eq!(p["billableArea"], Llsd::Integer(512));
        assert_eq!(p["currencyBuy"], Llsd::Integer(300));
        assert!(p.has("language"));
        assert!(!p.has("levelId") && !p.has("password"));
        let doc = xmlrpc::build_call("preflightBuyLandPrep", &p);
        assert!(doc.contains("<methodName>preflightBuyLandPrep</methodName>"));
    }

    #[test]
    fn preflight_answer() {
        let r = response(
            "<member><name>success</name><value><boolean>1</boolean></value></member>\
             <member><name>confirm</name><value><string>click</string></value></member>\
             <member><name>membership</name><value><struct>\
               <member><name>upgrade</name><value><boolean>1</boolean></value></member>\
               <member><name>action</name><value><string>Upgrade to Premium</string></value></member>\
               <member><name>levels</name><value><array><data>\
                 <value><struct><member><name>id</name><value><string>premium</string></value></member>\
                 <member><name>description</name><value><string>Premium, monthly</string></value></member></struct></value>\
               </data></array></value></member>\
             </struct></value></member>\
             <member><name>landUse</name><value><struct>\
               <member><name>upgrade</name><value><boolean>0</boolean></value></member>\
               <member><name>action</name><value><string>No change</string></value></member>\
             </struct></value></member>\
             <member><name>currency</name><value><struct>\
               <member><name>estimatedCost</name><value><int>425</int></value></member>\
               <member><name>estimatedLocalCost</name><value><string>US$ 4.25</string></value></member>\
             </struct></value></member>",
        );
        assert_eq!(
            parse_preflight(&r),
            Ok(LandToolInfo {
                membership_upgrade: true,
                membership_action: "Upgrade to Premium".into(),
                levels: vec![("premium".into(), "Premium, monthly".into())],
                land_use_upgrade: false,
                land_use_action: "No change".into(),
                usd_cents: Some(425),
                local_cost: Some("US$ 4.25".into()),
                confirm: "click".into(),
            })
        );
    }

    #[test]
    fn refusal_carries_message_and_page() {
        let r = response(
            "<member><name>success</name><value><boolean>0</boolean></value></member>\
             <member><name>errorMessage</name><value><string>Account on hold</string></value></member>\
             <member><name>errorURI</name><value><string>https://example.invalid/help</string></value></member>",
        );
        let error = CurrencyError {
            message: "Account on hold".into(),
            uri: "https://example.invalid/help".into(),
        };
        assert_eq!(parse_preflight(&r), Err(error.clone()));
        assert_eq!(parse_buy(&r), Err(error));
    }

    #[test]
    fn purchase_request() {
        let b = LandToolBuy {
            level_id: "noChange".into(),
            billable_area: 512,
            currency_buy: 1000,
            usd_cents: 425,
            local_cost: "US$ 4.25".into(),
            confirm: "password".into(),
            password: Password("typed".into()),
        };
        let p = buy_params(AGENT, SESSION, &b);
        assert_eq!(p["levelId"].as_str(), "noChange");
        assert_eq!(p["billableArea"], Llsd::Integer(512));
        assert_eq!(p["currencyBuy"], Llsd::Integer(1000));
        assert_eq!(p["estimatedCost"], Llsd::Integer(425));
        assert_eq!(p["estimatedLocalCost"].as_str(), "US$ 4.25");
        assert_eq!(p["confirm"].as_str(), "password");
        assert_eq!(p["password"].as_str(), "typed");
        assert!(!buy_params(AGENT, SESSION, &LandToolBuy::default()).has("password"));
        assert!(!format!("{b:?}").contains("typed"));
    }
}
