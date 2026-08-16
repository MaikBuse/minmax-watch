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
use overwatch_data::schema::{HeroesFile, MapsFile, MatchupsFile, SynergyEntry, SynergyFile};
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
    /// Hero win rates and map affinity, without re-scraping the whole matrix.
    ///
    /// Split out from `Counters` because the two go stale at different rates: a
    /// balance patch moves every win rate the same week and barely touches the
    /// matchup opinions, and re-reading 53 win rates should not mean re-fetching
    /// 160 pages and reviewing a diff of the entire matrix to find them.
    Strength,
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
            "synergy" => command = Command::Synergy,
            "strength" => command = Command::Strength,
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
usage: overwatch-ingest [roster|counters|synergy|strength|art|all|brand] [--refresh]

  roster      regenerate heroes.toml and maps.toml from the OverFast API
  counters    regenerate matchups.toml, strength.toml and map_affinity.toml
  synergy     regenerate the scraped half of synergy.toml, keeping curated rows
  strength    regenerate strength.toml and map_affinity.toml only
  art         redownload hero portraits and map thumbnails into overwatch-web/assets
  all         all four (default)
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

                let (strength, affinity) =
                    stats::build(&generated, &stats, &cwatch_rates, &known_maps);

                let blended = strength
                    .entries
                    .iter()
                    .filter(|e| e.cpgg.is_some() && e.cwatch.is_some())
                    .count();
                eprintln!(
                    "  strength: {} heroes rated ({blended} from both sources) | \
                     map affinity: {} hero/map pairs",
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
}
