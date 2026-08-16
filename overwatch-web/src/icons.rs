//! Portrait and thumbnail URLs, resolved from a hero or map key.
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
//! Both directories are produced by `just ingest-art`; see
//! `overwatch-ingest/src/art.rs` for where they come from and why they are
//! committed rather than hot-linked.

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
