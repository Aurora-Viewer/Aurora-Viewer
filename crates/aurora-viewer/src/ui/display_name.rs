//! "Définir le nom d'affichage", ported from LLFloaterDisplayName (Second
//! Life viewer and Firestorm, originally LGPL 2.1, Linden Research, Inc.): the new name
//! typed twice, locked for a week after a change, reset to the username.

use super::widgets::{Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};

/// Characters, not bytes.
const DISPLAY_NAME_MAX_LENGTH: usize = 31;

#[derive(Default)]
pub struct DisplayNameUi {
    pub open: bool,
    name: String,
    confirm: String,
    error: Option<String>,
}

/// Change to send: (old display name, new one; "" = back to the username).
pub type SetDisplayName = (String, String);

/// "le 15/10/2026 à 14:30 SLT" (Firestorm LockOutDateFormat, in SLT).
fn slt_datetime(secs: f64) -> String {
    let secs = secs as i64;
    // SLT is US Pacific time; DST approximated by day of year like the top bar
    let year_day = (secs / 86400).rem_euclid(365);
    let off = if (68..=307).contains(&year_day) { -7 } else { -8 };
    let t = secs + off * 3600;
    let days = t.div_euclid(86400);
    let tod = t.rem_euclid(86400);
    // civil date from days since 1970-01-01 (H. Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}/{m:02}/{y} à {:02}:{:02} SLT", tod / 3600, (tod / 60) % 60)
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl DisplayNameUi {
    /// onOpen: empty fields.
    pub fn open(&mut self) {
        self.open = true;
        self.name.clear();
        self.confirm.clear();
        self.error = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World) -> Option<SetDisplayName> {
        let screen = ctx.content_rect();
        let mut out = None;
        let mut open = self.open;
        Floater::new(
            "display_name",
            "Définir le nom d'affichage",
            egui::pos2(screen.center().x - 200.0, screen.center().y - 150.0),
            Vec2::new(400.0, 300.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            let names = &world.social.avatar_names;
            let me = names.get(&world.agent_id).cloned();
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Le nom que vous choisissez d'utiliser pour votre avatar est appelé le nom d'affichage. Vous pouvez le changer une fois par semaine.",
                    )
                    .size(12.0)
                    .color(p.ink),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            let Some(me) = me else {
                ui.label(RichText::new("Votre nom n'est pas encore connu…").size(12.0).color(p.muted));
                return;
            };
            ui.label(
                RichText::new(format!("Nom actuel : {}  ({})", me.display_name, me.username))
                    .size(12.0)
                    .color(p.muted),
            );
            let locked = now() < me.next_update;
            if locked {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("Vous ne pouvez changer votre nom avant le {}.", slt_datetime(me.next_update)))
                        .size(12.0)
                        .color(p.danger),
                );
            }
            ui.add_space(8.0);
            ui.add_enabled_ui(!locked, |ui| {
                ui.label(RichText::new("Nouveau nom :").size(12.0).color(p.ink));
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                ui.add_space(4.0);
                ui.label(RichText::new("Réécrivez le nom :").size(12.0).color(p.ink));
                ui.add(egui::TextEdit::singleline(&mut self.confirm).desired_width(f32::INFINITY));
            });
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                ui.add(egui::Label::new(RichText::new(e).size(12.0).color(p.danger)).wrap());
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                // FIRE-33330: the reset is allowed during the lock-out week
                if flat_button(ui, p, "Réinitialiser")
                    .on_hover_text("Utiliser le nom d'utilisateur comme nom d'affichage")
                    .clicked()
                {
                    out = Some((me.display_name.clone(), String::new()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, p, "Annuler").clicked() {
                        self.open = false;
                    }
                    let save = ui.add_enabled_ui(!locked, |ui| flat_button(ui, p, "Enregistrer").on_hover_text("Enregistre votre nouveau nom")).inner;
                    if save.clicked() {
                        // onSave
                        let name = self.name.trim().to_owned();
                        let user_name = if me.legacy_last.is_empty() || me.legacy_last == "Resident" {
                            me.legacy_first.clone()
                        } else {
                            format!("{} {}", me.legacy_first, me.legacy_last)
                        };
                        if name != self.confirm.trim() {
                            self.error = Some("Non-concordance des noms d'affichage saisis. Effectuez une nouvelle saisie.".into());
                        } else if name.is_empty() {
                            self.error = Some("Saisissez un nom.".into());
                        } else if name == user_name {
                            // a reset
                            out = Some((me.display_name.clone(), String::new()));
                        } else if name.chars().count() > DISPLAY_NAME_MAX_LENGTH {
                            self.error = Some(format!(
                                "Le nom saisi est trop long. Le nombre de caractères maximum est de {DISPLAY_NAME_MAX_LENGTH}.\n\nVeuillez essayer avec un nom plus court."
                            ));
                        } else {
                            out = Some((me.display_name.clone(), name));
                        }
                    }
                });
            });
        });
        if out.is_some() {
            open = false;
        }
        self.open = open && self.open;
        out
    }
}

/// Text of the answer to a display name change (onCacheSetName): the
/// welcome message, or the error.
pub fn reply_message(status: i32, content: &aurora_llsd::Llsd) -> Result<String, String> {
    if status == 200 {
        let name = content["display_name"].to_string_value();
        return Ok(format!(
            "Bonjour {name},\n\nComme dans la vie réelle, il faut quelque temps aux gens pour qu'ils se familiarisent avec un nouveau nom. Veuillez compter quelques jours avant la mise à jour de votre nom au niveau des objets, scripts, recherches, etc."
        ));
    }
    // known error tags (notifications.xml)
    let tag = content["error_tag"].to_string_value();
    let known = match tag.as_str() {
        "SetDisplayNameBlocked" => {
            Some("Impossible de changer de nom d'affichage. Si vous pensez qu'il s'agit d'une erreur, contactez l'Assistance du réseau.")
        }
        "SetDisplayNameMismatch" => Some("Non-concordance des noms d'affichage saisis. Effectuez une nouvelle saisie."),
        "AgentDisplayNameUpdateThresholdExceeded" => Some(
            "Le délai au bout duquel vous pouvez changer de nom d'affichage n'est pas encore écoulé.\n\nVoir http://wiki.secondlife.com/wiki/Setting_your_display_name",
        ),
        "AgentDisplayNameSetBlocked" => {
            Some("Impossible de définir le nom demandé car il contient un terme interdit.\n\nVeuillez essayer avec un nom différent.")
        }
        "AgentDisplayNameSetInvalidUnicode" => Some("Le nom d'affichage que vous souhaitez définir contient des caractères non valides."),
        "AgentDisplayNameSetOnlyPunctuation" => {
            Some("Votre nom d'affichage doit contenir des lettres autres que des signes de ponctuation.")
        }
        _ => None,
    };
    if let Some(m) = known {
        return Err(m.to_owned());
    }
    // the server's own message, in French when it has one
    let desc = &content["error_description"];
    for lang in ["fr", "en"] {
        let m = desc[lang].to_string_value();
        if !m.is_empty() {
            return Err(m);
        }
    }
    Err("Impossible de définir votre nom d'affichage. Veuillez réessayer ultérieurement.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockout_date_in_slt() {
        // 2026-10-15 21:30 UTC = 14:30 PDT
        assert_eq!(slt_datetime(1_792_099_800.0), "15/10/2026 à 14:30 SLT");
    }
}
