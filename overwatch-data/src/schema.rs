//! Serde mirrors of the committed TOML in `data/`.
//!
//! These types are shared by the loader and by `overwatch-ingest`, so the
//! generator and the consumer can never drift apart: if the ingest writes a
//! field the loader does not understand, it fails to compile rather than
//! silently dropping data.

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
    /// Set when the sources disagree by more than the blend threshold. Surfaced
    /// in the UI so bad data is visible rather than quietly averaged away.
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

/// Hand-curated pair synergies. No scraped source publishes these, so this file
/// starts empty and is filled in by hand as we learn what actually works for us.
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
    pub value: i8,
    /// Synergy is a property of the pair, so entries apply in both directions
    /// unless this is explicitly cleared.
    #[serde(default = "default_true")]
    pub symmetric: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_rate: Option<f32>,
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
