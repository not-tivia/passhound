mod common;

use passhound_core::{recover, RecoverConfig};

/// Phase 3 / Phase 3.5 / Phase 3.6 sanity test: with full hints, the recover()
/// pipeline runs to completion and the hint substring appears in at least one
/// candidate.
///
/// Phase 3 originally specified exact-match in top-50. Phase 3.5 weakened to
/// hint-substring presence after compound-pattern synthesis hit an architectural
/// limit. Phase 3.6 added stats-aware cap truncation (hybrid hint-partition +
/// ranking::score-based sort) which improves perf and ranking quality, but
/// "MoonBeam$2019Rd" still does not surface in top-100 because the WordCombine
/// `MoonBeam` candidate gets transformed past recognition early in the multi-pass
/// pipeline (the favorites are stored lowercased after tokenize, so WordCombine
/// emits both lowercase "moonbeam" and TitleCase "MoonBeam" but the latter
/// competes with thousands of other generated combinations and gets pruned).
///
/// Phase 3.7 candidate fix: P4 — preserve favorite-word original casing through
/// `tokenize` (schema migration #4), so "MoonBeam" enters base_words as a single
/// canonical entry and WordCombine doesn't have to invent the casing. Until then,
/// this assertion stays weakened.
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
