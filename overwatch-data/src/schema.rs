//! Serde mirrors of the committed TOML in `data/`.
//!
//! These types are shared by the loader and by `overwatch-ingest`, so the
//! generator and the consumer can never drift apart: if the ingest writes a
//! field the loader does not understand, it fails to compile rather than
//! silently dropping data.

use overwatch_core::Rank;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeroesFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, rename = "hero")]
    pub heroes: Vec<HeroEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeroEntry {
    pub key: String,
    pub name: String,
    pub role: String,
    /// The sub-role the roster API publishes, which decides the hero's passive.
    /// Skipped when absent so a roster generated before sub-roles existed still
    /// round-trips unchanged rather than growing an empty column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subrole: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MapsFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, rename = "map")]
    pub maps: Vec<MapEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEntry {
    pub key: String,
    pub name: String,
    pub mode: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MatchupsFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default)]
    pub patch: String,
    #[serde(default, rename = "matchup")]
    pub matchups: Vec<MatchupEntry>,
}

/// One directed matchup: how `hero` fares against `vs`, from `hero`'s side.
///
/// Per-source values are kept alongside the blend so a suspicious number can be
/// traced back to whichever site produced it, and so the review diff shows
/// *which* source moved when data changes. They are separate scalar fields
/// rather than a nested table because TOML array-of-tables entries must put all
/// scalars before any sub-table, and a flat layout reviews better in a diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchupEntry {
    pub hero: String,
    pub vs: String,
    /// Blended, on -100..=100.
    pub value: i8,
    /// Set when the sources disagree by more than the blend threshold.
    ///
    /// Read by `Dataset::sources_disagree`, which asks about the *pair* — the
    /// flag lands on a direction, and the secondary source rates only part of
    /// each hero's list, so a contradiction routinely marks one row of two.
    /// Reaches the screen as a `disputed` tag beside the matchup and a marker on
    /// the reason line, so a blend nobody could reconcile is visible rather than
    /// quietly averaged away.
    #[serde(default, skip_serializing_if = "is_false")]
    pub disagreement: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpgg: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opick: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwatch: Option<i8>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl MatchupEntry {
    /// The per-source readings that were actually available, paired with the
    /// source name for reporting.
    pub fn source_values(&self) -> Vec<(&'static str, i8)> {
        [
            ("counterpickgg", self.cpgg),
            ("overpicker", self.opick),
            ("counterwatch", self.cwatch),
        ]
        .into_iter()
        .filter_map(|(name, v)| v.map(|v| (name, v)))
        .collect()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MapAffinityFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default, rename = "affinity")]
    pub entries: Vec<MapAffinityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapAffinityEntry {
    pub map: String,
    pub hero: String,
    pub value: i8,
}

/// Pair synergies.
///
/// The one file in `data/` that is both generated and hand-curated, which is
/// why it carries a source column and an override column rather than a bare
/// value. counterwatch publishes a short top-N of duo partners per hero with a
/// measured "% above expected" beside each, which is real evidence but covers
/// only the pairs it chose to list; `curated` is how a pair it does not list
/// gets an opinion, and it wins wherever both are present.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SynergyFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default, rename = "synergy")]
    pub entries: Vec<SynergyEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynergyEntry {
    pub hero: String,
    pub with: String,
    /// The resolved number the loader reads: `curated` if set, else `cwatch`.
    pub value: i8,
    /// counterwatch's reading, kept beside the resolved value so a suspicious
    /// number can be traced back to the source that produced it — the same
    /// reason `matchups.toml` keeps its per-source columns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwatch: Option<i8>,
    /// A hand-written override. The ingest never writes this and must never
    /// drop it: it is the only way to rate a pair no source has listed, and
    /// re-running the scrape must not silently discard somebody's judgement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curated: Option<i8>,
    /// Why the curated value is what it is. Same bar as `side.toml`: a number
    /// with no source behind it is indistinguishable from a typo unless it
    /// says why.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Synergy is a property of the pair, so entries apply in both directions
    /// unless this is explicitly cleared.
    #[serde(default = "default_true")]
    pub symmetric: bool,
}

impl SynergyEntry {
    /// What the loader should score. Curated judgement beats the scrape,
    /// because the only reason to write one is that the scrape is wrong or
    /// silent about this pair.
    pub fn resolved(&self) -> i8 {
        self.curated.or(self.cwatch).unwrap_or(self.value)
    }
}

/// General hero strength independent of matchup, derived from published win
/// rates. Keeps the "is this hero just good right now" signal separate from the
/// "does it beat that hero" signal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrengthFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default, rename = "strength")]
    pub entries: Vec<StrengthEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrengthEntry {
    pub hero: String,
    pub value: i8,
    /// The blended rate `value` was derived from. Display only — nothing scores
    /// on it, because ranking on it directly would disagree with `value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate: Option<f32>,
    /// The two published readings behind the blend, kept for the same reason
    /// `matchups.toml` keeps its per-source columns: the sites disagree
    /// systematically, and a surprising number should be traceable to whichever
    /// one produced it without re-running the scrape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpgg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwatch: Option<f32>,
}

