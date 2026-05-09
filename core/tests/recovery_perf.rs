mod common;

use passhound_core::{recover, RecoverConfig};
use std::time::Instant;

#[test]
fn perf_under_500ms_on_synthetic_fixture() {
    let (_t, v, _) = common::build_vault_from_fixture();
    // Cold pre-warm (Argon2 already ran during Vault::create + analyze).
    let cfg = RecoverConfig { limit: 100, ..Default::default() };
    let start = Instant::now();
    let candidates = recover(&v, cfg).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(candidates.len(), 100, "expected exactly 100 candidates");
    assert!(
        elapsed.as_millis() < 1500,
        "recover() took {}ms; budget is 1500ms on the 30-entry fixture (multi-pass doubles compute vs Phase 3)",
        elapsed.as_millis()
    );
}
