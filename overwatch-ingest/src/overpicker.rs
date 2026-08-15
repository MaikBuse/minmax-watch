//! Scraper for overpicker.com.
//!
//! The whole matrix ships as one embedded JSON literal in an inline script:
//!
//! ```js
//! const counterMatrix = {"Ana":{"Ana":0,"Ashe":0,...},...};
//! ```
//!
//! So the entire source costs a single request. Values are keyed by display
//! name and quantised to five levels (-20, -10, 0, +10, +20) on a documented
//! -20..=+20 scale, read row-versus-column: `counterMatrix[A][B]` is how well
//! `A` does against `B`.
//!
//! It is a coarser source than counterpickgg — five levels against ten, and
//! only ~42% of pairs are antisymmetric — so it is blended in at a lower weight
//! and used mainly as a second opinion and as a fallback for heroes the primary
//! source has not rated yet.

use std::collections::HashMap;

use anyhow::{Context, Result};
use overwatch_core::normalize;

use crate::cache::Fetcher;

const URL: &str = "https://overpicker.com/counters";
const MARKER: &str = "counterMatrix";

/// The documented scale of the published values.
const SOURCE_MIN: f32 = -20.0;
const SOURCE_MAX: f32 = 20.0;

/// Extracts the balanced JSON object that follows `marker`.
///
/// Brace counting is string- and escape-aware so a `{` inside a hero name could
/// not truncate the object.
fn extract_json_object<'a>(script: &'a str, marker: &str) -> Option<&'a str> {
    let after_marker = &script[script.find(marker)? + marker.len()..];
    let start = after_marker.find('{')?;
    let bytes = after_marker.as_bytes();

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return after_marker.get(start..=i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parses the page into `(hero_key, opponent_key) -> value` on -100..=100.
///
/// `names` maps display name to hero key. Names the roster does not know are
/// skipped with a warning rather than failing the run: a hero added upstream
/// before our roster refresh should not break the ingest.
pub fn parse_page(
    html: &str,
    names: &HashMap<String, String>,
) -> Result<HashMap<(String, String), i8>> {
    let raw = extract_json_object(html, MARKER)
        .context("no `counterMatrix` object found - the overpicker page layout changed")?;

    let matrix: HashMap<String, HashMap<String, f32>> =
        serde_json::from_str(raw).context("parsing the overpicker counterMatrix JSON")?;

    anyhow::ensure!(
        !matrix.is_empty(),
        "the overpicker counterMatrix parsed as empty"
    );

    let mut unknown: Vec<&str> = Vec::new();
    let mut out = HashMap::new();

    for (hero_name, row) in &matrix {
        let Some(hero) = names.get(hero_name) else {
            unknown.push(hero_name);
            continue;
        };
        for (opponent_name, value) in row {
            let Some(opponent) = names.get(opponent_name) else {
                unknown.push(opponent_name);
                continue;
            };
            // The self-matchup diagonal carries junk on this site.
            if hero == opponent {
                continue;
            }
            out.insert(
                (hero.clone(), opponent.clone()),
                normalize(*value, SOURCE_MIN, SOURCE_MAX),
            );
        }
    }

    unknown.sort_unstable();
    unknown.dedup();
    for name in unknown {
        eprintln!("  note: overpicker lists {name:?}, which is not in our roster");
    }

    Ok(out)
}

pub async fn scrape(
    fetcher: &mut Fetcher,
    names: &HashMap<String, String>,
) -> Result<HashMap<(String, String), i8>> {
    let body = fetcher.get(URL, "overpicker-counters.html").await?;
    parse_page(&body, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> HashMap<String, String> {
        [
            ("Reinhardt", "reinhardt"),
            ("Pharah", "pharah"),
            ("D.Va", "dva"),
            ("Lúcio", "lucio"),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k.to_owned()))
        .collect()
    }

    const FIXTURE: &str = r#"
        <html><body>
        <script>
            const counterMatrix = {"Reinhardt":{"Reinhardt":-10,"Pharah":10,"D.Va":0,"Lúcio":-20},
                                   "Pharah":{"Reinhardt":0,"Pharah":0,"D.Va":-20,"Lúcio":0},
                                   "D.Va":{"Reinhardt":0,"Pharah":20,"D.Va":0,"Lúcio":10},
                                   "Lúcio":{"Reinhardt":20,"Pharah":0,"D.Va":-10,"Lúcio":0}};
            const other = {"ignored":1};
        </script>
        </body></html>
    "#;

    #[test]
    fn extracts_and_rescales_the_matrix() {
        let parsed = parse_page(FIXTURE, &names()).expect("fixture parses");

        // +20 on the source scale is the top of the range.
        assert_eq!(
            parsed.get(&("dva".to_owned(), "pharah".to_owned())),
            Some(&100)
        );
        assert_eq!(
            parsed.get(&("pharah".to_owned(), "dva".to_owned())),
            Some(&-100)
        );
        assert_eq!(
            parsed.get(&("reinhardt".to_owned(), "pharah".to_owned())),
            Some(&50)
        );
    }

    #[test]
    fn the_self_matchup_diagonal_is_dropped() {
        let parsed = parse_page(FIXTURE, &names()).expect("fixture parses");
        assert!(!parsed.contains_key(&("reinhardt".to_owned(), "reinhardt".to_owned())));
    }

    #[test]
    fn unicode_names_resolve() {
        let parsed = parse_page(FIXTURE, &names()).expect("fixture parses");
        assert_eq!(
            parsed.get(&("lucio".to_owned(), "reinhardt".to_owned())),
            Some(&100)
        );
    }

    #[test]
    fn brace_matching_stops_at_the_right_object() {
        let extracted = extract_json_object(FIXTURE, MARKER).expect("object found");
        assert!(extracted.starts_with('{'));
        assert!(extracted.ends_with('}'));
        assert!(!extracted.contains("ignored"), "stopped at the wrong brace");
    }

    #[test]
    fn braces_inside_strings_do_not_confuse_the_matcher() {
        let tricky = r#"const counterMatrix = {"a{b":{"c}d":1}}; trailing"#;
        let extracted = extract_json_object(tricky, MARKER).expect("object found");
        assert_eq!(extracted, r#"{"a{b":{"c}d":1}}"#);
    }

    #[test]
    fn a_missing_marker_is_an_error_not_a_panic() {
        assert!(parse_page("<html></html>", &names()).is_err());
    }
}
