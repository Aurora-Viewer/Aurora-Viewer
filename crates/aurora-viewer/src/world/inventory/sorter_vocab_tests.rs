use super::*;

#[test]
fn the_built_in_vocabulary_reads() {
    let v = Vocabulary::built_in();
    assert!(v.bodies.len() >= 10 && v.categories.len() >= 30);
    // every category is tried at most once: a second entry would never win
    let mut seen = std::collections::HashSet::new();
    assert!(v.categories.iter().all(|(c, _)| seen.insert(*c)));
}

#[test]
fn a_broken_vocabulary_is_refused_with_the_reason() {
    let good: serde_json::Value = serde_json::from_str(BUILT_IN).expect("json");
    let broken = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = good.clone();
        edit(&mut v);
        Vocabulary::parse(&v.to_string()).expect_err("refused")
    };
    let e = broken(&|v| v["categories"][0]["words"][0] = "Tattoo".into());
    assert!(e.contains("\"Tattoo\" is read as \"tattoo\""), "{e}");
    let e = broken(&|v| v["categories"][0]["category"] = "flying-cars".into());
    assert!(e.contains("unknown category"), "{e}");
    let e = broken(&|v| v["bodies"][0]["hides"] = serde_json::json!(["Nobody"]));
    assert!(e.contains("which is no body"), "{e}");
    let e = broken(&|v| v["surprise"] = 1.into());
    assert!(e.contains("surprise"), "{e}");
    assert!(Vocabulary::parse("not json").is_err());
}

/// The golden corpus: every name lands where corpus.json says. On a
/// difference, the message lists each name with what was expected and what
/// it got: update the vocabulary or, when the new result is better, the
/// corpus.
#[test]
fn vocabulary_corpus() {
    let rows = corpus();
    assert!(rows.len() >= 50, "the corpus reads");
    let diff = corpus_diff(&rows);
    assert!(
        diff.is_empty(),
        "{} of {} corpus names sort differently:\n{diff}",
        rows.iter().filter(|r| !r.matches()).count(),
        rows.len()
    );
}
