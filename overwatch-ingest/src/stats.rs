//! Turns the counterpickgg index table into `strength.toml` and
//! `map_affinity.toml`, and the two rank-sliced sources into
//! `strength_by_rank.toml`.
//!
//! The first two signals come from one fetch. They are weaker than the matchup
//! matrix and weighted accordingly in the scorer, but they are the difference
//! between "these two heroes are equally fine" and "one of them is actually good
//! on this map right now".
//!
//! The third is the same quantity read on each rung of the ladder, which is a
//! larger effect than it sounds: a hero's win rate moves a median of 4.7 points
//! between Bronze and Grandmaster, against an all-ranks between-hero band only
//! 10.7 points wide. It is stored as a *shift* away from the all-ranks figure
//! rather than as a strength of its own, because the scorer weights it
//! separately — see [`rank_shift`] and `score::Weights::rank`.

use std::collections::{HashMap, HashSet};

use overwatch_core::{normalize, Rank, Role};
use overwatch_data::schema::{
    MapAffinityEntry, MapAffinityFile, PrevalenceEntry, PrevalenceFile, StrengthByRankEntry,
    StrengthByRankFile, StrengthEntry, StrengthFile,
};

use crate::blizzard::BlizzardRates;
use crate::counterpickgg::HeroStats;
use crate::counterwatch::HeroRates;

/// Win rates cluster tightly, so the scale is anchored on a fixed band rather
/// than on the observed spread. A patch that compresses every hero towards 50%
/// should read as "nobody stands out", not amplify the remaining noise.
const WIN_RATE_FLOOR: f32 = 44.0;
const WIN_RATE_CEILING: f32 = 56.0;

/// How the three published win rates are weighed against each other.
///
/// Even, which is deliberately *not* the 0.75/0.25 the matchup blend uses. That
/// split was earned by a correlation argument about three sources measuring a
/// construct none of them defines the same way. This is one observable quantity
/// — what fraction of games a hero wins — measured three times, and when
/// instruments read the same thing the average of them beats any one.
///
/// If anything the case runs the other way. counterpickgg publishes a rounded
/// integer with no sample size and no stated method, which on the ±6 point band
/// above quantises the whole roster onto twelve values 16.67 apart;
/// counterwatch publishes a decimal, the number of tracked matches behind it,
/// and says it applies Bayesian shrinkage. Even weighting is the conservative
/// reading, and it keeps three populations in the estimate: the two sites track
/// different players, and one site's community is not the ladder.
///
/// Blizzard is the one that *is* the ladder — first-party, the whole ranked
/// population of a region, and the only source here without a community behind
/// it. It covers all 53 heroes and it disagrees with the other two no more than
/// they disagree with each other: r = 0.755 against counterpickgg and 0.742
/// against counterwatch, where those two sit at 0.871, and mean |Δ| is about 1.2
/// points for every pairing. That is the argument for including it, and it is
/// deliberately *not* an argument for weighting it above a third — any number
/// over 1/3 is one nobody measured.
///
/// It was excluded for a long time by omission rather than by judgement: this
/// step has always fetched it, and used it only as its own internal reference for
/// the rank shifts. What that cost is visible on Torbjörn, whose two community
/// readings of 58.0 and 55.7 put him alone on the rail at +100 while Blizzard
/// read 51.6.
const WIN_RATE_WEIGHT_CPGG: f32 = 0.5;
const WIN_RATE_WEIGHT_CWATCH: f32 = 0.5;
const WIN_RATE_WEIGHT_BLIZZARD: f32 = 0.5;

/// Averages whichever win rates are actually present.
///
/// Renormalised over the sources that answered, so a hero only one site rates
/// is reported at that site's figure rather than dragged toward the others'.
fn blend_win_rate(cpgg: Option<f32>, cwatch: Option<f32>, blizzard: Option<f32>) -> Option<f32> {
    let parts = [
        (cpgg, WIN_RATE_WEIGHT_CPGG),
        (cwatch, WIN_RATE_WEIGHT_CWATCH),
        (blizzard, WIN_RATE_WEIGHT_BLIZZARD),
    ];
    let total: f32 = parts
        .iter()
        .filter(|(v, _)| v.is_some())
        .map(|(_, w)| w)
        .sum();
    if total <= 0.0 {
        return None;
    }
    let sum: f32 = parts.iter().filter_map(|(v, w)| v.map(|v| v * w)).sum();
    Some(sum / total)
}

