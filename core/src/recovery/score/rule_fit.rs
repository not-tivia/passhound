//! Per-rule corpus-fit multipliers. Corpus-first: use observed applicability
//! when there's enough corpus; fall back to encoded human priors only when the
//! corpus is too thin. A tiny epsilon keeps an unused rule near-zero, not zero.

use crate::recovery::RuleId;
use std::collections::HashMap;

const MIN_CORPUS: usize = 20;
const EPSILON: f32 = 0.02;
const SATURATION: f32 = 4.0;

const PROFILED_RULES: [RuleId; 5] = [
    RuleId::SiteAffix,
    RuleId::SpecialSuffix,
    RuleId::NumberIncrement,
    RuleId::LeetSwap,
    RuleId::CaseVariations,
];

fn prior(rule: RuleId) -> f32 {
    match rule {
        RuleId::NumberIncrement => 0.70,
        RuleId::SpecialSuffix => 0.55,
        RuleId::CaseVariations => 0.45,
        RuleId::LeetSwap => 0.15,
        RuleId::SiteAffix => 0.05,
        _ => 1.0,
    }
}

pub fn compute(applicability: &HashMap<RuleId, f32>, corpus_size: usize) -> HashMap<RuleId, f32> {
    let mut out = HashMap::new();
    for rule in PROFILED_RULES {
        let fit = if corpus_size >= MIN_CORPUS {
            let app = applicability.get(&rule).copied().unwrap_or(0.0);
            EPSILON.max((app * SATURATION).min(1.0))
        } else {
            prior(rule)
        };
        out.insert(rule, fit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_site_affixes_zeroes_siteaffix_fit() {
        let mut app = HashMap::new();
        app.insert(RuleId::SiteAffix, 0.0);
        let fit = compute(&app, 100);
        assert!((fit[&RuleId::SiteAffix] - EPSILON).abs() < 1e-6);
    }

    #[test]
    fn frequent_increments_keep_full_fit() {
        let mut app = HashMap::new();
        app.insert(RuleId::NumberIncrement, 0.6); // 0.6 * 4 = 2.4 -> capped 1.0
        let fit = compute(&app, 100);
        assert!((fit[&RuleId::NumberIncrement] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn moderate_usage_scales_linearly() {
        let mut app = HashMap::new();
        app.insert(RuleId::SpecialSuffix, 0.10); // 0.10 * 4 = 0.4
        let fit = compute(&app, 100);
        assert!((fit[&RuleId::SpecialSuffix] - 0.4).abs() < 1e-6);
    }

    #[test]
    fn thin_corpus_falls_back_to_priors() {
        let app = HashMap::new(); // empty
        let fit = compute(&app, 5); // below MIN_CORPUS
        assert!((fit[&RuleId::SiteAffix] - 0.05).abs() < 1e-6);
        assert!((fit[&RuleId::NumberIncrement] - 0.70).abs() < 1e-6);
    }
}
