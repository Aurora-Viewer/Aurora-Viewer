//! Application bridge for the Buy Land window: which selection it follows
//! (About Land's or the land tool's, LLViewerParcelMgr's selection in
//! Firestorm), opening it (LLFloaterBuyLand::buyLand), the website's
//! answers, and the offline land helper of the demo (with its pass scenario).

use super::*;
use crate::world::land::powers;
use aurora_net::currency::CurrencyError;
use aurora_net::land::{LandCommand, PARCEL_RESULT_MULTIPLE};
use aurora_net::land_tool::LandToolInfo;
use ui::buy_land::{Selection, Target};

/// The selection the window is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum BuyLandSource {
    /// About Land's selection (`world.land.sel`): the parcel under the
    /// avatar, a click on the ground, the minimap.
    #[default]
    Parcel,
    /// The land tool's rectangle (build floater, Land).
    LandTool,
}

impl App {
    /// LLFloaterBuyLand::buyLand: only an officer with the power buys for the
    /// active group (OnlyOfficerCanBuyLand).
    pub(super) fn open_buy_land(&mut self, source: BuyLandSource, for_group: bool) {
        let can_deed = self
            .world
            .groups
            .group(&self.world.groups.active)
            .is_some_and(|g| g.powers & powers::LAND_DEED != 0);
        if for_group && !can_deed {
            self.world.notifications.push(
                crate::world::notifications::Kind::Alert,
                aurora_i18n::tr!("buy-land-title"),
                aurora_i18n::tr!("buy-land-only-officer"),
                crate::world::notifications::Data::None,
            );
            return;
        }
        self.buy_land_source = source;
        let helper = self.currency_helper();
        self.buy_land_ui.open(for_group, helper);
    }

    /// handle_buy_land: the parcel under the avatar when nothing is
    /// selected, then the window.
    pub(super) fn buy_land_here(&mut self) {
        if self.world.land.sel.is_none()
            && let Some(h) = self.world.main_region
        {
            let size = self.world.regions.get(&h).map_or((256.0, 256.0), |r| {
                (r.heightmap.size_x.max(256) as f32, r.heightmap.size_y.max(256) as f32)
            });
            let pos = self.world.agent.position;
            self.world.land.select_at(h, pos, size);
        }
        self.open_buy_land(BuyLandSource::Parcel, false);
    }

    /// Where the selection the window follows stands this frame.
    fn buy_land_selection(&self) -> Selection {
        match self.buy_land_source {
            BuyLandSource::Parcel => {
                let Some(sel) = self.world.land.sel.as_ref() else {
                    return Selection::Gone;
                };
                let Some(parcel) = sel.parcel.clone() else {
                    return Selection::Pending;
                };
                // a snapped selection is the whole parcel (processParcelProperties)
                let rect = (parcel.aabb_min.x, parcel.aabb_min.y, parcel.aabb_max.x, parcel.aabb_max.y);
                Selection::Ready(Target {
                    handle: sel.handle,
                    multiple: sel.result == PARCEL_RESULT_MULTIPLE,
                    parcel,
                    rect,
                    spans_regions: false,
                })
            }
            BuyLandSource::LandTool => {
                let land = &self.build.land;
                let Some((handle, west, south, east, north)) = land.selection_local(&self.world) else {
                    return Selection::Gone;
                };
                let Some((_, parcel)) = land.parcel.clone() else {
                    return Selection::Pending;
                };
                // setupParcelBuy: the north-east corner in the same region
                let spans_regions =
                    self.world.regions.get(&handle).is_some_and(|r| {
                        east - 0.5 > r.heightmap.size_x.max(256) as f32 || north - 0.5 > r.heightmap.size_y.max(256) as f32
                    });
                Selection::Ready(Target {
                    handle,
                    multiple: parcel.request_result == PARCEL_RESULT_MULTIPLE,
                    parcel,
                    rect: (west, south, east, north),
                    spans_regions,
                })
            }
        }
    }

    /// The window, once a frame; what it sends.
    pub(super) fn show_buy_land(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) -> Vec<NetCommand> {
        if let Some(for_group) = self.world.land.buy_request.take() {
            self.open_buy_land(BuyLandSource::Parcel, for_group);
        }
        if !self.buy_land_ui.open {
            return Vec::new();
        }
        let selection = self.buy_land_selection();
        let out = self.buy_land_ui.show(ctx, p, &mut self.world, &self.ui_images, selection);
        // a selection only the window used is not refreshed for nothing
        if !self.buy_land_ui.open && self.buy_land_source == BuyLandSource::Parcel && !self.panels.about_land {
            self.world.land.deselect();
        }
        out
    }

