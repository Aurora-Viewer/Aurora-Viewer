//! Print the English messages that need attention in one locale for a release.

use fluent_syntax::{ast, parser, serializer};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

fn read(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path).map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))
}

fn messages<'a>(source: &'a str, path: &Path) -> io::Result<BTreeMap<&'a str, ast::Message<&'a str>>> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let resource = parser::parse(source)
        .map_err(|(_, errors)| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {errors:?}", path.display())))?;
    let mut messages = BTreeMap::new();
    for entry in resource.body {
        if let ast::Entry::Message(message) = entry {
            let key = message.id.name;
            if messages.insert(key, message).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: duplicate message {key}", path.display()),
                ));
            }
        }
    }
    Ok(messages)
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [previous, current, target] = args.as_slice() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: catalog_delta PREVIOUS-EN.ftl CURRENT-EN.ftl TARGET.ftl",
        ));
    };
    let (previous, current, target) = (Path::new(previous), Path::new(current), Path::new(target));
    let previous_source = read(previous)?;
    let current_source = read(current)?;
    let target_source = read(target)?;
    let previous = messages(&previous_source, previous)?;
    let current = messages(&current_source, current)?;
    let target = messages(&target_source, target)?;

    let mut missing = 0;
    let mut changed = 0;
    println!("### Translation worklist with English source text");
    for (key, message) in &current {
        // A locale's native name is intentional and cannot be derived from English.
        if *key == "language-name" {
            continue;
        }
        let reason = if !target.contains_key(key) {
            missing += 1;
            "Missing translation"
        } else if previous
            .get(key)
            .is_none_or(|before| before.value != message.value || before.attributes != message.attributes)
        {
            changed += 1;
            "English changed"
        } else {
            continue;
        };
        println!("\n# {reason}");
        print!(
            "{}",
            serializer::serialize(&ast::Resource {
                body: vec![ast::Entry::Message(message.clone())]
            })
        );
    }
    for key in target.keys().filter(|key| !current.contains_key(*key)) {
        println!("\n# Removed from English: {key}");
    }
    eprintln!("{missing} missing, {changed} translated entries with changed English");
    Ok(())
}
