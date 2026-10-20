//! "Hide residents' names": before a copy leaves the viewer, every name and
//! id of the residents it mentions becomes "Avatar 1", "Avatar 2"… in the
//! order they first appear, and ours becomes "Me". Object names, texture
//! and object ids stay: they are what the copy is for.

use uuid::Uuid;

/// A resident the copy may mention: their id and every name the log can
/// show for them (complete "Display (user.name)", display name, user name,
/// legacy "First Last").
#[derive(Debug, Clone)]
pub struct Resident {
    pub id: Uuid,
    pub names: Vec<String>,
}

/// Names shorter than this are left alone: they would match inside words.
const MIN_NAME: usize = 3;

/// `text` with the names and ids of `residents` replaced; `me` is our own
/// avatar.
pub fn anonymize(text: &str, residents: &[Resident], me: Option<Uuid>) -> String {
    // each resident's patterns, the longest first so that a complete name is
    // replaced before the display name it contains
    let mut patterns: Vec<(String, usize)> = Vec::new();
    for (i, r) in residents.iter().enumerate() {
        patterns.push((r.id.to_string(), i));
        patterns.push((r.id.to_string().to_uppercase(), i));
        for n in &r.names {
            if n.chars().count() >= MIN_NAME {
                patterns.push((n.clone(), i));
            }
        }
    }
    patterns.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
    patterns.dedup_by(|a, b| a.0 == b.0);
    if patterns.is_empty() {
        return text.to_owned();
    }
    // numbers in the order the residents first appear
    let mut numbers: Vec<Option<usize>> = vec![None; residents.len()];
    let mut next = 1;
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    'scan: while !rest.is_empty() {
        for (pattern, who) in &patterns {
            if rest.starts_with(pattern.as_str()) {
                let label = if me == Some(residents[*who].id) {
                    "Me".to_owned()
                } else {
                    let n = *numbers[*who].get_or_insert_with(|| {
                        next += 1;
                        next - 1
                    });
                    format!("Avatar {n}")
                };
                out.push_str(&label);
                rest = &rest[pattern.len()..];
                continue 'scan;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: u128, names: &[&str]) -> Resident {
        Resident {
            id: Uuid::from_u128(id),
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn names_become_numbers_in_order_of_appearance() {
        let orion = r(
            1,
            &["Orion Exemple (orion.exemple)", "Orion Exemple", "orion.exemple", "Loup Violet"],
        );
        let nova = r(2, &["Nova", "nova.exemple"]);
        let text = format!(
            "attachments of 'Nova' near 'Orion Exemple (orion.exemple)' (legacy name Loup Violet, {})",
            Uuid::from_u128(1)
        );
        assert_eq!(
            anonymize(&text, &[orion, nova], None),
            "attachments of 'Avatar 1' near 'Avatar 2' (legacy name Avatar 2, Avatar 2)"
        );
    }

    #[test]
    fn our_avatar_is_me_and_short_names_stay() {
        let me = r(7, &["Al", "Alice Doe"]);
        let out = anonymize("Alice Doe wears Al's hat; Al stays", &[me], Some(Uuid::from_u128(7)));
        assert_eq!(out, "Me wears Al's hat; Al stays");
    }

    #[test]
    fn text_without_residents_is_unchanged() {
        assert_eq!(anonymize("prim 3 face 0 -> Blend", &[], None), "prim 3 face 0 -> Blend");
    }
}
