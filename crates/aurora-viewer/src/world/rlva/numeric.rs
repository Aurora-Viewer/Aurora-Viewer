//! How the Legacy Viewer reads the numbers and keys of an RLVa command.
//!
//! Every RLVa option, reply channel and setting value goes through one of
//! these, so that a script gets the same answer in both viewers: the C and
//! C++ conversions the Legacy Viewer uses accept leading blanks, ignore what
//! follows the number and, for `std::stof`, read `inf` and `nan`, where a
//! Rust `parse` refuses all of them. Each function names the Legacy function
//! it follows. The Legacy Viewer runs in the "C" locale (it never calls
//! `setlocale` for numbers, only LLLocale around some interface text), so
//! the decimal point is always '.'.
//!
//! A value that is not finite (`inf`, `nan`) is accepted where the Legacy
//! Viewer accepts it, so that the command is stored and listed the same way;
//! the places that compute with it (camera, distances, effects) keep their
//! own guards, see `finite`.

use uuid::Uuid;

/// `isspace` in the "C" locale.
fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

fn skip_space(text: &str) -> &str {
    let at = text.bytes().take_while(|b| is_space(*b)).count();
    &text[at..]
}

/// LLStringUtil::trim (indra/llcommon/llstring.h).
pub fn trim(text: &str) -> &str {
    let text = skip_space(text);
    let end = text.len() - text.bytes().rev().take_while(|b| is_space(*b)).count();
    &text[..end]
}

/// The leading integer of `text` as `strtol` reads it in base 10: blanks,
/// a sign, digits. None without a digit; Err on overflow.
fn leading_integer(text: &str) -> Option<Result<i64, ()>> {
    let text = skip_space(text);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let count = digits.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 {
        return None;
    }
    let mut value: i64 = 0;
    for b in digits[..count].bytes() {
        value = match value.checked_mul(10).and_then(|v| v.checked_add(i64::from(b - b'0'))) {
            Some(v) if v <= i64::from(u32::MAX) + 1 => v,
            _ => return Some(Err(())),
        };
    }
    Some(Ok(if negative { -value } else { value }))
}

/// `std::stoi` (RlvCommandOptionHelper::parseOption<int>, rlvhelper.cpp,
/// and RlvBehaviourModifier::convertOptionValue): the leading integer,
/// whatever follows it; none without a digit or out of range.
pub fn stoi(text: &str) -> Option<i32> {
    leading_integer(text)?.ok().and_then(|v| i32::try_from(v).ok())
}

/// LLStringUtil::convertToS32 (indra/llcommon/llstring.h): trimmed, then read
/// by `istream >> S32`, which stops at the first character that is not part
/// of the number and fails out of range.
pub fn convert_to_s32(text: &str) -> Option<i32> {
    if text.is_empty() {
        return None;
    }
    stoi(trim(text))
}

/// LLStringUtil::convertToU32 (indra/llcommon/llstring.h): `istream >> U32`,
/// which reads a minus sign as the C library's `strtoul` does, by
/// negating the value modulo 2^32.
pub fn convert_to_u32(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    let value = leading_integer(trim(text))?.ok()?;
    if value < 0 {
        u32::try_from(-value).ok().map(u32::wrapping_neg)
    } else {
        u32::try_from(value).ok()
    }
}

/// LLStringUtil::convertToBOOL (indra/llcommon/llstring.h).
pub fn convert_to_bool(text: &str) -> Option<bool> {
    match trim(text) {
        "1" | "T" | "t" | "TRUE" | "true" | "True" => Some(true),
        "0" | "F" | "f" | "FALSE" | "false" | "False" => Some(false),
        _ => None,
    }
}

/// RlvCommandOptionHelper::parseOption<bool> (rlvhelper.cpp): a number
/// first, as `std::stoi` reads it, valid as 0 or 1 only; without a number,
/// `istream >> std::boolalpha`, which reads "true" or "false" after blanks.
pub fn option_bool(text: &str) -> Option<bool> {
    match leading_integer(text) {
        Some(Ok(0)) => Some(false),
        Some(Ok(1)) => Some(true),
        Some(_) => None,
        None => {
            let text = skip_space(text);
            if text.starts_with("true") {
                Some(true)
            } else if text.starts_with("false") {
                Some(false)
            } else {
                None
            }
        }
    }
}

