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
//! into a value. It covers roughly a quarter of each row and is blended in at a
//! low weight, mostly as an independent check on the two opinion-based sources.
//!
//! The **stats** pages carry the same judgement as numbers, and that is where
//! most of this source's readings now come from: ten published counter ratings
//! per hero, each with a duel count and usually an estimated win-probability
//! swing, read by [`parse_matchup_ratings`]. Where one exists it is the reading
//! and the rank position is never consulted; [`hero_values`] is where that is
//! decided and why. The ranked lists still matter, because the two documents do
//! not rank a hero's matchups the same way — but they now cover very little: 530
//! pairs carry a published counter rating, another 641 carry a published
//! win-probability swing off the counters page, and only the remaining 48 exist
//! as a position alone. Before the swings were read that last figure was 265.
//!
//! Those pages belong to the `strength` step by every other measure, and are read
//! here anyway because the numbers on them land in `matchups.toml` and only the
//! `counters` step writes that file. Two steps writing one file is the hazard the
//! per-step split exists to prevent. The cost is that a matchup reading is only
//! as fresh as whichever of the two last refreshed the page it came off.
//!
//! The stats pages carry one more thing that is server-rendered: a "Performance
//! by Rank" table with a win rate, a pick rate and a **published match count**
//! for each of the eight divisions. That is the only per-rung sample size either
//! source publishes, and [`parse_rank_breakdown`] reads it off the same document
//! [`parse_win_rate`] already fetches, so it costs nothing. Note that the site's
//! rank *filter* is not URL-addressable — `?division=`, `?rank=` and `?tier=` all
//! return the same All-Ranks page — and neither the counters nor the duos pages
//! carry a per-division breakdown at all, which is why only strength is sliced
//! by rank anywhere in this project.
//!
//! The site's `best-duos` pages are the other half of what it is worth here,
//! and the only reason `synergy.toml` is not still empty. Those *are* rendered
//! server-side, but as markup rather than JSON-LD, so [`parse_duos`] reads the
//! anchors instead. See [`scrape_duos`] for what the numbers mean and what they
//! do not cover.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use overwatch_core::Rank;
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

/// Where the rank interpolation ends, at the last entry of a list.
///
/// A floor, and no longer a claim about that entry. The lists are truncated to a
/// hero's most lopsided matchups, so this used to read "even the last one is a
/// real edge" — but the ratings the site publishes for those same pairs run to a
/// median of 7.0 on its own scale, a modest edge, so calling the tail of a top
/// ten strong overstated it. Where a list has published ratings the anchor now
/// comes from those instead, and this only says where the decay stops.
///
/// It also matters far less than it did. Reading the swing the counters page
/// states in its row tooltips ([`parse_swings`]) took the interpolation from 265
/// of counterwatch's readings down to 48 — a published figure now covers almost
/// everything a rank position used to have to guess at.
const TAIL_MAGNITUDE: f32 = 25.0;

