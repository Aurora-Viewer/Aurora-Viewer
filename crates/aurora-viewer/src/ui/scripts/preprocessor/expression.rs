//! C preprocessor integer expressions, as evaluated by Firestorm Boost.Wave.
//! Parse inactive operands too, but short-circuit their arithmetic errors.

use super::lex::{Kind, Token};

pub(super) fn evaluate(tokens: &[Token]) -> Result<bool, String> {
    let mut parser = Parser { tokens, at: 0 };
    let result = parser.expression(0, true)?;
    if parser.at != tokens.len() {
        return Err(aurora_i18n::tr!("script-preproc-expression").into());
    }
    Ok(result != 0)
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

fn error() -> String {
    aurora_i18n::tr!("script-preproc-expression").into()
}

impl Parser<'_> {
    fn expression(&mut self, min: u8, active: bool) -> Result<i64, String> {
        let token = self.tokens.get(self.at).ok_or_else(error)?;
        self.at += 1;
        let mut left = match token.text.as_str() {
            "(" => {
                let n = self.expression(0, active)?;
                self.expect(")")?;
                n
            }
            "+" => self.expression(12, active)?,
            "-" => self.expression(12, active)?.wrapping_neg(),
            "!" => i64::from(self.expression(12, active)? == 0),
            "~" => !self.expression(12, active)?,
            _ if token.kind == Kind::Word => 0,
            _ if token.kind == Kind::String && token.text.starts_with('\'') => {
                let body = token.text.trim_matches('\'');
                match body {
                    "\\n" => 10,
                    "\\r" => 13,
                    "\\t" => 9,
                    "\\0" => 0,
                    _ if body.chars().count() == 1 => body.chars().next().expect("one char") as i64,
                    _ => return Err(error()),
                }
            }
            _ => number(&token.text)?,
        };
        while let Some(op) = self.tokens.get(self.at).map(|t| t.text.as_str()) {
            if op == "?" && min == 0 {
                self.at += 1;
                let yes = self.expression(0, active && left != 0)?;
                self.expect(":")?;
                let no = self.expression(0, active && left == 0)?;
                left = if left != 0 { yes } else { no };
                continue;
            }
            let Some(precedence) = precedence(op) else { break };
            if precedence < min {
                break;
            }
            self.at += 1;
            let right_active = active && !(op == "&&" && left == 0 || op == "||" && left != 0);
            let right = self.expression(precedence + 1, right_active)?;
            left = if !active { 0 } else { apply(op, left, right)? };
        }
        Ok(left)
    }
    fn expect(&mut self, text: &str) -> Result<(), String> {
        if self.tokens.get(self.at).is_some_and(|t| t.text == text) {
            self.at += 1;
            Ok(())
        } else {
            Err(error())
        }
    }
}

fn number(text: &str) -> Result<i64, String> {
    let text = text.trim_end_matches(['u', 'U', 'l', 'L']);
    let parsed = if let Some(n) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u64::from_str_radix(n, 16)
    } else if let Some(n) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        u64::from_str_radix(n, 2)
    } else if text.len() > 1 && text.starts_with('0') {
        u64::from_str_radix(text, 8)
    } else {
        text.parse::<u64>()
    };
    parsed.map(|n| n as i64).map_err(|_| error())
}

fn precedence(op: &str) -> Option<u8> {
    Some(match op {
        "||" => 1,
        "&&" => 2,
        "|" => 3,
        "^" => 4,
        "&" => 5,
        "==" | "!=" => 6,
        "<" | ">" | "<=" | ">=" => 7,
        "<<" | ">>" => 8,
        "+" | "-" => 9,
        "*" | "/" | "%" => 10,
        _ => return None,
    })
}

fn apply(op: &str, a: i64, b: i64) -> Result<i64, String> {
    Ok(match op {
        "||" => i64::from(a != 0 || b != 0),
        "&&" => i64::from(a != 0 && b != 0),
        "|" => a | b,
        "^" => a ^ b,
        "&" => a & b,
        "==" => i64::from(a == b),
        "!=" => i64::from(a != b),
        "<" => i64::from(a < b),
        ">" => i64::from(a > b),
        "<=" => i64::from(a <= b),
        ">=" => i64::from(a >= b),
        "<<" | ">>" if !(0..64).contains(&b) => return Err(error()),
        "<<" => a.wrapping_shl(b as u32),
        ">>" => a.wrapping_shr(b as u32),
        "+" => a.wrapping_add(b),
        "-" => a.wrapping_sub(b),
        "*" => a.wrapping_mul(b),
        "/" | "%" if b == 0 => return Err(error()),
        "/" => a.wrapping_div(b),
        "%" => a.wrapping_rem(b),
        _ => return Err(error()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditions_use_c_precedence_short_circuit_and_checked_errors() {
        for text in [
            "1+2*3==7",
            "(5 & 3)==1",
            "0 || 1",
            "1 || (1/0)",
            "0 ? 1/0 : 2",
            "'\\n'==10",
            "0x10UL==020",
            "1<<3==8",
        ] {
            assert!(evaluate(&super::super::lex::code(text)).unwrap(), "{text}");
        }
        assert!(!evaluate(&super::super::lex::code("0 && 1/0")).unwrap());
        for text in ["1/0", "1<<99", "1 +", "(1", "1.5", "1 2"] {
            assert!(evaluate(&super::super::lex::code(text)).is_err(), "{text}");
        }
    }
}
