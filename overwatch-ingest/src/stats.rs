//! Turns the counterpickgg index table into `strength.toml` and
//! `map_affinity.toml`.
//!
//! Both signals come from one fetch. They are weaker than the matchup matrix
//! and weighted accordingly in the scorer, but they are the difference between
//! "these two heroes are equally fine" and "one of them is actually good on
//! this map right now".

use std::collections::{HashMap, HashSet};

use overwatch_core::normalize;
use overwatch_data::schema::{MapAffinityEntry, MapAffinityFile, StrengthEntry, StrengthFile};

use crate::counterpickgg::HeroStats;

/// Win rates cluster tightly, so the scale is anchored on a fixed band rather
/// than on the observed spread. A patch that compresses every hero towards 50%
/// should read as "nobody stands out", not amplify the remaining noise.
const WIN_RATE_FLOOR: f32 = 44.0;
const WIN_RATE_CEILING: f32 = 56.0;

/// How the two published win rates are weighed against each other.
///
/// Even, which is deliberately *not* the 0.75/0.25 the matchup blend uses. That
/// split was earned by a correlation argument about three sources measuring a
/// construct none of them defines the same way. This is one observable quantity
/// — what fraction of games a hero wins — measured twice, and when two
/// instruments read the same thing the average of both beats either.
///
/// If anything the case runs the other way. counterpickgg publishes a rounded
/// integer with no sample size and no stated method, which on the ±6 point band
/// above quantises the whole roster onto twelve values 16.67 apart;
/// counterwatch publishes a decimal, the number of tracked matches behind it,
/// and says it applies Bayesian shrinkage. Even weighting is the conservative
/// reading, and it keeps a second population in the estimate: the two sites
/// track different players, and one site's community is not the ladder.
const WIN_RATE_WEIGHT_CPGG: f32 = 0.5;
const WIN_RATE_WEIGHT_CWATCH: f32 = 0.5;

/// Averages whichever win rates are actually present.
///
/// Renormalised over the sources that answered, so a hero only one site rates
/// is reported at that site's figure rather than dragged halfway to zero.
fn blend_win_rate(cpgg: Option<f32>, cwatch: Option<f32>) -> Option<f32> {
    let parts = [
        (cpgg, WIN_RATE_WEIGHT_CPGG),
        (cwatch, WIN_RATE_WEIGHT_CWATCH),
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

/// Values given to a hero's 1st, 2nd and 3rd best map.
///
/// The site publishes only the *best* maps, so this signal is positive-only:
/// there is no "bad on this map" data, and a zero means "nothing known" rather
/// than "average". That asymmetry is why the map term carries a low weight.
const MAP_RANK_VALUES: [i8; 3] = [60, 45, 30];

pub fn build(
    generated: &str,
    stats: &[HeroStats],
    cwatch_rates: &HashMap<String, f32>,
    known_maps: &HashSet<String>,
) -> (StrengthFile, MapAffinityFile) {
    let mut strength = Vec::with_capacity(stats.len());
    let mut affinity = Vec::new();
    let mut unknown_maps: Vec<String> = Vec::new();

    for hero in stats {
        let cpgg = Some(hero.win_rate);
        let cwatch = cwatch_rates.get(&hero.hero).copied();
        let blended = blend_win_rate(cpgg, cwatch).unwrap_or(hero.win_rate);

        strength.push(StrengthEntry {
            hero: hero.hero.clone(),
            value: normalize(blended, WIN_RATE_FLOOR, WIN_RATE_CEILING),
            win_rate: Some((blended * 10.0).round() / 10.0),
            cpgg,
            cwatch,
        });

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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn win_rates_map_onto_the_canonical_scale() {
        let (strength, _) = build("today", &stats(), &HashMap::new(), &known());

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
        let cwatch: HashMap<String, f32> =
            [("zenyatta".to_owned(), 53.6_f32)].into_iter().collect();

        let alone = &build("today", &zen, &HashMap::new(), &known()).0.entries[0];
        let blended = &build("today", &zen, &cwatch, &known()).0.entries[0];

        assert_eq!(alone.value, 83, "counterpickgg on its own");
        assert_eq!(blended.value, 72, "and the average of the two");

        assert_eq!(blended.cpgg, Some(55.0), "both readings stay traceable");
        assert_eq!(blended.cwatch, Some(53.6));
        assert_eq!(blended.win_rate, Some(54.3), "the midpoint of the two");
    }

    /// The band tops out at 56, so a hero both sites put above it saturates
    /// whatever the blend says. Pinned so that nobody reads the blend as a
    /// general fix for the ceiling.
    #[test]
    fn a_hero_above_the_band_still_saturates_after_blending() {
        let cwatch: HashMap<String, f32> =
            [("torbjorn".to_owned(), 55.7_f32)].into_iter().collect();

        let (blended, _) = build("today", &stats(), &cwatch, &known());
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
        let (only_cpgg, _) = build("today", &stats(), &HashMap::new(), &known());
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
        let (_, affinity) = build("today", &stats(), &HashMap::new(), &known());

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

        let (_, affinity) = build("today", &stats, &HashMap::new(), &known());
        assert!(affinity.entries.iter().all(|e| e.hero != "ana"));
    }

    #[test]
    fn output_is_ordered_for_a_clean_diff() {
        let (strength, affinity) = build("today", &stats(), &HashMap::new(), &known());

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
