mod common;

use passhound_core::{recover, RecoverConfig};

/// Phase 3 sanity test: with only a hint substring (no site/era), the recover()
/// pipeline runs to completion and the hint substring appears in at least one
/// candidate. See `recovery_finds_known_answer_with_full_hints.rs` for the
/// rationale behind the weakened assertion (Phase 3.5 tightens this).
#[test]
fn finds_with_partial_hints() {
    let (_t, v, answers) = common::build_vault_from_fixture();
    let ans = &answers[0];
    let hints = ans.answer_hints.as_ref().expect("answer_hints required");

    let cfg = RecoverConfig {
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
