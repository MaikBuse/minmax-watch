//! Turns the counterpickgg index table into `strength.toml` and
//! `map_affinity.toml`.
//!
//! Both signals come from one fetch. They are weaker than the matchup matrix
//! and weighted accordingly in the scorer, but they are the difference between
//! "these two heroes are equally fine" and "one of them is actually good on
//! this map right now".

use std::collections::HashSet;

use overwatch_core::normalize;
use overwatch_data::schema::{MapAffinityEntry, MapAffinityFile, StrengthEntry, StrengthFile};

use crate::counterpickgg::HeroStats;

/// Win rates cluster tightly, so the scale is anchored on a fixed band rather
/// than on the observed spread. A patch that compresses every hero towards 50%
/// should read as "nobody stands out", not amplify the remaining noise.
const WIN_RATE_FLOOR: f32 = 44.0;
const WIN_RATE_CEILING: f32 = 56.0;

/// Values given to a hero's 1st, 2nd and 3rd best map.
///
/// The site publishes only the *best* maps, so this signal is positive-only:
/// there is no "bad on this map" data, and a zero means "nothing known" rather
/// than "average". That asymmetry is why the map term carries a low weight.
const MAP_RANK_VALUES: [i8; 3] = [60, 45, 30];

pub fn build(
    generated: &str,
    stats: &[HeroStats],
    known_maps: &HashSet<String>,
) -> (StrengthFile, MapAffinityFile) {
    let mut strength = Vec::with_capacity(stats.len());
    let mut affinity = Vec::new();
    let mut unknown_maps: Vec<String> = Vec::new();

    for hero in stats {
        strength.push(StrengthEntry {
            hero: hero.hero.clone(),
            value: normalize(hero.win_rate, WIN_RATE_FLOOR, WIN_RATE_CEILING),
            win_rate: Some(hero.win_rate),
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
        let (strength, _) = build("today", &stats(), &known());

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

    #[test]
    fn map_affinity_decays_by_rank() {
        let (_, affinity) = build("today", &stats(), &known());

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

        let (_, affinity) = build("today", &stats, &known());
        assert!(affinity.entries.iter().all(|e| e.hero != "ana"));
    }

    #[test]
    fn output_is_ordered_for_a_clean_diff() {
        let (strength, affinity) = build("today", &stats(), &known());

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