/// counterwatch's own published shrinkage constant, reused as its weight in the
/// rank shift.
///
/// The site shrinks every division figure toward 50% with 400 imaginary matches,
/// so the number it prints for a bucket of `n` matches is `n/(n+400)` of its own
/// measurement and the rest of its prior. Weighting its shift by exactly that
/// fraction counts the part it measured and not the part it assumed — the same
/// reading of the same invariant as `Matrix::rating` returning `Option`: a bucket
/// the site barely measured is not evidence that a hero is average there.
///
/// What it does on the published medians: Gold (18,536 matches) weighs 0.98 and
/// the two sources land near even, matching the 0.5/0.5 above. Emerald (263)
/// weighs 0.40 and Grandmaster+ (353) weighs 0.47, so Blizzard carries about 70%
/// of the shift at exactly the two rungs where counterwatch has least to say —
/// which is measurably where the two sources stop agreeing (r = 0.51 and 0.50,
/// against 0.72–0.80 at every other rung). No special case for the thin tails;
/// they fall out of this one line.
///
/// The tempting inverse — algebraically undoing the shrinkage with
/// `cw + (400/n)(cw - 50)` — is wrong in the dangerous direction. At Emerald's
/// median it multiplies the deviation by 2.52, amplifying precisely the noise the
/// shrinkage exists to suppress, and hardest in the least trustworthy buckets.
const CWATCH_SHRINKAGE_K: f32 = 400.0;

/// Blizzard's weight in the rank shift.
///
/// Flat, because it publishes no sample sizes: there is nothing per-hero and
/// per-rung to weight it by. It is the whole ranked population of one region
/// rather than a community tracker, and it is the only source whose Master and
/// Grandmaster buckets are not a few hundred matches, so a flat 1.0 is the floor
/// it should never fall below.
const RANK_WEIGHT_BLIZZARD: f32 = 1.0;

/// Smoothing kernel applied across adjacent rungs, normalised by its own sum.
///
/// The claim it rests on: rank is an ordered axis and adjacent rungs are adjacent
/// populations, so a hero's true curve across them is smooth. 26 of 53 heroes
/// already read that way (|Spearman rho| > 0.7 across the eight rungs); the rest
/// show rung-to-rung spikes that are read noise from the thinnest buckets — D.Va
/// at 45.1 Diamond, 51.3 Master, 44.4 Grandmaster+, and Symmetra 51.6 -> 55.7 ->
/// 54.9. Nobody plays a hero that is six points better in Master alone.
///
/// `[1, 2, 1]` is chosen for two properties rather than for taste:
///
/// - **A straight trend is a fixed point.** For any linear sequence,
///   `(x[i-1] + 2*x[i] + x[i+1]) / 4 == x[i]` exactly, so the heroes with a clean
///   monotone rank curve come through untouched. That is the whole requirement:
///   the signal this feature exists for has to survive its own noise filter.
/// - **A one-rung alternation is annihilated.** The response at that frequency is
///   exactly zero, so a value that flips up-down-up between neighbours is removed
///   rather than merely reduced.
///
/// Measured over the roster it takes mean |shift| from 1.24 to 1.06 win-rate
/// points — it removes the noise and leaves the signal.
///
/// A two-tap moving average was rejected because it shifts phase, sliding every
/// hero's trend half a rung up the ladder. A five-tap was rejected because over an
/// eight-point axis it smears Bronze into Diamond, and the Bronze-versus-GM
/// difference is the entire point.
const RANK_SMOOTHING: [f32; 3] = [1.0, 2.0, 1.0];

/// Values given to a hero's 1st, 2nd and 3rd best map.
///
/// The site publishes only the *best* maps, so this signal is positive-only:
/// there is no "bad on this map" data, and a zero means "nothing known" rather
/// than "average". That asymmetry is why the map term carries a low weight.
const MAP_RANK_VALUES: [i8; 3] = [60, 45, 30];

