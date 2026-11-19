//! Viewer-local object water surface directive.

const MARKER: &str = "[AURORA_WATER";

/// A single directive can stand alone or follow the creator's description.
pub(crate) fn face(description: &str) -> Option<u8> {
    let description = description.trim();
    let (text, directive) = description.split_once(MARKER)?;
    if (!text.is_empty() && !text.ends_with(char::is_whitespace)) || directive.contains(MARKER) {
        return None;
    }
    let directive = directive.strip_suffix(']')?;
    if directive.is_empty() {
        return Some(0);
    }
    let index = directive.strip_prefix(":face=")?;
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    index.parse().ok()
}

/// Replace only a recognized suffix. Give malformed markers back to the editor for manual correction.
pub(crate) fn with_face(description: &str, selected: Option<u8>) -> Result<String, &'static str> {
    let text = if face(description).is_some() {
        let (text, _) = description
            .split_once(MARKER)
            .ok_or_else(|| aurora_i18n::tr!("build-surface-water-invalid-marker"))?;
        text.strip_suffix(char::is_whitespace).unwrap_or(text)
    } else if description.contains(MARKER) {
        return Err(aurora_i18n::tr!("build-surface-water-invalid-marker"));
    } else {
        description
    };

    let Some(selected) = selected else {
        return Ok(text.to_owned());
    };
    let separator = if text.is_empty() { "" } else { " " };
    let updated = format!("{text}{separator}{MARKER}:face={selected}]");
    if updated.len() > 127 {
        return Err(aurora_i18n::tr!("build-surface-water-description-full"));
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::{face, with_face};

    #[test]
    fn editing_preserves_description_and_checks_capacity() {
        let description = "  Courtyard fountain  ";
        let marked = with_face(description, Some(2)).unwrap();
        assert_eq!(face(&marked), Some(2));
        let moved = with_face(&marked, Some(5)).unwrap();
        assert_eq!(face(&moved), Some(5));
        assert_eq!(with_face(&moved, None).unwrap(), description);
        let exact = with_face(&"a".repeat(105), Some(0)).unwrap();
        assert_eq!(exact.len(), 127);
        assert!(with_face(&"a".repeat(106), Some(0)).is_err());
        assert!(with_face("[AURORA_WATER:face=-1]", Some(0)).is_err());
    }

    #[test]
    fn accepts_default_and_explicit_faces() {
        assert_eq!(face("[AURORA_WATER]"), Some(0));
        assert_eq!(face("[AURORA_WATER:face=0]"), Some(0));
        assert_eq!(face(" [AURORA_WATER:face=5] "), Some(5));
        assert_eq!(face("Courtyard fountain [AURORA_WATER:face=2]"), Some(2));
        assert_eq!(face("Pool [AURORA_WATER]"), Some(0));
    }

    #[test]
    fn rejects_invalid_and_unrelated_descriptions() {
        for description in [
            "",
            "Water fountain",
            "Pool[AURORA_WATER]",
            "Pool [AURORA_WATER] decorated",
            "[AURORA_WATER] [AURORA_WATER:face=2]",
            "[AURORA_WATER:face=]",
            "[AURORA_WATER:face=-1]",
            "[AURORA_WATER:face=+1]",
            "[AURORA_WATER:face=256]",
            "[AURORA_WATER:face=0,type=volume]",
            "[AURORA_WATER][AURORA_WATER]",
        ] {
            assert_eq!(face(description), None, "{description}");
        }
    }
}
