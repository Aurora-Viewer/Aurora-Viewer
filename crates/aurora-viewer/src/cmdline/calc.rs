//! Expression evaluator of the `calc` chat command.
//!
//! Port of LLCalc / LLCalcParser (indra/llmath/llcalc.cpp, llcalcparser.h,
//! originally LGPL 2.1): the same Boost.Spirit grammar rewritten as a
//! recursive descent parser, evaluated in `f32` like the C++ code. Trig
//! functions take and return degrees (`SIN`, `ASIN`…), their `…R` variants
//! work in radians (FIRE-14618). `RAND(min,max)` is expanded beforehand by
//! [`expand_rand`], as in the `calc` branch of cmd_line_chat.

use std::f32::consts::PI;

const DEG_TO_RAD: f32 = PI / 180.0;
const RAD_TO_DEG: f32 = 180.0 / PI;

/// Constants of LLCalc::LLCalc (llmath.h values).
fn constant(name: &str) -> Option<f32> {
    Some(match name {
        "PI" => PI,
        "TWO_PI" => 2.0 * PI,
        "PI_BY_TWO" => PI / 2.0,
        "SQRT_TWO_PI" => 2.506_628_3,
        "SQRT2" => std::f32::consts::SQRT_2,
        "SQRT3" => 1.732_050_8,
        "DEG_TO_RAD" => DEG_TO_RAD,
        "RAD_TO_DEG" => RAD_TO_DEG,
        "GRAVITY" => -9.8,
        _ => return None,
    })
}

/// Evaluates `expr` like LLCalc::evalString: case-insensitive, spaces
/// ignored between tokens, an optional leading `=`. `None` on any syntax or
/// domain error (Firestorm then says "Calculation Failed").
pub fn eval(expr: &str) -> Option<f32> {
    let upper = expr.to_uppercase();
    let mut p = Parser {
        s: upper.as_bytes(),
        pos: 0,
    };
    p.eat(b'=');
    let value = p.expression()?;
    p.skip_ws();
    (p.pos == p.s.len()).then_some(value)
}

/// Formats a result like `std::ostream << F32` (default `%g`, 6 significant
/// digits), which Firestorm prints after `<expr> = `.
pub fn format_result(v: f32) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let v = f64::from(v);
    // Exponent after rounding to 6 significant digits, as %g decides.
    let sci = format!("{v:.5e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..6).contains(&exp) {
        let decimals = usize::try_from(5 - exp).unwrap_or(0);
        trim_zeros(&format!("{v:.decimals$}")).to_string()
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim_zeros(mantissa), exp.unsigned_abs())
    }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// Replaces up to five `RAND(min,max)` of an upper-cased expression, as the
/// `calc` command does before LLCalc. `roll(n)` returns a random number in
/// `0..n`. Returns the new expression and the invalid ranges (max ≤ min or
/// outside ±10000, replaced by 0) for FSCmdLineCalcRandError.
pub fn expand_rand(expr: &str, mut roll: impl FnMut(u32) -> u32) -> (String, Vec<String>) {
    let mut expr = expr.to_string();
    let mut errors = Vec::new();
    // Firestorm loops at most five times (performance), even when a match
    // can't be replaced because atoi() changed its spelling ("RAND(05,9)").
    for _ in 0..5 {
        let Some((min_txt, max_txt)) = find_rand(&expr) else { break };
        let (min, max) = (atoi(&min_txt), atoi(&max_txt));
        let look_for = format!("RAND({min},{max})");
        let Some(at) = expr.find(&look_for) else { continue };
        let range = -10_000..=10_000;
        let number = if max > min && range.contains(&min) && range.contains(&max) {
            min + i32::try_from(roll(u32::try_from(max - min + 1).unwrap_or(1))).unwrap_or(0)
        } else {
            errors.push(look_for.clone());
            0
        };
        expr.replace_range(at..at + look_for.len(), &number.to_string());
    }
    (expr, errors)
}

