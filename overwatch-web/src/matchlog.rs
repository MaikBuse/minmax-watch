//! Recording how a match actually went.
//!
//! **This does not feed the scorer, and the decision not to is deliberate.** This
//! doc used to say the gap between the matrix and your own results was "exactly
//! what the personal overrides exist to close", which reads as a plan to infer
//! comfort from wins and losses. Comfort is *declared* instead — you say which
//! heroes you can play — and the four reasons are worth having in front of
//! whoever next reaches for this data:
//!
//! 1. **A result is the team's, not the hero's.** A `MatchRecord` holds one hero,
//!    one bit, and the ten-hero draft around it. Attributing the loss to your own
//!    pick is the selection problem the ingest corrects for in `stats.rs` — and
//!    that has tens of thousands of games per hero and still shrinks hard toward
//!    the role mean.
//! 2. **The sample is orders of magnitude too small.** One keystroke per match is
//!    the most that will realistically get done, which is single digits *per hero*
//!    for months, against a base rate near 50%. There is nothing to shrink toward
//!    except the prior.
//! 3. **It is a closed loop with nothing outside it.** The app recommends a hero,
//!    you play it, you win, and the app grows more confident in its own advice.
//!    Every other number in this repo is checked against something external — a
//!    scrape, a published win rate, `data/ban_rate.toml` as a yardstick. An
//!    inferred comfort term would be checked against the app.
//! 4. **You already know.** Saying so costs one click and is exact; inferring it
//!    costs a season and is approximate.
//!
//! What the log is for is the other question: an offline record of what was
//! drafted and how it went, greppable as JSON Lines, for judging *the app* by hand
//! — did the top pick win more? That is the one this file can actually answer.
//!
//! That budget of one keystroke is also the whole interface, which is what the
//! rest of this module is.
//!
//! Bound to Alt+W and Alt+L rather than bare W/L: recording a result also
//! clears the draft, so the modifier is a guard against a stray keypress
//! costing you the picks you just entered. That is the rule the whole chord
//! table follows — ctrl builds, alt costs — and `crate::keys` is where it
//! lives. Ctrl+W is the browser's close-tab and was never usable here anyway.

use overwatch_core::{Dataset, Draft, HeroId, Role};
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;
use web_sys::{Headers, Request, RequestInit};

/// Mirrors `overwatch_server::matchlog::MatchRecord`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchRecord {
    pub at: String,
    pub player: String,
    pub role: String,
    pub hero: String,
    pub map: Option<String>,
    #[serde(default)]
    pub side: Option<String>,
    pub enemies: Vec<String>,
    pub allies: Vec<String>,
    pub won: bool,
}

impl MatchRecord {
    /// Builds a record from the draft as it stands.
    ///
    /// `hero` is your locked pick; without one there is nothing meaningful to
    /// attribute the result to, so this returns `None` rather than guessing.
    pub fn from_draft(
        dataset: &Dataset,
        draft: &Draft,
        role: Role,
        player: &str,
        won: bool,
    ) -> Option<Self> {
        let hero = draft.locked?;
        let key = |hero: HeroId| dataset.hero(hero).ok().map(|h| h.key.clone());

        Some(Self {
            at: now_iso8601(),
            player: player.to_owned(),
            role: role.as_str().to_owned(),
            hero: key(hero)?,
            map: draft
                .map
                .and_then(|map| dataset.map(map).ok().map(|m| m.key.clone())),
            // Only meaningful on the modes that have sides, so a stale side
            // left over from another map is dropped rather than recorded.
            side: draft
                .map
                .and_then(|map| dataset.map(map).ok())
                .filter(|m| m.mode.has_sides())
                .and(draft.side)
                .map(|side| side.as_str().to_owned()),
            enemies: draft.enemies.iter().filter_map(|h| key(*h)).collect(),
            allies: draft.allies.iter().filter_map(|h| key(*h)).collect(),
            won,
        })
    }
}

fn now_iso8601() -> String {
    js_sys::Date::new_0().to_iso_string().into()
}

/// Posts a record, ignoring the outcome.
///
/// Losing a log entry because the server is off is not worth an error state in
/// the middle of a game — the point of the app keeps working either way.
pub fn record(entry: &MatchRecord) {
    let Ok(body) = serde_json::to_string(entry) else {
        return;
    };
    let Some(window) = web_sys::window() else {
        return;
    };

    let options = RequestInit::new();
    options.set_method("POST");
    options.set_body(&JsValue::from_str(&body));

    if let Ok(headers) = Headers::new() {
        let _ = headers.set("Content-Type", "application/json");
        options.set_headers(&headers);
    }

    if let Ok(request) = Request::new_with_str_and_init("/api/matches", &options) {
        let _ = window.fetch_with_request(&request);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_shape_matches_the_server() {
        let record = MatchRecord {
            at: "2026-08-15T20:00:00Z".to_owned(),
            player: "me".to_owned(),
            role: "tank".to_owned(),
            hero: "reinhardt".to_owned(),
            map: Some("kings-row".to_owned()),
            side: Some("attack".to_owned()),
            enemies: vec!["pharah".to_owned()],
            allies: vec![],
            won: true,
        };

        let json = serde_json::to_string(&record).expect("serialises");
        for field in [
            "at", "player", "role", "hero", "map", "side", "enemies", "allies", "won",
        ] {
            assert!(
                json.contains(&format!("\"{field}\"")),
                "missing {field}: {json}"
            );
        }
    }
}
