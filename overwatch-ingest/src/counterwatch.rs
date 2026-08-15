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

use std::collections::HashMap;

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
}