/// The length of the decimal number at the start of `text` (sign, digits,
/// point, exponent), as `strtod` delimits it: 0 when there is no digit.
fn decimal_length(text: &[u8]) -> usize {
    let mut at = usize::from(matches!(text.first(), Some(b'+' | b'-')));
    let whole = text[at..].iter().take_while(|b| b.is_ascii_digit()).count();
    at += whole;
    let mut fraction = 0;
    if text.get(at) == Some(&b'.') {
        fraction = text[at + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
        if whole + fraction > 0 {
            at += 1 + fraction;
        }
    }
    if whole + fraction == 0 {
        return 0;
    }
    if matches!(text.get(at), Some(b'e' | b'E')) {
        let mut exp = at + 1;
        exp += usize::from(matches!(text.get(exp), Some(b'+' | b'-')));
        let digits = text[exp..].iter().take_while(|b| b.is_ascii_digit()).count();
        if digits > 0 {
            at = exp + digits;
        }
    }
    at
}

/// A hexadecimal float (`0x1.8p3`) after its sign, as C99 `strtof` reads
/// it: the value and the length read, None when no hex digit follows "0x".
fn hex_float(text: &[u8]) -> Option<(f64, usize)> {
    if !(text.len() > 2 && text[0] == b'0' && matches!(text[1], b'x' | b'X')) {
        return None;
    }
    let mut at = 2;
    let mut mantissa = 0f64;
    let mut exponent: i64 = 0;
    let mut digits = 0;
    while let Some(d) = text.get(at).and_then(|b| (*b as char).to_digit(16)) {
        mantissa = mantissa * 16.0 + f64::from(d);
        digits += 1;
        at += 1;
    }
    if text.get(at) == Some(&b'.') {
        let mut point = at + 1;
        let mut fraction = 0;
        while let Some(d) = text.get(point).and_then(|b| (*b as char).to_digit(16)) {
            mantissa = mantissa * 16.0 + f64::from(d);
            exponent -= 4;
            fraction += 1;
            point += 1;
        }
        if digits + fraction > 0 {
            at = point;
            digits += fraction;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(text.get(at), Some(b'p' | b'P')) {
        let mut exp = at + 1;
        let negative = text.get(exp) == Some(&b'-');
        exp += usize::from(matches!(text.get(exp), Some(b'+' | b'-')));
        let count = text[exp..].iter().take_while(|b| b.is_ascii_digit()).count();
        if count > 0 {
            let value = text[exp..exp + count]
                .iter()
                .fold(0i64, |v, b| v.saturating_mul(10).saturating_add(i64::from(b - b'0')));
            exponent = exponent.saturating_add(if negative { -value } else { value });
            at = exp + count;
        }
    }
    let exponent = i32::try_from(exponent.clamp(-4000, 4000)).unwrap_or_default();
    Some((mantissa * 2f64.powi(exponent), at))
}

/// What `strtof` / `strtod` read at the start of `text` (after blanks): the
/// value and whether the text was a number. `inf`, `infinity` and `nan`
/// (any case, `nan(...)` too) and hex floats are numbers.
fn strtod(text: &str) -> Option<(f64, bool)> {
    let text = skip_space(text).as_bytes();
    let (negative, body) = match text.first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let sign = if negative { -1.0 } else { 1.0 };
    // Called for numeric rule options on every frame (name tag distances,
    // IM ranges): nothing here allocates.
    let starts = |word: &[u8]| body.get(..word.len()).is_some_and(|head| head.eq_ignore_ascii_case(word));
    if starts(b"inf") {
        return Some((sign * f64::INFINITY, true));
    }
    if starts(b"nan") {
        return Some((f64::NAN, true));
    }
    if let Some((value, _)) = hex_float(body) {
        return Some((sign * value, value != 0.0));
    }
    let length = decimal_length(text);
    if length == 0 {
        return None;
    }
    // Rust reads `.5`, `-.5`, `+.5` and `5.` as strtod does; the lone point
    // it refuses never gets here (decimal_length is 0 without a digit).
    let value = std::str::from_utf8(&text[..length]).ok()?.parse::<f64>().ok()?;
    let nonzero = text[..length]
        .iter()
        .take_while(|b| !matches!(b, b'e' | b'E'))
        .any(|b| matches!(b, b'1'..=b'9'));
    Some((value, nonzero))
}

/// `std::stof` (RlvCommandOptionHelper::parseOption<float> and
/// RlvBehaviourModifier::convertOptionValue, rlvhelper.cpp): `strtof`, and
/// out of range (ERANGE: too large, or a non-zero value too small for a
/// normal float) is an error.
pub fn stof(text: &str) -> Option<f32> {
    let (value, literal) = strtod(text)?;
    let float = value as f32;
    let finite_literal = value.is_finite();
    if finite_literal && float.is_infinite() {
        return None;
    }
    if finite_literal && literal && float.abs() < f32::MIN_POSITIVE {
        return None;
    }
    Some(float)
}

/// LLStringUtil::convertToF64 then convertToF32 (indra/llcommon/llstring.h):
/// trimmed, read by `istream >> F64` (decimal only: no `inf`, `nan` or hex,
/// failing out of range), then within ±F32_MAX.
pub fn convert_to_f32(text: &str) -> Option<f32> {
    if text.is_empty() {
        return None;
    }
    let text = trim(text);
    let length = decimal_length(text.as_bytes());
    if length == 0 {
        return None;
    }
    let value = strtod(&text[..length])?.0;
    (value.is_finite() && value.abs() <= f64::from(f32::MAX)).then_some(value as f32)
}

/// `sscanf("%f/%f/...")` (RlvCommandOptionHelper::parseOption<LLVector3>,
/// RlvBehaviourModifier::convertOptionValue): each field as `strtof` reads
/// it after blanks, a '/' right after it, and whatever follows the last field
/// ignored. `sscanf` does not fail out of range: a huge value is infinite.
pub fn scan_floats<const N: usize>(text: &str) -> Option<[f32; N]> {
    scan::<N>(text).map(|v| v.map(|v| v as f32))
}

/// `sscanf("%lf/%lf/%lf")` (RlvCommandOptionHelper::parseOption<LLVector3d>).
pub fn scan_doubles<const N: usize>(text: &str) -> Option<[f64; N]> {
    scan::<N>(text)
}

fn scan<const N: usize>(text: &str) -> Option<[f64; N]> {
    if text.is_empty() {
        return None;
    }
    let mut values = [0f64; N];
    let mut rest = text;
    for (i, slot) in values.iter_mut().enumerate() {
        if i > 0 {
            rest = rest.strip_prefix('/')?;
        }
        let start = skip_space(rest);
        let length = float_length(start.as_bytes())?;
        *slot = strtod(&start[..length])?.0;
        rest = &start[length..];
    }
    Some(values)
}

/// How much of `text` (blanks already skipped) `strtod` reads.
fn float_length(text: &[u8]) -> Option<usize> {
    let sign = usize::from(matches!(text.first(), Some(b'+' | b'-')));
    let body = &text[sign..];
    let lower: Vec<u8> = body.iter().take(8).map(u8::to_ascii_lowercase).collect();
    if lower.starts_with(b"infinity") {
        return Some(sign + 8);
    }
    if lower.starts_with(b"inf") {
        return Some(sign + 3);
    }
    if lower.starts_with(b"nan") {
        let mut at = 3;
        if body.get(3) == Some(&b'(')
            && let Some(close) = body[4..].iter().position(|b| *b == b')')
            && body[4..4 + close].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            at = 4 + close + 1;
        }
        return Some(sign + at);
    }
    if let Some((_, length)) = hex_float(body) {
        return Some(sign + length);
    }
    let length = decimal_length(text);
    (length > 0).then_some(length)
}

/// LLUUID::set(text, false) (indra/llcommon/lluuid.cpp), which
/// RlvCommandOptionHelper::parseOption<LLUUID> uses: an empty text is the
/// null key; otherwise 36 characters (or 35, the old form without the last
/// dash) whose hexadecimal digits are read in place. The dash positions are
/// skipped without being checked. 32 digits alone, braces or a `urn:` are
/// refused.
pub fn uuid_set(text: &str) -> Option<Uuid> {
    if text.is_empty() {
        return Some(Uuid::nil());
    }
    let bytes = text.as_bytes();
    let broken = match bytes.len() {
        36 => false,
        35 => true,
        _ => return None,
    };
    let mut data = [0u8; 16];
    let mut at = 0;
    for (i, byte) in data.iter_mut().enumerate() {
        if matches!(i, 4 | 6 | 8) || (i == 10 && !broken) {
            at += 1;
        }
        let high = (bytes[at] as char).to_digit(16)?;
        let low = (bytes[at + 1] as char).to_digit(16)?;
        *byte = u8::try_from(high << 4 | low).ok()?;
        at += 2;
    }
    Some(Uuid::from_bytes(data))
}

/// LLUUID::parseUUID (indra/llcommon/lluuid.cpp), which
/// RlvBehaviourModifier::convertOptionValue uses for a key: not empty,
/// trimmed, then LLUUID::validate and set.
pub fn parse_uuid(text: &str) -> Option<Uuid> {
    if text.is_empty() {
        return None;
    }
    let text = trim(text);
    if text.is_empty() {
        return None;
    }
    uuid_set(text)
}

/// A number read the Legacy way, kept only when it is finite: for the
/// places that compute with it, where `inf` or `nan` would break the
/// maths. The Legacy Viewer passes them on.
pub fn finite(value: Option<f32>) -> Option<f32> {
    value.filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_follow_stoi_and_convert_to_s32() {
        assert_eq!(stoi("5"), Some(5));
        assert_eq!(stoi(" 5abc"), Some(5));
        assert_eq!(stoi("+7"), Some(7));
        assert_eq!(stoi("-12"), Some(-12));
        assert_eq!(stoi("0x10"), Some(0));
        assert_eq!(stoi(""), None);
        assert_eq!(stoi("abc"), None);
        assert_eq!(stoi("- 5"), None);
        assert_eq!(stoi("2147483647"), Some(i32::MAX));
        assert_eq!(stoi("2147483648"), None);
        assert_eq!(stoi("-2147483648"), Some(i32::MIN));
        assert_eq!(stoi("99999999999999999999"), None);
        assert_eq!(convert_to_s32(" 123 "), Some(123));
        assert_eq!(convert_to_s32("123abc"), Some(123));
        assert_eq!(convert_to_s32("\t-4\n"), Some(-4));
        assert_eq!(convert_to_s32(""), None);
        assert_eq!(convert_to_s32("   "), None);
    }

    #[test]
    fn unsigned_reads_a_minus_like_strtoul() {
        assert_eq!(convert_to_u32(" 2 "), Some(2));
        assert_eq!(convert_to_u32("4294967295"), Some(u32::MAX));
        assert_eq!(convert_to_u32("4294967296"), None);
        assert_eq!(convert_to_u32("-1"), Some(u32::MAX));
        assert_eq!(convert_to_u32("3x"), Some(3));
        assert_eq!(convert_to_u32("x3"), None);
    }

    #[test]
    fn booleans() {
        for text in ["1", "t", "T", "true", "TRUE", "True", " t "] {
            assert_eq!(convert_to_bool(text), Some(true), "{text}");
        }
        for text in ["0", "f", "F", "false", "FALSE", "False"] {
            assert_eq!(convert_to_bool(text), Some(false), "{text}");
        }
        for text in ["", "yes", "tRue", "2"] {
            assert_eq!(convert_to_bool(text), None, "{text}");
        }
        assert_eq!(option_bool("1"), Some(true));
        assert_eq!(option_bool("1abc"), Some(true));
        assert_eq!(option_bool("-0"), Some(false));
        assert_eq!(option_bool("2"), None);
        assert_eq!(option_bool("-1"), None);
        assert_eq!(option_bool(" true"), Some(true));
        assert_eq!(option_bool("falsey"), Some(false));
        assert_eq!(option_bool("t"), None);
    }

    #[test]
    fn floats_follow_stof() {
        assert_eq!(stof("1.5"), Some(1.5));
        assert_eq!(stof(" 2.5m"), Some(2.5));
        assert_eq!(stof("5."), Some(5.0));
        assert_eq!(stof(".5"), Some(0.5));
        assert_eq!(stof("-.5"), Some(-0.5));
        assert_eq!(stof("+.5"), Some(0.5));
        assert_eq!(stof("+5."), Some(5.0));
        assert_eq!(stof("5.e1"), Some(50.0));
        assert_eq!(stof(".5e-1"), Some(0.05));
        assert_eq!(stof("-."), None);
        assert_eq!(stof("1e2"), Some(100.0));
        assert_eq!(stof("1e"), Some(1.0));
        assert_eq!(stof("1e+"), Some(1.0));
        assert_eq!(stof("0x1.8p1"), Some(3.0));
        assert_eq!(stof("0x10"), Some(16.0));
        assert_eq!(stof("0x"), Some(0.0));
        assert_eq!(stof("inf"), Some(f32::INFINITY));
        assert_eq!(stof("-Infinity"), Some(f32::NEG_INFINITY));
        assert_eq!(stof("INF"), Some(f32::INFINITY));
        assert_eq!(stof("in"), None);
        assert!(stof("nan").is_some_and(f32::is_nan));
        assert!(stof("NaN(1)").is_some_and(f32::is_nan));
        assert_eq!(stof("1e39"), None);
        assert_eq!(stof("1e-50"), None);
        assert_eq!(stof("0e-50"), Some(0.0));
        assert_eq!(stof("0"), Some(0.0));
        assert_eq!(stof(""), None);
        assert_eq!(stof("."), None);
        assert_eq!(stof("abc"), None);
        assert_eq!(stof("5,5"), Some(5.0));
    }

    #[test]
    fn convert_to_f32_is_decimal_only() {
        assert_eq!(convert_to_f32(" 1.25 "), Some(1.25));
        assert_eq!(convert_to_f32("12cm"), Some(12.0));
        assert_eq!(convert_to_f32("inf"), None);
        assert_eq!(convert_to_f32("nan"), None);
        assert_eq!(convert_to_f32("1e39"), None);
        assert_eq!(convert_to_f32("1e309"), None);
        assert_eq!(convert_to_f32(""), None);
    }

    #[test]
    fn vectors_follow_sscanf() {
        assert_eq!(scan_floats::<3>("1/2/3"), Some([1.0, 2.0, 3.0]));
        assert_eq!(scan_floats::<3>(" 1/ 2/ 3 junk"), Some([1.0, 2.0, 3.0]));
        assert_eq!(scan_floats::<3>("1/2/3/4"), Some([1.0, 2.0, 3.0]));
        assert_eq!(scan_floats::<3>("1 /2/3"), None);
        assert_eq!(scan_floats::<3>("1/2"), None);
        assert_eq!(scan_floats::<3>(""), None);
        assert_eq!(scan_floats::<3>("1/x/3"), None);
        let [x, ..] = scan_floats::<3>("1e39/0/0").unwrap();
        assert!(x.is_infinite());
        assert!(scan_floats::<3>("nan/inf/-infinity").is_some());
        assert_eq!(scan_floats::<4>("1/2/3/4"), Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(scan_doubles::<3>("1.5/2/3"), Some([1.5, 2.0, 3.0]));
    }

    #[test]
    fn keys_follow_lluuid_set() {
        let id = "01234567-89ab-cdef-0123-456789abcdef";
        let expected = Uuid::parse_str(id).unwrap();
        assert_eq!(uuid_set(id), Some(expected));
        assert_eq!(uuid_set(&id.to_uppercase()), Some(expected));
        assert_eq!(uuid_set(""), Some(Uuid::nil()));
        // The old form without the last dash.
        assert_eq!(uuid_set("01234567-89ab-cdef-0123456789abcdef"), Some(expected));
        // The dashes are not checked.
        assert_eq!(uuid_set("01234567x89abxcdefx0123x456789abcdef"), Some(expected));
        assert_eq!(uuid_set("0123456789abcdef0123456789abcdef"), None);
        assert_eq!(uuid_set("{01234567-89ab-cdef-0123-456789abcdef}"), None);
        assert_eq!(uuid_set("urn:uuid:01234567-89ab-cdef-0123-456789abcdef"), None);
        assert_eq!(uuid_set("g1234567-89ab-cdef-0123-456789abcdef"), None);
        assert_eq!(uuid_set(" 01234567-89ab-cdef-0123-456789abcde"), None);
        assert_eq!(parse_uuid(&format!(" {id} ")), Some(expected));
        assert_eq!(parse_uuid(""), None);
        assert_eq!(parse_uuid("  "), None);
    }
}
