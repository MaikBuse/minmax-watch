# MinMax brand guide

The source of truth is `overwatch-web/assets/style.css`. This file explains the
reasoning; when the two disagree, the stylesheet is right and this is stale.

## The name

**MinMax.** The wordmark is `minmax.watch` — the domain *is* the mark, because
the `.watch` TLD is the joke rather than an address bolted onto a logo.

"Overwatch" is never the product name. It is Blizzard's trademark, and using it
as our own name is the form that actually creates exposure. Using it to say what
the app is for — "Overwatch 2 draft assistant" — is nominative use and is fine.

| Surface | String |
| --- | --- |
| Browser tab | `MinMax — Overwatch 2 draft assistant` (set in `Dioxus.toml`, **not** `index.html`) |
| PWA name / short name | `MinMax` |
| Tagline | `Overwatch 2 draft assistant` |
| Supporting line | `scored locally · shared by code · nothing to install` |
| Description | `Overwatch 2 draft assistant. Counter picks, hero matchups and team synergy, scored in your browser and shared with your team by session code.` |

> `index.html` carries an empty `<title></title>` on purpose: `dx` appends the
> Dioxus.toml title *into* that element, so text in both places duplicates.

> The description is the one string written for a search box rather than for a
> reader: "counter picks" and "hero matchups" are in it because those are the
> words a player types. It is capped at ~155 characters, which is where Google
> truncates, and it is repeated in **five** places that have no way to notice
> they disagree — `index.html` three times (`description`, `og:description`,
> `twitter:description`) plus the JSON-LD block, and `manifest.json`. Change one,
> change all five. `overwatch-web/Cargo.toml` carries its own shorter variant for
> `cargo metadata`; it is not user-facing and does not have to match verbatim.

## Direction

Tactical HUD. Deep charcoal, hairline rules, hard corners, tabular numerals, and
saturation spent only where it encodes something. It should read as an overlay
you glance at during a thirty-second hero select, not as a marketing page.

Two rules carry over from the original stylesheet and are not negotiable:

1. **Colour never carries meaning alone.** Every hue is paired with a label, a
   glyph, a leading sign, an underline bar, or a contrast step, so the screen
   survives colour-blindness.
2. **Nothing appears or disappears mid-draft.** Unavailable things are drawn
   dimmed, not removed — a portrait stays where your hand learned it.

## Palette

Dark only. There is no light theme and none is planned; the app is used over a
game running full-screen.

### Surfaces

| Token | Value | Use |
| --- | --- | --- |
| `--bg` | `#0b0d11` | page |
| `--surface` | `#12151c` | panels, boards, the session bar |
| `--surface-raised` | `#181c25` | hover on a recommendation row |
| `--line` | `#242a36` | every border, chip background, art placeholder |
| `--line-strong` | `#333b4a` | hover borders, footer rules |
| `--overlay` | `#05070b` | the tile hover label — darker than the page, so it beats the artwork behind it |
| `--on-color` | `#0b1220` | text placed *on* a saturated fill |

### Text

| Token | Value | On `--bg` | On `--surface` |
| --- | --- | --- | --- |
| `--text` | `#e6eaf2` | 16.13:1 | 15.15:1 |
| `--text-muted` | `#8b94a7` | 6.38:1 | 5.99:1 |
| `--text-faint` | `#767f96` | 4.86:1 | 4.56:1 |

`--text-faint` is as dark as it can be while still clearing **WCAG AA (4.5:1) on
both surfaces**. Everything wearing it is 11–12px, which is exactly the size
that cannot afford a grey picked by eye. If you darken it, you have broken it.

### Brand and roles

| Token | Value | Contrast on `--surface` | Use |
| --- | --- | --- | --- |
| `--accent` | `#38bdf8` | 8.53:1 | focus rings, session code, active side, map title, the mark |
| `--accent-soft` | `#7dd3fc` | — | tints only |
| `--role-tank` | `#60a5fa` | 7.18:1 | tank |
| `--role-damage` | `#f87171` | 6.60:1 | damage |
| `--role-support` | `#4ade80` | 10.48:1 | support |

Role hues match what the game already trains you on; they are worth matching
rather than reinventing.

### Semantics and teams

| Token | Value | Contrast on `--surface` | Use |
| --- | --- | --- | --- |
| `--positive` | `#4ade80` | 10.48:1 | a reason in your favour, a swap, a win |
| `--negative` | `#f87171` | 6.60:1 | a reason against, a loss, an armed reset |
| `--warn` | `#fbbf24` | 10.94:1 | **"mine"** — pool markers, the star, a locked pick |
| `--ally` | `#60a5fa` | 7.18:1 | ally board |
| `--enemy` | `#fb7185` | 6.79:1 | enemy board |

**Why these are separate names from the roles above.** They used to be the same
tokens: `--enemy`, `--bad` and `--damage` were one `#f87171`, and `--ally` and
`--tank` were one `#60a5fa`. That made an enemy board tinted identically to a
damage-role tile sitting next to it and to a negative reason below it — three
unrelated facts sharing a hue, none of which could move without dragging the
other two. Splitting the names is the durable half of the change; shifting
`--enemy` to rose is what makes the split visible.

