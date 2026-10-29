//! Advanced inventory search: a tree of rules joined by All / Any and
//! negated by Not, built from menus, never typed as an expression. The
//! Legacy Viewer has no such search: its filters (LLInventoryFilter,
//! indra/newview/llinventoryfilter.cpp) are all ANDed and its text is a
//! plain substring. Each of those filters is a rule here, so they can be
//! ORed or negated; quick search keeps the Legacy Viewer behaviour.
//!
//! Text rules compile once per query to `regex` patterns built from the
//! escaped value, so a value never acts as a pattern of its own.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::view::Age;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Field {
    #[default]
    Name,
    Description,
    Creator,
    Uuid,
    /// The name of the folder holding the item.
    Folder,
    Type,
    Permissions,
    Worn,
    Link,
    Coalesced,
    Date,
}

impl Field {
    pub const ALL: [Field; 11] = [
        Field::Name,
        Field::Description,
        Field::Creator,
        Field::Uuid,
        Field::Folder,
        Field::Type,
        Field::Permissions,
        Field::Worn,
        Field::Link,
        Field::Coalesced,
        Field::Date,
    ];

    pub fn is_text(self) -> bool {
        matches!(
            self,
            Field::Name | Field::Description | Field::Creator | Field::Uuid | Field::Folder
        )
    }

    /// Whether a folder can satisfy the rule: folders have a name, an id and
    /// a parent, nothing else (the Legacy Viewer's folders match by name).
    fn applies_to_folders(self) -> bool {
        matches!(self, Field::Name | Field::Uuid | Field::Folder)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Op {
    #[default]
    Contains,
    /// A whole word, like quick search's `"word"`.
    Is,
    StartsWith,
    EndsWith,
    /// `*` any run of characters, `?` one character; the whole value.
    Pattern,
    /// Ends in a number, after the value if one is given.
    EndsInNumber,
    /// Holds a version (`v2`, `1.2`, `v1.2.3`), after the value if given.
    HasVersion,
}

impl Op {
    pub const ALL: [Op; 7] = [
        Op::Contains,
        Op::Is,
        Op::StartsWith,
        Op::EndsWith,
        Op::Pattern,
        Op::EndsInNumber,
        Op::HasVersion,
    ];

    /// Whether the rule needs a value to mean anything.
    pub fn needs_value(self) -> bool {
        !matches!(self, Op::EndsInNumber | Op::HasVersion)
    }
}

/// One condition. Text fields use `op` and `text`; Type and Permissions
/// use `bits` (inventory types, or the required M/C/T rights); Date uses
/// `age` and `hours`; Worn, Link and Coalesced are true or false, `not`
/// turning them around.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    pub field: Field,
    pub op: Op,
    pub text: String,
    pub bits: u32,
    pub age: Age,
    pub hours: u32,
    pub not: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            field: Field::Name,
            op: Op::Contains,
            text: String::new(),
            bits: 0,
            age: Age::Newer,
            hours: 24,
            not: false,
        }
    }
}

impl Rule {
    pub fn text(field: Field, op: Op, text: &str) -> Self {
        Self {
            field,
            op,
            text: text.to_string(),
            ..Self::default()
        }
    }

    pub fn of(field: Field) -> Self {
        Self { field, ..Self::default() }
    }

