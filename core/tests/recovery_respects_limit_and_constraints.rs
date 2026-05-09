mod common;

use passhound_core::{recover, RecoverConfig};

#[test]
fn respects_limit_and_constraints() {
    let (_t, v, _answers) = common::build_vault_from_fixture();
    let cfg = RecoverConfig {
        limit: 30,
        min_length: Some(12),
        require_symbol: true,
        ..Default::default()
    };
    let candidates = recover(&v, cfg).unwrap();
    assert!(candidates.len() <= 30, "limit not honored");
    for c in &candidates {
        assert!(c.password.len() >= 12, "min_length not honored: {}", c.password.as_str());
        assert!(c.password.chars().any(|ch| !ch.is_alphanumeric()), "require_symbol not honored: {}", c.password.as_str());
    }
}