Every pair in these tables clears WCAG AA. Re-run the check before changing any
value; the ratios above are measured, not asserted.

## Type

**Inter**, latin subset, variable weights 400–700, self-hosted at
`/fonts/inter-latin.woff2` (48K). Self-hosting is not a preference: this is an
offline-first PWA whose whole argument is that the network is never between a
keystroke and an answer, and a webfont fetched from a third party at load would
put it there.

The **system monospace stack** carries the score column and the session code. A
second family is not worth the bytes in a bundle tuned at `opt-level = "s"`, and
`tabular-nums` already does most of the work.

Six steps, down from the fifteen ad-hoc sizes the screen used to mix:

| Token | Size | Use |
| --- | --- | --- |
| `--fs-micro` | 11px | tags, mode counts, board roles, footer, reset |
| `--fs-xs` | 12px | hints, status pills, panel headings |
| `--fs-sm` | 13px | reasons, ban text, empty states, inputs |
| `--fs-base` | 14px | mode segments, header context, body |
| `--fs-md` | 16px | the score, the session code, the pool star |
| `--fs-lg` | 17px | map name, recommendation name |

Weights are 600 and 700 only. Uppercase micro-labels take `0.04`–`0.08em`
tracking. Every number takes `font-variant-numeric: tabular-nums`, because these
are compared down a column.

### Voice

All lowercase, terse, no marketing. The existing copy is the reference:
*"nothing here beats you — no ban worth spending"*, *"picking…"*, *"sure?"*,
*"every hero in this role is already on your team"*.

State what is true, including when it is nothing. "solo" and "offline" are
information, not errors — scoring is local either way — and they are styled
muted rather than alarming.

## Space and shape

    --sp-1: 2px   --sp-2: 4px   --sp-3: 8px
    --sp-4: 12px  --sp-5: 16px  --sp-6: 24px

    --r-sm: 2px   --r-md: 4px   --r-lg: 6px   --r-pill: 999px

Tighter than the 3/4/5/6/8px the screen used to mix — a HUD wants harder
corners. Deliberately *not* square: the 36px tiles are Blizzard portrait art and
a zero-radius crop on them looks harsh.

## Sizes

Every media query lives in one `--- sizes ---` section at the foot of
`style.css`, and **nothing may be appended after it** — a plain rule written
below would silently outrank every override inside it, same specificity and
later source, with no `!important` anywhere to make that visible.

The breakpoints are not round numbers. Each is the width at which one specific
thing stops fitting:

| Width | What changes | Why that number |
| --- | --- | --- |
| `≥ 1600px` | shell cap 1400 → 1680 | Below it the cap is spending a quarter of a 1920px screen on gutter. The width goes to the boards, where it buys tiles per row and a shorter page. |
| `≤ 880px` | boards go one column | Where a two-column board stops fitting eight tiles across: a column offers `(vw - 42)/2 - 96`, and eight tiles need `8×36 + 7×4 = 316`. Below it, two boards side by side come to nine tile rows where one full-width board stacked twice comes to eight — so stacking is the *denser* layout here. |
| `≤ 700px` | pick column goes one column | These panels are text, not artwork. What used to break them earlier was `.rec-head` refusing to wrap, which pushed the score out of the panel instead of onto a second line. |
| `≤ 560px` | phone | Role labels move above their tile grids, page padding drops to 8px, the session inputs take a line each. |

Two things are keyed on the **pointer**, not the width, because a phone that is
turned over crosses a width breakpoint mid-draft and must not change what is on
the screen when it does:

- `(hover: none) and (pointer: coarse)` — 40px tiles, 44px minimum on everything
  you aim at. Both conditions, never `pointer: coarse` alone: a touchscreen
  laptop with a trackpad reports a coarse *secondary* pointer, and 40px tiles on
  a 1920px screen with a mouse on it is a worse answer than 36.
- The same, plus `≤ 950px` — the answer strip. 950 keeps it through an iPhone's
  landscape (932) and off an iPad's (1024).

**40px and not 44.** 44 with its gap costs a whole extra row on the damage board
at 360px, which is a scroll tax charged for a tap that already lands. 40 + 6 is a
46px pitch, and pitch — not the drawn box — is what WCAG 2.5.8 measures.

### Touch is a third pointer, not a narrow mouse

Three things on this screen were reachable only with a cursor, and each needed
its own answer rather than a smaller breakpoint:

- **A tile's name.** It is bare artwork; the name is a `::after` on `:hover`.
  `:focus` cannot carry it — a click anywhere inside `.app` is caught by the
  root's own handler re-taking focus so the chords keep working, so a tapped
  tile holds focus for less than a frame. `:active` is the answer, and it makes
  press-and-hold the gesture: hold to read, slide off to put it back, lift to
  pick. `:focus-visible` was added at the same time, for the keyboard.
- **The attack/defend toggle.** It hides once a side is chosen and returns on
  hover or focus-within — but the only thing inside the slot that can take focus
  is the tile, and tapping that clears the map it is configuring. On a
  touchscreen it stays up.
