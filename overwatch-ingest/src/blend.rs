//! Combines the sources into the committed matchup matrix.
//!
//! Two of the three scraped sources are trusted, and this is a weighted average
//! over whichever of them has an opinion about a given pair, with the weights
//! renormalised over what is actually present:
//!
//! - **counterpickgg** dominates. It is the only complete, fine-grained source
//!   (nine levels), and it carries the rationale text. It is also perfectly
//!   self-antisymmetric — its two ratings for a pair always sum to 10, so
//!   `value(a,b) == -value(b,a)` for every pair once the conversion is anchored
//!   on the right midpoint — meaning each hero page is internally consistent with
//!   every other. The flip side is that its two directions are *not* independent
//!   readings: they are the same number stated twice.
//! - **counterwatch** is duel-derived rather than opinion-derived, and it
//!   independently agrees with counterpickgg (Pearson r = +0.51 forward,
//!   -0.51 transposed, over the pairs both cover). It only ranks a fraction of
//!   each row, so it refines rather than drives.
//!
//! **overpicker is deliberately excluded from the average.** Its published
//! matrix has no measurable relationship to either other source — r = -0.04
//! against counterpickgg and -0.07 against counterwatch, in both orientations,
//! and only -0.45 self-antisymmetric with 42% of cells at zero. Two sources
//! that independently agree with each other and disagree with a third is
//! evidence about the third. Its values are still recorded in each entry so the
//! judgement stays visible and reversible, but blending them in would inject
//! noise into an otherwise coherent matrix.
//!
//! Where the trusted sources disagree sharply the entry is flagged rather than
//! quietly averaged, so a bad number is visible in the UI and in the review
//! diff instead of being laundered into a plausible-looking mean.

use std::collections::HashMap;

use overwatch_core::difficulty_to_value;
use overwatch_data::schema::MatchupEntry;

use crate::counterpickgg::RawMatchup;

const WEIGHT_CPGG: f32 = 0.75;
const WEIGHT_CWATCH: f32 = 0.25;

/// Spread between the highest and lowest trusted source, on the -100..=100
/// scale, above which they are contradicting each other rather than merely
/// differing in precision.
const DISAGREEMENT_SPREAD: i16 = 60;

pub type SourceMap = HashMap<(String, String), i8>;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlendReport {
    pub pairs: usize,
    pub from_cpgg: usize,
    pub from_opick: usize,
    pub from_cwatch: usize,
    pub disagreements: usize,
    /// Pairs no source rated at all; these stay neutral in the matrix.
    pub unrated: usize,
    pub with_reason: usize,
}

impl BlendReport {
    pub fn render(&self) -> String {
        format!(
            "  matrix: {} directed pairs\n\
             \x20   counterpickgg {} | counterwatch {} | overpicker {} (recorded, not blended)\n\
             \x20   {} with rationale text, {} flagged as disagreements, {} unrated",
            self.pairs,
            self.from_cpgg,
            self.from_cwatch,
            self.from_opick,
            self.with_reason,
            self.disagreements,
            self.unrated
        )
    }
}

fn weighted_average(values: &[(f32, i8)]) -> Option<i8> {
    let total: f32 = values.iter().map(|(w, _)| w).sum();
    if total <= 0.0 {
        return None;
    }
    let sum: f32 = values.iter().map(|(w, v)| w * f32::from(*v)).sum();
    Some((sum / total).round().clamp(-100.0, 100.0) as i8)
}

fn spread(values: &[(f32, i8)]) -> i16 {
    let vals: Vec<i16> = values.iter().map(|(_, v)| i16::from(*v)).collect();
    match (vals.iter().min(), vals.iter().max()) {
        (Some(min), Some(max)) => max - min,
        _ => 0,
    }
}