/// First match of `RAND\(([0-9-]+),([0-9-]+)\)`: the two captured groups.
fn find_rand(expr: &str) -> Option<(String, String)> {
    let is_arg = |c: char| c.is_ascii_digit() || c == '-';
    let mut from = 0;
    while let Some(rel) = expr[from..].find("RAND(") {
        let start = from + rel + "RAND(".len();
        let rest = &expr[start..];
        let a_len = rest.find(|c: char| !is_arg(c)).unwrap_or(rest.len());
        if a_len > 0 && rest[a_len..].starts_with(',') {
            let rest_b = &rest[a_len + 1..];
            let b_len = rest_b.find(|c: char| !is_arg(c)).unwrap_or(rest_b.len());
            if b_len > 0 && rest_b[b_len..].starts_with(')') {
                return Some((rest[..a_len].to_string(), rest_b[..b_len].to_string()));
            }
        }
        from = start;
    }
    None
}

/// C `atoi`: optional sign then digits, 0 when there are none.
fn atoi(s: &str) -> i32 {
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    let n = digits[..end]
        .parse::<i64>()
        .unwrap_or(0)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let n = i32::try_from(n).unwrap_or(0);
    if neg { n.wrapping_neg() } else { n }
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.s.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.s.get(self.pos).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// expression = term (('+' | '-') term)*
    fn expression(&mut self) -> Option<f32> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Some(v);
            }
        }
    }

    /// term = power (('*' | '/' | '%') power)*
    fn term(&mut self) -> Option<f32> {
        let mut v = self.power()?;
        loop {
            if self.eat(b'*') {
                v *= self.power()?;
            } else if self.eat(b'/') {
                v /= self.power()?;
            } else if self.eat(b'%') {
                v %= self.power()?;
            } else {
                return Some(v);
            }
        }
    }

    /// power = unary ('^' unary)*, left-associative like the Spirit rule.
    fn power(&mut self) -> Option<f32> {
        let mut v = self.unary()?;
        while self.eat(b'^') {
            v = v.powf(self.unary()?);
        }
        Some(v)
    }

    /// unary = '+'? factor | '-' factor
    fn unary(&mut self) -> Option<f32> {
        if self.eat(b'-') {
            return self.factor().map(|v| -v);
        }
        self.eat(b'+');
        self.factor()
    }

    /// factor = number | group | function | constant, NaN = domain error.
    fn factor(&mut self) -> Option<f32> {
        let v = match self.peek()? {
            b'0'..=b'9' | b'.' => self.number()?,
            b'(' => {
                self.pos += 1;
                let v = self.expression()?;
                self.eat(b')').then_some(v)?
            }
            c if c.is_ascii_alphabetic() || c == b'_' => self.identifier()?,
            _ => return None,
        };
        (!v.is_nan()).then_some(v)
    }

    /// Unsigned real (Spirit `ureal_p`): `1`, `1.`, `.5`, `1.5E-3`.
    fn number(&mut self) -> Option<f32> {
        let start = self.pos;
        let digits = |p: &mut Self| {
            let from = p.pos;
            while p.s.get(p.pos).is_some_and(u8::is_ascii_digit) {
                p.pos += 1;
            }
            p.pos - from
        };
        let int = digits(self);
        let mut frac = 0;
        if self.s.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            frac = digits(self);
        }
        if int + frac == 0 {
            return None;
        }
        // The exponent only counts when digits follow it.
        if self.s.get(self.pos) == Some(&b'E') {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.s.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if digits(self) == 0 {
                self.pos = save;
            }
        }
        std::str::from_utf8(&self.s[start..self.pos]).ok()?.parse().ok()
    }

    /// A function call or a constant (LLCalcParser::lookup).
    fn identifier(&mut self) -> Option<f32> {
        let start = self.pos;
        while self.s.get(self.pos).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.s[start..self.pos]).ok()?;
        if !self.eat(b'(') {
            return constant(name);
        }
        let a = self.expression()?;
        let v = match name {
            "ATAN2" | "MIN" | "MAX" => {
                if !self.eat(b',') {
                    return None;
                }
                let b = self.expression()?;
                match name {
                    "ATAN2" => a.atan2(b),
                    "MIN" => a.min(b),
                    _ => a.max(b),
                }
            }
            "SIN" => (DEG_TO_RAD * a).sin(),
            "COS" => (DEG_TO_RAD * a).cos(),
            "TAN" => (DEG_TO_RAD * a).tan(),
            "ASIN" => a.asin() * RAD_TO_DEG,
            "ACOS" => a.acos() * RAD_TO_DEG,
            "ATAN" => a.atan() * RAD_TO_DEG,
            "SINR" => a.sin(),
            "COSR" => a.cos(),
            "TANR" => a.tan(),
            "ASINR" => a.asin(),
            "ACOSR" => a.acos(),
            "ATANR" => a.atan(),
            "SQRT" => a.sqrt(),
            "LOG" => a.ln(),
            "EXP" => a.exp(),
            "ABS" => a.abs(),
            "FLR" => a.floor(),
            "CEIL" => a.ceil(),
            _ => return None,
        };
        self.eat(b')').then_some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(s: &str) -> Option<String> {
        eval(s).map(format_result)
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(calc("2+2").as_deref(), Some("4"));
        assert_eq!(calc("2 + 3 * 4").as_deref(), Some("14"));
        assert_eq!(calc("(2+3)*4").as_deref(), Some("20"));
        assert_eq!(calc("1/3").as_deref(), Some("0.333333"));
        assert_eq!(calc("7 % 3").as_deref(), Some("1"));
        assert_eq!(calc("=2^10").as_deref(), Some("1024"));
        // Unary minus binds tighter than ^, as in the Spirit grammar.
        assert_eq!(calc("-2^2").as_deref(), Some("4"));
        assert_eq!(calc("2^-1").as_deref(), Some("0.5"));
        assert_eq!(calc("1e3 + .5").as_deref(), Some("1000.5"));
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(calc("sin(2+2)").as_deref(), Some("0.0697565"));
        assert_eq!(calc("SINR(PI/2)").as_deref(), Some("1"));
        assert_eq!(calc("atan2(1, 1)").as_deref(), Some("0.785398"));
        assert_eq!(calc("max(3, min(10, 7))").as_deref(), Some("7"));
        assert_eq!(calc("sqrt2 * sqrt2").as_deref(), Some("2"));
        assert_eq!(calc("flr(2.7) + ceil(0.2)").as_deref(), Some("3"));
        assert_eq!(calc("acos(0)").as_deref(), Some("90"));
    }

    #[test]
    fn errors() {
        assert_eq!(calc(""), None);
        assert_eq!(calc("2+"), None);
        assert_eq!(calc("foo"), None);
        assert_eq!(calc("sin(1"), None);
        assert_eq!(calc("2 2"), None);
        assert_eq!(calc("--2"), None);
        // NaN from a function is a domain error.
        assert_eq!(calc("sqrt(-1)"), None);
        assert_eq!(calc("1/0").as_deref(), Some("inf"));
    }

    #[test]
    fn g_formatting() {
        assert_eq!(format_result(1_000_000.0), "1e+06");
        assert_eq!(format_result(123_456.0), "123456");
        assert_eq!(format_result(0.0001), "0.0001");
        assert_eq!(format_result(0.000_012_5), "1.25e-05");
        assert_eq!(format_result(-2.5), "-2.5");
    }

    #[test]
    fn rand_expansion() {
        let (e, err) = expand_rand("RAND(1,6)+RAND(10,20)", |n| n - 1);
        assert_eq!(e, "6+20");
        assert!(err.is_empty());
        let (e, err) = expand_rand("RAND(5,1)", |_| 0);
        assert_eq!(e, "0");
        assert_eq!(err, vec!["RAND(5,1)".to_string()]);
        // atoi() changes the spelling: left as is, LLCalc then fails on it.
        let (e, _) = expand_rand("RAND(05,9)", |_| 0);
        assert_eq!(e, "RAND(05,9)");
        assert_eq!(atoi("-12"), -12);
        assert_eq!(atoi("3-4"), 3);
    }
}
