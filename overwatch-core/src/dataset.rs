use crate::error::CoreError;
use crate::hero::{Hero, HeroId, Role};
use crate::map::{GameMap, MapId};
use crate::matrix::Matrix;

/// Everything the scorer needs, resolved into dense index space.
///
/// Built once at startup from the committed TOML and then treated as immutable.
/// Personal overrides deliberately live outside it, in [`crate::score::UserContext`],
/// so regenerating the dataset from a scrape can never clobber user tuning.
#[derive(Debug, Clone)]
pub struct Dataset {
    heroes: Vec<Hero>,
    maps: Vec<GameMap>,
    matchups: Matrix,
    synergy: Matrix,
    /// Row-major `maps.len() x heroes.len()`.
    map_affinity: Vec<i8>,
    base_strength: Vec<i8>,
    /// Positive leans attack, negative leans defend. Only consulted on the modes
    /// that have sides at all.
    side_lean: Vec<i8>,
    /// Row-major `n x n`, parallel to `matchups`. Empty string means "no text".
    reasons: Vec<String>,
    /// Free-form provenance shown in the UI so stale data is visible.
    pub generated: String,
    pub patch: String,
}

/// Inputs for [`Dataset::new`], kept as a struct because there are enough
/// parallel arrays here that positional arguments would be easy to transpose.
#[derive(Debug, Clone)]
pub struct DatasetParts {
    pub heroes: Vec<Hero>,
    pub maps: Vec<GameMap>,
    pub matchups: Matrix,
    pub synergy: Matrix,
    pub map_affinity: Vec<i8>,
    pub base_strength: Vec<i8>,
    pub side_lean: Vec<i8>,
    pub reasons: Vec<String>,
    pub generated: String,
    pub patch: String,
}

impl Dataset {
    pub fn new(parts: DatasetParts) -> Result<Self, CoreError> {
        let n = parts.heroes.len();
        if n > crate::hero::HeroSet::CAPACITY {
            return Err(CoreError::RosterTooLarge(n, crate::hero::HeroSet::CAPACITY));
        }
        if parts.matchups.n() != n {
            return Err(CoreError::MatrixShape {
                n,
                expected: n * n,
                actual: parts.matchups.n() * parts.matchups.n(),
            });
        }
        if parts.synergy.n() != n {
            return Err(CoreError::MatrixShape {
                n,
                expected: n * n,
                actual: parts.synergy.n() * parts.synergy.n(),
            });
        }
        if parts.base_strength.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "base_strength",
                expected: n,
                actual: parts.base_strength.len(),
            });
        }
        if parts.side_lean.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "side_lean",
                expected: n,
                actual: parts.side_lean.len(),
            });
        }
        if parts.reasons.len() != n * n {
            return Err(CoreError::RosterLengthMismatch {
                what: "reasons",
                expected: n * n,
                actual: parts.reasons.len(),
            });
        }
        let expected_affinity = parts.maps.len() * n;
        if parts.map_affinity.len() != expected_affinity {
            return Err(CoreError::RosterLengthMismatch {
                what: "map_affinity",
                expected: expected_affinity,
                actual: parts.map_affinity.len(),
            });
        }

        Ok(Self {
            heroes: parts.heroes,
            maps: parts.maps,
            matchups: parts.matchups,
            synergy: parts.synergy,
            map_affinity: parts.map_affinity,
            base_strength: parts.base_strength,
            side_lean: parts.side_lean,
            reasons: parts.reasons,
            generated: parts.generated,
            patch: parts.patch,
        })
    }

    pub fn hero_count(&self) -> usize {
        self.heroes.len()
    }

    pub fn heroes(&self) -> &[Hero] {
        &self.heroes
    }

    pub fn maps(&self) -> &[GameMap] {
        &self.maps
    }

    pub fn hero(&self, id: HeroId) -> Result<&Hero, CoreError> {
        self.heroes
            .get(id.index())
            .ok_or(CoreError::UnknownHero(id.0, self.heroes.len()))
    }

    pub fn map(&self, id: MapId) -> Result<&GameMap, CoreError> {
        self.maps
            .get(id.index())
            .ok_or(CoreError::UnknownMap(id.0, self.maps.len()))
    }

    pub fn hero_by_key(&self, key: &str) -> Result<HeroId, CoreError> {
        self.heroes
            .iter()
            .position(|h| h.key == key)
            .map(|i| HeroId(i as u16))
            .ok_or_else(|| CoreError::UnknownHeroKey(key.to_owned()))
    }

    pub fn map_by_key(&self, key: &str) -> Result<MapId, CoreError> {
        self.maps
            .iter()
            .position(|m| m.key == key)
            .map(|i| MapId(i as u16))
            .ok_or_else(|| CoreError::UnknownMapKey(key.to_owned()))
    }

    pub fn heroes_in_role(&self, role: Role) -> impl Iterator<Item = HeroId> + '_ {
        self.heroes
            .iter()
            .enumerate()
            .filter(move |(_, h)| h.role == role)
            .map(|(i, _)| HeroId(i as u16))
    }

    pub fn matchups(&self) -> &Matrix {
        &self.matchups
    }

    pub fn synergy(&self) -> &Matrix {
        &self.synergy
    }

    pub fn base_strength(&self, hero: HeroId) -> i8 {
        self.base_strength.get(hero.index()).copied().unwrap_or(0)
    }

    /// How much this hero prefers attacking over defending, on -100..=100.
    /// Zero — the value for most of the roster — means it makes no difference.
    pub fn side_lean(&self, hero: HeroId) -> i8 {
        self.side_lean.get(hero.index()).copied().unwrap_or(0)
    }

    /// Neutral (0) when the map is unknown or affinity data is missing, so a
    /// partially-populated dataset still scores rather than failing.
    pub fn map_affinity(&self, map: MapId, hero: HeroId) -> i8 {
        let n = self.heroes.len();
        if map.index() >= self.maps.len() || hero.index() >= n {
            return 0;
        }
        self.map_affinity
            .get(map.index() * n + hero.index())
            .copied()
            .unwrap_or(0)
    }

    /// Human-readable rationale for `attacker` vs `defender`, scraped from
    /// counterpickgg. This is what makes the "why" panel real text rather than
    /// a number dressed up as an explanation.
    pub fn reason(&self, attacker: HeroId, defender: HeroId) -> Option<&str> {
        let n = self.heroes.len();
        if attacker.index() >= n || defender.index() >= n {
            return None;
        }
        self.reasons
            .get(attacker.index() * n + defender.index())
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
}
