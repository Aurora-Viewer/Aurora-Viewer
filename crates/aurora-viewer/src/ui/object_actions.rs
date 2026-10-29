//! Purchase and object-payment dialogs. Behavior follows LLFloaterBuy and
//! LLFloaterPay (Firestorm indra/newview, originally LGPL 2.1).
use super::widgets::{Floater, flat_button_sized};
use crate::{
    interaction::{Action, Interactions, purchase_item},
    theme::Palette,
    world::World,
};
use egui::{RichText, Vec2};

mod contents_list;
#[cfg(test)]
mod contents_tests;

pub(crate) use contents_list::demo_select_all as contents_demo_select_all;
pub(crate) use contents_list::shortcuts as contents_shortcuts;

fn money(amount: i32) -> String {
    format!("L$ {}", super::hud::group_digits(amount.max(0) as u32))
}

fn permission_name(name: &str, mask: u32) -> String {
    use aurora_net::build::perm;
    let mut text = name.to_owned();
    for (bit, suffix) in [
        (perm::COPY, aurora_i18n::tr!("ui-object-actions-no-copy")),
        (perm::MODIFY, aurora_i18n::tr!("ui-object-actions-no-modify")),
        (perm::TRANSFER, aurora_i18n::tr!("ui-object-actions-no-transfer")),
    ] {
        if mask & bit == 0 {
            text.push_str(&format!(" ({suffix})"));
        }
    }
    text
}

fn named_row(ui: &mut egui::Ui, p: &Palette, icon: &str, tint: egui::Color32, text: &str) {
    ui.horizontal(|ui| {
        if let Some(t) = super::icons::global(icon) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
        }
        ui.add(egui::Label::new(RichText::new(text).color(p.ink)).truncate())
            .on_hover_text(text);
    });
}