/// One hero's shift away from its own all-ranks win rate, per rung, in win-rate
/// points. `None` where no source covered that rung.
///
/// Each source contributes a shift measured **within itself** — Blizzard against
/// Blizzard's own `tier=All`, counterwatch against the figure on the same page —
/// and the two shifts are then averaged. Never a difference across instruments:
/// `blizzard(h, Diamond) - blend(h, all)` would be part rank effect and part
/// "Blizzard reads 1.3 points differently from counterpickgg", with no way to
/// tell which.
fn rank_shift(
    hero: &str,
    cwatch: Option<&HeroRates>,
    blizzard: &BlizzardRates,
) -> [Option<f32>; Rank::DIVISIONS.len()] {
    let cwatch_baseline = cwatch.map(|rates| rates.all_ranks);
    let blizzard_baseline = blizzard.baseline.get(hero).copied();

    let mut raw = [None; Rank::DIVISIONS.len()];
    for (slot, rank) in raw.iter_mut().zip(Rank::DIVISIONS) {
        let mut weighted = 0.0;
        let mut total = 0.0;

        if let (Some(base), Some(rate)) = (
            blizzard_baseline,
            blizzard
                .by_rank
                .get(&rank)
                .and_then(|t| t.get(hero))
                .copied(),
        ) {
            weighted += RANK_WEIGHT_BLIZZARD * (rate - base);
            total += RANK_WEIGHT_BLIZZARD;
        }

        if let (Some(base), Some(row)) = (
            cwatch_baseline,
            cwatch.and_then(|rates| rates.by_rank.iter().find(|row| row.rank == rank)),
        ) {
            let weight = row.matches as f32 / (row.matches as f32 + CWATCH_SHRINKAGE_K);
            weighted += weight * (row.win_rate - base);
            total += weight;
        }

        // Renormalised over whoever answered, the same way `blend_win_rate` is:
        // a rung only one source covers is reported at that source's figure
        // rather than dragged halfway to zero by a silence.
        if total > 0.0 {
            *slot = Some(weighted / total);
        }
    }

    smooth_across_rungs(raw)
}

/// Applies [`RANK_SMOOTHING`] across the ladder.
///
/// A missing neighbour is extrapolated linearly from the other side rather than
/// reflected. Reflection would break the fixed-point property at exactly the two
/// ends of the ladder, which are the rungs the picker is most often set to: on a
/// straight ramp a reflected Bronze neighbour drags Bronze a quarter of a step
/// toward Silver. With neither neighbour present the rung is left as it was.
fn smooth_across_rungs(
    raw: [Option<f32>; Rank::DIVISIONS.len()],
) -> [Option<f32>; Rank::DIVISIONS.len()] {
    let mut out = raw;
    for index in 0..raw.len() {
        let Some(centre) = raw[index] else { continue };
        let before = index.checked_sub(1).and_then(|i| raw[i]);
        let after = raw.get(index + 1).copied().flatten();

        let left = before
            .or_else(|| after.map(|a| 2.0 * centre - a))
            .unwrap_or(centre);
        let right = after
            .or_else(|| before.map(|b| 2.0 * centre - b))
            .unwrap_or(centre);

        let weights = RANK_SMOOTHING;
        out[index] = Some(
            (weights[0] * left + weights[1] * centre + weights[2] * right)
                / (weights[0] + weights[1] + weights[2]),
        );
    }
    out
}

/// Puts a shift onto the canonical scale by normalising either side of it and
/// subtracting.
///
/// Normalising the shifted *rate* and then subtracting, rather than normalising
/// the shift directly, is what makes a hero with no rank effect store exactly
/// zero — so choosing a division moves nothing for them. It also keeps the two
/// numbers on one scale through the same clamp, instead of one of them measuring
/// a distance the other cannot express.
fn shift_to_value(base_rate: f32, shift: f32) -> i8 {
    let shifted = i16::from(normalize(
        base_rate + shift,
        WIN_RATE_FLOOR,
        WIN_RATE_CEILING,
    ));
    let plain = i16::from(normalize(base_rate, WIN_RATE_FLOOR, WIN_RATE_CEILING));
    (shifted - plain).clamp(-100, 100) as i8
}

