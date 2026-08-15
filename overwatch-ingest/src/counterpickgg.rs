//! Scraper for counterpickgg.com — the primary matchup source.
//!
//! One hero page carries that hero's complete row against every other hero, so
//! the whole matrix costs one fetch per hero rather than one per pairing. Each
//! card looks like this (SVG and image markup elided):
//!
//! ```html
//! <article>
//!   <header>
//!     <a href="/heroes/pharah"><h3>Pharah</h3></a>
//!     <div class="...">9/10</div>
//!   </header>
//!   <a aria-label="View Pharah matchup details" href="/heroes/reinhardt/vs/pharah">
//!     <ul><li><span>・</span><span><span>Reinhardt is very weak against airborne targets.</span></span></li></ul>
//!   </a>
//! </article>
//! ```
//!
//! The page is server-rendered, so the DOM is real and CSS selectors are enough
//! — no headless browser, no flight-payload unescaping.
//!
//! The rating reads as *difficulty for the subject hero*: 9/10 means Pharah is
//! a nightmare for Reinhardt, so it converts to a strongly negative value from
//! Reinhardt's side.

use anyhow::{Context, Result};
use scraper::{Html, Selector};

use crate::cache::Fetcher;

const BASE: &str = "https://counterpickgg.com";

/// One directed matchup exactly as the site states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMatchup {
    pub hero: String,
    pub vs: String,
    /// 1..=10, or `None` for heroes the site has not rated yet — the very
    /// newest releases have cards but no numbers.
    pub difficulty: Option<u8>,
    pub reason: String,
}

/// A row of the index table: overall strength and map performance for one hero.
///
/// The index is one fetch and yields both the win rate that feeds
/// `base_strength` and the only map-affinity data any source exposes to a plain
/// HTTP client.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroStats {
    pub hero: String,
    pub win_rate: f32,
    pub pick_rate: f32,
    /// Map keys where this hero performs best, strongest first.
    pub best_maps: Vec<String>,
}

/// Reads the first number out of a cell like `"58 % 58 %"`, which renders the
/// value twice — once as a bar, once as a label.
fn first_percentage(text: &str) -> Option<f32> {
    let trimmed = text.trim();
    let end = trimmed
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(trimmed.len());
    if end == 0 {
        return None;
    }
    trimmed[..end].parse().ok()
}

/// Pulls the map key out of `/maps?selected=eichenwalde#hero-rankings`.
fn parse_map_href(href: &str) -> Option<String> {
    let query = href.split('?').nth(1)?;
    let value = query
        .split('&')
        .find_map(|param| param.strip_prefix("selected="))?;
    let key = value.split('#').next().unwrap_or(value);
    (!key.is_empty()).then(|| key.to_owned())
}

