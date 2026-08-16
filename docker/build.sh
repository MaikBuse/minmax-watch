#!/usr/bin/env bash
#
# Container build step. Run from the workspace root by the Dockerfile, inside a
# layer whose target/ and cargo registry are BuildKit cache mounts.
#
# This is the `build-web` recipe in the justfile plus the two things only the
# container wants: a release build of the server alone, and a precompressed
# bundle. Keep it in step with that recipe — an asset added there and forgotten
# here works under `just serve` and 404s in production.
set -euo pipefail

web_out=target/dx/overwatch-web/release/web/public

# `--debug-symbols false` is load-bearing rather than a size tweak. dx defaults
# it to true even for a release build, and the DWARF rustc emits is newer than
# the Binaryen dx ships can parse: wasm-opt aborts with "compile unit size was
# incorrect", dx logs it and carries on, and the bundle silently ships
# unoptimised. Worth 1.6M -> 1.2M.
#
# dx has to run from overwatch-web/ but writes to the workspace-root target/.
(cd overwatch-web && dx build --platform web --release --debug-symbols false)

# The service worker and manifest belong at the bundle root rather than behind
# asset!(): a service worker only controls the scope it is served from, so a
# content-hashed path under /assets would control nothing. The rest are each
# referenced by an absolute root path — from index.html, from manifest.json, or
# by a crawler that only ever looks at /favicon.ico and /robots.txt.
cp overwatch-web/assets/sw.js                "$web_out/sw.js"
cp overwatch-web/assets/manifest.json        "$web_out/manifest.json"
cp overwatch-web/assets/icon.svg             "$web_out/icon.svg"
cp overwatch-web/assets/logo.svg             "$web_out/logo.svg"
cp overwatch-web/assets/favicon.ico          "$web_out/favicon.ico"
cp overwatch-web/assets/apple-touch-icon.png "$web_out/apple-touch-icon.png"
cp overwatch-web/assets/icon-192.png         "$web_out/icon-192.png"
cp overwatch-web/assets/icon-512.png         "$web_out/icon-512.png"
cp overwatch-web/assets/og.png               "$web_out/og.png"
cp overwatch-web/assets/robots.txt           "$web_out/robots.txt"

# The stylesheet's @font-face points at /fonts/, for the same reason.
mkdir -p "$web_out/fonts"
cp overwatch-web/assets/fonts/inter-latin.woff2 "$web_out/fonts/inter-latin.woff2"
cp overwatch-web/assets/fonts/LICENSE-Inter.txt "$web_out/fonts/LICENSE-Inter.txt"

# The artwork, for a third reason: dx re-encodes every image it bundles, and the
# only WebP its encoder can write is lossless, so routing these through asset!()
# inflates the 700K of art back to 4M. See overwatch-web/src/icons.rs.
mkdir -p "$web_out/heroes" "$web_out/maps"
cp overwatch-web/assets/heroes/*.webp "$web_out/heroes/"
cp overwatch-web/assets/maps/*.webp   "$web_out/maps/"

# `-p`, not `--workspace`: overwatch-ingest is the dev-only scraper and pulls in
# reqwest, scraper, image and resvg, none of which the deployed server needs.
cargo build --release --locked -p overwatch-server

# Out of the cache mount and into the image. Everything below this line has to
# live under /out or it does not reach the runtime stage.
mkdir -p /out/bin /out/public
cp target/release/overwatch-server /out/bin/overwatch-server
cp -a "$web_out/." /out/public/

# Precompress rather than compressing per request: the server's ServeDir picks
# up the .br and .gz siblings, so this buys -q 11 for a one-off build cost
# instead of a worse ratio charged to every visitor's CPU time.
#
# Only the compressible types. The WebP artwork and the woff2 are already
# compressed; a sibling for those would add image size to deliver the same bytes.
# The wasm is the one that matters — around 1.4M raw, on the critical path of
# every first visit.
find /out/public -type f \
  \( -name '*.wasm' -o -name '*.js' -o -name '*.css' -o -name '*.html' \
  -o -name '*.json' -o -name '*.svg' -o -name '*.txt' \) \
  -print0 > /tmp/compressible

xargs -0 -r brotli -q 11 -k -f < /tmp/compressible
xargs -0 -r gzip -9 -k -f < /tmp/compressible

echo "bundle: $(du -sh /out/public | cut -f1)"
