//! Portrait, thumbnail and rank-badge URLs, resolved from a key.
//!
//! **These are deliberately not `asset!()` assets.** They used to be two folder
//! assets, which is the obvious way to do it, and it silently undid the reason
//! they are WebP at all: `dx` treats every `.png`/`.jpg`/`.webp` it bundles as
//! an image to re-encode, and the encoder it re-encodes with (`image` 0.25) can
//! only write WebP *losslessly*. A 13 kB lossy map thumbnail came back out of
//! the bundle at 93 kB. There is no CLI flag or `Dioxus.toml` key to turn that
//! off, and a `public/` directory takes the same path.
//!
//! So the artwork is copied into the bundle root by hand — by `build-web` in the
//! justfile and by `docker/build.sh`, alongside `sw.js` and the fonts, which are
//! there for the neighbouring reason — and addressed by absolute path. Anything
//! that changes where these live has to change both of those, or the portraits
//! 404 in production while `just serve` looks fine.
//!
//! All three directories are produced by `just ingest-art`; see
//! `overwatch-ingest/src/art.rs` for where they come from and why they are
//! committed rather than hot-linked.

use overwatch_core::Rank;

/// Portrait for a hero key, as it appears in `data/heroes.toml`.
pub fn hero(key: &str) -> String {
    format!("/heroes/{key}.webp")
}

/// Thumbnail for a map key, as it appears in `data/maps.toml`.
///
/// Not every map has one — OverFast lists a screenshot for a couple of maps it
/// has not actually published — so callers must survive a URL that 404s. They
/// do: every use is a CSS background rather than an `<img>`, which degrades to
/// an empty box instead of a broken-image glyph.
pub fn map(key: &str) -> String {
    format!("/maps/{key}.webp")
}

/// The badge for one rung of the ladder.
///
/// An `Option`, unlike [`hero`] and [`map`], and that is the point: [`Rank::All`]
/// is the whole ladder rather than a rung of it, Blizzard has never drawn a badge
/// for it and never will, so there is no path to build and nothing to 404 on.
/// Returning one would put a permanently broken tile in the header for everybody
/// who never opens the picker — which is the default.
///
/// Built from [`Rank::as_str`] and never [`Rank::label`]: the label carries the
/// `+` on `grandmaster+`, and no file on disk is named that.
pub fn rank(rank: Rank) -> Option<String> {
    match rank {
        Rank::All => None,
        rung => Some(format!("/ranks/{}.webp", rung.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one bug the `as_str`/`label` split exists to prevent, and it is
    /// invisible in review because it would 404 for the top rung alone.
    #[test]
    fn a_badge_path_is_built_from_the_stable_key_and_never_from_the_label() {
        assert_eq!(
            rank(Rank::Grandmaster).as_deref(),
            Some("/ranks/grandmaster.webp")
        );
        assert_eq!(
            Rank::Grandmaster.label(),
            "grandmaster+",
            "which is not a filename"
        );
    }

    #[test]
    fn the_whole_ladder_has_no_badge_because_it_is_not_a_rung() {
        assert_eq!(rank(Rank::All), None);
        for rung in Rank::DIVISIONS {
            assert!(rank(rung).is_some(), "{rung:?} needs a badge");
        }
    }
}
