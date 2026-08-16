//! Hero portraits and map thumbnails for the draft screen.
//!
//! Sources, named honestly: the portraits are Blizzard's own hero art, served
//! from their CDN and indexed by the OverFast API; the map screenshots are
//! hosted by OverFast itself. Neither is ours.
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

#[derive(Default)]
pub struct Report {
    pub heroes: usize,
    pub maps: usize,
    pub changed: usize,
    pub removed: usize,
    /// Keys the dataset has but no artwork exists for. The UI falls back to the
    /// name alone for these, so a gap costs an icon rather than a hero.
    pub missing: Vec<String>,
}

/// Downloads, resizes and writes every portrait and map thumbnail.
///
/// `hero_keys` and `map_keys` come from the committed TOML, which is what
/// decides coverage: artwork is only fetched for keys the dataset knows, and
/// artwork for a key the dataset dropped is deleted.
pub async fn build(
    fetcher: &mut Fetcher,
    web_assets: &Path,
    hero_keys: &HashSet<String>,
    map_keys: &HashSet<String>,
) -> Result<Report> {
    let (portraits, screenshots) = overfast::art_urls(fetcher).await?;

    let hero_dir = web_assets.join("heroes");
    let map_dir = web_assets.join("maps");
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

    // Individual gaps are tolerable; nothing at all means the sources moved and
    // the UI would silently lose every image.
    anyhow::ensure!(
        report.heroes > 0 && report.maps > 0,
        "no artwork could be fetched at all - has the OverFast schema or CDN changed?"
    );

    // A hero or map that left the dataset would otherwise leave its art behind
    // forever: the asset directory ships whole, so orphans cost bundle size.
    report.removed =
        prune(&hero_dir, "webp", hero_keys).await? + prune(&map_dir, "webp", map_keys).await?;

    report.missing.sort_unstable();
    Ok(report)
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
