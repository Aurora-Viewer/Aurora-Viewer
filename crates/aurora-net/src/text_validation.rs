//! Character policies used by restricted viewer fields and their mutations.
//! Port of LLTextValidate validators (indra/llui/lltextvalidate.cpp,
//! originally LGPL 2.1). Required names, lengths and numeric ranges are
//! checked separately: an empty field is a normal intermediate edit.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    Ascii,
    PrintableNoPipe,
    MultilineNoPipe,
    /// AO folder serialization also reserves the colon.
    AoName,
    Digits,
    /// Character prevalidation for decimal editors; final parsing and the
    /// selected decimal separator remain the numeric control's job.
    Decimal,
}

impl Policy {
    pub fn accepts(self, text: &str) -> bool {
        text.chars().all(|c| self.accepts_char(c))
    }

    pub fn accepts_char(self, c: char) -> bool {
        match self {
            Self::Ascii => (' '..='\u{7f}').contains(&c),
            Self::PrintableNoPipe => (' '..='~').contains(&c) && c != '|',
            Self::MultilineNoPipe => (c == '\n' || (' '..='\u{7f}').contains(&c)) && c != '|',
            Self::AoName => (' '..='~').contains(&c) && !matches!(c, '|' | ':'),
            Self::Digits => c.is_ascii_digit(),
            Self::Decimal => c.is_ascii_digit() || matches!(c, '-' | '.' | ',' | ' '),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Policy::*;

    #[test]
    fn restricted_text_preserves_each_reference_character_policy() {
        for policy in [Ascii, PrintableNoPipe, MultilineNoPipe, AoName, Digits, Decimal] {
            assert!(policy.accepts(""));
            for text in ["桜", "龙", "Café", "Ａ", "😀", "\t", "\0"] {
                assert!(!policy.accepts(text), "{policy:?}: {text:?}");
            }
        }
        assert!(Ascii.accepts(" A-z 012 !|:\u{7f}"));
        assert!(PrintableNoPipe.accepts(" A-z 012 !:"));
        assert!(!PrintableNoPipe.accepts("a|b"));
        assert!(!PrintableNoPipe.accepts("\u{7f}"));
        assert!(MultilineNoPipe.accepts("First\nSecond\u{7f}"));
        assert!(!MultilineNoPipe.accepts("First\r\nSecond"));
        assert!(!MultilineNoPipe.accepts("a|b"));
        assert!(!AoName.accepts("a:b"));
        assert!(!AoName.accepts("a|b"));
        assert!(Digits.accepts("0123456789"));
        assert!(Decimal.accepts("-12.5"));
        assert!(Decimal.accepts("-12,5"));
        assert!(!Decimal.accepts("1e3"));
        for text in ["１", "١", "1.5", "-1", "1 2", "1e3"] {
            assert!(!Digits.accepts(text));
        }
    }
}
