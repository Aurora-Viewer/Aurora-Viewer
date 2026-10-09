//! The `rolld` chat command: dice rolls with modifiers.
//!
//! Port of the FSCmdLineRollDice branch of cmd_line_chat
//! (indra/newview/chatbar_as_cmdline.cpp, Firestorm), with the French texts
//! of its strings.xml (FSCmdLineRollDice*). Syntax: `rolld [dice faces
//! [modifier value]]`, 1 d6 without arguments; modifiers `+`, `-` (bonus,
//! penalty), `>`, `<` (successes), `!`, `!>`, `!<` (exploding), `!p`, `!p>`,
//! `!p<` (penetrating), `r`, `r>`, `r<` (reroll).

use super::scan::Scanner;

const MODIFIERS: [&str; 13] = ["+", "-", "<", ">", "!>", "!<", "!", "!p", "!p>", "!p<", "r", "r>", "r<"];

/// Rolls the dice described by `args` (the text after the command) and
/// returns the lines to report. `roll(n)` returns a random number in `0..n`.
pub fn roll_dice(command: &str, args: &str, mut roll: impl FnMut(u32) -> u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut die = |faces: i32| 1 + i32::try_from(roll(faces.unsigned_abs())).unwrap_or(0);
    let mut scan = Scanner::new(args);
    let (dice, faces, result, modifier_text);
    if let (Some(d), Some(f)) = (scan.int(), scan.int()) {
        if !(1..=100).contains(&d) || !(1..=1000).contains(&f) {
            out.push("Vous devez fournir des valeurs positives pour les dés (max 100) et les faces (max 1000).".into());
            return out;
        }
        (dice, faces) = (d, f);
        let modifier_type = scan.word().map(str::to_lowercase).unwrap_or_default();
        let modifier = if modifier_type.is_empty() { Some(0) } else { scan.int() };
        let modifier = match modifier {
            Some(m) if (-1000..=1000).contains(&m) && (modifier_type.is_empty() || MODIFIERS.contains(&modifier_type.as_str())) => m,
            _ => {
                out.push(invalid_modifiers(command));
                return out;
            }
        };
        let m = modifier_type.as_str();
        let mut total = 0;
        let mut successes = 0;
        let mut die_iter = 1;
        let mut freeze_guard = 0;
        let mut penetrated = false;
        while die_iter <= dice {
            // Each die may have a different value rolled.
            let mut value = die(faces);
            if penetrated {
                value -= 1;
                penetrated = false;
                out.push(format!("#{die_iter} 1d{faces}-1: {value}."));
            } else {
                out.push(format!("#{die_iter} 1d{faces}: {value}."));
            }
            total += value;
            die_iter += 1;

            let hit = |exact: &str, above: &str, below: &str| {
                (m == exact && value == modifier) || (m == above && value >= modifier) || (m == below && value <= modifier)
            };
            if m == "<" || m == ">" {
                if (m == "<" && value <= modifier) || (m == ">" && value >= modifier) {
                    out.push("  ^-- Succès".into());
                    successes += 1;
                } else {
                    total -= value;
                }
            } else if hit("!", "!>", "!<") {
                out.push("  ^-- Explosé".into());
                die_iter -= 1;
            } else if hit("!p", "!p>", "!p<") {
                out.push("  ^-- Pénétré".into());
                penetrated = true;
                die_iter -= 1;
            } else if hit("r", "r>", "r<") {
                total -= value;
                out.push("  ^-- Relancer".into());
                die_iter -= 1;
            }

            freeze_guard += 1;
            if freeze_guard > 1000 {
                // Probably an endless loop ("rolld 1 6 !> 0").
                out.push(
                    "Cette opération ne peut être réalisée car le client se figerait dans une boucle infinie. \
                     Veuillez modifier les critères."
                        .into(),
                );
                return out;
            }
        }
        match m {
            "+" => total += modifier,
            "-" => total -= modifier,
            "<" | ">" => out.push(format!("Succès: {successes}")),
            _ => {}
        }
        modifier_text = if m.is_empty() { String::new() } else { format!("{m}{modifier}") };
        result = total;
    } else {
        // Roll a standard die when no parameters were provided.
        (dice, faces, modifier_text) = (1, 6, String::new());
        result = die(6);
    }
    out.push(format!("Résultat total pour {dice}d{faces}{modifier_text}: {result}."));
    out
}

fn invalid_modifiers(command: &str) -> String {
    format!(
        "Si vous souhaitez utiliser un modificateur, vous devez fournir un numéro de modificateur valide et le type de \
         modificateur. Le modificateur doit être compris entre -1000 et 1000. Les types de modificateurs valides sont \
         \"+\" (bonus), \"-\" (pénalités), \">\", \"<\" (succès), \"r>\", \"r<\", \"r\" (relancer), \"!p>\", \"!p<\", \
         \"!p\" (pénétrations), \"!>\", \"!<\" and \"!\" (explosions). Vous ne pouvez utiliser qu'un seul type de \
         modificateur par tirage. Exemples : \"{command} 1 20 + 5\", \"{command} 5 40 > 15\", \"{command} 10 25 ! 25\", \
         \"{command} 15 25 !< 10\"."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed sequence of rolls (each value is the die result minus one).
    fn seq(values: &[u32]) -> impl FnMut(u32) -> u32 + '_ {
        let mut it = values.iter().cycle();
        move |_| *it.next().unwrap_or(&0)
    }

    #[test]
    fn default_and_plain_rolls() {
        assert_eq!(roll_dice("rolld", "", seq(&[3])), vec!["Résultat total pour 1d6: 4."]);
        // "2d20" is not "2 20": istream reads 2 then fails on "d20".
        assert_eq!(roll_dice("rolld", "2d20", seq(&[0])), vec!["Résultat total pour 1d6: 1."]);
        assert_eq!(
            roll_dice("rolld", "2 20 + 5", seq(&[9, 14])),
            vec!["#1 1d20: 10.", "#2 1d20: 15.", "Résultat total pour 2d20+5: 30."]
        );
    }

    #[test]
    fn successes_and_rerolls() {
        assert_eq!(
            roll_dice("rolld", "2 20 > 10", seq(&[14, 2])),
            vec![
                "#1 1d20: 15.",
                "  ^-- Succès",
                "#2 1d20: 3.",
                "Succès: 1",
                "Résultat total pour 2d20>10: 15."
            ]
        );
        assert_eq!(
            roll_dice("rolld", "1 6 r 1", seq(&[0, 4])),
            vec!["#1 1d6: 1.", "  ^-- Relancer", "#1 1d6: 5.", "Résultat total pour 1d6r1: 5."]
        );
        assert_eq!(
            roll_dice("rolld", "1 6 !p 6", seq(&[5, 3])),
            vec!["#1 1d6: 6.", "  ^-- Pénétré", "#1 1d6-1: 3.", "Résultat total pour 1d6!p6: 9."]
        );
    }

    #[test]
    fn limits_and_errors() {
        assert_eq!(roll_dice("rolld", "0 6", seq(&[0])).len(), 1);
        assert!(roll_dice("rolld", "1 6 +5", seq(&[0]))[0].starts_with("Si vous souhaitez"));
        assert!(roll_dice("rolld", "1 6 x 5", seq(&[0]))[0].contains("\"rolld 1 20 + 5\""));
        let endless = roll_dice("rolld", "1 6 !> 0", seq(&[0]));
        assert!(endless.last().is_some_and(|l| l.contains("boucle infinie")));
    }
}
