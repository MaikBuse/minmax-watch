//! Hero portraits, map thumbnails and rank badges for the draft screen.
//!
//! Sources, named honestly: the portraits are Blizzard's own hero art, served
//! from their CDN and indexed by the OverFast API; the map screenshots are
//! hosted by OverFast itself; the rank badges are Blizzard's competitive tier
//! art from the same CDN. None of it is ours.
//!
//! They are downloaded, downscaled here and committed into
//! `overwatch-web/assets/` rather than linked at runtime, because the app is
//! offline-first: on a LAN with no internet, hot-linked art would break exactly
//! when the draft screen is being used. The raw originals stay in the gitignored
//! `data/sources/` cache, so re-encoding at a different size later is free.
//!
//! Everything written out is lossy WebP. The whole hero board is on screen at
//! once — there is no scroll to lazy-load behind and no route to defer past — so
//! all 53 portraits plus a map thumbnail land on the first paint, which for a
//! long time was 1.9 MB against a 341 kB wasm bundle. The artwork, not the
//! framework, was the download.
//!
//! Nothing here runs at application runtime.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::DynamicImage;

use overwatch_core::Rank;

use crate::cache::{write_bytes_if_changed, Fetcher};
use crate::overfast;

/// Portraits are square. 96 px covers the largest on-screen use (34 px) at
/// nearly 3x device pixel ratio; beyond that we are shipping bytes nobody sees.
const HERO_PX: u32 = 96;

/// Map thumbnails are 16:9, sized for the header chip and the picker list.
const MAP_W: u32 = 320;
const MAP_H: u32 = 180;

/// Portraits, at the quality the whole hero board is downloaded at.
///
/// Measured over all 53 committed portraits against the PNGs this replaced:
/// 1118 kB to 255 kB, and the worst single hero came out at an RMSE of 9.9/255
/// across the pixels with any opacity at all — invisible at the 22-34 px these
/// are drawn at. Alpha survives exactly, which is what matters: 52 of the 53
/// are cut-outs, and a portrait with a grey box behind it is worse than no
/// portrait. libwebp compresses the alpha plane losslessly by default and
/// nothing here overrides that.
const HERO_QUALITY: f32 = 90.0;

/// Screenshots are photographic and sit behind text, so they can take more
/// compression than the portraits: they are never the thing being read.
const MAP_QUALITY: f32 = 80.0;

/// Lanczos costs a few milliseconds per image and is the difference between a
/// portrait that reads at 22 px and one that looks like a thumbnail of a
/// thumbnail.
const FILTER: FilterType = FilterType::Lanczos3;

/// Badges are not one shape: bronze through diamond are 80x80, master is
/// 100x80 and grandmaster is 118x80 — the wings are what tells the top rungs
/// apart, and they grow sideways off a shared 80 px baseline. So the invariant
/// across the set is *height*, and that is what this normalises; the width
/// follows and the stylesheet letterboxes it.
///
/// 80 because that is the source height, and there is nothing larger to fetch.
/// It is also enough on its own terms: the largest on-screen use is the 24 px
/// badge in the picker's rows, so this is 3.3x — past the 2.8x `HERO_PX` buys
/// at its 34 px.
const RANK_PX: u32 = 80;

/// Where the rank badges live, and the filenames that address them.
///
/// The path is digest-addressed: the hex is a content hash, not a name, and
/// Blizzard publishes no listing endpoint for it. These eight were found by
/// sampling public career profiles through OverFast — `/players?name=…` to get
/// ids, then `/players/{id}/summary`, where every role's competitive rank
/// carries a `division` beside the `rank_icon` URL — until all eight divisions
/// had turned up. If one starts 404ing, that is a re-export rather than a
/// removal: sample again the same way, and see the hard failure in `build` —
/// a missing badge stops the run rather than degrading, because a 404 here is
/// a stale hash rather than art that was never published.
///
/// The tempting simplification is the unhashed legacy path,
/// `d1u1mce87gyfbn.cloudfront.net/game/rank-icons/rank-{Tier}Tier.png`, which
/// needs no hashes at all. It is the Overwatch 1 art set: **it has no Emerald**
/// — that division postdates it and 404s — and its badges carry a heavy black
/// outline this set does not, so the seven it does have cannot be mixed with a
/// modern Emerald without the seam showing at 24 px. A complete set in one
/// style beats an enumerable one with a hole in the middle.
///
/// Keyed on [`Rank::as_str`], which is also the committed filename and the
/// `/ranks/{key}.webp` path the client asks for.
const RANK_BASE: &str = "https://static.playoverwatch.com/images/pages/career/icons/rank";

