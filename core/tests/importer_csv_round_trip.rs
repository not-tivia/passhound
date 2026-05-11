use passhound_core::importer::{csv as csv_imp, pipeline};
use passhound_core::repo::accounts;
use passhound_core::repo::passwords;
use passhound_core::repo::sites;
use passhound_core::Vault;
use std::io::Write;
use tempfile::{NamedTempFile, TempDir};

#[test]
fn csv_import_round_trip() {
    let tmp = TempDir::new().unwrap();
    let vault_path = tmp.path().join("v.db");
    let v = Vault::create(&vault_path, b"hunter2").unwrap();

    let mut csv_file = NamedTempFile::new().unwrap();
    csv_file
        .write_all(
            b"name,url,username,password,note\n\
RuneScape,runescape.com,chris,Fluffy!2014,a note\n\
Amazon,amazon.com,chris@example.com,Bezos$Buy1,\n",
        )
        .unwrap();

    let parse = csv_imp::parse_file(&v, csv_file.path(), None, None).unwrap();
    assert_eq!(parse.entries.len(), 2);
    let preview = pipeline::preview(&v, parse.entries).unwrap();
    assert_eq!(preview.new, 2);

    pipeline::commit(&v, preview, "csv", Some(csv_file.path())).unwrap();

    let all_sites = sites::list(&v).unwrap();
    assert_eq!(all_sites.len(), 2);
    let runescape = all_sites.iter().find(|s| s.name == "RuneScape").unwrap();
    let accs = accounts::list_for_site(&v, runescape.id).unwrap();
    assert_eq!(accs.len(), 1);
    let pt = passwords::current_plaintext(&v, accs[0].id).unwrap().unwrap();
    assert_eq!(pt.as_str(), "Fluffy!2014");
}
