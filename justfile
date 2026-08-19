set shell := ["bash", "-uc"]

web_out := "target/dx/overwatch-web/release/web/public"
web_out_debug := "target/dx/overwatch-web/debug/web/public"

default:
    @just --list

# --- checks -----------------------------------------------------------------

# Everything CI would run
check: fmt-check lint test wasm-check

# Run the whole test suite
test:
    cargo test --workspace

# Clippy, warnings denied
lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# Prove the shared crates still compile to wasm
wasm-check:
    cargo check -p overwatch-core -p overwatch-data --target wasm32-unknown-unknown

# --- build & run ------------------------------------------------------------

# Build the release client bundle
#
# docker/build.sh is the same sequence for the container image. The two have to
# stay in step: an asset added here and forgotten there works under `just serve`
# and 404s in production.
build-web:
    # dx writes content-hashed filenames and never removes the old ones, so the
    # output directory otherwise accumulates a copy of every past build.
    rm -rf {{web_out}}
    # `--debug-symbols false` is load-bearing, not a size tweak. dx defaults it
    # to true even for a release build, and the DWARF that rustc emits is newer
    # than the Binaryen dx ships can parse: wasm-opt aborts with "compile unit
    # size was incorrect", dx logs it and carries on, and the bundle silently
    # ships unoptimised. Setting it false means there is no DWARF to trip over.
    # (wasm-opt is already passed --strip-debug, but it parses the debug info
    # before stripping it, so the strip does not save it.) Worth 1.6M -> 1.2M.
    #
    # MINMAX_BUILD is what the footer shows and what `/health` reports; unset, it
    # reads "dev". It has to be on this line rather than exported above, because
    # `set shell` makes every recipe line its own shell. `|| echo dev` keeps the
    # recipe working outside a checkout — `-u` would otherwise abort on nothing.
    cd overwatch-web && MINMAX_BUILD="$(git rev-parse HEAD 2>/dev/null || echo dev)" dx build --platform web --release --debug-symbols false
    # The service worker and manifest go to the bundle root rather than through
    # `asset!()`: a service worker only controls the scope it is served from, so
    # a hashed path under /assets would control nothing.
    cp overwatch-web/assets/sw.js {{web_out}}/sw.js
    cp overwatch-web/assets/manifest.json {{web_out}}/manifest.json
    # Same reason: every one of these is referenced by an absolute root path —
    # from index.html, from manifest.json, or by a crawler that only ever looks
    # at /favicon.ico and /robots.txt. `asset!()` cannot deliver any of them,
    # because it content-hashes the name and nothing here can know the hash.
    #
    # Anything added to index.html or manifest.json as an absolute path has to
    # be added here too, and to docker/build.sh, or it 404s in that bundle.
    # Note the direction: it is this list that makes them exist. `dx serve`
    # copies none of them, so under `just dev` every one of these paths 404s —
    # which is why nothing on the first-paint path may depend on one.
    cp overwatch-web/assets/icon.svg {{web_out}}/icon.svg
    cp overwatch-web/assets/logo.svg {{web_out}}/logo.svg
    cp overwatch-web/assets/favicon.ico {{web_out}}/favicon.ico
    cp overwatch-web/assets/apple-touch-icon.png {{web_out}}/apple-touch-icon.png
    cp overwatch-web/assets/icon-192.png {{web_out}}/icon-192.png
    cp overwatch-web/assets/icon-512.png {{web_out}}/icon-512.png
    cp overwatch-web/assets/og.png {{web_out}}/og.png
    cp overwatch-web/assets/robots.txt {{web_out}}/robots.txt
    # The stylesheet's @font-face points at /fonts/, for the same reason.
    mkdir -p {{web_out}}/fonts
    cp overwatch-web/assets/fonts/inter-latin.woff2 {{web_out}}/fonts/inter-latin.woff2
    cp overwatch-web/assets/fonts/LICENSE-Inter.txt {{web_out}}/fonts/LICENSE-Inter.txt
    # The artwork, for a third reason: dx re-encodes every image it bundles, and
    # the only WebP its encoder can write is lossless — which turns a 13K map
    # thumbnail back into 93K. See overwatch-web/src/icons.rs.
    just _art {{web_out}}
    @echo "bundle: $(du -sh {{web_out}} | cut -f1)"

