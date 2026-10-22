//! Attachment naming from RlvAttachPtLookup and the balanced-parenthesis
//! helpers in rlvlocks.cpp / rlvhelper.cpp (originally LGPL 2.1).
use super::inventory::point;

fn first(text: &str) -> Option<(usize, &str)> {
    let start = text.find('(')?;
    let mut depth = 0usize;
    for (offset, byte) in text.as_bytes()[start..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some((start, &text[start + 1..start + offset]));
                }
            }
            _ => {}
        }
    }
    None
}

fn last(text: &str) -> Option<&str> {
    let end = text.rfind(')')?;
    let mut depth = 0usize;
    for (index, byte) in text.as_bytes()[..=end].iter().enumerate().rev() {
        match byte {
            b')' => depth += 1,
            b'(' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[index + 1..end]);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn item_point(name: &str) -> Option<u8> {
    point(last(name)?.trim())
}

pub(super) fn folder_point(name: &str, legacy: bool) -> Option<u8> {
    let first = first(name);
    let value = if !legacy {
        let (start, value) = first?;
        if start != 1 || !name.starts_with('.') {
            return None;
        }
        value
    } else if let Some((start, value)) = first {
        // Preserve the reference's leading-block precedence, including its
        // index test for a block immediately after the first character.
        if start != 0 && (start != 1 || name.starts_with('.')) && start + value.len() + 1 != name.len() {
            last(name)?
        } else {
            value
        }
    } else {
        name.strip_prefix('.').unwrap_or(name)
    };
    point(value.trim())
}

pub(super) fn folded(name: &str, legacy: bool) -> bool {
    name == ".(nostrip)" || folder_point(name, legacy).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlva_attachment_names_use_balanced_blocks_and_native_legacy_precedence() {
        assert_eq!(item_point("Bracelet (old version) ( Right Hand )"), point("right hand"));
        assert_eq!(item_point("(chest) (unmatched"), point("chest"));
        assert_eq!(item_point("name ((chest))"), None);
        assert_eq!(item_point("étiquette (Chest) trailing"), point("chest"));
        assert_eq!(folder_point("Outfit (old) ( Skull )", true), point("skull"));
        assert_eq!(folder_point("(chest) (skull)", true), point("chest"));
        assert_eq!(folder_point("x(chest) (skull)", true), point("chest"));
        for name in ["Chest", ".chest", "(Chest)", "Outfit (chest)", ".( Chest ) trailing"] {
            assert!(folded(name, true), "{name}");
        }
        assert_eq!(folder_point(".( Chest ) trailing", false), point("chest"));
        for name in ["Chest", ".chest", "(Chest)", "Outfit (chest)", "x(chest)"] {
            assert!(!folded(name, false), "{name}");
        }
        for name in ["", "(", ")", "é((", ".(chest", ".((chest))", ".(chest))"] {
            assert!(folder_point(name, true).is_none(), "{name}");
        }
        assert!(folded(".(nostrip)", false));
    }
}