/// How far each hero's strength moves from the ladder average on each rung of
/// it, on the same -100..=100 scale and against the same win-rate band as
/// [`StrengthEntry::value`].
///
/// A separate file from `strength.toml` rather than eight more columns on it,
/// for the same reason `map_affinity.toml` is separate: `stats::build` produces
/// both from one fetch and the split is by what the number means. It also buys
/// a failure mode — half of this file is reachable only through Blizzard, and a
/// run that could not reach it leaves the committed file alone instead of
/// needing merge logic to keep a failed fetch from blanking eight columns.
///
/// **Only strength is sliced by rank**, and it is the one signal that can be:
/// neither source publishes per-rung matchups or duos. See [`overwatch_core::Rank`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrengthByRankFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default, rename = "strength")]
    pub entries: Vec<StrengthByRankEntry>,
}

/// One hero's whole rank curve, as a shift away from its all-ranks strength.
///
/// Columns rather than one row per (hero, rung). The diff review is the curation
/// step, and a spurious Master-only spike is visible in a column of eight numbers
/// and invisible when those eight are thirty lines apart. It is also a third of
/// the bytes, which the wasm bundle carries.
///
/// A shift rather than an absolute strength because the scorer weights it
/// separately from the all-ranks value — the two terms decompose rather than
/// double-count. It also keeps `strength.toml` byte-shaped exactly as it was, so
/// the diff of this feature is one new file and nothing else moved.
///
/// Every column is optional and skipped when absent, and absent is **not** zero
/// in the file: zero is a real and common reading meaning "no rank effect
/// measured", while an omitted column means no source covered that rung. Same
/// distinction [`overwatch_core::Matrix::rating`] draws by returning `Option`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrengthByRankEntry {
    pub hero: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bronze: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silver: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platinum: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emerald: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diamond: Option<i8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master: Option<i8>,
    /// Grandmaster and Champion together, which is the finest slice either
    /// source publishes. Named without the `+` both sites display, because this
    /// is a TOML key — see [`overwatch_core::Rank::label`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grandmaster: Option<i8>,
}

impl StrengthByRankEntry {
    /// The column for one rung, so the loader and the ingest walk
    /// [`Rank::DIVISIONS`] instead of naming eight fields at every site. Adding a
    /// rung to the ladder is then a non-exhaustive-match error here rather than a
    /// column nothing reads.
    ///
    /// [`Rank::All`] has no column of its own — it is the value every column is
    /// measured against — so it reads as no shift.
    pub fn value_for(&self, rank: Rank) -> Option<i8> {
        match rank {
            Rank::All => None,
            Rank::Bronze => self.bronze,
            Rank::Silver => self.silver,
            Rank::Gold => self.gold,
            Rank::Platinum => self.platinum,
            Rank::Emerald => self.emerald,
            Rank::Diamond => self.diamond,
            Rank::Master => self.master,
            Rank::Grandmaster => self.grandmaster,
        }
    }

    /// The mirror of [`Self::value_for`], so the ingest builds a row by walking
    /// the same list the loader reads it with.
    pub fn set(&mut self, rank: Rank, value: Option<i8>) {
        match rank {
            // Not an error and not a panic: this crate is on the wasm path and
            // the ingest walks `Rank::DIVISIONS`, which never yields it.
            Rank::All => {}
            Rank::Bronze => self.bronze = value,
            Rank::Silver => self.silver = value,
            Rank::Gold => self.gold = value,
            Rank::Platinum => self.platinum = value,
            Rank::Emerald => self.emerald = value,
            Rank::Diamond => self.diamond = value,
            Rank::Master => self.master = value,
            Rank::Grandmaster => self.grandmaster = value,
        }
    }
}

/// Hand-curated attack/defend leanings. No source publishes these, so like
/// `synergy.toml` this file is written by hand and the ingest never touches it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SideFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default, rename = "side")]
    pub entries: Vec<SideEntry>,
}

/// How much one hero prefers attacking, on -100..=100. Negative leans defend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideEntry {
    pub hero: String,
    pub value: i8,
    /// Why this hero leans the way it does. Curated numbers with no source
    /// behind them are worth a sentence, or the next person cannot tell a
    /// considered value from a typo.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Hand-curated playstyle axes. Like `synergy.toml` and `side.toml` this is
/// written by hand and the ingest never touches it — the scraped sources are
/// pairwise duels, and a duel says nothing about the shape of the team it
/// happens inside.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchetypeFile {
    #[serde(default)]
    pub generated: String,
    #[serde(default, rename = "archetype")]
    pub entries: Vec<ArchetypeEntry>,
}

/// How much one hero wants each kind of fight, on 0..=100 per axis.
///
/// The three do not sum to anything. Most kits are genuinely strong on two —
/// Zarya brawls but travels with a dive — and normalising them would make every
/// hero a specialist by arithmetic rather than by kit. An absent axis is zero,
/// which is the honest reading for a hero the axis simply does not describe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchetypeEntry {
    pub hero: String,
    #[serde(default)]
    pub dive: i8,
    #[serde(default)]
    pub poke: i8,
    #[serde(default)]
    pub brawl: i8,
    /// Why this hero reads the way it does. Curated numbers with no source
    /// behind them are worth a sentence, or the next person cannot tell a
    /// considered value from a typo.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

fn default_true() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}
