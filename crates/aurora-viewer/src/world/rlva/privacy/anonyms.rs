//! The stand-ins of hidden names: the `anonyms` arrays of the Legacy
//! Viewer's rlva_strings.xml (indra/newview/skins/default/xui/<language>/,
//! originally LGPL 2.1), kept as data and not as interface messages.
//! RlvStrings::initClass (indra/newview/rlvcommon.cpp) loads the English file
//! and the file of the interface language (LLDir::findSkinnedFilenames with
//! ALL_SKINS) and RlvStrings::loadFromFile appends the anonyms of each: in
//! French the list is the 28 English stand-ins followed by the 28 French
//! ones, and RlvStrings::getAnonym picks in all 56. A language whose file has
//! no `anonyms` (de, es, pt) or no file at all keeps the English 28.

const EN: [&str; 28] = [
    "A resident",
    "This resident",
    "That resident",
    "An individual",
    "This individual",
    "That individual",
    "A person",
    "This person",
    "That person",
    "A stranger",
    "This stranger",
    "That stranger",
    "A being",
    "This being",
    "That being",
    "An agent",
    "This agent",
    "That agent",
    "A soul",
    "This soul",
    "That soul",
    "Somebody",
    "Some people",
    "Someone",
    "Mysterious one",
    "An unknown being",
    "Unidentified one",
    "An unknown person",
];

/// xui/fr/rlva_strings.xml, its repeated entries included.
const FR: [&str; 28] = [
    "Un résident",
    "Ce résident",
    "Ce résident",
    "Un individu",
    "Cet individu",
    "Cet individu",
    "Une personne",
    "Cette personne",
    "Cette personne",
    "Un étranger",
    "Cet étranger",
    "Cet étranger",
    "Un être",
    "Cet être",
    "Cet être",
    "Un agent",
    "Cet agent",
    "Cet agent",
    "Une âme",
    "Cette âme",
    "Cette âme",
    "Quelqu'un",
    "Certaines personnes",
    "Quelqu'un",
    "Le mystérieux",
    "Un être inconnu",
    "Une personne non identifiée",
    "Une personne inconnue",
];

/// xui/ja/rlva_strings.xml.
const JA: [&str; 28] = [
    "住民",
    "この住民",
    "その住民",
    "人",
    "この人",
    "その人",
    "人物",
    "この人物",
    "その人物",
    "知らない人",
    "この知らない人",
    "その知らない人",
    "存在",
    "この存在",
    "その存在",
    "エージェント",
    "このエージェント",
    "そのエージェント",
    "魂",
    "この魂",
    "その魂",
    "誰か",
    "ある人",
    "何者か",
    "不思議ななにか",
    "未知の存在",
    "正体不明のもの",
    "謎の人物",
];

/// xui/ru/rlva_strings.xml.
const RU: [&str; 28] = [
    "Житель",
    "Этот житель",
    "Тот житель",
    "Индивидум",
    "Этот индивидум",
    "Тот индивидум",
    "Персона",
    "Эта персона",
    "Та персона",
    "Незнакомец",
    "Этот незнакомец",
    "Тот незнакомец",
    "Существо",
    "Это существо",
    "То существо",
    "Деятель",
    "Этот деятель",
    "Тот деятель",
    "Человек",
    "Этот человек",
    "Тот человек",
    "Кто-то",
    "Некоторые люди",
    "Некто",
    "Таинственный",
    "Неизвестное существо",
    "Неопознанный",
    "Неизвестный человек",
];

/// xui/zh/rlva_strings.xml, the Legacy Viewer's one Chinese interface: both
/// of Aurora's Chinese catalogues are its `zh` (`aurora_i18n::grid_code`).
const ZH: [&str; 28] = [
    "一位居民",
    "這位居民",
    "那位居民",
    "一位個體",
    "這位個體",
    "那位個體",
    "一個人",
    "這個人",
    "那個人",
    "一個陌生人",
    "這位陌生人",
    "那位陌生人",
    "一個生物",
    "這個生物",
    "那個生物",
    "一個使用者",
    "這個使用者",
    "那個使用者",
    "一個靈魂",
    "這個靈魂",
    "那個靈魂",
    "某人",
    "一些人",
    "有人",
    "神秘的人",
    "一個未知的生物",
    "身份不明的人",
    "一個未知的人",
];

/// xui/pl/rlva_strings.xml, seven stand-ins only.
const PL: [&str; 7] = [
    "Rezydent",
    "Osoba",
    "Nieznajomy",
    "Ktoś",
    "Tajemniczy ktoś",
    "Ktoś niezidentyfikowany",
    "Nieznana osoba",
];

/// xui/az/rlva_strings.xml.
const AZ: [&str; 28] = [
    "Qəsəbə sakini",
    "Bu qəsəbə sakini",
    "O qəsəbə sakini",
    "Birey",
    "Bu birey",
    "O birey",
    "Şəxs",
    "Bu şəxs",
    "O şəxs",
    "Naməlum",
    "Bu naməlum",
    "O naməlum",
    "Məxluq",
    "Bu məxluq",
    "O məxluq",
    "Fəaliyyətçi",
    "Bu fəaliyyətçi",
    "O fəaliyyətçi",
    "İnsan",
    "Bu insan",
    "O insan",
    "Birisi",
    "Bəzi insanlar",
    "Kimsə",
    "Müəmmalı",
    "Naməlum məxluq",
    "Təyin edilməmiş",
    "Naməlum insan",
];

/// The list RlvStrings::getAnonym indexes for the interface language
/// `language` (a catalogue code: `fr`, `zh-Hant`…): English, then the
/// language's own stand-ins when its Legacy file has some.
pub(super) fn list(language: &str) -> Vec<&'static str> {
    let own: &[&str] = match aurora_i18n::grid_code(language) {
        "fr" => &FR,
        "ja" => &JA,
        "ru" => &RU,
        "zh" => &ZH,
        "pl" => &PL,
        "az" => &AZ,
        _ => &[],
    };
    EN.iter().chain(own).copied().collect()
}
