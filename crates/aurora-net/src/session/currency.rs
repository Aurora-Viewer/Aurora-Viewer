//! L$ and land purchase commands: one XML-RPC exchange each with the grid's
//! helper pages (`crate::currency`, `crate::land_tool`), answered as an
//! event.

use super::Session;
use crate::currency::{self, CurrencyQuote, Password};
use crate::land_tool::{self, LandToolBuy};
use crate::types::NetEvent;

impl Session<'_> {
    /// LLCurrencyUIManager::Impl::updateCurrencyInfo.
    pub(super) fn currency_quote(&mut self, helper: String, amount: i32) {
        let params = currency::quote_params(self.login.agent_id, self.login.secure_session_id, amount);
        let http = self.sh.http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = currency::call(&http, &helper, currency::PAGE, "getCurrencyQuote", params)
                .await
                .and_then(|r| currency::parse_quote(&r));
            let _ = events.send(NetEvent::CurrencyQuote { amount, result });
        });
    }

    /// LLCurrencyUIManager::Impl::startCurrencyBuy.
    pub(super) fn buy_currency(&mut self, helper: String, quote: CurrencyQuote, password: Password) {
        log::info!("buyCurrency: L$ {}", quote.amount);
        let params = currency::buy_params(self.login.agent_id, self.login.secure_session_id, &quote, &password);
        let http = self.sh.http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = currency::call(&http, &helper, currency::PAGE, "buyCurrency", params)
                .await
                .and_then(|r| currency::parse_buy(&r));
            match &result {
                Ok(()) => log::info!("buyCurrency: accepted"),
                Err(e) => log::warn!("buyCurrency: refused ({})", e.message),
            }
            let _ = events.send(NetEvent::CurrencyBought(result));
        });
    }

    /// LLFloaterBuyLandUI::updateWebSiteInfo.
    pub(super) fn land_tool_preflight(&mut self, helper: String, billable_area: i32, currency_buy: i32) {
        let params = land_tool::preflight_params(self.login.agent_id, self.login.secure_session_id, billable_area, currency_buy);
        let http = self.sh.http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = currency::call(&http, &helper, land_tool::PAGE, "preflightBuyLandPrep", params)
                .await
                .and_then(|r| land_tool::parse_preflight(&r));
            let _ = events.send(NetEvent::LandToolPreflight {
                billable_area,
                currency_buy,
                result,
            });
        });
    }

    /// LLFloaterBuyLandUI::runWebSitePrep.
    pub(super) fn land_tool_buy(&mut self, helper: String, request: LandToolBuy) {
        log::info!("buyLandPrep: {} m², L$ {}", request.billable_area, request.currency_buy);
        let params = land_tool::buy_params(self.login.agent_id, self.login.secure_session_id, &request);
        let http = self.sh.http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = currency::call(&http, &helper, land_tool::PAGE, "buyLandPrep", params)
                .await
                .and_then(|r| land_tool::parse_buy(&r));
            match &result {
                Ok(()) => log::info!("buyLandPrep: accepted"),
                Err(e) => log::warn!("buyLandPrep: refused ({})", e.message),
            }
            let _ = events.send(NetEvent::LandToolPrepared(result));
        });
    }
}
