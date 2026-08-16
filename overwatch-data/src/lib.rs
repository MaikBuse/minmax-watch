//! Loads the committed dataset in `data/` into an [`overwatch_core::Dataset`].
//!
//! The TOML is embedded at compile time with `include_str!` because the client
//! runs as wasm in a browser tab and has no filesystem. Regenerating the data
//! therefore means rebuilding, which is intentional: `data/` is reviewed as a
//! git diff, and that review is the curation step.

#![forbid(unsafe_code)]

pub mod schema;

use std::collections::HashMap;

use overwatch_core::{
    Dataset, DatasetParts, GameMap, GameMode, Hero, HeroId, MapId, Matrix, Role, Subrole,
};
use thiserror::Error;

use crate::schema::{
    ArchetypeFile, HeroesFile, MapAffinityFile, MapsFile, MatchupsFile, SideFile, StrengthFile,
    SynergyFile,
};

pub const HEROES_TOML: &str = include_str!("../../data/heroes.toml");
pub const MAPS_TOML: &str = include_str!("../../data/maps.toml");
pub const MATCHUPS_TOML: &str = include_str!("../../data/matchups.toml");
pub const MAP_AFFINITY_TOML: &str = include_str!("../../data/map_affinity.toml");
pub const SYNERGY_TOML: &str = include_str!("../../data/synergy.toml");
pub const STRENGTH_TOML: &str = include_str!("../../data/strength.toml");
pub const SIDE_TOML: &str = include_str!("../../data/side.toml");
pub const ARCHETYPE_TOML: &str = include_str!("../../data/archetype.toml");

#[derive(Debug, Error)]
pub enum DataError {
    #[error("failed to parse {file}: {source}")]
    Parse {
        file: &'static str,
        #[source]
        source: toml::de::Error,
    },

