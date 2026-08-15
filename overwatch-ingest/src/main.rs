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
mod brand;
mod cache;
mod counterpickgg;
mod counterwatch;
mod overfast;
mod overpicker;
mod slugs;
mod stats;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use overwatch_data::schema::{HeroesFile, MapsFile, MatchupsFile};
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
    /// Hero portraits and map thumbnails into the web assets.
    Art,
    /// Everything.
    All,
    /// Rasterise the brand SVGs into favicons, PWA icons and the OG card.
    ///
    /// Not part of `All`: it touches no upstream source, needs no network, and
    /// only has anything to do when the artwork itself changes.
    Brand,
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
            "art" => command = Command::Art,
            "all" => command = Command::All,
            "brand" => command = Command::Brand,
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
usage: overwatch-ingest [roster|counters|art|all|brand] [--refresh]

  roster      regenerate heroes.toml and maps.toml from the OverFast API
  counters    regenerate matchups.toml, strength.toml and map_affinity.toml
  art         redownload hero portraits and map thumbnails into overwatch-web/assets
  all         all three (default)
  brand       rasterise the brand SVGs into favicons, PWA icons and og.png
              (local only - no network, and not included in `all`)

  --refresh   ignore the cache in data/sources and re-fetch everything";

fn print_usage() {
    println!("{USAGE}");
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
        eprintln!("art: portraits and map thumbnails");

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
            "  {} portraits / {} thumbnails; {} written, {} orphan(s) removed",
            report.heroes, report.maps, report.changed, report.removed
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

        eprintln!("  source: counterwatch ({} pages)", hero_keys.len());
        let cwatch = counterwatch::scrape(&mut fetcher, &hero_keys, &names)
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

        let file = MatchupsFile {
            generated: generated.clone(),
            patch: patch_label(&generated),
            matchups,
        };
        let toml = toml::to_string_pretty(&file).context("serialising matchups.toml")?;
        if write_if_changed(&data_dir.join("matchups.toml"), &toml).await? {
            changed.push("matchups.toml");
        }

        // The index table is one extra fetch and carries both hero win rates
        // and the only map-performance data reachable without a browser.
        eprintln!("  source: counterpickgg index (1 page)");
        match counterpickgg::scrape_index(&mut fetcher).await {
            Ok(stats) => {
                let known_maps: HashSet<String> = load_map_keys(&data_dir).await?;
                let (strength, affinity) = stats::build(&generated, &stats, &known_maps);

                eprintln!(
                    "  strength: {} heroes rated | map affinity: {} hero/map pairs",
                    strength.entries.len(),
                    affinity.entries.len()
                );

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
            }
            Err(err) => eprintln!("  warn: counterpickgg index unusable: {err:#}"),
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