/// None = no confirmation; Some(None) = buy; Some(Some(amount)) = pay.
pub fn show(ctx: &egui::Context, p: &Palette, state: &mut Interactions, world: &World) -> Option<Option<i32>> {
    let d = state.dialog.as_mut()?;
    let buy = d.target.action == Action::Buy;
    let mut open = true;
    let mut cancel = false;
    let mut confirm = None;
    let center = ctx.content_rect().center();
    let width = if buy {
        320.0
    } else {
        let longest = d
            .prices
            .as_ref()
            .map(|(_, buttons)| {
                buttons
                    .iter()
                    .filter(|v| **v > 0)
                    .map(|v| ui_text_width(ctx, &money(*v)) + 24.0)
                    .fold(0.0_f32, f32::max)
            })
            .unwrap_or(0.0);
        280.0_f32.max(longest * 2.0 + 28.0)
    };
    let title = if buy {
        match d.props.as_ref().map(|p| p.sale_type) {
            Some(1) => aurora_i18n::tr!("ui-object-actions-buy-original"),
            Some(2) => aurora_i18n::tr!("ui-object-actions-buy-a-copy"),
            Some(3) => aurora_i18n::tr!("ui-object-actions-buy-contents"),
            _ => aurora_i18n::tr!("common-buy-object"),
        }
    } else {
        aurora_i18n::tr!("common-pay-object")
    };
    Floater::new(
        if buy { "object_purchase" } else { "object_payment" },
        title,
        egui::pos2(center.x - width / 2.0, center.y - 140.0),
        Vec2::new(width, 280.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        // PayPrice arrives after opening: grow for its longest label, and keep
        // unrelated transactions from inheriting a former window's width.
        ui.set_width(width);
        ui.spacing_mut().interact_size.y = 20.0;
        ui.spacing_mut().item_spacing.y = 4.0;
        let available = world.objects.index_of_uuid(&d.target.object).is_some();
        if !available {
            ui.label(RichText::new(aurora_i18n::tr!("ui-object-actions-this-object-is-no-longer-available")).color(p.danger));
        } else if let Some(props) = &d.props {
            let seller = world
                .social
                .avatar_names
                .complete(&props.owner_id)
                .or_else(|| world.avatar_name(&props.owner_id))
                .unwrap_or_else(|| aurora_i18n::tr!("ui-object-actions-loading-name").into());
            let affordable = |amount: i32| amount >= 0 && world.balance.is_some_and(|b| amount <= b);
            if buy {
                let object_label = if props.sale_type == 3 {
                    props.name.clone()
                } else {
                    permission_name(&props.name, props.next_owner_mask)
                };
                named_row(ui, p, "box", p.ink, &object_label);
                ui.add_space(6.0);
                ui.label(aurora_i18n::tr!("ui-object-actions-contents"));
                egui::Frame::new()
                    .fill(p.raised)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .min_scrolled_height(140.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| match &d.inventory {
                                Some(Ok(items)) => {
                                    let mut empty = true;
                                    for item in items.iter().filter(|item| purchase_item(item, world.agent_id, props.sale_type)) {
                                        empty = false;
                                        named_row(
                                            ui,
                                            p,
                                            super::inventory::item_icon(item.asset_type as i32, item.inv_type as i32, item.flags),
                                            super::inventory::item_icon_color(item.asset_type as i32, item.inv_type as i32, p),
                                            &permission_name(&item.name, item.next_owner_mask),
                                        );
                                    }
                                    if empty {
                                        ui.label(aurora_i18n::tr!("ui-object-actions-no-transferable-items"));
                                    }
                                }
                                Some(Err(error)) => {
                                    ui.label(RichText::new(error).color(p.danger));
                                }
                                None => {
                                    ui.label(aurora_i18n::tr!("ui-object-actions-loading-contents"));
                                }
                            });
                    });
                ui.add_space(6.0);
                ui.label(aurora_i18n::tr!(
                    "ui-object-actions-buy-for-from",
                    value0 = money(props.sale_price).to_string()
                ));
                named_row(ui, p, "user-filled", p.ink, &seller);
                if !affordable(props.sale_price) {
                    ui.label(RichText::new(aurora_i18n::tr!("common-insufficient-or-unknown-balance")).color(p.danger));
                }
                if !(1..=3).contains(&props.sale_type) {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-object-actions-this-object-is-no-longer-for-sale")).color(p.danger));
                }
            } else {
                named_row(ui, p, "user-filled", p.ink, &seller);
                ui.label(RichText::new(aurora_i18n::tr!("ui-object-actions-via-object")).color(p.muted));
                named_row(ui, p, "box", p.ink, &props.name);
                ui.add_space(6.0);
                if let Some((default, buttons)) = &d.prices {
                    if buttons.iter().any(|v| *v > 0) {
                        let button_width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        egui::Grid::new("quick_pay")
                            .num_columns(2)
                            .spacing(Vec2::new(8.0, 6.0))
                            .show(ui, |ui| {
                                for index in 0..4 {
                                    if let Some(&amount) = buttons.get(index).filter(|v| **v > 0) {
                                        if ui
                                            .add_enabled_ui(affordable(amount), |ui| {
                                                flat_button_sized(ui, p, &money(amount), Vec2::new(button_width, 24.0))
                                            })
                                            .inner
                                            .clicked()
                                        {
                                            confirm = Some(Some(amount));
                                        }
                                    } else {
                                        ui.allocate_space(Vec2::new(button_width, 24.0));
                                    }
                                    if index % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.add_space(6.0);
                    }
                    if *default != -1 {
                        ui.label(aurora_i18n::tr!("ui-object-actions-or-choose-an-amount"));
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add(egui::TextEdit::singleline(&mut d.amount).desired_width(110.0).char_limit(10));
                                ui.label("L$");
                            });
                        });
                    }
                } else {
                    ui.label(aurora_i18n::tr!("ui-object-actions-loading-suggested-amounts"));
                }
            }
        } else {
            ui.label(aurora_i18n::tr!("ui-object-actions-loading-object-information"));
        }
        if d.opened.elapsed().as_secs() > 15 && (d.props.is_none() || (buy && d.inventory.is_none()) || (!buy && d.prices.is_none())) {
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-object-actions-the-simulator-did-not-respond-close-and-try-again"
                ))
                .color(p.danger),
            );
        }
        if let Some(error) = &d.error {
            ui.label(RichText::new(error).color(p.danger));
        }
        ui.add_space(10.0);
        let button_width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        ui.horizontal(|ui| {
            let amount = d.amount.trim().parse::<i32>().ok().filter(|v| *v > 0);
            let show_confirm = buy || d.prices.as_ref().is_none_or(|(default, _)| *default != -1);
            if show_confirm {
                let valid = available
                    && d.props.as_ref().is_some_and(|props| {
                        let price = if buy { Some(props.sale_price) } else { amount };
                        price.is_some_and(|v| v >= 0 && world.balance.is_some_and(|b| v <= b))
                            && if buy {
                                (1..=3).contains(&props.sale_type)
                                    && (props.sale_type != 3
                                        || d.inventory.as_ref().is_some_and(|r| {
                                            r.as_ref()
                                                .is_ok_and(|items| items.iter().any(|item| purchase_item(item, world.agent_id, 3)))
                                        }))
                            } else {
                                d.prices.is_some()
                            }
                    });
                if ui
                    .add_enabled_ui(valid, |ui| {
                        flat_button_sized(
                            ui,
                            p,
                            if buy {
                                aurora_i18n::tr!("common-buy")
                            } else {
                                aurora_i18n::tr!("common-pay")
                            },
                            Vec2::new(button_width, 24.0),
                        )
                    })
                    .inner
                    .clicked()
                {
                    confirm = if buy { Some(None) } else { Some(amount) };
                }
            } else {
                ui.add_space(button_width + ui.spacing().item_spacing.x);
            }
            if flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(button_width, 24.0)).clicked() {
                cancel = true;
            }
        });
    });
    if !open || cancel {
        state.dialog = None;
        confirm = None;
    }
    confirm
}

