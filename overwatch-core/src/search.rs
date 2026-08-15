//! Resolving what you typed into a hero, fast.
//!
//! This is where the speed requirement is actually met. A hero select gives you
//! seconds, so the matcher is tuned so that the shortest thing you would
//! plausibly type resolves to exactly one hero:
//!
//! - a curated alias wins outright — `rh` is Reinhardt, never Roadhog by
//!   subsequence;
//! - then prefixes, so `rei` lands before anything that merely contains those
//!   letters;
//! - then subsequences, so `wrb` still finds Wrecking Ball.
//!
//! Ties break towards the shorter name, because short names are the ones people
//! type when they are in a hurry.
//!
//! A generic fuzzy matcher would rank purely on character positions and get the
//! alias cases wrong, which is why this is hand-rolled rather than pulled in.

use crate::dataset::Dataset;
use crate::hero::{HeroId, HeroSet, Role};
use crate::map::MapId;

/// How a candidate matched, best first. The discriminant order *is* the
/// ranking, so keep the variants ordered from strongest to weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    AliasExact,
    KeyExact,
    AliasPrefix,
    NamePrefix,
    NameWordPrefix,
    Subsequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub hero: HeroId,
    pub kind: MatchKind,
    /// Length of the matched text; shorter is a tighter fit.
    pub length: usize,
}

/// Which heroes are allowed to come back.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    /// `None` accepts any role — used for enemy picks, which are not restricted
    /// to the role you happen to be playing.
    pub role: Option<Role>,
    /// An empty pool means no restriction.
    pub pool: HeroSet,
    /// Heroes already taken, filtered out so they cannot be entered twice.
    pub exclude: HeroSet,
}

impl Scope {
    /// Enemy picks: any hero, no pool restriction. The enemy team is not
    /// limited to what you happen to play.
    pub fn any() -> Self {
        Self {
            role: None,
            pool: HeroSet::empty(),
            exclude: HeroSet::empty(),
        }
    }

    /// Your own picks: your role, and your pool if you have set one. Narrowing
    /// the candidate set is what makes one or two keystrokes enough.
    pub fn mine(role: Role, pool: HeroSet) -> Self {
        Self {
            role: Some(role),
            pool,
            exclude: HeroSet::empty(),
        }
    }

    pub fn excluding(mut self, exclude: HeroSet) -> Self {
        self.exclude = exclude;
        self
    }

    fn admits(&self, dataset: &Dataset, hero: HeroId) -> bool {
        if self.exclude.contains(hero) {
            return false;
        }
        if !self.pool.is_empty() && !self.pool.contains(hero) {
            return false;
        }
        match self.role {
            Some(role) => dataset.hero(hero).map(|h| h.role == role).unwrap_or(false),
            None => true,
        }
    }
}

/// Case- and punctuation-insensitive comparison key.
///
/// Hero names carry characters nobody types mid-draft — `D.Va`, `Lúcio`,
/// `Soldier: 76`, `Torbjörn` — so both sides are reduced to bare alphanumerics
/// with the common accents folded. Typing `lucio` or `dva` has to just work.
fn fold(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => Some('a'),
            'é' | 'è' | 'ê' | 'ë' => Some('e'),
            'í' | 'ì' | 'î' | 'ï' => Some('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => Some('o'),
            'ú' | 'ù' | 'û' | 'ü' => Some('u'),
            'ñ' => Some('n'),
            'ç' => Some('c'),
            c if c.is_ascii_alphanumeric() => Some(c.to_ascii_lowercase()),
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Whether `needle`'s characters appear in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle
        .chars()
        .all(|target| chars.any(|candidate| candidate == target))
}

/// Best match kind for one hero, or `None` if the query does not match at all.
fn classify(dataset: &Dataset, hero: HeroId, query: &str) -> Option<(MatchKind, usize)> {
    let entry = dataset.hero(hero).ok()?;
    let name = fold(&entry.name);
    let key = fold(&entry.key);

    let mut best: Option<(MatchKind, usize)> = None;
    let mut consider = |kind: MatchKind, length: usize| {
        if best.is_none_or(|(current, _)| kind < current) {
            best = Some((kind, length));
        }
    };

    for alias in &entry.aliases {
        let alias = fold(alias);
        if alias == query {
            consider(MatchKind::AliasExact, alias.len());
        } else if alias.starts_with(query) {
            consider(MatchKind::AliasPrefix, alias.len());
        }
    }

    if key == query {
        consider(MatchKind::KeyExact, key.len());
    }
    if name.starts_with(query) || key.starts_with(query) {
        consider(MatchKind::NamePrefix, name.len());
    }

    // `Junker Queen` should be reachable by typing `queen`.
    if entry
        .name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|word| fold(word).starts_with(query))
    {
        consider(MatchKind::NameWordPrefix, name.len());
    }

    if is_subsequence(query, &name) || is_subsequence(query, &key) {
        consider(MatchKind::Subsequence, name.len());
    }

    best
}

