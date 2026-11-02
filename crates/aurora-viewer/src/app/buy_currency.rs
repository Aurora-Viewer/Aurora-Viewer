//! Application bridge for the Buy L$ window: which helper page the grid
//! has, the outcome of a purchase, and the offline currency helper of the
//! demo.

use super::*;
use crate::settings::GridChoice;
use aurora_net::currency::{CurrencyError, CurrencyQuote};

/// Stand-in for the helper page in demo mode: its requests are answered by
/// `demo_currency_command` and never leave the viewer.
const DEMO_HELPER: &str = "demo://currency/";

/// A region's currency-base-uri as a helper page, checked as the grid's
/// is (`Grid::helper_page`: http(s), a host, no credentials).
fn region_page(region: Option<&str>) -> Option<String> {
    let helper = region?.trim().to_owned();
    crate::grids::Grid {
        helper,
        ..Default::default()
    }
    .helper_page()
}

/// LLStartUp's opensim currency support: the login's `currency`, else OS$
/// on an OpenSim grid that names none, L$ on Second Life.
pub(super) fn grid_currency(login: &aurora_llsd::Llsd, opensim: bool) -> String {
    match login["currency"].to_string_value().trim() {
        symbol if !symbol.is_empty() && opensim => symbol.to_owned(),
        _ if opensim => "OS$".into(),
        _ => "L$".into(),
    }
}

/// LLGridManager::getHelperURI: the region's currency-base-uri first
/// (LFSimFeatureHandler::helperUriOverride), then the grid's helper.
fn helper_page(region: Option<&str>, settings: &crate::settings::Settings) -> Option<String> {
    if let Some(page) = region_page(region) {
        return Some(page);
    }
    if settings.grid != GridChoice::Custom {
        return aurora_net::currency::helper_uri(&settings.login_uri()).map(str::to_owned);
    }
    crate::grids::active(settings).and_then(crate::grids::Grid::helper_page)
}

impl App {
    /// The grid's helper pages (currency.php, landtool.php), or the demo's
    /// (LLGridManager::getHelperURI): the agent's region may name its own,
    /// then Second Life's are known, an OpenSim grid publishes its own in
    /// get_grid_info.
    pub(super) fn currency_helper(&self) -> Option<String> {
        if self.demo {
            return Some(DEMO_HELPER.into());
        }
        helper_page(self.world.currency_base_uri(), &self.settings)
    }

    /// Each frame: the symbol shown for L$ follows the agent's region
    /// (Tea::setRegionCurrency, LFSimFeatureHandler::updateCurrencySymbols).
    pub(super) fn follow_currency_symbol(&self) {
        aurora_i18n::set_currency(self.world.currency_symbol());
    }

    /// LLStatusBar::onClickBuyCurrency.
    pub(super) fn open_buy_currency(&mut self) {
        let helper = self.currency_helper();
        let second_life = self.settings.grid != GridChoice::Custom;
        self.buy_currency_ui.open(&mut self.world, helper, second_life);
    }

    /// At login on an OpenSim grid, read its get_grid_info again in the
    /// background: a grid saved before Aurora kept the helper, or whose
    /// helper moved, has the right one by the time Buy L$ is opened.
    pub(super) fn refresh_grid_helper(&mut self) {
        if self.demo || self.settings.grid != GridChoice::Custom {
            return;
        }
        let Some(service) = crate::grids::active(&self.settings).map(|grid| grid.service.clone()) else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.net.runtime().spawn(async move {
            let _ = tx.send(crate::grids::discover(&service).await.map(|grid| (service, grid.helper)));
        });
        self.grid_helper_refresh = Some(rx);
    }

    /// CurrencyURIOverrideReceived: a region that names its own currency
    /// helper sends the purchases, the session and the password a helper
    /// may ask for to that server, so the resident is told each time the
    /// region's value changes (LFSimFeatureHandler::setSupportedFeatures).
    pub(super) fn notice_region_helper(&mut self) {
        let page = region_page(self.world.currency_base_uri());
        if page == self.region_helper_seen {
            return;
        }
        self.region_helper_seen = page.clone();
        let Some(page) = page else {
            return;
        };
        let host = ui::buy_currency::helper_host(&page);
        log::info!("currency: this region names its own currency helper on {host}");
        self.world.notifications.push(
            crate::world::notifications::Kind::Alert,
            aurora_i18n::tr!("currency-title"),
            aurora_i18n::tr!("currency-region-helper", host = host),
            crate::world::notifications::Data::None,
        );
    }