pub fn build(
    generated: &str,
    stats: &[HeroStats],
    cwatch_rates: &HashMap<String, HeroRates>,
    blizzard: &BlizzardRates,
    known_maps: &HashSet<String>,
) -> (StrengthFile, MapAffinityFile, StrengthByRankFile) {
    let mut strength = Vec::with_capacity(stats.len());
    let mut by_rank: Vec<StrengthByRankEntry> = Vec::with_capacity(stats.len());
    let mut affinity = Vec::new();
    let mut unknown_maps: Vec<String> = Vec::new();

    for hero in stats {
        let cpgg = Some(hero.win_rate);
        let rates = cwatch_rates.get(&hero.hero);
        let cwatch = rates.map(|rates| rates.all_ranks);
        // The same figure `rank_shift` already uses as Blizzard's own baseline,
        // read here as a third reading of the quantity rather than only as the
        // thing its own rungs are measured against.
        let blizzard_rate = blizzard.baseline.get(&hero.hero).copied();
        let blended = blend_win_rate(cpgg, cwatch, blizzard_rate).unwrap_or(hero.win_rate);

        strength.push(StrengthEntry {
            hero: hero.hero.clone(),
            value: normalize(blended, WIN_RATE_FLOOR, WIN_RATE_CEILING),
            win_rate: Some((blended * 10.0).round() / 10.0),
            cpgg,
            cwatch,
            blizzard: blizzard_rate,
        });

        let shifts = rank_shift(&hero.hero, rates, blizzard);
        if shifts.iter().any(Option::is_some) {
            let mut entry = StrengthByRankEntry {
                hero: hero.hero.clone(),
                ..StrengthByRankEntry::default()
            };
            for (shift, rank) in shifts.iter().zip(Rank::DIVISIONS) {
                entry.set(rank, shift.map(|shift| shift_to_value(blended, shift)));
            }
            by_rank.push(entry);
        }

        for (rank, map) in hero.best_maps.iter().enumerate() {
            let Some(value) = MAP_RANK_VALUES.get(rank) else {
                break;
            };
            if !known_maps.contains(map) {
                unknown_maps.push(map.clone());
                continue;
            }
            affinity.push(MapAffinityEntry {
                map: map.clone(),
                hero: hero.hero.clone(),
                value: *value,
            });
        }
    }

    unknown_maps.sort_unstable();
    unknown_maps.dedup();
    for map in unknown_maps {
        eprintln!("  note: counterpickgg rates map {map:?}, which is not in maps.toml");
    }

    // Stable order so the committed files diff cleanly.
    strength.sort_by(|a, b| a.hero.cmp(&b.hero));
    by_rank.sort_by(|a, b| a.hero.cmp(&b.hero));
    affinity.sort_by(|a, b| (&a.map, &a.hero).cmp(&(&b.map, &b.hero)));

    (
        StrengthFile {
            generated: generated.to_owned(),
            entries: strength,
        },
        MapAffinityFile {
            generated: generated.to_owned(),
            entries: affinity,
        },
        StrengthByRankFile {
            generated: generated.to_owned(),
            entries: by_rank,
        },
    )
}

/// Slots each role fills on a team, in the population Blizzard measured.
///
/// 5v5 competitive role queue, because that is the only thing the endpoint serves
/// — see `blizzard::QUEUE`, which records that there is no open-queue or 6v6
/// response to ask for. Deliberately **not** derived from
/// `overwatch_core::Capacity`, which knows about formats these numbers were never
/// measured over: a fair share divided by 6v6 slots would be a share of a
/// population that did not produce them.
const SLOTS_5V5: [(Role, f32); 3] = [(Role::Tank, 1.0), (Role::Damage, 2.0), (Role::Support, 2.0)];

/// The band a hero's log-ratio to its fair share is stretched over, in octaves.
///
/// ±2 is "a quarter of its fair share up to four times it", and it is chosen so
/// the clamp stays an exception rather than a feature: across the nine published
/// columns it pins 8 of 477 cells and none at all at the all-ranks rung. `±1.0`
/// would pin 19 of 53 heroes there, which is the scale describing the clamp
/// rather than the roster. On the canonical scale the middle half of readings then
/// lands between 18 and 61, so an ordinary hero occupies the middle of the range
/// and the ends are left for the heroes that really are absent or everywhere.
const PREVALENCE_BAND: f32 = 2.0;

/// The pick rate a hero would have if its role's slots were shared out evenly.
///
/// The zero point of `prevalence.toml`, and it holds no data: summed over a role,
/// pick rate comes to exactly `100 x slots(role)` at every rung, because role
/// queue admits no duplicates and the column is P(hero is on a team). So the role
/// mean *is* this number by construction, at every rung, and a hero above it is
/// above it in a sense that does not shift when the patch does.
fn fair_share(role: Role, in_role: usize) -> Option<f32> {
    if in_role == 0 {
        return None;
    }
    let slots = SLOTS_5V5
        .iter()
        .find(|(each, _)| *each == role)
        .map(|(_, slots)| *slots)?;
    Some(100.0 * slots / in_role as f32)
}

/// Puts one published pick rate onto the canonical scale, against its role.
fn prevalence_to_value(pick: f32, share: f32) -> i8 {
    if pick <= 0.0 {
        // A hero nobody picked is a real reading of "as rare as it gets" rather
        // than a missing one, but `log2(0)` is negative infinity, so the floor is
        // stated instead of arrived at. No rung has ever published a zero — the
        // lowest across all nine is 0.8.
        return -100;
    }
    normalize((pick / share).log2(), -PREVALENCE_BAND, PREVALENCE_BAND)
}

