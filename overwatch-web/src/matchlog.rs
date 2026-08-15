//! Recording how a match actually went.
//!
//! The counter matrix says what *should* work. Only your own results say what
//! works for you, and the gap between the two is exactly what the personal
//! overrides exist to close. One keystroke per match is the most that will
//! realistically get done, so that is the whole interface.
//!
//! Bound to Alt+W and Alt+L rather than bare W/L: recording a result also
//! clears the draft, so the modifier is a guard against a stray keypress
//! costing you the picks you just entered. Ctrl+W is the browser's close-tab
//! and is not usable here.

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