const RANKS: &[(Rank, &str)] = &[
    (
        Rank::Bronze,
        "Rank_BronzeTier.f46bc3540601455a7768761396f9310d805052ad.png",
    ),
    (
        Rank::Silver,
        "Rank_SilverTier.305e60dca5e95356ced59825426844cc7cdb4948.png",
    ),
    (
        Rank::Gold,
        "Rank_GoldTier.8d40eab551020b46a84002019dd98714149ed5e2.png",
    ),
    (
        Rank::Platinum,
        "Rank_PlatinumTier.694340e3a5b2031d4fac9eb1d8f52c6855d4c072.png",
    ),
    (
        Rank::Emerald,
        "Rank_EmeraldTier.d82e76cb2deeac6d1f4d4cf4d8914517d903eeeb.png",
    ),
    (
        Rank::Diamond,
        "Rank_DiamondTier.bc8c5d6005f916c62c51728087aeb26f4facaa9b.png",
    ),
    (
        Rank::Master,
        "Rank_MasterTier.d5994999a94e84aeff892ddc2df4019afe41bf59.png",
    ),
    (
        Rank::Grandmaster,
        "Rank_GrandmasterTier.cf8bc4567786439baf485ca6d63f6ff27fe4a281.png",
    ),
];

#[derive(Default)]
pub struct Report {
    pub heroes: usize,
    pub maps: usize,
    pub ranks: usize,
    pub changed: usize,
    pub removed: usize,
    /// Keys the dataset has but no artwork exists for. The UI falls back to the
    /// name alone for these, so a gap costs an icon rather than a hero.
    pub missing: Vec<String>,
}

