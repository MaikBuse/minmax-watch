//! Blizzard's own published hero rates, sliced by rank.
//!
//! The one source in this pipeline that is first-party telemetry rather than a
//! community tracker, and the only one that slices by rank at a URL.
//!
//! It is not discoverable from the page. `overwatch.blizzard.com/en-us/rates/`
//! renders its table client-side and the query string on it is inert — a plain
//! fetch of `?tier=Bronze` returns the same markup with `data-selected` still
//! saying `All`. The JSON behind it is what the page's own script asks for, and
//! that is what this reads.
//!
//! It backs `strength_by_rank.toml` and nothing else: it publishes one row per
//! hero and never a pair, so it says nothing about matchups or duos. It also
//! publishes no sample sizes at all — `pickrate` is the only volume figure —
//! which is why counterwatch's per-division `matches` column is worth reading
//! beside it. See [`crate::stats`] for how the two are weighed.

use std::collections::HashMap;

use anyhow::{Context, Result};
use overwatch_core::Rank;
use serde::Deserialize;

use crate::cache::Fetcher;

const BASE: &str = "https://overwatch.blizzard.com/en-us/rates/data/";

/// PC rather than Console.
///
/// Measured mean |Δ win rate| between the two inputs is 1.08 points across the
/// roster — real, but smaller than the 1.3–2.0 the two win-rate sources already
/// disagree by, and this app is a browser tab beside the game.
const INPUT: &str = "PC";

/// One region is enough.
///
/// Europe against Americas measures a mean |Δ| of 0.43 points and a maximum of
/// 1.7 — the smallest of every axis here, and well inside the noise the two
/// sources already carry. Three regions would triple the requests to average
/// away something that is not there.
const REGION: &str = "Americas";

/// Unfiltered by map.
///
/// Filtering measures a mean |Δ| of 1.24, but which maps suit a hero is already
/// `map_affinity.toml`'s job, and folding it into base strength would count it
/// twice.
const MAP: &str = "all-maps";

/// Every role in one response, so the eight rungs cost eight requests rather
/// than twenty-four. The role is on the roster already.
const ROLE: &str = "All";

/// 2 is competitive role queue; 0 is quick play role queue, which measures a
/// mean |Δ| of 1.21 against it.
///
/// There is no open-queue or 6v6 value — the endpoint does not offer one — so
/// this source cannot answer the [`overwatch_core::Queue::Open`] half of a
/// format. That is a limit of the published data rather than an omission here,
/// and the scorer applies these numbers to every format anyway, on the same
/// reasoning `EnemyRoleWeights` is applied to 6v6 despite being measured on 5v5.
const QUEUE: &str = "2";

/// A `tier=` value.
///
/// [`Tier::Baseline`] is the endpoint's `All`, which is not a rung of the ladder
/// and is deliberately not modelled as one — see [`Rank`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Baseline,
    Rung(Rank),
}

impl Tier {
    /// Blizzard's spelling: capitalised, and `Grandmaster` without the `+`
    /// counterwatch displays.
    fn query(self) -> &'static str {
        match self {
            Tier::Baseline => "All",
            Tier::Rung(Rank::Bronze) => "Bronze",
            Tier::Rung(Rank::Silver) => "Silver",
            Tier::Rung(Rank::Gold) => "Gold",
            Tier::Rung(Rank::Platinum) => "Platinum",
            Tier::Rung(Rank::Emerald) => "Emerald",
            Tier::Rung(Rank::Diamond) => "Diamond",
            Tier::Rung(Rank::Master) => "Master",
            Tier::Rung(Rank::Grandmaster) => "Grandmaster",
            // `Rank::All` never reaches here: the rungs come from
            // `Rank::DIVISIONS`, and the aggregate is `Tier::Baseline`.
            Tier::Rung(Rank::All) => "All",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Tier::Baseline => Rank::All.as_str(),
            Tier::Rung(rank) => rank.as_str(),
        }
    }
}

fn url(tier: Tier) -> String {
    format!(
        "{BASE}?input={INPUT}&map={MAP}&region={REGION}&role={ROLE}&rq={QUEUE}&tier={}",
        tier.query()
    )
}

#[derive(Debug, Deserialize)]
struct RatesResponse {
    rates: RatesBody,
}

