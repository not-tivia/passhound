use passhound_core::repo::{accounts::{self, NewAccount}, passwords::{self, NewPassword, Confidence}, sites::{self, NewSite}};
use passhound_core::Vault;
use tempfile::TempDir;

#[test]
fn full_create_unlock_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("vault.db");

    // 1. Create vault.
    let v = Vault::create(&path, b"correct-horse-battery-staple").unwrap();
    let site = sites::create(&v, NewSite {
        name: "RuneScape".into(),
        url: Some("runescape.com".into()),
        category: Some("Gaming".into()),
        abbreviations: vec!["RS".into()],
        notes: None,
    }).unwrap();
    let account = accounts::create(&v, NewAccount {
        site_id: site.id,
        username: Some("chris".into()),
        alias: Some("main".into()),
        notes: None,
    }).unwrap();
    passwords::insert(&v, NewPassword {
        account_id: account.id,
        plaintext: "Fluffy!2014",
        source: "manual".into(),
        confidence: Confidence::Certain,
        notes: None,
        created_at: None,
    }).unwrap();
    drop(v); // close

    // 2. Reopen, fail with wrong password, then succeed with right one.
    let mut v = Vault::open(&path).unwrap();
    assert!(v.unlock(b"wrong").is_err());
    v.unlock(b"correct-horse-battery-staple").unwrap();

    // 3. Read it back.
    let pt = passwords::current_plaintext(&v, account.id).unwrap().unwrap();
    assert_eq!(pt.as_str(), "Fluffy!2014");
}
