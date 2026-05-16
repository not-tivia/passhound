//! Recovery feedback persistence + auto-tune multiplier computation.
//!
//! Phase 4.12 ships persistence (record / clear) + a simple per-rule
//! multiplier (compute_multipliers). Multipliers are applied in
//! `ranking::score` as a bounded average across the candidate's
//! provenance.

use crate::error::Result;
use crate::recovery::RuleId;
use crate::vault::Vault;
use chrono::Utc;
use rusqlite::params;
use std::collections::HashMap;

const MIN_SAMPLES: usize = 3;
const ALPHA: f32 = 0.4;
const MULTIPLIER_LOWER: f32 = 0.8;
const MULTIPLIER_UPPER: f32 = 1.2;

pub struct FeedbackEvent {
    pub account_id: Option<i64>,
    pub provenance: Vec<RuleId>,
    pub score: f32,
    pub rank: i64,
    pub worked: bool,
    pub length: i64,
    pub has_digit: bool,
    pub has_symbol: bool,
    pub has_upper: bool,
    pub has_lower: bool,
}

/// Insert a feedback row.
pub fn record(vault: &Vault, event: FeedbackEvent) -> Result<()> {
    let provenance_str = encode_provenance(&event.provenance);
    let now = Utc::now().to_rfc3339();
    vault.conn().execute(
        "INSERT INTO recovery_feedback
           (account_id, provenance, score, rank, worked,
            length, has_digit, has_symbol, has_upper, has_lower, feedback_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            event.account_id,
            provenance_str,
            event.score as f64,
            event.rank,
            event.worked as i64,
            event.length,
            event.has_digit as i64,
            event.has_symbol as i64,
            event.has_upper as i64,
            event.has_lower as i64,
            now,
        ],
    )?;
    Ok(())
}

/// Delete every feedback row. Returns the number of rows deleted.
pub fn clear(vault: &Vault) -> Result<usize> {
    vault.conn().execute("DELETE FROM recovery_feedback", [])?;
    Ok(vault.conn().changes() as usize)
}

/// Compute a per-rule multiplier from accumulated feedback. Rules with fewer
/// than MIN_SAMPLES events are omitted from the result (callers treat missing
/// keys as 1.0 = no adjustment).
pub fn compute_multipliers(vault: &Vault) -> Result<HashMap<RuleId, f32>> {
    let mut stmt = vault.conn().prepare(
        "SELECT provenance, worked FROM recovery_feedback",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    // Accumulate (worked_count, total_count) per RuleId.
    let mut totals: HashMap<RuleId, (usize, usize)> = HashMap::new();
    for (provenance_str, worked) in rows {
        for tag in provenance_str.split(',') {
            let tag = tag.trim();
            if tag.is_empty() {
                continue;
            }
            if let Some(rule) = RuleId::from_tag(tag) {
                let entry = totals.entry(rule).or_insert((0, 0));
                if worked {
                    entry.0 += 1;
                }
                entry.1 += 1;
            }
            // Unknown tags silently dropped (defensive; shouldn't happen).
        }
    }

    let mut out: HashMap<RuleId, f32> = HashMap::new();
    for (rule, (worked_count, total)) in totals {
        if total < MIN_SAMPLES {
            continue;
        }
        let rate = worked_count as f32 / total as f32;
        let raw = 1.0 + ALPHA * (rate - 0.5);
        let clamped = raw.clamp(MULTIPLIER_LOWER, MULTIPLIER_UPPER);
        out.insert(rule, clamped);
    }
    Ok(out)
}

fn encode_provenance(rules: &[RuleId]) -> String {
    rules
        .iter()
        .map(|r| r.tag())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> (TempDir, Vault) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("v.db");
        let v = Vault::create(&path, b"hunter2").unwrap();
        (tmp, v)
    }

    fn event_with_worked(rules: Vec<RuleId>, worked: bool) -> FeedbackEvent {
        FeedbackEvent {
            account_id: None,
            provenance: rules,
            score: 0.5,
            rank: 1,
            worked,
            length: 12,
            has_digit: true,
            has_symbol: true,
            has_upper: true,
            has_lower: true,
        }
    }

    #[test]
    fn compute_multipliers_returns_empty_on_fresh_vault() {
        let (_t, v) = vault();
        let m = compute_multipliers(&v).unwrap();
        assert!(m.is_empty(), "fresh vault should yield no multipliers, got {:?}", m);
    }

    #[test]
    fn compute_multipliers_boosts_high_success_rule() {
        let (_t, v) = vault();
        for _ in 0..5 {
            record(&v, event_with_worked(vec![RuleId::BaseWordPool], true)).unwrap();
        }
        let m = compute_multipliers(&v).unwrap();
        let val = m.get(&RuleId::BaseWordPool).copied().expect("BaseWordPool should be present");
        assert!(val > 1.0, "5 worked + 0 didn't-work should boost above 1.0, got {}", val);
        assert!(val <= MULTIPLIER_UPPER, "should not exceed upper clamp, got {}", val);
    }

    #[test]
    fn compute_multipliers_penalizes_low_success_rule() {
        let (_t, v) = vault();
        for _ in 0..5 {
            record(&v, event_with_worked(vec![RuleId::LeetSwap], false)).unwrap();
        }
        let m = compute_multipliers(&v).unwrap();
        let val = m.get(&RuleId::LeetSwap).copied().expect("LeetSwap should be present");
        assert!(val < 1.0, "0 worked + 5 didn't-work should penalize below 1.0, got {}", val);
        assert!(val >= MULTIPLIER_LOWER, "should not go below lower clamp, got {}", val);
    }
}
