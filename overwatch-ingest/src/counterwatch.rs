//! Scraper for counterwatch.gg.
//!
//! This is the only source derived from actual match data rather than opinion —
//! it measures who wins duels in community kill feeds and strips out each
//! hero's general strength — so it is worth having even though it gives the
//! least.
//!
//! Its full ranked table renders client-side (the page ships a
//! `BAILOUT_TO_CLIENT_SIDE_RENDERING` marker where the table would be), and
//! there is no API, so a plain fetch cannot see it. What *is* server-rendered
//! is JSON-LD, which per hero page gives:
//!
//! - an `ItemList` of the top 10 heroes that counter this hero, in order;
//! - the numeric counter rating of the single strongest one;
//! - an FAQ sentence naming the top three heroes this hero beats.
//!
//! That is ranks rather than values, so [`rank_to_value`] converts position
//! into a value anchored on the one published number. It covers roughly a
//! quarter of each row and is blended in at a low weight, mostly as an
//! independent check on the two opinion-based sources.
//!
//! The site's `best-duos` pages are the other half of what it is worth here,
//! and the only reason `synergy.toml` is not still empty. Those *are* rendered
//! server-side, but as markup rather than JSON-LD, so [`parse_duos`] reads the
//! anchors instead. See [`scrape_duos`] for what the numbers mean and what they
//! do not cover.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use scraper::{Html, Selector};
use serde_json::Value;

use crate::cache::Fetcher;

const BASE: &str = "https://www.counterwatch.gg";

/// What one hero page yields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeroCounters {
    /// Heroes that beat this hero, strongest first.
    pub countered_by: Vec<String>,
    /// Heroes this hero beats, strongest first.
    pub punishes: Vec<String>,
    /// Published counter rating of `countered_by[0]`, on the site's own scale.
    pub top_rating: Option<f32>,
}

/// Converts a 1-based rank into a value on -100..=100.
///
/// `top` is the magnitude given to rank 1 and decays linearly to
/// [`TAIL_MAGNITUDE`] at `len`. The list is already truncated to the most
/// lopsided matchups, so even the last entry is a real edge, not a coin flip.
const TAIL_MAGNITUDE: f32 = 25.0;

fn rank_to_value(position: usize, len: usize, top: f32) -> i8 {
    if position == 0 || len == 0 {
        return 0;
    }
    let magnitude = if len == 1 {
        top
    } else {
        let t = (position - 1) as f32 / (len - 1) as f32;
        top + (TAIL_MAGNITUDE - top) * t
    };
    magnitude.round().clamp(-100.0, 100.0) as i8
}

/// Maps the site's published rating for the single best counter onto our scale.
///
/// Ratings in the +10..+25 range correspond to the most lopsided matchups the
/// site reports, so that band is stretched onto 55..90 rather than taken
/// literally — the absolute numbers mean something different from ours.
fn top_magnitude(rating: Option<f32>) -> f32 {
    match rating {
        Some(r) => (55.0 + (r.abs() - 10.0) * 2.5).clamp(55.0, 90.0),
        None => 65.0,
    }
}