/// Produces one entry per ordered pair that any source rated.
///
/// Output is sorted by `(hero, vs)` so the committed file has a stable order
/// and the review diff shows only genuine data changes.
pub fn blend(
    hero_keys: &[String],
    cpgg: &[RawMatchup],
    opick: &SourceMap,
    cwatch: &SourceMap,
) -> (Vec<MatchupEntry>, BlendReport) {
    let mut cpgg_values: SourceMap = HashMap::new();
    let mut reasons: HashMap<(String, String), String> = HashMap::new();

    for raw in cpgg {
        let key = (raw.hero.clone(), raw.vs.clone());
        if let Some(difficulty) = raw.difficulty {
            cpgg_values.insert(key.clone(), difficulty_to_value(f32::from(difficulty)));
        }
        if !raw.reason.is_empty() {
            reasons.insert(key, raw.reason.clone());
        }
    }

    let mut report = BlendReport::default();
    let mut out = Vec::new();

    for hero in hero_keys {
        for vs in hero_keys {
            if hero == vs {
                continue;
            }
            report.pairs += 1;
            let key = (hero.clone(), vs.clone());

            let cpgg_value = cpgg_values.get(&key).copied();
            let opick_value = opick.get(&key).copied();
            let cwatch_value = cwatch.get(&key).copied();

            // Only the trusted sources reach `weighted`; overpicker is recorded
            // on the entry but takes no part in the value or the spread.
            let mut weighted = Vec::new();
            if let Some(v) = cpgg_value {
                weighted.push((WEIGHT_CPGG, v));
                report.from_cpgg += 1;
            }
            if let Some(v) = cwatch_value {
                weighted.push((WEIGHT_CWATCH, v));
                report.from_cwatch += 1;
            }
            if opick_value.is_some() {
                report.from_opick += 1;
            }

            let Some(value) = weighted_average(&weighted) else {
                report.unrated += 1;
                continue;
            };

            let disagreement = weighted.len() > 1 && spread(&weighted) > DISAGREEMENT_SPREAD;
            if disagreement {
                report.disagreements += 1;
            }

            let reason = reasons.get(&key).cloned().unwrap_or_default();
            if !reason.is_empty() {
                report.with_reason += 1;
            }

            out.push(MatchupEntry {
                hero: hero.clone(),
                vs: vs.clone(),
                value,
                disagreement,
                cpgg: cpgg_value,
                opick: opick_value,
                cwatch: cwatch_value,
                reason,
            });
        }
    }

    out.sort_by(|a, b| (&a.hero, &a.vs).cmp(&(&b.hero, &b.vs)));
    (out, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<String> {
        ["reinhardt", "pharah", "brigitte"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn raw(hero: &str, vs: &str, difficulty: Option<u8>, reason: &str) -> RawMatchup {
        RawMatchup {
            hero: hero.to_owned(),
            vs: vs.to_owned(),
            difficulty,
            reason: reason.to_owned(),
        }
    }

    fn pair(hero: &str, vs: &str) -> (String, String) {
        (hero.to_owned(), vs.to_owned())
    }

    fn find<'a>(entries: &'a [MatchupEntry], hero: &str, vs: &str) -> &'a MatchupEntry {
        entries
            .iter()
            .find(|e| e.hero == hero && e.vs == vs)
            .expect("pair present")
    }

    #[test]
    fn the_primary_source_dominates_the_blend() {
        let cpgg = vec![raw("reinhardt", "pharah", Some(9), "weak to air")];
        let cwatch = HashMap::from([(pair("reinhardt", "pharah"), -50)]);

        let (entries, _) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);
        let entry = find(&entries, "reinhardt", "pharah");

        // cpgg -100 at weight .75 against cwatch -50 at weight .25.
        assert_eq!(entry.value, -88);
        assert_eq!(entry.cpgg, Some(-100));
        assert_eq!(entry.cwatch, Some(-50));
        assert_eq!(entry.reason, "weak to air");
    }

    /// The judgement that overpicker is not usable data lives in the blend, so
    /// it is worth a test: its values must be recorded and ignored.
    #[test]
    fn overpicker_is_recorded_but_never_blended() {
        let cpgg = vec![raw("reinhardt", "pharah", Some(9), "")];
        let opick = HashMap::from([(pair("reinhardt", "pharah"), 100)]);

        let (entries, report) = blend(&keys(), &cpgg, &opick, &SourceMap::new());
        let entry = find(&entries, "reinhardt", "pharah");

        assert_eq!(entry.opick, Some(100), "recorded for transparency");
        assert_eq!(entry.value, -100, "but it did not move the blend");
        assert!(!entry.disagreement, "and it cannot raise a disagreement");
        assert_eq!(report.from_opick, 1, "still counted in coverage");
    }

    #[test]
    fn sharp_contradictions_between_trusted_sources_are_flagged() {
        let cpgg = vec![raw("reinhardt", "pharah", Some(9), "")];
        let cwatch = HashMap::from([(pair("reinhardt", "pharah"), 50)]);

        let (entries, report) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);

        assert!(
            find(&entries, "reinhardt", "pharah").disagreement,
            "-100 versus +50 is a contradiction"
        );
        assert_eq!(report.disagreements, 1);
    }

    #[test]
    fn sources_that_merely_differ_in_precision_are_not_flagged() {
        let cpgg = vec![raw("reinhardt", "brigitte", Some(3), "")];
        let cwatch = HashMap::from([(pair("reinhardt", "brigitte"), 50)]);

        let (entries, report) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);

        assert!(
            !find(&entries, "reinhardt", "brigitte").disagreement,
            "+50 and +50 agree"
        );
        assert_eq!(report.disagreements, 0);
    }

    #[test]
    fn a_trusted_secondary_covers_a_gap_in_the_primary() {
        // counterpickgg has a card but no rating, as with the newest heroes.
        let cpgg = vec![raw("reinhardt", "pharah", None, "")];
        let cwatch = HashMap::from([(pair("reinhardt", "pharah"), 40)]);

        let (entries, report) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);
        let entry = find(&entries, "reinhardt", "pharah");

        assert_eq!(entry.value, 40, "falls back to the only source available");
        assert_eq!(entry.cpgg, None);
        assert_eq!(report.from_cpgg, 0);
    }

    #[test]
    fn an_untrusted_source_alone_leaves_the_pair_unrated() {
        let cpgg = vec![raw("reinhardt", "pharah", None, "")];
        let opick = HashMap::from([(pair("reinhardt", "pharah"), 40)]);

        let (entries, report) = blend(&keys(), &cpgg, &opick, &SourceMap::new());

        assert!(
            entries
                .iter()
                .all(|e| !(e.hero == "reinhardt" && e.vs == "pharah")),
            "overpicker alone is not enough to rate a pair"
        );
        assert_eq!(report.unrated, 6);
    }

    #[test]
    fn rationale_survives_even_when_the_rating_does_not() {
        let cpgg = vec![raw("reinhardt", "pharah", None, "weak to air")];
        let cwatch = HashMap::from([(pair("reinhardt", "pharah"), 40)]);

        let (entries, _) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);
        assert_eq!(find(&entries, "reinhardt", "pharah").reason, "weak to air");
    }

    #[test]
    fn pairs_no_source_rated_are_left_out_and_counted() {
        let (entries, report) = blend(&keys(), &[], &SourceMap::new(), &SourceMap::new());

        assert!(entries.is_empty());
        assert_eq!(report.pairs, 6, "3 heroes give 6 directed pairs");
        assert_eq!(report.unrated, 6);
    }

    #[test]
    fn self_matchups_are_never_emitted() {
        let cpgg = vec![raw("reinhardt", "reinhardt", Some(5), "")];
        let (entries, _) = blend(&keys(), &cpgg, &SourceMap::new(), &SourceMap::new());
        assert!(entries.iter().all(|e| e.hero != e.vs));
    }

    #[test]
    fn output_order_is_stable() {
        let cpgg = vec![
            raw("pharah", "reinhardt", Some(2), ""),
            raw("reinhardt", "pharah", Some(9), ""),
            raw("brigitte", "pharah", Some(6), ""),
        ];
        let (entries, _) = blend(&keys(), &cpgg, &SourceMap::new(), &SourceMap::new());

        let order: Vec<_> = entries
            .iter()
            .map(|e| (e.hero.as_str(), e.vs.as_str()))
            .collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted);
    }
}
