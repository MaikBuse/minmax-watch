//! Regenerates the committed dataset in `data/`.
//!
//! Run with `just ingest`. Output is written into the repo and reviewed as a
//! git diff - that review *is* the curation step, so this tool optimises for
//! producing a readable, stable diff over being clever.
//!
//! Nothing here runs at application runtime. The app ships the generated TOML
//! compiled in and never talks to these sites.

mod aliases;
mod art;
mod blend;
mod blizzard;
mod brand;
mod cache;
mod counterpickgg;
mod counterwatch;
mod overfast;
mod overpicker;
mod prose;
mod slugs;
mod stats;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use overwatch_core::{Rank, Role};
use overwatch_data::schema::{
    HeroesFile, MapsFile, MatchupEntry, MatchupsFile, StrengthByRankFile, SynergyEntry,
    SynergyFile, MATCHUPS_NOTE,
};
use time::format_description::well_known::Iso8601;
use time::OffsetDateTime;

use crate::cache::{write_if_changed, Fetcher};

fn workspace_root() -> PathBuf {
    // The ingest crate lives one level below the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Reads the roster the `roster` step wrote, so `counters` can run on its own.
async fn load_roster(data_dir: &Path) -> Result<HeroesFile> {
    let path = data_dir.join("heroes.toml");
    let text = tokio::fs::read_to_string(&path).await.with_context(|| {
        format!(
            "reading {} - run `just ingest-roster` first",
            path.display()
        )
    })?;
    let roster: HeroesFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

    anyhow::ensure!(
        !roster.heroes.is_empty(),
        "{} has no heroes - run `just ingest-roster` first",
        path.display()
    );
    Ok(roster)
}

/// Map keys we know about, used to reject affinity rows for maps we do not
/// draft on (arcade and workshop maps the counter site still rates).
async fn load_map_keys(data_dir: &Path) -> Result<HashSet<String>> {
    let path = data_dir.join("maps.toml");
    let text = tokio::fs::read_to_string(&path).await.with_context(|| {
        format!(
            "reading {} - run `just ingest-roster` first",
            path.display()
        )
    })?;
    let maps: MapsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(maps.maps.into_iter().map(|m| m.key).collect())
}

/// Reads whatever `synergy.toml` already says, so a re-scrape can keep it.
///
/// A missing or unparseable file is not fatal here: the scrape can rebuild the
/// generated half from nothing. It is only the `curated` column that cannot be
/// reconstructed, and losing that silently is the failure worth guarding.
async fn load_synergy(data_dir: &Path) -> Result<SynergyFile> {
    let path = data_dir.join("synergy.toml");
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return Ok(SynergyFile::default());
    };
    toml::from_str(&text).with_context(|| {
        format!(
            "parsing {} - fix it by hand rather than letting the scrape overwrite it",
            path.display()
        )
    })
}

/// Merges a fresh scrape over the existing file, curated rows winning.
///
/// Three things have to survive a re-run, and they are the whole reason this is
/// not just "serialise the scrape":
///
/// - a curated row for a pair the scrape does not list must stay in the file;
/// - a curated row for a pair it *does* list must keep its value and its note,
///   with the fresh reading recorded beside it rather than on top of it;
/// - a pair the scrape used to list and no longer does must lose its `cwatch`
///   reading, because the source no longer says it — but must not lose a
///   curated opinion that was written about it.
fn merge_synergy(
    generated: &str,
    existing: &SynergyFile,
    scraped: &HashMap<(String, String), i8>,
) -> SynergyFile {
    let mut curated: HashMap<(String, String), &SynergyEntry> = HashMap::new();
    for entry in &existing.entries {
        if entry.curated.is_some() {
            curated.insert((entry.hero.clone(), entry.with.clone()), entry);
        }
    }

    let mut keys: HashSet<(String, String)> = scraped.keys().cloned().collect();
    keys.extend(curated.keys().cloned());

    let mut entries: Vec<SynergyEntry> = keys
        .into_iter()
        .map(|(hero, with)| {
            let previous = curated.get(&(hero.clone(), with.clone()));
            let cwatch = scraped.get(&(hero.clone(), with.clone())).copied();
            let curated_value = previous.and_then(|e| e.curated);
            SynergyEntry {
                // Kept in step with `resolved()` so the committed file reads the
                // same whether you go through the loader or your eyes.
                value: curated_value.or(cwatch).unwrap_or(0),
                hero,
                with,
                cwatch,
                curated: curated_value,
                note: previous.map(|e| e.note.clone()).unwrap_or_default(),
                // The site publishes a duo from one side at a time, and it
                // lists plenty of pairs from both. Writing each row as
                // symmetric is what stops "Zarya pairs with Vendetta" being
                // silently one-directional advice.
                symmetric: previous.map(|e| e.symmetric).unwrap_or(true),
            }
        })
        .collect();

    // Sorted for a readable diff, which is the point of the whole tool.
    entries.sort_by(|a, b| a.hero.cmp(&b.hero).then_with(|| a.with.cmp(&b.with)));

    SynergyFile {
        generated: generated.to_owned(),
        entries,
    }
}

/// Reads whatever `matchups.toml` already says, so a re-scrape can keep the
/// curated column.
///
/// A missing or unparseable file is not fatal here, for the same reason it is not
/// in [`load_synergy`]: the scrape can rebuild every other column from nothing.
/// It is only `curated` and `note` that cannot be reconstructed, and losing those
/// silently is the failure worth guarding.
async fn load_matchups(data_dir: &Path) -> Result<MatchupsFile> {
    let path = data_dir.join("matchups.toml");
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return Ok(MatchupsFile::default());
    };
    toml::from_str(&text).with_context(|| {
        format!(
            "parsing {} - fix it by hand rather than letting the scrape overwrite it",
            path.display()
        )
    })
}

