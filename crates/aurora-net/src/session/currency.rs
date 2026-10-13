//! L$ purchase commands: one XML-RPC exchange each with the grid's currency
//! helper (`crate::currency`), answered as an event.

use super::Session;
use crate::currency::{self, CurrencyQuote, Password};
use crate::types::NetEvent;

impl Session<'_> {
    /// LLCurrencyUIManager::Impl::updateCurrencyInfo.
    pub(super) fn currency_quote(&mut self, helper: String, amount: i32) {
        let params = currency::quote_params(self.login.agent_id, self.login.secure_session_id, amount);
        let http = self.sh.http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = currency::call(&http, &helper, "getCurrencyQuote", params)
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
            let result = currency::call(&http, &helper, "buyCurrency", params)
                .await
                .and_then(|r| currency::parse_buy(&r));
            match &result {
                Ok(()) => log::info!("buyCurrency: accepted"),
                Err(e) => log::warn!("buyCurrency: refused ({})", e.message),
            }
            let _ = events.send(NetEvent::CurrencyBought(result));
        });
    }
}
