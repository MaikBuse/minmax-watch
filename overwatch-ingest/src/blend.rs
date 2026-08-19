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
//! Where the trusted sources contradict each other the entry is flagged **and
//! pulled toward even**, rather than quietly averaged, so a number nobody could
//! reconcile is visible in the UI and in the review diff instead of being
//! laundered into a plausible-looking mean. The verdict is reached for the
//! *unordered pair* and applied to both of its directions; see [`verdict`] for
//! why, and what that deliberately cannot see.

use std::collections::HashMap;

use overwatch_core::difficulty_to_value;
use overwatch_data::schema::MatchupEntry;

use crate::counterpickgg::RawMatchup;

const WEIGHT_CPGG: f32 = 0.75;
const WEIGHT_CWATCH: f32 = 0.25;

/// Spread between the two trusted sources, on the -100..=100 scale, above which
/// they are contradicting each other rather than merely differing in precision.
///
/// One constant for two jobs on purpose: where flagging starts is where
/// shrinking starts, so there is no band in which the app marks a row as
/// disputed while still reading it at face value.
const DISAGREEMENT_SPREAD: i32 = 60;

/// The spread at which a contradiction says nothing at all about the sign.
///
/// Not a tuning knob — both sources are bounded by ±100, so this is the widest
/// they can possibly be apart, and it is named only so the arithmetic below
/// reads as the interpolation it is.
const HOPELESS_SPREAD: i32 = 200;

/// Denominator of the shrink factor, doubled along with everything else.
const SHRINK_DEN: i32 = 2 * (HOPELESS_SPREAD - DISAGREEMENT_SPREAD);

/// How many movers the calibration block names.
const MOVERS: usize = 10;

/// Inclusive upper bounds of the `|value|` bands the calibration block counts.
const MAGNITUDE_BANDS: [i16; 5] = [19, 39, 59, 79, 100];

pub type SourceMap = HashMap<(String, String), i8>;

/// One row the verdict moved, for the calibration block.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Mover {
    pub hero: String,
    pub vs: String,
    /// The plain weighted average, before the pair's verdict reached it.
    pub before: i8,
    pub after: i8,
    /// Twice each source's mean reading, oriented to **this row**.
    ///
    /// Oriented to the row and not to the pair, because half of any mover list
    /// is the `b -> a` direction and printing the pair's numbers beside it
    /// inverts their sign — which reads as the blend being broken rather than
    /// as the report being sloppy.
    pub cpgg_doubled: i32,
    pub cwatch_doubled: i32,
    pub shrink_permille: u16,
}

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
    /// Rows the verdict pulled off their plain weighted average.
    pub moved: usize,
    pub movers: Vec<Mover>,
    /// `|value|` counts per [`MAGNITUDE_BANDS`], before and after the shrink.
    ///
    /// Measured inside this run against the unshrunk blend, never as a diff
    /// against the file on disk: `blend` does not read `matchups.toml`, and a
    /// number defined against it would lie whenever the scrape itself moved.
    pub magnitude_before: [usize; MAGNITUDE_BANDS.len()],
    pub magnitude_after: [usize; MAGNITUDE_BANDS.len()],
}

impl BlendReport {
    pub fn render(&self) -> String {
        let mut out = format!(
            "  matrix: {} directed pairs\n\
             \x20   counterpickgg {} | counterwatch {} | overpicker {} (recorded, not blended)\n\
             \x20   {} with rationale text, {} flagged as disagreements, {} unrated\n\
             \x20   {} row(s) pulled toward even | |value| {}: {} -> {}",
            self.pairs,
            self.from_cpgg,
            self.from_cwatch,
            self.from_opick,
            self.with_reason,
            self.disagreements,
            self.unrated,
            self.moved,
            band_labels(),
            counts(&self.magnitude_before),
            counts(&self.magnitude_after),
        );

        for mover in &self.movers {
            out.push_str(&format!(
                "\n\x20     {} vs {}: {:+} -> {:+} (counterpickgg {}, counterwatch {}, x{:.3})",
                mover.hero,
                mover.vs,
                mover.before,
                mover.after,
                halves(mover.cpgg_doubled),
                halves(mover.cwatch_doubled),
                f32::from(mover.shrink_permille) / 1000.0,
            ));
        }

        out
    }
}