/// Merges a fresh blend over the existing file, curated rows winning.
///
/// The counter matrix is otherwise written wholesale, which is what makes this
/// necessary: `curated` and `note` are the only two columns nothing can
/// reconstruct, and *both* write paths rebuild every other column from the
/// sources — `counters` from a live scrape and `reblend` from the committed
/// per-source columns. Either one would drop a hand-written row without this.
///
/// Three things have to survive a re-run:
///
/// - a curated row for a pair no trusted source rated must stay in the file.
///   `blend_values` emits *nothing* for such a direction rather than an even row,
///   so this function is the only thing keeping it;
/// - a curated row for a pair the sources do rate must keep its override and its
///   note, with the fresh blend recorded in `value` beside it rather than on top
///   of it;
/// - a row the sources have stopped rating must otherwise leave with them,
///   exactly as it does today.
///
/// Unlike `merge_synergy`, `value` is *not* kept in step with `resolved()`. It
/// stays the pure blend, so `reblend` can keep deriving every value in the file
/// from the columns beside it and still come out with an empty diff.
fn merge_matchups(
    generated: &str,
    patch: &str,
    existing: &MatchupsFile,
    blended: Vec<MatchupEntry>,
) -> MatchupsFile {
    let mut curated: HashMap<(String, String), &MatchupEntry> = HashMap::new();
    for entry in &existing.matchups {
        if entry.curated.is_some() {
            curated.insert((entry.hero.clone(), entry.vs.clone()), entry);
        }
    }

    let mut matchups = blended;
    let mut blended_keys: HashSet<(String, String)> = HashSet::new();
    for entry in &mut matchups {
        let key = (entry.hero.clone(), entry.vs.clone());
        if let Some(previous) = curated.get(&key) {
            entry.curated = previous.curated;
            entry.note = previous.note.clone();
        }
        blended_keys.insert(key);
    }

    for (key, previous) in &curated {
        if blended_keys.contains(key) {
            continue;
        }
        matchups.push(MatchupEntry {
            hero: key.0.clone(),
            vs: key.1.clone(),
            // The blend of nothing. `resolved()` reads `curated` for this row,
            // and keeping `value` at zero is what stops the file claiming a
            // measurement that does not exist.
            value: 0,
            disagreement: false,
            cpgg: None,
            opick: None,
            cwatch: None,
            reason: String::new(),
            curated: previous.curated,
            note: previous.note.clone(),
        });
    }

    // Sorted for a readable diff, which is the point of the whole tool. The
    // iteration order of `curated` above is arbitrary; this is what makes the
    // output deterministic anyway.
    matchups.sort_by(|a, b| (&a.hero, &a.vs).cmp(&(&b.hero, &b.vs)));

    MatchupsFile {
        generated: generated.to_owned(),
        patch: patch.to_owned(),
        // Rewritten every run rather than carried over, so the rule in the file
        // is always the rule this build enforces.
        note: MATCHUPS_NOTE.to_owned(),
        matchups,
    }
}

/// Says how much of the matrix is hand-written, and warns where a curated row is
/// missing its mirror.
///
/// One-sidedness is the failure mode worth reporting rather than asserting: the
/// scorer folds a pair as `(forward - reverse) / 2` whenever both directions are
/// rated, so a curated `+40` opposite a scraped `0` lands at `+20`. A warning
/// rather than an error because the file is legitimately one-sided while a batch
/// is being written; `a_curated_matchup_is_curated_from_both_sides` is what
/// refuses to let it ship that way.
fn report_curated_matchups(file: &MatchupsFile) {
    let curated: Vec<&MatchupEntry> = file
        .matchups
        .iter()
        .filter(|entry| entry.curated.is_some())
        .collect();
    if curated.is_empty() {
        return;
    }

    let rated: HashSet<(&str, &str)> = file
        .matchups
        .iter()
        .map(|entry| (entry.hero.as_str(), entry.vs.as_str()))
        .collect();
    let overridden: HashSet<(&str, &str)> = curated
        .iter()
        .map(|entry| (entry.hero.as_str(), entry.vs.as_str()))
        .collect();

    eprintln!("  {} curated row(s)", curated.len());
    for entry in &curated {
        let mirror = (entry.vs.as_str(), entry.hero.as_str());
        if overridden.contains(&mirror) || !rated.contains(&mirror) {
            continue;
        }
        eprintln!(
            "  warn: {} vs {} is curated but its mirror is rated and is not - \
             the pair will score at half the curated magnitude",
            entry.hero, entry.vs
        );
    }
}

/// There is no published patch identifier we can read, so the ingest date
/// stands in. It exists so the UI can say how old the data is.
fn patch_label(generated: &str) -> String {
    format!("ingested {generated}")
}

fn today() -> String {
    OffsetDateTime::now_utc()
        .format(&Iso8601::DATE)
        .unwrap_or_else(|_| "unknown".to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    /// Roster and map list from the OverFast API.
    Roster,
    /// Counter matrix from the three community sites.
    Counters,
    /// Duo synergies from counterwatch, merged over the curated overrides.
    Synergy,
    /// Hero win rates, map affinity and the rank slices, without re-scraping the
    /// whole matrix.
    ///
    /// Split out from `Counters` because the two go stale at different rates: a
    /// balance patch moves every win rate the same week and barely touches the
    /// matchup opinions, and re-reading 53 win rates should not mean re-fetching
    /// 160 pages and reviewing a diff of the entire matrix to find them.
    ///
    /// The rank slices belong here by that same test rather than in a step of
    /// their own: they *are* win rates, on the same clock. Half of them come off
    /// the counterwatch stats pages this step already fetches, and the other half
    /// costs nine requests to Blizzard.
    Strength,
    /// Hero portraits, map thumbnails and rank badges into the web assets.
    Art,
    /// Everything.
    All,
    /// Rasterise the brand SVGs into favicons, PWA icons and the OG card.
    ///
    /// Not part of `All`: it touches no upstream source, needs no network, and
    /// only has anything to do when the artwork itself changes.
    Brand,
    /// Re-run the blend over the columns already in `matchups.toml`.
    ///
    /// Every committed `value` is reproducible from the `cpgg` and `cwatch`
    /// columns of the pair's two rows, so a change to the blend can be reviewed as
    /// a diff of exactly the rows the blend moved - and of the rows it newly
    /// rates, because a lone `cwatch` reading is read across the mirror and can
    /// rate a direction that was reaching the matrix as nothing at all.
    /// `Counters` would answer the same question by re-fetching 107 pages, and the
    /// diff would then carry every opinion the sites have changed since the last
    /// run mixed in with it.
    ///
    /// Not part of `All`, for the same reason as `Brand`: no network, and nothing
    /// to do unless the blend itself has changed. Idempotent, because it derives
    /// `value` from the source columns rather than from `value`.
    ///
    /// It also runs [`prose::clean_file`], which makes it the no-network way to
    /// normalise the rationale text: the sentences come off the committed rows and
    /// go back with the site's template residue removed, while every `value` is
    /// still derived from the columns beside it. So a change to the cleaner reviews
    /// as a diff of nothing but `reason` lines.
    Reblend,
}

struct Args {
    command: Command,
    refresh: bool,
}

fn parse_args() -> Result<Args> {
    let mut command = Command::All;
    let mut refresh = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "roster" => command = Command::Roster,
            "counters" => command = Command::Counters,
            "synergy" => command = Command::Synergy,
            "strength" => command = Command::Strength,
            "art" => command = Command::Art,
            "all" => command = Command::All,
            "brand" => command = Command::Brand,
            "reblend" => command = Command::Reblend,
            "--refresh" | "-r" => refresh = true,
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => anyhow::bail!("unrecognised argument {other:?}\n\n{USAGE}"),
        }
    }

    Ok(Args { command, refresh })
}