    /// A rule that would match everything is left out of its group: an
    /// empty text, no type or right chosen, or Date with no age.
    pub fn is_blank(&self) -> bool {
        match self.field {
            f if f.is_text() => self.op.needs_value() && self.text.trim().is_empty(),
            Field::Type | Field::Permissions => self.bits == 0,
            Field::Date => self.age == Age::Any,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Node {
    Rule(Rule),
    Group(Group),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Group {
    /// Any rule matches (OR); otherwise all of them (AND).
    pub any: bool,
    pub not: bool,
    pub nodes: Vec<Node>,
}

impl Group {
    fn is_blank(&self) -> bool {
        self.nodes.iter().all(|node| match node {
            Node::Rule(rule) => rule.is_blank(),
            Node::Group(group) => group.is_blank(),
        })
    }

    fn uses(&self, field: Field) -> bool {
        self.nodes.iter().any(|node| match node {
            Node::Rule(rule) => rule.field == field && !rule.is_blank(),
            Node::Group(group) => group.uses(field),
        })
    }

    fn only_folder_fields(&self) -> bool {
        self.nodes.iter().all(|node| match node {
            Node::Rule(rule) => rule.is_blank() || rule.field.applies_to_folders(),
            Node::Group(group) => group.only_folder_fields(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SmartQuery {
    pub root: Group,
}

impl SmartQuery {
    pub fn is_empty(&self) -> bool {
        self.root.is_blank()
    }

    pub fn uses(&self, field: Field) -> bool {
        self.root.uses(field)
    }

    /// The advanced search a quick search stands for, so switching keeps
    /// what was typed: `a+b` is all of its words, `"word"` a whole word,
    /// the All field any of the text fields.
    pub fn from_quick(text: &str, field: super::view::SearchField) -> Self {
        use super::view::SearchField;
        let text = text.trim();
        let mut query = Self::default();
        if text.is_empty() {
            return query;
        }
        let words: Vec<(Op, &str)> = match text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
            Some(exact) => vec![(Op::Is, exact)],
            None => text
                .split('+')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(|t| (Op::Contains, t))
                .collect(),
        };
        let fields: &[Field] = match field {
            SearchField::Name => &[Field::Name],
            SearchField::Description => &[Field::Description],
            SearchField::Creator => &[Field::Creator],
            SearchField::Uuid => &[Field::Uuid],
            SearchField::All => &[Field::Name, Field::Description, Field::Creator, Field::Uuid],
        };
        for (op, word) in words {
            if let [field] = fields {
                query.root.nodes.push(Node::Rule(Rule::text(*field, op, word)));
            } else {
                query.root.nodes.push(Node::Group(Group {
                    any: true,
                    not: false,
                    nodes: fields.iter().map(|f| Node::Rule(Rule::text(*f, op, word))).collect(),
                }));
            }
        }
        query
    }
}

/// What a rule can look at, for one inventory entry. Text is lowercased,
/// as the view cache stores it; item facts are the link's original's.
pub struct Subject<'a> {
    pub id: Uuid,
    pub folder: bool,
    pub name: &'a str,
    pub desc: &'a str,
    pub parent_name: &'a str,
    pub creator: Uuid,
    /// Inventory type, with snapshots (17) counted as textures (6).
    pub kind: i32,
    pub owner_mask: u32,
    pub worn: bool,
    pub linked: bool,
    pub coalesced: bool,
    pub date: i64,
}

enum Test {
    Text(Field, Regex),
    /// Creators whose name matched, resolved once per query.
    Creators(HashSet<Uuid>),
    Types(u32),
    Rights(u32),
    Worn,
    Link,
    Coalesced,
    Newer(i64),
    Older(i64),
}

enum Compiled {
    Test(Test, bool),
    Group { any: bool, not: bool, nodes: Vec<Compiled> },
}

/// A query ready to run over many entries: patterns built and creator
/// names resolved once, then shared by the parallel filter.
pub struct Matcher {
    root: Compiled,
    folders: bool,
}

impl Matcher {
    /// `names` are the known creator names; `recent` is the Recent cutoff
    /// (Date "since logout"); `time` the clock, rounded to the minute so
    /// relative dates do not change the query every frame.
    pub fn new(query: &SmartQuery, names: &HashMap<Uuid, String>, recent: i64, time: i64) -> Self {
        let lowered: Vec<(Uuid, String)> = if query.uses(Field::Creator) {
            names.iter().map(|(id, name)| (*id, name.to_lowercase())).collect()
        } else {
            Vec::new()
        };
        Self {
            root: compile_group(&query.root, &lowered, recent, time / 60 * 60),
            folders: query.root.only_folder_fields(),
        }
    }

    pub fn matches(&self, subject: &Subject) -> bool {
        // A folder is a result only when every rule could speak about it;
        // otherwise "not a texture" would list every folder.
        if subject.folder && !self.folders {
            return false;
        }
        eval(&self.root, subject)
    }
}

fn compile_group(group: &Group, names: &[(Uuid, String)], recent: i64, time: i64) -> Compiled {
    let nodes = group
        .nodes
        .iter()
        .filter_map(|node| match node {
            Node::Rule(rule) if rule.is_blank() => None,
            Node::Rule(rule) => compile_rule(rule, names, recent, time).map(|test| Compiled::Test(test, rule.not)),
            Node::Group(group) if group.is_blank() => None,
            Node::Group(group) => Some(compile_group(group, names, recent, time)),
        })
        .collect();
    Compiled::Group {
        any: group.any,
        not: group.not,
        nodes,
    }
}

fn compile_rule(rule: &Rule, names: &[(Uuid, String)], recent: i64, time: i64) -> Option<Test> {
    Some(match rule.field {
        Field::Type => Test::Types(rule.bits),
        Field::Permissions => Test::Rights(rule.bits),
        Field::Worn => Test::Worn,
        Field::Link => Test::Link,
        Field::Coalesced => Test::Coalesced,
        Field::Date => match rule.age {
            Age::Any => return None,
            Age::SinceLogout => Test::Newer(recent),
            Age::Newer => Test::Newer(time - i64::from(rule.hours.min(876_000)) * 3600),
            Age::Older => Test::Older(time - i64::from(rule.hours.min(876_000)) * 3600),
        },
        // a creator given by key (Find similar) needs no known name
        Field::Creator if rule.op == Op::Is && Uuid::parse_str(rule.text.trim()).is_ok() => {
            Test::Creators(Uuid::parse_str(rule.text.trim()).into_iter().collect())
        }
        field => {
            let pattern = match pattern(rule.op, &rule.text.trim().to_lowercase()) {
                Ok(pattern) => pattern,
                Err(err) => {
                    // Escaped values cannot fail; only the size limit can.
                    log::warn!("advanced inventory search: rule skipped: {err}");
                    return None;
                }
            };
            if field == Field::Creator {
                Test::Creators(names.iter().filter(|(_, name)| pattern.is_match(name)).map(|(id, _)| *id).collect())
            } else {
                Test::Text(field, pattern)
            }
        }
    })
}

/// The pattern of a text rule. The value is escaped: only the fragments
/// added here are pattern syntax.
pub fn pattern(op: Op, value: &str) -> Result<Regex, regex::Error> {
    let v = regex::escape(value);
    let source = match op {
        Op::Contains => v,
        Op::Is => format!(r"(?:^|\s){v}(?:\s|$)"),
        Op::StartsWith => format!("^{v}"),
        Op::EndsWith => format!("{v}$"),
        Op::Pattern => format!("^{}$", v.replace(r"\*", ".*").replace(r"\?", ".")),
        Op::EndsInNumber => format!(r"{v}.*\d\)?\s*$"),
        Op::HasVersion => format!(r"{v}.*(?:\bv\d+(?:\.\d+)*\b|\b\d+(?:\.\d+)+\b)"),
    };
    regex::RegexBuilder::new(&format!("(?s){source}")).size_limit(1 << 20).build()
}

fn eval(node: &Compiled, s: &Subject) -> bool {
    match node {
        Compiled::Test(test, not) => run(test, s) != *not,
        Compiled::Group { any, not, nodes } => {
            let hit = if nodes.is_empty() {
                true
            } else if *any {
                nodes.iter().any(|n| eval(n, s))
            } else {
                nodes.iter().all(|n| eval(n, s))
            };
            hit != *not
        }
    }
}

fn run(test: &Test, s: &Subject) -> bool {
    match test {
        Test::Text(field, pattern) => match field {
            Field::Name => pattern.is_match(s.name),
            Field::Description => !s.folder && pattern.is_match(s.desc),
            Field::Uuid => pattern.is_match(&s.id.to_string()),
            Field::Folder => pattern.is_match(s.parent_name),
            _ => false,
        },
        Test::Creators(ids) => !s.folder && ids.contains(&s.creator),
        Test::Types(bits) => !s.folder && (0..32).contains(&s.kind) && bits & (1_u32 << s.kind) != 0,
        Test::Rights(bits) => !s.folder && s.owner_mask & bits == *bits,
        Test::Worn => !s.folder && s.worn,
        Test::Link => !s.folder && s.linked,
        Test::Coalesced => !s.folder && s.coalesced,
        Test::Newer(cutoff) => s.date > 0 && s.date >= *cutoff,
        Test::Older(cutoff) => s.date > 0 && s.date <= *cutoff,
    }
}

/// A name without its trailing version or number and the punctuation
/// before it ("Velvet Jacket v2.1" → "velvet jacket"), for Find similar.
pub fn name_stem(name: &str) -> String {
    static TAIL: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"(?i)[\s\-_.#(\[]*(?:v?\d+(?:\.\d+)*[a-z]?)[)\]]?\s*$").expect("static pattern"));
    let lower = name.trim().to_lowercase();
    let stem = TAIL.replace(&lower, "");
    let stem = stem.trim();
    if stem.is_empty() { lower } else { stem.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(name: &str) -> Subject<'_> {
        Subject {
            id: Uuid::from_u128(7),
            folder: false,
            name,
            desc: "",
            parent_name: "",
            creator: Uuid::nil(),
            kind: 6,
            owner_mask: 0,
            worn: false,
            linked: false,
            coalesced: false,
            date: 1000,
        }
    }

    fn one(rule: Rule) -> SmartQuery {
        SmartQuery {
            root: Group {
                nodes: vec![Node::Rule(rule)],
                ..Group::default()
            },
        }
    }

    fn hit(rule: Rule, name: &str) -> bool {
        Matcher::new(&one(rule), &HashMap::new(), 0, 0).matches(&subject(name))
    }

    #[test]
    fn text_ops_match_as_named_and_values_are_never_patterns() {
        assert!(hit(Rule::text(Field::Name, Op::Contains, "Jack"), "velvet jacket"));
        assert!(hit(Rule::text(Field::Name, Op::Is, "jacket"), "velvet jacket v2"));
        assert!(!hit(Rule::text(Field::Name, Op::Is, "jack"), "velvet jacket"));
        assert!(hit(Rule::text(Field::Name, Op::StartsWith, "velvet"), "velvet jacket"));
        assert!(!hit(Rule::text(Field::Name, Op::StartsWith, "jacket"), "velvet jacket"));
        assert!(hit(Rule::text(Field::Name, Op::EndsWith, "jacket"), "velvet jacket"));
        assert!(hit(Rule::text(Field::Name, Op::Pattern, "v*t ja?ket"), "velvet jacket"));
        assert!(!hit(Rule::text(Field::Name, Op::Pattern, "velvet"), "velvet jacket"));
        // Pattern syntax in a value is literal text.
        assert!(hit(Rule::text(Field::Name, Op::Contains, "a.b*"), "x a.b* y"));
        assert!(!hit(Rule::text(Field::Name, Op::Contains, "a.b*"), "axbbb"));
        assert!(!hit(Rule::text(Field::Name, Op::Pattern, "a.c"), "abc"));
        assert!(hit(Rule::text(Field::Name, Op::Contains, "(1)"), "copy (1)"));
    }

    #[test]
    fn numbers_and_versions_are_recognised_after_an_optional_value() {
        let number = |text: &str, name: &str| hit(Rule::text(Field::Name, Op::EndsInNumber, text), name);
        assert!(number("", "box 12"));
        assert!(number("", "box (3)"));
        assert!(!number("", "box"));
        assert!(number("box", "box 4"));
        assert!(!number("hat", "box 4"));
        let version = |text: &str, name: &str| hit(Rule::text(Field::Name, Op::HasVersion, text), name);
        assert!(version("", "hud v2"));
        assert!(version("", "hud 1.2.3 (boxed)"));
        assert!(version("hud", "hud v1.4"));
        assert!(!version("", "hud 2"));
        assert!(!version("", "velvet"));
    }

    #[test]
    fn groups_join_rules_with_all_any_and_not() {
        let rule = |t: &str| Node::Rule(Rule::text(Field::Name, Op::Contains, t));
        let query = |any, not, nodes| SmartQuery {
            root: Group { any, not, nodes },
        };
        let run = |q: &SmartQuery, name: &str| Matcher::new(q, &HashMap::new(), 0, 0).matches(&subject(name));
        let both = query(false, false, vec![rule("hat"), rule("red")]);
        assert!(run(&both, "red hat"));
        assert!(!run(&both, "red shoe"));
        let either = query(true, false, vec![rule("hat"), rule("shoe")]);
        assert!(run(&either, "red shoe"));
        assert!(!run(&either, "red coat"));
        let neither = query(true, true, vec![rule("hat"), rule("shoe")]);
        assert!(run(&neither, "red coat"));
        let mut not_red = Rule::text(Field::Name, Op::Contains, "red");
        not_red.not = true;
        let nested = query(
            false,
            false,
            vec![
                Node::Rule(not_red),
                Node::Group(Group {
                    any: true,
                    not: false,
                    nodes: vec![rule("hat"), rule("shoe")],
                }),
            ],
        );
        assert!(run(&nested, "blue hat"));
        assert!(!run(&nested, "red hat"));
        assert!(!run(&nested, "blue coat"));
    }

    #[test]
    fn blank_rules_are_left_out_and_an_empty_query_matches_everything() {
        let blank = Rule::text(Field::Name, Op::Contains, "  ");
        assert!(blank.is_blank());
        assert!(one(blank.clone()).is_empty());
        let either = SmartQuery {
            root: Group {
                any: true,
                not: false,
                nodes: vec![Node::Rule(blank), Node::Rule(Rule::text(Field::Name, Op::Contains, "hat"))],
            },
        };
        // The blank rule must not make the OR match everything.
        let m = Matcher::new(&either, &HashMap::new(), 0, 0);
        assert!(m.matches(&subject("hat")));
        assert!(!m.matches(&subject("shoe")));
        assert!(Matcher::new(&SmartQuery::default(), &HashMap::new(), 0, 0).matches(&subject("anything")));
    }

    #[test]
    fn creator_given_by_key_needs_no_name() {
        let key = Uuid::from_u128(7);
        let m = Matcher::new(&one(Rule::text(Field::Creator, Op::Is, &key.to_string())), &HashMap::new(), 0, 0);
        let mut s = subject("hat");
        s.creator = key;
        assert!(m.matches(&s));
        s.creator = Uuid::from_u128(8);
        assert!(!m.matches(&s));
    }

    #[test]
    fn creators_resolve_by_name_once_and_item_facts_are_rules() {
        let alice = Uuid::from_u128(1);
        let names = HashMap::from([(alice, "Alice Doe".to_string()), (Uuid::from_u128(2), "Bob".to_string())]);
        let m = Matcher::new(&one(Rule::text(Field::Creator, Op::StartsWith, "alice")), &names, 0, 0);
        let mut s = subject("hat");
        s.creator = alice;
        assert!(m.matches(&s));
        s.creator = Uuid::from_u128(2);
        assert!(!m.matches(&s));

        let mut types = Rule::of(Field::Type);
        types.bits = 1 << 0;
        assert!(!hit(types.clone(), "hat"));
        types.bits |= 1 << 6;
        assert!(hit(types, "hat"));
        let mut rights = Rule::of(Field::Permissions);
        rights.bits = 0b11;
        let mut s = subject("hat");
        s.owner_mask = 0b111;
        assert!(Matcher::new(&one(rights), &HashMap::new(), 0, 0).matches(&s));
        let mut worn = Rule::of(Field::Worn);
        worn.not = true;
        assert!(hit(worn, "hat"));
        let mut old = Rule::of(Field::Date);
        old.age = Age::Older;
        old.hours = 1;
        assert!(Matcher::new(&one(old), &HashMap::new(), 0, 7200).matches(&subject("hat")));
    }

    #[test]
    fn folders_match_only_rules_that_can_speak_about_them() {
        let mut folder = subject("hats");
        folder.folder = true;
        assert!(Matcher::new(&one(Rule::text(Field::Name, Op::Contains, "hat")), &HashMap::new(), 0, 0).matches(&folder));
        let mut not_texture = Rule::of(Field::Type);
        not_texture.bits = 1 << 0;
        not_texture.not = true;
        assert!(!Matcher::new(&one(not_texture), &HashMap::new(), 0, 0).matches(&folder));
    }

    #[test]
    fn quick_search_converts_to_rules() {
        use super::super::view::SearchField;
        assert!(SmartQuery::from_quick(" ", SearchField::Name).is_empty());
        let q = SmartQuery::from_quick("red+hat", SearchField::Name);
        assert_eq!(q.root.nodes.len(), 2);
        let m = Matcher::new(&q, &HashMap::new(), 0, 0);
        assert!(m.matches(&subject("red top hat")));
        assert!(!m.matches(&subject("red shoe")));
        let exact = SmartQuery::from_quick("\"hat\"", SearchField::Name);
        assert_eq!(exact.root.nodes, [Node::Rule(Rule::text(Field::Name, Op::Is, "hat"))]);
        let all = SmartQuery::from_quick("hat", SearchField::All);
        let Node::Group(group) = &all.root.nodes[0] else {
            panic!("All is a group")
        };
        assert!(group.any && group.nodes.len() == 4);
    }

    #[test]
    fn name_stems_drop_trailing_versions_and_numbers() {
        assert_eq!(name_stem("Velvet Jacket v2.1"), "velvet jacket");
        assert_eq!(name_stem("Box (3)"), "box");
        assert_eq!(name_stem("HUD 1.2.3"), "hud");
        assert_eq!(name_stem("Hair_04"), "hair");
        assert_eq!(name_stem("Plain hat"), "plain hat");
        assert_eq!(name_stem("2024"), "2024");
    }
}