/// `0-19/20-39/...`, derived from [`MAGNITUDE_BANDS`] so the label cannot drift
/// from the counts beside it.
fn band_labels() -> String {
    let mut low = 0;
    let mut parts = Vec::new();
    for high in MAGNITUDE_BANDS {
        parts.push(format!("{low}-{high}"));
        low = high + 1;
    }
    parts.join("/")
}

fn counts(bands: &[usize; MAGNITUDE_BANDS.len()]) -> String {
    bands
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

fn band(value: i8) -> usize {
    let magnitude = i16::from(value).abs();
    MAGNITUDE_BANDS
        .iter()
        .position(|high| magnitude <= *high)
        .unwrap_or(MAGNITUDE_BANDS.len() - 1)
}

/// Renders a doubled reading as the mean it stands for.
///
/// Keeps the half a pair mean can land on, because that half is load-bearing:
/// `genji`/`hazard` averages to -56.5 and so reads as agreement, where either
/// direction alone would have been flagged.
fn halves(doubled: i32) -> String {
    if doubled % 2 == 0 {
        format!("{:+}", doubled / 2)
    } else {
        format!("{:+.1}", f64::from(doubled) / 2.0)
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

/// Twice the mean of whichever readings a source has for the pair, oriented
/// `a -> b`.
///
/// Doubled so the entire verdict stays in integers: a mean over one reading is
/// `2v`, over two it is `x + y`, and both are exact. The alternative is an `f32`
/// factor, and six of the rows the shrink moves land on an exact `.5` product
/// where `f32` has well under a half-ULP of margin — they round correctly today
/// by luck rather than by construction, and would flip under `f64`, `mul_add`,
/// or a different pair of constants above.
fn doubled_mean(ab: Option<i8>, ba: Option<i8>) -> Option<i32> {
    match (ab, ba) {
        // The mirror is the same claim with its sign flipped.
        (Some(x), Some(y)) => Some(i32::from(x) - i32::from(y)),
        (Some(x), None) => Some(2 * i32::from(x)),
        (None, Some(y)) => Some(-2 * i32::from(y)),
        (None, None) => None,
    }
}

/// What a contradiction does to a pair, in both of its directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Verdict {
    /// Twice each source's mean, oriented `a -> b`; `None` where that source
    /// said nothing about the pair in either direction.
    cpgg: Option<i32>,
    cwatch: Option<i32>,
    /// Numerator of the factor both directions are multiplied by, over
    /// [`SHRINK_DEN`].
    numerator: i32,
}

impl Verdict {
    fn disagreement(self) -> bool {
        self.numerator < SHRINK_DEN
    }

    /// The factor in per mille, for the report only.
    fn permille(self) -> u16 {
        ((self.numerator * 2000 + SHRINK_DEN) / (2 * SHRINK_DEN)) as u16
    }
}

/// Reaches one verdict for an unordered pair, from all four readings of it.
///
/// Four properties, in the order they matter:
///
/// - **It reads the pair, not the row.** counterwatch rates only part of each
///   hero's list, so a contradiction routinely lands on one row of two — and
///   before this, the other row kept its full magnitude *and* its clean flag.
///   `winston vs zarya` read +100 against -31 and was flagged, while
///   `zarya vs winston` sat at -100 unflagged because that source happened to be
///   silent there, and the scorer averaged the two into a near-maximum edge.
/// - **The value is continuous, the flag is a step.** There is no cliff between
///   a spread of 59 and 61: at the threshold the factor is still exactly 1.
/// - **The endpoint is principled.** Two sources at opposite rails carry no
///   information about the sign, so the factor is 0 and the honest reading is
///   even. That is the claim `Matrix::rating` already makes by returning
///   `Option`, applied to contradiction instead of absence — and the row is
///   still *emitted*, because a rated dead even is not an unrated pair.
/// - **Antisymmetry is still not forced.** Only the flag and the factor
///   propagate across the mirror; the values stay whatever their own sources
///   said.
///
/// What it deliberately cannot see is a source contradicting *itself*: averaging
/// counterwatch's two directions folds that away, and 148 pairs have a reading
/// both ways with a median oriented disagreement of 12 and a worst case of 47
/// (`doomfist`/`mauga`, -72 against +25). Because counterpickgg is exactly
/// antisymmetric, the pair rule is therefore never harsher than a per-row one —
/// two pairs that used to be flagged no longer are. `counterwatch.rs` notes that
/// forcing symmetry on its *values* would hide a genuine disagreement, and that
/// still holds; this forces symmetry only on the verdict.
fn verdict(
    cpgg_ab: Option<i8>,
    cpgg_ba: Option<i8>,
    cwatch_ab: Option<i8>,
    cwatch_ba: Option<i8>,
) -> Verdict {
    let cpgg = doubled_mean(cpgg_ab, cpgg_ba);
    let cwatch = doubled_mean(cwatch_ab, cwatch_ba);

    let doubled_spread = match (cpgg, cwatch) {
        (Some(a), Some(b)) => (a - b).abs(),
        // Silence is not contradiction. A pair only one source rated keeps its
        // full magnitude, which is what stops absence being laundered into
        // uncertainty.
        _ => 0,
    };

    let numerator = if doubled_spread <= 2 * DISAGREEMENT_SPREAD {
        SHRINK_DEN
    } else {
        // Unreachable below zero while `HOPELESS_SPREAD` is the bound the two
        // sources actually obey; live the moment anyone lowers it.
        (2 * HOPELESS_SPREAD - doubled_spread).max(0)
    };

    Verdict {
        cpgg,
        cwatch,
        numerator,
    }
}

/// Pulls a blended value toward even by the pair's verdict.
///
/// Shrinks the value **after** it has been rounded to the canonical scale, not
/// the unrounded mean. That order is load-bearing rather than incidental:
/// shrinking the mean first moves 15 rows of the committed matrix by a point,
/// and this is the only place either rounding happens.
fn shrink(value: i8, verdict: Verdict) -> i8 {
    let scaled = i32::from(value) * verdict.numerator;
    // Half away from zero, matching `f32::round` and so every other scale
    // helper in the workspace.
    let rounded = (scaled.abs() * 2 + SHRINK_DEN) / (2 * SHRINK_DEN) * scaled.signum();
    rounded.clamp(-100, 100) as i8
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

    blend_values(hero_keys, &cpgg_values, &reasons, opick, cwatch)
}

/// The blend itself, over per-source values already on the canonical scale.
///
/// Split from [`blend`] so the same pass can run off the columns of a committed
/// `matchups.toml` with no network — every value in that file is reproducible
/// from the `cpgg` and `cwatch` columns beside it, which is what lets a change to
/// the blend be reviewed as a diff of exactly the rows the blend moved.
pub fn blend_values(
    hero_keys: &[String],
    cpgg: &SourceMap,
    reasons: &HashMap<(String, String), String>,
    opick: &SourceMap,
    cwatch: &SourceMap,
) -> (Vec<MatchupEntry>, BlendReport) {
    let mut report = BlendReport::default();
    let mut out = Vec::new();
    let mut movers: Vec<Mover> = Vec::new();

    for (index, hero) in hero_keys.iter().enumerate() {
        for vs in &hero_keys[index + 1..] {
            // Dead for a unique roster, but a duplicated key in heroes.toml
            // would otherwise emit a self-matchup, and the `i < j` walk is what
            // stopped the outer guard from catching it.
            if hero == vs {
                continue;
            }

            let ab = (hero.clone(), vs.clone());
            let ba = (vs.clone(), hero.clone());

            // One verdict for the pair, applied to both of its rows.
            // Orientation-safe by construction: the spread is an absolute
            // difference and the factor is applied identically both ways, so the
            // roster order this loop follows cannot reach a value. The sort at
            // the end is what puts the file back into its committed order.
            let verdict = verdict(
                cpgg.get(&ab).copied(),
                cpgg.get(&ba).copied(),
                cwatch.get(&ab).copied(),
                cwatch.get(&ba).copied(),
            );

            // `orientation` flips the pair's means onto the row being written.
            for (key, orientation) in [(&ab, 1), (&ba, -1)] {
                report.pairs += 1;

                let cpgg_value = cpgg.get(key).copied();
                let opick_value = opick.get(key).copied();
                let cwatch_value = cwatch.get(key).copied();

                // Only the trusted sources reach `weighted`; overpicker is
                // recorded on the entry but takes no part in the value or the
                // verdict.
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

                let Some(plain) = weighted_average(&weighted) else {
                    report.unrated += 1;
                    continue;
                };

                let value = shrink(plain, verdict);
                report.magnitude_before[band(plain)] += 1;
                report.magnitude_after[band(value)] += 1;

                if value != plain {
                    report.moved += 1;
                    // Both means are present whenever the factor is under one,
                    // because silence cannot shrink anything.
                    if let (Some(cpgg_mean), Some(cwatch_mean)) = (verdict.cpgg, verdict.cwatch) {
                        movers.push(Mover {
                            hero: key.0.clone(),
                            vs: key.1.clone(),
                            before: plain,
                            after: value,
                            cpgg_doubled: orientation * cpgg_mean,
                            cwatch_doubled: orientation * cwatch_mean,
                            shrink_permille: verdict.permille(),
                        });
                    }
                }

                let disagreement = verdict.disagreement();
                if disagreement {
                    report.disagreements += 1;
                }

                let reason = reasons.get(key).cloned().unwrap_or_default();
                if !reason.is_empty() {
                    report.with_reason += 1;
                }

                out.push(MatchupEntry {
                    hero: key.0.clone(),
                    vs: key.1.clone(),
                    value,
                    disagreement,
                    cpgg: cpgg_value,
                    opick: opick_value,
                    cwatch: cwatch_value,
                    reason,
                    // The blend has no opinion about the curated lane; it is
                    // `merge_matchups` that carries it over from the committed
                    // file, for the same reason `merge_synergy` does.
                    curated: None,
                    note: String::new(),
                });
            }
        }
    }

    // Named by size, then by key: the cut at `MOVERS` is clean today, but the
    // order inside it must not depend on the roster's.
    movers.sort_by(|a, b| {
        let delta = |m: &Mover| i16::from(m.before).abs_diff(i16::from(m.after));
        delta(b)
            .cmp(&delta(a))
            .then_with(|| (&a.hero, &a.vs).cmp(&(&b.hero, &b.vs)))
    });
    movers.truncate(MOVERS);
    report.movers = movers;

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

        // cpgg -100 at weight .75 against cwatch -50 at weight .25. The pair
        // spread is 50, below the threshold, so this is also the assertion that
        // ordinary differences in precision are left alone.
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
        let entry = find(&entries, "reinhardt", "pharah");

        assert!(entry.disagreement, "-100 versus +50 is a contradiction");
        assert_eq!(report.disagreements, 1);

        // The value assertion, not only the flag: without it this test passes
        // just as happily through a shrink that is a complete no-op. -100 and
        // +50 blend to -63 and the spread of 150 keeps 100/280 of it. The
        // product is exactly -22.5, which is the pin on rounding away from zero.
        assert_eq!(entry.value, -23);
        assert_eq!(report.moved, 1);
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

    /// The bug this whole pass exists for: counterwatch rates only part of each
    /// hero's list, so a contradiction usually lands on one row of two, and the
    /// other one used to keep both its full magnitude and a clean flag.
    #[test]
    fn a_contradiction_on_one_side_shrinks_the_mirror_too() {
        let cpgg = vec![
            raw("reinhardt", "pharah", Some(8), ""),
            raw("pharah", "reinhardt", Some(2), ""),
        ];
        // Said about one direction only, exactly as the real source does.
        let cwatch = HashMap::from([(pair("reinhardt", "pharah"), 20)]);

        let (entries, report) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);

        // Pair means -75 against +20: a spread of 95 keeps 210/280 of both rows.
        let forward = find(&entries, "reinhardt", "pharah");
        let mirror = find(&entries, "pharah", "reinhardt");
        assert_eq!(forward.value, -38, "-51 blended, then pulled toward even");
        assert_eq!(
            mirror.value, 56,
            "and +75 with no reading of its own moves by the same factor"
        );

        assert!(forward.disagreement);
        assert!(
            mirror.disagreement,
            "the sources disagree about the pair, so both of its rows say so"
        );
        assert_eq!(report.disagreements, 2);
        assert_eq!(report.moved, 2);

        // Coverage is still counted per direction. Reading it off the pair
        // instead would halve every figure the report prints.
        assert_eq!(report.from_cpgg, 2);
        assert_eq!(report.from_cwatch, 1);
        assert_eq!(report.unrated, 4);
    }

    /// Two sources at opposite rails carry no information about the sign, so the
    /// honest reading is even - and it is a *rated* even, which is not the same
    /// answer as an unrated pair.
    #[test]
    fn sources_at_opposite_rails_read_as_even() {
        let cpgg = vec![
            raw("reinhardt", "pharah", Some(1), ""),
            raw("pharah", "reinhardt", Some(9), ""),
        ];
        let cwatch = HashMap::from([(pair("reinhardt", "pharah"), -100)]);

        let (entries, report) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);

        assert_eq!(entries.len(), 2, "both rows are still emitted");
        assert_eq!(report.unrated, 4);
        assert_eq!(find(&entries, "reinhardt", "pharah").value, 0);
        assert_eq!(
            find(&entries, "pharah", "reinhardt").value,
            0,
            "and -100 scaled by nothing is 0, never a minus zero"
        );
        assert!(entries.iter().all(|e| e.disagreement));
    }

    /// The flag is a step and the value is not, which is the property that lets
    /// one constant do both jobs: nothing lurches when a scrape moves a reading
    /// by a point.
    #[test]
    fn the_shrink_is_continuous_at_the_flag_threshold() {
        let cpgg = vec![raw("reinhardt", "pharah", Some(8), "")];
        let blend_with = |cwatch_value: i8| {
            let cwatch = HashMap::from([(pair("reinhardt", "pharah"), cwatch_value)]);
            let (entries, _) = blend(&keys(), &cpgg, &SourceMap::new(), &cwatch);
            let entry = find(&entries, "reinhardt", "pharah");
            (entry.value, entry.disagreement)
        };

        // -75 against -15 is a spread of exactly 60: at the threshold, untouched.
        assert_eq!(blend_with(-15), (-60, false));
        // A single point further and the row is flagged, while its value has not
        // yet moved at all.
        assert_eq!(blend_with(-14), (-60, true));
        // Twice the threshold takes well over half of it.
        assert_eq!(blend_with(45), (-26, true));
    }

    /// Absence is not contradiction. A pair one source rated alone keeps its full
    /// magnitude, and a source contradicting *itself* is not what this measures.
    #[test]
    fn a_pair_only_one_source_rated_is_never_shrunk() {
        let cpgg = vec![
            raw("reinhardt", "pharah", Some(9), ""),
            raw("pharah", "reinhardt", Some(1), ""),
        ];
        let (entries, report) = blend(&keys(), &cpgg, &SourceMap::new(), &SourceMap::new());

        assert_eq!(find(&entries, "reinhardt", "pharah").value, -100);
        assert_eq!(find(&entries, "pharah", "reinhardt").value, 100);
        assert_eq!(report.disagreements, 0);
        assert_eq!(report.moved, 0);

        // counterwatch alone, claiming both heroes win the duel by a mile. The
        // pair mean folds that away to +5 and there is nothing to compare it
        // against, so both rows pass through untouched. Documented rather than
        // fixed: the rule measures disagreement *between* sources.
        let cwatch = HashMap::from([
            (pair("reinhardt", "brigitte"), 80),
            (pair("brigitte", "reinhardt"), 70),
        ]);
        let (entries, report) = blend(&keys(), &[], &SourceMap::new(), &cwatch);

        assert_eq!(find(&entries, "reinhardt", "brigitte").value, 80);
        assert_eq!(find(&entries, "brigitte", "reinhardt").value, 70);
        assert_eq!(report.disagreements, 0);
        assert_eq!(report.from_cwatch, 2);
    }
}
