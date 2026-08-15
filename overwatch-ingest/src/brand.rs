//! Rasterises the committed brand SVGs into the PNG and ICO files the web
//! bundle serves.
//!
//! The SVGs in `overwatch-web/assets` are the source of truth; everything this
//! module writes is derived and can be deleted and regenerated. It lives here
//! rather than in a shell recipe so the set is reproducible from `cargo` alone:
//! the repo has no node toolchain, and requiring ImageMagick and librsvg to
//! change an icon is a prerequisite nobody remembers a year later.
//!
//! The wordmark SVGs carry their type as outlines rather than `<text>`, so no
//! font has to be installed for this to render correctly. See `docs/BRAND.md`.

use std::path::Path;

use anyhow::{Context, Result};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

/// One derived file: which source, how big, where it goes.
struct Target {
    source: &'static str,
    out: &'static str,
    width: u32,
    height: u32,
}

const TARGETS: &[Target] = &[
    // Android's install prompt wants a PNG; it ignores the SVG entry.
    Target {
        source: "icon.svg",
        out: "icon-192.png",
        width: 192,
        height: 192,
    },
    Target {
        source: "icon.svg",
        out: "icon-512.png",
        width: 512,
        height: 512,
    },
    // iOS ignores both the manifest icons and SVG favicons. Without this, "add
    // to home screen" saves a screenshot of the page instead of the mark.
    Target {
        source: "icon.svg",
        out: "apple-touch-icon.png",
        width: 180,
        height: 180,
    },
    Target {
        source: "og.svg",
        out: "og.png",
        width: 1200,
        height: 630,
    },
];

/// Sizes packed into `favicon.ico`. 16 and 32 are what browsers actually ask
/// for; 48 is what Windows uses for a pinned shortcut.
const ICO_SIZES: &[u32] = &[16, 32, 48];

/// Renders an SVG file at an explicit pixel size.
fn render(source: &Path, width: u32, height: u32) -> Result<Pixmap> {
    let data = std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
    let tree = Tree::from_data(&data, &Options::default())
        .with_context(|| format!("parsing {}", source.display()))?;

    let size = tree.size();
    let mut pixmap = Pixmap::new(width, height)
        .with_context(|| format!("allocating {width}x{height} pixmap"))?;

    // Scale each axis independently rather than uniformly: the OG card is
    // 1200x630 and a uniform fit would letterbox it.
    let transform =
        Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Packs already-encoded PNGs into an ICO container.
///
/// Written out by hand rather than pulled from a crate: the format is a 6-byte
/// header, a 16-byte directory entry per image, and then the payloads verbatim.
/// PNG-compressed entries have been supported by every browser since IE11, so
/// there is no BMP path to get wrong.
fn pack_ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = images.len() as u16;
    let mut out = Vec::new();

    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 = icon
    out.extend_from_slice(&count.to_le_bytes());

    // Payloads start after the header and the whole directory.
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        // 0 means 256 in this field; none of our sizes reach it, but the cast
        // is what makes that true rather than an accident.
        out.push(if *size >= 256 { 0 } else { *size as u8 }); // width
        out.push(if *size >= 256 { 0 } else { *size as u8 }); // height
        out.push(0); // palette size, 0 for truecolour
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }

    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}

/// Regenerates every derived brand asset. Returns the names that changed.
pub fn generate(assets_dir: &Path) -> Result<Vec<String>> {
    let mut changed = Vec::new();

    for target in TARGETS {
        let pixmap = render(&assets_dir.join(target.source), target.width, target.height)?;
        let png = pixmap
            .encode_png()
            .with_context(|| format!("encoding {}", target.out))?;
        if write_if_changed(&assets_dir.join(target.out), &png)? {
            changed.push(target.out.to_owned());
        }
    }

    let icon = assets_dir.join("icon.svg");
    let mut frames = Vec::new();
    for &size in ICO_SIZES {
        let png = render(&icon, size, size)?
            .encode_png()
            .with_context(|| format!("encoding {size}px favicon frame"))?;
        frames.push((size, png));
    }
    if write_if_changed(&assets_dir.join("favicon.ico"), &pack_ico(&frames))? {
        changed.push("favicon.ico".to_owned());
    }

    Ok(changed)
}

/// Writes only when the bytes differ, so re-running leaves mtimes — and the git
/// diff — alone. The same contract as `cache::write_if_changed`, but for bytes
/// rather than text.
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool> {
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(false);
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}