#[derive(Debug, Deserialize)]
struct RatesBody {
    rates: Vec<RateRow>,
    /// The server's echo of the query it actually served, and the reason it is
    /// modelled at all — see [`scrape`].
    selected: Selected,
}

#[derive(Debug, Deserialize)]
struct Selected {
    tier: String,
}

#[derive(Debug, Deserialize)]
struct RateRow {
    /// Blizzard's hero id, which is our hero key. OverFast mirrors these ids and
    /// `heroes.toml` is written from OverFast, so the two agree by construction
    /// rather than by luck — hyphens included (`soldier-76`, `wrecking-ball`).
    id: String,
    cells: Cells,
}

#[derive(Debug, Deserialize)]
struct Cells {
    winrate: f32,
    /// Not written to `data/`. Kept because it is the only volume figure this
    /// source publishes, and a rung resting on a 1% pick rate is worth naming in
    /// the run report.
    pickrate: f32,
    // `banrate` is deliberately not modelled. It swings enormously by rung —
    // Sombra runs 73.2% at Bronze and 0.1% at Grandmaster — but it measures who
    // gets banned, not who is strong, and the ban list refuses to put two
    // arguments into one number. See `score::ban_by_strength`.
}

/// Every rung's win rates, plus the baseline they are measured against.
#[derive(Debug, Clone, Default)]
pub struct BlizzardRates {
    /// `tier=All`, keyed by hero.
    pub baseline: HashMap<String, f32>,
    /// One map per rung, keyed by hero.
    pub by_rank: HashMap<Rank, HashMap<String, f32>>,
    /// Pick rate at each rung, for the run report only.
    pub pick_rate: HashMap<(Rank, String), f32>,
}

impl BlizzardRates {
    pub fn is_empty(&self) -> bool {
        self.baseline.is_empty() || self.by_rank.is_empty()
    }
}

async fn fetch_tier(fetcher: &mut Fetcher, tier: Tier) -> Result<Vec<RateRow>> {
    // The tier has to be in the cache key. Slugs are flat filenames with no room
    // for a query variant, so a slug without it caches the first response and
    // serves it for all nine requests — producing a well-formed file with eight
    // identical columns that loads, scores and passes every count-based test.
    let cache_slug = format!("blizzard-rates-{}.json", tier.slug());
    let body = fetcher.get(&url(tier), &cache_slug).await?;
    let payload: RatesResponse = serde_json::from_str(&body)
        .with_context(|| format!("parsing the Blizzard rates response for {:?}", tier.query()))?;

    // An unknown, renamed or retired tier is *not* a transport error here: the
    // endpoint answers `tier=Bogus` with HTTP 200 and the full All-Ranks table.
    // This echo is the only thing that says so, and without it "Blizzard renamed
    // Emerald" is eight identical columns rather than a run that stops.
    anyhow::ensure!(
        payload.rates.selected.tier == tier.query(),
        "asked Blizzard for tier {:?} and it answered with {:?} — the tier vocabulary has changed",
        tier.query(),
        payload.rates.selected.tier,
    );
    anyhow::ensure!(
        !payload.rates.rates.is_empty(),
        "Blizzard returned an empty roster for tier {:?}",
        tier.query(),
    );

    Ok(payload.rates.rates)
}

/// Fetches the baseline and all eight rungs.
///
/// The baseline goes first on purpose: every rung is only ever used as a *shift*
/// away from it, so a run that cannot read the baseline has nothing to offer and
/// should say so before spending eight more requests.
pub async fn scrape(fetcher: &mut Fetcher) -> Result<BlizzardRates> {
    let mut out = BlizzardRates::default();

    for row in fetch_tier(fetcher, Tier::Baseline).await? {
        out.baseline.insert(row.id, row.cells.winrate);
    }

    for rank in Rank::DIVISIONS {
        let rows = fetch_tier(fetcher, Tier::Rung(rank)).await?;
        let mut tier = HashMap::with_capacity(rows.len());
        for row in rows {
            out.pick_rate
                .insert((rank, row.id.clone()), row.cells.pickrate);
            tier.insert(row.id, row.cells.winrate);
        }
        out.by_rank.insert(rank, tier);
    }

    Ok(out)
}

