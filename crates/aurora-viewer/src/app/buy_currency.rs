//! Application bridge for the Buy L$ window: which helper page the grid
//! has, the outcome of a purchase, and the offline currency helper of the
//! demo.

use super::*;
use aurora_net::currency::{CurrencyError, CurrencyQuote};

/// Stand-in for the helper page in demo mode: its requests are answered by
/// `demo_currency_command` and never leave the viewer.
const DEMO_HELPER: &str = "demo://currency/";

impl App {
    /// The grid's helper pages (currency.php, landtool.php), or the demo's.
    pub(super) fn currency_helper(&self) -> Option<&'static str> {
        if self.demo {
            Some(DEMO_HELPER)
        } else {
            aurora_net::currency::helper_uri(&self.settings.login_uri())
        }
    }

    /// LLStatusBar::onClickBuyCurrency.
    pub(super) fn open_buy_currency(&mut self) {
        let helper = self.currency_helper();
        self.buy_currency_ui.open(&mut self.world, helper);
    }

    /// LLFloaterBuyCurrencyUI::draw: BuyLindenDollarSuccess once bought.
    pub(super) fn currency_bought(&mut self, result: Result<(), CurrencyError>) {
        if self.buy_currency_ui.bought(result) {
            self.world.notifications.push(
                crate::world::notifications::Kind::Info,
                aurora_i18n::tr!("currency-title"),
                aurora_i18n::tr!("currency-bought"),
                crate::world::notifications::Data::Url(ui::buy_currency::ACCOUNT_URL.into()),
            );
        }
    }

    /// AURORA_DEMO_BUY_CURRENCY: a balance, and the window in the state its
    /// mode names (docs/TESTING.md).
    pub(super) fn demo_buy_currency(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_BUY_CURRENCY") else {
            return;
        };
        self.world.apply(NetEvent::Balance(479));
        if mode != "click" {
            let helper = self.currency_helper();
            self.buy_currency_ui.demo(&mut self.world, helper, &mode);
        }
    }

    /// The demo's currency helper: an invented rate, and a purchase that
    /// only moves the simulated balance. No real L$, no request sent.
    pub(super) fn demo_currency_command(&mut self, cmd: &NetCommand) -> bool {
        match cmd {
            NetCommand::CurrencyQuote { amount, .. } => {
                let mode = std::env::var("AURORA_DEMO_BUY_CURRENCY").unwrap_or_default();
                let result = if mode == "error" {
                    Err(CurrencyError {
                        message: aurora_i18n::tr!("demo-currency-refused").into(),
                        uri: "https://secondlife.com/my/lindex/".into(),
                    })
                } else {
                    let cents = (i64::from(*amount) * 32 / 100 + 149).min(i64::from(i32::MAX)) as i32;
                    Ok(CurrencyQuote {
                        amount: *amount,
                        usd_cents: Some(cents),
                        local_cost: Some(format!("US$ {}.{:02}", cents / 100, cents % 100)),
                        confirm: match mode.as_str() {
                            "password" => "password",
                            "direct" => "",
                            _ => "click",
                        }
                        .into(),
                    })
                };
                self.on_app_event(NetEvent::CurrencyQuote { amount: *amount, result });
                true
            }
            NetCommand::BuyCurrency { quote, .. } => {
                log::info!("demo action: achat de L$ {} (simulé)", quote.amount);
                let balance = self.world.balance.unwrap_or(0).saturating_add(quote.amount);
                self.on_app_event(NetEvent::CurrencyBought(Ok(())));
                self.world.apply(NetEvent::Balance(balance));
                true
            }
            _ => false,
        }
    }
}