/// Parses the hero index table.
pub fn parse_index(html: &str) -> Result<Vec<HeroStats>> {
    let sel =
        |s: &str| Selector::parse(s).map_err(|e| anyhow::anyhow!("invalid selector {s:?}: {e}"));
    let row_sel = sel("tr")?;
    let cell_sel = sel("td")?;
    let link_sel = sel("a")?;

    let document = Html::parse_document(html);
    let mut out: Vec<HeroStats> = Vec::new();

    for row in document.select(&row_sel) {
        let cells: Vec<_> = row.select(&cell_sel).collect();
        // hero | win% | pick% | ...counters... | best maps
        if cells.len() < 3 {
            continue;
        }

        let Some(hero) = cells[0]
            .select(&link_sel)
            .filter_map(|a| a.value().attr("href"))
            .find_map(|href| href.strip_prefix("/heroes/"))
            .map(crate::slugs::counterpickgg_to_ours)
        else {
            continue;
        };

        let Some(win_rate) = first_percentage(&cells[1].text().collect::<String>()) else {
            continue;
        };
        let pick_rate = first_percentage(&cells[2].text().collect::<String>()).unwrap_or_default();

        // The best-maps column is last and is the only cell linking to /maps.
        let best_maps = cells
            .last()
            .map(|cell| {
                cell.select(&link_sel)
                    .filter_map(|a| a.value().attr("href"))
                    .filter_map(parse_map_href)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if out.iter().any(|s| s.hero == hero) {
            continue;
        }
        out.push(HeroStats {
            hero,
            win_rate,
            pick_rate,
            best_maps,
        });
    }

    Ok(out)
}

pub async fn scrape_index(fetcher: &mut Fetcher) -> Result<Vec<HeroStats>> {
    let body = fetcher.get(BASE, "counterpickgg-index.html").await?;
    let stats = parse_index(&body).context("parsing the counterpickgg index table")?;
    anyhow::ensure!(
        !stats.is_empty(),
        "the counterpickgg index yielded no rows - the table markup changed"
    );
    Ok(stats)
}

struct Selectors {
    article: Selector,
    detail_link: Selector,
    badge: Selector,
    reason: Selector,
}

impl Selectors {
    fn new() -> Result<Self> {
        // `Selector::parse` errors borrow their input, so they cannot go
        // straight into `anyhow::Error`; render them instead.
        let sel = |s: &str| {
            Selector::parse(s).map_err(|e| anyhow::anyhow!("invalid selector {s:?}: {e}"))
        };
        Ok(Self {
            article: sel("article")?,
            detail_link: sel(r#"a[href*="/vs/"]"#)?,
            badge: sel("header div")?,
            reason: sel("li")?,
        })
    }
}

/// Pulls `{subject}` and `{opponent}` out of `/heroes/{subject}/vs/{opponent}`.
///
/// Taking the direction from the href rather than from page context means a
/// layout change cannot silently transpose the matrix.
fn parse_detail_href(href: &str) -> Option<(String, String)> {
    let path = href.split(['?', '#']).next().unwrap_or(href);
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    if segments.next()? != "heroes" {
        return None;
    }
    let subject = segments.next()?;
    if segments.next()? != "vs" {
        return None;
    }
    let opponent = segments.next()?;
    if subject.is_empty() || opponent.is_empty() {
        return None;
    }
    Some((subject.to_owned(), opponent.to_owned()))
}

/// Reads a `9/10` badge. Returns `None` for anything that is not exactly a
/// rating out of ten, so unrelated `<div>`s in the header are ignored.
fn parse_difficulty(text: &str) -> Option<u8> {
    let (numerator, denominator) = text.trim().split_once('/')?;
    if denominator.trim() != "10" {
        return None;
    }
    let value: u8 = numerator.trim().parse().ok()?;
    (1..=10).contains(&value).then_some(value)
}

/// Extracts every matchup on one hero page.
///
/// The page renders each card twice (a mobile and a desktop copy), so results
/// are deduplicated on `(hero, vs)`; the first copy wins.
pub fn parse_page(html: &str) -> Result<Vec<RawMatchup>> {
    let selectors = Selectors::new()?;
    let document = Html::parse_document(html);

    let mut out: Vec<RawMatchup> = Vec::new();

    for article in document.select(&selectors.article) {
        let Some(link) = article.select(&selectors.detail_link).next() else {
            continue;
        };
        let Some((hero, vs)) = link.value().attr("href").and_then(parse_detail_href) else {
            continue;
        };
        if out.iter().any(|m| m.hero == hero && m.vs == vs) {
            continue;
        }

        let difficulty = article
            .select(&selectors.badge)
            .find_map(|el| parse_difficulty(&el.text().collect::<String>()));

        // `scraper` decodes HTML entities, so apostrophes arrive as real
        // characters. The bullet glyph is decoration and is dropped.
        let reason = link
            .select(&selectors.reason)
            .map(|li| li.text().collect::<String>().replace('\u{30fb}', ""))
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .find(|text| !text.is_empty())
            .unwrap_or_default();

        out.push(RawMatchup {
            hero,
            vs,
            difficulty,
            reason,
        });
    }

    Ok(out)
}

/// Fetches and parses one page per hero.
///
/// Candidate slugs are tried in order and judged by whether the page actually
/// yields matchups — the site answers an unknown hero with HTTP 200 and a
/// "Hero Not Found" body, so the status code proves nothing.
///
/// A hero that cannot be resolved is reported and skipped rather than aborting
/// the run: a partial matrix is far more useful than none, and the gap shows up
/// in the coverage report at the end of the ingest.
pub async fn scrape(fetcher: &mut Fetcher, hero_keys: &[String]) -> Result<Vec<RawMatchup>> {
    let mut out = Vec::new();
    let mut unresolved: Vec<&str> = Vec::new();

    for (i, key) in hero_keys.iter().enumerate() {
        // Cache under our key, not theirs, so the cache stays stable if an
        // override is added or removed later.
        let cache_slug = format!("counterpickgg-{key}.html");

        if fetcher.is_missing(&cache_slug).await {
            unresolved.push(key);
            continue;
        }

        let mut resolved = false;

        for candidate in crate::slugs::counterpickgg(key) {
            let url = format!("{BASE}/heroes/{candidate}");

            let body = match fetcher.get(&url, &cache_slug).await {
                Ok(body) => body,
                Err(err) => {
                    eprintln!("  warn: counterpickgg {key} ({candidate}): {err:#}");
                    continue;
                }
            };

            let matchups = parse_page(&body)
                .with_context(|| format!("parsing the counterpickgg page for {key}"))?;

            if matchups.is_empty() {
                // Wrong slug, or the markup moved. Try the next candidate, and
                // discard the useless cache entry so a retry is not poisoned.
                fetcher.forget(&cache_slug).await;
                continue;
            }

            // Guard against a layout change that would transpose the matrix.
            if let Some(wrong) = matchups.iter().find(|m| m.hero != candidate) {
                anyhow::bail!(
                    "counterpickgg page for {candidate} yielded matchups for {:?} - the URL scheme changed",
                    wrong.hero
                );
            }

            // Translate the site's spelling back into our keys on both sides.
            out.extend(matchups.into_iter().map(|m| RawMatchup {
                hero: key.clone(),
                vs: crate::slugs::counterpickgg_to_ours(&m.vs),
                ..m
            }));
            resolved = true;
            break;
        }

        if !resolved {
            // Every candidate failed, so record the absence and stop paying for
            // it on future runs. `--refresh` clears this.
            fetcher.mark_missing(&cache_slug).await;
            unresolved.push(key);
        }

        if (i + 1) % 10 == 0 {
            eprintln!("  counterpickgg: {}/{} heroes", i + 1, hero_keys.len());
        }
    }

    if !unresolved.is_empty() {
        eprintln!(
            "  warn: counterpickgg has no page for: {}",
            unresolved.join(", ")
        );
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from the real Reinhardt page, including the duplicated card the
    /// responsive layout emits.
    const FIXTURE: &str = r##"
      <div>
        <article>
          <header>
            <a href="/heroes/pharah"><h3>Pharah</h3></a>
            <div class="badge">9/10</div>
          </header>
          <a aria-label="View Pharah matchup details" href="/heroes/reinhardt/vs/pharah">
            <ul><li><span>・</span><span><span>Reinhardt is very weak against airborne targets.</span></span></li></ul>
          </a>
        </article>
        <article>
          <header>
            <a href="/heroes/pharah"><h3>Pharah</h3></a>
            <div class="badge">9/10</div>
          </header>
          <a aria-label="View Pharah matchup details" href="/heroes/reinhardt/vs/pharah">
            <ul><li><span>・</span><span><span>Reinhardt is very weak against airborne targets.</span></span></li></ul>
          </a>
        </article>
        <article>
          <header>
            <a href="/heroes/brigitte"><h3>Brigitte</h3></a>
            <div class="badge">2/10</div>
          </header>
          <a aria-label="View Brigitte matchup details" href="/heroes/reinhardt/vs/brigitte">
            <ul><li><span>・</span><span><span>Reinhardt&#x27;s primary attack deals very high damage.</span></span></li></ul>
          </a>
        </article>
        <article>
          <header>
            <a href="/heroes/anran"><h3>Anran</h3></a>
          </header>
          <a aria-label="View Anran matchup details" href="/heroes/reinhardt/vs/anran">
            <ul></ul>
          </a>
        </article>
        <article><header><h2>Not a matchup card</h2></header></article>
      </div>
    "##;

    #[test]
    fn parses_direction_score_and_reason() {
        let parsed = parse_page(FIXTURE).expect("fixture parses");

        assert_eq!(
            parsed.len(),
            3,
            "duplicate card collapsed, header card ignored"
        );

        let pharah = &parsed[0];
        assert_eq!(pharah.hero, "reinhardt");
        assert_eq!(pharah.vs, "pharah");
        assert_eq!(pharah.difficulty, Some(9));
        assert_eq!(
            pharah.reason,
            "Reinhardt is very weak against airborne targets."
        );
    }

    #[test]
    fn html_entities_are_decoded() {
        let parsed = parse_page(FIXTURE).expect("fixture parses");
        let brigitte = parsed
            .iter()
            .find(|m| m.vs == "brigitte")
            .expect("brigitte card present");
        assert_eq!(brigitte.difficulty, Some(2));
        assert!(
            brigitte.reason.starts_with("Reinhardt's"),
            "got {:?}",
            brigitte.reason
        );
    }

    #[test]
    fn unrated_heroes_survive_without_a_score() {
        let parsed = parse_page(FIXTURE).expect("fixture parses");
        let anran = parsed
            .iter()
            .find(|m| m.vs == "anran")
            .expect("anran card present");
        assert_eq!(anran.difficulty, None);
        assert!(anran.reason.is_empty());
    }

    #[test]
    fn difficulty_badges_are_strictly_out_of_ten() {
        assert_eq!(parse_difficulty("9/10"), Some(9));
        assert_eq!(parse_difficulty(" 10/10 "), Some(10));
        assert_eq!(parse_difficulty("1/10"), Some(1));
        // Not ratings.
        assert_eq!(parse_difficulty("55%"), None);
        assert_eq!(parse_difficulty("3/5"), None);
        assert_eq!(parse_difficulty("0/10"), None);
        assert_eq!(parse_difficulty("11/10"), None);
        assert_eq!(parse_difficulty(""), None);
    }

    /// Trimmed from the real index table, keeping the doubled win-rate cell.
    const INDEX_FIXTURE: &str = r##"
      <table>
        <tr><th>HERO</th><th>WIN%</th><th>PICK%</th><th>BEST MAPS</th></tr>
        <tr>
          <td><a href="/heroes/torbjorn">Torbjörn</a></td>
          <td>58 % 58 %</td>
          <td>5 %</td>
          <td>
            <a href="/maps?selected=eichenwalde#hero-rankings">Eichenwalde</a>
            <a href="/maps?selected=havana#hero-rankings">Havana</a>
            <a href="/maps?selected=paraiso#hero-rankings">Paraíso</a>
          </td>
        </tr>
        <tr>
          <td><a href="/heroes/freya">Freja</a></td>
          <td>49 % 49 %</td>
          <td>2 %</td>
          <td></td>
        </tr>
      </table>
    "##;

    #[test]
    fn the_index_yields_rates_and_best_maps() {
        let stats = parse_index(INDEX_FIXTURE).expect("fixture parses");
        assert_eq!(stats.len(), 2, "the header row is skipped");

        let torb = &stats[0];
        assert_eq!(torb.hero, "torbjorn");
        assert_eq!(torb.win_rate, 58.0);
        assert_eq!(torb.pick_rate, 5.0);
        assert_eq!(torb.best_maps, vec!["eichenwalde", "havana", "paraiso"]);
    }

    #[test]
    fn index_rows_are_translated_into_our_keys() {
        let stats = parse_index(INDEX_FIXTURE).expect("fixture parses");
        assert_eq!(
            stats[1].hero, "freja",
            "the site's `freya` maps back to ours"
        );
        assert!(stats[1].best_maps.is_empty());
    }

    #[test]
    fn doubled_percentage_cells_read_once() {
        assert_eq!(first_percentage("58 % 58 %"), Some(58.0));
        assert_eq!(first_percentage("5 %"), Some(5.0));
        assert_eq!(first_percentage("52.4%"), Some(52.4));
        assert_eq!(first_percentage("-"), None);
        assert_eq!(first_percentage(""), None);
    }

    #[test]
    fn map_hrefs_yield_keys() {
        assert_eq!(
            parse_map_href("/maps?selected=circuit-royal#hero-rankings"),
            Some("circuit-royal".to_owned())
        );
        assert_eq!(parse_map_href("/maps"), None);
        assert_eq!(parse_map_href("/heroes/ana"), None);
    }

    #[test]
    fn detail_hrefs_yield_both_sides() {
        assert_eq!(
            parse_detail_href("/heroes/reinhardt/vs/pharah"),
            Some(("reinhardt".to_owned(), "pharah".to_owned()))
        );
        assert_eq!(
            parse_detail_href("/heroes/wrecking-ball/vs/soldier-76?x=1"),
            Some(("wrecking-ball".to_owned(), "soldier-76".to_owned()))
        );
        assert_eq!(parse_detail_href("/heroes/reinhardt"), None);
        assert_eq!(parse_detail_href("/about"), None);
    }
}