fn json_ld_blocks(html: &str) -> Result<Vec<Value>> {
    let selector = Selector::parse(r#"script[type="application/ld+json"]"#)
        .map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let document = Html::parse_document(html);

    Ok(document
        .select(&selector)
        .filter_map(|el| serde_json::from_str::<Value>(&el.text().collect::<String>()).ok())
        .collect())
}

/// Walks arbitrarily nested JSON-LD looking for nodes of a given `@type`.
fn collect_typed<'a>(node: &'a Value, wanted: &str, out: &mut Vec<&'a Value>) {
    match node {
        Value::Object(map) => {
            if map.get("@type").and_then(Value::as_str) == Some(wanted) {
                out.push(node);
            }
            for value in map.values() {
                collect_typed(value, wanted, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_typed(item, wanted, out);
            }
        }
        _ => {}
    }
}

/// Pulls the first `+12.3`-style number out of a sentence.
fn first_rating(text: &str) -> Option<f32> {
    let bytes = text.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'+' && b != b'-' {
            continue;
        }
        let rest = &text[i + 1..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if end == 0 {
            continue;
        }
        if let Ok(value) = rest[..end].parse::<f32>() {
            return Some(if b == b'-' { -value } else { value });
        }
    }
    None
}

/// Reads "X punishes A, B, and C hardest right now" into `[A, B, C]`.
fn parse_punishes(text: &str) -> Vec<String> {
    let Some(start) = text.find(" punishes ") else {
        return Vec::new();
    };
    let rest = &text[start + " punishes ".len()..];
    let Some(end) = rest.find(" hardest") else {
        return Vec::new();
    };

    rest[..end]
        .split(',')
        .flat_map(|part| part.split(" and "))
        .map(|name| name.trim().trim_end_matches('.').trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn parse_page(html: &str) -> Result<HeroCounters> {
    let blocks = json_ld_blocks(html)?;
    let mut out = HeroCounters::default();

    for block in &blocks {
        let mut lists = Vec::new();
        collect_typed(block, "ItemList", &mut lists);
        for list in lists {
            let Some(elements) = list.get("itemListElement").and_then(Value::as_array) else {
                continue;
            };
            // Several ItemLists can appear; the counters one is named.
            let name = list.get("name").and_then(Value::as_str).unwrap_or_default();
            if !name.to_ascii_lowercase().contains("counters to") {
                continue;
            }
            let mut ranked: Vec<(u64, String)> = elements
                .iter()
                .filter_map(|el| {
                    let position = el.get("position").and_then(Value::as_u64)?;
                    let hero = el
                        .get("item")
                        .and_then(|i| i.get("name"))
                        .or_else(|| el.get("name"))
                        .and_then(Value::as_str)?;
                    Some((position, hero.to_owned()))
                })
                .collect();
            ranked.sort_by_key(|(position, _)| *position);
            out.countered_by = ranked.into_iter().map(|(_, hero)| hero).collect();
        }

        let mut faqs = Vec::new();
        collect_typed(block, "FAQPage", &mut faqs);
        for faq in faqs {
            let Some(questions) = faq.get("mainEntity").and_then(Value::as_array) else {
                continue;
            };
            for question in questions {
                let answer = question
                    .get("acceptedAnswer")
                    .and_then(|a| a.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();

                if answer.contains("counter rating of") && out.top_rating.is_none() {
                    out.top_rating = first_rating(answer);
                }
                if out.punishes.is_empty() {
                    out.punishes = parse_punishes(answer);
                }
            }
        }
    }

    Ok(out)
}

/// Fetches every hero page and converts the rankings into matchup values.
///
/// Returns `(hero, opponent) -> value` from `hero`'s perspective, covering only
/// the pairs the site actually ranks.
pub async fn scrape(
    fetcher: &mut Fetcher,
    hero_keys: &[String],
    names: &HashMap<String, String>,
) -> Result<HashMap<(String, String), i8>> {
    let mut out = HashMap::new();

    let mut unresolved: Vec<&str> = Vec::new();

    for (i, key) in hero_keys.iter().enumerate() {
        let cache_slug = format!("counterwatch-{key}.html");

        if fetcher.is_missing(&cache_slug).await {
            unresolved.push(key);
            continue;
        }

        let mut parsed = HeroCounters::default();
        for candidate in crate::slugs::counterwatch(key) {
            let url = format!("{BASE}/stats/overwatch/counters/{candidate}");

            let Ok(body) = fetcher.get(&url, &cache_slug).await else {
                continue;
            };

            parsed = parse_page(&body)
                .with_context(|| format!("parsing the counterwatch page for {key}"))?;
            if !parsed.countered_by.is_empty() || !parsed.punishes.is_empty() {
                break;
            }
            fetcher.forget(&cache_slug).await;
        }

        if parsed.countered_by.is_empty() && parsed.punishes.is_empty() {
            // Record the absence so this hero stops costing a request per run.
            fetcher.mark_missing(&cache_slug).await;
            unresolved.push(key);
            continue;
        }

        let top = top_magnitude(parsed.top_rating);
        let len = parsed.countered_by.len();

        // "Countered by X at rank r" is a negative value for this hero.
        for (idx, opponent_name) in parsed.countered_by.iter().enumerate() {
            let Some(opponent) = names.get(opponent_name) else {
                continue;
            };
            if opponent == key {
                continue;
            }
            let value = -rank_to_value(idx + 1, len, top);
            out.insert((key.clone(), opponent.clone()), value);
        }

        // "Punishes A, B, C" is a positive value, and must not overwrite a
        // stronger signal already recorded from the countered-by list.
        let punish_len = parsed.punishes.len();
        for (idx, opponent_name) in parsed.punishes.iter().enumerate() {
            let Some(opponent) = names.get(opponent_name) else {
                continue;
            };
            if opponent == key {
                continue;
            }
            out.entry((key.clone(), opponent.clone()))
                .or_insert_with(|| rank_to_value(idx + 1, punish_len, top));
        }

        if (i + 1) % 10 == 0 {
            eprintln!("  counterwatch: {}/{} heroes", i + 1, hero_keys.len());
        }
    }

    if !unresolved.is_empty() {
        eprintln!(
            "  warn: counterwatch has no page for: {}",
            unresolved.join(", ")
        );
    }

    Ok(out)
}

/// Reads the hero's *own* win rate out of a stats page.
///
/// The figure lives in the JSON-LD `description`, phrased as
/// `"Zenyatta (Support) Overwatch stats: 53.6% win rate across 128,636
/// community-tracked matches in 5V5, All Ranks."`. Worth having beside
/// counterpickgg's column because the two disagree systematically and this is
/// the better-instrumented of the pair: one decimal rather than a rounded
/// integer, a published sample size, and Bayesian shrinkage applied.
///
/// Anchored on the whole `Overwatch stats: ` phrase rather than on `% win rate`
/// alone. The page carries sixty-odd win rates — every hero in the counters and
/// duos tables has one — and only this sentence is about the hero whose page
/// it is. Matching the bare label would silently read a neighbour's number.
pub fn parse_win_rate(html: &str) -> Option<f32> {
    const MARKER: &str = "Overwatch stats: ";
    let rest = &html[html.find(MARKER)? + MARKER.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(rest.len());
    if end == 0 || !rest[end..].starts_with("% win rate") {
        return None;
    }
    rest[..end].parse::<f32>().ok()
}

/// Fetches every hero's stats page for its win rate.
///
/// Heroes the site has no page for are simply absent, which the blend reads as
/// "this source has no opinion" rather than as a zero.
pub async fn scrape_win_rates(
    fetcher: &mut Fetcher,
    hero_keys: &[String],
) -> Result<HashMap<String, f32>> {
    let mut out = HashMap::new();
    let mut unresolved: Vec<&str> = Vec::new();

    for (i, key) in hero_keys.iter().enumerate() {
        let cache_slug = format!("counterwatch-stats-{key}.html");

        if fetcher.is_missing(&cache_slug).await {
            unresolved.push(key);
            continue;
        }

        let mut rate = None;
        for candidate in crate::slugs::counterwatch(key) {
            let url = format!("{BASE}/stats/overwatch/heroes/{candidate}");

            let Ok(body) = fetcher.get(&url, &cache_slug).await else {
                continue;
            };
            rate = parse_win_rate(&body);
            if rate.is_some() {
                break;
            }
            fetcher.forget(&cache_slug).await;
        }

        match rate {
            Some(rate) => {
                out.insert(key.clone(), rate);
            }
            None => {
                fetcher.mark_missing(&cache_slug).await;
                unresolved.push(key);
            }
        }

        if (i + 1) % 10 == 0 {
            eprintln!("  counterwatch stats: {}/{} heroes", i + 1, hero_keys.len());
        }
    }

    if !unresolved.is_empty() {
        eprintln!(
            "  warn: counterwatch has no win rate for: {}",
            unresolved.join(", ")
        );
    }

    Ok(out)
}

/// The band a duo's "% above expected" is stretched over to reach -100..=100.
///
/// Fixed rather than fitted, for the same reason [`crate::stats`] anchors win
/// rates on a fixed band: a season in which every duo converges on its expected
/// win rate should read as "nothing stands out", not have the remaining noise
/// amplified to fill the scale. Three points above expected is an enormous
/// effect for a pair of heroes — the observed spread across the roster runs
/// from about +0.3 to +3.0 — so that is the ceiling.
const SYNERGY_CEILING: f32 = 3.0;

/// One duo entry: the partner, in our hero keys, and its value on -100..=100.
#[derive(Debug, Clone, PartialEq)]
pub struct Duo {
    pub with: String,
    pub value: i8,
}

/// Pulls the duo partners off one `best-duos` page.
///
/// The published list is short — three partners per role — and the site chooses
/// it by kit combo before annotating each with its measured effect, so this is
/// a curated shortlist carrying real numbers rather than a full ranking. That
/// makes the signal positive-only: a pair that is not listed is not "measured to
/// do nothing for each other", it is simply not in the top three, which is why
/// the loader must treat an absent pair as unrated.
///
/// Partners are read off the `href` rather than the displayed name. The slug
/// survives `Lúcio` and `Soldier: 76` without an accent or a colon to normalise,
/// and it is the same spelling [`crate::slugs`] already knows how to translate.
fn parse_duos(html: &str) -> Result<Vec<Duo>> {
    let anchor = Selector::parse(r#"a[href^="/stats/overwatch/heroes/"]"#)
        .map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let document = Html::parse_document(html);

    let mut out: Vec<Duo> = Vec::new();
    for element in document.select(&anchor) {
        let text: String = element.text().collect::<Vec<_>>().join(" ");
        // The percentage alone is not enough to identify the row: the page
        // carries other figures. The label beside it is what makes it a duo.
        if !text.contains("above expected") {
            continue;
        }
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Some(slug) = href.rsplit('/').next().filter(|s| !s.is_empty()) else {
            continue;
        };
        let with = crate::slugs::counterwatch_to_ours(slug);

        let Some(percent) = first_percentage(&text) else {
            continue;
        };
        let value = overwatch_core::normalize(percent, -SYNERGY_CEILING, SYNERGY_CEILING);

        // The page repeats its top pick in a highlight strip above the list, so
        // the same partner arrives twice with the same number.
        if out.iter().any(|d| d.with == with) {
            continue;
        }
        out.push(Duo { with, value });
    }

    Ok(out)
}

/// Reads the first signed percentage out of a duo row, e.g. `+1.7%` or `-0.4%`.
///
/// Deliberately stricter than [`first_rating`]: the trailing `%` is required, so
/// a digit that wandered in from a hero name — `Soldier: 76` shares the row —
/// cannot be mistaken for the measurement.
fn first_percentage(text: &str) -> Option<f32> {
    for (i, &b) in text.as_bytes().iter().enumerate() {
        if b != b'+' && b != b'-' {
            continue;
        }
        let rest = &text[i + 1..];
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if end == 0 || !rest[end..].starts_with('%') {
            continue;
        }
        if let Ok(value) = rest[..end].parse::<f32>() {
            return Some(if b == b'-' { -value } else { value });
        }
    }
    None
}

/// Fetches every hero's duo page and returns `(hero, partner) -> value`.
///
/// One direction per row as published. The caller decides what to do about the
/// other direction; the site lists a pair from both sides often enough that
/// forcing symmetry here would hide a genuine disagreement.
pub async fn scrape_duos(
    fetcher: &mut Fetcher,
    hero_keys: &[String],
) -> Result<HashMap<(String, String), i8>> {
    let known: HashSet<&str> = hero_keys.iter().map(String::as_str).collect();
    let mut out = HashMap::new();
    let mut unresolved: Vec<&str> = Vec::new();

    for (i, key) in hero_keys.iter().enumerate() {
        let cache_slug = format!("counterwatch-duos-{key}.html");

        if fetcher.is_missing(&cache_slug).await {
            unresolved.push(key);
            continue;
        }

        let mut duos: Vec<Duo> = Vec::new();
        for candidate in crate::slugs::counterwatch(key) {
            let url = format!("{BASE}/stats/overwatch/best-duos/{candidate}");

            let Ok(body) = fetcher.get(&url, &cache_slug).await else {
                continue;
            };
            duos = parse_duos(&body)
                .with_context(|| format!("parsing the counterwatch duos page for {key}"))?;
            if !duos.is_empty() {
                break;
            }
            fetcher.forget(&cache_slug).await;
        }

        if duos.is_empty() {
            fetcher.mark_missing(&cache_slug).await;
            unresolved.push(key);
            continue;
        }

        for duo in duos {
            // A slug the roster cannot name is a hero we do not draft, or a
            // spelling nobody has taught `slugs` about yet. Either way it is a
            // row to drop rather than to invent a key for.
            if duo.with == *key || !known.contains(duo.with.as_str()) {
                continue;
            }
            out.insert((key.clone(), duo.with), duo.value);
        }

        if (i + 1) % 10 == 0 {
            eprintln!("  counterwatch duos: {}/{} heroes", i + 1, hero_keys.len());
        }
    }

    if !unresolved.is_empty() {
        eprintln!(
            "  warn: counterwatch has no duo page for: {}",
            unresolved.join(", ")
        );
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r##"
      <html><body>
      <script type="application/ld+json">
      {"@context":"https://schema.org","@type":"ItemList",
       "name":"Best counters to Reinhardt in Overwatch",
       "itemListElement":[
         {"@type":"ListItem","position":1,"item":{"@type":"Thing","name":"Pharah"}},
         {"@type":"ListItem","position":2,"item":{"@type":"Thing","name":"Torbjörn"}},
         {"@type":"ListItem","position":3,"item":{"@type":"Thing","name":"Zenyatta"}}
       ]}
      </script>
      <script type="application/ld+json">
      {"@context":"https://schema.org","@type":"FAQPage","mainEntity":[
        {"@type":"Question","name":"What is the best counter to Reinhardt in Overwatch?",
         "acceptedAnswer":{"@type":"Answer","text":"Pharah is the strongest pick against Reinhardt right now, with a counter rating of +11.4 (an estimated +10% swing to an even fight), followed by Torbjörn and Zenyatta."}},
        {"@type":"Question","name":"Who is Reinhardt good against in Overwatch?",
         "acceptedAnswer":{"@type":"Answer","text":"Reinhardt punishes Hazard, Anran, and Wrecking Ball hardest right now. If the enemy runs one of them, staying on Reinhardt is usually right."}}
      ]}
      </script>
      </body></html>
    "##;

    #[test]
    fn reads_the_ranked_counter_list() {
        let parsed = parse_page(FIXTURE).expect("fixture parses");
        assert_eq!(parsed.countered_by, vec!["Pharah", "Torbjörn", "Zenyatta"]);
    }

    #[test]
    fn reads_the_top_rating_and_the_punish_list() {
        let parsed = parse_page(FIXTURE).expect("fixture parses");
        assert_eq!(parsed.top_rating, Some(11.4));
        assert_eq!(parsed.punishes, vec!["Hazard", "Anran", "Wrecking Ball"]);
    }

    #[test]
    fn a_page_without_json_ld_is_empty_not_an_error() {
        let parsed = parse_page("<html><body>nothing</body></html>").expect("parses");
        assert_eq!(parsed, HeroCounters::default());
    }

    #[test]
    fn ranks_decay_from_the_top_to_the_tail() {
        let top = top_magnitude(Some(11.4));
        let first = rank_to_value(1, 10, top);
        let last = rank_to_value(10, 10, top);
        assert!(first > last, "rank 1 must outweigh rank 10");
        assert_eq!(last, TAIL_MAGNITUDE as i8);
        assert!((55..=90).contains(&first));
    }

    #[test]
    fn rank_conversion_handles_degenerate_input() {
        assert_eq!(rank_to_value(0, 10, 70.0), 0);
        assert_eq!(rank_to_value(1, 0, 70.0), 0);
        assert_eq!(rank_to_value(1, 1, 70.0), 70);
    }

    #[test]
    fn ratings_are_extracted_from_prose() {
        assert_eq!(
            first_rating("a counter rating of +11.4 (an estimated"),
            Some(11.4)
        );
        assert_eq!(first_rating("down -3.5 overall"), Some(-3.5));
        assert_eq!(first_rating("no numbers here"), None);
    }

    #[test]
    fn punish_lists_survive_the_oxford_comma() {
        assert_eq!(
            parse_punishes(
                "Reinhardt punishes Hazard, Anran, and Wrecking Ball hardest right now."
            ),
            vec!["Hazard", "Anran", "Wrecking Ball"]
        );
        assert_eq!(
            parse_punishes("Ana punishes Roadhog and Mauga hardest right now."),
            vec!["Roadhog", "Mauga"]
        );
        assert!(parse_punishes("nothing relevant").is_empty());
    }

    /// Trimmed from a real `best-duos` page: the highlight strip repeats the
    /// top pick, and the partner is only identifiable from the `href`.
    const DUOS_FIXTURE: &str = r##"
      <html><body>
      <a class="group" href="/stats/overwatch/heroes/echo">
        <div>Best <!-- -->Damage</div>
        <p class="truncate">Echo</p>
        <p><span>+1.7%</span><span> <!-- -->above expected</span></p>
      </a>
      <a class="group" href="/stats/overwatch/heroes/echo">
        <p>Echo</p><p><span>+1.7%</span><span> above expected</span></p>
      </a>
      <a class="group" href="/stats/overwatch/heroes/soldier76">
        <p>Soldier: 76</p><p><span>+0.8%</span><span> above expected</span></p>
      </a>
      <a class="group" href="/stats/overwatch/heroes/jetpackcat">
        <p>Jetpack Cat</p><p><span>-0.4%</span><span> above expected</span></p>
      </a>
      <a class="group" href="/stats/overwatch/heroes/mercy">
        <p>Mercy</p><p><span>54.2%</span><span> win rate</span></p>
      </a>
      </body></html>
    "##;

    #[test]
    fn duo_partners_are_read_from_the_slug_not_the_displayed_name() {
        let duos = parse_duos(DUOS_FIXTURE).expect("the fixture parses");
        let keys: Vec<&str> = duos.iter().map(|d| d.with.as_str()).collect();

        // Translated back out of counterwatch's spelling, and deduplicated
        // against the highlight strip that repeats the top pick.
        assert_eq!(keys, vec!["echo", "soldier-76", "jetpack-cat"]);
    }

    #[test]
    fn a_row_without_the_above_expected_label_is_not_a_duo() {
        let duos = parse_duos(DUOS_FIXTURE).expect("the fixture parses");
        assert!(
            !duos.iter().any(|d| d.with == "mercy"),
            "a win-rate row was read as a synergy"
        );
    }

    #[test]
    fn the_published_percentage_is_stretched_onto_our_scale() {
        let duos = parse_duos(DUOS_FIXTURE).expect("the fixture parses");
        let value = |key: &str| duos.iter().find(|d| d.with == key).expect("present").value;

        // +3.0 is the ceiling, so +1.7 lands a little over half way up.
        assert_eq!(value("echo"), 57);
        assert_eq!(value("soldier-76"), 27);
        // Nothing forces the sign positive: if the site ever publishes a
        // negative pairing, it has to survive as one.
        assert_eq!(value("jetpack-cat"), -13);
    }

    #[test]
    fn a_heros_own_win_rate_is_read_and_not_a_neighbours() {
        // The counters table above the description carries other heroes' rates,
        // and the hero's own sentence is the one that names the page.
        let page = concat!(
            r#"<div title="Reinhardt · 54.5% win rate · 12,580 tracked matches"></div>"#,
            r#"<script type="application/ld+json">{"description":"Zenyatta (Support) "#,
            r#"Overwatch stats: 53.6% win rate across 128,636 community-tracked "#,
            r#"matches in 5V5, All Ranks."}</script>"#,
        );
        assert_eq!(parse_win_rate(page), Some(53.6));
    }

    #[test]
    fn a_page_missing_the_stats_sentence_yields_nothing_rather_than_a_guess() {
        assert_eq!(parse_win_rate("<html>no stats here</html>"), None);
        assert_eq!(
            parse_win_rate("Overwatch stats: coming soon"),
            None,
            "a sentence without a number is not a measurement"
        );
    }

    #[test]
    fn a_percentage_is_only_read_when_it_carries_its_sign_and_its_sigil() {
        assert_eq!(first_percentage("+1.7% above expected"), Some(1.7));
        assert_eq!(first_percentage("-0.4% above expected"), Some(-0.4));
        // A hero name shares the row, and 76 is not a measurement.
        assert_eq!(first_percentage("Soldier: 76"), None);
        // A bare number without the sigil is some other figure entirely.
        assert_eq!(first_percentage("+12 games"), None);
    }
}
