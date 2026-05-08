use passhound_core::importer::{parse_paste, pipeline};
use passhound_core::repo::accounts;
use passhound_core::repo::passwords;
use passhound_core::repo::sites;
use passhound_core::Vault;
use tempfile::TempDir;

#[test]
fn paste_import_round_trip() {
    let tmp = TempDir::new().unwrap();
    let vault_path = tmp.path().join("v.db");
    let v = Vault::create(&vault_path, b"hunter2").unwrap();

    let input = "\
site: RuneScape
username: chris
password: Fluffy!2014

site: Amazon
username: chris@example.com
password: Bezos$Buy1
";
    let parse = parse_paste(input);
    assert_eq!(parse.entries.len(), 2);
    let preview = pipeline::preview(&v, parse.entries).unwrap();
    assert_eq!(preview.new, 2);

    pipeline::commit(&v, preview, "paste", None).unwrap();

    let all_sites = sites::list(&v).unwrap();
    assert_eq!(all_sites.len(), 2);
    let amazon = all_sites.iter().find(|s| s.name == "Amazon").unwrap();
    let accs = accounts::list_for_site(&v, amazon.id).unwrap();
    let pt = passwords::current_plaintext(&v, accs[0].id).unwrap().unwrap();
    assert_eq!(pt.as_str(), "Bezos$Buy1");
}
