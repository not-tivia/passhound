//! Shared loader for the synthetic JSON fixture. Used by the recovery_* integration tests.

#![allow(dead_code)]

use chrono::DateTime;
use passhound_core::repo::accounts::{self, NewAccount};
use passhound_core::repo::base_words;
use passhound_core::repo::eras;
use passhound_core::repo::passwords::{self, Confidence, NewPassword};
use passhound_core::repo::sites::{self, NewSite};
use passhound_core::Vault;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

const FIXTURE_PATH: &str = "test-fixtures/synthetic_history.json";

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub schema_version: u32,
    pub created_at: String,
    pub notes: String,
    pub eras: Vec<EraEntry>,
    pub favorite_words: Vec<String>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct EraEntry {
    pub name: String,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub site: String,
    pub category: Option<String>,
    pub url: Option<String>,
    pub abbreviations: Option<Vec<String>>,
    pub username: Option<String>,
    pub password: String,
    pub created_at: String,
    #[serde(default)]
    pub is_answer: bool,
    #[serde(default)]
    pub answer_hints: Option<AnswerHints>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnswerHints {
    pub site: Option<String>,
    pub era_name: Option<String>,
    pub hint: Option<String>,
}

pub fn load_fixture() -> Fixture {
    let raw = std::fs::read_to_string(FIXTURE_PATH).expect("fixture file present");
    serde_json::from_str(&raw).expect("fixture parses")
}

/// Build a fully-populated vault from the fixture.
/// - Inserts all entries EXCEPT those with `is_answer: true`.
/// - Adds eras, runs analyze, manually promotes `favorite_words` so analyze re-runs preserve them.
/// Returns: (TempDir, Vault, list of answer entries).
pub fn build_vault_from_fixture() -> (tempfile::TempDir, Vault, Vec<Entry>) {
    let fx = load_fixture();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let path = tmp.path().join("v.db");
    let v = Vault::create(&path, b"hunter2").expect("create vault");

    // Eras.
    for e in &fx.eras {
        let start = e.start.as_deref().map(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap());
        let end   = e.end.as_deref().map(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap());
        eras::add(&v, &e.name, start, end, None).unwrap();
    }

    // Entries: create the site row for every entry (including is_answer) so the
    // recovery pipeline's name-matched-abbreviation pre-query can find the
    // answer's site declaration. Skip inserting the PASSWORD row for is_answer
    // entries — that's what makes them "hidden" answers.
    let mut answers: Vec<Entry> = Vec::new();
    let mut sites_cache: HashMap<String, i64> = HashMap::new();
    let mut accounts_cache: HashMap<(i64, String), i64> = HashMap::new();
    for entry in fx.entries {
        // Always create the site row so its abbreviations and category are visible.
        let site_id = *sites_cache.entry(entry.site.clone()).or_insert_with(|| {
            let s = sites::create(&v, NewSite {
                name: entry.site.clone(),
                url: entry.url.clone(),
                category: entry.category.clone(),
                abbreviations: entry.abbreviations.clone().unwrap_or_default(),
                notes: None,
            }).unwrap();
            s.id
        });
        if entry.is_answer {
            answers.push(entry);
            continue;
        }
        let username_key = entry.username.clone().unwrap_or_default();
        let account_id = *accounts_cache.entry((site_id, username_key.clone())).or_insert_with(|| {
            let a = accounts::create(&v, NewAccount {
                site_id,
                username: entry.username.clone(),
                display_name: None,
                alias: None,
                notes: None,
            }).unwrap();
            a.id
        });
        let created_at = DateTime::parse_from_rfc3339(&entry.created_at).unwrap().with_timezone(&chrono::Utc);
        passwords::insert(&v, NewPassword {
            account_id,
            plaintext: &entry.password,
            source: "fixture".into(),
            confidence: Confidence::Certain,
            notes: None,
            created_at: Some(created_at),
        }).unwrap();
    }

    // Run analyze.
    passhound_core::extract_base_words_from_history(&v, fx.favorite_words.len()).unwrap();

    // Manually promote each declared favorite (preserves them across future analyze runs).
    let words = base_words::fetch_decrypted(&v).unwrap();
    for fav in &fx.favorite_words {
        if let Some(w) = words.iter().find(|w| w.word.as_str() == fav) {
            base_words::promote(&v, w.id).unwrap();
        }
    }

    (tmp, v, answers)
}

/// Build a vault but do NOT unlock it. Useful for the locked-vault test.
pub fn build_locked_vault_from_fixture() -> (tempfile::TempDir, Vault) {
    let (tmp, mut v, _) = build_vault_from_fixture();
    v.lock();
    (tmp, v)
}

pub fn _silence_unused(_p: &Path) {}