/// Names any hero the two sides disagree about, in both directions.
///
/// Verified identical at the time of writing, so this exists to notice the day
/// it stops. Reported rather than fatal, in the shape [`crate::stats`] already
/// uses for maps it does not know: a roster that has moved on should cost the
/// rank columns for one hero, not the whole run.
pub fn report_key_drift(rates: &BlizzardRates, hero_keys: &[String]) {
    let ours: std::collections::HashSet<&str> = hero_keys.iter().map(String::as_str).collect();

    let mut theirs_only: Vec<&str> = rates
        .baseline
        .keys()
        .map(String::as_str)
        .filter(|key| !ours.contains(key))
        .collect();
    let mut ours_only: Vec<&str> = hero_keys
        .iter()
        .map(String::as_str)
        .filter(|key| !rates.baseline.contains_key(*key))
        .collect();

    theirs_only.sort_unstable();
    ours_only.sort_unstable();

    for key in theirs_only {
        eprintln!("  note: blizzard rates {key:?}, which is not in heroes.toml");
    }
    if !ours_only.is_empty() {
        eprintln!(
            "  warn: blizzard has no rank data for: {}",
            ours_only.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rung_asks_for_a_cache_key_of_its_own() {
        let mut slugs: Vec<&str> = std::iter::once(Tier::Baseline)
            .chain(Rank::DIVISIONS.into_iter().map(Tier::Rung))
            .map(Tier::slug)
            .collect();
        let count = slugs.len();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(
            slugs.len(),
            count,
            "two tiers sharing a cache key would serve one response for both"
        );
        assert_eq!(count, 9, "the baseline plus eight rungs");
    }

    /// The guard in `fetch_tier` compares against these strings, so a typo here
    /// is a run that always aborts or — worse, if it matched the wrong rung — a
    /// column filled from a different population.
    #[test]
    fn the_tier_names_are_the_ones_blizzard_publishes() {
        assert_eq!(Tier::Baseline.query(), "All");
        assert_eq!(Tier::Rung(Rank::Grandmaster).query(), "Grandmaster");
        assert_eq!(Tier::Rung(Rank::Platinum).query(), "Platinum");

        for rank in Rank::DIVISIONS {
            let query = Tier::Rung(rank).query();
            assert_ne!(query, "All", "{rank:?} must not fall back to the baseline");
            assert_eq!(
                Rank::parse(query),
                Ok(rank),
                "{query:?} must read back as the rung it was asked for"
            );
        }
    }

    #[test]
    fn the_url_carries_every_axis_the_endpoint_filters_on() {
        let built = url(Tier::Rung(Rank::Diamond));
        for expected in [
            "input=PC",
            "map=all-maps",
            "region=Americas",
            "role=All",
            "rq=2",
            "tier=Diamond",
        ] {
            assert!(built.contains(expected), "{built} is missing {expected}");
        }
    }

    #[test]
    fn a_tier_the_server_did_not_serve_is_rejected() {
        // The shape the endpoint really returns for an unknown tier: HTTP 200,
        // a full table, and `selected.tier` quietly reading "All".
        let body = r#"{"rates":{"rates":[{"id":"ana","cells":{"winrate":48.5,"pickrate":25.1}}],
                       "selected":{"tier":"All"}},"columns":[]}"#;
        let payload: RatesResponse = serde_json::from_str(body).expect("parses");
        assert_eq!(payload.rates.selected.tier, "All");
        assert_ne!(
            payload.rates.selected.tier,
            Tier::Rung(Rank::Bronze).query(),
            "this mismatch is what `fetch_tier` refuses to accept"
        );
    }

    #[test]
    fn the_unmodelled_columns_are_ignored_rather_than_fatal() {
        let body = r#"{"rates":{"rates":[{"id":"ana","cells":{"name":"Ana","winrate":48.5,
                       "pickrate":25.1,"banrate":12.7},"hero":{"role":"SUPPORT"}}],
                       "selected":{"tier":"Bronze","region":"Americas"}},"columns":[],"extrema":{}}"#;
        let payload: RatesResponse = serde_json::from_str(body).expect("parses");
        assert_eq!(payload.rates.rates[0].id, "ana");
        assert_eq!(payload.rates.rates[0].cells.winrate, 48.5);
    }
}
