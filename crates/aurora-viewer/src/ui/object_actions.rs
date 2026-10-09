//! Purchase and object-payment dialogs. Behavior follows LLFloaterBuy and
//! LLFloaterPay (Firestorm indra/newview, originally LGPL 2.1).
use super::widgets::{Floater, flat_button_sized};
use crate::{
    interaction::{Action, Interactions, purchase_item},
    theme::Palette,
    world::World,
};
use egui::{RichText, Vec2};

fn money(amount: i32) -> String {
    format!("L$ {}", super::hud::group_digits(amount.max(0) as u32))
}

fn permission_name(name: &str, mask: u32) -> String {
    use aurora_net::build::perm;
    let mut text = name.to_owned();
    for (bit, suffix) in [
        (perm::COPY, "non copiable"),
        (perm::MODIFY, "non modifiable"),
        (perm::TRANSFER, "non transférable"),
    ] {
        if mask & bit == 0 {
            text.push_str(&format!(" ({suffix})"));
        }
    }
    text
}

fn named_row(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &str) {
    ui.horizontal(|ui| {
        if let Some(t) = super::icons::global(icon) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.ink));
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
            Some(1) => "Acheter l'original",
            Some(2) => "Acheter une copie",
            Some(3) => "Acheter le contenu",
            _ => "Acheter l'objet",
        }
    } else {
        "Payer l'objet"
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
            ui.label(RichText::new("Cet objet n'est plus disponible.").color(p.danger));
        } else if let Some(props) = &d.props {
            let seller = world
                .social
                .avatar_names
                .complete(&props.owner_id)
                .or_else(|| world.avatar_name(&props.owner_id))
                .unwrap_or_else(|| "Nom en cours de chargement…".into());
            let affordable = |amount: i32| amount >= 0 && world.balance.is_some_and(|b| amount <= b);
            if buy {
                let object_label = if props.sale_type == 3 {
                    props.name.clone()
                } else {
                    permission_name(&props.name, props.next_owner_mask)
                };
                named_row(ui, p, "cube", &object_label);
                ui.add_space(6.0);
                ui.label("Contenu :");
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
                                            super::inventory::item_icon(item.asset_type as i32, item.inv_type as i32),
                                            &permission_name(&item.name, item.next_owner_mask),
                                        );
                                    }
                                    if empty {
                                        ui.label("Aucun élément transférable.");
                                    }
                                }
                                Some(Err(error)) => {
                                    ui.label(RichText::new(error).color(p.danger));
                                }
                                None => {
                                    ui.label("Chargement du contenu…");
                                }
                            });
                    });
                ui.add_space(6.0);
                ui.label(format!("Acheter pour {} à :", money(props.sale_price)));
                named_row(ui, p, "user", &seller);
                if !affordable(props.sale_price) {
                    ui.label(RichText::new("Solde insuffisant ou encore inconnu.").color(p.danger));
                }
                if !(1..=3).contains(&props.sale_type) {
                    ui.label(RichText::new("Cet objet n'est plus en vente.").color(p.danger));
                }
            } else {
                named_row(ui, p, "user", &seller);
                ui.label(RichText::new("Via l'objet :").color(p.muted));
                named_row(ui, p, "cube", &props.name);
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
                        ui.label("Ou choisissez le montant :");
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.add(egui::TextEdit::singleline(&mut d.amount).desired_width(110.0).char_limit(10));
                                ui.label("L$");
                            });
                        });
                    }
                } else {
                    ui.label("Chargement des montants proposés…");
                }
            }
        } else {
            ui.label("Chargement des informations de l'objet…");
        }
        if d.opened.elapsed().as_secs() > 15 && (d.props.is_none() || (buy && d.inventory.is_none()) || (!buy && d.prices.is_none())) {
            ui.label(RichText::new("Le simulateur n'a pas répondu. Fermez puis réessayez.").color(p.danger));
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
                        flat_button_sized(ui, p, if buy { "Acheter" } else { "Payer" }, Vec2::new(button_width, 24.0))
                    })
                    .inner
                    .clicked()
                {
                    confirm = if buy { Some(None) } else { Some(amount) };
                }
            } else {
                ui.add_space(button_width + ui.spacing().item_spacing.x);
            }
            if flat_button_sized(ui, p, "Annuler", Vec2::new(button_width, 24.0)).clicked() {
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

/// LLFloaterOpenObject: load and show the root object's task inventory.
pub fn show_contents(ctx: &egui::Context, p: &Palette, icons: &super::icons::Icons, state: &mut Interactions, world: &World) {
    let Some(c) = state.contents.as_ref() else { return };
    let mut open = true;
    let center = ctx.content_rect().center();
    Floater::new(
        "object_contents",
        "Contenu de l'objet",
        egui::pos2(center.x - 190.0, center.y - 130.0),
        Vec2::new(380.0, 260.0),
    )
    .show(ctx, p, &mut open, |ui| {
        if let Some(props) = state.props.get(&c.target.root) {
            ui.label(RichText::new(&props.name).strong().color(p.ink));
        }
        if world.objects.index_of_uuid(&c.target.object).is_none() {
            ui.label(RichText::new("Cet objet n'est plus disponible.").color(p.danger));
        } else {
            match &c.result {
                Some(Ok(items)) if items.is_empty() => {
                    ui.label("Cet objet est vide.");
                }
                Some(Ok(items)) => {
                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        for item in items {
                            ui.horizontal(|ui| {
                                if let Some(t) = icons.get(super::inventory::item_icon(item.asset_type, item.inv_type)) {
                                    ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.ink));
                                }
                                ui.label(&item.name).on_hover_text(&item.desc);
                            });
                        }
                    });
                }
                Some(Err(error)) => {
                    ui.label(RichText::new(error).color(p.danger));
                }
                None if c.opened.elapsed().as_secs() > 30 => {
                    ui.label(RichText::new("Le simulateur n'a pas répondu. Fermez puis réessayez.").color(p.danger));
                }
                None => {
                    ui.label("Chargement du contenu…");
                }
            }
        }
    });
    if !open {
        state.contents = None;
    }
}
