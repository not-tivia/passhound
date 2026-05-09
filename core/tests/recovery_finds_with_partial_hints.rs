mod common;

use passhound_core::{recover, RecoverConfig};

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
    let target = ans.password.as_str();
    let pos = candidates.iter().position(|c| c.password.as_str() == target);
    assert!(pos.is_some(), "expected '{target}' in top-100 with partial hints; got {} candidates",
        candidates.len());
}
