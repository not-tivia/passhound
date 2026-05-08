use passhound_core::importer::{parse_paste, pipeline};
use passhound_core::repo::sites;
use passhound_core::Vault;
use tempfile::TempDir;

#[test]
fn undo_round_trip() {
    let tmp = TempDir::new().unwrap();
    let vault_path = tmp.path().join("v.db");
    let v = Vault::create(&vault_path, b"hunter2").unwrap();

    let parse = parse_paste("site: Foo\npassword: pw\n");
    let preview = pipeline::preview(&v, parse.entries).unwrap();
    let id = pipeline::commit(&v, preview, "paste", None).unwrap();

    assert_eq!(sites::list(&v).unwrap().len(), 1);
    let counts = pipeline::undo(&v, id).unwrap();
    assert_eq!(counts.passwords, 1);
    assert_eq!(counts.accounts, 1);
    assert_eq!(counts.sites, 1);
    assert!(sites::list(&v).unwrap().is_empty());
}
