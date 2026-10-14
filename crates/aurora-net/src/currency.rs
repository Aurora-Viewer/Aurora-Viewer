//! Buying L$ through the grid's currency helper: the XML-RPC calls
//! `getCurrencyQuote` and `buyCurrency` on `<helper>currency.php`. Port of
//! LLCurrencyUIManager::Impl (indra/newview/llcurrencyuimanager.cpp,
//! originally LGPL 2.1) and of the helper pages of LLGridManager
//! (llviewernetwork.cpp).

use crate::xmlrpc;
use aurora_llsd::{Llsd, Map};
use std::time::Duration;
use uuid::Uuid;

/// Helper pages of the Linden grids (LLGridManager's MAINGRID and Aditi).
const SL_MAIN_HELPER: &str = "https://secondlife.com/helpers/";
const SL_BETA_HELPER: &str = "https://secondlife.aditi.lindenlab.com/helpers/";

/// The helper page of the L$ purchases.
pub(crate) const PAGE: &str = "currency.php";

/// LLXMLRPCTransaction's default timeout (llxmlrpctransaction.cpp).
const TIMEOUT: Duration = Duration::from_secs(40);

/// Helper page of the grid logged in at `login_uri`; None for a grid whose
/// helper is not known (OpenSim publishes its own in get_grid_info).
pub fn helper_uri(login_uri: &str) -> Option<&'static str> {
    match login_uri {
        crate::login::SL_MAIN_GRID => Some(SL_MAIN_HELPER),
        crate::login::SL_BETA_GRID => Some(SL_BETA_HELPER),
        _ => None,
    }
}

/// Answer of `getCurrencyQuote` (finishCurrencyInfo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrencyQuote {
    /// L$ the estimate is for: the helper may change the amount asked.
    pub amount: i32,
    /// Old helpers: the cost in US cents.
    pub usd_cents: Option<i32>,
    /// Newer helpers: the cost as text with its currency ("US$ 4.25").
    pub local_cost: Option<String>,
    /// Confirmation the helper wants before the purchase: "click",
    /// "password", or anything else for none (LLConfirmationManager).
    pub confirm: String,
}

/// Refusal of the helper, or a failed exchange with it (setError).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CurrencyError {
    pub message: String,
    /// Page explaining the refusal (may be empty).
    pub uri: String,
}

/// Account password typed for a "password" confirmation. It only travels
/// from the dialog to the request: its `Debug` never prints it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Password(pub String);

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(…)")
    }
}

/// Parameters shared by both calls (updateCurrencyInfo / startCurrencyBuy).
fn base_params(agent_id: Uuid, secure_session_id: Uuid, amount: i32) -> Map {
    let mut version = aurora_version::login_version().split('.');
    let mut number = || version.next().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let mut p = Map::new();
    p.insert("agentId".into(), agent_id.to_string().into());
    p.insert("secureSessionId".into(), secure_session_id.to_string().into());
    p.insert("language".into(), aurora_i18n::language().into());
    p.insert("currencyBuy".into(), amount.into());
    p.insert("viewerChannel".into(), crate::login::CHANNEL.into());
    p.insert("viewerMajorVersion".into(), number().into());
    p.insert("viewerMinorVersion".into(), number().into());
    p.insert("viewerPatchVersion".into(), number().into());
    // a string, like Firestorm: an XML-RPC int is too narrow for a build number
    p.insert("viewerBuildVersion".into(), version.next().unwrap_or("0").into());
    p
}

pub(crate) fn quote_params(agent_id: Uuid, secure_session_id: Uuid, amount: i32) -> Llsd {
    Llsd::Map(base_params(agent_id, secure_session_id, amount))
}

/// The purchase repeats the estimate it accepts, so the helper can refuse a
/// rate that changed in between.
pub(crate) fn buy_params(agent_id: Uuid, secure_session_id: Uuid, quote: &CurrencyQuote, password: &Password) -> Llsd {
    let mut p = base_params(agent_id, secure_session_id, quote.amount);
    p.insert("confirm".into(), quote.confirm.clone().into());
    if let Some(cents) = quote.usd_cents {
        p.insert("estimatedCost".into(), cents.into());
    }
    if let Some(cost) = &quote.local_cost {
        p.insert("estimatedLocalCost".into(), cost.clone().into());
    }
    if !password.0.is_empty() {
        p.insert("password".into(), password.0.clone().into());
    }
    Llsd::Map(p)
}

/// The helper's `success: false` answer (both helper pages use it).
pub(crate) fn refusal(result: &Llsd) -> Result<(), CurrencyError> {
    if result["success"].as_bool() {
        return Ok(());
    }
    Err(CurrencyError {
        message: result["errorMessage"].to_string_value(),
        uri: result["errorURI"].to_string_value(),
    })
}

pub(crate) fn parse_quote(result: &Llsd) -> Result<CurrencyQuote, CurrencyError> {
    refusal(result)?;
    let currency = &result["currency"];
    Ok(CurrencyQuote {
        amount: currency["currencyBuy"].as_i32(),
        usd_cents: currency.has("estimatedCost").then(|| currency["estimatedCost"].as_i32()),
        local_cost: currency
            .has("estimatedLocalCost")
            .then(|| currency["estimatedLocalCost"].to_string_value()),
        confirm: result["confirm"].to_string_value(),
    })
}

pub(crate) fn parse_buy(result: &Llsd) -> Result<(), CurrencyError> {
    refusal(result)
}