/// Downloads, resizes and writes every portrait, map thumbnail and rank badge.
///
/// `hero_keys` and `map_keys` come from the committed TOML, which is what
/// decides coverage: artwork is only fetched for keys the dataset knows, and
/// artwork for a key the dataset dropped is deleted. The badges take no
/// argument for the same reason in reverse — the ladder is a closed set in
/// core, not something the dataset has an opinion about.
pub async fn build(
    fetcher: &mut Fetcher,
    web_assets: &Path,
    hero_keys: &HashSet<String>,
    map_keys: &HashSet<String>,
) -> Result<Report> {
    let (portraits, screenshots) = overfast::art_urls(fetcher).await?;

    let hero_dir = web_assets.join("heroes");
    let map_dir = web_assets.join("maps");
    let rank_dir = web_assets.join("ranks");
    let mut report = Report::default();

    // Anything the dataset knows but the index never listed is already a gap,
    // before a single request goes out.
    report.missing.extend(unlisted(hero_keys, &portraits));
    report.missing.extend(unlisted(map_keys, &screenshots));

    for (key, url) in portraits.iter().filter(|(key, _)| hero_keys.contains(key)) {
        let Some(raw) = fetcher
            .get_bytes(url, &format!("art-hero-{key}.png"))
            .await?
        else {
            report.missing.push(key.clone());
            continue;
        };
        let image = decode(&raw, url)?.resize_exact(HERO_PX, HERO_PX, FILTER);

        // RGBA, not RGB: the portraits are cut-outs and the board shows the
        // tile colour through them.
        let rgba = image.to_rgba8();
        let encoded = encode_webp(
            webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height()),
            HERO_QUALITY,
        )
        .with_context(|| format!("encoding the portrait for {key}"))?;

        if write_bytes_if_changed(&hero_dir.join(format!("{key}.webp")), &encoded).await? {
            report.changed += 1;
        }
        report.heroes += 1;
    }

    for (key, url) in screenshots.iter().filter(|(key, _)| map_keys.contains(key)) {
        let Some(raw) = fetcher
            .get_bytes(url, &format!("art-map-{key}.jpg"))
            .await?
        else {
            report.missing.push(key.clone());
            continue;
        };
        // `resize_to_fill` crops to the target aspect from the centre, so a
        // screenshot that is not already 16:9 loses its edges rather than being
        // squashed.
        let image = decode(&raw, url)?.resize_to_fill(MAP_W, MAP_H, FILTER);

        // A screenshot is opaque, and going through RGB8 says so rather than
        // paying for an alpha plane that is 255 everywhere.
        let rgb = image.to_rgb8();
        let encoded = encode_webp(
            webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()),
            MAP_QUALITY,
        )
        .with_context(|| format!("encoding the thumbnail for {key}"))?;

        if write_bytes_if_changed(&map_dir.join(format!("{key}.webp")), &encoded).await? {
            report.changed += 1;
        }
        report.maps += 1;
    }

    for (rank, file) in RANKS {
        let key = rank.as_str();
        let url = format!("{RANK_BASE}/{file}");
        let Some(raw) = fetcher.get_bytes(&url, &rank_slug(*rank, file)).await? else {
            // Deliberately not pushed onto `report.missing`. That list means
            // "the source published no art for this", which is a fact about the
            // source and costs an icon; a 404 here means the hash in the table
            // has gone stale, and `get_bytes` remembers a 404, so letting it
            // pass would be a permanent silent gap. The `ensure!` below is what
            // this falls into.
            continue;
        };
        let image = decode(&raw, &url)?;
        // `resize`, not `resize_exact` or `resize_to_fill`: the badges run from
        // 80x80 to 118x80, and this is the one of the three that preserves
        // aspect and fits inside the box, so the wings are neither stretched
        // nor cropped off. Skipped entirely when the source is already the
        // right height, which today is all eight — Lanczos at 1:1 is not the
        // identity, it re-filters, and it softens the thin gold filigree these
        // are mostly made of.
        let image = if image.height() == RANK_PX {
            image
        } else {
            image.resize(RANK_PX, RANK_PX, FILTER)
        };

        // Cut-outs on a dark sheet, so alpha matters exactly as it does for the
        // portraits above — and `HERO_QUALITY` for the same reason, checked the
        // same way. Lossless was the obvious alternative here, since these are
        // hard gold edges over large transparent regions rather than
        // photographs, and eight files could afford it. It is not needed: all
        // eight come to 23 kB lossy and show no visible artefact at the 24 px
        // they are drawn at, which is the size the argument is actually about.
        let rgba = image.to_rgba8();
        let encoded = encode_webp(
            webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height()),
            HERO_QUALITY,
        )
        .with_context(|| format!("encoding the badge for {key}"))?;

        if write_bytes_if_changed(&rank_dir.join(format!("{key}.webp")), &encoded).await? {
            report.changed += 1;
        }
        report.ranks += 1;
    }

    // Individual gaps are tolerable; nothing at all means the sources moved and
    // the UI would silently lose every image.
    anyhow::ensure!(
        report.heroes > 0 && report.maps > 0,
        "no artwork could be fetched at all - has the OverFast schema or CDN changed?"
    );

    // The badges are the one category where a *partial* result is a bug rather
    // than a gap. A hero with no portrait means OverFast never published one; a
    // rung with no badge means the content hash in `RANKS` is stale, and the
    // picker would ship one blank row among eight good ones with nothing but
    // this to say why.
    anyhow::ensure!(
        report.ranks == RANKS.len(),
        "only {}/{} rank badges could be fetched - the filenames in RANKS carry a \
         content hash and Blizzard has re-exported them. See the doc comment above \
         the table for how to re-discover the set; `just ingest-refresh` gets past \
         the cached 404s.",
        report.ranks,
        RANKS.len(),
    );

    // A hero or map that left the dataset would otherwise leave its art behind
    // forever: the asset directory ships whole, so orphans cost bundle size.
    let rank_keys: HashSet<String> = Rank::DIVISIONS
        .iter()
        .map(|rank| rank.as_str().to_owned())
        .collect();
    report.removed = prune(&hero_dir, "webp", hero_keys).await?
        + prune(&map_dir, "webp", map_keys).await?
        + prune(&rank_dir, "webp", &rank_keys).await?;

    report.missing.sort_unstable();
    Ok(report)
}