/// Converts a 1-based rank into a value on -100..=100.
///
/// `top` is the magnitude given to rank 1, decaying linearly to
/// [`TAIL_MAGNITUDE`] at `len`. `top` can come from a published rating rather
/// than from [`top_magnitude`]'s stretched band, and those are routinely smaller
/// than the tail — hence the `min`, without which a list anchored at 20 would
/// read *stronger* the further down it you read.
fn rank_to_value(position: usize, len: usize, top: f32) -> i8 {
    if position == 0 || len == 0 {
        return 0;
    }
    let tail = TAIL_MAGNITUDE.min(top);
    let magnitude = if len == 1 {
        top
    } else {
        let t = (position - 1) as f32 / (len - 1) as f32;
        top + (tail - top) * t
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

/// The band a published counter rating is stretched over to reach -100..=100.
///
/// Fixed rather than fitted, for the reason [`SYNERGY_CEILING`] gives. Across the
/// 530 ratings published today the magnitude runs min 2.9, median 7.0, p90 11.3,
/// max 41.5 — so ±25 puts the middle half of them at 23..36, the same
/// modest-edge band counterpickgg spends its ±25 step on, and clamps exactly four
/// rows: both directions of D.Mon/Torbjörn at 41.5 and of D.Mon/Illari at 32.0.
///
/// Deliberately not [`TAIL_MAGNITUDE`], which is also 25 and means something
/// else entirely: that one is a magnitude on our scale, this is a ceiling on the
/// site's.
const COUNTER_CEILING: f32 = 25.0;

/// The band a published win-probability swing is stretched over to reach
/// -100..=100, and the counterpart to [`COUNTER_CEILING`] on the other document.
///
/// Not fitted to taste: the counters page states a swing for 869 pairs and the
/// stats page publishes a rating for 530, and 225 of them are the same pair. Over
/// those 225, `committed value / swing` has a median of 4.333 and a mean of 4.327
/// with a standard deviation of 0.240, running 3.12 to 5.00 — so the two documents
/// are quoting one quantity at two precisions, and 100/23 = 4.35 lands on it.
///
/// It clamps two of the 869: the single swing of 27 and the single swing of 32.
/// Everything else lands inside the scale, and the distribution is heavily
/// bottom-weighted — 745 of 869 sit at a swing of 6 or less, which is ±26 or less
/// on our scale. That is why reading this column adds no pressure at the rail:
/// **none** of the 644 pairs it newly rates reaches ±75, let alone the ±90 that
/// `the_extreme_end_of_the_matchup_scale_stays_rare` counts.
const SWING_CEILING: f32 = 23.0;

/// One published win-probability swing, from the perspective of the hero whose
/// page it is — so `value` is always **unfavourable** for that hero. See
/// [`parse_swings`] for why that is a property of the source and not a bug.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchupSwing {
    /// The opponent, in our hero keys.
    pub vs: String,
    /// The site's own figure, in percentage points of win probability.
    pub swing: u8,
    /// On the canonical -100..=100 scale, from the page subject's side.
    pub value: i8,
}

/// Reads the win-probability swing the counters page states in its row tooltips.
///
/// **This is a different document from [`parse_matchup_ratings`] and a much
/// larger one.** The stats page publishes a rating for a hero's ten most lopsided
/// matchups, 530 rows across the roster. The counters page carries a tooltip on
/// *every* row — 52 per page, 2,756 in all, the complete directed matrix — and
/// 869 of those state a counter impact. Until this existed the pipeline read those
/// pages only for rank *position*, so 644 published readings were being thrown
/// away and replaced with an interpolation.
///
/// Three things make the direction safe, and it needs all three because the
/// phrasing gives nothing away:
///
/// - **The subject is never the leader.** All 2,756 tooltips read "{opponent}
///   takes N% of the kills traded with {subject}", so the positional rule
///   `parse_matchup_ratings` relies on is constant here and carries no signal.
/// - **The swing always favours the leader.** Of the 225 pairs the stats page also
///   rates, the committed value is positive for the leader in 225. Of the 445
///   whose mirror carries a committed value, the mirror is negative in 445. Both
///   are exact, so this is not a tendency.
/// - **A pair is stated once.** Not one of the 869 appears on both heroes' pages,
///   so the source can never contradict itself here and no tie-break is needed.
///
/// Checked against the one source that shares no code with this: counterpickgg
/// agrees the leader is favoured on 69.5% of the 606 overlapping pairs, and the
/// agreement rises with the swing — 62.7% at a swing of 1-2, 68.1% at 3-5, 79.3%
/// at 6-10, 81.0% above that. That is the shape of two instruments disagreeing
/// about soft matchups and converging on hard ones, and 70% is the rate these two
/// sources already reach across the committed matrix. A parsing error would look
/// flat, or worse than chance.
///
/// Selecting on the tooltip rather than a class, for the reason
/// `parse_matchup_ratings` gives: a restyle must not be able to change what a
/// number means. Rows without a counter-impact clause are skipped rather than
/// read off the kill-share beside it — that figure is 47% explained by *which
/// hero it is* (Mercy takes 10% of kill trades against the whole roster, Roadhog
/// 68%), and against this source's own rating it manages r = +0.375 even after
/// normalising each hero out. It measures how killy a hero is, not who beats whom.
pub fn parse_swings(html: &str, subject_name: &str) -> Result<Vec<MatchupSwing>> {
    let anchor = Selector::parse(r#"a[title*=" of the kills traded with "]"#)
        .map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let portrait =
        Selector::parse("img[alt]").map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let document = Html::parse_document(html);

    let mut out: Vec<MatchupSwing> = Vec::new();
    for row in document.select(&anchor) {
        let Some(title) = row.value().attr("title") else {
            continue;
        };
        let Some(swing) = swing_points(title) else {
            continue;
        };

        let Some(slug) = row
            .value()
            .attr("href")
            .and_then(|href| href.rsplit('/').next())
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        let vs = crate::slugs::counterwatch_to_ours(slug);

        // Split on the verb, never on punctuation — `D.Mon` and `D.Va` would
        // truncate to `D`. The same rule as `parse_matchup_ratings`.
        let leader = title.split(" takes ").next().unwrap_or_default().trim();
        let Some(opponent_name) = row
            .select(&portrait)
            .next()
            .and_then(|img| img.value().attr("alt"))
        else {
            continue;
        };

        // The invariant this document holds and the other one does not. If the
        // subject ever leads a sentence here, the phrasing has changed and the
        // direction rule above is no longer safe to apply.
        anyhow::ensure!(
            leader != subject_name,
            "counterwatch now names {subject_name} first in its own counters row against              {vs}; the swing direction can no longer be taken from the leader"
        );
        if leader != opponent_name {
            continue;
        }

        // Unfavourable for the subject, always: the swing favours the leader and
        // the leader is always the other hero.
        let value = overwatch_core::normalize(-f32::from(swing), -SWING_CEILING, SWING_CEILING);

        if out.iter().any(|row| row.vs == vs) {
            continue;
        }
        out.push(MatchupSwing { vs, swing, value });
    }

    Ok(out)
}

/// The `Counter impact: an estimated +N% swing` figure, or `None` on the rows
/// that state kill shares and stop there.
///
/// Anchored on the label rather than on a bare percentage, because the sentence
/// before it carries two other percentages — the kill share and the death share —
/// and either would parse.
fn swing_points(title: &str) -> Option<u8> {
    let tail = title.split("Counter impact:").nth(1)?;
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// One published counter rating, from the perspective of the hero whose page it
/// is.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchupRating {
    /// The opponent, in our hero keys.
    pub vs: String,
    /// The rating on the canonical -100..=100 scale.
    pub value: i8,
    /// The site's own figure, signed by us. Reported rather than committed.
    pub rating: f32,
    /// Estimated win-probability swing, where the page states one — 450 of the
    /// 530 rows do, and the other 80 omit both the figure and the clause of the
    /// tooltip that would have carried it.
    pub swing: Option<u8>,
    /// Players who contributed duel data for this matchup.
    pub duels: u32,
}

struct RatingSelectors {
    anchor: Selector,
    prose: Selector,
    portrait: Selector,
    favourable: Selector,
    unfavourable: Selector,
    confidence: Selector,
}

impl RatingSelectors {
    fn new() -> Result<Self> {
        let sel = |s: &str| {
            Selector::parse(s).map_err(|e| anyhow::anyhow!("invalid selector {s:?}: {e}"))
        };
        Ok(Self {
            anchor: sel(r#"a[href^="/stats/overwatch/heroes/"]"#)?,
            prose: sel(r#"div[title*=" of the kills traded with "]"#)?,
            portrait: sel("img[alt]")?,
            favourable: sel(".text-green-300")?,
            unfavourable: sel(".text-red-300")?,
            confidence: sel("abbr[title]")?,
        })
    }
}

/// Reads the counter ratings a hero's stats page publishes for its ten most
/// lopsided matchups.
///
/// This is the one thing on the site that is a *measurement* of a pair rather
/// than a position in a list. [`rank_to_value`] exists because the counters page
/// publishes one number and ten ranks; this page publishes ten numbers.
///
/// **The prose is both the row selector and the direction.** Three other things
/// on the page look like a matchup row and are not: the duo rows share this
/// anchor's whole class list and differ only in having no `title` on the rating
/// block; the "heroes to ban against" rows restate these same ratings on five
/// more anchors with no prose and no colour; and the map rows link to
/// `/maps/`, so they are not hero anchors at all. Selecting on the tooltip is
/// what tells the four apart — it appears exactly ten times per page, on all 53.
///
/// The direction rule is **positional, not semantic**: whichever hero the
/// sentence names first is the one the row is favourable for. It is tempting to
/// read the percentage instead, and wrong — Ana's page carries *"Ana takes 28% of
/// the kills traded with Roadhog"* among her **easiest** matchups, because the
/// rating also weighs the teamfight side and strips out each hero's general
/// strength. Only 176 of the 265 favourable rows have the subject taking more
/// than half the kill trades.
///
/// The colour class is then required to agree, and a disagreement stops the
/// parse rather than being absorbed. Both cues hold for all 530 rows today, so
/// one of them moving is a change at the site and not a row to drop: reading the
/// class alone would let a restyle silently invert every reading, and reading the
/// prose alone would let a reworded sentence do the same.
///
/// Every rating is printed with a leading `+` — 530 of 530 — so the sign in the
/// text carries no information and only the magnitude is read from it.
pub fn parse_matchup_ratings(html: &str, subject_name: &str) -> Result<Vec<MatchupRating>> {
    let sel = RatingSelectors::new()?;
    let document = Html::parse_document(html);

    let mut out: Vec<MatchupRating> = Vec::new();
    for anchor in document.select(&sel.anchor) {
        let Some(prose) = anchor.select(&sel.prose).next() else {
            continue;
        };
        let Some(title) = prose.value().attr("title") else {
            continue;
        };
        let Some(slug) = anchor
            .value()
            .attr("href")
            .and_then(|href| href.rsplit('/').next())
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        let vs = crate::slugs::counterwatch_to_ours(slug);

        // The opponent's own spelling, off the portrait rather than a styled
        // span, so the comparison below does not depend on a class name.
        let Some(opponent_name) = anchor
            .select(&sel.portrait)
            .next()
            .and_then(|img| img.value().attr("alt"))
        else {
            continue;
        };

        // Split on the verb, never on punctuation. Terminating the name at a `.`
        // or a `,` truncates `D.Mon` and `D.Va` to `D`, which matches neither
        // hero and silently drops all ten rows on their pages.
        let leader = title.split(" takes ").next().unwrap_or_default().trim();
        let favourable = if leader == subject_name {
            true
        } else if leader == opponent_name {
            false
        } else {
            continue;
        };

        let styled_favourable = match (
            prose.select(&sel.favourable).next(),
            prose.select(&sel.unfavourable).next(),
        ) {
            (Some(good), None) => (true, good),
            (None, Some(bad)) => (false, bad),
            // Neither colour, or both: not a row this parser understands.
            _ => continue,
        };
        let (styled, rating_element) = styled_favourable;
        anyhow::ensure!(
            styled == favourable,
            "counterwatch contradicts itself about {subject_name} vs {vs}: the tooltip \
             names {leader:?} first while the rating is styled the other way round"
        );

        let Some(magnitude) = first_rating(&rating_element.text().collect::<String>()) else {
            continue;
        };
        let rating = if favourable {
            magnitude.abs()
        } else {
            -magnitude.abs()
        };

        // `first_percentage` wants the sigil, so it walks past the bare rating
        // and lands on the `≈ +17%` swing, and returns `None` on the 80 rows
        // that do not state one.
        let text: String = prose.text().collect::<Vec<_>>().join(" ");
        let swing = first_percentage(&text).map(|swing| swing.abs().round() as u8);

        let Some(duels) = prose
            .select(&sel.confidence)
            .next()
            .and_then(|el| el.value().attr("title"))
            .and_then(duel_count)
        else {
            continue;
        };

        // No page lists a pair twice today; this follows `parse_duos` in not
        // relying on that.
        if out.iter().any(|row| row.vs == vs) {
            continue;
        }
        out.push(MatchupRating {
            value: overwatch_core::normalize(rating, -COUNTER_CEILING, COUNTER_CEILING),
            vs,
            rating,
            swing,
            duels,
        });
    }

    Ok(out)
}

/// Reads the sample size out of a confidence tooltip, e.g.
/// `"High confidence: 1,010 players contributed duel data for this matchup."`.
///
/// Deliberately not [`first_rating`] or either percentage reader: this figure
/// carries thousands separators and no sign and no sigil, so all three of them
/// return `None` on every row. The trailing phrase is required because the same
/// page states a confidence interval in the same shape, over `tracked matches`.
fn duel_count(title: &str) -> Option<u32> {
    let (head, _) = title.split_once(" players contributed duel data")?;
    let reversed: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    reversed.chars().rev().collect::<String>().parse().ok()
}

/// The anchor magnitude for one ranked list, preferring what the site measured
/// over what a rank position implies.
///
/// The earliest position carrying a published rating is the closest thing to a
/// rank-1 reading, because the list is ordered by severity. `fallback` is what a
/// list with no published rating anywhere on it gets.
fn list_anchor(
    key: &str,
    list: &[String],
    ratings: &HashMap<(String, String), MatchupRating>,
    names: &HashMap<String, String>,
    fallback: f32,
) -> f32 {
    list.iter()
        .filter_map(|name| names.get(name))
        .find_map(|opponent| ratings.get(&(key.to_owned(), opponent.clone())))
        .map(|rating| f32::from(rating.value).abs())
        .unwrap_or(fallback)
}

/// Turns one hero's counters page into `(hero, opponent) -> value` readings,
/// with the published ratings winning wherever they exist.
///
/// Measurements are seeded **first** and never overwritten, so "a number beats a
/// rank position" is structural rather than a consequence of which loop runs.
/// That is also what lets a pair the counters page never ranked reach the matrix
/// at all: the two documents rank a hero's matchups differently, so the stats
/// page rates ten pairs that are not the same ten.
///
/// Each list is anchored on its own numbers. Sharing one anchor is what gave a
/// hero's *easiest* matchups the magnitude of its hardest one — the punish list
/// has no published rating of its own on the counters page, so it was handed
/// `countered_by[0]`'s and then decayed across three positions instead of ten.
///
/// A published rating keeps its own sign and is never negated. Where it
/// contradicts the list the counters page filed the pair under, the number wins
/// and the caller counts it, because the site disagreeing with itself across two
/// of its own pages is worth seeing rather than smoothing.
fn hero_values(
    key: &str,
    parsed: &HeroCounters,
    swings: &[MatchupSwing],
    ratings: &HashMap<(String, String), MatchupRating>,
    names: &HashMap<String, String>,
) -> HashMap<(String, String), i8> {
    let mut out: HashMap<(String, String), i8> = HashMap::new();

    for ((hero, opponent), rating) in ratings {
        if hero == key {
            out.insert((hero.clone(), opponent.clone()), rating.value);
        }
    }

    // Second tier, between the two documents' own precisions. The stats page
    // quotes a rating to one decimal for ten pairs; the counters page quotes a
    // whole-number swing for as many as 52, and the two agree to r = +0.978 where
    // they overlap. So the finer figure is seeded first and kept, and this fills
    // the rest — 644 pairs that were reaching the matrix as a rank interpolation
    // or not at all.
    for swing in swings {
        if swing.vs == key {
            continue;
        }
        out.entry((key.to_owned(), swing.vs.clone()))
            .or_insert(swing.value);
    }

    let countered_top = list_anchor(
        key,
        &parsed.countered_by,
        ratings,
        names,
        top_magnitude(parsed.top_rating),
    );
    let punish_top = list_anchor(key, &parsed.punishes, ratings, names, top_magnitude(None));

    let len = parsed.countered_by.len();
    for (idx, opponent_name) in parsed.countered_by.iter().enumerate() {
        let Some(opponent) = names.get(opponent_name) else {
            continue;
        };
        if opponent == key {
            continue;
        }
        // "Countered by X at rank r" is a negative value for this hero.
        out.entry((key.to_owned(), opponent.clone()))
            .or_insert_with(|| -rank_to_value(idx + 1, len, countered_top));
    }

    let punish_len = parsed.punishes.len();
    for (idx, opponent_name) in parsed.punishes.iter().enumerate() {
        let Some(opponent) = names.get(opponent_name) else {
            continue;
        };
        if opponent == key {
            continue;
        }
        out.entry((key.to_owned(), opponent.clone()))
            .or_insert_with(|| rank_to_value(idx + 1, punish_len, punish_top));
    }

    out
}

/// Fetches every hero page and turns its rankings into matchup values.
///
/// Returns `(hero, opponent) -> value` from `hero`'s perspective. `ratings` is
/// whatever [`scrape_matchup_ratings`] read off the stats pages: wherever it
/// holds a number for a pair, that number *is* the reading and the pair's rank
/// position is never consulted. [`hero_values`] is where that is decided, and
/// carries the reasoning.
pub async fn scrape(
    fetcher: &mut Fetcher,
    hero_keys: &[String],
    names: &HashMap<String, String>,
    ratings: &HashMap<(String, String), MatchupRating>,
) -> Result<HashMap<(String, String), i8>> {
    let mut out = HashMap::new();

    let mut unresolved: Vec<&str> = Vec::new();
    let mut contradicted = 0usize;
    let mut swung: HashSet<(String, String)> = HashSet::new();

    for (i, key) in hero_keys.iter().enumerate() {
        let cache_slug = format!("counterwatch-{key}.html");

        let mut parsed = HeroCounters::default();
        let mut swings: Vec<MatchupSwing> = Vec::new();
        if fetcher.is_missing(&cache_slug).await {
            unresolved.push(key);
        } else {
            for candidate in crate::slugs::counterwatch(key) {
                let url = format!("{BASE}/stats/overwatch/counters/{candidate}");

                let Ok(body) = fetcher.get(&url, &cache_slug).await else {
                    continue;
                };

                parsed = parse_page(&body)
                    .with_context(|| format!("parsing the counterwatch page for {key}"))?;
                // Off the same document, and free: the ranking comes out of the
                // page's JSON-LD, the swings out of its DOM.
                let subject_name = names
                    .iter()
                    .find(|(_, our_key)| *our_key == key)
                    .map(|(name, _)| name.as_str())
                    .unwrap_or_default();
                swings = parse_swings(&body, subject_name)
                    .with_context(|| format!("parsing the counterwatch swings for {key}"))?;
                if !parsed.countered_by.is_empty() || !parsed.punishes.is_empty() {
                    break;
                }
                fetcher.forget(&cache_slug).await;
            }

            if parsed.countered_by.is_empty() && parsed.punishes.is_empty() {
                // Record the absence so this hero stops costing a request per run.
                fetcher.mark_missing(&cache_slug).await;
                unresolved.push(key);
            }
        }

        // The site ranks a hero's matchups differently on its two pages, so a
        // published rating can land on the opposite list from the one that
        // ranked it. The number wins; this counts how often it has to.
        for (list, ranked_against) in [(&parsed.countered_by, true), (&parsed.punishes, false)] {
            for opponent_name in list {
                let Some(opponent) = names.get(opponent_name) else {
                    continue;
                };
                let Some(rating) = ratings.get(&(key.clone(), opponent.clone())) else {
                    continue;
                };
                if rating.value != 0 && (rating.value < 0) != ranked_against {
                    contradicted += 1;
                }
            }
        }

        // Counted before the merge, because afterwards a swing and a rank
        // interpolation are indistinguishable in `out` — and reporting a published
        // reading as a guess is the kind of quiet wrongness this tool exists to
        // avoid.
        for swing in &swings {
            if swing.vs != *key {
                swung.insert((key.clone(), swing.vs.clone()));
            }
        }

        // Merged even for a hero with no counters page at all: `hero_values` over
        // an empty ranking is exactly the rows its stats page published, and
        // dropping those would make a missing document cost readings it has.
        out.extend(hero_values(key, &parsed, &swings, ratings, names));

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

    let published = out.keys().filter(|key| ratings.contains_key(*key)).count();
    let swung = out
        .keys()
        .filter(|key| !ratings.contains_key(*key) && swung.contains(*key))
        .count();
    eprintln!(
        "  counterwatch: {} readings - {published} rated, {swung} from a published swing, \
         {} from a rank position\n\
         \x20   {contradicted} published rating(s) contradict the list that ranked them",
        out.len(),
        out.len() - published - swung,
    );

    Ok(out)
}

/// Fetches every hero's stats page for the counter ratings it publishes.
///
/// Reads the same document as [`scrape_win_rates`], under the same cache slug, so
/// on a warm cache this costs nothing at all and `--refresh` grows the counters
/// step from 107 requests to 160. The consequence worth knowing is that matchup
/// readings are now only as fresh as whichever of `counters` or `strength` last
/// refreshed that page.
///
/// Two things it deliberately does not do, both because the slug is shared:
///
/// - it never calls `mark_missing`, because that sentinel would make the
///   `strength` step skip a document it can read perfectly well — a page with no
///   matchup rows can still carry a win rate;
/// - it never calls `forget`, unlike its two sibling scrapers. There the point is
///   to let a second URL spelling actually re-fetch, but every counterwatch slug
///   resolves on the first candidate today, and against that the downside is a
///   markup change wiping a 53-page cache that another step depends on.
pub async fn scrape_matchup_ratings(
    fetcher: &mut Fetcher,
    hero_keys: &[String],
    subject_names: &HashMap<String, String>,
) -> Result<HashMap<(String, String), MatchupRating>> {
    let known: HashSet<&str> = hero_keys.iter().map(String::as_str).collect();
    let mut out = HashMap::new();
    let mut unreadable: Vec<&str> = Vec::new();

    for (i, key) in hero_keys.iter().enumerate() {
        let cache_slug = format!("counterwatch-stats-{key}.html");

        if fetcher.is_missing(&cache_slug).await {
            unreadable.push(key);
            continue;
        }
        let Some(subject_name) = subject_names.get(key) else {
            unreadable.push(key);
            continue;
        };

        let mut rows = Vec::new();
        for candidate in crate::slugs::counterwatch(key) {
            let url = format!("{BASE}/stats/overwatch/heroes/{candidate}");
            let Ok(body) = fetcher.get(&url, &cache_slug).await else {
                continue;
            };
            rows = parse_matchup_ratings(&body, subject_name)
                .with_context(|| format!("reading the counter ratings on {key}'s stats page"))?;
            break;
        }

        if rows.is_empty() {
            unreadable.push(key);
            continue;
        }

        for row in rows {
            // `counterwatch_to_ours` hands an unknown slug straight back, so the
            // roster is what decides whether a row names a hero we draft.
            if row.vs == *key || !known.contains(row.vs.as_str()) {
                continue;
            }
            out.insert((key.clone(), row.vs.clone()), row);
        }

        if (i + 1) % 10 == 0 {
            eprintln!(
                "  counterwatch ratings: {}/{} heroes",
                i + 1,
                hero_keys.len()
            );
        }
    }

    if !unreadable.is_empty() {
        eprintln!(
            "  warn: counterwatch publishes no counter ratings for: {}",
            unreadable.join(", ")
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

/// One row of a hero's "Performance by Rank" table.
#[derive(Debug, Clone, PartialEq)]
pub struct RankRow {
    pub rank: Rank,
    pub win_rate: f32,
    /// The site's published sample size for this bucket, and the reason the
    /// table is worth reading at all: it is the only per-rung volume figure
    /// either source publishes, and the thin buckets are very thin. Median
    /// across the roster runs 18,536 at Gold against 263 at Emerald and 353 at
    /// Grandmaster+. [`crate::stats`] weighs the row by it.
    pub matches: u32,
}

/// Everything one stats page is worth: the hero's own win rate, and its curve
/// across the ladder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeroRates {
    /// The all-ranks figure from the JSON-LD sentence — see [`parse_win_rate`].
    ///
    /// The baseline every rung is measured against, and the point of taking it
    /// from here rather than from anywhere else: it is the same instrument and
    /// the same shrinkage regime as the rungs themselves, so the difference
    /// between them is a rank effect and not an instrument disagreement.
    pub all_ranks: f32,
    /// Empty for a page that carries the stats sentence but no breakdown table.
    pub by_rank: Vec<RankRow>,
}

/// Reads the rank breakdown table off a stats page.
///
/// The page ships this twice: as a `<table>` and again inside the Next.js flight
/// payload at sixteen digits of precision. The table is what gets read. The
/// flight payload is a framework private that changes shape on any framework
/// bump and is not what the site promises anyone, while the table is the
/// accessible fallback shipped on purpose — and it is what a human reviewing the
/// cached HTML against a committed number can actually check.
///
/// The extra precision would be a lie in any case. The baseline these rows are
/// subtracted from is [`parse_win_rate`]'s figure, which is one decimal, so
/// pairing it with a sixteen-digit rung would make the *difference* look far
/// more precise than the thing it is measured against.
///
/// The table carries no class or id, so it is found by its caption.
pub fn parse_rank_breakdown(html: &str) -> Result<Vec<RankRow>> {
    let table = Selector::parse("table").map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let caption =
        Selector::parse("caption").map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let row = Selector::parse("tbody tr").map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let cell = Selector::parse("td").map_err(|e| anyhow::anyhow!("invalid selector: {e}"))?;
    let document = Html::parse_document(html);

    let Some(breakdown) = document.select(&table).find(|t| {
        // The caption is split into several text nodes by the `<!-- -->` markers
        // the framework emits, so match on the joined text the way `parse_duos`
        // does rather than on a single node.
        t.select(&caption)
            .any(|c| c.text().collect::<String>().contains("by rank division"))
    }) else {
        return Ok(Vec::new());
    };

    let mut out: Vec<RankRow> = Vec::new();
    for tr in breakdown.select(&row) {
        let cells: Vec<String> = tr
            .select(&cell)
            .map(|td| td.text().collect::<String>().trim().to_owned())
            .collect();
        // division, win rate, pick rate, matches. Pick rate is read past rather
        // than kept: Blizzard publishes the same thing for the whole roster in
        // one request.
        let [division, win_rate, _pick_rate, matches] = cells.as_slice() else {
            continue;
        };
        let Ok(rank) = Rank::parse(division) else {
            continue;
        };
        let Some(win_rate) = trailing_percentage(win_rate) else {
            continue;
        };
        // A count that will not parse drops the row rather than defaulting to
        // zero. Zero would weigh the row out of the blend by accident rather
        // than on purpose, and would look like a measurement of nothing instead
        // of the absence of one.
        let Ok(matches) = matches.replace(',', "").parse::<u32>() else {
            continue;
        };
        out.push(RankRow {
            rank,
            win_rate,
            matches,
        });
    }

    Ok(out)
}

/// Reads an unsigned percentage like `48.5%`.
///
/// Deliberately not [`first_percentage`], which requires a leading `+` or `-`
/// and so returns `None` on every cell in this table. Wiring that one up here
/// yields an empty breakdown with no error at all.
fn trailing_percentage(text: &str) -> Option<f32> {
    let trimmed = text.trim().strip_suffix('%')?;
    trimmed.parse::<f32>().ok()
}

/// Fetches every hero's stats page for its win rate and its rank curve.
///
/// Both come off one document in one pass. Reading the breakdown in a second
/// scrape would either re-fetch all 53 pages on a cold cache or depend on this
/// one having run first — the hidden ordering dependency the per-step ingest
/// split exists to prevent — and would re-parse a 330 KB document a second time
/// for every hero.
///
/// Heroes the site has no page for are simply absent, which the blend reads as
/// "this source has no opinion" rather than as a zero. A page that has the stats
/// sentence but no breakdown table is still usable for the win rate; it just
/// arrives with an empty `by_rank`.
pub async fn scrape_win_rates(
    fetcher: &mut Fetcher,
    hero_keys: &[String],
) -> Result<HashMap<String, HeroRates>> {
    let mut out: HashMap<String, HeroRates> = HashMap::new();
    let mut unresolved: Vec<&str> = Vec::new();
    let mut no_breakdown: Vec<&str> = Vec::new();

    for (i, key) in hero_keys.iter().enumerate() {
        let cache_slug = format!("counterwatch-stats-{key}.html");

        if fetcher.is_missing(&cache_slug).await {
            unresolved.push(key);
            continue;
        }

        let mut rates = None;
        for candidate in crate::slugs::counterwatch(key) {
            let url = format!("{BASE}/stats/overwatch/heroes/{candidate}");

            let Ok(body) = fetcher.get(&url, &cache_slug).await else {
                continue;
            };
            if let Some(all_ranks) = parse_win_rate(&body) {
                rates = Some(HeroRates {
                    all_ranks,
                    by_rank: parse_rank_breakdown(&body).unwrap_or_default(),
                });
                break;
            }
            fetcher.forget(&cache_slug).await;
        }

        match rates {
            Some(rates) => {
                if rates.by_rank.is_empty() {
                    no_breakdown.push(key);
                }
                out.insert(key.clone(), rates);
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
    if !no_breakdown.is_empty() {
        eprintln!(
            "  warn: counterwatch has no rank breakdown for: {}",
            no_breakdown.join(", ")
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

    /// Trimmed from `data/sources/counterwatch-stats-ana.html`, keeping the
    /// `<!-- -->` markers the framework splits the caption with — that is what
    /// makes matching a single text node the wrong thing to do.
    const RANK_TABLE: &str = r#"<html><body><table><caption>Ana<!-- --> win rate and pick rate by rank division</caption>
        <thead><tr><th>Division</th><th>Win rate</th><th>Pick rate</th><th>Matches</th></tr></thead>
        <tbody>
        <tr><td>Bronze</td><td>48.5%</td><td>30.2%</td><td>8,532</td></tr>
        <tr><td>Silver</td><td>48.3%</td><td>38.4%</td><td>35,923</td></tr>
        <tr><td>Gold</td><td>48.2%</td><td>48.5%</td><td>60,524</td></tr>
        <tr><td>Platinum</td><td>48.2%</td><td>60.2%</td><td>68,971</td></tr>
        <tr><td>Emerald</td><td>49.3%</td><td>65.3%</td><td>1,158</td></tr>
        <tr><td>Diamond</td><td>48.8%</td><td>69.9%</td><td>33,198</td></tr>
        <tr><td>Master</td><td>48.4%</td><td>75.6%</td><td>9,830</td></tr>
        <tr><td>Grandmaster+</td><td>49.6%</td><td>73.2%</td><td>1,891</td></tr>
        </tbody></table></body></html>"#;

    #[test]
    fn the_rank_breakdown_is_read_off_the_table_with_its_sample_sizes() {
        let rows = parse_rank_breakdown(RANK_TABLE).expect("parses");

        assert_eq!(rows.len(), 8, "one row per rung of the ladder");
        assert_eq!(
            rows.iter().map(|r| r.rank).collect::<Vec<_>>(),
            Rank::DIVISIONS.to_vec(),
            "and in ladder order"
        );
        assert_eq!(
            rows[0],
            RankRow {
                rank: Rank::Bronze,
                win_rate: 48.5,
                matches: 8_532,
            }
        );
        // The site's own spelling of the top bucket, which folds Champion in.
        assert_eq!(rows[7].rank, Rank::Grandmaster);
        assert_eq!(rows[7].matches, 1_891);
        // The thin bucket the shrinkage weighting exists for.
        assert_eq!(rows[4].matches, 1_158, "Emerald is thin and must say so");
    }

    /// A page that has the stats sentence but no breakdown costs the rank
    /// columns for that hero, not the run.
    #[test]
    fn a_page_without_a_rank_table_yields_no_rows_rather_than_an_error() {
        assert_eq!(
            parse_rank_breakdown("<html><body><p>nothing here</p></body></html>").expect("parses"),
            Vec::new()
        );
        // A table that is not this one must not be mistaken for it.
        let other = "<html><table><caption>Map performance</caption><tbody>\
                     <tr><td>Busan</td><td>47.6%</td><td>1%</td><td>10</td></tr></tbody></table></html>";
        assert_eq!(parse_rank_breakdown(other).expect("parses"), Vec::new());
    }

    /// `first_percentage` requires a leading sign, so wiring it up here would
    /// return an empty breakdown with no error at all — a whole feature silently
    /// switched off. This pins the two apart.
    #[test]
    fn an_unsigned_percentage_needs_its_own_reader() {
        assert_eq!(trailing_percentage("48.5%"), Some(48.5));
        assert_eq!(trailing_percentage(" 49.6% "), Some(49.6));
        assert_eq!(trailing_percentage("48.5"), None, "the sigil is required");
        assert_eq!(
            first_percentage("48.5%"),
            None,
            "which is exactly why this one cannot be reused"
        );
    }

    /// Zero would weigh the row out of the blend by accident rather than on
    /// purpose, and would look like a measurement of nothing rather than the
    /// absence of one.
    #[test]
    fn a_row_with_an_unreadable_sample_size_is_dropped_rather_than_counted_as_none() {
        let broken =
            "<html><table><caption>x win rate and pick rate by rank division</caption><tbody>\
                      <tr><td>Bronze</td><td>48.5%</td><td>30.2%</td><td>—</td></tr>\
                      <tr><td>Silver</td><td>48.3%</td><td>38.4%</td><td>35,923</td></tr>\
                      </tbody></table></html>";
        let rows = parse_rank_breakdown(broken).expect("parses");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rank, Rank::Silver);
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

    /// Trimmed from `data/sources/counterwatch-stats-winston.html`, keeping one
    /// row of every kind the page carries and dropping the portrait URLs and the
    /// confidence dots. `data/sources/` is gitignored, so this is inline for the
    /// same reason `RANK_TABLE` is.
    ///
    /// The Genji row is Ana's real shape moved onto this page: a *favourable* row
    /// where the subject takes well under half the kill trades.
    const RATINGS_FIXTURE: &str = r##"<html><body>
<h3>HARDEST MATCHUPS</h3>
<ul>
<li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/roadhog"><img alt="Roadhog" class="size-9"/><span class="text-sm font-medium text-white truncate">Roadhog</span><div class="shrink-0 flex flex-col items-end gap-1" title="Roadhog takes 80% of the kills traded with Winston, and Winston suffers 58% of the deaths while both are up. The rating strips each hero&#x27;s general strength and leans on the teamfight side when a hero rarely trades kills directly. Counter impact: an estimated +17% swing to an even fight, from duel and teamfight outcomes. Not a win rate."><span class="text-base font-semibold tabular-nums leading-none text-red-300">+19.6</span><span class="text-[10px] tabular-nums leading-none text-muted">≈ +17%</span><abbr title="High confidence: 612 players contributed duel data for this matchup." class="no-underline"><span class="text-xs">High</span></abbr></div></a></div></li>
<li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/dmon"><img alt="D.Mon" class="size-9"/><div class="shrink-0 flex flex-col items-end gap-1" title="D.Mon takes 61% of the kills traded with Winston, and Winston suffers 52% of the deaths while both are up. Counter impact: an estimated +6% swing to an even fight, from duel and teamfight outcomes. Not a win rate."><span class="text-base font-semibold text-red-300">+7.4</span><span class="text-[10px] text-muted">≈ +6%</span><abbr title="Good confidence: 1,010 players contributed duel data for this matchup."><span class="text-xs">Good</span></abbr></div></a></div></li>
</ul>
<h3>EASIEST MATCHUPS</h3>
<ul>
<li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/widowmaker"><img alt="Widowmaker" class="size-9"/><div class="shrink-0 flex flex-col items-end gap-1" title="Winston takes 70% of the kills traded with Widowmaker, and Widowmaker suffers 56% of the deaths while both are up. Counter impact: an estimated +15% swing to an even fight, from duel and teamfight outcomes. Not a win rate."><span class="text-base font-semibold text-green-300">+16.1</span><span class="text-[10px] text-muted">≈ +15%</span><abbr title="High confidence: 577 players contributed duel data for this matchup."><span class="text-xs">High</span></abbr></div></a></div></li>
<li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/genji"><img alt="Genji" class="size-9"/><div class="shrink-0 flex flex-col items-end gap-1" title="Winston takes 28% of the kills traded with Genji, and Genji suffers 44% of the deaths while both are up. Counter impact: an estimated +11% swing to an even fight, from duel and teamfight outcomes. Not a win rate."><span class="text-base font-semibold text-green-300">+11.8</span><span class="text-[10px] text-muted">≈ +11%</span><abbr title="High confidence: 788 players contributed duel data for this matchup."><span class="text-xs">High</span></abbr></div></a></div></li>
<li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/lifeweaver"><img alt="Lifeweaver" class="size-9"/><div class="shrink-0 flex flex-col items-end gap-1" title="Winston takes 77% of the kills traded with Lifeweaver, and Lifeweaver suffers 49% of the deaths while both are up. The rating strips each hero&#x27;s general strength and leans on the teamfight side when a hero rarely trades kills directly."><span class="text-base font-semibold text-green-300">+9.0</span><abbr title="Very high confidence: 881 players contributed duel data for this matchup."><span class="text-xs">Very high</span></abbr></div></a></div></li>
</ul>
<h3>STRONGEST DUOS</h3>
<ul><li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/torbjorn"><img alt="Torbjörn" class="size-9"/><div class="shrink-0 flex flex-col items-end gap-1"><span class="text-base font-semibold text-primary">54.3%</span><abbr title="Good confidence: 2,089 tracked matches give a 95% confidence interval of ±2.1% around the win rate."><span class="text-xs">Good</span></abbr></div></a></div></li></ul>
<h2>Heroes to ban against Winston</h2>
<ul><li><div class="group flex"><a class="flex flex-1 sm:px-4" href="/stats/overwatch/heroes/reinhardt"><img alt="Reinhardt" class="size-10"/><div class="mt-1 flex"><span class="tabular-nums"><span class="uppercase">Counter rating </span><span class="text-zinc-200 font-medium">+5.7</span></span><abbr title="Very high confidence: 940 players contributed duel data for this matchup."><span class="text-xs">Very high</span></abbr></div></a></div></li></ul>
</body></html>"##;

    /// The Roadhog row with its colour class swapped and its sentence left alone,
    /// which is what a restyle at the site would look like.
    const RESTYLED_FIXTURE: &str = r##"<html><body>
<li><div class="group flex"><a class="flex flex-1" href="/stats/overwatch/heroes/roadhog"><img alt="Roadhog" class="size-9"/><div class="shrink-0 flex flex-col items-end gap-1" title="Roadhog takes 80% of the kills traded with Winston, and Winston suffers 58% of the deaths while both are up."><span class="text-base font-semibold text-green-300">+19.6</span><abbr title="High confidence: 612 players contributed duel data for this matchup."><span class="text-xs">High</span></abbr></div></a></div></li>
</body></html>"##;

    fn ratings() -> Vec<MatchupRating> {
        parse_matchup_ratings(RATINGS_FIXTURE, "Winston").expect("the fixture parses")
    }

    fn row<'a>(rows: &'a [MatchupRating], vs: &str) -> &'a MatchupRating {
        rows.iter().find(|r| r.vs == vs).expect("row present")
    }

    #[test]
    fn a_hardest_matchup_is_negative_for_the_hero_whose_page_it_is() {
        let rows = ratings();
        let roadhog = row(&rows, "roadhog");
        assert_eq!(roadhog.rating, -19.6, "the printed + carries no direction");
        // +-25 is the ceiling, so 19.6 lands most of the way down.
        assert_eq!(roadhog.value, -78);
        assert_eq!(roadhog.swing, Some(17));
        assert_eq!(roadhog.duels, 612);
    }

    #[test]
    fn an_easiest_matchup_is_positive_for_the_hero_whose_page_it_is() {
        let rows = ratings();
        let widow = row(&rows, "widowmaker");
        assert_eq!(widow.rating, 16.1);
        assert_eq!(widow.value, 64);
        assert_eq!(widow.swing, Some(15));
    }

    /// The sentence is ordered, not argued: whichever hero it names first is the
    /// one the row favours, however the kill share reads.
    #[test]
    fn the_direction_of_a_counter_rating_comes_from_the_prose_and_not_the_percentage() {
        let rows = ratings();
        let genji = row(&rows, "genji");
        assert!(
            genji.value > 0,
            "Winston takes 28% of the kill trades and still wins this row"
        );
        assert_eq!(genji.value, 47);
    }

    /// Both cues hold for all 530 published rows, so one of them moving is a
    /// change at the site rather than a row to quietly drop. Reading the class
    /// alone would let a restyle invert every reading with nothing to notice it.
    #[test]
    fn a_row_whose_colour_contradicts_its_prose_stops_the_parse() {
        let err = parse_matchup_ratings(RESTYLED_FIXTURE, "Winston")
            .expect_err("a contradiction must not be absorbed");
        assert!(
            format!("{err:#}").contains("contradicts itself"),
            "the error has to say what happened: {err:#}"
        );
    }

    /// Terminating the leading name at punctuation truncates `D.Mon` to `D`,
    /// which matches neither hero and silently drops all ten rows on that page.
    #[test]
    fn a_hero_whose_name_contains_a_full_stop_is_read_whole() {
        let rows = ratings();
        let dmon = row(&rows, "dmon");
        assert_eq!(dmon.value, -30);
        assert_eq!(dmon.duels, 1010, "and the separator is not a terminator");
    }

    #[test]
    fn the_duo_rows_on_the_same_page_are_not_read_as_matchups() {
        assert!(
            !ratings().iter().any(|r| r.vs == "torbjorn"),
            "a duo row shares the anchor and has no tooltip on its rating"
        );
    }

    #[test]
    fn the_ban_list_rows_on_the_same_page_are_not_read_as_matchups() {
        assert!(
            !ratings().iter().any(|r| r.vs == "reinhardt"),
            "the ban list restates these ratings with no sentence and no colour"
        );
    }

    #[test]
    fn a_row_without_a_swing_is_still_a_reading() {
        let rows = ratings();
        let weaver = row(&rows, "lifeweaver");
        assert_eq!(weaver.swing, None, "80 of the 530 rows state none");
        assert_eq!(weaver.value, 36);
        assert_eq!(weaver.duels, 881);
    }

    #[test]
    fn the_page_carries_exactly_the_matchups_and_nothing_else() {
        let rows = ratings();
        let mut keys: Vec<&str> = rows.iter().map(|r| r.vs.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["dmon", "genji", "lifeweaver", "roadhog", "widowmaker"]
        );
    }

    fn merge_names() -> HashMap<String, String> {
        [
            ("Pharah", "pharah"),
            ("Brigitte", "brigitte"),
            ("Zarya", "zarya"),
        ]
        .into_iter()
        .map(|(name, key)| (name.to_owned(), key.to_owned()))
        .collect()
    }

    fn published(pairs: &[(&str, f32)]) -> HashMap<(String, String), MatchupRating> {
        pairs
            .iter()
            .map(|(vs, rating)| {
                (
                    ("reinhardt".to_owned(), (*vs).to_owned()),
                    MatchupRating {
                        vs: (*vs).to_owned(),
                        value: overwatch_core::normalize(
                            *rating,
                            -COUNTER_CEILING,
                            COUNTER_CEILING,
                        ),
                        rating: *rating,
                        swing: None,
                        duels: 500,
                    },
                )
            })
            .collect()
    }

    fn ranked(countered_by: &[&str], punishes: &[&str], top_rating: Option<f32>) -> HeroCounters {
        HeroCounters {
            countered_by: countered_by.iter().map(|s| (*s).to_owned()).collect(),
            punishes: punishes.iter().map(|s| (*s).to_owned()).collect(),
            top_rating,
        }
    }

    #[test]
    fn a_published_rating_beats_a_synthesised_rank() {
        let parsed = ranked(&["Pharah", "Brigitte", "Zarya"], &[], Some(20.0));
        let out = hero_values(
            "reinhardt",
            &parsed,
            &[],
            &published(&[("pharah", -12.0)]),
            &merge_names(),
        );

        // -12.0 against a +-25 ceiling, and not the -80 that rank 1 of 3 anchored
        // on a published top rating of 20.0 would have produced.
        assert_eq!(out[&("reinhardt".to_owned(), "pharah".to_owned())], -48);
    }

    /// Rank 2 of 3 has to interpolate from something. Anchoring it on the number
    /// the site published for rank 1 is what stops the guesses coming out louder
    /// than the measurements beside them.
    #[test]
    fn a_synthesised_tail_is_anchored_on_the_numbers_the_site_published() {
        let parsed = ranked(&["Pharah", "Brigitte", "Zarya"], &[], Some(20.0));
        let out = hero_values(
            "reinhardt",
            &parsed,
            &[],
            &published(&[("pharah", -12.0)]),
            &merge_names(),
        );

        // Half way from Pharah's measured 48 to the tail's 25. Anchored on
        // `top_magnitude(Some(20.0))`'s 80 instead, this row would read -53.
        assert_eq!(out[&("reinhardt".to_owned(), "brigitte".to_owned())], -37);
        assert_eq!(out[&("reinhardt".to_owned(), "zarya".to_owned())], -25);
    }

    /// The punish list has no published rating of its own on the counters page, so
    /// it used to be handed `countered_by[0]`'s and decayed across three positions
    /// instead of ten — asserting a hero's easiest matchups at the strength of its
    /// hardest one.
    #[test]
    fn the_punish_list_no_longer_borrows_the_countered_by_anchor() {
        let parsed = ranked(&["Pharah"], &["Brigitte", "Zarya"], Some(25.0));
        let out = hero_values("reinhardt", &parsed, &[], &HashMap::new(), &merge_names());

        assert_eq!(
            out[&("reinhardt".to_owned(), "pharah".to_owned())],
            -90,
            "the countered-by side still reads its own published anchor"
        );
        assert_eq!(
            out[&("reinhardt".to_owned(), "brigitte".to_owned())],
            65,
            "and the punish side reads the default, not that 90"
        );
        assert_eq!(out[&("reinhardt".to_owned(), "zarya".to_owned())], 25);
    }

    #[test]
    fn a_list_with_no_published_rating_falls_back_to_the_published_top_rating() {
        let parsed = ranked(&["Pharah", "Brigitte"], &[], Some(10.0));
        let out = hero_values("reinhardt", &parsed, &[], &HashMap::new(), &merge_names());

        assert_eq!(out[&("reinhardt".to_owned(), "pharah".to_owned())], -55);
        assert_eq!(out[&("reinhardt".to_owned(), "brigitte".to_owned())], -25);
    }

    /// A published anchor can land below the tail, and a decay that ran to it
    /// regardless would report a hero's seventh-worst matchup as worse than its
    /// first.
    #[test]
    fn a_rank_decay_never_rises_toward_the_tail() {
        assert!(rank_to_value(3, 3, 20.0) <= rank_to_value(1, 3, 20.0));
        assert_eq!(rank_to_value(3, 3, 20.0), 20);
        // Unchanged wherever the anchor is above the tail, which is every list
        // that falls back to `top_magnitude`.
        assert_eq!(rank_to_value(3, 3, 90.0), TAIL_MAGNITUDE as i8);
    }

    /// The two documents rank a hero's matchups differently, so the stats page
    fn swing_row(
        opponent: &str,
        subject: &str,
        kills: u32,
        deaths: u32,
        swing: Option<u32>,
    ) -> String {
        let impact = match swing {
            Some(s) => format!(
                " Counter impact: an estimated +{s}% swing to an even fight, from duel and \
                 teamfight outcomes. Not a win rate."
            ),
            None => String::new(),
        };
        // counterwatch's own slugs drop the full stop and hyphenate spaces —
        // `dmon`, `wrecking-ball` — as the cached pages show.
        let slug = opponent
            .to_ascii_lowercase()
            .replace('.', "")
            .replace(' ', "-");
        format!(
            r#"<li><a title="{opponent} takes {kills}% of the kills traded with {subject}, and                {subject} suffers {deaths}% of the deaths when both are up.{impact}"                href="/stats/overwatch/heroes/{slug}"><img alt="{opponent}"/></a></li>"#
        )
    }

    /// The direction rule, which is the whole risk in reading this document: the
    /// swing favours the hero the sentence names first, and that is never the page
    /// subject. So every reading off a hero's own page is unfavourable for it.
    #[test]
    fn a_swing_is_unfavourable_for_the_hero_whose_page_it_is() {
        let html = swing_row("Mauga", "Wrecking Ball", 61, 47, Some(20));
        let out = parse_swings(&html, "Wrecking Ball").expect("parses");

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].vs, "mauga");
        assert_eq!(out[0].swing, 20);
        // 20 of a 23-point ceiling, negated for the subject.
        assert_eq!(out[0].value, -87);
    }

    /// A row that states kill shares and stops there is skipped, not read off the
    /// percentages beside it. Those measure how killy a hero is — 47% of their
    /// variance is which hero, not which matchup — so they are not a matchup value.
    #[test]
    fn a_row_without_a_counter_impact_is_not_read_off_the_kill_share() {
        let html = swing_row("Symmetra", "Wrecking Ball", 41, 42, None);
        let out = parse_swings(&html, "Wrecking Ball").expect("parses");

        assert!(
            out.is_empty(),
            "a kill share is not a counter reading: {out:?}"
        );
    }

    /// The swing is anchored on its own label, because the sentence in front of it
    /// carries two other percentages that would both parse.
    #[test]
    fn the_swing_is_read_past_the_kill_and_death_percentages() {
        let html = swing_row("Roadhog", "Wrecking Ball", 61, 52, Some(9));
        let out = parse_swings(&html, "Wrecking Ball").expect("parses");

        assert_eq!(out[0].swing, 9, "read 61 or 52 and the row is nonsense");
    }

    /// Past the ceiling the reading clamps rather than wrapping, and the ceiling is
    /// set so that this happens to two of the 869 rows the site publishes.
    #[test]
    fn a_swing_past_the_ceiling_clamps_to_the_rail() {
        let html = swing_row("Mauga", "Wrecking Ball", 61, 47, Some(32));
        let out = parse_swings(&html, "Wrecking Ball").expect("parses");

        assert_eq!(out[0].value, -100);
    }

    /// A hero whose name contains a full stop survives the split, for the same
    /// reason `parse_matchup_ratings` splits on the verb: terminating the name at
    /// punctuation truncates `D.Mon` to `D` and silently drops every row.
    #[test]
    fn a_swing_row_for_a_hero_whose_name_contains_a_full_stop_is_read_whole() {
        let html = swing_row("D.Mon", "Wrecking Ball", 34, 33, Some(6));
        let out = parse_swings(&html, "Wrecking Ball").expect("parses");

        assert_eq!(out.len(), 1, "D.Mon was truncated: {out:?}");
        assert_eq!(out[0].vs, "dmon");
    }

    /// If the site ever names the subject first, the direction rule stops being
    /// safe and the parse must stop rather than invert every reading on the page.
    #[test]
    fn the_subject_leading_its_own_row_stops_the_parse() {
        let html = swing_row("Wrecking Ball", "Mauga", 61, 47, Some(20));
        let err = parse_swings(&html, "Wrecking Ball").expect_err("must refuse");

        assert!(
            err.to_string().contains("names Wrecking Ball first"),
            "unexpected error: {err}"
        );
    }

    /// The finer figure wins. Both documents quote one quantity — they agree to
    /// r = +0.978 over the 225 pairs they share — so where the stats page published
    /// a rating to one decimal, the counters page's whole-number swing must not
    /// overwrite it.
    #[test]
    fn a_published_rating_outranks_a_swing_for_the_same_pair() {
        let swings = vec![MatchupSwing {
            vs: "zarya".to_owned(),
            swing: 20,
            value: -87,
        }];
        let out = hero_values(
            "reinhardt",
            &HeroCounters::default(),
            &swings,
            &published(&[("zarya", 6.8)]),
            &merge_names(),
        );

        assert_eq!(
            out[&("reinhardt".to_owned(), "zarya".to_owned())],
            overwatch_core::normalize(6.8, -COUNTER_CEILING, COUNTER_CEILING),
            "the rating is the reading; the swing only fills what it does not cover"
        );
    }

    /// And a swing beats a rank interpolation, which is the point of reading it:
    /// 644 pairs were arriving as a guess from list position.
    #[test]
    fn a_swing_outranks_a_rank_interpolation() {
        let parsed = ranked(&["Zarya", "Pharah"], &[], None);
        let swings = vec![MatchupSwing {
            vs: "zarya".to_owned(),
            swing: 3,
            value: -13,
        }];
        let out = hero_values(
            "reinhardt",
            &parsed,
            &swings,
            &HashMap::new(),
            &merge_names(),
        );

        assert_eq!(
            out[&("reinhardt".to_owned(), "zarya".to_owned())],
            -13,
            "a published swing must not be replaced by where the list put it"
        );
    }

    /// rates ten pairs that are not the ten the counters page lists.
    #[test]
    fn a_pair_the_counters_page_never_ranked_still_reaches_the_matrix() {
        let out = hero_values(
            "reinhardt",
            &HeroCounters::default(),
            &[],
            &published(&[("zarya", 6.8)]),
            &merge_names(),
        );

        assert_eq!(
            out.len(),
            1,
            "a hero with no counters page keeps its numbers"
        );
        assert_eq!(out[&("reinhardt".to_owned(), "zarya".to_owned())], 27);
    }
}