/// Heroes matching `query`, best first.
///
/// An empty query returns everything in scope, so the list is browsable before
/// you have typed anything.
pub fn search(dataset: &Dataset, query: &str, scope: &Scope, limit: usize) -> Vec<Match> {
    let query = fold(query);

    let mut out: Vec<Match> = (0..dataset.hero_count())
        .map(|i| HeroId(i as u16))
        .filter(|hero| scope.admits(dataset, *hero))
        .filter_map(|hero| {
            if query.is_empty() {
                return dataset.hero(hero).ok().map(|entry| Match {
                    hero,
                    kind: MatchKind::Subsequence,
                    length: entry.name.len(),
                });
            }
            classify(dataset, hero, &query).map(|(kind, length)| Match { hero, kind, length })
        })
        .collect();

    // Match quality, then the shorter name, then roster order for determinism -
    // the same keystrokes must always produce the same first result.
    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.length.cmp(&b.length))
            .then(a.hero.cmp(&b.hero))
    });
    out.truncate(limit);
    out
}

/// The single hero a query resolves to, if any.
pub fn resolve(dataset: &Dataset, query: &str, scope: &Scope) -> Option<HeroId> {
    search(dataset, query, scope, 1).first().map(|m| m.hero)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapMatch {
    pub map: MapId,
    pub kind: MatchKind,
    pub length: usize,
}

fn classify_map(dataset: &Dataset, map: MapId, query: &str) -> Option<(MatchKind, usize)> {
    let entry = dataset.map(map).ok()?;
    let name = fold(&entry.name);
    let key = fold(&entry.key);

    let mut best: Option<(MatchKind, usize)> = None;
    let mut consider = |kind: MatchKind, length: usize| {
        if best.is_none_or(|(current, _)| kind < current) {
            best = Some((kind, length));
        }
    };

    for alias in &entry.aliases {
        let alias = fold(alias);
        if alias == query {
            consider(MatchKind::AliasExact, alias.len());
        } else if alias.starts_with(query) {
            consider(MatchKind::AliasPrefix, alias.len());
        }
    }
    if key == query {
        consider(MatchKind::KeyExact, key.len());
    }
    if name.starts_with(query) || key.starts_with(query) {
        consider(MatchKind::NamePrefix, name.len());
    }
    // `Watchpoint: Gibraltar` should be reachable by typing `gib`.
    if entry
        .name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|word| fold(word).starts_with(query))
    {
        consider(MatchKind::NameWordPrefix, name.len());
    }
    if is_subsequence(query, &name) || is_subsequence(query, &key) {
        consider(MatchKind::Subsequence, name.len());
    }

    best
}

/// Maps matching `query`, best first.
///
/// The map is entered once per match rather than once per pick, so this matters
/// less than hero search - but it is on the critical path at the start of a
/// match, when you have the least time.
pub fn search_maps(dataset: &Dataset, query: &str, limit: usize) -> Vec<MapMatch> {
    let query = fold(query);

    let mut out: Vec<MapMatch> = (0..dataset.maps().len())
        .map(|i| MapId(i as u16))
        .filter_map(|map| {
            if query.is_empty() {
                return dataset.map(map).ok().map(|entry| MapMatch {
                    map,
                    kind: MatchKind::Subsequence,
                    length: entry.name.len(),
                });
            }
            classify_map(dataset, map, &query).map(|(kind, length)| MapMatch { map, kind, length })
        })
        .collect();

    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then(a.length.cmp(&b.length))
            .then(a.map.cmp(&b.map))
    });
    out.truncate(limit);
    out
}

pub fn resolve_map(dataset: &Dataset, query: &str) -> Option<MapId> {
    search_maps(dataset, query, 1).first().map(|m| m.map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_strips_what_nobody_types() {
        assert_eq!(fold("Lúcio"), "lucio");
        assert_eq!(fold("D.Va"), "dva");
        assert_eq!(fold("Soldier: 76"), "soldier76");
        assert_eq!(fold("Torbjörn"), "torbjorn");
        assert_eq!(fold("Wrecking Ball"), "wreckingball");
    }

    #[test]
    fn subsequence_matching_is_ordered() {
        assert!(is_subsequence("wrb", "wreckingball"));
        assert!(is_subsequence("", "anything"));
        assert!(!is_subsequence("brw", "wreckingball"));
        assert!(!is_subsequence("xyz", "wreckingball"));
    }

    #[test]
    fn match_kinds_rank_aliases_above_fuzzy() {
        assert!(MatchKind::AliasExact < MatchKind::AliasPrefix);
        assert!(MatchKind::AliasPrefix < MatchKind::NamePrefix);
        assert!(MatchKind::NamePrefix < MatchKind::Subsequence);
    }
}
