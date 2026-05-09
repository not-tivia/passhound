mod common;

use passhound_core::{recover, Error, RecoverConfig};

#[test]
fn returns_locked_on_locked_vault() {
    let (_t, v) = common::build_locked_vault_from_fixture();
    let cfg = RecoverConfig { limit: 100, ..Default::default() };
    let err = recover(&v, cfg).unwrap_err();
    assert!(matches!(err, Error::Locked), "expected Error::Locked, got {err:?}");
}