const USAGE: &str = "\
usage: overwatch-ingest [roster|counters|synergy|strength|art|all|brand|reblend] [--refresh]

  roster      regenerate heroes.toml and maps.toml from the OverFast API
  counters    regenerate matchups.toml
  synergy     regenerate the scraped half of synergy.toml, keeping curated rows
  strength    regenerate strength.toml, map_affinity.toml and strength_by_rank.toml
  art         redownload hero portraits, map thumbnails and rank badges
  all         all four (default)
  brand       rasterise the brand SVGs into favicons, PWA icons and og.png
              (local only - no network, and not included in `all`)
  reblend     re-run the blend over the columns already in matchups.toml
              (local only - no network, and not included in `all`)

  --refresh   ignore the cache in data/sources and re-fetch everything";

fn print_usage() {
    println!("{USAGE}");
}

/// Says enough about the rank slices to decide whether to trust the diff.
///
/// The coverage count and the biggest movers, because the failure this file is
/// most exposed to is a plausible one: every way of getting it wrong — a cache
/// key without its tier, a smoothing bug that flattens everything — produces a
/// file that parses and loads. A run that reports 424 cells and no movers is
/// telling you it fetched the same page nine times.
fn report_rank_slices(by_rank: &StrengthByRankFile) {
    let cells = by_rank.entries.len() * Rank::DIVISIONS.len();
    let present: usize = by_rank
        .entries
        .iter()
        .map(|e| {
            Rank::DIVISIONS
                .iter()
                .filter(|rank| e.value_for(**rank).is_some())
                .count()
        })
        .sum();

    let mut movers: Vec<(i16, &str)> = by_rank
        .entries
        .iter()
        .filter_map(|e| {
            let low = e.value_for(Rank::Bronze)?;
            let high = e.value_for(Rank::Grandmaster)?;
            Some((i16::from(high) - i16::from(low), e.hero.as_str()))
        })
        .collect();
    movers.sort_by_key(|(span, _)| -span.abs());

    let biggest: Vec<String> = movers
        .iter()
        .take(3)
        .map(|(span, hero)| format!("{hero} {span:+}"))
        .collect();

    eprintln!(
        "  rank slices: {} heroes x {} rungs ({present}/{cells} cells) | \
         biggest bronze->gm movers: {}",
        by_rank.entries.len(),
        Rank::DIVISIONS.len(),
        if biggest.is_empty() {
            "none".to_owned()
        } else {
            biggest.join(", ")
        },
    );
}

/// Warns when a role's pick rates stop summing to `100 x slots(role)`.
///
/// The guard the whole prevalence scale rests on. Summed over a role the column
/// must come to exactly that, because role queue admits no duplicates and the
/// figure is P(hero is on a team) — which is what gives `prevalence.toml` a zero
/// point with no data in it. If Blizzard ever redefines the column into something
/// else, every value shifts by a constant per role and nothing else here notices:
/// the file still parses, still loads, and still passes every count-based test.
/// Same class of failure as `tier=Bogus` being answered with HTTP 200.
///
/// Observed deviation has never exceeded 0.2 across the nine responses, so the
/// tolerance below is generous by an order of magnitude and a warning means the
/// meaning of the column moved rather than that rounding drifted.
fn report_pick_rate_shape(rates: &blizzard::BlizzardRates, roles: &HashMap<String, Role>) {
    const TOLERANCE: f32 = 1.0;

    for rank in Rank::CHOICES {
        for role in Role::ALL {
            let slots = match role {
                Role::Tank => 1.0,
                Role::Damage | Role::Support => 2.0,
            };
            let sum: f32 = roles
                .iter()
                .filter(|(_, each)| **each == role)
                .filter_map(|(hero, _)| rates.pick_rate.get(&(rank, hero.clone())))
                .sum();
            if sum <= 0.0 {
                continue;
            }
            let expected = 100.0 * slots;
            if (sum - expected).abs() > TOLERANCE {
                eprintln!(
                    "  warn: {} pick rates at {} sum to {sum:.1}, not {expected:.0} - \
                     the column no longer means P(hero is on a team)",
                    role.as_str(),
                    rank.as_str(),
                );
            }
        }
    }
}

/// Cross-checks counterpickgg's own pick-rate column against Blizzard's.
///
/// In the shape [`blizzard::report_key_drift`] uses: the two agree at the time of
/// writing, so this exists to notice the day they stop. counterpickgg is not an
/// input and cannot be — it publishes no rank axis, so it could only fill one
/// column of nine on a different instrument from the other eight, which is the
/// error `rank_shift`'s doc forbids; and it rounds to an integer, so 1% and 2%
/// collapse into one cell at exactly the tail this feature exists to demote.
///
/// It is worth reading anyway, because it is a second measurement of the same
/// quantity: its column sums to 502 against Blizzard's 500, and it ranks the
/// roster nearly the same way. This is also the only reader
/// `HeroStats::pick_rate` has ever had.
fn report_pick_rate_agreement(rates: &blizzard::BlizzardRates, stats: &[counterpickgg::HeroStats]) {
    /// Points of disagreement worth naming. counterpickgg's rounding alone can
    /// account for a point or two.
    const NOISY: f32 = 10.0;

    let mut worst: Vec<(f32, &str, f32, f32)> = stats
        .iter()
        .filter_map(|hero| {
            let theirs = rates
                .pick_rate
                .get(&(Rank::All, hero.hero.clone()))
                .copied()?;
            let delta = (hero.pick_rate - theirs).abs();
            (delta >= NOISY).then_some((delta, hero.hero.as_str(), hero.pick_rate, theirs))
        })
        .collect();
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));

    let total: f32 = stats.iter().map(|hero| hero.pick_rate).sum();
    eprintln!("  pick rates: counterpickgg's column sums to {total:.1}, blizzard's to 500");
    for (_, hero, theirs, blizzard) in worst.iter().take(3) {
        eprintln!(
            "    note: {hero} reads {theirs:.1} at counterpickgg and {blizzard:.1} at blizzard"
        );
    }
}