- **The resting tile dimming.** 50% opacity is restored by a mouse passing over.
  Nothing restores it on a touchscreen, so the whole roster sat permanently
  dimmed and the dimming stopped meaning "not picked".

### The answer strip

The one component that exists on one class of device. On a phone every panel
stacks and the pick column — the thing the app is for — ends up below six of
them. The strip pins its top three to the foot of the viewport.

It does not break *"nothing appears or disappears mid-draft"*: it is rendered
unconditionally and shown by a media query on the pointer, so nothing that can
change during a draft can remove it — not a pick, not a role, not turning the
phone over. When there is nothing to suggest it says so in the same words the
panel it mirrors uses, rather than collapsing.

It takes the same `RecRow` the pick column does. Two renderings of one list are
already a risk; two *resolutions* of it would be a build away from disagreeing
about what the best pick is.

### The root font-size is a preference, not a constant

`html` is `font-size: 100%`, never a hard pixel value. Every step in the type
scale is in `rem`, so pinning the root reduces a browser's own text-size setting
to nothing. This is also why `--role-gutter` is `6em` rather than the 66px it
used to be: at 200% text zoom, 66 hard pixels clip "SUPPORT 2" while still
charging the tiles for all 66.

## The mark

`assets/icon.svg` — three role arcs (tank, damage, support, in mode-switch
order) ringing an accent reticle, on a full-bleed panel gradient.

- Original artwork, never a hero portrait. The portraits are Blizzard's; the
  app's identity should be ours.
- Drawn for `purpose: maskable` — full-bleed background so a circular mask never
  cuts to transparency, every mark inside the central 80% safe zone.
- **In the app, the glyph only.** The header already carries the mode switch,
  the map, the side toggle, the sync light, the ingest date and a reset. A
  screen whose entire argument is density cannot spend a hundred pixels naming
  the app you already opened. The wordmark works on the tab, the install prompt
  and the link preview instead.

`assets/logo.svg` is the wordmark lockup: mark, then `minmax` in `--text` and
`.watch` in `--accent`. Minimum width 200px; keep clear space of one reticle
diameter on every side.

### Why the type is outlines

Both `logo.svg` and `og.svg` carry their text as vector outlines rather than
`<text>`. An SVG loaded through `<img>` is an isolated document that cannot see
the page's `@font-face`, and the rasteriser has no Inter installed — live text
would silently fall back to whatever the viewer happened to have.

They were generated once with `fontTools`, from the same woff2 the app ships:

```python
# pip install fonttools brotli
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools.pens.svgPathPen import SVGPathPen

font = TTFont("overwatch-web/assets/fonts/inter-latin.woff2")
instancer.instantiateVariableFont(font, {"wght": 600}, inplace=True)
cmap, glyphs, hmtx = font.getBestCmap(), font.getGlyphSet(), font["hmtx"]

x = 0                                   # advance, in font units (upem 2048)
for ch in "minmax.watch":
    name = cmap[ord(ch)]
    pen = SVGPathPen(glyphs)
    glyphs[name].draw(pen)
    print(f'<path transform="translate({x} 0)" d="{pen.getCommands()}" />')
    x += hmtx[name][0]
```

Wrap the result in `<g transform="translate(X Y) scale(s -s)">` — the Y flip is
because font coordinates go up and SVG's go down.

## Derived assets

`icon.svg` and `og.svg` are source. Everything below is generated by
`just brand-icons` (`overwatch-ingest/src/brand.rs`, pure Rust via `resvg`) and
is safe to delete:

| File | From | Why it exists |
| --- | --- | --- |
| `favicon.ico` | `icon.svg` | 16/32/48, PNG-compressed frames. Still what browsers probe for |
| `apple-touch-icon.png` | `icon.svg` | 180×180. iOS ignores manifest icons *and* SVG favicons; without this, "add to home screen" saves a screenshot |
| `icon-192.png`, `icon-512.png` | `icon.svg` | Android's install prompt wants PNG |
| `og.png` | `og.svg` | 1200×630 link preview |

The recipe writes a file only when its bytes change, so a no-op run leaves the
git diff empty.

## The build trap

Everything referenced by an **absolute root path** — from `index.html`, from
`manifest.json`, or by a crawler that only looks at `/favicon.ico` — must be
copied by hand in `justfile`'s `build-web`. `asset!()` cannot deliver them: it
content-hashes the filename, and nothing that references a root path can know
the hash. The service worker has the same constraint for a different reason — it
only controls the scope it is served from.

Add a root asset without adding the `cp`, and it works under `just dev` and
404s in the bundle.

Four places pin the background colour and move together: `style.css` `--bg`,
`index.html` `theme-color`, `manifest.json` (`background_color` *and*
`theme_color`), and `icon.svg`.

Changing `sw.js` means bumping its `CACHE` constant, or clients keep serving the
old shell.

## Attribution

MinMax is MIT. Inter is bundled under the SIL Open Font License 1.1
(`assets/fonts/LICENSE-Inter.txt`).

Hero portraits and map thumbnails are Blizzard's, regenerated by
`just ingest-art` and not rebrandable material. The app footer says so, because
serving them to the open internet is a different posture than serving them
across a LAN.