/// Cache key for a badge, carrying the content hash out of its filename.
///
/// The hash has to be in the slug. Slugs are flat filenames, so one that named
/// only the rung would survive a re-export — and the next run would serve the
/// *old* bytes out of `data/sources/` under the new URL, forever, with nothing
/// to say so. Same lesson as `blizzard-rates-{tier}.json`: the tier there, the
/// hash here.
fn rank_slug(rank: Rank, file: &str) -> String {
    // `Rank_BronzeTier.<40 hex>.png` — the middle segment. A filename that is
    // not that shape falls back to itself, which is already unique and already
    // safe to put in a path.
    match file.split('.').nth(1) {
        Some(hash) if hash.len() >= 8 => {
            format!("art-rank-{}-{}.png", rank.as_str(), &hash[..8])
        }
        _ => format!("art-rank-{file}"),
    }
}

fn decode(bytes: &[u8], url: &str) -> Result<DynamicImage> {
    image::load_from_memory(bytes).with_context(|| format!("decoding the image at {url}"))
}

/// Lossy WebP at `quality`, as owned bytes.
///
/// `Encoder::encode` panics on a libwebp failure; `encode_simple` returns the
/// error instead, and the error type is not `std::error::Error`, so it is
/// spelled out here rather than at both call sites.
fn encode_webp(encoder: webp::Encoder<'_>, quality: f32) -> Result<Vec<u8>> {
    encoder
        .encode_simple(false, quality)
        .map(|mem| mem.to_vec())
        .map_err(|err| anyhow::anyhow!("libwebp rejected the frame: {err:?}"))
}

/// Dataset keys the artwork index never mentioned.
fn unlisted(keys: &HashSet<String>, urls: &[(String, String)]) -> Vec<String> {
    let listed: HashSet<&str> = urls.iter().map(|(key, _)| key.as_str()).collect();
    keys.iter()
        .filter(|key| !listed.contains(key.as_str()))
        .cloned()
        .collect()
}

/// Deletes `*.ext` files in `dir` whose stem is no longer in `keys`.
async fn prune(dir: &Path, ext: &str, keys: &HashSet<String>) -> Result<usize> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        // Nothing written yet is not a stale-file problem.
        Err(_) => return Ok(0),
    };

    let mut removed = 0;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(ext) {
            continue;
        }
        let stale = path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| !keys.contains(stem));
        if stale {
            tokio::fs::remove_file(&path)
                .await
                .with_context(|| format!("removing {}", path.display()))?;
            removed += 1;
        }
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is hand-written and nothing indexes it, so a rung added to
    /// `overwatch-core` — Champion splitting back out of Grandmaster is the one
    /// that could actually happen — would otherwise ship as a blank row in the
    /// picker with no error anywhere.
    #[test]
    fn every_division_has_exactly_one_badge_and_the_aggregate_has_none() {
        assert_eq!(RANKS.len(), Rank::DIVISIONS.len());
        for rung in Rank::DIVISIONS {
            let found = RANKS.iter().filter(|(rank, _)| *rank == rung).count();
            assert_eq!(found, 1, "{rung:?} needs exactly one badge");
        }
        assert!(
            RANKS.iter().all(|(rank, _)| *rank != Rank::All),
            "the whole ladder is not a rung and has no badge"
        );
    }

    /// Slugs are flat filenames. One that named only the rung would survive a
    /// re-export, and the next run would serve the old bytes out of the cache
    /// under the new URL forever, with nothing to say so.
    #[test]
    fn a_badge_cache_slug_changes_when_the_content_hash_does() {
        let before = rank_slug(
            Rank::Bronze,
            "Rank_BronzeTier.f46bc3540601455a7768761396f9310d805052ad.png",
        );
        let after = rank_slug(
            Rank::Bronze,
            "Rank_BronzeTier.0000000000000000000000000000000000000000.png",
        );
        assert_ne!(before, after);
        assert!(before.starts_with("art-rank-bronze-"), "got {before}");
    }

    /// A filename that is not the shape this expects still has to produce a
    /// usable, unique key rather than colliding with its neighbours.
    #[test]
    fn a_badge_filename_of_an_unexpected_shape_still_gets_its_own_slug() {
        assert_ne!(
            rank_slug(Rank::Bronze, "bronze.png"),
            rank_slug(Rank::Bronze, "bronze-2.png")
        );
    }
}