    #[error("{file} refers to hero {key:?}, which is not in heroes.toml")]
    UnknownHero { file: &'static str, key: String },

    #[error("{file} refers to map {key:?}, which is not in maps.toml")]
    UnknownMap { file: &'static str, key: String },

    #[error("heroes.toml lists {key:?} more than once")]
    DuplicateHero { key: String },

    #[error("maps.toml lists {key:?} more than once")]
    DuplicateMap { key: String },

    #[error(transparent)]
    Core(#[from] overwatch_core::CoreError),
}

fn parse<T: serde::de::DeserializeOwned>(file: &'static str, text: &str) -> Result<T, DataError> {
    toml::from_str(text).map_err(|source| DataError::Parse { file, source })
}

/// Parses the embedded dataset. Call once at startup and share the result.
pub fn load() -> Result<Dataset, DataError> {
    load_from(Sources {
        heroes: HEROES_TOML,
        maps: MAPS_TOML,
        matchups: MATCHUPS_TOML,
        map_affinity: MAP_AFFINITY_TOML,
        synergy: SYNERGY_TOML,
        strength: STRENGTH_TOML,
        side: SIDE_TOML,
        archetype: ARCHETYPE_TOML,
    })
}

/// The eight TOML documents that make up a dataset, as raw text.
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub heroes: &'a str,
    pub maps: &'a str,
    pub matchups: &'a str,
    pub map_affinity: &'a str,
    pub synergy: &'a str,
    pub strength: &'a str,
    pub side: &'a str,
    pub archetype: &'a str,
}

pub fn load_from(sources: Sources<'_>) -> Result<Dataset, DataError> {
    let heroes_file: HeroesFile = parse("heroes.toml", sources.heroes)?;
    let maps_file: MapsFile = parse("maps.toml", sources.maps)?;
    let matchups_file: MatchupsFile = parse("matchups.toml", sources.matchups)?;
    let affinity_file: MapAffinityFile = parse("map_affinity.toml", sources.map_affinity)?;
    let synergy_file: SynergyFile = parse("synergy.toml", sources.synergy)?;
    let strength_file: StrengthFile = parse("strength.toml", sources.strength)?;
    let side_file: SideFile = parse("side.toml", sources.side)?;
    let archetype_file: ArchetypeFile = parse("archetype.toml", sources.archetype)?;

    // --- roster -----------------------------------------------------------
    let mut hero_index: HashMap<String, HeroId> = HashMap::new();
    let mut heroes: Vec<Hero> = Vec::with_capacity(heroes_file.heroes.len());
    for entry in &heroes_file.heroes {
        if hero_index.contains_key(&entry.key) {
            return Err(DataError::DuplicateHero {
                key: entry.key.clone(),
            });
        }
        hero_index.insert(entry.key.clone(), HeroId(heroes.len() as u16));
        let subrole = entry.subrole.as_deref().map(Subrole::parse).transpose()?;
        heroes.push(Hero {
            key: entry.key.clone(),
            name: entry.name.clone(),
            role: Role::parse(&entry.role)?,
            subrole,
            aliases: entry.aliases.clone(),
        });
    }
    let n = heroes.len();

    let mut map_index: HashMap<String, MapId> = HashMap::new();
    let mut maps: Vec<GameMap> = Vec::with_capacity(maps_file.maps.len());
    for entry in &maps_file.maps {
        if map_index.contains_key(&entry.key) {
            return Err(DataError::DuplicateMap {
                key: entry.key.clone(),
            });
        }
        map_index.insert(entry.key.clone(), MapId(maps.len() as u16));
        maps.push(GameMap {
            key: entry.key.clone(),
            name: entry.name.clone(),
            mode: GameMode::parse(&entry.mode)?,
            aliases: entry.aliases.clone(),
        });
    }

    let hero_id = |file: &'static str, key: &str| -> Result<HeroId, DataError> {
        hero_index
            .get(key)
            .copied()
            .ok_or_else(|| DataError::UnknownHero {
                file,
                key: key.to_owned(),
            })
    };

    // --- matchups and their rationale ------------------------------------
    let mut matchups = Matrix::unrated(n);
    let mut reasons = vec![String::new(); n * n];
    for entry in &matchups_file.matchups {
        let a = hero_id("matchups.toml", &entry.hero)?;
        let b = hero_id("matchups.toml", &entry.vs)?;
        matchups.set(a, b, entry.value)?;
        if !entry.reason.is_empty() {
            if let Some(slot) = reasons.get_mut(a.index() * n + b.index()) {
                *slot = entry.reason.clone();
            }
        }
    }

    // --- synergy ----------------------------------------------------------
    let mut synergy = Matrix::unrated(n);
    for entry in &synergy_file.entries {
        let a = hero_id("synergy.toml", &entry.hero)?;
        let b = hero_id("synergy.toml", &entry.with)?;
        let value = entry.resolved();
        synergy.set(a, b, value)?;
        if entry.symmetric {
            synergy.set(b, a, value)?;
        }
    }

    // --- map affinity -----------------------------------------------------
    let mut map_affinity = vec![0i8; maps.len() * n];
    for entry in &affinity_file.entries {
        let hero = hero_id("map_affinity.toml", &entry.hero)?;
        let map = map_index
            .get(&entry.map)
            .copied()
            .ok_or_else(|| DataError::UnknownMap {
                file: "map_affinity.toml",
                key: entry.map.clone(),
            })?;
        if let Some(slot) = map_affinity.get_mut(map.index() * n + hero.index()) {
            *slot = entry.value;
        }
    }

    // --- base strength ----------------------------------------------------
    // The published rate rides along beside the normalised value it was scaled
    // from. Nothing scores on it — it is what lets a panel arguing from patch
    // strength show a figure a reader can check against the source.
    let mut base_strength = vec![0i8; n];
    let mut win_rate = vec![None; n];
    for entry in &strength_file.entries {
        let hero = hero_id("strength.toml", &entry.hero)?;
        if let Some(slot) = base_strength.get_mut(hero.index()) {
            *slot = entry.value;
        }
        if let Some(slot) = win_rate.get_mut(hero.index()) {
            *slot = entry.win_rate;
        }
    }

    // --- attack/defend lean -----------------------------------------------
    // Absent from the file means zero, which is the honest answer for most of
    // the roster rather than a gap to be filled.
    let mut side_lean = vec![0i8; n];
    for entry in &side_file.entries {
        let hero = hero_id("side.toml", &entry.hero)?;
        if let Some(slot) = side_lean.get_mut(hero.index()) {
            *slot = entry.value;
        }
    }

    // --- playstyle axes ---------------------------------------------------
    // All-zero for a hero the file does not mention, which reads downstream as
    // "nobody has curated this kit" rather than as a hero that wants none of
    // the three fights — see `overwatch_core::archetype::shape_of`.
    let mut shape = vec![[0i8; 3]; n];
    for entry in &archetype_file.entries {
        let hero = hero_id("archetype.toml", &entry.hero)?;
        if let Some(slot) = shape.get_mut(hero.index()) {
            *slot = [entry.dive, entry.poke, entry.brawl];
        }
    }

    Ok(Dataset::new(DatasetParts {
        heroes,
        maps,
        matchups,
        synergy,
        map_affinity,
        base_strength,
        win_rate,
        side_lean,
        shape,
        reasons,
        generated: matchups_file.generated.clone(),
        patch: matchups_file.patch.clone(),
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed dataset must always load. This is the guard against a bad
    /// ingest landing in the repo.
    #[test]
    fn embedded_dataset_loads() {
        let ds = load().expect("committed data/ must parse");
        assert!(
            ds.hero_count() > 0,
            "roster is empty - has `just ingest` been run?"
        );
    }

    #[test]
    fn unknown_hero_references_are_rejected() {
        let sources = Sources {
            heroes: r#"
                [[hero]]
                key = "reinhardt"
                name = "Reinhardt"
                role = "tank"
            "#,
            maps: "",
            matchups: r#"
                [[matchup]]
                hero = "reinhardt"
                vs = "sombra"
                value = -20
            "#,
            map_affinity: "",
            synergy: "",
            strength: "",
            side: "",
            archetype: "",
        };

        let err = load_from(sources).expect_err("dangling reference must fail");
        assert!(matches!(
            err,
            DataError::UnknownHero {
                file: "matchups.toml",
                ..
            }
        ));
    }

    #[test]
    fn duplicate_hero_keys_are_rejected() {
        let sources = Sources {
            heroes: r#"
                [[hero]]
                key = "ana"
                name = "Ana"
                role = "support"
                [[hero]]
                key = "ana"
                name = "Ana"
                role = "support"
            "#,
            maps: "",
            matchups: "",
            map_affinity: "",
            synergy: "",
            strength: "",
            side: "",
            archetype: "",
        };

        assert!(matches!(
            load_from(sources),
            Err(DataError::DuplicateHero { .. })
        ));
    }

    #[test]
    fn synergy_defaults_to_symmetric() {
        let sources = Sources {
            heroes: r#"
                [[hero]]
                key = "zarya"
                name = "Zarya"
                role = "tank"
                [[hero]]
                key = "mercy"
                name = "Mercy"
                role = "support"
            "#,
            maps: "",
            matchups: "",
            map_affinity: "",
            synergy: r#"
                [[synergy]]
                hero = "zarya"
                with = "mercy"
                value = 40
            "#,
            strength: "",
            side: "",
            archetype: "",
        };

        let ds = load_from(sources).expect("loads");
        let zarya = ds.hero_by_key("zarya").expect("present");
        let mercy = ds.hero_by_key("mercy").expect("present");
        assert_eq!(ds.synergy().get(zarya, mercy), 40);
        assert_eq!(ds.synergy().get(mercy, zarya), 40);
    }
}
