mod common;

use passhound_core::{recover, RecoverConfig};

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
    let target = ans.password.as_str();
    let pos = candidates.iter().position(|c| c.password.as_str() == target);
    assert!(pos.is_some(), "expected '{target}' in top-100 with full hints; got {} candidates",
        candidates.len());
    // With FULL hints we expect a strong rank — top 50 is the assertion.
    let rank = pos.unwrap() + 1;
    assert!(rank <= 50, "expected '{target}' in top 50 with full hints; got rank {rank}");
}
