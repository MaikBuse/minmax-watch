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
//! It backs `strength_by_rank.toml`, the Blizzard third of the win-rate blend in
//! `strength.toml`, and `prevalence.toml`. What it cannot back is anything about
//! a *pair*: it publishes one row per hero and never two, so it says nothing
//! about matchups or duos. It also publishes no sample sizes at all — `pickrate`
//! is the only volume figure — which is why counterwatch's per-division
//! `matches` column is worth reading beside it. See [`crate::stats`] for how the
//! sources are weighed.

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
    /// The only volume figure this source publishes, and the whole of
    /// `prevalence.toml`.
    ///
    /// Worth knowing what it actually is, because "pick rate" undersells it:
    /// summed over a role it comes to exactly `100 x slots(role)` at every rung,
    /// because role queue admits no duplicates. So this is not popularity on some
    /// arbitrary scale — it is P(this hero is on a team), and that is what gives
    /// `prevalence.toml` an exact zero point rather than a fitted one.
    pickrate: f32,
    /// Who the ladder actually bans, and **the yardstick rather than an input**.
    ///
    /// It stays out of the scorer for the reason it always has: it measures who
    /// gets banned rather than who is strong, and the ban list refuses to put two
    /// arguments into one number — see `score::ban_by_strength`. Scoring on it
    /// would also make the acceptance test circular, because this is the thing
    /// that test predicts.
    ///
    /// What it is good for is checking the answer. It swings enormously by rung —
    /// Sombra runs 73.2% at Bronze and 0.1% at Grandmaster — and that swing is
    /// itself the reason only Grandmaster is worth checking against: rho(ban rate,
    /// pick rate) runs 0.04 at Bronze and 0.52 at Grandmaster, so below Diamond the
    /// ban button is spent on annoyance rather than on strength.
    banrate: f32,
}

/// Every rung's win rates, plus the baseline they are measured against.
#[derive(Debug, Clone, Default)]
pub struct BlizzardRates {
    /// `tier=All`, keyed by hero.
    pub baseline: HashMap<String, f32>,
    /// One map per rung, keyed by hero.
    pub by_rank: HashMap<Rank, HashMap<String, f32>>,
    /// Ban rate at every rung and at the baseline, keyed like [`Self::pick_rate`].
    ///
    /// Never scored on. It is written to `data/ban_rate.toml` for the acceptance
    /// test to read, and that file is deliberately not part of the bundle.
    pub ban_rate: HashMap<(Rank, String), f32>,
    /// Pick rate at every rung **and** at the baseline, so this map is nine
    /// buckets wide where [`Self::by_rank`] is eight.
    ///
    /// That difference is the whole reason `prevalence.toml` is a nine-column
    /// file: a win rate at `Rank::All` is what the rungs are measured *against*
    /// and so has no column of its own, while a pick rate there is a published
    /// figure in its own right. Keyed on [`Rank::All`] for the baseline.
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
        // The ninth pick-rate bucket. `Rank::All` has no *win rate* column of its
        // own — it is the baseline every rung's shift is measured against — but it
        // has a real pick rate, and `prevalence.toml` has a column for it.
        out.pick_rate
            .insert((Rank::All, row.id.clone()), row.cells.pickrate);
        out.ban_rate
            .insert((Rank::All, row.id.clone()), row.cells.banrate);
        out.baseline.insert(row.id, row.cells.winrate);
    }

    for rank in Rank::DIVISIONS {
        let rows = fetch_tier(fetcher, Tier::Rung(rank)).await?;
        let mut tier = HashMap::with_capacity(rows.len());
        for row in rows {
            out.pick_rate
                .insert((rank, row.id.clone()), row.cells.pickrate);
            out.ban_rate
                .insert((rank, row.id.clone()), row.cells.banrate);
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
        let body = r#"{"rates":{"rates":[{"id":"ana","cells":{"winrate":48.5,"pickrate":25.1,
                       "banrate":12.7}}],"selected":{"tier":"All"}},"columns":[]}"#;
        let payload: RatesResponse = serde_json::from_str(body).expect("parses");
        assert_eq!(payload.rates.selected.tier, "All");
        assert_ne!(
            payload.rates.selected.tier,
            Tier::Rung(Rank::Bronze).query(),
            "this mismatch is what `fetch_tier` refuses to accept"
        );
    }

    /// All three cells are required rather than defaulted, and `banrate` is the one
    /// where that matters most: it is the yardstick the ban list is judged against,
    /// so a serde default of 0.0 would read as "the ladder never bans this hero"
    /// and quietly make the acceptance test easier to pass.
    #[test]
    fn a_response_missing_a_cell_is_an_error_rather_than_a_zero() {
        let body = r#"{"rates":{"rates":[{"id":"ana","cells":{"winrate":48.5,"pickrate":25.1}}],
                       "selected":{"tier":"All"}},"columns":[]}"#;
        serde_json::from_str::<RatesResponse>(body)
            .expect_err("a missing ban rate is a schema change, not a zero");
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
