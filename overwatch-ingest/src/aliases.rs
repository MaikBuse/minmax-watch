//! Short forms typed during a draft.
//!
//! This module is small but it is where the speed requirement is actually won:
//! within a restricted pool, a good alias makes a pick resolvable in one or two
//! keystrokes. OverFast does not publish nicknames, so the community shorthand
//! is curated here and anything unknown falls back to derived forms.

use std::collections::HashMap;

/// Community shorthand, keyed by OverFast hero key.
const HERO_ALIASES: &[(&str, &[&str])] = &[
    ("ana", &["ana"]),
    ("ashe", &["ashe"]),
    ("baptiste", &["bap", "bapt"]),
    ("bastion", &["bast"]),
    ("brigitte", &["brig"]),
    ("cassidy", &["cass", "mccree"]),
    ("doomfist", &["doom", "df"]),
    ("dva", &["dva"]),
    ("echo", &["echo"]),
    ("freja", &["freja"]),
    ("genji", &["genji"]),
    ("hanzo", &["hanzo"]),
    ("hazard", &["haz"]),
    ("illari", &["illari"]),
    ("jetpack-cat", &["cat", "jpc"]),
    ("junker-queen", &["jq", "queen"]),
    ("junkrat", &["rat", "junk"]),
    ("juno", &["juno"]),
    ("kiriko", &["kiri"]),
    ("lifeweaver", &["lw", "weaver"]),
    ("lucio", &["lucio"]),
    ("mauga", &["mauga"]),
    ("mei", &["mei"]),
    ("mercy", &["mercy"]),
    ("mizuki", &["miz"]),
    ("moira", &["moira"]),
    ("orisa", &["orisa"]),
    ("pharah", &["pharah"]),
    ("ramattra", &["ram"]),
    ("reaper", &["reap"]),
    ("reinhardt", &["rein", "rh"]),
    ("roadhog", &["hog", "road"]),
    ("sigma", &["sig"]),
    ("sojourn", &["soj"]),
    ("soldier-76", &["76", "s76", "soldier"]),
    ("sombra", &["somb"]),
    ("symmetra", &["sym"]),
    ("torbjorn", &["torb"]),
    ("tracer", &["tracer"]),
    ("venture", &["vent"]),
    ("widowmaker", &["widow"]),
    ("winston", &["winston", "monkey"]),
    ("wrecking-ball", &["ball", "hammond", "wb"]),
    ("wuyang", &["wu"]),
    ("zarya", &["zar"]),
    ("zenyatta", &["zen"]),
];

/// Shorthand for the maps whose full names are slow to type.
const MAP_ALIASES: &[(&str, &[&str])] = &[
    ("antarctic-peninsula", &["ap", "antarctic"]),
    ("blizzard-world", &["bw", "blizz"]),
    ("circuit-royal", &["cr", "circuit"]),
    ("kings-row", &["kr", "kings"]),
    ("lijiang-tower", &["lt", "lijiang"]),
    ("new-queen-street", &["nqs"]),
    ("new-junk-city", &["njc"]),
    ("shambali-monastery", &["shambali"]),
    ("throne-of-anubis", &["throne", "toa"]),
    ("watchpoint-gibraltar", &["gib", "wpg"]),
];

fn lookup(table: &[(&str, &[&str])], key: &str) -> Vec<String> {
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, aliases)| aliases.iter().map(|a| (*a).to_owned()).collect())
        .unwrap_or_default()
}

/// Derived fallbacks for anything not in the curated tables - notably the
/// newest heroes, who have no settled community shorthand yet.
fn derived(key: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();

    // The key with separators stripped: `wrecking-ball` -> `wreckingball`.
    let flat: String = key.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !flat.is_empty() {
        out.push(flat.clone());
    }

    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    // Initials for multi-word names: `Junker Queen` -> `jq`.
    if words.len() > 1 {
        let initials: String = words
            .iter()
            .filter_map(|w| w.chars().next())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if initials.len() >= 2 {
            out.push(initials);
        }
    }

    // A three-letter prefix is usually enough inside a restricted pool.
    if flat.chars().count() > 3 {
        let prefix: String = flat.chars().take(3).collect();
        out.push(prefix);
    }

    out
}

/// Curated aliases first, then derived ones, deduplicated and lowercased.
pub fn for_hero(key: &str, name: &str) -> Vec<String> {
    merge(lookup(HERO_ALIASES, key), derived(key, name))
}

pub fn for_map(key: &str, name: &str) -> Vec<String> {
    merge(lookup(MAP_ALIASES, key), derived(key, name))
}

fn merge(curated: Vec<String>, derived: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for alias in curated.into_iter().chain(derived) {
        let alias = alias.trim().to_ascii_lowercase();
        if !alias.is_empty() && !out.contains(&alias) {
            out.push(alias);
        }
    }
    out
}

/// Aliases that resolve to more than one entry.
///
/// These are not fatal - the UI ranks candidates and shows them - but an
/// ambiguous alias costs a keystroke, so the ingest reports them for curation.
pub fn collisions(entries: &[(String, Vec<String>)]) -> Vec<(String, Vec<String>)> {
    let mut by_alias: HashMap<&str, Vec<String>> = HashMap::new();
    for (key, aliases) in entries {
        for alias in aliases {
            by_alias
                .entry(alias.as_str())
                .or_default()
                .push(key.clone());
        }
    }

    let mut out: Vec<(String, Vec<String>)> = by_alias
        .into_iter()
        .filter(|(_, keys)| keys.len() > 1)
        .map(|(alias, keys)| (alias.to_owned(), keys))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_aliases_come_first() {
        let aliases = for_hero("reinhardt", "Reinhardt");
        assert_eq!(aliases.first().map(String::as_str), Some("rein"));
        assert!(aliases.contains(&"rh".to_owned()));
    }

    #[test]
    fn unknown_heroes_still_get_usable_short_forms() {
        // A 2026 hero with no settled nickname yet.
        let aliases = for_hero("jetpack-cat", "Jetpack Cat");
        assert!(aliases.contains(&"cat".to_owned()));

        let aliases = for_hero("dmon", "D.Mon");
        assert!(aliases.contains(&"dmon".to_owned()));
        assert!(aliases.contains(&"dmo".to_owned()));
    }

    #[test]
    fn multi_word_names_get_initials() {
        let aliases = for_hero("wrecking-ball", "Wrecking Ball");
        assert!(aliases.contains(&"ball".to_owned()));
        assert!(aliases.contains(&"wb".to_owned()));
    }

    #[test]
    fn aliases_are_deduplicated() {
        let aliases = for_hero("ana", "Ana");
        let mut sorted = aliases.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), aliases.len());
    }

    #[test]
    fn collisions_are_detected() {
        let entries = vec![
            ("mei".to_owned(), vec!["mei".to_owned()]),
            ("mercy".to_owned(), vec!["mer".to_owned(), "mei".to_owned()]),
        ];
        let found = collisions(&entries);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "mei");
    }
}