    /// Each frame: fill in the helper the grid answered when the saved grid
    /// has none (only that field: the rest of the entry may hold the user's
    /// own edits, and a helper typed in the grid manager stays).
    pub(super) fn poll_grid_helper(&mut self) {
        let Some(rx) = &self.grid_helper_refresh else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        self.grid_helper_refresh = None;
        let Ok((service, helper)) = result else {
            log::debug!("grid info refresh for the currency helper failed");
            return;
        };
        if helper.is_empty() {
            return;
        }
        if let Some(grid) = self.settings.grids.iter_mut().find(|grid| grid.service == service)
            && grid.helper.trim().is_empty()
        {
            log::info!("grid currency helper: {helper}");
            grid.helper = helper;
            self.settings.save();
        }
    }

    /// LLFloaterBuyCurrencyUI::draw: BuyLindenDollarSuccess once bought.
    pub(super) fn currency_bought(&mut self, result: Result<(), CurrencyError>) {
        if self.buy_currency_ui.bought(result) {
            self.world.notifications.push(
                crate::world::notifications::Kind::Info,
                aurora_i18n::tr!("currency-title"),
                aurora_i18n::tr!("currency-bought"),
                // Second Life's account page; an OpenSim grid's is unknown
                if self.settings.grid == GridChoice::Custom {
                    crate::world::notifications::Data::None
                } else {
                    crate::world::notifications::Data::Url(ui::buy_currency::ACCOUNT_URL.into())
                },
            );
        }
    }

    /// AURORA_DEMO_BUY_CURRENCY: a balance, and the window in the state its
    /// mode names (docs/TESTING.md).
    pub(super) fn demo_buy_currency(&mut self) {
        // AURORA_DEMO_CURRENCY=<symbol>: an OpenSim grid's own money symbol
        if let Ok(symbol) = std::env::var("AURORA_DEMO_CURRENCY") {
            self.world.grid_currency = symbol;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn opensim(helper: &str) -> Settings {
        let mut settings = Settings::default();
        let mut grid = crate::grids::Grid::manual("grid.example:8002").expect("grid");
        grid.helper = helper.into();
        settings.custom_login_uri = grid.login.clone();
        crate::grids::store(&mut settings, grid, None).expect("store");
        settings.grid = GridChoice::Custom;
        settings
    }

    #[test]
    fn a_region_page_is_checked_as_the_grid_s_and_shown_by_its_host() {
        assert_eq!(
            region_page(Some("http://money.example:8008")).as_deref(),
            Some("http://money.example:8008/")
        );
        for bad in ["javascript:alert(1)", "http://user:secret@money.example/", "http://", "  "] {
            assert_eq!(region_page(Some(bad)), None, "{bad}");
        }
        assert_eq!(ui::buy_currency::helper_host("https://money.example/helper/"), "money.example");
        assert_eq!(ui::buy_currency::helper_host("http://money.example:8008/"), "money.example:8008");
    }

    #[test]
    fn the_symbol_comes_from_the_login_on_opensim_only() {
        let login = |currency: &str| aurora_llsd::llsd_map! { "currency" => currency };
        assert_eq!(grid_currency(&login("KC"), true), "KC");
        assert_eq!(grid_currency(&login(" "), true), "OS$", "the Legacy Viewer's OpenSim default");
        assert_eq!(grid_currency(&aurora_llsd::Llsd::Undef, true), "OS$");
        assert_eq!(grid_currency(&login("KC"), false), "L$", "Second Life is always L$");
    }

    #[test]
    fn helper_follows_the_region_then_the_grid() {
        let settings = opensim("https://grid.example/helper");
        assert_eq!(helper_page(None, &settings).as_deref(), Some("https://grid.example/helper/"));
        assert_eq!(
            helper_page(Some("http://region.example:8008"), &settings).as_deref(),
            Some("http://region.example:8008/"),
            "currency-base-uri wins"
        );
        assert_eq!(
            helper_page(Some("javascript:alert(1)"), &settings).as_deref(),
            Some("https://grid.example/helper/"),
            "only a web page"
        );
        assert_eq!(helper_page(None, &opensim("")), None, "unavailable without a helper");
        let settings = Settings {
            grid: GridChoice::SecondLife,
            ..Default::default()
        };
        assert_eq!(helper_page(None, &settings).as_deref(), Some("https://secondlife.com/helpers/"));
    }
}
