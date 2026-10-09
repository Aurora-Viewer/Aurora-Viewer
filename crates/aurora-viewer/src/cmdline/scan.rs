//! Token reader with the semantics of `std::istringstream >> value`, which
//! cmd_line_chat uses to read command arguments: whitespace is skipped, a
//! number is read as a prefix ("20+5" gives 20, then "+5" stays for the next
//! read), and the first failed read makes every later read fail too.

pub struct Scanner<'a> {
    s: &'a str,
    pos: usize,
    failed: bool,
}

impl<'a> Scanner<'a> {
    pub fn new(s: &'a str) -> Self {
        Self { s, pos: 0, failed: false }
    }

    fn skip_ws(&mut self) {
        let rest = &self.s[self.pos..];
        self.pos += rest.len() - rest.trim_start().len();
    }

    /// Length of the numeric prefix of the remaining text (after spaces).
    fn number_len(&self, real: bool) -> usize {
        let b = &self.s.as_bytes()[self.pos..];
        let digits_from = |mut i: usize| {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            i
        };
        let mut i = usize::from(matches!(b.first(), Some(b'+' | b'-')));
        let int_end = digits_from(i);
        let mut any = int_end > i;
        i = int_end;
        if real {
            if b.get(i) == Some(&b'.') {
                let frac_end = digits_from(i + 1);
                any |= frac_end > i + 1;
                i = frac_end;
            }
            if any && matches!(b.get(i), Some(b'e' | b'E')) {
                let mut j = i + 1;
                if matches!(b.get(j), Some(b'+' | b'-')) {
                    j += 1;
                }
                let exp_end = digits_from(j);
                if exp_end > j {
                    i = exp_end;
                }
            }
        }
        if any { i } else { 0 }
    }

    fn number<T: std::str::FromStr>(&mut self, real: bool) -> Option<T> {
        if self.failed {
            return None;
        }
        self.skip_ws();
        let len = self.number_len(real);
        let value = (len > 0).then(|| self.s[self.pos..self.pos + len].parse().ok()).flatten();
        match value {
            Some(v) => {
                self.pos += len;
                Some(v)
            }
            None => {
                self.failed = true;
                None
            }
        }
    }

    /// `i >> S32`.
    pub fn int(&mut self) -> Option<i32> {
        self.number(false)
    }

    /// `i >> F32`.
    pub fn float(&mut self) -> Option<f32> {
        self.number(true)
    }

    /// `i >> std::string`: the next run of non-space characters.
    pub fn word(&mut self) -> Option<&'a str> {
        if self.failed {
            return None;
        }
        self.skip_ws();
        let rest = &self.s[self.pos..];
        let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        if len == 0 {
            self.failed = true;
            return None;
        }
        self.pos += len;
        Some(&rest[..len])
    }

    /// `i >> LLUUID`: a word that must be a UUID.
    pub fn uuid(&mut self) -> Option<uuid::Uuid> {
        let id = self.word().and_then(|w| uuid::Uuid::parse_str(w).ok());
        if id.is_none() {
            self.failed = true;
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_like_istream() {
        let mut s = Scanner::new("  2 20+5 x");
        assert_eq!(s.int(), Some(2));
        assert_eq!(s.int(), Some(20));
        assert_eq!(s.word(), Some("+5"));
        assert_eq!(s.int(), None);
        // Failure is sticky, as with failbit.
        assert_eq!(s.word(), None);

        let mut s = Scanner::new("128.5 -3 .5e1 abc");
        assert_eq!(s.float(), Some(128.5));
        assert_eq!(s.float(), Some(-3.0));
        assert_eq!(s.float(), Some(5.0));
        assert_eq!(s.float(), None);

        let mut s = Scanner::new("2d20");
        assert_eq!(s.int(), Some(2));
        assert_eq!(s.int(), None);
    }
}