fn ui_text_width(ctx: &egui::Context, text: &str) -> f32 {
    ctx.fonts_mut(|f| {
        f.layout_no_wrap(text.into(), egui::FontId::proportional(12.0), egui::Color32::PLACEHOLDER)
            .size()
            .x
    })
}

pub enum ContentsAction {
    Script(uuid::Uuid, uuid::Uuid),
    Notecard(uuid::Uuid, uuid::Uuid),
    Copy(Box<CopyRequest>),
    Take(uuid::Uuid, Vec<aurora_net::build::TaskItem>),
    Remove(uuid::Uuid, Vec<uuid::Uuid>),
    Drop(uuid::Uuid, Vec<uuid::Uuid>, bool),
}

/// "Copy to inventory" (LLFloaterOpenObject::onClickCopy): every item into
/// a new folder named after the object, then worn in `wear` mode ("Replace
/// outfit with contents" / "Add contents to outfit").
#[derive(Clone)]
pub struct CopyRequest {
    pub object: uuid::Uuid,
    pub name: String,
    pub items: Vec<aurora_net::build::TaskItem>,
    pub transfer: crate::build::task_inventory::ContentsTransfer,
    pub wear: Option<crate::world::appearance::WearMode>,
}

/// A copy that moves no-copy items, waiting for the user's answer
/// (warn_move_inventory: MoveInventoryFromObject / ...FromScriptedObject).
pub struct CopyConfirm {
    pub request: CopyRequest,
    pub ignore: bool,
}

/// The copy the contents window's button asks for (without wearing), or why
/// it cannot: the listing not there yet, the object gone, an item refused.
pub fn copy_request(
    c: &crate::interaction::Contents,
    props: Option<&aurora_net::build::ObjectProps>,
    world: &World,
) -> Result<CopyRequest, String> {
    use crate::build::task_inventory as task;
    if world.objects.index_of_uuid(&c.target.object).is_none() {
        return Err(aurora_i18n::tr!("ui-object-actions-this-object-is-no-longer-available").to_owned());
    }
    let items = match &c.result {
        Some(Ok(items)) => items,
        Some(Err(error)) => return Err(error.clone()),
        None => return Err(aurora_i18n::tr!("ui-object-actions-loading-contents").to_owned()),
    };
    let items: Vec<_> = items.iter().map(|item| task::from_inventory(c.target.object, item)).collect();
    let transfer = task::contents_transfer(world, c.target.object, &items, task::owned(world, c.target.object, props))?;
    Ok(CopyRequest {
        object: c.target.object,
        name: props.map(|props| props.name.clone()).unwrap_or_default(),
        items,
        transfer,
        wear: None,
    })
}

