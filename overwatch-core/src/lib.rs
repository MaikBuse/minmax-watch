//! Domain model and scoring engine for the Overwatch hero picker.
//!
//! This crate is deliberately I/O-free so it can compile to `wasm32` and run
//! inside the draft UI. Recommendations are computed on the client; the sync
//! server only ever moves [`session::SessionState`] between the people in a
//! session, so the network never sits between a keystroke and an answer.
//!
//! It also never panics: there is no `unwrap` or `expect` outside tests, and
//! lookups that could go out of range return neutral values or [`error::CoreError`].

#![forbid(unsafe_code)]

pub mod archetype;
pub mod dataset;
pub mod draft;
pub mod error;
#[cfg(test)]
pub(crate) mod fixture;
pub mod format;
pub mod hero;
pub mod map;
pub mod matrix;
pub mod score;
pub mod search;
pub mod session;

pub use archetype::{shape_of, Archetype, Shape};
pub use dataset::{Dataset, DatasetParts};
pub use draft::{enemies_in_role, fit_to_format, Draft};
pub use error::CoreError;
pub use format::{Capacity, Format, Queue, TeamSize};
pub use hero::{Hero, HeroId, HeroSet, Role, Subrole};
pub use map::{GameMap, GameMode, MapId, Side};
pub use matrix::Matrix;
pub use score::{
    ban_recommendations, recommend, threats, BanBoard, BanCandidate, BanSubject, Defended,
    DefendedTeam, EnemyRoleWeights, Knowledge, Reason, ReasonKind, Recommendation, Threat,
    UserContext, Weights,
};
pub use search::{resolve, resolve_map, search, search_maps, MapMatch, Match, MatchKind, Scope};
pub use session::{Board, Seat, SessionState};

/// Converts a counterpickgg difficulty into the canonical -100..=100 scale, from
/// the perspective of the hero whose page it was scraped from.
///
/// The site reads "how hard is this opponent for me", so a high rating is a
/// disaster and a low one is free. It renders the badge as `N/10`, but the scale
/// it actually uses is **1..9 with 5 as neutral**, which is what this anchors on.
/// Three independent readings of the cached pages agree:
///
/// - `difficulty(a, b) + difficulty(b, a) == 10` for all 2378 mirrored pairs, with
///   no exceptions. 5 is the only self-consistent value, so it is the site's
///   neutral by construction.
/// - The rating histogram is perfectly symmetric about 5:
///   `{1:23, 2:177, 3:326, 4:404, 5:612, 6:404, 7:326, 8:177, 9:23}`. A 10 never
///   appears — its mirror would have to be 0.
/// - The badge is yellow at 5, red below it and green above it.
///
/// Taking the `/10` literally — a 5.5 midpoint over a 4.5 half-span — is what this
/// replaced. It put a `+11` on every dead-even matchup and mapped `1` to `+100`
/// while `9` only reached `-78`.
pub fn difficulty_to_value(difficulty: f32) -> i8 {
    let clamped = difficulty.clamp(1.0, 9.0);
    let scaled = (5.0 - clamped) / 4.0 * 100.0;
    scaled.round().clamp(-100.0, 100.0) as i8
}

/// Rescales an arbitrary source range onto -100..=100.
pub fn normalize(value: f32, source_min: f32, source_max: f32) -> i8 {
    let span = source_max - source_min;
    if span.abs() < f32::EPSILON {
        return 0;
    }
    let midpoint = (source_min + source_max) / 2.0;
    let scaled = (value - midpoint) / (span / 2.0) * 100.0;
    scaled.round().clamp(-100.0, 100.0) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_maps_onto_the_canonical_scale() {
        // Pharah is rated 9/10 into Reinhardt: the worst it gets.
        assert_eq!(difficulty_to_value(9.0), -100);
        // Reinhardt is rated 2/10 into Brigitte: strongly positive.
        assert_eq!(difficulty_to_value(2.0), 75);
        assert_eq!(difficulty_to_value(1.0), 100);
        // 5 is the site's neutral, and its most common rating.
        assert_eq!(difficulty_to_value(5.0), 0);
    }

    /// The site's own invariant, and the evidence the 5.0 midpoint rests on: it
    /// rates every pair from both sides such that the two ratings sum to 10. A
    /// midpoint that did not sit at 5 would fail this.
    #[test]
    fn mirrored_difficulties_produce_mirrored_values() {
        for difficulty in 1..=9 {
            let forward = difficulty_to_value(difficulty as f32);
            let reverse = difficulty_to_value((10 - difficulty) as f32);
            assert_eq!(
                forward,
                -reverse,
                "{difficulty}/10 and {}/10 must be exact opposites",
                10 - difficulty
            );
        }
    }

    #[test]
    fn difficulty_clamps_out_of_range_input() {
        assert_eq!(difficulty_to_value(0.0), 100);
        // A 10 never appears in the data, but must not read as anything other
        // than the bottom of the scale if the site ever emits one.
        assert_eq!(difficulty_to_value(10.0), -100);
        assert_eq!(difficulty_to_value(99.0), -100);
    }

    #[test]
    fn normalize_rescales_the_overpicker_range() {
        // overpicker publishes -20..=+20.
        assert_eq!(normalize(20.0, -20.0, 20.0), 100);
        assert_eq!(normalize(-20.0, -20.0, 20.0), -100);
        assert_eq!(normalize(0.0, -20.0, 20.0), 0);
        assert_eq!(normalize(10.0, -20.0, 20.0), 50);
    }

    #[test]
    fn normalize_survives_a_degenerate_range() {
        assert_eq!(normalize(5.0, 3.0, 3.0), 0);
    }
}
