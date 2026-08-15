//! Roster and map list from the OverFast API.
//!
//! This is the one part of the dataset with a real API behind it, so it needs
//! no scraping and stays current with Blizzard's releases on its own.

use anyhow::{Context, Result};
use overwatch_core::{GameMode, Role};
use overwatch_data::schema::{HeroEntry, HeroesFile, MapEntry, MapsFile};
use serde::Deserialize;

use crate::aliases;
use crate::cache::Fetcher;

const BASE: &str = "https://overfast-api.tekrop.fr";

#[derive(Debug, Deserialize)]
struct ApiHero {
    key: String,
    name: String,
    role: String,
    /// Square portrait on Blizzard's CDN. Defaulted rather than required: a
    /// missing image should cost us an icon, not the whole roster.
    #[serde(default)]
    portrait: String,
}

#[derive(Debug, Deserialize)]
struct ApiMap {
    key: String,
    name: String,
    #[serde(default)]
    gamemodes: Vec<String>,
    /// Wide screenshot, hosted by OverFast itself.
    #[serde(default)]
    screenshot: String,
}

pub async fn fetch_heroes(fetcher: &mut Fetcher, generated: &str) -> Result<HeroesFile> {
    let body = fetcher
        .get(&format!("{BASE}/heroes"), "overfast-heroes.json")
        .await?;
    let api: Vec<ApiHero> =
        serde_json::from_str(&body).context("parsing the OverFast heroes response")?;

    anyhow::ensure!(!api.is_empty(), "OverFast returned an empty roster");

    let mut heroes = Vec::with_capacity(api.len());
    for hero in &api {
        // Fail loudly on an unrecognised role rather than guessing: a new role
        // would change how the whole app is laid out.
        let role = Role::parse(&hero.role)
            .with_context(|| format!("hero {:?} has role {:?}", hero.key, hero.role))?;
        heroes.push(HeroEntry {
            key: hero.key.clone(),
            name: hero.name.clone(),
            role: role.as_str().to_owned(),
            aliases: aliases::for_hero(&hero.key, &hero.name),
        });
    }

    heroes.sort_by(|a, b| a.key.cmp(&b.key));
    report_collisions(
        "hero",
        &heroes
            .iter()
            .map(|h| (h.key.clone(), h.aliases.clone()))
            .collect::<Vec<_>>(),
    );

    Ok(HeroesFile {
        generated: generated.to_owned(),
        source: format!("{BASE}/heroes"),
        heroes,
    })
}

pub async fn fetch_maps(fetcher: &mut Fetcher, generated: &str) -> Result<MapsFile> {
    let body = fetcher
        .get(&format!("{BASE}/maps"), "overfast-maps.json")
        .await?;
    let api: Vec<ApiMap> =
        serde_json::from_str(&body).context("parsing the OverFast maps response")?;

    anyhow::ensure!(!api.is_empty(), "OverFast returned an empty map list");

    let mut maps = Vec::new();
    for map in &api {
        // OverFast also lists arcade, workshop and deathmatch maps. We only
        // ever draft on the competitive modes, and including the rest would
        // pad the map picker with noise.
        let Some(mode) = map.gamemodes.iter().find_map(|g| GameMode::parse(g).ok()) else {
            continue;
        };

        maps.push(MapEntry {
            key: map.key.clone(),
            name: map.name.clone(),
            mode: mode.as_str().to_owned(),
            aliases: aliases::for_map(&map.key, &map.name),
        });
    }

    anyhow::ensure!(
        !maps.is_empty(),
        "no competitive maps survived filtering - has the OverFast schema changed?"
    );

    maps.sort_by_key(|a| (a.mode.clone(), a.key.clone()));
    report_collisions(
        "map",
        &maps
            .iter()
            .map(|m| (m.key.clone(), m.aliases.clone()))
            .collect::<Vec<_>>(),
    );

    Ok(MapsFile {
        generated: generated.to_owned(),
        source: format!("{BASE}/maps"),
        maps,
    })
}

/// Where the artwork for each hero and map lives, as `(key, url)` pairs.
///
/// Reads the same two cached responses the roster step used, so running the art
/// step straight after `roster` costs no extra live requests. Entries without a
/// URL are dropped here; the caller decides whether the resulting gap matters.
pub async fn art_urls(
    fetcher: &mut Fetcher,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
    let heroes_body = fetcher
        .get(&format!("{BASE}/heroes"), "overfast-heroes.json")
        .await?;
    let heroes: Vec<ApiHero> =
        serde_json::from_str(&heroes_body).context("parsing the OverFast heroes response")?;

    let maps_body = fetcher
        .get(&format!("{BASE}/maps"), "overfast-maps.json")
        .await?;
    let maps: Vec<ApiMap> =
        serde_json::from_str(&maps_body).context("parsing the OverFast maps response")?;

    let portraits = heroes
        .into_iter()
        .filter(|h| !h.portrait.is_empty())
        .map(|h| (h.key, h.portrait))
        .collect();
    let screenshots = maps
        .into_iter()
        .filter(|m| !m.screenshot.is_empty())
        .map(|m| (m.key, m.screenshot))
        .collect();

    Ok((portraits, screenshots))
}

fn report_collisions(kind: &str, entries: &[(String, Vec<String>)]) {
    for (alias, keys) in aliases::collisions(entries) {
        eprintln!(
            "  note: {kind} alias {alias:?} is ambiguous between {}",
            keys.join(", ")
        );
    }
}
