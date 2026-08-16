<p align="center">
  <img src="overwatch-web/assets/logo.svg" alt="minmax.watch" width="380">
</p>

<p align="center"><strong>Keyboard-first Overwatch 2 draft assistant.</strong><br>
Hero select is seconds long, so the scoring engine is compiled into the client and
runs locally on every pick.</p>

---

The LAN sync server only moves session state between the people drafting; it is
never in the path between a keystroke and an answer.

| Crate | What it is |
| --- | --- |
| `overwatch-core` | Domain model and scoring engine. I/O-free, so it compiles to `wasm32`. |
| `overwatch-data` | Loads the committed dataset in `data/` into a `Dataset`. |
| `overwatch-ingest` | Regenerates `data/*.toml`, the art assets, and the brand rasters. |
| `overwatch-server` | LAN sync socket, and serves the wasm bundle. |
| `overwatch-web` | The draft screen (Dioxus). |

## Prerequisites

- **Rust 1.88 or newer** — the workspace sets `rust-version = "1.88"`.
- **The `wasm32-unknown-unknown` target**, which the client compiles to:

      rustup target add wasm32-unknown-unknown

- **`dx`, the Dioxus CLI** — `just build-web` (and therefore `just serve`)
  shells out to `dx build`. Without it the recipe fails with
  `dx: command not found`. Install it pinned to the same version as the
  `dioxus` dependency, currently 0.7.10:

      cargo install dioxus-cli@0.7.10 --locked

  A CLI whose minor version disagrees with the `dioxus` crate is not expected to
  work, so bump both together.

- **`just`**, to run the recipes below.

## Running it

    just serve   # build the release bundle, then serve it and the sync socket on the LAN
    just dev     # client only, with hot reload and no sync
    just url     # print the addresses to open

`just serve` prints both a `localhost` address and the LAN address the other
person should open. `just --list` shows every recipe, including the `ingest-*`
family that regenerates `data/`.

### Configuration

The server reads three optional environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `OVERWATCH_ADDR` | `0.0.0.0:8080` | A full socket address. A bare port or bare IP does not parse and silently falls back to the default. |
| `OVERWATCH_ASSETS` | `target/dx/overwatch-web/release/web/public` | The bundle to serve. Relative to the working directory. |
| `OVERWATCH_MATCH_LOG` | `data/matches.jsonl` | Where match results are appended. **Set it to the empty string to switch the match log off**, which makes `/api/matches` a 404 in both directions. |

> **On exposing it.** There is no authentication: `POST /api/session` mints rooms
> and `/ws/{room}` is joinable by code with no rate limiting. A session code is a
> convenience for finding the right draft, not a secret. Sessions are capped at
> 1024, and a code nobody opens is dropped after two minutes and evicted first
> when the map is full, so minting them cannot push a real team out — but nothing
> caps concurrent sockets.
>
> The match log is the part that is actually personal — `GET /api/matches`
> returns the whole history and `POST` appends to it — so the public deployment
> runs with `OVERWATCH_MATCH_LOG=""` and the endpoint closed. Anything else you
> put behind a public address needs the rest dealt with first.

`GET /health` answers with the running commit and a census of the sessions:

```
$ curl -s localhost:8080/health
{"status":"ok","rooms":7,"claimed":3,"active":2,"connected":5,"capacity":1024,"build":"7a03cac"}
```

`rooms` against `capacity` is the memory picture. The other three separate the
states a room can be in, and the gaps are the useful part: `rooms` well above
`claimed` is codes being minted and never opened, `claimed` above `active` is
drafts riding out their grace period, and `connected` counts people rather than
sessions. Sweeping is lazy, so an idle server keeps reporting rooms whose time
is up until the next person creates or joins one — the memory is still held at
that point, so the count is honest rather than stale.

## Deployment

`minmax.watch` runs from a container built by
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) on every push to `main`
and published to `ghcr.io/maikbuse/minmax-watch:<sha>-amd64`. The deployment
manifests live outside this repository, and
image-updater tooling promotes each new build.

The [`Dockerfile`](Dockerfile) reproduces the `build-web` recipe from the
`justfile`. **The two have to stay in step** — anything added to the recipe's
list of root-path assets has to be added to the Dockerfile too, or it works
locally and 404s in production. The image additionally brotli-compresses the
bundle, which `just serve` does not do.

## Checks

    just check   # everything CI would run: fmt-check, lint, test, wasm-check

## Brand

The palette, type scale and logo rules live in [docs/BRAND.md](docs/BRAND.md).
`assets/icon.svg` and `assets/og.svg` are the source artwork; every PNG and the
favicon are derived from them:

    just brand-icons   # rasterise the SVGs; writes only what actually changed

## License

MIT — see [LICENSE](LICENSE).

Inter is bundled under the SIL Open Font License 1.1
([overwatch-web/assets/fonts/LICENSE-Inter.txt](overwatch-web/assets/fonts/LICENSE-Inter.txt)).

Not affiliated with or endorsed by Blizzard Entertainment. Overwatch, and the
hero and map artwork regenerated by `just ingest-art`, are Blizzard's.
