//! Longest name/location matches at Unicode word boundaries. A shared trie
//! avoids testing every nearby name at every character of every chat line.
use std::collections::BTreeMap;

#[derive(Default)]
struct Node {
    edges: BTreeMap<char, usize>,
    terminal: Option<usize>,
}

pub(super) struct Matcher {
    nodes: Vec<Node>,
}

pub(super) fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Matcher {
    pub fn new(replacements: &[(String, String)]) -> Self {
        let mut nodes = vec![Node::default()];
        for (index, (needle, _)) in replacements.iter().enumerate() {
            let mut current = 0;
            for c in needle.chars() {
                let next = if let Some(next) = nodes[current].edges.get(&c) {
                    *next
                } else {
                    let next = nodes.len();
                    nodes.push(Node::default());
                    nodes[current].edges.insert(c, next);
                    next
                };
                current = next;
            }
            if nodes[current].terminal.is_none() {
                nodes[current].terminal = Some(index);
            }
        }
        Self { nodes }
    }

    pub fn at(&self, text: &str) -> Option<(usize, usize)> {
        let mut current = 0;
        let mut longest = None;
        'scan: for (byte, c) in text.char_indices() {
            // A lowercase scalar can expand (İ -> i + combining dot). Keep
            // original byte boundaries for slicing the unmodified source.
            for lower in c.to_lowercase() {
                let Some(next) = self.nodes[current].edges.get(&lower) else {
                    break 'scan;
                };
                current = *next;
            }
            let end = byte + c.len_utf8();
            if let Some(index) = self.nodes[current].terminal
                && !text[end..].chars().next().is_some_and(word)
            {
                longest = Some((end, index));
            }
        }
        longest
    }
}