/// Says enough about the prevalence columns to decide whether to trust the diff.
///
/// The same reasoning as [`report_rank_slices`], against the same failure: every
/// way of getting a nine-column file wrong produces one that parses and loads.
/// Coverage alone cannot tell nine readings from one reading written nine times,
/// so this reports the spread as well — and the clamped count, because the band is
/// only honest while the rail stays an exception.
fn report_prevalence(file: &overwatch_data::schema::PrevalenceFile) {
    let cells = file.entries.len() * Rank::CHOICES.len();
    let present: usize = file
        .entries
        .iter()
        .map(|entry| {
            Rank::CHOICES
                .iter()
                .filter(|rank| entry.value_for(**rank).is_some())
                .count()
        })
        .sum();
    let clamped: usize = file
        .entries
        .iter()
        .flat_map(|entry| Rank::CHOICES.map(|rank| entry.value_for(rank)))
        .filter(|value| value.is_some_and(|value| value.abs() == 100))
        .count();

    let mut extremes: Vec<(i8, &str)> = file
        .entries
        .iter()
        .filter_map(|entry| entry.all.map(|value| (value, entry.hero.as_str())))
        .collect();
    extremes.sort_by_key(|(value, _)| -i16::from(*value));

    let named = |slice: &[(i8, &str)]| {
        slice
            .iter()
            .map(|(value, hero)| format!("{hero} {value:+}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    eprintln!(
        "  prevalence: {} heroes x {} columns ({present}/{cells} cells, {clamped} clamped)",
        file.entries.len(),
        Rank::CHOICES.len(),
    );
    if extremes.len() >= 6 {
        eprintln!(
            "    most picked for their role: {} | least: {}",
            named(&extremes[..3]),
            named(&extremes[extremes.len() - 3..]),
        );
    }
}

/// Says enough about the published counter ratings to decide whether the parse
/// read the right row.
///
/// The distribution is the tell. Every way of getting this wrong lands on a
/// document that still parses: reading the duo rows instead gives percentages
/// near 50, reading the ban list gives the hardest-matchup ratings twice and no
/// easy ones, and taking the direction from the wrong cue leaves the count intact
/// while inverting half the signs. A run reporting 530 rows, a median near 7 and a
/// even split of favourable and unfavourable is telling you it read the matchup
/// table.
///
/// `swing` and `duels` are printed and never committed. They are here because
/// they are the fields that prove the row was the right one — a matchup row is
/// the only thing on the page carrying a duel count — and `swing` deserves a
/// slice of its own before it becomes a column.
fn report_matchup_ratings(ratings: &HashMap<(String, String), counterwatch::MatchupRating>) {
    if ratings.is_empty() {
        eprintln!("  warn: no published counter ratings were read at all");
        return;
    }

    let mut magnitudes: Vec<f32> = ratings.values().map(|r| r.rating.abs()).collect();
    magnitudes.sort_by(f32::total_cmp);
    let median = |sorted: &[f32]| sorted[sorted.len() / 2];

    let favourable = ratings.values().filter(|r| r.rating > 0.0).count();
    let clamped = ratings.values().filter(|r| r.value.abs() == 100).count();

    let mut swings: Vec<u8> = ratings.values().filter_map(|r| r.swing).collect();
    swings.sort_unstable();
    let mut duels: Vec<u32> = ratings.values().map(|r| r.duels).collect();
    duels.sort_unstable();

    eprintln!(
        "    {} published ratings | {favourable} favourable, {} against | {clamped} clamped at the ceiling",
        ratings.len(),
        ratings.len() - favourable,
    );
    eprintln!(
        "    |rating| median {:.1}, max {:.1} | swing on {}/{} (median {}%) | duels median {}, min {}",
        median(&magnitudes),
        magnitudes.last().copied().unwrap_or(0.0),
        swings.len(),
        ratings.len(),
        swings.get(swings.len() / 2).copied().unwrap_or(0),
        duels.get(duels.len() / 2).copied().unwrap_or(0),
        duels.first().copied().unwrap_or(0),
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    let root = workspace_root();
    let data_dir = root.join("data");
    let cache_dir = data_dir.join("sources");

    // Handled before the fetcher exists: this step never goes near the network,
    // and building one would create the source cache directory for nothing.
    if args.command == Command::Brand {
        let assets = root.join("overwatch-web").join("assets");
        let changed = brand::generate(&assets)?;
        match changed.len() {
            0 => eprintln!("brand: already up to date"),
            n => eprintln!("brand: wrote {n} file(s): {}", changed.join(", ")),
        }
        return Ok(());
    }

    // Handled before the fetcher exists, for the same reason `brand` is: this
    // rereads a file the repo already holds and never goes near the network.
    if args.command == Command::Reblend {
        let roster = load_roster(&data_dir).await?;
        let hero_keys: Vec<String> = roster.heroes.iter().map(|h| h.key.clone()).collect();
        // Key to display name, for the prose pass below: it puts a hero's own
        // spelling back and resolves a dangling pronoun to whichever of the pair
        // the sentence does not already name.
        let names: HashMap<String, String> = roster
            .heroes
            .iter()
            .map(|h| (h.key.clone(), h.name.clone()))
            .collect();

        let path = data_dir.join("matchups.toml");
        let text = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;
        // Fatal where `load_synergy` is forgiving: there is no scrape here to
        // rebuild the file from, so an unparseable one is the whole input gone.
        let existing: MatchupsFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

        let mut cpgg = blend::SourceMap::new();
        let mut opick = blend::SourceMap::new();
        let mut cwatch = blend::SourceMap::new();
        let mut reasons: HashMap<(String, String), String> = HashMap::new();
        for entry in &existing.matchups {
            let key = (entry.hero.clone(), entry.vs.clone());
            if let Some(value) = entry.cpgg {
                cpgg.insert(key.clone(), value);
            }
            if let Some(value) = entry.opick {
                opick.insert(key.clone(), value);
            }
            if let Some(value) = entry.cwatch {
                cwatch.insert(key.clone(), value);
            }
            if !entry.reason.is_empty() {
                reasons.insert(key, entry.reason.clone());
            }
        }

        eprintln!(
            "reblend: {} committed rows for {} heroes, no network",
            existing.matchups.len(),
            hero_keys.len()
        );
        let (matchups, report) = blend::blend_values(&hero_keys, &cpgg, &reasons, &opick, &cwatch);
        eprintln!("{}", report.render());

        // `generated` and `patch` carry over untouched. Nothing was fetched, and
        // stamping today's date would claim otherwise.
        //
        // Through `merge_matchups` for the curated column: this path harvests
        // only the source columns above, so serialising the blend directly would
        // drop every hand-written override with no network round trip to blame.
        let mut file = merge_matchups(&existing.generated, &existing.patch, &existing, matchups);
        report_curated_matchups(&file);
        // The same pass the `counters` path runs, which is what makes this the
        // no-network way to normalise the prose: the sentences come off the
        // committed rows above and go back cleaned, with every `value` still
        // derived from the per-source columns beside them.
        eprintln!("{}", prose::clean_file(&mut file, &names)?.render());
        let toml = toml::to_string_pretty(&file).context("serialising matchups.toml")?;
        if write_if_changed(&path, &toml).await? {
            eprintln!("reblend: updated matchups.toml");
        } else {
            eprintln!("reblend: already up to date");
        }
        return Ok(());
    }

    let mut fetcher = Fetcher::new(cache_dir, args.refresh)?;
    let generated = today();
    let mut changed: Vec<&str> = Vec::new();

    if matches!(args.command, Command::Roster | Command::All) {
        eprintln!("roster: fetching heroes and maps from OverFast");

        let heroes = overfast::fetch_heroes(&mut fetcher, &generated).await?;
        let maps = overfast::fetch_maps(&mut fetcher, &generated).await?;

        let tanks = heroes.heroes.iter().filter(|h| h.role == "tank").count();
        let damage = heroes.heroes.iter().filter(|h| h.role == "damage").count();
        let supports = heroes.heroes.iter().filter(|h| h.role == "support").count();
        eprintln!(
            "  roster: {} heroes ({tanks} tank / {damage} damage / {supports} support)",
            heroes.heroes.len()
        );
        eprintln!("  maps:   {} competitive maps", maps.maps.len());

        let heroes_toml = toml::to_string_pretty(&heroes).context("serialising heroes.toml")?;
        let maps_toml = toml::to_string_pretty(&maps).context("serialising maps.toml")?;

        if write_if_changed(&data_dir.join("heroes.toml"), &heroes_toml).await? {
            changed.push("heroes.toml");
        }
        if write_if_changed(&data_dir.join("maps.toml"), &maps_toml).await? {
            changed.push("maps.toml");
        }
    }

    if matches!(args.command, Command::Art | Command::All) {
        // Runs before the counter scrape rather than after it: it is derived
        // from the roster we just wrote, and a failure here should surface in
        // seconds instead of behind three minutes of page fetching.
        eprintln!("art: portraits, map thumbnails and rank badges");

        let hero_keys: HashSet<String> = load_roster(&data_dir)
            .await?
            .heroes
            .into_iter()
            .map(|h| h.key)
            .collect();
        let map_keys = load_map_keys(&data_dir).await?;
        let web_assets = root.join("overwatch-web").join("assets");

        let report = art::build(&mut fetcher, &web_assets, &hero_keys, &map_keys).await?;
        eprintln!(
            "  {} portraits / {} thumbnails / {} rank badges; {} written, {} orphan(s) removed",
            report.heroes, report.maps, report.ranks, report.changed, report.removed
        );
        if !report.missing.is_empty() {
            // Named rather than counted: the draft screen will show these as
            // text only, and knowing which ones makes that look deliberate.
            eprintln!(
                "  note: no artwork published for {}",
                report.missing.join(", ")
            );
        }
        if report.changed > 0 || report.removed > 0 {
            changed.push("overwatch-web/assets");
        }
    }

    if matches!(args.command, Command::Counters | Command::All) {
        let roster = load_roster(&data_dir).await?;
        let hero_keys: Vec<String> = roster.heroes.iter().map(|h| h.key.clone()).collect();
        let names: HashMap<String, String> = roster
            .heroes
            .iter()
            .map(|h| (h.name.clone(), h.key.clone()))
            .collect();
        // The other direction, because the stats pages state which hero a row is
        // favourable for by naming it, and the parser has to recognise the name.
        let subject_names: HashMap<String, String> = roster
            .heroes
            .iter()
            .map(|h| (h.key.clone(), h.name.clone()))
            .collect();

        eprintln!(
            "counters: building the matrix for {} heroes",
            hero_keys.len()
        );

        // Each source is fetched independently and a failure degrades the blend
        // rather than aborting the run: two sources still beat none, and the
        // coverage report below makes the gap obvious.
        eprintln!("  source: counterpickgg ({} pages)", hero_keys.len());
        let cpgg = counterpickgg::scrape(&mut fetcher, &hero_keys)
            .await
            .unwrap_or_else(|err| {
                eprintln!("  warn: counterpickgg unusable: {err:#}");
                Vec::new()
            });

        eprintln!("  source: overpicker (1 page)");
        let opick = overpicker::scrape(&mut fetcher, &names)
            .await
            .unwrap_or_else(|err| {
                eprintln!("  warn: overpicker unusable: {err:#}");
                blend::SourceMap::new()
            });

        // The stats pages the `strength` step already caches, read here because
        // the numbers on them land in matchups.toml and only this step writes
        // that file. Free on a warm cache; 53 requests on a cold one.
        eprintln!(
            "  source: counterwatch stats ({} pages, shared with strength)",
            hero_keys.len()
        );
        let ratings =
            counterwatch::scrape_matchup_ratings(&mut fetcher, &hero_keys, &subject_names)
                .await
                .unwrap_or_else(|err| {
                    // Degrades to the rank synthesis rather than aborting, which is
                    // what the cross-check inside the parser is allowed to rely on.
                    eprintln!("  warn: counterwatch ratings unusable: {err:#}");
                    HashMap::new()
                });
        report_matchup_ratings(&ratings);

        eprintln!("  source: counterwatch ({} pages)", hero_keys.len());
        let cwatch = counterwatch::scrape(&mut fetcher, &hero_keys, &names, &ratings)
            .await
            .unwrap_or_else(|err| {
                eprintln!("  warn: counterwatch unusable: {err:#}");
                blend::SourceMap::new()
            });

        // overpicker is recorded but not blended, so it alone cannot justify
        // overwriting the committed matrix.
        anyhow::ensure!(
            !cpgg.is_empty() || !cwatch.is_empty(),
            "every trusted counter source failed; refusing to overwrite data/matchups.toml with nothing"
        );

        let (matchups, report) = blend::blend(&hero_keys, &cpgg, &opick, &cwatch);
        eprintln!("{}", report.render());

        // Deliberately *after* the guard above rather than folded into it, which
        // is where the synergy step puts its curated count. The two files fail
        // differently: an empty duo scrape costs synergy.toml nothing it cannot
        // rebuild, while every trusted counter source failing would trade 2,472
        // blended rows for a handful of curated ones. Refusing to write is what
        // keeps the curated rows safe here, so they must not license the write.
        let existing = load_matchups(&data_dir).await?;
        let mut file = merge_matchups(&generated, &patch_label(&generated), &existing, matchups);
        report_curated_matchups(&file);
        // After the merge, so it runs over exactly the rows about to be written,
        // and in both write paths, because either one rebuilds `reason` from the
        // sources and a cleaner in only one of them regresses through the other.
        eprintln!("{}", prose::clean_file(&mut file, &subject_names)?.render());
        let toml = toml::to_string_pretty(&file).context("serialising matchups.toml")?;
        if write_if_changed(&data_dir.join("matchups.toml"), &toml).await? {
            changed.push("matchups.toml");
        }
    }

    if matches!(args.command, Command::Strength | Command::All) {
        let roster = load_roster(&data_dir).await?;
        let hero_keys: Vec<String> = roster.heroes.iter().map(|h| h.key.clone()).collect();

        // The index table is one fetch and carries both hero win rates and the
        // only map-performance data reachable without a browser.
        eprintln!("strength: counterpickgg index (1 page)");
        match counterpickgg::scrape_index(&mut fetcher).await {
            Ok(stats) => {
                let known_maps: HashSet<String> = load_map_keys(&data_dir).await?;

                // A second reading of the same quantity. Degrades to the single
                // source rather than aborting: one published win rate is still
                // better than none, and the blend renormalises over whoever
                // answered.
                eprintln!("  source: counterwatch stats ({} pages)", hero_keys.len());
                let cwatch_rates = counterwatch::scrape_win_rates(&mut fetcher, &hero_keys)
                    .await
                    .unwrap_or_else(|err| {
                        eprintln!("  warn: counterwatch win rates unusable: {err:#}");
                        HashMap::new()
                    });

                // The only rank-sliced source with a URL behind it. Degrades the
                // same way counterwatch does above: the rank columns are worth
                // having, but not at the price of the two files that already
                // worked.
                eprintln!("  source: blizzard rates (9 pages: all ranks + 8 rungs)");
                let blizzard = blizzard::scrape(&mut fetcher).await.unwrap_or_else(|err| {
                    eprintln!("  warn: blizzard rates unusable: {err:#}");
                    blizzard::BlizzardRates::default()
                });
                if !blizzard.is_empty() {
                    blizzard::report_key_drift(&blizzard, &hero_keys);
                }

                // Roles are the app's own, off the roster, and not Blizzard's —
                // the response carries one but `heroes.toml` is what everything
                // else in this project means by a hero's role.
                let roles: HashMap<String, Role> = roster
                    .heroes
                    .iter()
                    .filter_map(|hero| {
                        Role::parse(&hero.role)
                            .ok()
                            .map(|role| (hero.key.clone(), role))
                    })
                    .collect();

                let (strength, affinity, by_rank) = stats::build(
                    &generated,
                    &stats,
                    &cwatch_rates,
                    &blizzard,
                    &known_maps,
                    &roles,
                );

                let blended = strength
                    .entries
                    .iter()
                    .filter(|e| e.cpgg.is_some() && e.cwatch.is_some() && e.blizzard.is_some())
                    .count();
                eprintln!(
                    "  strength: {} heroes rated ({blended} from all three sources) | \
                     map affinity: {} hero/map pairs",
                    strength.entries.len(),
                    affinity.entries.len()
                );
                report_rank_slices(&by_rank);

                let strength_toml =
                    toml::to_string_pretty(&strength).context("serialising strength.toml")?;
                if write_if_changed(&data_dir.join("strength.toml"), &strength_toml).await? {
                    changed.push("strength.toml");
                }

                let affinity_toml =
                    toml::to_string_pretty(&affinity).context("serialising map_affinity.toml")?;
                if write_if_changed(&data_dir.join("map_affinity.toml"), &affinity_toml).await? {
                    changed.push("map_affinity.toml");
                }

                // Leaving the committed file alone is the right failure here.
                // Half of it is reachable only through Blizzard, so a run that
                // could not reach it has nothing better to offer than what is
                // already on disk — and unlike the synergy step this is only
                // half of a step, so aborting would throw away the two writes
                // above that succeeded.
                if by_rank.entries.is_empty() {
                    eprintln!(
                        "  warn: no rank-sliced win rates from either source; \
                         leaving data/strength_by_rank.toml alone"
                    );
                } else {
                    let by_rank_toml = toml::to_string_pretty(&by_rank)
                        .context("serialising strength_by_rank.toml")?;
                    if write_if_changed(&data_dir.join("strength_by_rank.toml"), &by_rank_toml)
                        .await?
                    {
                        changed.push("strength_by_rank.toml");
                    }
                }

                // Prevalence comes off responses already fetched above, which is
                // why it belongs to this step rather than one of its own: pick
                // rates move on the patch, on the same clock as the win rates.
                let prevalence = stats::prevalence(&generated, &blizzard, &roles);

                // Same failure shape as the rank slices, and for a sharper reason:
                // this file has exactly one source, so a run that could not reach
                // Blizzard has nothing at all to offer and must not overwrite
                // nine columns with none.
                if prevalence.entries.is_empty() {
                    eprintln!(
                        "  warn: no pick rates from blizzard; \
                         leaving data/prevalence.toml alone"
                    );
                } else {
                    report_pick_rate_shape(&blizzard, &roles);
                    report_pick_rate_agreement(&blizzard, &stats);
                    report_prevalence(&prevalence);

                    let prevalence_toml = toml::to_string_pretty(&prevalence)
                        .context("serialising prevalence.toml")?;
                    if write_if_changed(&data_dir.join("prevalence.toml"), &prevalence_toml).await?
                    {
                        changed.push("prevalence.toml");
                    }

                    // The yardstick, off the same nine responses. Never loaded by
                    // the app — see `BanRateFile`.
                    let ban_rates = stats::ban_rates(&generated, &blizzard);
                    let ban_rate_toml =
                        toml::to_string_pretty(&ban_rates).context("serialising ban_rate.toml")?;
                    if write_if_changed(&data_dir.join("ban_rate.toml"), &ban_rate_toml).await? {
                        changed.push("ban_rate.toml");
                    }
                }
            }
            Err(err) => eprintln!("  warn: counterpickgg index unusable: {err:#}"),
        }
    }

    if matches!(args.command, Command::Synergy | Command::All) {
        let roster = load_roster(&data_dir).await?;
        let hero_keys: Vec<String> = roster.heroes.iter().map(|h| h.key.clone()).collect();

        eprintln!("synergy: counterwatch duos ({} pages)", hero_keys.len());
        let scraped = counterwatch::scrape_duos(&mut fetcher, &hero_keys)
            .await
            .unwrap_or_else(|err| {
                eprintln!("  warn: counterwatch duos unusable: {err:#}");
                HashMap::new()
            });

        let existing = load_synergy(&data_dir).await?;
        let curated = existing
            .entries
            .iter()
            .filter(|e| e.curated.is_some())
            .count();

        // The same guard the counter blend has, and for the same reason: an
        // empty scrape is what a site redesign looks like, and it must not be
        // allowed to quietly empty a file the scorer reads.
        anyhow::ensure!(
            !scraped.is_empty() || curated > 0,
            "counterwatch published no duos; refusing to overwrite data/synergy.toml with nothing"
        );

        let file = merge_synergy(&generated, &existing, &scraped);
        eprintln!(
            "  {} pair(s): {} scraped, {} curated",
            file.entries.len(),
            scraped.len(),
            curated
        );

        let toml = toml::to_string_pretty(&file).context("serialising synergy.toml")?;
        if write_if_changed(&data_dir.join("synergy.toml"), &toml).await? {
            changed.push("synergy.toml");
        }
    }

    eprintln!(
        "\n{} live request(s); {}",
        fetcher.live_requests(),
        if changed.is_empty() {
            "no files changed".to_owned()
        } else {
            format!("updated {}", changed.join(", "))
        }
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curated(hero: &str, with: &str, value: i8, note: &str) -> SynergyEntry {
        SynergyEntry {
            hero: hero.to_owned(),
            with: with.to_owned(),
            value,
            cwatch: None,
            curated: Some(value),
            note: note.to_owned(),
            symmetric: true,
        }
    }

    fn scrape(pairs: &[(&str, &str, i8)]) -> HashMap<(String, String), i8> {
        pairs
            .iter()
            .map(|(a, b, v)| (((*a).to_owned(), (*b).to_owned()), *v))
            .collect()
    }

    /// The whole reason this merge exists rather than a plain serialise.
    #[test]
    fn a_curated_pair_the_scrape_does_not_list_survives_the_scrape() {
        let existing = SynergyFile {
            generated: "old".to_owned(),
            entries: vec![curated(
                "mercy",
                "pharah",
                80,
                "she is the reason he stays up",
            )],
        };
        let fresh = scrape(&[("winston", "lucio", 50)]);

        let merged = merge_synergy("today", &existing, &fresh);

        let kept = merged
            .entries
            .iter()
            .find(|e| e.hero == "mercy")
            .expect("the curated pair is still in the file");
        assert_eq!(kept.curated, Some(80));
        assert_eq!(kept.value, 80);
        assert_eq!(kept.note, "she is the reason he stays up");
        assert_eq!(
            kept.cwatch, None,
            "nothing measured it, so nothing claims to"
        );
    }

    #[test]
    fn a_curated_value_outranks_the_scraped_one_without_hiding_it() {
        let existing = SynergyFile {
            generated: "old".to_owned(),
            entries: vec![curated("winston", "lucio", 80, "speed onto the dive")],
        };
        let fresh = scrape(&[("winston", "lucio", 20)]);

        let merged = merge_synergy("today", &existing, &fresh);
        let entry = &merged.entries[0];

        assert_eq!(entry.value, 80, "curated wins");
        assert_eq!(entry.curated, Some(80));
        assert_eq!(entry.cwatch, Some(20), "and the reading is still traceable");
        assert_eq!(entry.resolved(), 80, "the loader agrees with the file");
    }

    /// A row the source has stopped publishing must lose its reading, or the
    /// file would keep asserting something nothing measures any more.
    #[test]
    fn a_scraped_pair_that_disappears_upstream_leaves_with_it() {
        let existing = SynergyFile {
            generated: "old".to_owned(),
            entries: vec![SynergyEntry {
                hero: "zarya".to_owned(),
                with: "vendetta".to_owned(),
                value: 100,
                cwatch: Some(100),
                curated: None,
                note: String::new(),
                symmetric: true,
            }],
        };

        let merged = merge_synergy("today", &existing, &scrape(&[]));
        assert!(
            merged.entries.is_empty(),
            "an unsourced, uncurated row outlived its source"
        );
    }

    #[test]
    fn the_file_is_written_in_a_stable_order() {
        let fresh = scrape(&[
            ("winston", "lucio", 50),
            ("ana", "zarya", 30),
            ("winston", "brigitte", 40),
        ]);

        let merged = merge_synergy("today", &SynergyFile::default(), &fresh);
        let order: Vec<(&str, &str)> = merged
            .entries
            .iter()
            .map(|e| (e.hero.as_str(), e.with.as_str()))
            .collect();

        assert_eq!(
            order,
            vec![
                ("ana", "zarya"),
                ("winston", "brigitte"),
                ("winston", "lucio")
            ]
        );
    }

    fn curated_matchup(hero: &str, vs: &str, value: i8, note: &str) -> MatchupEntry {
        MatchupEntry {
            hero: hero.to_owned(),
            vs: vs.to_owned(),
            // Zero, not `value`: a curated row that no source rated has no blend
            // behind it, and this column only ever holds the blend.
            value: 0,
            disagreement: false,
            cpgg: None,
            opick: None,
            cwatch: None,
            reason: String::new(),
            curated: Some(value),
            note: note.to_owned(),
        }
    }

    /// A row shaped the way `blend_values` emits them: a blended `value` with the
    /// source column that produced it, and nothing in the curated lane.
    fn blended_row(hero: &str, vs: &str, value: i8) -> MatchupEntry {
        MatchupEntry {
            hero: hero.to_owned(),
            vs: vs.to_owned(),
            value,
            disagreement: false,
            cpgg: Some(value),
            opick: None,
            cwatch: None,
            reason: String::new(),
            curated: None,
            note: String::new(),
        }
    }

    fn find_matchup<'a>(file: &'a MatchupsFile, hero: &str, vs: &str) -> &'a MatchupEntry {
        file.matchups
            .iter()
            .find(|e| e.hero == hero && e.vs == vs)
            .unwrap_or_else(|| panic!("{hero} vs {vs} is missing from the merged file"))
    }

    /// The whole reason this merge exists rather than a plain serialise, and the
    /// case `blend_values` cannot produce on its own: it skips a direction no
    /// trusted source rated instead of emitting an even row for it.
    #[test]
    fn a_curated_matchup_the_blend_does_not_list_survives_the_blend() {
        let existing = MatchupsFile {
            generated: "old".to_owned(),
            patch: "ingested old".to_owned(),
            note: String::new(),
            matchups: vec![curated_matchup("kiriko", "ana", 40, "suzu cleanses nade")],
        };

        let merged = merge_matchups(
            "new",
            "ingested new",
            &existing,
            vec![blended_row("winston", "zarya", 36)],
        );

        let entry = find_matchup(&merged, "kiriko", "ana");
        assert_eq!(entry.curated, Some(40));
        assert_eq!(entry.note, "suzu cleanses nade");
        assert_eq!(entry.resolved(), 40, "the loader reads the override");
        assert_eq!(
            entry.value, 0,
            "nothing measured this pair, so the blend column claims nothing"
        );
        assert_eq!(entry.cpgg, None);
        assert_eq!(entry.opick, None);
        assert_eq!(entry.cwatch, None);
    }

    /// Curated wins, and the blend it overrode stays legible beside it — the same
    /// traceability argument the per-source columns exist for.
    #[test]
    fn a_curated_matchup_outranks_the_blend_without_hiding_it() {
        let existing = MatchupsFile {
            generated: "old".to_owned(),
            patch: "ingested old".to_owned(),
            note: String::new(),
            matchups: vec![curated_matchup("kiriko", "ana", 40, "suzu cleanses nade")],
        };

        // The dominant real case: the sources rate the pair, and rate it even.
        let merged = merge_matchups(
            "new",
            "ingested new",
            &existing,
            vec![blended_row("kiriko", "ana", 0)],
        );

        assert_eq!(
            merged.matchups.len(),
            1,
            "the curated row is not duplicated"
        );
        let entry = &merged.matchups[0];
        assert_eq!(entry.curated, Some(40));
        assert_eq!(entry.note, "suzu cleanses nade");
        assert_eq!(entry.resolved(), 40, "curated wins");
        assert_eq!(
            entry.value, 0,
            "and `value` is still the blend, so reblend stays reproducible"
        );
        assert_eq!(entry.cpgg, Some(0), "and the reading is still traceable");
    }

    /// A row the sources have stopped rating must leave with them, or the file
    /// would keep asserting something nothing measures any more. Curation is the
    /// only thing that buys a row a stay of execution.
    #[test]
    fn an_uncurated_matchup_the_sources_drop_leaves_with_them() {
        let existing = MatchupsFile {
            generated: "old".to_owned(),
            patch: "ingested old".to_owned(),
            note: String::new(),
            matchups: vec![blended_row("emre", "zarya", 100)],
        };

        let merged = merge_matchups("new", "ingested new", &existing, Vec::new());

        assert!(
            merged.matchups.is_empty(),
            "an unsourced, uncurated row outlived its source: {:?}",
            merged.matchups
        );
    }

    /// Both keys, because the curated rows are appended out of a `HashMap` whose
    /// iteration order is arbitrary — the sort is what makes the diff reviewable.
    #[test]
    fn the_matchup_file_is_written_in_a_stable_order() {
        let existing = MatchupsFile {
            generated: "old".to_owned(),
            patch: "ingested old".to_owned(),
            note: String::new(),
            matchups: vec![
                curated_matchup("winston", "ana", 10, "note"),
                curated_matchup("ana", "winston", -10, "note"),
            ],
        };

        let merged = merge_matchups(
            "new",
            "ingested new",
            &existing,
            vec![
                blended_row("winston", "zarya", 36),
                blended_row("ana", "zarya", 20),
                blended_row("winston", "brigitte", 40),
            ],
        );

        let order: Vec<(&str, &str)> = merged
            .matchups
            .iter()
            .map(|e| (e.hero.as_str(), e.vs.as_str()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("ana", "winston"),
                ("ana", "zarya"),
                ("winston", "ana"),
                ("winston", "brigitte"),
                ("winston", "zarya"),
            ]
        );
    }

    /// `generated` and `patch` come from the caller, not from the file being
    /// merged over — `reblend` hands its own back precisely because nothing was
    /// fetched, and `counters` must not inherit a stale date from the old file.
    #[test]
    fn the_merge_stamps_the_provenance_it_is_handed() {
        let existing = MatchupsFile {
            generated: "old".to_owned(),
            patch: "ingested old".to_owned(),
            note: String::new(),
            matchups: vec![curated_matchup("kiriko", "ana", 40, "note")],
        };

        let merged = merge_matchups("2026-08-19", "ingested 2026-08-19", &existing, Vec::new());

        assert_eq!(merged.generated, "2026-08-19");
        assert_eq!(merged.patch, "ingested 2026-08-19");
    }
}