    /// buyLandPrep answered: the ParcelBuy follows.
    pub(super) fn land_tool_prepared(&mut self, result: Result<(), CurrencyError>) {
        for cmd in self.buy_land_ui.prepared(result) {
            self.send(cmd);
        }
    }

    /// AURORA_DEMO_BUY_LAND=<mode>: a balance, the plaza parcel for sale
    /// and the window on it (docs/TESTING.md).
    pub(super) fn demo_buy_land(&mut self) {
        let Some(mode) = crate::demo::land::buy_scenario() else {
            return;
        };
        self.world.apply(NetEvent::Balance(if mode == "short" { 1200 } else { 12000 }));
        self.world.apply(NetEvent::LandHoldings {
            credit: 4096,
            committed: 1024,
        });
        self.buy_land_here();
        if crate::demo::land::buy_confirms() {
            self.buy_land_ui.demo_confirm();
        }
    }

    /// AURORA_DEMO_BUY_PASS=<mode>: the ground menu's "Buy access" where the
    /// avatar stands, on the plaza parcel (docs/TESTING.md).
    pub(super) fn demo_buy_pass(&mut self) {
        if crate::demo::land::pass_scenario().is_some() {
            let at = self.world.agent.position;
            self.on_ctx_action(ui::context::CtxAction::BuyPass(at));
        }
    }

    /// The demo's land helper: invented fees and estimates, a purchase that
    /// only moves the simulated balance. Nothing leaves the viewer.
    pub(super) fn demo_buy_land_command(&mut self, cmd: &NetCommand) -> bool {
        let mode = crate::demo::land::buy_scenario().unwrap_or_default();
        match cmd {
            NetCommand::LandToolPreflight {
                billable_area,
                currency_buy,
                ..
            } => {
                let result = if mode == "error" {
                    Err(CurrencyError {
                        message: aurora_i18n::tr!("demo-buy-land-refused").into(),
                        uri: "https://secondlife.com/my/account/".into(),
                    })
                } else {
                    let upgrade = mode == "upgrade";
                    let cents = (i64::from(*currency_buy) * 32 / 100 + 149).min(i64::from(i32::MAX)) as i32;
                    Ok(LandToolInfo {
                        membership_upgrade: upgrade,
                        membership_action: if upgrade {
                            aurora_i18n::tr!("demo-buy-land-upgrade-membership").into()
                        } else {
                            aurora_i18n::tr!("demo-buy-land-keep-membership").into()
                        },
                        levels: if upgrade {
                            vec![
                                ("premium-monthly".into(), aurora_i18n::tr!("demo-buy-land-plan-monthly").into()),
                                ("premium-annual".into(), aurora_i18n::tr!("demo-buy-land-plan-annual").into()),
                            ]
                        } else {
                            Vec::new()
                        },
                        land_use_upgrade: upgrade,
                        land_use_action: if upgrade {
                            aurora_i18n::tr!("demo-buy-land-land-use-up").into()
                        } else {
                            aurora_i18n::tr!("demo-buy-land-land-use-same").into()
                        },
                        usd_cents: (*currency_buy > 0).then_some(cents),
                        local_cost: (*currency_buy > 0).then(|| format!("US$ {}.{:02}", cents / 100, cents % 100)),
                        confirm: if upgrade { "password" } else { "click" }.into(),
                    })
                };
                self.on_app_event(NetEvent::LandToolPreflight {
                    billable_area: *billable_area,
                    currency_buy: *currency_buy,
                    result,
                });
                true
            }
            NetCommand::LandToolBuy { request, .. } => {
                log::info!("demo action: buyLandPrep, L$ {} (simulé)", request.currency_buy);
                let balance = self.world.balance.unwrap_or(0).saturating_add(request.currency_buy);
                self.world.apply(NetEvent::Balance(balance));
                self.on_app_event(NetEvent::LandToolPrepared(Ok(())));
                true
            }
            NetCommand::Land(buy @ LandCommand::Buy { price, area, .. }) => {
                log::info!("demo action: ParcelBuy L$ {price}, {area} m² (simulé)");
                let balance = self.world.balance.unwrap_or(0).saturating_sub(*price);
                self.world.apply(NetEvent::Balance(balance));
                self.world.apply(NetEvent::LandHoldings {
                    credit: 4096,
                    committed: self.world.land_committed.saturating_add(*area),
                });
                // the parcel changes hands: About Land shows it at its next answer
                for ev in crate::demo::land::reply(buy) {
                    if let Some(e) = self.world.apply(ev) {
                        self.on_app_event(e);
                    }
                }
                self.world
                    .system_notice(aurora_i18n::tr!("demo-buy-land-bought"), crate::world::notice_topics::Topic::Region);
                true
            }
            _ => false,
        }
    }
}