/// Builds `prevalence.toml` from the pick rates Blizzard published.
///
/// A function of its own rather than a fourth thing [`build`] returns, because it
/// shares none of that function's inputs except the roster and none of its
/// outputs: it reads one source where `build` blends three, and it is the only
/// thing here that needs to know a hero's role.
///
/// **The columns are deliberately not smoothed**, unlike [`rank_shift`]'s. That
/// kernel exists because a per-rung *win rate* rests on a thin bucket of games and
/// wobbles for want of sample, and none of that applies here: a pick rate is a
/// proportion over every game played at a rung rather than one conditional on the
/// outcome, so it is estimated far better. Only 11 of 53 heroes read monotone
/// across the rungs, and that is the point — a hero genuinely popular in Diamond
/// and not in Master is a fact about the ladder, not read noise. Applying
/// [`smooth_across_rungs`] anyway moves 353 of 424 cells by up to 15 points and
/// leaves mean |value| where it was: churn in the review diff, no change in what
/// the column says.
pub fn prevalence(
    generated: &str,
    blizzard: &BlizzardRates,
    roles: &HashMap<String, Role>,
) -> PrevalenceFile {
    let mut in_role: HashMap<Role, usize> = HashMap::new();
    for role in roles.values() {
        *in_role.entry(*role).or_default() += 1;
    }

    let mut entries: Vec<PrevalenceEntry> = roles
        .iter()
        .filter_map(|(hero, role)| {
            let share = fair_share(*role, in_role.get(role).copied().unwrap_or(0))?;

            let mut entry = PrevalenceEntry {
                hero: hero.clone(),
                ..PrevalenceEntry::default()
            };
            for rank in Rank::CHOICES {
                let pick = blizzard.pick_rate.get(&(rank, hero.clone())).copied();
                entry.set(rank, pick.map(|pick| prevalence_to_value(pick, share)));
            }

            // A hero the source covered at no rung at all leaves no row, rather
            // than a row of nine absences.
            Rank::CHOICES
                .iter()
                .any(|rank| entry.value_for(*rank).is_some())
                .then_some(entry)
        })
        .collect();

    entries.sort_by(|a, b| a.hero.cmp(&b.hero));

    PrevalenceFile {
        generated: generated.to_owned(),
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counterwatch::RankRow;

    fn stats() -> Vec<HeroStats> {
        vec![
            HeroStats {
                hero: "torbjorn".to_owned(),
                win_rate: 58.0,
                pick_rate: 5.0,
                best_maps: vec![
                    "eichenwalde".to_owned(),
                    "havana".to_owned(),
                    "paraiso".to_owned(),
                ],
            },
            HeroStats {
                hero: "ana".to_owned(),
                win_rate: 50.0,
                pick_rate: 12.0,
                best_maps: vec!["nepal".to_owned()],
            },
        ]
    }

    fn known() -> HashSet<String> {
        ["eichenwalde", "havana", "paraiso", "nepal"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// No rank-sliced source answered. The two all-ranks files must come out
    /// exactly as they always did.
    fn no_ranks() -> BlizzardRates {
        BlizzardRates::default()
    }

    /// counterwatch's all-ranks figure, with no rank breakdown behind it.
    fn cwatch_all(pairs: &[(&str, f32)]) -> HashMap<String, HeroRates> {
        pairs
            .iter()
            .map(|(hero, rate)| {
                (
                    (*hero).to_owned(),
                    HeroRates {
                        all_ranks: *rate,
                        by_rank: Vec::new(),
                    },
                )
            })
            .collect()
    }

    /// A Blizzard response for one hero: the baseline, then one rate per rung in
    /// [`Rank::DIVISIONS`] order.
    fn blizzard(hero: &str, baseline: f32, rungs: [f32; 8]) -> BlizzardRates {
        let mut rates = BlizzardRates {
            baseline: [(hero.to_owned(), baseline)].into_iter().collect(),
            ..BlizzardRates::default()
        };
        for (rank, rate) in Rank::DIVISIONS.into_iter().zip(rungs) {
            rates
                .by_rank
                .insert(rank, [(hero.to_owned(), rate)].into_iter().collect());
        }
        rates
    }

    fn curve(file: &StrengthByRankFile, hero: &str) -> Vec<Option<i8>> {
        let entry = file
            .entries
            .iter()
            .find(|e| e.hero == hero)
            .expect("hero has a rank row");
        Rank::DIVISIONS
            .iter()
            .map(|rank| entry.value_for(*rank))
            .collect()
    }

    #[test]
    fn win_rates_map_onto_the_canonical_scale() {
        let (strength, _, _) = build("today", &stats(), &HashMap::new(), &no_ranks(), &known());

        let ana = strength
            .entries
            .iter()
            .find(|e| e.hero == "ana")
            .expect("present");
        assert_eq!(ana.value, 0, "50% is the middle of the 44..56 band");

        let torb = strength
            .entries
            .iter()
            .find(|e| e.hero == "torbjorn")
            .expect("present");
        assert!(torb.value > 0);
        assert_eq!(torb.win_rate, Some(58.0), "the raw rate stays visible");
    }

    /// The disagreement this blend exists for. These are the real committed
    /// figures for Zenyatta: counterpickgg rounds to 55, counterwatch measures
    /// 53.6, and taking the first at face value overstates him by 23 points on
    /// the scale the scorer reads.
    #[test]
    fn the_two_published_win_rates_are_averaged_rather_than_one_being_believed() {
        let zen = vec![HeroStats {
            hero: "zenyatta".to_owned(),
            win_rate: 55.0,
            pick_rate: 8.0,
            best_maps: Vec::new(),
        }];
        let cwatch = cwatch_all(&[("zenyatta", 53.6)]);

        let alone = &build("today", &zen, &HashMap::new(), &no_ranks(), &known())
            .0
            .entries[0];
        let blended = &build("today", &zen, &cwatch, &no_ranks(), &known())
            .0
            .entries[0];

        assert_eq!(alone.value, 83, "counterpickgg on its own");
        assert_eq!(blended.value, 72, "and the average of the two");

        assert_eq!(blended.cpgg, Some(55.0), "both readings stay traceable");
        assert_eq!(blended.cwatch, Some(53.6));
        assert_eq!(blended.win_rate, Some(54.3), "the midpoint of the two");
    }

    /// The band tops out at 56, so a hero both sites put above it saturates
    /// whatever the blend says. Pinned so that nobody reads the blend as a
    /// general fix for the ceiling.
    /// The real case that made this a three-source blend. Torbjörn's two
    /// community readings put him alone on the rail at +100; Blizzard, the one
    /// source that is the ladder rather than a tracker, read him four points
    /// lower and takes him off it.
    #[test]
    fn a_third_reading_pulls_a_two_source_outlier_back() {
        let torb = vec![HeroStats {
            hero: "torbjorn".to_owned(),
            win_rate: 58.0,
            pick_rate: 5.0,
            best_maps: Vec::new(),
        }];
        let cwatch = cwatch_all(&[("torbjorn", 55.7)]);

        let two = &build("today", &torb, &cwatch, &no_ranks(), &known())
            .0
            .entries[0];
        let three = &build(
            "today",
            &torb,
            &cwatch,
            &blizzard("torbjorn", 51.6, [51.6; 8]),
            &known(),
        )
        .0
        .entries[0];

        assert_eq!(two.win_rate, Some(56.9), "the two-source blend");
        assert_eq!(two.value, 100, "which saturates the band");

        assert_eq!(three.win_rate, Some(55.1), "and the average of all three");
        assert_eq!(three.value, 85, "off the rail");
        assert_eq!(
            three.blizzard,
            Some(51.6),
            "every reading stays traceable to the source that published it"
        );
    }

    /// Renormalisation, from the third source's side: a hero only Blizzard rates
    /// is reported at Blizzard's figure rather than dragged toward the two that
    /// said nothing.
    #[test]
    fn a_hero_only_blizzard_rates_keeps_that_figure() {
        let ana = vec![HeroStats {
            hero: "ana".to_owned(),
            win_rate: 50.0,
            pick_rate: 12.0,
            best_maps: Vec::new(),
        }];

        let entry = &build(
            "today",
            &ana,
            &HashMap::new(),
            &blizzard("ana", 53.0, [53.0; 8]),
            &known(),
        )
        .0
        .entries[0];

        // counterpickgg's 50.0 and Blizzard's 53.0, evenly weighted, with
        // counterwatch absent rather than counted as anything.
        assert_eq!(entry.win_rate, Some(51.5));
        assert_eq!(entry.cwatch, None);
        assert_eq!(entry.blizzard, Some(53.0));
    }

    #[test]
    fn a_hero_above_the_band_still_saturates_after_blending() {
        let cwatch = cwatch_all(&[("torbjorn", 55.7)]);

        let (blended, _, _) = build("today", &stats(), &cwatch, &no_ranks(), &known());
        let torb = blended
            .entries
            .iter()
            .find(|e| e.hero == "torbjorn")
            .expect("present");

        assert_eq!(torb.win_rate, Some(56.9));
        assert_eq!(
            torb.value, 100,
            "56.9 is still off the top of the 44..56 band"
        );
    }

    #[test]
    fn a_hero_only_one_site_rates_keeps_that_sites_number() {
        let (only_cpgg, _, _) = build("today", &stats(), &HashMap::new(), &no_ranks(), &known());
        let ana = only_cpgg
            .entries
            .iter()
            .find(|e| e.hero == "ana")
            .expect("present");

        // Renormalised over whoever answered, rather than averaged against a
        // zero that would report a 50% hero as a catastrophe.
        assert_eq!(ana.win_rate, Some(50.0));
        assert_eq!(ana.cwatch, None);
        assert_eq!(ana.value, 0);
    }

    #[test]
    fn map_affinity_decays_by_rank() {
        let (_, affinity, _) = build("today", &stats(), &HashMap::new(), &no_ranks(), &known());

        let value_for = |map: &str| {
            affinity
                .entries
                .iter()
                .find(|e| e.map == map && e.hero == "torbjorn")
                .map(|e| e.value)
        };
        assert_eq!(value_for("eichenwalde"), Some(60));
        assert_eq!(value_for("havana"), Some(45));
        assert_eq!(value_for("paraiso"), Some(30));
    }

    #[test]
    fn maps_we_do_not_know_are_skipped_not_fatal() {
        let mut stats = stats();
        stats[1].best_maps = vec!["atlantis".to_owned()];

        let (_, affinity, _) = build("today", &stats, &HashMap::new(), &no_ranks(), &known());
        assert!(affinity.entries.iter().all(|e| e.hero != "ana"));
    }

    /// The failure this whole file is most exposed to. Every way of getting the
    /// rank fetch wrong — a cache key without its tier, a source that quietly
    /// serves the aggregate — produces eight identical columns, and eight
    /// identical columns must not read as eight measurements.
    #[test]
    fn a_hero_with_no_rank_effect_stores_no_shift_at_all() {
        let flat = blizzard("ana", 50.0, [50.0; 8]);
        let (_, _, by_rank) = build("today", &stats(), &HashMap::new(), &flat, &known());

        assert_eq!(
            curve(&by_rank, "ana"),
            vec![Some(0); 8],
            "a flat curve must leave the ranking exactly where it was"
        );
    }

    /// The signal the feature exists for, in the shape the real data has it:
    /// Reinhardt reads well above the ladder average at Bronze and below it at
    /// Grandmaster.
    #[test]
    fn a_monotone_rank_curve_survives_the_smoothing_that_removes_spikes() {
        let ramp = blizzard(
            "ana",
            50.0,
            [53.0, 52.0, 51.0, 50.0, 49.0, 48.0, 47.0, 46.0],
        );
        let (_, _, by_rank) = build("today", &stats(), &HashMap::new(), &ramp, &known());
        let curve = curve(&by_rank, "ana");

        // A straight line is a fixed point of `[1, 2, 1] / 4`, including at the
        // ends, which is what the linear extrapolation is there to preserve.
        assert_eq!(
            curve,
            vec![
                Some(50),
                Some(33),
                Some(17),
                Some(0),
                Some(-17),
                Some(-33),
                Some(-50),
                Some(-67)
            ],
            "the ramp must come through undamped, ends included"
        );
        for pair in curve.windows(2) {
            assert!(pair[0] > pair[1], "the curve must stay monotone");
        }
    }

    /// The D.Va case: 45.1 at Diamond, 51.3 at Master, 44.4 at Grandmaster+.
    /// Nobody plays a hero that is six points better in Master alone.
    #[test]
    fn a_single_rung_spike_is_cut_down_rather_than_believed() {
        let spike = blizzard(
            "ana",
            50.0,
            [50.0, 50.0, 50.0, 50.0, 50.0, 50.0, 56.0, 50.0],
        );
        let (_, _, by_rank) = build("today", &stats(), &HashMap::new(), &spike, &known());
        let curve = curve(&by_rank, "ana");

        let master = curve[Rank::Master.column().expect("a rung")].expect("rated");
        // 6 points would be +100 on the band; the kernel halves the excursion.
        assert_eq!(master, 50, "the spike survives at half its height");
        for (index, value) in curve.iter().enumerate() {
            if index == Rank::Master.column().expect("a rung") {
                continue;
            }
            assert!(
                value.expect("rated").abs() <= 25,
                "the spike must not smear across the whole ladder"
            );
        }
    }

    /// counterwatch shrinks every division toward 50% with 400 imaginary
    /// matches, so a bucket it barely measured is mostly its own prior. The
    /// weight is that fraction, which means a thin bucket cannot outvote the
    /// source that measured the whole population.
    #[test]
    fn a_thin_counterwatch_bucket_barely_moves_the_shift() {
        let bliz = blizzard("ana", 50.0, [54.0; 8]);

        let thin = HashMap::from([(
            "ana".to_owned(),
            HeroRates {
                all_ranks: 50.0,
                by_rank: Rank::DIVISIONS
                    .into_iter()
                    .map(|rank| RankRow {
                        rank,
                        win_rate: 46.0,
                        matches: 40,
                    })
                    .collect(),
            },
        )]);
        let fat = HashMap::from([(
            "ana".to_owned(),
            HeroRates {
                all_ranks: 50.0,
                by_rank: Rank::DIVISIONS
                    .into_iter()
                    .map(|rank| RankRow {
                        rank,
                        win_rate: 46.0,
                        matches: 40_000,
                    })
                    .collect(),
            },
        )]);

        let read = |cwatch: &HashMap<String, HeroRates>| {
            let (_, _, by_rank) = build("today", &stats(), cwatch, &bliz, &known());
            curve(&by_rank, "ana")[Rank::Gold.column().expect("a rung")].expect("rated")
        };

        let alone = {
            let (_, _, by_rank) = build("today", &stats(), &HashMap::new(), &bliz, &known());
            curve(&by_rank, "ana")[Rank::Gold.column().expect("a rung")].expect("rated")
        };

        assert!(
            (read(&thin) - alone).abs() < (read(&fat) - alone).abs(),
            "40 matches must pull less than 40,000 do"
        );
        assert!(
            read(&fat) < read(&thin),
            "a well-measured disagreement has to actually land"
        );
    }

    /// A rung only one source covers is reported at that source's figure, the
    /// same way `blend_win_rate` renormalises over whoever answered rather than
    /// dragging a hero halfway to zero because the other side was silent.
    #[test]
    fn a_rung_no_source_covered_is_left_out_rather_than_written_as_even() {
        let mut partial = blizzard("ana", 50.0, [54.0; 8]);
        partial
            .by_rank
            .get_mut(&Rank::Emerald)
            .expect("a rung")
            .remove("ana");

        let (_, _, by_rank) = build("today", &stats(), &HashMap::new(), &partial, &known());
        let entry = by_rank
            .entries
            .iter()
            .find(|e| e.hero == "ana")
            .expect("present");

        assert_eq!(
            entry.value_for(Rank::Emerald),
            None,
            "an uncovered rung is absent, not zero"
        );
        assert!(
            entry.value_for(Rank::Diamond).is_some_and(|v| v > 0),
            "the rungs either side still read"
        );
    }

    /// The two all-ranks files must not notice this feature exists.
    #[test]
    fn a_run_with_no_rank_source_leaves_the_all_ranks_files_untouched() {
        let (with, _, by_rank) = build(
            "today",
            &stats(),
            &HashMap::new(),
            &blizzard("ana", 50.0, [55.0; 8]),
            &known(),
        );
        let (without, _, empty) = build("today", &stats(), &HashMap::new(), &no_ranks(), &known());

        let values =
            |file: &StrengthFile| -> Vec<i8> { file.entries.iter().map(|e| e.value).collect() };
        assert_eq!(values(&with), values(&without));
        assert!(empty.entries.is_empty(), "no source, no file");
        assert!(!by_rank.entries.is_empty());
    }

    #[test]
    fn output_is_ordered_for_a_clean_diff() {
        let (strength, affinity, _) =
            build("today", &stats(), &HashMap::new(), &no_ranks(), &known());

        let heroes: Vec<_> = strength.entries.iter().map(|e| e.hero.as_str()).collect();
        let mut sorted = heroes.clone();
        sorted.sort();
        assert_eq!(heroes, sorted);

        let pairs: Vec<_> = affinity
            .entries
            .iter()
            .map(|e| (e.map.as_str(), e.hero.as_str()))
            .collect();
        let mut sorted_pairs = pairs.clone();
        sorted_pairs.sort();
        assert_eq!(pairs, sorted_pairs);
    }
}
