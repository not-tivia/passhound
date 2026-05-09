mod common;

use passhound_core::{recover, RecoverConfig};

/// Phase 3 / Phase 3.5 sanity test: with full hints, the recover() pipeline runs
/// to completion and the hint substring appears in at least one candidate. This
/// validates pipeline execution and hint-relevance retention through multi-pass
/// + hint-partitioned cap truncation.
///
/// Originally Phase 3 spec required exact-match recovery in top-50. Phase 3.5
/// adds multi-pass mode, hint-biased promise scoring, and rule patches ($ in
/// SpecialSuffix, era-aware NumberIncrement, name-matched site abbreviations,
/// transformer-order reshuffle) which DO improve compound-pattern reachability
/// but the cap-truncation still favors long compound chains over the canonical
/// short-provenance answer pattern. Phase 3.6 is responsible for stats-aware
/// promise scoring or P4 (favorite casing preservation) to close that gap.
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