# Copy the hero, map and rank artwork to the bundle root, unmodified.
#
# Its own recipe because `dev` needs it too: `dx serve` does not run the copies
# above, so without this the draft screen comes up with no portraits on it.
_art out:
    mkdir -p {{out}}/heroes {{out}}/maps {{out}}/ranks
    cp overwatch-web/assets/heroes/*.webp {{out}}/heroes/
    cp overwatch-web/assets/maps/*.webp {{out}}/maps/
    cp overwatch-web/assets/ranks/*.webp {{out}}/ranks/

# Serve the app and the sync socket on the LAN
serve: build-web
    # The same stamp the bundle got above. The server is a separate compile, so
    # leaving it off here would have `/health` say "dev" while the footer on the
    # page it is serving says the sha — the one thing this feature must not do.
    MINMAX_BUILD="$(git rev-parse HEAD 2>/dev/null || echo dev)" cargo run --release -p overwatch-server

# Client only, with hot reload and no sync
dev:
    # dx serve never empties its output directory, so seeding the artwork once
    # before it starts is enough — a rebuild leaves these in place.
    just _art {{web_out_debug}}
    cd overwatch-web && dx serve --platform web

# Print the URLs to open
url:
    @echo "this machine   http://localhost:8080"
    @echo "this network   http://$(hostname -I | awk '{print $1}'):8080"
    @grep -qiE 'microsoft|wsl' /proc/version 2>/dev/null \
        && echo "(under WSL the network address is WSL's own - see the notes printed by \`just serve\`)" \
        || true

# --- data -------------------------------------------------------------------

# Regenerate data/*.toml, then review the diff
ingest:
    # The diff review *is* the curation step. Cached responses in data/sources
    # make a second run free.
    cargo run -p overwatch-ingest -- all

# Roster and maps only, from the OverFast API
ingest-roster:
    cargo run -p overwatch-ingest -- roster

# Hero portraits, map thumbnails and rank badges into overwatch-web/assets
ingest-art:
    cargo run -p overwatch-ingest -- art

# Counter matrix only, from the community sites. Curated rows survive this.
ingest-counters:
    cargo run -p overwatch-ingest -- counters

# Duo synergies only. Curated rows in synergy.toml survive this.
ingest-synergy:
    cargo run -p overwatch-ingest -- synergy

# Win rates, map affinity and the rank slices, without re-scraping the whole
# matrix. The rank half costs nine extra requests to Blizzard; the counterwatch
# half comes off the stats pages this step already fetches.
ingest-strength:
    cargo run -p overwatch-ingest -- strength

# Re-fetch every source, ignoring the cache
ingest-refresh:
    # Slow and hits the sites. Use when a patch has landed and the data is
    # genuinely stale, or to re-check heroes a source did not have last time.
    cargo run -p overwatch-ingest -- all --refresh

# --- brand ------------------------------------------------------------------

# Rasterise the brand SVGs into favicons, PWA icons and the OG card
brand-icons:
    # Sources are assets/icon.svg and assets/og.svg; everything this writes is
    # derived and safe to delete. Local only - no network - and it rewrites a
    # file only when the bytes actually change, so a no-op run leaves the git
    # diff empty. Run it after editing either SVG.
    cargo run -p overwatch-ingest -- brand

# Re-run the blend over the columns already in data/matchups.toml
reblend:
    # No network. Every committed value is reproducible from the per-source
    # columns beside it, so a change to the blend reviews as a diff of exactly
    # the rows the blend moved. A second run must produce an empty diff.
    #
    # Curated rows survive this, and that is not free: the blend emits nothing at
    # all for a pair no source rated, so `merge_matchups` puts them back.
    cargo run -p overwatch-ingest -- reblend

# Show what the last ingest changed
ingest-diff:
    @git diff --stat -- data/ 2>/dev/null || echo "not a git repository yet"

# Summarise the recorded match results
matches:
    @cat data/matches.jsonl 2>/dev/null | wc -l | xargs -I{} echo "{} matches recorded"