/// One XML-RPC exchange with the helper's `page` (`currency.php`,
/// `landtool.php`). Sent once: a purchase is never retried. Neither the
/// parameters (session, password) nor the answer are logged.
pub(crate) async fn call(http: &reqwest::Client, helper: &str, page: &str, method: &str, params: Llsd) -> Result<Llsd, CurrencyError> {
    let failed = |detail: String| {
        log::warn!("{method}: {detail}");
        CurrencyError {
            message: aurora_i18n::tr!("currency-helper-unreachable").into(),
            uri: String::new(),
        }
    };
    let response = http
        .post(format!("{helper}{page}"))
        .header("Content-Type", "text/xml")
        .timeout(TIMEOUT)
        .body(xmlrpc::build_call(method, &params))
        .send()
        .await
        .map_err(|e| failed(e.without_url().to_string()))?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| failed(e.without_url().to_string()))?;
    if !status.is_success() {
        return Err(failed(format!("HTTP {status}")));
    }
    match xmlrpc::parse_response(&body) {
        Ok(result) => Ok(result),
        Err(xmlrpc::XmlRpcError::Fault { code, message }) => {
            log::warn!("{method}: fault {code}");
            Err(CurrencyError {
                message,
                uri: String::new(),
            })
        }
        Err(e) => Err(failed(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: Uuid = Uuid::from_u128(0x1111);
    const SESSION: Uuid = Uuid::from_u128(0x2222);

    fn response(body: &str) -> Llsd {
        xmlrpc::parse_response(format!("<?xml version=\"1.0\"?><methodResponse><params><param><value><struct>{body}</struct></value></param></params></methodResponse>").as_bytes())
            .expect("well-formed test response")
    }

    #[test]
    fn helper_only_for_linden_grids() {
        assert_eq!(helper_uri(crate::login::SL_MAIN_GRID), Some("https://secondlife.com/helpers/"));
        assert_eq!(
            helper_uri(crate::login::SL_BETA_GRID),
            Some("https://secondlife.aditi.lindenlab.com/helpers/")
        );
        assert_eq!(helper_uri("http://127.0.0.1:9000/"), None);
    }

    #[test]
    fn quote_request_follows_the_viewer_fields() {
        let p = quote_params(AGENT, SESSION, 1000);
        assert_eq!(p["agentId"].as_str(), AGENT.to_string());
        assert_eq!(p["secureSessionId"].as_str(), SESSION.to_string());
        assert_eq!(p["currencyBuy"], Llsd::Integer(1000));
        assert_eq!(p["viewerChannel"].as_str(), "Aurora Viewer");
        assert!(matches!(p["viewerMajorVersion"], Llsd::Integer(_)));
        assert!(matches!(p["viewerBuildVersion"], Llsd::String(_)));
        assert!(!p.has("confirm") && !p.has("password"));
        let doc = xmlrpc::build_call("getCurrencyQuote", &p);
        assert!(doc.contains("<methodName>getCurrencyQuote</methodName>"));
        assert!(doc.contains("<name>currencyBuy</name><value><int>1000</int></value>"));
    }

    #[test]
    fn new_style_quote() {
        let r = response(
            "<member><name>success</name><value><boolean>1</boolean></value></member>\
             <member><name>confirm</name><value><string>click</string></value></member>\
             <member><name>currency</name><value><struct>\
             <member><name>currencyBuy</name><value><int>1000</int></value></member>\
             <member><name>estimatedCost</name><value><int>425</int></value></member>\
             <member><name>estimatedLocalCost</name><value><string>US$ 4.25</string></value></member>\
             </struct></value></member>",
        );
        assert_eq!(
            parse_quote(&r),
            Ok(CurrencyQuote {
                amount: 1000,
                usd_cents: Some(425),
                local_cost: Some("US$ 4.25".into()),
                confirm: "click".into(),
            })
        );
    }

    #[test]
    fn old_style_quote_and_adjusted_amount() {
        let r = response(
            "<member><name>success</name><value><boolean>1</boolean></value></member>\
             <member><name>currency</name><value><struct>\
             <member><name>currencyBuy</name><value><int>250</int></value></member>\
             <member><name>estimatedCost</name><value><int>129</int></value></member>\
             </struct></value></member>",
        );
        let quote = parse_quote(&r).expect("accepted quote");
        assert_eq!((quote.amount, quote.usd_cents, quote.local_cost), (250, Some(129), None));
        assert_eq!(quote.confirm, "");
    }

    #[test]
    fn refusal_carries_message_and_page() {
        let r = response(
            "<member><name>success</name><value><boolean>0</boolean></value></member>\
             <member><name>errorMessage</name><value><string>No payment method</string></value></member>\
             <member><name>errorURI</name><value><string>https://example.invalid/help</string></value></member>",
        );
        let error = CurrencyError {
            message: "No payment method".into(),
            uri: "https://example.invalid/help".into(),
        };
        assert_eq!(parse_quote(&r), Err(error.clone()));
        assert_eq!(parse_buy(&r), Err(error));
    }

    #[test]
    fn purchase_repeats_the_accepted_estimate() {
        let quote = CurrencyQuote {
            amount: 1000,
            usd_cents: Some(425),
            local_cost: Some("US$ 4.25".into()),
            confirm: "password".into(),
        };
        let p = buy_params(AGENT, SESSION, &quote, &Password("typed".into()));
        assert_eq!(p["currencyBuy"], Llsd::Integer(1000));
        assert_eq!(p["confirm"].as_str(), "password");
        assert_eq!(p["estimatedCost"], Llsd::Integer(425));
        assert_eq!(p["estimatedLocalCost"].as_str(), "US$ 4.25");
        assert_eq!(p["password"].as_str(), "typed");
        let click = buy_params(AGENT, SESSION, &CurrencyQuote { usd_cents: None, ..quote }, &Password::default());
        assert!(!click.has("password") && !click.has("estimatedCost"));
    }

    #[test]
    fn password_is_not_printed() {
        assert!(!format!("{:?}", Password("typed".into())).contains("typed"));
    }
}
