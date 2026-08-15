set shell := ["bash", "-uc"]

web_out := "target/dx/overwatch-web/release/web/public"

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
    cd overwatch-web && dx build --platform web --release --debug-symbols false
    # The service worker and manifest go to the bundle root rather than through
    # `asset!()`: a service worker only controls the scope it is served from, so
    # a hashed path under /assets would control nothing.
    cp overwatch-web/assets/sw.js {{web_out}}/sw.js
    cp overwatch-web/assets/manifest.json {{web_out}}/manifest.json
    # Same reason: index.html and the manifest both point at /icon.svg.
    cp overwatch-web/assets/icon.svg {{web_out}}/icon.svg
    @echo "bundle: $(du -sh {{web_out}} | cut -f1)"

# Serve the app and the sync socket on the LAN
serve: build-web
    cargo run --release -p overwatch-server

# Client only, with hot reload and no sync
dev:
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

# Hero portraits and map thumbnails into overwatch-web/assets
ingest-art:
    cargo run -p overwatch-ingest -- art

# Counter matrix only, from the community sites
ingest-counters:
    cargo run -p overwatch-ingest -- counters

# Re-fetch every source, ignoring the cache
ingest-refresh:
    # Slow and hits the sites. Use when a patch has landed and the data is
    # genuinely stale, or to re-check heroes a source did not have last time.
    cargo run -p overwatch-ingest -- all --refresh

# Show what the last ingest changed
ingest-diff:
    @git diff --stat -- data/ 2>/dev/null || echo "not a git repository yet"

# Summarise the recorded match results
matches:
    @cat data/matches.jsonl 2>/dev/null | wc -l | xargs -I{} echo "{} matches recorded"