/// LLFloaterOpenObject: load and show the root object's task inventory.
pub fn show_contents(
    ctx: &egui::Context,
    p: &Palette,
    icons: &super::icons::Icons,
    state: &mut Interactions,
    world: &World,
    build: &crate::build::BuildTool,
) -> Vec<ContentsAction> {
    let Some(c) = state.contents.as_ref() else { return Vec::new() };
    let mut documents = Vec::new();
    let mut open = true;
    let mut close = false;
    let center = ctx.content_rect().center();
    Floater::new(
        "object_contents",
        aurora_i18n::tr!("common-object-contents"),
        egui::pos2(center.x - 190.0, center.y - 130.0),
        Vec2::new(380.0, 300.0),
    )
    .show(ctx, p, &mut open, |ui| {
        let props = state.props.get(&c.target.root);
        if let Some(props) = props {
            ui.label(RichText::new(&props.name).strong().color(p.ink));
        }
        let present = world.objects.index_of_uuid(&c.target.object).is_some();
        // the button stays in view, greyed with the reason (the Legacy Viewer's
        // OpenObjectCannotCopy alert after the click)
        let copyable = copy_request(c, props, world);
        let (mut list, footer) = super::widgets::above_footer(ui);
        let outer = ui;
        let ui = &mut list;
        if !present {
            ui.label(RichText::new(aurora_i18n::tr!("ui-object-actions-this-object-is-no-longer-available")).color(p.danger));
        } else {
            match &c.result {
                Some(Ok(items)) => {
                    documents.extend(contents_list::show(ui, p, icons, c.target.object, items, world, build));
                }
                Some(Err(error)) => {
                    ui.label(RichText::new(error).color(p.danger));
                }
                None if c.opened.elapsed().as_secs() > 30 => {
                    ui.label(
                        RichText::new(aurora_i18n::tr!(
                            "ui-object-actions-the-simulator-did-not-respond-close-and-try-again"
                        ))
                        .color(p.danger),
                    );
                }
                None => {
                    ui.label(aurora_i18n::tr!("ui-object-actions-loading-contents"));
                }
            }
        }
        footer.show(outer, |ui| {
            use super::widgets::{ActionKind, action_button_if, action_menu_button};
            let mut copy = None;
            ui.horizontal(|ui| {
                let ready = copyable.is_ok();
                let tip = match &copyable {
                    Ok(_) => aurora_i18n::tr!("contents-copy-tip"),
                    Err(reason) => reason.as_str(),
                };
                let label = aurora_i18n::tr!("contents-copy-button");
                let size = Vec2::new(0.0, 26.0);
                let button = action_button_if(ui, p, "briefcase-filled", label, ActionKind::Primary, size, ready);
                if button.on_hover_text(tip).on_disabled_hover_text(tip).clicked() {
                    copy = Some(None);
                }
                let wear = ui.add_enabled(ready, |ui: &mut egui::Ui| {
                    action_menu_button(ui, p, "shirt-filled", aurora_i18n::tr!("contents-copy-wear"), false, 26.0)
                });
                let wear = wear.on_disabled_hover_text(tip);
                super::menu::dropdown(&wear, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    use crate::world::appearance::WearMode;
                    if super::menu::item(ui, p, "shirt-filled", aurora_i18n::tr!("contents-copy-replace")) {
                        copy = Some(Some(WearMode::ReplaceOutfit));
                    }
                    if super::menu::item(ui, p, "plus", aurora_i18n::tr!("contents-copy-add")) {
                        copy = Some(Some(WearMode::Append));
                    }
                });
            });
            // the window closes on the copy (LLFloaterOpenObject::onClickCopy)
            if let Some(wear) = copy
                && let Ok(request) = &copyable
            {
                documents.push(ContentsAction::Copy(Box::new(CopyRequest { wear, ..request.clone() })));
                close = true;
            }
        });
    });
    if close {
        open = false;
    }
    if !open {
        contents_list::clear(ctx);
        state.contents = None;
    }
    documents
}

/// The move warning of a copy with no-copy items: their names, the scripts'
/// warning, "do not show again". Returns the confirmed copy.
pub fn confirm_copy(ctx: &egui::Context, p: &Palette, pending: &mut Option<CopyConfirm>) -> Option<CopyConfirm> {
    let confirm = pending.as_mut()?;
    let mut answer = None;
    super::widgets::Modal::new(egui::Id::unique("object_contents_copy")).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("task-transfer-title"))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(8.0);
        let request = &confirm.request;
        ui.label(
            RichText::new(crate::build::task_inventory::contents_folder_name(&request.name))
                .strong()
                .color(p.ink),
        );
        ui.label(aurora_i18n::tr!("contents-copy-move-warning", count = request.transfer.moved.len()));
        for id in request.transfer.moved.iter().take(MOVED_SHOWN) {
            if let Some(item) = request.items.iter().find(|item| item.item_id == *id) {
                ui.label(RichText::new(format!("• {}", item.name)).color(p.ink));
            }
        }
        if request.transfer.moved.len() > MOVED_SHOWN {
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "contents-copy-move-more",
                    count = request.transfer.moved.len() - MOVED_SHOWN
                ))
                .color(p.muted),
            );
        }
        if request.transfer.scripted {
            ui.label(RichText::new(aurora_i18n::tr!("task-transfer-scripted-warning")).color(p.warn));
        }
        ui.add_space(8.0);
        let ignore = aurora_i18n::tr!("task-transfer-ignore");
        super::widgets::switch_labeled(ui, p, &mut confirm.ignore, ignore);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            use super::widgets::{ActionKind, action_button};
            let size = Vec2::new(0.0, 26.0);
            if action_button(ui, p, "check", aurora_i18n::tr!("task-transfer-confirm"), ActionKind::Primary, size).clicked() {
                answer = Some(true);
            }
            if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                answer = Some(false);
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
    });
    let answer = answer?;
    let confirm = pending.take()?;
    answer.then_some(confirm)
}

/// The no-copy items named in the move warning, the rest counted.
const MOVED_SHOWN: usize = 8;
