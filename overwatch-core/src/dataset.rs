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
    /// **Nothing about a pair is sliced this way.** No source publishes per-rung
    /// matchups or duos, so `matchups` and `synergy` gain no third axis and
    /// nothing on screen may suggest they did. See [`Rank`].
    rank_shift: Vec<[i8; Rank::DIVISIONS.len()]>,
    /// How often each hero is picked relative to its role's fair share, one row
    /// per hero in [`Rank::CHOICES`] order.
    ///
    /// **Nine columns where `rank_shift` has eight**, indexed by [`Rank::slot`]
    /// and not [`Rank::column`]. A shift is measured against the aggregate so the
    /// aggregate has none of its own; a pick rate at all ranks is a published
    /// reading like every other rung's. Crossing the two index spaces reads every
    /// rung one column out and passes every count-based test.
    ///
    /// Zero means "picked exactly as often as its role's share", which is the
    /// honest reading for an unremarkable hero and also what an absent row gives
    /// — the two say the same thing to the one term that reads this.
    prevalence: Vec<[i8; Rank::CHOICES.len()]>,
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
    /// Row-major `n x n`, parallel to `matchups`: the sources that rated this
    /// direction contradicted each other by more than the blend threshold.
    ///
    /// Read through [`Self::sources_disagree`] rather than directly, because the
    /// contradiction belongs to the pair and the flag lands on a direction.
    disputed: Vec<bool>,
    /// How many unordered hero pairs any source rated, and how many of those
    /// carry a published sentence.
    ///
    /// Counted in [`Self::new`] rather than written down anywhere, because the
    /// panel that quotes them is making a claim about *this* bundle's tables and
    /// a figure typed into copy is wrong the first time the ingest runs. Read
    /// through [`Self::pairs_rated`] and [`Self::notes_published`].
    ///
    /// **Pairs and not directed rows**, because both are claims about a pair —
    /// the sources have an opinion about this matchup, the site wrote a sentence
    /// about it — and a halved row count is not the same number. It happens to
    /// agree today (1,330 pairs behind 2,660 rated rows, 533 behind 1,066
    /// sentences) and nothing guarantees it, in exactly the sense
    /// [`Self::sources_disagree`] documents: the blend can rate one direction
    /// whose mirror it says nothing about, and it writes a sentence only where it
    /// found one.
    pairs_rated: usize,
    notes_published: usize,
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
    /// One row per hero in [`Rank::CHOICES`] order — nine wide, indexed by
    /// [`Rank::slot`]. See the field of the same name on [`Dataset`].
    pub prevalence: Vec<[i8; Rank::CHOICES.len()]>,
    pub win_rate: Vec<Option<f32>>,
    pub side_lean: Vec<i8>,
    pub shape: Vec<[i8; 3]>,
    pub reasons: Vec<String>,
    /// Row-major `n x n`, parallel to `matchups`: the sources that rated this
    /// direction contradicted each other by more than the blend threshold.
    pub disputed: Vec<bool>,
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
        if parts.prevalence.len() != n {
            return Err(CoreError::RosterLengthMismatch {
                what: "prevalence",
                expected: n,
                actual: parts.prevalence.len(),
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
        if parts.disputed.len() != n * n {
            return Err(CoreError::RosterLengthMismatch {
                what: "disputed",
                expected: n * n,
                actual: parts.disputed.len(),
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

        // Coverage, over unordered pairs. `rating` and not `get`: an unrated cell
        // and a rated dead even both read as 0 through `get`, and a quarter of
        // this matrix is rated even, so a `!= 0` test would report a fraction of
        // the pairs the app actually has an opinion about.
        let mut pairs_rated = 0usize;
        let mut notes_published = 0usize;
        let noted = |x: usize, y: usize| {
            parts
                .reasons
                .get(x * n + y)
                .is_some_and(|text| !text.is_empty())
        };
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (HeroId(i as u16), HeroId(j as u16));
                if parts.matchups.rating(a, b).is_none() && parts.matchups.rating(b, a).is_none() {
                    continue;
                }
                pairs_rated += 1;
                if noted(i, j) || noted(j, i) {
                    notes_published += 1;
                }
            }
        }

        Ok(Self {
            heroes: parts.heroes,
            maps: parts.maps,
            matchups: parts.matchups,
            synergy: parts.synergy,
            map_affinity: parts.map_affinity,
            base_strength: parts.base_strength,
            rank_shift: parts.rank_shift,
            prevalence: parts.prevalence,
            win_rate: parts.win_rate,
            side_lean: parts.side_lean,
            shape: parts.shape,
            reasons: parts.reasons,
            disputed: parts.disputed,
            pairs_rated,
            notes_published,
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

    /// How often this hero is picked on one rung of the ladder, relative to its
    /// role's fair share, on the same -100..=100 scale as everything else.
    ///
    /// Positive means picked more often than its role's slots divided evenly;
    /// negative means less. Zero is the reading for an unremarkable hero, and also
    /// what a hero the source never covered gets — unlike a matchup, the two are
    /// the same answer to the only question anything asks of this: apply no
    /// discount.
    ///
    /// Read at [`Rank::slot`], because this table has a column for the whole
    /// ladder where [`Self::rank_shift`] cannot: a shift away from the aggregate
    /// is zero by definition, and a pick rate at all ranks is a figure Blizzard
    /// publishes.
    ///
    /// Out-of-range lookups read as neutral rather than panicking, as
    /// [`Self::rank_shift`] does: a draft in progress must never take down the UI
    /// over a bad index.
    pub fn prevalence_at(&self, rank: Rank, hero: HeroId) -> i8 {
        self.prevalence
            .get(hero.index())
            .and_then(|row| row.get(rank.slot()))
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

    /// Whether the sources contradicted each other about this pair, in either
    /// direction.
    ///
    /// Read as `at(a, b) || at(b, a)` on purpose. The sources disagree about the
    /// *pair*, and the flag lands on a direction. The ingest reaches its verdict
    /// per pair and so writes both rows — 164 of the 2534 directed rows today,
    /// from 82 flagged pairs — but that is the ingest's choice rather than this
    /// function's guarantee, and it has not always been true: `winston vs zarya`
    /// used to be flagged at `+100` against `-31` while its mirror escaped the
    /// flag purely because the secondary source was silent on the other side.
    /// Reading one direction would have hidden the contradiction from whichever
    /// half of the draft happened to be on screen, and it would do so again the
    /// next time where the flag lands moves.
    ///
    /// Gated on [`Matrix::rating`] rather than on a presence bit of its own: a
    /// pair nobody rated cannot be one the sources fought over, and [`Matrix`]
    /// keeps "rated" in one place so the two cannot drift apart.
    pub fn sources_disagree(&self, a: HeroId, b: HeroId) -> bool {
        let n = self.heroes.len();
        if a.index() >= n || b.index() >= n {
            return false;
        }
        if self.matchups.rating(a, b).is_none() && self.matchups.rating(b, a).is_none() {
            return false;
        }
        let at = |x: HeroId, y: HeroId| {
            self.disputed
                .get(x.index() * n + y.index())
                .copied()
                .unwrap_or(false)
        };
        at(a, b) || at(b, a)
    }

    /// How many hero pairs any source rated at all.
    ///
    /// The denominator for a coverage claim on screen, counted off the matrix
    /// the scorer reads rather than off the file. See the field for why it counts
    /// pairs and not directed rows.
    pub fn pairs_rated(&self) -> usize {
        self.pairs_rated
    }

    /// How many rated pairs carry a written rationale.
    ///
    /// Always the numerator to [`Self::pairs_rated`]: a sentence about a pair
    /// nobody rated is not counted, because there is nothing on screen for it to
    /// explain.
    pub fn notes_published(&self) -> usize {
        self.notes_published
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{GameMap, GameMode};

    const A: HeroId = HeroId(0);
    const B: HeroId = HeroId(1);

    /// Two heroes, with `disputed` and the matchup matrix handed in so a test can
    /// describe exactly the asymmetry it is about.
    fn dataset(matchups: Matrix, disputed: Vec<bool>) -> Dataset {
        let n = 2;
        dataset_with_reasons(matchups, disputed, vec![String::new(); n * n])
    }

    /// The same two heroes, with the rationale text handed in as well, for the
    /// counts that read it.
    fn dataset_with_reasons(
        matchups: Matrix,
        disputed: Vec<bool>,
        reasons: Vec<String>,
    ) -> Dataset {
        let heroes = ["a", "b"]
            .into_iter()
            .map(|key| Hero {
                key: key.to_owned(),
                name: key.to_owned(),
                role: Role::Tank,
                subrole: None,
                aliases: Vec::new(),
            })
            .collect::<Vec<_>>();
        let n = heroes.len();

        Dataset::new(DatasetParts {
            heroes,
            maps: vec![GameMap {
                key: "kings-row".to_owned(),
                name: "King's Row".to_owned(),
                mode: GameMode::Hybrid,
                aliases: Vec::new(),
            }],
            matchups,
            synergy: Matrix::unrated(n),
            map_affinity: vec![0; n],
            base_strength: vec![0; n],
            rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
            prevalence: vec![[0; Rank::CHOICES.len()]; n],
            win_rate: vec![None; n],
            side_lean: vec![0; n],
            shape: vec![[0; 3]; n],
            reasons,
            disputed,
            generated: "test".to_owned(),
            patch: "test".to_owned(),
        })
        .expect("a two-hero dataset is valid")
    }

    /// The counts are over pairs, so a reading on one side of a pair counts
    /// once — not half of one, and not twice.
    ///
    /// Committed data has no one-sided pair today, which is exactly why this is
    /// pinned here: the day the blend rates a direction whose mirror it says
    /// nothing about, a halved row count reports a fraction of a pair, and a
    /// sentence written on one side only reports half a sentence.
    #[test]
    fn a_pair_rated_in_one_direction_only_still_counts_once() {
        let mut matchups = Matrix::unrated(2);
        matchups.set(A, B, 40).expect("in range");

        let reasons = vec![
            String::new(),
            "A dives B before the bubble is up.".to_owned(),
            String::new(),
            String::new(),
        ];
        let ds = dataset_with_reasons(matchups, vec![false; 4], reasons);

        assert_eq!(ds.pairs_rated(), 1, "one pair, rated on one side");
        assert_eq!(ds.notes_published(), 1, "one sentence about that one pair");
    }

    /// The distinction the whole coverage claim rests on, and the reason the walk
    /// reads `Matrix::rating` and never `Matrix::get`: a pair rated dead even is
    /// an opinion, and an unrated pair is the absence of one. Through `get` they
    /// are the same zero.
    #[test]
    fn a_rated_dead_even_counts_as_coverage_and_an_unrated_pair_does_not() {
        let unrated =
            dataset_with_reasons(Matrix::unrated(2), vec![false; 4], vec![String::new(); 4]);
        assert_eq!(unrated.pairs_rated(), 0);

        let mut even = Matrix::unrated(2);
        even.set(A, B, 0).expect("in range");
        even.set(B, A, 0).expect("in range");
        let rated = dataset_with_reasons(even, vec![false; 4], vec![String::new(); 4]);
        assert_eq!(rated.pairs_rated(), 1);
        // Nothing was written about it, so it is coverage without a sentence —
        // which is the majority of the committed matrix.
        assert_eq!(rated.notes_published(), 0);
    }

    /// The Winston/Zarya shape: the secondary source rated one direction and
    /// contradicted the primary there, and said nothing about the mirror. Both
    /// halves of the draft have to see the same dispute.
    #[test]
    fn a_pair_flagged_in_either_direction_reads_as_disputed() {
        let mut matchups = Matrix::unrated(2);
        matchups.set(A, B, 67).expect("in range");
        matchups.set(B, A, -100).expect("in range");

        let flagged_forward_only = vec![false, true, false, false];
        let ds = dataset(matchups.clone(), flagged_forward_only);
        assert!(ds.sources_disagree(A, B));
        assert!(
            ds.sources_disagree(B, A),
            "the mirror escaped flagging only because one source was silent on it"
        );

        let flagged_neither = vec![false; 4];
        let quiet = dataset(matchups, flagged_neither);
        assert!(!quiet.sources_disagree(A, B));
        assert!(!quiet.sources_disagree(B, A));
    }

    /// A pair nobody rated cannot be one the sources fought over. The flag is
    /// gated on the matrix rather than trusted on its own, so a stray `true` in a
    /// hand-built dataset cannot invent a dispute about nothing.
    #[test]
    fn an_unrated_pair_is_never_disputed() {
        let ds = dataset(Matrix::unrated(2), vec![true; 4]);
        assert!(!ds.sources_disagree(A, B));
        assert!(!ds.sources_disagree(B, A));
    }

    /// A draft in progress must never take down the UI over a bad index, which
    /// is the rule every other lookup here follows.
    #[test]
    fn an_out_of_range_hero_is_not_disputed_rather_than_a_panic() {
        let mut matchups = Matrix::unrated(2);
        matchups.set(A, B, 40).expect("in range");
        let ds = dataset(matchups, vec![true; 4]);

        assert!(!ds.sources_disagree(HeroId(99), B));
        assert!(!ds.sources_disagree(A, HeroId(99)));
    }
}
