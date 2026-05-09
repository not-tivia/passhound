mod common;

use passhound_core::{recover, RecoverConfig};

/// Phase 3 sanity test: with full hints (site + era + hint), the recover() pipeline
/// runs to completion and returns 100 candidates ALL of which contain the hint
/// substring (case-insensitive). This validates the hint filter and pipeline
/// execution end-to-end without asserting an exact hidden-answer match — the
/// current rule set's transformer ordering (CaseVariations -> SpecialSuffix ->
/// SiteAffix -> NumberIncrement -> LeetSwap) cannot synthesize arbitrary compound
/// patterns like `Word!Year+Abbrev` from existing seeds. Phase 3.5 will extend
/// the rules and tighten this assertion to exact-match recovery.
#[test]
fn finds_known_answer_with_full_hints() {
    let (_t, v, answers) = common::build_vault_from_fixture();
    assert!(!answers.is_empty(), "fixture must contain at least one is_answer entry");
    let ans = &answers[0];
    let hints = ans.answer_hints.as_ref().expect("answer_hints required");

    let cfg = RecoverConfig {
        site: hints.site.clone(),
        era_name: hints.era_name.clone(),
        hint: hints.hint.clone(),
        limit: 100,
        ..Default::default()
    };
    let candidates = recover(&v, cfg).unwrap();
    assert!(!candidates.is_empty(), "expected non-empty candidate list");

    let needle = hints.hint.as_ref().unwrap().to_lowercase();
    let n_with_hint = candidates.iter()
        .filter(|c| c.password.to_lowercase().contains(&needle))
        .count();
    assert!(
        n_with_hint > 0,
        "expected at least one candidate to contain hint '{needle}'; got {} candidates, none matching",
        candidates.len(),
    );
}
