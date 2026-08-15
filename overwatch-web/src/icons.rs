//! Portrait and thumbnail URLs, resolved from a hero or map key.
//!
//! The artwork is bundled as two folder assets rather than ~90 individual
//! `asset!()` call sites. A folder asset keeps its directory name in the output
//! (manganis does not hash folders), which is what lets a URL be built at
//! runtime from the key the dataset already carries.
//!
//! Both files are produced by `just ingest-art`; see `overwatch-ingest/src/art.rs`
//! for where they come from and why they are committed rather than hot-linked.

use dioxus::prelude::*;

static HEROES: Asset = asset!("/assets/heroes", AssetOptions::folder());
static MAPS: Asset = asset!("/assets/maps", AssetOptions::folder());

/// Portrait for a hero key, as it appears in `data/heroes.toml`.
pub fn hero(key: &str) -> String {
    format!("{HEROES}/{key}.png")
}

/// Thumbnail for a map key, as it appears in `data/maps.toml`.
///
/// Not every map has one — OverFast lists a screenshot for a couple of maps it
/// has not actually published — so callers must survive a URL that 404s. They
/// do: every use is a CSS background rather than an `<img>`, which degrades to
/// an empty box instead of a broken-image glyph.
pub fn map(key: &str) -> String {
    format!("{MAPS}/{key}.jpg")
}
