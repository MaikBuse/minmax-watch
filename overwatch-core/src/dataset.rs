use crate::error::CoreError;
use crate::hero::{Hero, HeroId, Role};
use crate::map::{GameMap, MapId};
use crate::matrix::Matrix;
use crate::rank::Rank;

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
    /// How far each hero's strength moves from the ladder average on each rung
    /// of it, one row per hero in [`Rank::DIVISIONS`] order.
    ///
    /// A shift rather than a strength of its own, because the scorer weights it
    /// separately from `base_strength` — the two decompose rather than
    /// double-count. Zero is the common and correct reading: no rank effect
    /// measured, so choosing a rung moves nothing for that hero.
    ///
    /// **Only strength is sliced this way.** No source publishes per-rung
    /// matchups or duos, so `matchups` and `synergy` gain no third axis and
    /// nothing on screen may suggest they did. See [`Rank`].
    rank_shift: Vec<[i8; Rank::DIVISIONS.len()]>,
    /// The raw published win rate behind `base_strength`, as a percentage.
    ///
    /// Carried alongside rather than derived back out of it, because
    /// `base_strength` is a normalised i8 and the reverse mapping is lossy. It
    /// is never scored on — the scorer reads `base_strength` — and exists so a
    /// panel arguing from patch strength can show the evidence rather than a
    /// rescaled number nobody can check against a stats site.
    ///
    /// `None` for a hero the source published no rate for.
    win_rate: Vec<Option<f32>>,
    /// Positive leans attack, negative leans defend. Only consulted on the modes
    /// that have sides at all.
    side_lean: Vec<i8>,
    /// Dive, poke and brawl per hero, in [`crate::archetype::Archetype::ALL`]
    /// order.
    ///
    /// All-zero means "nobody has read this kit yet", which is deliberately the
    /// same state as a hero absent from the file — see
    /// [`crate::archetype::shape_of`] for why that silence is left out of a
    /// team's mean rather than counted as a shape of its own.
    shape: Vec<[i8; 3]>,
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
    pub rank_shift: Vec<[i8; Rank::DIVISIONS.len()]>,
    pub win_rate: Vec<Option<f32>>,
    pub side_lean: Vec<i8>,
    pub shape: Vec<[i8; 3]>,
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
        if parts.rank_shift.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "rank_shift",
                expected: n,
                actual: parts.rank_shift.len(),
            });
        }
        if parts.win_rate.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "win_rate",
                expected: n,
                actual: parts.win_rate.len(),
            });
        }
        if parts.side_lean.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "side_lean",
                expected: n,
                actual: parts.side_lean.len(),
            });
        }
        if parts.shape.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "shape",
                expected: n,
                actual: parts.shape.len(),
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
            rank_shift: parts.rank_shift,
            win_rate: parts.win_rate,
            side_lean: parts.side_lean,
            shape: parts.shape,
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

    /// How far this hero's strength moves from the ladder average on one rung of
    /// it, on the same -100..=100 scale as [`Self::base_strength`].
    ///
    /// Zero for [`Rank::All`] by construction — the aggregate is what every rung
    /// is measured against — and zero for a hero no source rated at that rung.
    /// Both mean the same thing here and both are the honest answer: no rank
    /// effect to apply.
    pub fn rank_shift(&self, rank: Rank, hero: HeroId) -> i8 {
        let Some(column) = rank.column() else {
            return 0;
        };
        self.rank_shift
            .get(hero.index())
            .and_then(|row| row.get(column))
            .copied()
            .unwrap_or(0)
    }

    /// Patch strength as read on one rung of the ladder.
    ///
    /// [`Self::base_strength`] plus [`Self::rank_shift`], clamped back onto the
    /// canonical scale. For the one place a *single* number has to stand for
    /// "strong right now" — the ban list's patch rung, which sorts on it. The
    /// scorer itself does not use this: it weights the two halves separately so
    /// they can be argued about separately.
    pub fn base_strength_at(&self, rank: Rank, hero: HeroId) -> i8 {
        let combined = i16::from(self.base_strength(hero)) + i16::from(self.rank_shift(rank, hero));
        combined.clamp(-100, 100) as i8
    }

    /// The published win rate as a percentage, where the source gave one.
    ///
    /// For display only. Ranking on it directly would disagree with
    /// [`Self::base_strength`], which is what the scorer uses and what the
    /// heroes missing a rate are still ordered by.
    pub fn win_rate(&self, hero: HeroId) -> Option<f32> {
        self.win_rate.get(hero.index()).copied().flatten()
    }

    /// How much this hero prefers attacking over defending, on -100..=100.
    /// Zero — the value for most of the roster — means it makes no difference.
    pub fn side_lean(&self, hero: HeroId) -> i8 {
        self.side_lean.get(hero.index()).copied().unwrap_or(0)
    }

    /// How much this hero wants each kind of fight — dive, poke, brawl — on
    /// 0..=100 each, in [`crate::archetype::Archetype::ALL`] order.
    ///
    /// All-zero for a hero nobody has curated, and for one the axes genuinely
    /// do not describe. The two are the same silence and are treated the same.
    pub fn shape(&self, hero: HeroId) -> [i8; 3] {
        self.shape.get(hero.index()).copied().unwrap_or([0; 3])
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
