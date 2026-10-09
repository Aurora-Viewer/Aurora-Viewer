//! Purchase and object-payment dialogs. Behavior follows LLFloaterBuy and
//! LLFloaterPay (Firestorm indra/newview, originally LGPL 2.1).
use super::widgets::{Floater, flat_button};
use crate::{
    interaction::{Action, Interactions},
    theme::Palette,
    world::World,
};
use egui::{RichText, Vec2};

/// None = no confirmation; Some(None) = buy; Some(Some(amount)) = pay.
pub fn show(ctx: &egui::Context, p: &Palette, state: &mut Interactions, world: &World) -> Option<Option<i32>> {
    let d = state.dialog.as_mut()?;
    let buy = d.target.action == Action::Buy;
    let mut open = true;
    let mut cancel = false;
    let mut confirm = None;
    let center = ctx.content_rect().center();
    Floater::new(
        "object_transaction",
        if buy { "Acheter l'objet" } else { "Payer l'objet" },
        egui::pos2(center.x - 190.0, center.y - 130.0),
        Vec2::new(380.0, 260.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        if world.objects.index_of_uuid(&d.target.object).is_none() {
            ui.label(RichText::new("Cet objet n'est plus disponible.").color(p.danger));
        } else if let Some(props) = &d.props {
            ui.label(RichText::new(&props.name).strong().color(p.ink));
            let seller = world
                .social
                .avatar_names
                .complete(&props.owner_id)
                .or_else(|| world.avatar_name(&props.owner_id))
                .unwrap_or_else(|| "Nom en cours de chargement…".into());
            ui.label(RichText::new(format!("Propriétaire : {seller}")).color(p.muted));
            if !props.description.is_empty() {
                ui.label(&props.description);
            }
            ui.add_space(8.0);
            let affordable = |amount: i32| amount >= 0 && world.balance.is_some_and(|b| amount <= b);
            if buy {
                let mode = match props.sale_type {
                    1 => "Original",
                    2 => "Copie",
                    3 => "Contenu",
                    _ => "Pas en vente",
                };
                ui.label(format!("{mode} — L$ {}", props.sale_price));
                let valid = (1..=3).contains(&props.sale_type) && affordable(props.sale_price);
                if ui
                    .add_enabled_ui(valid, |ui| flat_button(ui, p, &format!("Acheter pour L$ {}", props.sale_price)))
                    .inner
                    .clicked()
                {
                    confirm = Some(None);
                }
                if !affordable(props.sale_price) {
                    ui.label(RichText::new("Solde insuffisant ou encore inconnu.").color(p.danger));
                }
            } else if let Some((default, buttons)) = &d.prices {
                if *default != -1 {
                    ui.horizontal(|ui| {
                        ui.label("Montant L$ :");
                        ui.add(egui::TextEdit::singleline(&mut d.amount).desired_width(90.0).char_limit(10));
                        let amount = d.amount.trim().parse::<i32>().ok().filter(|v| *v > 0);
                        if ui
                            .add_enabled_ui(amount.is_some_and(affordable), |ui| flat_button(ui, p, "Payer"))
                            .inner
                            .clicked()
                        {
                            confirm = Some(amount);
                        }
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    for &amount in buttons.iter().filter(|v| **v > 0) {
                        if ui
                            .add_enabled_ui(affordable(amount), |ui| flat_button(ui, p, &format!("Payer L$ {amount}")))
                            .inner
                            .clicked()
                        {
                            confirm = Some(Some(amount));
                        }
                    }
                });
            } else {
                ui.label("Chargement des montants proposés…");
            }
        } else {
            ui.label("Chargement des informations de l'objet…");
        }
        if d.opened.elapsed().as_secs() > 15 && (d.props.is_none() || (!buy && d.prices.is_none())) {
            ui.label(RichText::new("Le simulateur n'a pas répondu. Fermez puis réessayez.").color(p.danger));
        }
        if let Some(error) = &d.error {
            ui.label(RichText::new(error).color(p.danger));
        }
        ui.add_space(10.0);
        if flat_button(ui, p, "Annuler").clicked() {
            cancel = true;
        }
    });
    if !open || cancel {
        state.dialog = None;
        confirm = None;
    }
    confirm
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
