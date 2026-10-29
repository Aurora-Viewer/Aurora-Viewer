//! The languages offered for translation.

/// A language: its ISO 639-1 code, its English name and its own name.
#[derive(Debug, PartialEq, Eq)]
pub struct Lang {
    pub code: &'static str,
    pub english: &'static str,
    pub native: &'static str,
}

const fn l(code: &'static str, english: &'static str, native: &'static str) -> Lang {
    Lang { code, english, native }
}

/// The languages of `/tr` and of the conversation pill.
pub static LANGS: &[Lang] = &[
    l("en", "English", "English"),
    l("es", "Spanish", "Español"),
    l("fr", "French", "Français"),
    l("de", "German", "Deutsch"),
    l("it", "Italian", "Italiano"),
    l("pt", "Portuguese", "Português"),
    l("nl", "Dutch", "Nederlands"),
    l("pl", "Polish", "Polski"),
    l("ru", "Russian", "Русский"),
    l("uk", "Ukrainian", "Українська"),
    l("tr", "Turkish", "Türkçe"),
    l("ar", "Arabic", "العربية"),
    l("he", "Hebrew", "עברית"),
    l("fa", "Persian", "فارسی"),
    l("hi", "Hindi", "हिन्दी"),
    l("bn", "Bengali", "বাংলা"),
    l("id", "Indonesian", "Bahasa Indonesia"),
    l("vi", "Vietnamese", "Tiếng Việt"),
    l("th", "Thai", "ไทย"),
    l("zh", "Chinese", "中文"),
    l("ja", "Japanese", "日本語"),
    l("ko", "Korean", "한국어"),
    l("sv", "Swedish", "Svenska"),
    l("da", "Danish", "Dansk"),
    l("no", "Norwegian", "Norsk"),
    l("fi", "Finnish", "Suomi"),
    l("cs", "Czech", "Čeština"),
    l("el", "Greek", "Ελληνικά"),
    l("ro", "Romanian", "Română"),
    l("hu", "Hungarian", "Magyar"),
];

/// The language of an ISO 639-1 code.
pub fn lang(code: &str) -> Option<&'static Lang> {
    LANGS.iter().find(|l| l.code == code)
}

/// A language typed by the user: its code, English name or own name, in
/// any case and with or without accents (`es`, `Spanish`, `espanol`).
pub fn parse_lang(input: &str) -> Option<&'static Lang> {
    let key = fold(input.trim());
    if key.is_empty() {
        return None;
    }
    LANGS
        .iter()
        .find(|l| l.code == key || fold(l.english) == key || fold(l.native) == key)
}

/// Lower case without the accents of the Latin names above.
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'ế' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' | 'č' => 'c',
            'ș' => 's',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lang_accepts_codes_and_names() {
        for input in ["ES", "es", "spanish", "Español", "espanol", " Spanish "] {
            assert_eq!(parse_lang(input).map(|l| l.code), Some("es"), "{input}");
        }
        assert_eq!(parse_lang("japanese").map(|l| l.code), Some("ja"));
        assert_eq!(parse_lang("日本語").map(|l| l.code), Some("ja"));
        assert_eq!(parse_lang("klingon"), None);
        assert_eq!(parse_lang(""), None);
    }

    #[test]
    fn lang_by_code() {
        assert_eq!(lang("fr").map(|l| l.english), Some("French"));
        assert_eq!(LANGS.len(), 30);
    }
}
