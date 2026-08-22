//! Presentational components.
//!
//! These take plain values rather than reaching into shared state, so what each
//! panel depends on is visible in its signature and the whole screen stays
//! re-renderable from one recomputed frame.

use dioxus::prelude::*;
use overwatch_core::{
    ComfortStep, Coverage, Dataset, Format, HeroId, MapId, Queue, Rank, Reason, ReasonKind,
    Recommendation, Role, Side, TeamSize, TermKind, Threat,
};

/// A reset that asks first, for the ones that throw away configuration rather
/// than match state.
///
/// Clearing the enemy board costs you a re-click; clearing your pool costs you
/// the list you built up over weeks, so those arm on the first click and commit
/// on the second. Disarms on blur, so clicking anywhere else calls it off — no
/// timer, and no modal in the middle of a hero select.
#[component]
fn ResetButton(confirm: bool, on_reset: EventHandler<()>) -> Element {
    let mut armed = use_signal(|| false);

    rsx! {
        button {
            class: if armed() { "reset armed" } else { "reset" },
            onfocusout: move |_| armed.set(false),
            onclick: move |evt| {
                // The boards sit inside clickable regions; a reset must not
                // also register as a pick.
                evt.stop_propagation();
                if confirm && !armed() {
                    armed.set(true);
                    return;
                }
                armed.set(false);
                on_reset.call(());
            },
            if armed() { "sure?" } else { "reset" }
        }
    }
}

/// The chords, on hover.
///
/// They used to be a line of prose along the top of the screen, spent on every
/// draft to teach a feature most people never press twice. The people who do
/// press them do not read it either — they already know. So it moved behind a
/// key that reveals it, which costs a hover to the person who wants it and
/// nothing at all to everyone else.
///
/// Revealed on focus as well as hover, which is not optional: a list of
/// keyboard shortcuts that can only be reached with a mouse is a joke at the
/// expense of the people it is for.
#[component]
fn KeyHelp() -> Element {
    rsx! {
        div { class: "keys",
            button {
                class: "keys-button",
                r#type: "button",
                aria_label: "keyboard shortcuts",
                // The sheet is the content; the button only reveals it, so it
                // does nothing on click and must not steal a pick.
                onclick: move |evt| evt.stop_propagation(),
                "⌨"
            }
            div { class: "keys-sheet", role: "tooltip",
                for (chord, what) in crate::keys::SHORTCUTS {
                    div { key: "{chord}", class: "keys-row",
                        kbd { "{chord}" }
                        span { "{what}" }
                    }
                }
            }
        }
    }
}

fn role_class(role: Role) -> &'static str {
    match role {
        Role::Tank => "role-tank",
        Role::Damage => "role-damage",
        Role::Support => "role-support",
    }
}

/// One segment of the mode switch: a role you can pick as, and how much of it
/// you have marked as yours.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeChip {
    pub role: Role,
    /// The spoken word, from [`Role::label`] — "dps", not "damage".
    pub label: String,
    /// How many heroes of this role you have marked as yours.
    ///
    /// A zero is honest: the pool has never restricted what the list shows, so an
    /// empty one costs you the comfort term on that role and nothing else. What
    /// it counts changed underneath this without the number moving — it is now
    /// the heroes with a comfort value above zero, which is the same set the
    /// board draws and the same set the seat publishes.
    pub pool_size: usize,
    /// How many heroes the role has, so the count has something to be out of.
    pub roster_size: usize,
}

/// The app mark: the three role arcs around the reticle, same drawing as
/// `assets/icon.svg`.
///
/// The glyph alone, never the `minmax.watch` wordmark. The header already
/// carries the mode switch, the map, the sync light, the ingest date and a
/// reset, and a screen whose entire argument is density cannot spend a hundred
/// pixels of it naming the app you already opened. The wordmark does its work
/// on the tab, the install prompt and the link preview.
///
/// Drawn inline rather than as an `<img>` so it needs no network round trip and
/// cannot flash in after the first paint.
fn brand_mark() -> Element {
    rsx! {
        svg {
            view_box: "0 0 96 96",
            width: "22",
            height: "22",
            "aria-hidden": "true",
            g {
                fill: "none",
                stroke_width: "10",
                stroke_linecap: "round",
                transform: "rotate(-90 48 48)",
                circle {
                    cx: "48", cy: "48", r: "32",
                    stroke: "var(--role-tank)",
                    stroke_dasharray: "55.85 145.2",
                }
                circle {
                    cx: "48", cy: "48", r: "32",
                    stroke: "var(--role-damage)",
                    stroke_dasharray: "55.85 145.2",
                    stroke_dashoffset: "-67.02",
                }
                circle {
                    cx: "48", cy: "48", r: "32",
                    stroke: "var(--role-support)",
                    stroke_dasharray: "55.85 145.2",
                    stroke_dashoffset: "-134.04",
                }
            }
            g {
                stroke: "currentColor",
                stroke_width: "8",
                stroke_linecap: "round",
                fill: "none",
                circle { cx: "48", cy: "48", r: "10" }
                path { d: "M48 30v-6M48 66v6M30 48h-6M66 48h6" }
            }
        }
    }
}

/// The commit this bundle was compiled from, or `"dev"` when nothing stamped it.
///
/// Set on the `dx build` line in the justfile and exported by `docker/build.sh`
/// from the `MINMAX_BUILD` build arg; `overwatch-server` reads the same variable
/// so the footer and `/health` can never name different commits. `option_env!`
/// rather than `env!` is what keeps `just dev` and a bare `cargo build`
/// compiling, and it is deliberately not a `build.rs` — rustc records the read
/// in its dep-info, so cargo already rebuilds this crate when the value moves.
///
/// A `match` rather than `unwrap_or`, which is not const-stable.
const BUILD: &str = match option_env!("MINMAX_BUILD") {
    Some(sha) => sha,
    None => "dev",
};

/// Every site the committed tables are built from, and what each one gives.
///
/// `(name, url, what it provides)`, held as a table rather than written into the
/// markup so a test can check the panel names all of them —
/// `keys::SHORTCUTS` is the precedent, and the reason is the same: a disclosure
/// that has quietly stopped listing a source is worse than none.
///
/// **Five rows, and the fifth is the argument.** overpicker is fetched, recorded
/// in every row of `matchups.toml` and deliberately not blended, on measured
/// evidence rather than taste. Leaving it out would make the list shorter and
/// the disclosure weaker.
const SOURCES: [(&str, &str, &str); 5] = [
    (
        "OverFast API",
        "https://overfast-api.tekrop.fr",
        "the hero roster, the map list, and the portrait and screenshot artwork",
    ),
    (
        "counterpickgg",
        "https://counterpickgg.com",
        "hero matchups with the written rationale, win and pick rates, best maps",
    ),
    (
        "counterwatch",
        "https://www.counterwatch.gg",
        "hero matchups measured from duels, win rates, best duos, win rate per rank",
    ),
    (
        "Blizzard hero rates",
        "https://overwatch.blizzard.com/en-us/rates/",
        "first-party win, pick and ban rates, per rank division",
    ),
    (
        "overpicker",
        "https://overpicker.com",
        "recorded in every row for comparison, and deliberately not used: its matrix has no measurable relationship to either of the other two",
    ),
];

/// Where the numbers come from, at the foot of the page.
///
/// The provenance was written long before this component was: the readme holds
/// the sources, the blend weights, the evidence overpicker is excluded on and
/// which files are hand-curated judgement. It was reachable only by guessing that
/// a footer link labelled `source` led to a repository whose front page explains
/// the scoring, and the first person to review the app publicly had to go and
/// find it. A tool that argues from numbers has to say whose they are on the
/// screen the numbers are on.
///
/// A native `<details>`, which is a deliberate divergence from the two
/// disclosures already here: `RankPicker` toggles a class off a signal and
/// `KeyHelp` is CSS hover and focus. Both are header controls that must not move
/// what is under them mid-draft. This is a page-foot block that may — it grows
/// the page downward and moves nothing above it, so "nothing appears or
/// disappears mid-draft" holds without a single line of state.
///
/// **Escape is deliberately not bound to close it.** `keys::command_for`
/// answers `Code::Escape` with `Command::Clear` on any modifier, the handler is
/// on `div.app`, and closing a panel by clearing everyone's board is not a
/// trade. Clicking the summary again is the way out, which is what a `<details>`
/// does for free.
///
/// The counts are passed in from the dataset rather than written into the copy,
/// so the sentence about coverage is a measurement of the tables in this bundle.
#[component]
pub fn HowItWorks(generated: String, with_note: usize, rated: usize) -> Element {
    rsx! {
        details { class: "how",
            summary {
                class: "how-summary",
                // `KeyHelp`'s reason: the root's onclick hands focus back to
                // `div.app` on every click, which would take it off the summary
                // the instant it opened and leave Enter and Space doing nothing.
                // The element's own default toggle is unaffected.
                onclick: move |evt| evt.stop_propagation(),
                "where these numbers come from"
            }
            div { class: "how-body",
                h3 { "the score" }
                p {
                    "A weighted sum of eight terms: how strong the hero is in the current patch, \
                     and how far that moves at your rank; its matchups against every enemy you have \
                     entered; rated duos with your allies; how it does on this map; the attack or \
                     defend lean; dive, poke and brawl against the shape of the enemy team; and your \
                     own comfort on it \u{2014} the one term nothing on this screen can set yet. Each \
                     row shows the three terms that moved it most."
                }
                h3 { "the words" }
                p {
                    "The matchup sentences are counterpickgg's, quoted exactly \u{2014} {with_note} of the \
                     {rated} rated pairs have one. The dive/poke/brawl readings and the attack/defend \
                     leans are written by hand in this repository, because no site publishes either, \
                     and the sentence beside each one is shown on the row it argues for. Every other \
                     line under a pick is this app's own wording over its own arithmetic, set in \
                     lowercase so the two are told apart."
                }
                h3 { "the sources" }
                ul { class: "how-sources",
                    for (name, url, what) in SOURCES {
                        li { key: "{name}",
                            a { href: "{url}", rel: "noopener", "{name}" }
                            span { class: "how-what", "{what}" }
                        }
                    }
                }
                p {
                    "Matchups are a weighted average of counterpickgg at 0.75 and counterwatch at \
                     0.25, renormalised over whichever of the two has an opinion about a given pair. \
                     Where they contradict each other the reading is pulled toward even and the row \
                     says so."
                }
                p {
                    "Nothing talks to any of these sites while you draft. The tables are compiled \
                     into this page and the scoring runs in your browser."
                }
                p { class: "how-foot",
                    "counter data ingested {generated} \u{2014} the full method is in the "
                    a {
                        href: "https://github.com/MaikBuse/minmax-watch#where-the-numbers-come-from",
                        rel: "noopener",
                        "readme"
                    }
                }
            }
        }
    }
}

/// The one place the app says who it is and whose artwork it is borrowing.
///
/// The portraits, map shots and rank badges in this bundle are Blizzard's.
/// Serving them
/// across a LAN and serving them to the open internet are different postures,
/// and the second one should say so out loud rather than leave it to be
/// inferred from a licence file nobody opens.
///
/// It also says which build it is, because a draft screen that scores locally
/// gives no other clue whether the tab in front of you is the deploy that just
/// went out or the one cached before it.
#[component]
pub fn Footer() -> Element {
    rsx! {
        footer { class: "footer",
            a { href: "https://minmax.watch/", "minmax.watch" }
            span { class: "sep", "·" }
            // `code` and not `source`: the readme it leads to is the long form of
            // the panel above, and a reader following the word "source" to find
            // out where the numbers came from used to land on a compiler input.
            a {
                href: "https://github.com/MaikBuse/minmax-watch",
                rel: "noopener",
                title: "the source code on github",
                "code"
            }
            span { class: "sep", "·" }
            span { "MIT" }
            span { class: "sep", "·" }
            // Seven characters is what a sha is quoted as; the href carries the
            // whole thing, which is also the `<sha>-amd64` image tag, so the
            // link is how you get from "what am I looking at" to the diff.
            // `.get(..7)` rather than a slice: a short or non-hex value is a
            // build-script mistake, not a reason to panic on someone's screen.
            if BUILD == "dev" {
                span { title: "a local build — nothing stamped a commit into it", "build dev" }
            } else {
                a {
                    href: "https://github.com/MaikBuse/minmax-watch/commit/{BUILD}",
                    rel: "noopener",
                    title: "the commit this build came from",
                    "build {BUILD.get(..7).unwrap_or(BUILD)}"
                }
            }
            span { class: "sep", "·" }
            span {
                "not affiliated with or endorsed by Blizzard Entertainment. \
                 Overwatch, hero, map and rank artwork are Blizzard's."
            }
        }
    }
}

/// The role glyph on a mode segment, drawn inline rather than set in type.
///
/// The obvious candidates in Unicode — a shogi piece for the shield, a position
/// indicator for the reticle — have patchy font coverage and would land as tofu
/// on the machines that lack them, which is worse than no glyph at all. Three
/// paths cost less than that risk, and `currentColor` means each one picks up
/// its role's accent without a second rule.
fn role_glyph(role: Role) -> Element {
    rsx! {
        svg {
            class: "mode-glyph",
            view_box: "0 0 16 16",
            width: "14",
            height: "14",
            fill: "currentColor",
            "aria-hidden": "true",
            match role {
                // A shield: the thing that walks in front.
                Role::Tank => rsx! {
                    path { d: "M8 1 2.5 3.2v4.4c0 3.3 2.3 6.2 5.5 7.4 3.2-1.2 5.5-4.1 5.5-7.4V3.2L8 1Z" }
                },
                // A reticle: the thing that picks a target.
                Role::Damage => rsx! {
                    path { d: "M7.25 1h1.5v2.6h-1.5V1Zm0 11.4h1.5V15h-1.5v-2.6ZM1 7.25h2.6v1.5H1v-1.5Zm11.4 0H15v1.5h-2.6v-1.5Z" }
                    path { d: "M8 4.4a3.6 3.6 0 1 0 0 7.2 3.6 3.6 0 0 0 0-7.2Zm0 1.5a2.1 2.1 0 1 1 0 4.2 2.1 2.1 0 0 1 0-4.2Z" }
                },
                // A cross: the thing that keeps everyone up.
                Role::Support => rsx! {
                    path { d: "M6.4 1h3.2v5.4H15v3.2H9.6V15H6.4V9.6H1V6.4h5.4V1Z" }
                },
            }
        }
    }
}

/// Artwork is drawn as a CSS background rather than an `<img>`.
///
/// Two reasons. A key whose art was never published — OverFast has no
/// screenshot for every map it lists — degrades to an empty box instead of a
/// browser's broken-image glyph, with the layout unmoved. And the portraits are
/// decorative: every one of them sits next to the name it depicts, so there is
/// no alt text to lose.
fn art(url: &str) -> String {
    format!("background-image:url({url})")
}

/// A hero as the panels display it, with the portrait already resolved.
///
/// Components stay purely presentational — they never reach into the dataset —
/// so the URL is looked up once at the call site that already has the hero in
/// hand.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroChip {
    pub hero: HeroId,
    pub name: String,
    pub icon: String,
}

/// A map as the header and the picker display it.
#[derive(Debug, Clone, PartialEq)]
pub struct MapChip {
    pub map: MapId,
    pub name: String,
    pub icon: String,
}

/// One map on the map board.
#[derive(Debug, Clone, PartialEq)]
pub struct MapTile {
    pub map: MapId,
    pub name: String,
    pub icon: String,
    pub selected: bool,
    /// Whether this map is one of the ones where attack and defend mean
    /// something — [`GameMode::has_sides`], resolved at the call site, because
    /// components here never see the dataset.
    ///
    /// [`GameMode::has_sides`]: overwatch_core::GameMode::has_sides
    pub has_sides: bool,
}

/// What one tile is, on the board it is drawn on.
///
/// One state rather than a pair of booleans, and that is the point. `selected`
/// and `disabled` spelled four combinations of which only three meant
/// anything, so the code building them had to remember to suppress the fourth.
/// It did not: a teammate's pick came out selected *and* clickable, and a click
/// on it quietly wrote the shared board. Here a tile is exactly one of these
/// and the impossible combination cannot be written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileState {
    /// Not on this board. Clicking puts it there.
    Free,
    /// On this board because somebody put it there. Clicking takes it back.
    Picked,
    /// Your own pick. Clicking takes it back like any other pick of yours; it
    /// is drawn apart because it is the one hero on the team that is you.
    Mine,
    /// A teammate's own pick, arriving from their seat. Theirs to change, not
    /// yours — a click here would have nothing to write.
    Theirs,
    /// The team has no room left in this role, so a click cannot land.
    Blocked,
}

impl TileState {
    /// The class modifier for this state. `Blocked` keeps the `disabled` class
    /// it has always had, so its styling survives the rename untouched.
    pub fn class(self) -> &'static str {
        match self {
            TileState::Free => "",
            TileState::Picked => " selected",
            TileState::Mine => " mine",
            TileState::Theirs => " theirs",
            TileState::Blocked => " disabled",
        }
    }

    /// Whether a click on this tile has anywhere to go.
    pub fn is_clickable(self) -> bool {
        !matches!(self, TileState::Theirs | TileState::Blocked)
    }
}

/// One hero on one of the roster boards.
///
/// The state is computed per board, not per hero: the same hero is picked on
/// the enemy board, unpicked on the ally board, and in your pool all at once.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroTile {
    pub hero: HeroId,
    pub name: String,
    pub icon: String,
    pub state: TileState,
    /// Whose it is, on a [`TileState::Theirs`] tile. It rides in the hover
    /// label, because "not yours to click" is only half an answer without it.
    pub owner: Option<String>,
    /// How well you play this hero, on the pool board. `None` everywhere else,
    /// and `None` on the pool board for a hero you have not claimed.
    ///
    /// A field beside [`TileState`] rather than a variant inside it, deliberately.
    /// That enum is what `board::ally_tile_state` returns, and its five states
    /// exist so an impossible combination cannot be written down; a `Comfort(u8)`
    /// arm would put a pool concept into the type the *team* board is checked
    /// against. So a claimed pool tile stays [`TileState::Free`] and carries its
    /// level here.
    pub comfort: Option<ComfortStep>,
}

/// What the pool board says a click does.
///
/// A `const` rather than a literal at the call site so a test can read the same
/// string the screen does — and because a line continuation inside `rsx!` is one
/// `just fmt` away from baking its own indentation into the copy, which is
/// exactly what happened to the first version of this.
pub const POOL_NOTE: &str =
    "click to cycle \u{2014} ok \u{b7} good \u{b7} main. comfort is the second-heaviest term in the score.";

/// The class list for one tile.
/// The class list for one tile.
///
/// A free function rather than a `format!` inside the markup, for the reason
/// `ban_text` is one: the comfort level is drawn by class and nothing else, so
/// a reader who cannot tell two ambers apart depends entirely on this being
/// right, and logic no test can reach is logic that drifts.
fn tile_class(tile: &HeroTile) -> String {
    format!(
        "tile{}{}",
        tile.state.class(),
        match tile.comfort {
            Some(ComfortStep::Ok) => " c1",
            Some(ComfortStep::Good) => " c2",
            Some(ComfortStep::Main) => " c3",
            None => "",
        },
    )
}

/// What the hold label says: the hero, and whichever of the two things about it
/// is worth a second word.
///
/// The two arms never collide. `owner` belongs to [`TileState::Theirs`], which
/// the pool board cannot produce, and `comfort` is set on the pool board alone.
fn tile_label(tile: &HeroTile) -> String {
    match (&tile.owner, tile.comfort) {
        (Some(owner), _) => format!("{} \u{b7} {}", tile.name, owner),
        (None, Some(step)) => format!("{} \u{b7} {}", tile.name, step.label()),
        (None, None) => tile.name.clone(),
    }
}

/// One role's worth of a roster board.
/// One role's worth of a roster board.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardRow {
    pub role: Role,
    pub label: String,
    /// How many of this role the team can still take, or `None` on a board that
    /// is not a team and has nothing to be out of.
    ///
    /// Without it a greyed-out row is unexplained: the only other feedback that
    /// a cap exists is a click that does nothing.
    pub capacity: Option<usize>,
    /// This row's remaining slot is the one you are holding open yourself.
    ///
    /// Shown instead of the count, because the count would be a lie. Your own
    /// unspent reservation makes `capacity` read zero while every tile in the
    /// row is live — the slot is not gone, it is yours, and saying so is both
    /// truer and more use than a number.
    pub mine: bool,
    pub tiles: Vec<HeroTile>,
}

/// What kind of fight a team is built for, already resolved into a word.
///
/// The archetype maths lives in `overwatch_core::archetype`; this is only what
/// the header says about it, so the component stays comparable and knows
/// nothing about the dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeChip {
    /// `dive`, `poke`, `brawl`, `mixed`, or an em-dash when nothing is picked.
    pub label: String,
    /// Whether enough of the team is known for the read to be stated plainly.
    /// A tentative chip is drawn muted — the word is still the signal, so the
    /// colour is never carrying the meaning on its own.
    pub confident: bool,
    /// The long form, for the title a header has no room to spell out.
    pub hint: String,
}

/// Every map, always on screen.
///
/// A match is played on one map, so this is a single-choice board: clicking the
/// map you are on selects it, and clicking it again clears it.
/// The map board, with a filter box.
///
/// The board is the one place a single click has 37 targets, so narrowing it by
/// name is worth the text input this app otherwise does without. Filtering in
/// place rather than dropping down a result list keeps the tile you were
/// reaching for where it was once you clear the query.
#[component]
pub fn MapBoard(
    maps: Vec<MapTile>,
    query: String,
    /// Attack or defend, for the one selected tile that has both. Lives here
    /// rather than in the header because the side is a property of the map, and
    /// the map is picked here.
    side: Option<Side>,
    on_query: EventHandler<String>,
    on_submit: EventHandler<()>,
    on_focus: EventHandler<bool>,
    on_pick: EventHandler<MapId>,
    on_side: EventHandler<Option<Side>>,
    on_reset: EventHandler<()>,
) -> Element {
    rsx! {
        section { class: "board board-map",
            div { class: "board-head",
                h3 { class: "board-title map", "map" }
                input {
                    class: "map-search",
                    r#type: "text",
                    value: "{query}",
                    placeholder: "filter maps…",
                    // The shortcuts live on the root element, so while this has
                    // focus the root has to stop treating letters as commands.
                    onfocusin: move |_| on_focus.call(true),
                    onfocusout: move |_| on_focus.call(false),
                    oninput: move |evt| on_query.call(evt.value()),
                    onkeydown: move |evt| {
                        match evt.key() {
                            Key::Enter => {
                                evt.prevent_default();
                                on_submit.call(());
                            }
                            Key::Escape => {
                                evt.prevent_default();
                                on_query.call(String::new());
                            }
                            _ => {}
                        }
                    },
                }
                ResetButton { confirm: false, on_reset }
            }
            if maps.is_empty() {
                p { class: "empty", "no map matches that" }
            }
            div { class: "tiles",
                for tile in maps.iter() {
                    // Every tile gets the slot, not just the one wearing the
                    // toggle: a wrapper that came and went with the selection
                    // would change the shape of the list under the diff every
                    // time you picked a different map.
                    div { key: "{tile.map.0}", class: "map-slot",
                        button {
                            class: if tile.selected { "tile map-tile selected" } else { "tile map-tile" },
                            style: art(&tile.icon),
                            // The tile is bare artwork, so the name has to be
                            // carried by the label rather than by any child text.
                            aria_label: "{tile.name}",
                            "data-name": "{tile.name}",
                            onclick: {
                                let map = tile.map;
                                move |_| on_pick.call(map)
                            },
                        }
                        // A sibling of the tile rather than a child of it — a
                        // button inside a button is not markup, and the click
                        // would carry on into the tile and take the map back.
                        //
                        // After it rather than before, though it is drawn above:
                        // it floats, so source order costs nothing visually and
                        // buys the tab order the eye expects — the map, then the
                        // question about the map.
                        if tile.selected && tile.has_sides {
                            SideToggle { side, on_side, label: tile.name.clone() }
                        }
                    }
                }
            }
        }
    }
}

/// One roster board: every hero, grouped by role, with the picked ones lit.
///
/// The whole point of having three of these — enemy, ally, pool — rather than
/// one list and a mode is that the board you click *is* the answer to "which
/// team". Nothing appears or disappears as you pick, so a portrait stays where
/// your hand learned it.
///
/// `side` is the CSS accent only (`enemy`, `ally`, `pool`); the behaviour is
/// entirely in the handler the caller passes.
#[component]
pub fn HeroBoard(
    title: String,
    side: String,
    rows: Vec<BoardRow>,
    /// Whether the reset throws away persisted configuration rather than the
    /// current draft, in which case it asks first.
    #[props(default = false)]
    reset_confirm: bool,
    /// The next click on this board takes a hero for *you* rather than for a
    /// teammate. Colours the hover border in the same amber the resulting tile
    /// will take, so which of the two a click means is answered before it is
    /// spent rather than after.
    #[props(default = false)]
    claiming: bool,
    /// What kind of fight this team is built for, if it is a team at all. The
    /// pool board passes nothing: a pool is a list of heroes you play, not a
    /// comp, and it has no shape to state.
    #[props(default)]
    shape: Option<ShapeChip>,
    /// What a click on this board does, where that is not obvious from the
    /// board. `None` on the ally, enemy and map boards, where a click picks and
    /// a second click takes it back — which is the whole of it.
    ///
    /// The pool board is the exception and the reason this exists: a click there
    /// cycles rather than toggles, and nothing on screen said so.
    #[props(default)]
    note: Option<String>,
    on_toggle: EventHandler<HeroId>,
    on_reset: EventHandler<()>,
) -> Element {
    let board_class = if claiming {
        format!("board board-{side} claiming")
    } else {
        format!("board board-{side}")
    };

    rsx! {
        section { class: "{board_class}",
            div { class: "board-head",
                h3 { class: "board-title {side}", "{title}" }
                // Drawn even when there is nothing yet to say — an em-dash
                // rather than an absence — so the header does not reflow on the
                // first pick of a draft.
                if let Some(shape) = &shape {
                    span {
                        class: if shape.confident { "shape" } else { "shape tentative" },
                        title: "{shape.hint}",
                        aria_label: "{shape.hint}",
                        "{shape.label}"
                    }
                }
                ResetButton { confirm: reset_confirm, on_reset }
            }
            // Standing rather than conditional, for `.rank-note`'s reason: a
            // caveat you have to discover is not a caveat. It sits under the head
            // and above the rows because it is about what the rows do.
            if let Some(note) = &note {
                p { class: "board-note", "{note}" }
            }
            for row in rows.iter() {
                // `mine` is exactly "the row your next click claims", so the
                // amber claiming hover can be scoped to it rather than painted
                // over a whole board where two of the three rows type a
                // teammate in.
                div {
                    key: "{row.label}",
                    class: if row.mine { "board-row mine" } else { "board-row" },
                    span { class: format!("board-role {}", role_class(row.role)),
                        "{row.label}"
                        // A zero here is the answer to "why can I not click
                        // this", which a disabled tile alone does not give —
                        // except on the row whose last slot is your own, where
                        // the zero would contradict a row of live tiles.
                        if row.mine {
                            span { class: "board-free you", title: "this slot is yours", "you" }
                        } else if let Some(free) = row.capacity {
                            span { class: "board-free", "{free}" }
                        }
                    }
                    div { class: "tiles",
                        for tile in row.tiles.iter() {
                            button {
                                key: "{tile.hero.0}",
                                class: tile_class(tile),
                                style: art(&tile.icon),
                                aria_label: "{tile.name}",
                                // Whose it is, where that is the reason it
                                // cannot be clicked — or how well you play it,
                                // on the one board where that is the question.
                                //
                                // Free, and worth having for that alone: this
                                // label already fires on hover, focus-visible
                                // *and* `:active`, so the level is readable in
                                // words on all three pointer kinds through a
                                // gesture people already use to read a portrait.
                                // The pips say how many; this says which.
                                "data-name": tile_label(tile),
                                // Only a genuinely full row gets the attribute.
                                // A teammate's pick is inert but stays focusable
                                // and hoverable, or a keyboard user could never
                                // reach the label saying whose it is.
                                disabled: matches!(tile.state, TileState::Blocked),
                                aria_disabled: !tile.state.is_clickable(),
                                // Guarded here rather than in the handler, so a
                                // click that cannot land never leaves the
                                // component at all.
                                onclick: {
                                    let hero = tile.hero;
                                    let state = tile.state;
                                    move |_| if state.is_clickable() { on_toggle.call(hero) }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Which queue this match is in: how many a team fields, and whether the roles
/// are held to a split.
///
/// Two controls rather than one four-way switch, because the two questions are
/// independent — the game asks them separately, and a combined control would
/// make "same size, other queue" a hunt rather than a click. Shaped like
/// [`SideToggle`] rather than like [`ModeSwitch`], because it is the same kind
/// of statement as the side: a small either/or about the match in front of you,
/// not a mode the app is in.
///
/// No keyboard shortcut, deliberately. Every chord that exists is something
/// done repeatedly inside a draft, where the seconds are real; this changes
/// when you change queue, once an evening, and a chord for it would be one more
/// thing in the table earning its place on the strength of nothing.
#[component]
fn FormatSwitch(format: Format, on_format: EventHandler<Format>) -> Element {
    rsx! {
        div { class: "format-bar",
            div { class: "formats", role: "group", aria_label: "team size",
                for option in TeamSize::BOTH {
                    button {
                        key: "{option.as_str()}",
                        class: if format.size == option { "format active" } else { "format" },
                        aria_pressed: "{format.size == option}",
                        // Unlike the side toggle there is no click-to-clear: a
                        // match always has a size, so there is nothing for an
                        // empty state to mean.
                        onclick: move |_| on_format.call(Format { size: option, ..format }),
                        "{option.label()}"
                    }
                }
            }
            div { class: "queues", role: "group", aria_label: "queue",
                for option in Queue::BOTH {
                    button {
                        key: "{option.as_str()}",
                        class: if format.queue == option { "queue active" } else { "queue" },
                        aria_pressed: "{format.queue == option}",
                        // The segment says "role"; what that costs you is two
                        // words too long for the header and exactly right here.
                        title: "{option.description()}",
                        aria_label: "{option.description()}",
                        onclick: move |_| on_format.call(Format { queue: option, ..format }),
                        "{option.label()}"
                    }
                }
            }
        }
    }
}

/// Attack or defend, drawn on the map it is asking about.
///
/// It used to sit in the header, a control the width of the screen away from
/// the board you had just clicked to raise the question. Pinned to the selected
/// tile instead, picking the map and picking the half of it you are playing are
/// one motion in one place, and the toggle is found by looking at the thing it
/// belongs to rather than remembered.
///
/// Rendered only where the question has an answer — Push, Control, Flashpoint
/// and Clash start both teams in the same posture, so the board renders nothing
/// on those tiles rather than a disabled control nobody can use.
///
/// `label` is the map's name: the pills say only "attack" and "defend", which
/// is the whole point on a floating control, but a screen reader arriving at
/// them needs to know which of 37 tiles they belong to.
#[component]
pub fn SideToggle(
    side: Option<Side>,
    label: String,
    on_side: EventHandler<Option<Side>>,
) -> Element {
    rsx! {
        div { class: "sides map-sides", role: "group", aria_label: "side on {label}",
            for option in Side::BOTH {
                button {
                    key: "{option.as_str()}",
                    class: if side == Some(option) { "side active" } else { "side" },
                    aria_pressed: "{side == Some(option)}",
                    // Clicking the active side clears it, the same way clicking
                    // a picked map or portrait takes it back.
                    onclick: move |_| on_side.call(if side == Some(option) { None } else { Some(option) }),
                    "{option.as_str()}"
                }
            }
        }
    }
}

/// What the number on a mode segment is counting.
fn mode_count_label(mode: &ModeChip) -> String {
    if mode.pool_size == 0 {
        format!("no {} heroes in your pool yet", mode.label)
    } else {
        format!(
            "{} of {} {} heroes in your pool",
            mode.pool_size, mode.roster_size, mode.label
        )
    }
}

/// The mode switch: which role you are picking for.
///
/// A segmented control rather than three loose pills, because the choice is
/// one-of-three and the shape should say so. Each segment carries its role's
/// own accent, its glyph and its pool count, so the answer to "what am I in, and
/// how much of that role have I marked" is legible without switching to find
/// out.
///
/// Colour is never the only signal — the live segment also takes the underline
/// bar, the raised contrast and `aria-pressed`.
#[component]
fn ModeSwitch(role: Role, modes: Vec<ModeChip>, on_role: EventHandler<Role>) -> Element {
    rsx! {
        div { class: "modes", role: "group", aria_label: "pick mode",
            for mode in modes.iter() {
                button {
                    key: "{mode.role.as_str()}",
                    class: format!(
                        "mode {}{}",
                        role_class(mode.role),
                        if mode.role == role { " active" } else { "" },
                    ),
                    aria_pressed: "{mode.role == role}",
                    // The bare number on the segment cannot say on its own what
                    // it is out of, so the thing it is counting is named here
                    // rather than left to be inferred.
                    title: "{mode_count_label(mode)}",
                    aria_label: "pick as {mode.label} — {mode_count_label(mode)}",
                    onclick: {
                        let next = mode.role;
                        move |_| on_role.call(next)
                    },
                    {role_glyph(mode.role)}
                    span { class: "mode-label", "{mode.label}" }
                    // Straight count of what you have marked. A zero here means
                    // "nothing marked yet", not "nothing to pick" — the mode
                    // still offers the whole role either way.
                    span { class: "mode-count", "{mode.pool_size}" }
                }
            }
        }
    }
}

#[component]
pub fn Header(
    role: Role,
    /// The queue the room is in: team size and whether roles are split.
    format: Format,
    map: Option<MapChip>,
    /// Whether attack and defend mean anything on the map that is picked.
    /// `false` on a symmetric mode, or when no map is picked yet.
    sides_apply: bool,
    side: Option<Side>,
    /// One per playable role, in switch order.
    modes: Vec<ModeChip>,
    generated: String,
    sync_status: String,
    /// `Some(won)` just after a result was recorded, so the keystroke has
    /// visible confirmation. It sits beside the sync light because both are
    /// transient status about what just happened rather than part of the draft.
    logged: Option<bool>,
    on_role: EventHandler<Role>,
    on_format: EventHandler<Format>,
    on_reset_all: EventHandler<()>,
) -> Element {
    let sync_class = format!("sync sync-{}", sync_status.replace(' ', "-"));

    rsx! {
        header { class: "header",
            // The page's only heading once the boot splash has gone. The name
            // is real text rather than an aria-label so that it is content a
            // crawler indexes and not just an accessible name — the mark itself
            // is a glyph, and the rest of this screen is hero names and
            // one-word board titles. `.sr-only` keeps the header looking
            // exactly as it did; see the note above `.brand-heading`.
            h1 { class: "brand-heading",
                a {
                    class: "brand",
                    href: "/",
                    title: "minmax.watch",
                    {brand_mark()}
                    span { class: "sr-only", "MinMax — Overwatch 2 draft assistant" }
                }
            }
            ModeSwitch { role, modes, on_role }
            div { class: "context",
                // First, because it is the widest-scope fact about the match —
                // queue, then map, then side — and because it never disappears,
                // so the cluster keeps a stable left edge as picks land.
                FormatSwitch { format, on_format }
                // The shot, the name and the side are one fact and one element,
                // so a header that has run out of room wraps them together
                // rather than leaving "attack" alone at the head of a line with
                // nothing to say which map it is about.
                div { class: "map-chip",
                    match map {
                        Some(map) => rsx! {
                            span { class: "map-thumb", style: art(&map.icon) }
                            span { class: "map", "{map.name}" }
                            // A readout, not a control: the toggle lives on the map
                            // board now, on the tile it is about. This is here so
                            // the header still states the whole match in one line —
                            // offering it twice would only make the two places
                            // something to choose between.
                            if let Some(side) = side.filter(|_| sides_apply) {
                                span { class: "map-side", "{side.as_str()}" }
                            }
                        },
                        None => rsx! { span { class: "map unset", "no map" } },
                    }
                }
                // The rank picker used to sit here, after the map. It states a
                // fact about the match like everything else in this row, which is
                // exactly why it read as one — nothing here connected it to the
                // two lists it reorders. It is in the pick panel's own head now.
                // The pool count used to sit here, adrift between the map and
                // the sync light. It lives on the mode segment it describes now,
                // where it is also legible for the modes you are not in.
                // Whether the other screen is actually attached. Scoring is
                // local either way, so "offline" costs sync, not function.
                span { class: "{sync_class}", "{sync_status}" }
                // Confirmation that ⌥W/⌥L landed. Absent the rest of the time,
                // and cleared by the next ordinary action rather than a timer.
                match logged {
                    Some(true) => rsx! { span { class: "logged win", "win recorded" } },
                    Some(false) => rsx! { span { class: "logged loss", "loss recorded" } },
                    None => rsx! {},
                }
                // Counter data ages with every patch; showing when it was last
                // pulled is the difference between trusting it and trusting it
                // blindly.
                span { class: "generated", title: "counter data last ingested", "{generated}" }
                KeyHelp {}
                // Everything, map and side included — the "new match" reset, as
                // opposed to Esc, which keeps the map for the next round.
                ResetButton { confirm: true, on_reset: on_reset_all }
            }
        }
    }
}

/// The badge slot for a rung, as the menu draws it.
///
/// [`Rank::All`] keeps its slot and gets a short rule in it, because in a column
/// of nine the badges are a column too and the labels have to share a left edge.
/// Deliberately not the `--line` placeholder tile the rest of the artwork
/// degrades to: that says "this image failed", and it would be saying it about a
/// file that was never supposed to exist, on the row that is the default.
fn rank_badge(rank: Rank) -> Element {
    match crate::icons::rank(rank) {
        Some(url) => rsx! { span { class: "rank-badge", style: art(&url) } },
        None => rsx! { span { class: "rank-badge none", aria_hidden: "true" } },
    }
}

/// The same badge on the chip, where [`Rank::All`] draws nothing at all.
///
/// No empty slot here, unlike the menu. There is no column to line up with, and
/// the rule that reads as an icon between eight others reads as punctuation
/// between two words — "rank — all ranks". Nor is there a width to protect: the
/// chip already resizes on every pick, because "all ranks" and "grandmaster+"
/// are different lengths and the label is the wider half of the control.
fn rank_chip_badge(rank: Rank) -> Element {
    match crate::icons::rank(rank) {
        Some(url) => rsx! { span { class: "rank-badge", style: art(&url) } },
        None => rsx! {},
    }
}

/// Which rung of the ladder patch strength is read on.
///
/// A chip that states its own answer, plus a menu of the alternatives behind a
/// click. The header has been overflowing below 950px since before this existed
/// — see the note above `.header` in the stylesheet — so the resting cost is one
/// chip and everything else folds away.
///
/// The menu is a column, not a wrapped row of pills. Nine options carrying
/// artwork have one obvious reading order and it is vertical; wrapped, the
/// badges made a grid whose rows meant nothing, when Bronze to Grandmaster is
/// the one control in this app where the order *is* the meaning.
///
/// The badges are Blizzard's own, at `/ranks/{as_str}.webp`. They are here
/// because recognition beats reading in a menu opened under a thirty-second
/// clock, and they do not stand alone: every row keeps its label, `aria-pressed`
/// says which is live, and the selected row takes a bar and a weight step as
/// well as a tint. The aggregate has no badge and never will — it is not a rung
/// — so its slot is drawn as a rule rather than a picture, and drawn at all
/// rather than omitted so the nine labels share one left edge and the chip does
/// not change width when you pick a rung.
///
/// That is not the rule about nothing appearing mid-draft. What that forbids is
/// something a *draft* can reveal or remove; this opens on an explicit click on
/// a control that is always there, the same shape as the QR panel and the key
/// help. The current value is never hidden — it is printed on the button.
///
/// No keyboard chord, on the argument at [`FormatSwitch`]: every chord that
/// exists is for something done repeatedly inside a draft, and this is changed
/// when you change bracket.
///
/// **The scope caveat is not in here.** "Only patch strength is sliced by rank"
/// is rendered by [`Recommendations`], beside the control rather than inside a
/// sheet you have to open to be told what the control does not do. Anything else
/// that mounts this picker has to carry that line too — a rank control with no
/// caveat on it implies the counter term moved, which is the one reading
/// `overwatch_core::Rank`'s own module docs forbid.
#[component]
fn RankPicker(
    rank: Rank,
    open: bool,
    on_rank: EventHandler<Rank>,
    on_open: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "rank-picker",
            button {
                class: if open { "rank-chip open" } else { "rank-chip" },
                r#type: "button",
                aria_expanded: "{open}",
                aria_controls: "rank-sheet",
                title: "which rung of the ladder patch strength is read on \u{2014} nothing else changes",
                onclick: move |_| on_open.call(()),
                span { class: "rank-chip-label", "rank" }
                {rank_chip_badge(rank)}
                span { class: "rank-chip-value", "{rank.label()}" }
                span { class: "rank-caret", aria_hidden: "true", "\u{25be}" }
            }
            // Rendered whether or not it is open, so `aria-controls` always
            // resolves to a real element and the sheet is a class away from
            // visible rather than a mount away.
            div {
                id: "rank-sheet",
                class: if open { "ranks open" } else { "ranks" },
                role: "group",
                aria_label: "which rung of the ladder to read patch strength on",
                for option in Rank::CHOICES {
                    button {
                        key: "{option.as_str()}",
                        // `.rank-row`, and no longer `.rank-option`: these left
                        // the .side/.format/.queue pill family when they gained
                        // artwork and a column. Neither may be `.rank`, which is
                        // the ordinal column in the pick list and is 14px wide.
                        class: if rank == option { "rank-row active" } else { "rank-row" },
                        r#type: "button",
                        aria_pressed: "{rank == option}",
                        title: "{option.description()}",
                        onclick: move |_| on_rank.call(option),
                        {rank_badge(option)}
                        span { class: "rank-row-label", "{option.label()}" }
                    }
                }
            }
        }
    }
}

/// One hero the ban list is arguing for.
///
/// Everything is pre-resolved here for the same reason as [`RecRow`]: the
/// component stays comparable, so Dioxus can skip the list when only the draft's
/// unrelated half changed.
#[derive(Debug, Clone, PartialEq)]
pub struct BanRow {
    pub hero: HeroId,
    pub name: String,
    pub icon: String,
    /// The weighted danger, already formatted.
    pub score: String,
    /// Which of the team's heroes this hurts most. `None` when the team comes to
    /// one locked hero, in which case the answer is "you" and saying so would be
    /// noise, and on the patch rung, where no pair produced the score.
    pub worst: Option<String>,
    /// Who plays `worst`, when it is somebody other than you.
    pub worst_owner: Option<String>,
    /// The scraped sentence for that pair, or the win rate on the patch rung.
    pub text: String,
    /// True when `text` is a source's sentence rather than a figure this app
    /// formatted. Resolved with `text` in [`ban_text`], so the line and the claim
    /// about who wrote it cannot be decided in two places and disagree.
    pub cited: bool,
    /// Why this row sits higher or lower than its matchup alone would put it, when
    /// prevalence moved it far enough to be worth saying. `None` for the ordinary
    /// middle of the roster, which is most of it.
    pub prevalence: Option<String>,
}

/// How much of the enemy board this candidate was actually read against.
///
/// The counter mean divides by *every* enemy rather than only the rated ones —
/// see `score_hero`, which spends a page on why — so thin coverage no longer
/// reads as conviction in the ranking. What it could not do is say so: two rows
/// at +41 and +38 look alike whether one was read against five enemies and the
/// other against one.
///
/// Silence means complete. `5 of 5` on every row is a fraction nobody reads
/// twice, and the rows that say nothing are then the ones with nothing to admit.
///
/// The `rated == 0` sentence is the one `main.rs` already gives the empty threat
/// panel, with `your pick` becoming `it` because the subject here is a candidate
/// rather than the hero you are on. Two silences about the same absence should
/// read alike.
///
/// **A mirror counts as rated.** `matchup_term` answers `Some(0.0)` when the
/// candidate is the enemy, so mirroring an entered pick adds to `rated`. That is
/// the app's position rather than an oversight — a mirror is a rated dead even,
/// and the threat row beside this one prints `the mirror — even by definition`.
///
/// **The fraction can never describe a single pick.** Reaching it needs
/// `0 < rated < entered`, so `entered` is at least two by construction and
/// `their {entered} picks` is never singular. That is why this carries none of
/// the singular/plural split `p.threat-note` needs.
pub fn coverage_note(counter: Coverage) -> Option<String> {
    if counter.entered == 0 || counter.rated == counter.entered {
        return None;
    }
    if counter.rated == 0 {
        return Some("no source has rated it against any of them".to_owned());
    }
    Some(format!(
        "read against {} of their {} picks",
        counter.rated, counter.entered
    ))
}

/// Says out loud that a hero is picked far more or far less often than its role's
/// share, when it is.
///
/// The threshold is `|50|`, which is not a round number picked for looking like
/// one: on the log scale `prevalence.toml` uses, 50 is exactly the point where a
/// hero is twice or half its fair share. Below that the row would be explaining
/// an ordinary hero to no purpose, and the discount it earned is small enough that
/// naming it would overstate what moved.
///
/// **Deliberately no percentage.** The dataset holds nine log-compressed
/// comparisons against a role, not nine pick rates, so there is no figure here to
/// print — and a percentage read off one rung's population, sitting beside a list
/// sorted on another, would be a column disagreeing with itself. The rung is named
/// instead, which is the part that is actually true.
pub fn prevalence_note(value: i8, rank: Rank) -> Option<String> {
    if value.abs() < 50 {
        return None;
    }
    let common = value > 0;
    Some(match (common, rank) {
        (true, Rank::All) => "commonly picked".to_owned(),
        (false, Rank::All) => "rarely picked".to_owned(),
        (true, rank) => format!("commonly picked at {}", rank.label()),
        (false, rank) => format!("rarely picked at {}", rank.label()),
    })
}

/// Who to deny the enemy, before anyone has picked.
///
/// The counterpart to [`Recommendations`], and the one panel that is about the
/// phase *before* the draft. `subject` is spelled out in the heading rather than
/// left to be inferred, because the number in each row means something different
/// depending on it — one locked hero's own matchup, an average over everything
/// the team might end up on, or, before anybody has said anything, the patch.
///
/// Nothing the team plays is ever on this list. A ban takes the hero off the
/// table for everyone, so a row here is only an argument at all if denying it
/// costs your side nothing — which is why there is no longer a "one of yours"
/// marker to explain: the case it explained cannot occur.
#[component]
pub fn BanPanel(subject: String, items: Vec<BanRow>) -> Element {
    rsx! {
        section { class: "panel bans",
            div { class: "panel-head",
                h2 { "ban" }
                span { class: "subject", "{subject}" }
            }
            if items.is_empty() {
                p { class: "empty", "nothing here beats your team — no ban worth spending" }
            }
            for (index, ban) in items.iter().enumerate() {
                div {
                    key: "{ban.hero.0}",
                    class: "ban",
                    span { class: "rank", "{index + 1}" }
                    span { class: "rec-portrait", style: art(&ban.icon) }
                    div { class: "rec-body",
                        div { class: "rec-head",
                            span { class: "rec-name", "{ban.name}" }
                            span { class: "score", "{ban.score}" }
                        }
                        // Two separate claims, so they are two lines: whose hero
                        // takes the worst of it, and whatever the sources
                        // actually say about that pair.
                        if let Some(worst) = &ban.worst {
                            match &ban.worst_owner {
                                Some(owner) => rsx! {
                                    p { class: "ban-worst", "hardest on {worst} · {owner}" }
                                },
                                None => rsx! {
                                    p { class: "ban-worst", "hardest on {worst}" }
                                },
                            }
                        }
                        if !ban.text.is_empty() {
                            p { class: "ban-text",
                                "{ban.text}"
                                // Never on the patch rung, where the line is a
                                // win rate this app formatted rather than
                                // anything a site said about a pair.
                                if ban.cited {
                                    span {
                                        class: "cite",
                                        title: "quoted from counterpickgg",
                                        "counterpickgg"
                                    }
                                }
                            }
                        }
                        // A third claim, so a third line: how often this hero
                        // turns up at all, which is what moved the row relative
                        // to the matchup above it.
                        if let Some(prevalence) = &ban.prevalence {
                            p { class: "ban-worst", "{prevalence}" }
                        }
                    }
                }
            }
        }
    }
}

/// One enemy, read against the hero you are on.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreatRow {
    pub enemy: HeroId,
    pub name: String,
    pub icon: String,
    /// Already formatted and already negated, so below zero is losing exactly
    /// as it is in the two columns beside this one.
    pub score: String,
    /// Whether that number came out above zero, for the tint. Carried rather
    /// than re-derived from the string, so the colour cannot disagree with the
    /// glyph on a value that rounded across the boundary.
    pub favourable: bool,
    /// True for a dead flat `+0`, which is neither.
    pub even: bool,
    /// The scraped sentence for the pair, empty for most of them.
    pub text: String,
    /// The two trusted sources contradicted each other about this pair.
    ///
    /// Not the roster's `contested`, which is two teammates on one hero. This is
    /// about the number, not about the seat.
    pub disputed: bool,
    /// The words on this row are counterpickgg's, quoted exactly.
    ///
    /// This panel is *not* uniform, which is why the marker is per row here as
    /// well: the mirror gets a line this app wrote, and a panel-level "quoted
    /// from counterpickgg" would credit the site with it.
    pub cited: bool,
}

impl ThreatRow {
    /// Resolves one threat into display form.
    ///
    /// `locked` is the hero the whole panel is read against, needed only to
    /// recognise the mirror.
    pub fn build(threat: &Threat, locked: HeroId, dataset: &Dataset) -> Self {
        // `severity` is positive when the enemy is winning, so negating it puts
        // this column on the same footing as the pick and ban columns. Rounded
        // to an integer *before* the sign is read: `format!("{:+.0}", -0.004)`
        // prints "-0", and a red minus-zero is a claim the data does not make.
        let points = (threat.severity * -100.0).round() as i32;

        // Read before the mirror substitution below, because that line is ours.
        // `threats()` fills this field from `ds.reason` and from nowhere else, so
        // the raw field is the whole question.
        let cited = !threat.text.is_empty();

        let text = if !threat.text.is_empty() {
            threat.text.clone()
        } else if threat.enemy == locked {
            // The mirror is rated 0.0 by definition rather than by anybody's
            // measurement, and no source writes a sentence about it. Left bare
            // it is a portrait and a "+0" — indistinguishable from the missing
            // data it is the opposite of.
            "the mirror — even by definition".to_owned()
        } else {
            // Everything else stays bare. Unlike a recommendation there is no
            // `kind` to phrase from, and the only thing left to say would be
            // the number again in words.
            String::new()
        };

        Self {
            enemy: threat.enemy,
            name: hero_name(dataset, threat.enemy),
            icon: dataset
                .hero(threat.enemy)
                .map(|h| crate::icons::hero(&h.key))
                .unwrap_or_default(),
            score: format!("{points:+}"),
            favourable: points > 0,
            even: points == 0,
            text,
            disputed: threat.disputed,
            cited,
        }
    }
}

/// The enemy team, read against the hero you are on.
///
/// The counterpart to [`BanPanel`] directly above it, and deliberately not a
/// second copy of it: a ban candidate is by construction a hero *nobody has
/// picked* — [`overwatch_core::ban_recommendations`] filters the enemy board out
/// — while every row here is a hero who is already on it. The two lists cannot
/// overlap, and a hero moves from that panel to this one the moment the enemy
/// locks it.
///
/// The heading says "matchups" rather than "threats" because the core function
/// returns every rated enemy, including the ones you beat. A `+18` row is not a
/// threat, and a panel that called it one would be lying in the one place the
/// user is checking whether to trust it.
///
/// `subject` fixes the referent, which matters more here than on the ban panel.
/// Every row shows an *enemy* portrait and an enemy name, so "Pharah −30" reads
/// as a claim about Pharah unless the header says whose number it is.
#[component]
pub fn ThreatPanel(
    subject: Option<String>,
    items: Vec<ThreatRow>,
    /// How many enemies are entered but unrated, so the gap between this list
    /// and the enemy board is stated rather than left to be noticed.
    unrated: usize,
    empty: String,
) -> Element {
    rsx! {
        section { class: "panel threats",
            div { class: "panel-head",
                h2 { "matchups" }
                if let Some(subject) = &subject {
                    span { class: "subject", "{subject}" }
                }
            }
            // Standing, and above the rows rather than below them: this is the
            // base rate every number in the column is read against, not a
            // footnote about whichever rows happen to be on screen. `.threat-note`
            // at the foot is the opposite — it counts something, and appears only
            // when there is something to count.
            //
            // The base rate is stated once and only the pairs in active dispute
            // are marked. Tagging three quarters of the matrix as thin would be
            // noise rather than context, which is the complaint this answers.
            p { class: "matchup-note",
                "three quarters of matchups are one site's rating \u{2014} \u{201c}disputed\u{201d} means the second source disagreed, and the number has been pulled toward even"
            }
            if items.is_empty() {
                p { class: "empty", "{empty}" }
            }
            for threat in items.iter() {
                div {
                    key: "{threat.enemy.0}",
                    class: "threat",
                    span { class: "rec-portrait", style: art(&threat.icon) }
                    div { class: "rec-body",
                        div { class: "rec-head",
                            span { class: "rec-name", "{threat.name}" }
                            // A word rather than a hue, so the caveat survives
                            // the reader who cannot tell the two tints apart.
                            if threat.disputed {
                                span {
                                    class: "tag",
                                    title: "the two sources disagree about this matchup, so the reading has been pulled toward even",
                                    "disputed"
                                }
                            }
                            span {
                                class: if threat.even {
                                    "score"
                                } else if threat.favourable {
                                    "score good"
                                } else {
                                    "score bad"
                                },
                                "{threat.score}"
                            }
                        }
                        if !threat.text.is_empty() {
                            p { class: "threat-text",
                                "{threat.text}"
                                // Per row rather than per panel, because this
                                // panel is not uniform: the mirror carries a line
                                // this app wrote, and one standing note over the
                                // column would credit counterpickgg with it.
                                if threat.cited {
                                    span {
                                        class: "cite",
                                        title: "quoted from counterpickgg",
                                        "counterpickgg"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Naming the silence, rather than letting the row count quietly
            // disagree with the number of portraits on the enemy board.
            if unrated > 0 {
                p { class: "threat-note",
                    if unrated == 1 { "1 of their picks unrated" } else { "{unrated} of their picks unrated" }
                }
            }
        }
    }
}

/// How many reason lines a row shows.
///
/// Here rather than in the scorer, which is where it used to live. How many
/// sentences fit beside a portrait in a 62-character column is a question about
/// this column, and core answering it meant the `why` panel could not have the
/// rest of them. The scorer still sorts by impact — that ordering is a claim
/// about the terms, and it is what makes taking the front of the list the right
/// three to take.
const MAX_REASONS: usize = 3;

/// A recommendation with every name and number already resolved.
///
/// Components stay purely presentational and comparable, which is what lets
/// Dioxus skip re-rendering the list when only the query changed.
#[derive(Debug, Clone, PartialEq)]
pub struct RecRow {
    pub hero: HeroId,
    pub name: String,
    pub icon: String,
    /// Absolute score, or the delta against your locked hero in swap mode.
    pub score: String,
    pub is_locked: bool,
    pub worth_swapping: bool,
    /// Where the scorer sorted this hero, 0-based.
    ///
    /// Carried rather than taken from the loop index, because this list is
    /// rendered twice — the pick column and the answer strip — and both are
    /// components that take a `Vec<RecRow>` from outside. Numbering by position
    /// in your own copy is only right for as long as the two copies agree, and
    /// nothing said they did.
    pub place: usize,
    /// Whether the scorer could separate this hero from the best one.
    ///
    /// The set is a prefix of the list, which is what lets the panel draw it as a
    /// boundary between two rows rather than as a mark on each of them — see
    /// `Recommendation::tied_with_top`.
    pub tied_with_top: bool,
    /// How well you have said you play this hero, on the canonical -100..=100
    /// scale. Zero for a hero you have said nothing about.
    ///
    /// **The value and not a `bool`, and not a [`ComfortStep`] either.** It was a
    /// bool while the pool was a highlight and nothing else; it is the number now
    /// because the row asks two different questions of it. *Is this one of yours*
    /// is [`RecRow::claimed`], which has to agree with `Profile::pool`, the mode
    /// chip's count and the pool the ban list defends — all of them `> 0`. *Which
    /// rung* is [`comfort_claim`], which is an exact match and has no answer for a
    /// hand-edited `21`. An `Option<ComfortStep>` would answer the second question
    /// in place of the first, and that row would go out unstarred while the
    /// reasons beneath it still called the hero one of your comfort picks.
    ///
    /// So a claimed hero still says so twice — a star up here and a comfort line
    /// in the reasons below — but the two now say *which* claim, out of one
    /// function, and the star is what survives when impact-sorting drops the line.
    pub comfort: i8,
    pub reasons: Vec<ReasonLine>,
    /// How much of the enemy board this row was read against, when that is worth
    /// saying. `None` on a complete read and on an empty board alike — see
    /// [`coverage_note`] for why silence is the right shape for both.
    pub coverage: Option<String>,
}

/// One line of a row's "why", resolved to the words it will show.
///
/// A struct rather than the `(bool, String)` tuple it grew out of, because the
/// line now carries a claim *about* its own evidence as well as the evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ReasonLine {
    /// The term moved the score up. `>= 0.0`, which is the tint this list has
    /// always drawn.
    pub positive: bool,
    pub text: String,
    /// The two trusted sources contradicted each other about this matchup.
    ///
    /// Set for the counter terms and nothing else: they are the only ones that
    /// read the matchup matrix, so they are the only ones with two sources that
    /// could have disagreed.
    pub disputed: bool,
    /// The words on this line are counterpickgg's, quoted exactly.
    ///
    /// Not a synonym for "the text came from the dataset". The typographic
    /// register separates *prose* from *templates*, and the hand-written notes in
    /// `side.toml` and `archetype.toml` are prose too — so the moment those reach
    /// `Reason.text`, a line carrying a sentence is no longer necessarily carrying
    /// a site's sentence. Only the kind can answer that, which is why [`RecRow`]
    /// resolves this the way it resolves `disputed`.
    pub cited: bool,
}

impl ReasonLine {
    /// Resolves one reason into the words and the markers it will show.
    ///
    /// Extracted so the pick row and the `why` panel cannot disagree about a
    /// sentence — and, more sharply, about whose sentence it is. The two markers
    /// are resolved from the reason's *kind*, so a panel that assembled its own
    /// lines would be one forgotten `matches!` away from printing a
    /// counterpickgg sentence with no attribution on it, on the same screen as
    /// the legend explaining what the attribution means.
    pub fn build(reason: &Reason, hero: HeroId, rank: Rank, ds: &Dataset) -> Self {
        // Read before the words are chosen, because half of them depend on it:
        // the sign is a CSS pseudo-element and the wording it prefixes has to be
        // the one it does not contradict.
        let positive = reason.contribution >= 0.0;
        let text = if reason.text.is_empty() {
            // Only ~40% of matchups carry a scraped sentence; the rest get
            // phrasing generated from the reason kind, because a bare number
            // explains nothing.
            let head = phrasing(reason.kind, hero, rank, ds).under(positive);
            match hand_written_note(reason.kind, hero, ds) {
                // The head has to survive rather than be replaced, and the shape
                // kinds are why: the head names *their* leading axis while the
                // note is about *this* kit. "answers their dive" alone says what
                // the portrait already said, and the note alone answers a
                // question nobody asked.
                Some(note) => format!("{head} \u{2014} {note}"),
                None => head,
            }
        } else {
            reason.text.clone()
        };
        // Only the counter terms read the matchup matrix, so only they can be in
        // dispute. Asked of the pair rather than of the row the scorer happened
        // to average, which is what `sources_disagree` is for.
        let disputed = match reason.kind {
            ReasonKind::BeatsEnemy(enemy) | ReasonKind::LosesToEnemy(enemy) => {
                ds.sources_disagree(hero, enemy)
            }
            _ => false,
        };
        // Gated on the *kind* and not merely on the text being there.
        // `!text.is_empty()` alone answers correctly today and becomes a false
        // attribution the moment a term other than the counter ones carries
        // prose: the notes in `side.toml` and `archetype.toml` are this
        // repository's own words, and marking them `counterpickgg` would credit
        // a site that never saw them.
        let cited = matches!(
            reason.kind,
            ReasonKind::BeatsEnemy(_) | ReasonKind::LosesToEnemy(_)
        ) && !reason.text.is_empty();

        Self {
            positive,
            text,
            disputed,
            cited,
        }
    }
}

/// The one rule for turning a score into the whole number a reader sees.
///
/// Rounded before the sign is read, which is the argument `ThreatRow` makes in
/// place: `format!("{:+.0}", -0.004)` prints `-0`, and a red minus-zero is a
/// claim the data does not make. Both idioms were in this file — the threat
/// column and the ledger's terms rounded first, the pick row and the ledger's
/// total let the formatter do it — so numbers meant to be compared with each
/// other were arrived at two different ways, and half-to-even against
/// half-away-from-zero can disagree on a single value with no sum involved.
pub fn points(value: f32) -> i32 {
    (value * 100.0).round() as i32
}

/// Round a set of values to whole points so they sum to the total's own
/// rounding.
///
/// A sum of rounded values is not the rounding of a sum. Eight terms rounded
/// one at a time drift from the rounded total by up to four points, and the
/// panel above them says out loud that they add up — so for a while they did
/// not. Two terms at 1.4 print as `+1` and `+1` under a total of `+3`.
///
/// Largest remainder, which is the standard answer for a table that has to
/// balance: floor every value, then hand the units still owed to the largest
/// fractional parts. Every result is within one point of its own honest
/// rounding, and together they come to `points(total)` exactly — so the rows add
/// up to the footer *and* the footer is still the number on the row, which no
/// single rounding of each value can give you.
///
/// Ties break by index rather than by value, so the same draft always draws the
/// same table. A comparison that fell back on float order would move a point
/// between two rows for no reason a reader could see.
fn apportion(values: &[f32], total: f32) -> Vec<i32> {
    let scaled: Vec<f32> = values.iter().map(|value| value * 100.0).collect();
    let mut out: Vec<i32> = scaled.iter().map(|value| value.floor() as i32).collect();

    // Non-negative and at most `values.len()` by construction, since flooring
    // each value loses less than a point apiece. Clamped anyway: `total` is the
    // caller's, and nothing here should be able to index past the end.
    let owed = (points(total) - out.iter().sum::<i32>()).clamp(0, out.len() as i32);

    let mut order: Vec<usize> = (0..scaled.len()).collect();
    order.sort_by(|a, b| {
        let fraction = |i: usize| scaled[i] - scaled[i].floor();
        fraction(*b)
            .partial_cmp(&fraction(*a))
            .unwrap_or(core::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    for index in order.into_iter().take(owed as usize) {
        out[index] += 1;
    }
    out
}

/// One term of the ledger, resolved to what it will show.
#[derive(Debug, Clone, PartialEq)]
pub struct WhyTerm {
    pub label: &'static str,
    /// Already signed, because the sign is what carries the direction and the
    /// colour only reinforces it.
    pub value: String,
    pub positive: bool,
    /// A dead flat `+0`, which is neither. Takes no tint at all — the rule
    /// `ThreatRow` established: "+0" is not an argument in either direction.
    pub even: bool,
    /// What this term is worth against the comparand, when there is one.
    pub delta: Option<String>,
    pub delta_positive: bool,
    pub delta_even: bool,
}

/// The whole arithmetic behind one row, and the admissions the row has no space
/// for.
///
/// The pick row shows three sentences chosen by impact, and by deliberate rule
/// they can never add up: a zero term produces none, and the shape term against
/// an enemy board that commits to nothing moves the score with no archetype for
/// a sentence to name. This is where the eight terms, their zeros, the coverage
/// and every reason are shown together, summing to the number on the row.
#[derive(Debug, Clone, PartialEq)]
pub struct WhyView {
    pub hero: HeroId,
    pub name: String,
    /// Eight, in [`TermKind::ALL`] order and **never** sorted by contribution.
    /// The sorted view is the reason list below it; a table whose rows move
    /// between heroes is one you re-read every time.
    pub terms: Vec<WhyTerm>,
    pub total: String,
    /// Stated always here, complete or not, unlike the row's deliberate silence
    /// on a full read. The row is a list to scan; this is the answer to
    /// "how much of this did you actually know".
    pub coverage: String,
    pub allies: Option<String>,
    /// Their shape, when it reads mixed. The one term that can move a score with
    /// no reason line behind it, so this is the only place it can be said.
    pub shape: Option<String>,
    pub tie: Option<String>,
    /// The hero the second column is measured against, when there is one: your
    /// locked hero, or the top pick. `None` also when the hero being read *is*
    /// that hero, where a column of `+0`s would answer a question nobody asked.
    pub against: Option<String>,
    pub delta_total: Option<String>,
    /// Every one of them, which is what dropping `take(MAX_REASONS)` buys.
    pub reasons: Vec<ReasonLine>,
}

impl WhyView {
    /// Resolves one recommendation into its full explanation.
    ///
    /// `tied` is how many rows the scorer could not separate, and it is here for
    /// one reason: the top hero is inside the band of itself, so `tied_with_top`
    /// alone would print a tie on every leading row.
    ///
    /// `rank` for the reason [`RecRow::build`] takes it: the patch-strength
    /// sentence prints a ladder-wide win rate and has to say so once a rung is
    /// chosen.
    /// `against` is the hero the second column reads against — the one you are
    /// on, or the one at the top. Dropped here rather than at the call site when
    /// it is this row's own hero, so the rule that a hero is not compared with
    /// itself is under test rather than in a component.
    pub fn build(
        rec: &Recommendation,
        against: Option<&Recommendation>,
        tied: usize,
        tie_band: f32,
        rank: Rank,
        ds: &Dataset,
    ) -> Self {
        let against = against.filter(|other| other.hero != rec.hero);
        let breakdown = &rec.breakdown;
        // Apportioned rather than rounded one at a time, so the eight rows come
        // to the total printed under them. See `apportion`.
        let contributions: Vec<f32> = TermKind::ALL
            .into_iter()
            .map(|kind| breakdown.term(kind).contribution())
            .collect();
        let terms = TermKind::ALL
            .into_iter()
            .zip(apportion(&contributions, rec.score))
            .map(|(kind, points)| WhyTerm {
                label: kind.label(),
                value: format!("{points:+}"),
                positive: points > 0,
                even: points == 0,
                delta: None,
                delta_positive: false,
                delta_even: true,
            })
            .collect::<Vec<WhyTerm>>();

        // The second column, apportioned the same way and against the same kind
        // of total, so it balances for the same reason the first one does.
        let (terms, delta_total) = match against {
            None => (terms, None),
            Some(other) => {
                let gaps: Vec<f32> = TermKind::ALL
                    .into_iter()
                    .map(|kind| {
                        breakdown.term(kind).contribution()
                            - other.breakdown.term(kind).contribution()
                    })
                    .collect();
                let total = rec.score - other.score;
                let mut terms = terms;
                for (term, points) in terms.iter_mut().zip(apportion(&gaps, total)) {
                    term.delta = Some(format!("{points:+}"));
                    term.delta_positive = points > 0;
                    term.delta_even = points == 0;
                }
                (terms, Some(format!("{:+}", points(total))))
            }
        };

        let counter = breakdown.counter;
        let coverage = match counter.entered {
            0 => "nothing on their side yet".to_owned(),
            entered => coverage_note(counter)
                .unwrap_or_else(|| format!("read against all {entered} of their picks")),
        };
        let allies = match breakdown.synergy.entered {
            0 => None,
            entered => Some(format!(
                "paired with {} of your {} allies in the duo table",
                breakdown.synergy.rated, entered
            )),
        };

        // `is_mixed` and not `!leading().is_some()`: an unrated board is a
        // question nobody has answered and its term is zero anyway, while a
        // mixed one is an answer that no reason line can carry.
        let shape = breakdown.shape.is_mixed().then(|| {
            "their shape reads mixed \u{2014} no axis leads, so the term counts with nothing to name"
                .to_owned()
        });

        let tie = (tied >= 2 && rec.tied_with_top).then(|| {
            format!(
                "picks within {:.0} of the top are too close to call",
                (tie_band * 100.0).round()
            )
        });

        Self {
            hero: rec.hero,
            name: hero_name(ds, rec.hero),
            terms,
            total: format!("{:+}", points(rec.score)),
            coverage,
            allies,
            shape,
            tie,
            against: against.map(|other| hero_name(ds, other.hero)),
            delta_total,
            reasons: rec
                .reasons
                .iter()
                .map(|reason| ReasonLine::build(reason, rec.hero, rank, ds))
                .collect(),
        }
    }
}

/// A hero's display name, or `?` for an id the roster cannot resolve.
///
/// A free function because three places need it — both row builders and
/// [`phrasing`] — and a closure living inside one of them cannot be shared with
/// the others.
fn hero_name(ds: &Dataset, hero: HeroId) -> String {
    ds.hero(hero)
        .map(|h| h.name.clone())
        .unwrap_or_else(|_| "?".to_owned())
}

/// A map's display name, or `?` for an id the map list cannot resolve.
fn map_name(ds: &Dataset, map: MapId) -> String {
    ds.map(map)
        .map(|m| m.name.clone())
        .unwrap_or_else(|_| "?".to_owned())
}

/// The published win rate, worded so a rung cannot be read off it.
///
/// Qualified once a rank is chosen, because from then on the list is ordered on
/// that rung and this rate is still the whole ladder's — the per-rung win rate is
/// not in the dataset, only the shift the scorer reads. The score column beside
/// it *is* the sorted figure and does move with the rank, so the ordering is
/// accounted for; what this must not do is let a ladder number pass for the
/// bracket's.
///
/// One function rather than a `format!` at each site, because the ban panel and
/// the pick list's patch-strength line both print this. A qualifier that reached
/// one of them and not the other would be worse than one that reached neither.
pub fn win_rate_text(rate: f32, rank: Rank) -> String {
    if rank == Rank::All {
        format!("{rate:.1}% win rate")
    } else {
        format!("{rate:.1}% win rate across the ladder")
    }
}

/// The ban row's sentence, and whether those words are a source's.
///
/// Two returns rather than two functions, and rather than the `match` this grew
/// out of at the call site: the patch rung shows a figure instead of a rationale
/// there is none of, and the figure is ours. Deciding the line in one place and
/// the attribution in another is how the two end up disagreeing about a row.
///
/// A free function in this module for the reason [`win_rate_text`] and
/// [`prevalence_note`] are: `main.rs` builds [`BanRow`] inline, and logic that
/// only exists inside that component is logic no test can reach.
pub fn ban_text(
    win_rate: Option<f32>,
    patch_subject: bool,
    scraped: &str,
    rank: Rank,
) -> (String, bool) {
    match win_rate {
        // The wording, and the reason it is qualified once a rank is chosen, live
        // in `win_rate_text` — the pick list's patch-strength line prints the same
        // figure and the two must not drift.
        Some(rate) if patch_subject => (win_rate_text(rate, rank), false),
        _ => (scraped.to_owned(), !scraped.is_empty()),
    }
}

/// A place in the list, as English reads it.
///
/// The list is cut to eight, so nothing past `8th` can reach a screen today. The
/// eleventh-to-thirteenth rule is written anyway, because the cap is a `take(8)`
/// in `main.rs` and this should not be the thing that breaks when it moves.
fn ordinal(place: usize) -> String {
    let n = place + 1;
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// What a screen reader is told about one row when it lands on the button.
///
/// Everything else in the row — the reasons, the coverage line, the tags — is
/// text outside the button and is read in browse mode. This is the name of the
/// *control*, which is a shorter question: which hero, how good, where in the
/// list, and whichever of the three marks beside it are lit.
///
/// A free function for the reason `tile_class` gives in place: the row's state
/// is drawn by class and read out by this, and logic no test can reach drifts.
///
/// **`worth swapping` is here although the row's own tag is the only other place
/// it appears.** Every clause below answers to something visible; dropping one
/// would hand a screen-reader user strictly less than the person beside them.
///
/// The locked row is named as a state rather than as an action, because pressing
/// it is not one: `Seat::lock` assigns rather than toggles, so the current row
/// re-locks the hero it already holds and nothing moves. The control stays all
/// the same — a button that came and went would reorder the tab stops mid-draft,
/// which is worse than one that is idempotent.
fn pick_label(row: &RecRow) -> String {
    let mut label = if row.is_locked {
        format!("{}, the hero you are on", row.name)
    } else {
        format!("pick {}", row.name)
    };
    label.push_str(&format!(", {}, {}", row.score, ordinal(row.place)));

    // In the order the eye meets them along the row: the swap tag, then the tie
    // boundary under it, then the star at the far right.
    if row.worth_swapping {
        label.push_str(", worth swapping");
    }
    if row.tied_with_top {
        label.push_str(", too close to call");
    }
    if row.claimed() {
        label.push_str(", one of yours");
    }
    label
}

/// The class list for one row of the pick list.
///
/// Out of `rsx!` for the reason [`tile_class`] is: three of the four states are
/// drawn by class and nothing else, so a reader who cannot tell two borders apart
/// depends entirely on this being right, and logic no test can reach drifts.
///
/// `after-tie` goes on the row *following* the tied ones, so the hairline it draws
/// along its own top edge falls below the last of them. Derived from the count
/// rather than carried as a fifth field: the boundary is a property of the set,
/// and the set is a prefix, so its position is the count itself.
///
/// `> 1` because the top hero is inside the band of itself by definition. A rule
/// drawn under row one alone would mark a tie that does not exist.
fn rec_class(rec: &RecRow, tied: usize) -> String {
    format!(
        "rec{}{}{}{}",
        if rec.is_locked { " locked" } else { "" },
        if rec.worth_swapping { " swap" } else { "" },
        if rec.claimed() { " pooled" } else { "" },
        if tied > 1 && rec.place == tied {
            " after-tie"
        } else {
            ""
        },
    )
}

/// What the score column means, in one line under the list.
///
/// The number has always been a weighted sum on a scale of this app's own
/// invention and nothing on screen ever said so, which leaves `+41` beside a
/// portrait reading as a percentage. Locked, it is not even that — it is a gain
/// over the hero you are on, which is a second meaning for one column and the
/// reason the sentence changes rather than being appended to.
///
/// A free function here for the reason [`win_rate_text`] and [`ban_text`] are:
/// `main.rs` assembles the props inline, and logic that exists only inside a
/// component is logic no test can reach.
///
/// `tied` is how many of the shown rows the scorer could not separate, and it
/// takes the line over when there are two or more — see [`crate::tie_count`] for
/// why the caller sometimes reports none even though the scorer found some.
///
/// `lead` is in **displayed** points and the caller owes it that: the note sits
/// directly above the two figures it subtracts, so it has to be their difference
/// rather than the rounding of their difference. That makes a lead of zero
/// ordinary rather than exceptional — any two heroes inside 0.005 print the same
/// figure — hence an arm that says so instead of claiming a margin of nothing.
pub fn score_note(
    locked: Option<&str>,
    top: Option<&str>,
    lead: Option<i32>,
    tied: usize,
    shown: usize,
    any_swap: bool,
    swap_threshold: f32,
) -> String {
    // Rounded here rather than by the caller so the derivation is tested. It is
    // the same weight `worth_swapping` is measured against, and a stored profile
    // can have moved it — a hard-coded 15 would be a number on screen that
    // nothing behind it agrees with.
    //
    // "over" and not "at": the tag compares the raw f32 against the raw
    // threshold, so a row printing exactly +15 may fall either side of it. The
    // wording is the one that stays true across that rounding rather than a
    // claim the arithmetic cannot support.
    let bar = (swap_threshold * 100.0).round();

    if let Some(locked) = locked {
        let clause = if any_swap {
            format!("over +{bar:.0} is worth the swap")
        } else {
            format!("nothing here clears +{bar:.0}")
        };
        return format!("the column is the gain over {locked} \u{2014} {clause}");
    }

    // The tie **replaces** the scale rather than joining it. When the answer is
    // "any of these three", what the number is measured in is the less useful of
    // the two sentences, and there is one line.
    //
    // `>= 2` and not `>= 1`: the top hero is inside the band of itself, always, so
    // a count of one is no tie at all. `shown` separates the two wordings and is
    // the only reason it is a parameter — "all 8 here" and "top 8" are different
    // claims, and the second one implies a ninth row nobody can see. The caller
    // counts over the rows it is about to draw, so this can never name more heroes
    // than are on screen.
    if tied >= 2 {
        let clause = "take the one you are comfortable on";
        return if tied >= shown {
            format!("all {tied} here are too close to call \u{2014} {clause}")
        } else {
            format!("top {tied} too close to call \u{2014} {clause}")
        };
    }

    // Said first and every time, because it is the half a newcomer needs and the
    // half that is true whatever the draft is doing.
    let scale = "weighted sum, not a percentage";
    match (top, lead) {
        (Some(top), Some(lead)) if lead > 0 => {
            format!("{scale} \u{2014} {top} leads the next by {lead}")
        }
        // Not "leads by 0". The figures are equal as printed and saying so is the
        // honest reading of a column the eye has already compared.
        //
        // Unreachable at any sensible band, and kept anyway: two rows printing the
        // same figure are inside 0.005 of each other and therefore inside a band
        // of 0.15, so the tie clause above answers first. What reaches this is a
        // hand-edited `tie_band` near zero — where it is exactly the right
        // sentence, and where the alternative is a lead of nothing.
        (Some(_), Some(_)) => format!("{scale} \u{2014} the top two are level"),
        (Some(_), None) => format!("{scale} \u{2014} the only hero left in this role"),
        // Nothing to compare. The scale is still worth stating: the panel below
        // says why the list is empty, and this says what its numbers would mean.
        (None, _) => scale.to_owned(),
    }
}

/// One reason's wording, and whether it survives a leading minus.
///
/// The sign on a reason line is a CSS pseudo-element driven by the sign of the
/// term's contribution, and it used to be the only thing that knew which way the
/// term went: the words were fixed per kind, so all 22 heroes with a negative
/// base term rendered it as `− strong in the current patch`. Returning this
/// instead of a `String` is what turns forgetting into a compile error for
/// whoever adds the next kind.
enum Phrasing {
    /// Reads under either sign, because it names a thing rather than claiming
    /// one.
    Symmetric(String),
    /// Two wordings, because the phrase makes a claim of its own and the
    /// negative one is not the positive one with a sign in front of it.
    Signed { positive: String, negative: String },
}

impl Phrasing {
    /// The wording the leading sign will not contradict.
    fn under(self, positive: bool) -> String {
        match self {
            Phrasing::Symmetric(text) => text,
            Phrasing::Signed {
                positive: up,
                negative: down,
            } => {
                if positive {
                    up
                } else {
                    down
                }
            }
        }
    }
}

/// What a comfort value claims, in the app's own words.
///
/// A free function with two callers, which is the whole reason it is one: the
/// reason line renders it, and so does the star in the row's header. The star is
/// not a duplicate of that line — reasons are sorted by impact and cut to
/// `MAX_REASONS`, so the lowest rung's 0.12 loses to any three matchups and the
/// line never renders, leaving the glyph as the only account of a term that moved
/// the score. Two places saying it means one place deciding what is said.
///
/// [`ComfortStep::of`] is an exact match, so a value the ladder cannot name keeps
/// the sentence this line has always carried rather than being rounded onto a rung
/// the player never chose. That arm is reachable — a hand-edited profile, or a
/// value stored before the rungs were what they are — not dead code.
fn comfort_claim(value: i8) -> &'static str {
    match ComfortStep::of(value) {
        Some(ComfortStep::Ok) => "in your pool",
        Some(ComfortStep::Good) => "one you play well",
        Some(ComfortStep::Main) => "one of your mains",
        None => "one of your comfort picks",
    }
}

/// The app's own wording for one reason, for the rows where no source published
/// a sentence of its own.
///
/// `hero` is the candidate the reason is about — [`ReasonKind::BaseStrength`]
/// carries no payload and the win rate belongs to the hero, not to the kind.
fn phrasing(kind: ReasonKind, hero: HeroId, rank: Rank, ds: &Dataset) -> Phrasing {
    match kind {
        // Four kinds whose direction is fixed by the *kind* rather than by the
        // sign, so one wording is the whole answer. Deliberately not a third
        // `Phrasing` variant: it would render identically to `Symmetric` and buy
        // nothing except this comment. A minus can still reach them, but only
        // through a negative `counter` weight in a stored profile — weights load
        // unclamped — and no wording survives a reader who has inverted the term.
        //
        // These two used to read `strong into X` and `struggles against X`, and
        // the verb was wrong on every row it ever appeared on. A template is
        // rendered only where no source published a sentence, and published
        // coverage tracks magnitude almost exactly: over the committed matrix,
        // folded the way `matchup_term` folds a pair, the 1,416 rows that reach a
        // template run |term| p50 17, p90 25, **max 47**, while every one of the
        // 530 rows at 50 or more carries a sentence that replaces the template.
        // So the app claimed strength on precisely the set of matchups that have
        // none of it, and structurally could not have said it anywhere else.
        //
        // `rated` is the verb the app can support: it has a number from a source,
        // not an opinion about the fight. It is also true at any magnitude, which
        // is why this is a rewording rather than a threshold — nothing here has
        // to stay under 47 for the line to keep being honest.
        ReasonKind::BeatsEnemy(enemy) => {
            Phrasing::Symmetric(format!("rated ahead of {}", hero_name(ds, enemy)))
        }
        ReasonKind::LosesToEnemy(enemy) => {
            Phrasing::Symmetric(format!("rated behind {}", hero_name(ds, enemy)))
        }
        // Their shape, not this hero's: the portrait beside the line already says
        // what the candidate is.
        ReasonKind::CountersShape(theirs) => {
            Phrasing::Symmetric(format!("answers their {}", theirs.label()))
        }
        ReasonKind::LosesToShape(theirs) => {
            Phrasing::Symmetric(format!("walks into their {}", theirs.label()))
        }

        // A figure rather than a claim, which is exactly why it reads under
        // either sign — 22 of the 53 heroes have a negative base term, and this
        // line called every one of them strong.
        //
        // The sign is the selection-corrected term while the figure is the rate
        // as published, and today the two agree for all 53: no minus sits beside
        // a figure above 50.0. That is measured, not structural. The ingest
        // normalises around 50.0 while its selection shrink pulls toward role
        // means of 50.04 to 50.22, so a lightly-picked damage hero at 49.9%
        // would read `+ 49.9% win rate`. An oddity rather than a contradiction,
        // which is the whole difference between the two variants above.
        ReasonKind::BaseStrength => Phrasing::Symmetric(match ds.win_rate(hero) {
            Some(rate) => win_rate_text(rate, rank),
            // Unreachable on the committed data, where all 53 heroes carry a
            // published rate. A label rather than a second pair of wordings: with
            // no figure to show there is no claim left to hedge either.
            None => "patch strength".to_owned(),
        }),

        // The one term that gains a real comparative, because a side has an
        // opposite and a rung does not. That sentence is the whole reason this
        // arm is `Signed` and `RankFit` below is not.
        ReasonKind::SideFit(side) => Phrasing::Signed {
            positive: format!("suits {}", side.as_str()),
            negative: format!("leans {}", side.other().as_str()),
        },

        // Both of these negative arms are unreachable from the committed data —
        // 159 map affinities and 441 duo ratings, not one of them below zero —
        // and are written anyway. The worst-map figures the sources already
        // publish would make the first of them reachable in the same commit that
        // landed them, and the failure would be silent: a hero would simply be
        // told it performs well on the map it is worst on.
        ReasonKind::MapFit(map) => Phrasing::Signed {
            positive: format!("performs well on {}", map_name(ds, map)),
            negative: format!("a poor fit for {}", map_name(ds, map)),
        },
        ReasonKind::PairsWithAlly(ally) => Phrasing::Signed {
            positive: format!("pairs well with {}", hero_name(ds, ally)),
            negative: format!("a poor pair with {}", hero_name(ds, ally)),
        },

        // The only line on the whole screen that names a rung. Never produced at
        // `Rank::All`, where the term is zero and a zero term is never explained
        // — so an unset rank leaves the panel exactly as it has always read.
        //
        // Stays symmetric, and the reason is one sentence: a rung has no
        // opposite. This term goes negative as often as positive — half the
        // roster is worse at any given rung than across the ladder — and a
        // comparative like "stronger at master" reads as a contradiction the
        // moment it is prefixed with a minus, while "weaker at master" is not the
        // same claim as anything the other direction says.
        //
        // "right now" is what keeps it about the patch. Without it the line reads
        // as a claim about matchups at that rung, which nothing behind this
        // feature measured.
        ReasonKind::RankFit(rung) => {
            Phrasing::Symmetric(format!("suits {} right now", rung.label()))
        }

        // The one arm whose positive side is four sentences rather than one,
        // because the player chose between them: `ok`, `good` and `main` are three
        // different claims, and every one of them used to arrive here as `one of
        // your comfort picks`. The wording lives in [`comfort_claim`] because the
        // star in the row's header says the same thing and the two must not drift.
        //
        // Still `Signed`, and the negative side deliberately does not name a rung:
        // there is no negative rung to name. The ladder is three positive steps by
        // construction, and a value below zero is a hand-edited "rank this down" —
        // a different statement rather than a fourth level, which is the argument
        // `comfort.rs` and `Profile::pool` both make in place.
        ReasonKind::Comfort(value) => Phrasing::Signed {
            positive: comfort_claim(value).to_owned(),
            negative: "one you rated down".to_owned(),
        },
    }
}

/// The curated sentence behind a term the app scored off a hand-written file, if
/// there is one.
///
/// `None` for every other kind, and that is the whole rule rather than a default:
/// the rest read scraped tables, and a table has no sentence to give. The two
/// files this does reach — `side.toml` and `archetype.toml` — have no source
/// behind them at all, so the sentence is not a second opinion on the number, it
/// is the only argument for it. Coverage is total by construction: `SideFit`
/// fires only on a non-zero lean and the shape kinds only on a curated kit, and
/// every entry that can produce a line carries a note (27 of 27, 53 of 53).
///
/// Read here rather than carried on `Reason.text`, which stays exactly one thing:
/// a sentence a *source* published. That is what keeps [`ReasonLine::cited`]
/// honest and what leaves [`phrasing`] returning this app's own voice, lowercase
/// and unpunctuated, for the test that holds the register apart.
fn hand_written_note(kind: ReasonKind, hero: HeroId, ds: &Dataset) -> Option<&str> {
    match kind {
        ReasonKind::SideFit(_) => ds.side_note(hero),
        ReasonKind::CountersShape(_) | ReasonKind::LosesToShape(_) => ds.shape_note(hero),
        _ => None,
    }
}

impl RecRow {
    /// Resolves one scored recommendation into display form.
    pub fn build(
        rec: &Recommendation,
        dataset: &Dataset,
        swap_mode: bool,
        comfort: i8,
        rank: Rank,
    ) -> Self {
        // Once you are locked in, the absolute score is noise: the only
        // question is whether a swap gains you anything.
        //
        // Including on your own row, which used to be the exception. It showed
        // its absolute score while everything around it showed a delta, so under
        // a heading asking "should you swap?" the row you were already on read
        // +41 beside candidates reading +3 and the column meant two things at
        // once. Its delta is exactly `Some(0.0)` by construction, so this reads
        // `+0` and nothing had to be special-cased to get there. Nothing is lost:
        // the row carries the `current` tag, and the `why` panel has its total.
        // Through `points` like every other number this column compares against:
        // `format!("{:+.0}", -0.004)` prints `-0`, which is a red minus on a
        // reading of nothing, and it is the idiom the threat column rejected in
        // writing while this one went on using it.
        let score = if swap_mode {
            match rec.delta_vs_locked {
                Some(delta) => format!("{:+}", points(delta)),
                None => String::new(),
            }
        } else {
            format!("{:+}", points(rec.score))
        };

        // `take` here and not inside the builder: the cut to three is a fact
        // about this column's width, and the `why` panel wanting every line is
        // then one missing call rather than a second implementation.
        let reasons = rec
            .reasons
            .iter()
            .take(MAX_REASONS)
            .map(|reason| ReasonLine::build(reason, rec.hero, rank, dataset))
            .collect();

        Self {
            hero: rec.hero,
            name: hero_name(dataset, rec.hero),
            icon: dataset
                .hero(rec.hero)
                .map(|h| crate::icons::hero(&h.key))
                .unwrap_or_default(),
            score,
            is_locked: rec.is_locked,
            worth_swapping: rec.worth_swapping,
            place: rec.place,
            tied_with_top: rec.tied_with_top,
            comfort,
            reasons,
            coverage: coverage_note(rec.breakdown.counter),
        }
    }

    /// Whether this hero is one of yours.
    ///
    /// `> 0` and not `!= 0`, and not `ComfortStep::of(..).is_some()` either: this
    /// is the same predicate `Profile::pool` derives the pool from, and the two
    /// have to answer alike or the row disagrees with the mode chip beside it. A
    /// negative is a hand-edited "rank this down", which is the opposite of a
    /// claim on the hero rather than a low one.
    pub fn claimed(&self) -> bool {
        self.comfort > 0
    }
}

/// The whole arithmetic behind one row, at the foot of the list.
///
/// One panel, not eight anchored sheets. Per row would mean eight always-mounted
/// panels for `aria-controls` to resolve against, and a sheet anchored to row two
/// covers rows three through eight — the rows it was opened to compare with. In
/// normal flow below the last row it opens without moving a single aim target,
/// because there is nothing beneath it.
///
/// Rendered whether or not anything is open, which is `RankPicker`'s rule: the id
/// `aria-controls` names has to resolve to a real element, and the panel is then
/// a class away from visible rather than a mount away.
///
/// This is not the rule about nothing appearing mid-draft. What that forbids is
/// something a *draft* can reveal or remove; this opens on an explicit click on a
/// control that is always there, and no pick, role change or rotation can add or
/// take away either the control or the panel.
#[component]
fn WhyPanel(view: Option<WhyView>) -> Element {
    rsx! {
        div {
            id: "why",
            class: if view.is_some() { "why open" } else { "why" },
            role: "group",
            if let Some(view) = &view {
                h3 { class: "why-head", "why \u{b7} {view.name}" }
                p { class: "why-lede", "all eight terms, and they add up to the score" }
                // A line rather than a column header: a cell wide enough for
                // "Δ vs Wrecking Ball" costs more width than the ledger has, and
                // the wording is the one `score_note` already uses for the same
                // relation one panel up.
                if let Some(against) = &view.against {
                    p { class: "why-lede", "the right column is the gain over {against}" }
                }
                div { class: "why-terms",
                    for term in view.terms.iter() {
                        div { key: "{term.label}", class: "why-term",
                            span { class: "why-term-label", "{term.label}" }
                            span {
                                // The even case takes neither tint, which is the
                                // rule the threat column set: "+0" is not an
                                // argument in either direction. The sign carries
                                // the direction; the colour only reinforces it.
                                class: if term.even {
                                    "score"
                                } else if term.positive {
                                    "score good"
                                } else {
                                    "score bad"
                                },
                                "{term.value}"
                            }
                            if let Some(delta) = &term.delta {
                                span {
                                    class: if term.delta_even {
                                        "why-delta"
                                    } else if term.delta_positive {
                                        "why-delta good"
                                    } else {
                                        "why-delta bad"
                                    },
                                    "{delta}"
                                }
                            }
                        }
                    }
                    div { class: "why-term why-total",
                        span { class: "why-term-label", "total" }
                        span { class: "score", "{view.total}" }
                        if let Some(delta) = &view.delta_total {
                            span { class: "why-delta", "{delta}" }
                        }
                    }
                }
                p { class: "why-note", "{view.coverage}" }
                if let Some(allies) = &view.allies {
                    p { class: "why-note", "{allies}" }
                }
                if let Some(shape) = &view.shape {
                    p { class: "why-note", "{shape}" }
                }
                if let Some(tie) = &view.tie {
                    p { class: "why-note", "{tie}" }
                }
                // The same markup the row uses, so the sentences, their signs and
                // both markers cannot read differently in the two places.
                ul { class: "reasons",
                    for (index, line) in view.reasons.iter().enumerate() {
                        li {
                            key: "{index}",
                            class: if line.positive { "reason good" } else { "reason bad" },
                            "{line.text}"
                            if line.disputed {
                                span {
                                    class: "caveat",
                                    title: "the two sources disagree about this matchup, so the reading has been pulled toward even",
                                    "disputed"
                                }
                            }
                            if line.cited {
                                span {
                                    class: "cite",
                                    title: "quoted from counterpickgg",
                                    "counterpickgg"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn Recommendations(
    items: Vec<RecRow>,
    swap_mode: bool,
    /// What the score column means, from [`score_note`].
    ///
    /// Passed in rather than derived here, on the `ThreatPanel::subject`
    /// precedent: the sentence is arithmetic about the scores and [`RecRow`] has
    /// already formatted those into strings, so the numbers it needs cannot be
    /// reached from inside this component.
    note: String,
    /// The row whose arithmetic is open, already resolved. `None` closes the
    /// panel — including when the hero it named has left the list, which is why
    /// the caller derives this from the rows rather than trusting a stored id.
    why: Option<WhyView>,
    /// The rung patch strength is read on. The control lives here rather than in
    /// the header because this is the list it reorders: selecting one changes the
    /// top row for a fifth to well over a quarter of drafts, depending on the
    /// rung, and a chip among the header's match facts claimed none of that.
    rank: Rank,
    rank_open: bool,
    on_lock: EventHandler<HeroId>,
    on_why: EventHandler<HeroId>,
    on_rank: EventHandler<Rank>,
    on_rank_open: EventHandler<()>,
) -> Element {
    // Counted once for the whole list rather than per row. Kept even in swap
    // mode, unlike the note: the column there is the score minus a constant, so
    // `best - score` is unchanged by it and a tie means exactly what it means
    // anywhere else. Two candidates that both clear the bar and cannot be told
    // apart is the moment this is most worth drawing.
    let tied = items.iter().filter(|row| row.tied_with_top).count();
    // Read once for the whole list rather than per row, and off the resolved
    // view rather than a second signal: the panel and the eight carets have to
    // agree about which row is open.
    let open_why = why.as_ref().map(|view| view.hero);

    rsx! {
        section { class: "panel recommendations",
            div { class: "panel-head",
                h2 {
                    if swap_mode { "should you swap?" } else { "pick" }
                }
                RankPicker { rank, open: rank_open, on_rank, on_open: on_rank_open }
            }
            // Outside the sheet on purpose: a caveat you have to open a menu to
            // read is not a caveat on a control this prominent. Rank slices win
            // rate and nothing else — no source publishes matchups or duos per
            // rung — and next to the pick list is where that has to be said,
            // because the list is what would otherwise imply it.
            p { class: "rank-note",
                "only patch strength is sliced by rank \u{2014} matchups read the same at every rung"
            }
            // Below the rank caveat rather than above it. That one qualifies the
            // picker in the head and sits with it; this qualifies the list, so it
            // sits against the rows — the same argument `.cite-legend` makes at
            // the other end of the panel.
            p { class: "score-note", "{note}" }
            if items.is_empty() {
                p { class: "empty", "every hero in this role is already on your team" }
            }
            for rec in items.iter() {
                div {
                    key: "{rec.hero.0}",
                    class: rec_class(rec, tied),
                    onclick: {
                        let hero = rec.hero;
                        move |_| on_lock.call(hero)
                    },
                    span { class: "rank", "{rec.place + 1}" }
                    // The portrait spans the name and its reasons, so a row
                    // reads as one block rather than two stacked lines.
                    span { class: "rec-portrait", style: art(&rec.icon) }
                    div { class: "rec-body",
                        div { class: "rec-head",
                            // The one control the app exists to offer, and until
                            // now the one surface that was not a button: every
                            // board tile, the mode switch, the rank picker and
                            // the answer strip already are. A keyboard could
                            // reach the strip's top three and nothing else.
                            //
                            // The name and not the row, and the row is not given
                            // `role="button"` either: it holds the reasons, the
                            // coverage line and the tags, and a button role makes
                            // its descendants presentational in most assistive
                            // tech — hiding the app's shows-its-work from exactly
                            // the readers who need it spoken. Siblings rather
                            // than nesting, which is `MapBoard`'s rule.
                            button {
                                class: "rec-pick",
                                aria_label: "{pick_label(rec)}",
                                onclick: {
                                    let hero = rec.hero;
                                    move |evt: Event<MouseData>| {
                                        // The row's own click still locks for the
                                        // pointer, and the root takes focus back
                                        // on every click. Same guard the strip,
                                        // the reset and the key sheet all need.
                                        evt.stop_propagation();
                                        on_lock.call(hero);
                                    }
                                },
                                "{rec.name}"
                            }
                            if rec.is_locked {
                                span { class: "tag", "current" }
                            }
                            if rec.worth_swapping {
                                span { class: "tag swap-tag", "swap" }
                            }
                            span { class: "score", "{rec.score}" }
                            // A read-out, not a control: the pool board is the
                            // one place it is edited, so there is no second
                            // click target here to mistake for the row's own.
                            //
                            // It names the rung, and that is not decoration. The
                            // reason list is sorted by impact and cut to three, so
                            // the lowest step's 0.12 loses to any three matchups
                            // and the line saying you play this hero never renders
                            // — leaving this glyph as the only account of a term
                            // that moved the score. The same sentence that line
                            // would have carried, out of the same function.
                            if rec.claimed() {
                                span {
                                    class: "star on",
                                    title: "{comfort_claim(rec.comfort)}",
                                    aria_label: "{comfort_claim(rec.comfort)}",
                                    "★"
                                }
                            }
                            // Deliberately not the score, however much better
                            // that would be for density: tapping the number
                            // already locks the hero, and re-pointing a gesture
                            // the hand has learned is the break the rule about
                            // nothing moving mid-draft exists to prevent.
                            button {
                                class: if open_why == Some(rec.hero) { "rec-why open" } else { "rec-why" },
                                r#type: "button",
                                aria_expanded: "{open_why == Some(rec.hero)}",
                                aria_controls: "why",
                                onclick: {
                                    let hero = rec.hero;
                                    move |evt: Event<MouseData>| {
                                        evt.stop_propagation();
                                        on_why.call(hero);
                                    }
                                },
                                "why"
                                span { class: "rec-why-caret", aria_hidden: "true", "\u{25be}" }
                            }
                        }
                        ul { class: "reasons",
                            for (index, line) in rec.reasons.iter().enumerate() {
                                li {
                                    key: "{index}",
                                    class: if line.positive { "reason good" } else { "reason bad" },
                                    "{line.text}"
                                    // A sibling, never appended to `text`: most
                                    // of these sentences are counterpickgg's,
                                    // quoted exactly, and our editorial must not
                                    // read as part of theirs.
                                    if line.disputed {
                                        span {
                                            class: "caveat",
                                            title: "the two sources disagree about this matchup, so the reading has been pulled toward even",
                                            "disputed"
                                        }
                                    }
                                    // Last, and dot-separated: attribution closes
                                    // a quote. 244 of the 254 disputed rows also
                                    // carry a sentence, so this pair of markers
                                    // is the common case rather than the edge,
                                    // and the caveat has to sit tight to the
                                    // claim it qualifies rather than to the name
                                    // of whoever wrote it.
                                    if line.cited {
                                        span {
                                            class: "cite",
                                            title: "quoted from counterpickgg",
                                            "counterpickgg"
                                        }
                                    }
                                }
                            }
                        }
                        // After the reasons, because it is about all of them at
                        // once: how much of the enemy board the sentences above
                        // could have been drawn from. The slot `.ban-worst`
                        // occupies on a ban row, and the register `.threat-note`
                        // uses, which is the panel already admitting this about
                        // the hero you are on.
                        if let Some(coverage) = &rec.coverage {
                            p { class: "rec-coverage", "{coverage}" }
                        }
                    }
                }
            }
            // Standing, and at the foot. Standing because a legend that appears
            // only once there is something to explain is a legend the reader
            // meets after the thing it explains; at the foot because it is about
            // the lines, which is the mirror of `.rank-note` above sitting with
            // the control it qualifies.
            WhyPanel { view: why.clone() }
            p { class: "cite-legend",
                "lines marked counterpickgg are quoted from that site \u{00b7} everything else is this app's own words"
            }
        }
    }
}

/// The top of [`Recommendations`], pinned to the bottom of a phone.
///
/// The stylesheet decides who sees this; it is rendered unconditionally and
/// hidden by default. That is deliberate — the alternative is a component that
/// mounts and unmounts as the viewport changes, which on a phone means it
/// disappears when the screen is turned over, mid-draft, which is the one thing
/// this screen never does.
///
/// Takes the same [`RecRow`] the pick column does rather than a shape of its
/// own. Two renderings of one list are already a risk; two *resolutions* of it
/// would be a build away from disagreeing about what the best pick is.
///
/// The reasons are dropped and only the name and the number survive. This is not
/// the panel — it is the answer, for the moment when the panel is a scroll away.
#[component]
pub fn AnswerStrip(
    /// Ranked, same order as the pick column. Everything past the third is
    /// ignored here rather than by the caller, so the two lists cannot be
    /// sliced differently.
    items: Vec<RecRow>,
    swap_mode: bool,
    on_lock: EventHandler<HeroId>,
) -> Element {
    rsx! {
        div {
            class: "answer-strip",
            // It mirrors a list that is also on the page. Naming it here is what
            // stops a screen reader meeting the same three heroes twice with no
            // account of why.
            aria_label: if swap_mode { "best swaps" } else { "best picks" },
            div { class: "strip-row",
                if items.is_empty() {
                    p { class: "strip-empty", "every hero in this role is already on your team" }
                }
                for rec in items.iter().take(3) {
                    button {
                        key: "{rec.hero.0}",
                        class: format!(
                            "strip-pick{}{}{}",
                            if rec.is_locked { " locked" } else { "" },
                            if rec.worth_swapping { " swap" } else { "" },
                            if rec.claimed() { " pooled" } else { "" },
                        ),
                        // The rank is the reading order here rather than a
                        // column of its own — there is no room for one, and
                        // three items left to right is already an order.
                        aria_label: "{rec.place + 1}. {rec.name}, {rec.score}",
                        onclick: {
                            let hero = rec.hero;
                            move |evt: Event<MouseData>| {
                                // The boards sit inside clickable regions and
                                // the root takes focus back on every click; a
                                // pick from down here must not also read as one
                                // of those. Same guard `ResetButton` needs.
                                evt.stop_propagation();
                                on_lock.call(hero);
                            }
                        },
                        span { class: "strip-portrait", style: art(&rec.icon) }
                        span { class: "strip-name", "{rec.name}" }
                        span { class: "strip-score", "{rec.score}" }
                    }
                }
            }
        }
    }
}

// --- the session --------------------------------------------------------

/// One person in the session, flattened for the view.
///
/// Resolved by the caller, like every other row type here: these components
/// never reach into the dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct RosterRow {
    pub name: String,
    /// The spoken word, from [`Role::label`].
    pub role_label: String,
    /// What they locked, if they have.
    pub hero: Option<String>,
    pub icon: Option<String>,
    pub connected: bool,
    /// Whether this row is the person looking at it.
    pub is_me: bool,
    /// The portraits of what they play, for a seat that has not picked yet.
    ///
    /// Shown where the pick will go, because until it arrives this is the better
    /// answer to the same question — "picking…" says only that they have not,
    /// while a pool says what they are choosing between. Empty once they lock,
    /// where the pick itself is the answer.
    pub pool: Vec<String>,
    /// How many more are in that pool than the strip has room for. A pill row
    /// that grew with somebody's pool would push the rest of the roster around.
    pub pool_extra: usize,
    /// Somebody else in the session has taken the same hero.
    ///
    /// The team cannot field two of them, so the derivation counts the hero
    /// once — which means without this the roster shows two people on it and
    /// the boards show one, with nothing to say why. The game will refuse it in
    /// a moment; the point is that the two of you find out here first.
    pub contested: bool,
    /// The rung of the ladder they are reading patch strength on.
    ///
    /// Shown because a five-stack spanning Gold to Diamond is answering a
    /// different draft than one that does not, and nobody else can know that
    /// unless the roster says so. `Rank::All` is drawn as nothing: most rows will
    /// carry it, and a column of "all ranks" says less than the space it costs.
    pub rank: Rank,
}

/// Who is in the session and what they are on.
///
/// The point of the whole feature, made visible: four names with four heroes
/// next to them is the thing nobody has to type into four separate screens any
/// more. Someone who has dropped keeps their row — the team is still playing
/// around the hero they are on — but is drawn dimmed, because their picks have
/// stopped updating and that has to be legible at a glance.
#[component]
pub fn Roster(rows: Vec<RosterRow>) -> Element {
    rsx! {
        section { class: "roster",
            h3 { class: "board-title", "session" }
            ul { class: "roster-list",
                for row in rows {
                    li {
                        class: if row.connected { "roster-row" } else { "roster-row gone" },
                        span {
                            class: if row.is_me { "roster-name me" } else { "roster-name" },
                            "{row.name}"
                        }
                        span { class: "roster-role", "{row.role_label}" }
                        if row.rank != Rank::All {
                            span {
                                class: "roster-rank",
                                title: "reads patch strength at {row.rank.description()}",
                                "{row.rank.label()}"
                            }
                        }
                        match (row.icon, row.hero) {
                            (Some(icon), Some(hero)) => rsx! {
                                span {
                                    class: if row.contested { "roster-hero contested" } else { "roster-hero" },
                                    span { class: "roster-portrait", style: art(&icon) }
                                    "{hero}"
                                    if row.contested {
                                        span {
                                            class: "roster-clash",
                                            title: "somebody else has taken this hero too",
                                            "×2"
                                        }
                                    }
                                }
                            },
                            // An empty slot is information: they are still
                            // choosing, which is worth seeing during a draft —
                            // and what they are choosing between is worth more,
                            // where they have said.
                            _ if !row.pool.is_empty() => rsx! {
                                span { class: "roster-pool", title: "what they play",
                                    for (index, icon) in row.pool.iter().enumerate() {
                                        span {
                                            key: "{index}",
                                            class: "roster-pool-portrait",
                                            style: art(icon),
                                        }
                                    }
                                    if row.pool_extra > 0 {
                                        span { class: "roster-pool-more", "+{row.pool_extra}" }
                                    }
                                }
                            },
                            _ => rsx! { span { class: "roster-hero unset", "picking…" } },
                        }
                        if !row.connected {
                            span { class: "roster-state", "offline" }
                        }
                    }
                }
            }
        }
    }
}

/// Starting, joining, sharing and leaving a session.
///
/// Two states rather than a mode switch: either you are drafting alone and the
/// bar offers a way not to be, or you are in a session and it shows the code,
/// the link and the way out. Nothing here is on the path to a pick — the app is
/// entirely usable without ever touching this bar, which is why it is one line
/// rather than a screen you have to get past.
#[component]
#[allow(clippy::too_many_arguments)]
pub fn SessionBar(
    /// `None` when drafting alone.
    code: Option<String>,
    share_url: Option<String>,
    qr: Option<String>,
    qr_open: bool,
    status: String,
    name: String,
    /// What is currently typed in the join box.
    entry: String,
    on_entry: EventHandler<String>,
    on_name: EventHandler<String>,
    on_focus: EventHandler<bool>,
    on_create: EventHandler<()>,
    on_join: EventHandler<()>,
    on_leave: EventHandler<()>,
    on_copy: EventHandler<()>,
    on_qr: EventHandler<()>,
) -> Element {
    rsx! {
        section { class: "session",
            div { class: "session-row",
                // Your name travels with your seat, so it is editable in both
                // states: joining a session with a name already set is one less
                // thing to do once four people are waiting.
                input {
                    class: "session-name",
                    r#type: "text",
                    value: "{name}",
                    placeholder: "your name",
                    // The shortcuts live on the root element, so while this has
                    // focus the root has to stop treating letters as commands.
                    onfocusin: move |_| on_focus.call(true),
                    onfocusout: move |_| on_focus.call(false),
                    oninput: move |evt| on_name.call(evt.value()),
                }

                match code {
                    Some(code) => rsx! {
                        span { class: "session-code", title: "share this with your team", "{code}" }
                        if let Some(url) = share_url {
                            button {
                                class: "session-action",
                                title: "{url}",
                                onclick: move |_| on_copy.call(()),
                                "copy link"
                            }
                            // Gated on the link rather than on `qr`, which is
                            // only rendered while the panel is open — keying the
                            // button off it would make the button vanish the
                            // moment you closed the thing it opens.
                            button {
                                class: if qr_open { "session-action on" } else { "session-action" },
                                onclick: move |_| on_qr.call(()),
                                "qr"
                            }
                        }
                        button {
                            class: "session-action leave",
                            onclick: move |_| on_leave.call(()),
                            "leave"
                        }
                    },
                    None => rsx! {
                        button {
                            class: "session-action start",
                            onclick: move |_| on_create.call(()),
                            "start a session"
                        }
                        span { class: "session-or", "or" }
                        input {
                            class: "session-entry",
                            r#type: "text",
                            value: "{entry}",
                            placeholder: "paste a code or link",
                            onfocusin: move |_| on_focus.call(true),
                            onfocusout: move |_| on_focus.call(false),
                            oninput: move |evt| on_entry.call(evt.value()),
                            onkeydown: move |evt| {
                                if evt.key() == Key::Enter {
                                    evt.prevent_default();
                                    on_join.call(());
                                }
                            },
                        }
                        button {
                            class: "session-action",
                            onclick: move |_| on_join.call(()),
                            "join"
                        }
                    },
                }

                span { class: "session-status", "{status}" }
            }

            // Held open deliberately rather than on hover: this exists to be
            // pointed at with a phone, and something that vanishes when the
            // mouse moves cannot be.
            if qr_open {
                if let Some(qr) = qr {
                    div { class: "session-qr",
                        img { src: "{qr}", alt: "session link" }
                    }
                }
            }
        }
    }
}

/// The two parts of this module that can be wrong in a way a test can catch.
///
/// Everything else here is markup, but [`ThreatRow::build`] inverts a sign, and
/// a silently un-inverted threat column would read as the exact opposite of what
/// it means while looking entirely plausible. [`phrasing`] is the second, and it
/// is where the same failure was actually shipped: the words were fixed per kind
/// while the sign in front of them was not, so a hero the data called weak was
/// described as strong with a minus in front of it.
#[cfg(test)]
mod tests {
    use super::*;
    use overwatch_core::{
        Archetype, Breakdown, ComfortStep, DatasetParts, GameMap, GameMode, Hero, Matrix, Reason,
    };

    const REINHARDT: HeroId = HeroId(0);
    const PHARAH: HeroId = HeroId(1);

    fn hero(key: &str, name: &str, role: Role) -> Hero {
        Hero {
            key: key.to_owned(),
            name: name.to_owned(),
            role,
            subrole: None,
            aliases: vec![key.to_owned()],
        }
    }

    fn fixture() -> Dataset {
        dataset(Matrix::unrated(2), vec![false; 4])
    }

    /// Reinhardt rated into Pharah, and the sources fighting over that one pair.
    /// The plain [`fixture`] cannot express this: its matrix is entirely unrated,
    /// and an unrated pair is never disputed.
    fn disputed_fixture() -> Dataset {
        let mut matchups = Matrix::unrated(2);
        matchups
            .set(REINHARDT, PHARAH, -60)
            .expect("the fixture roster has both");
        // Flagged in one direction only, which is the shape the committed data
        // actually has — see `Dataset::sources_disagree`.
        dataset(matchups, vec![false, true, false, false])
    }

    fn dataset(matchups: Matrix, disputed: Vec<bool>) -> Dataset {
        Dataset::new(parts(matchups, disputed)).expect("a two-hero dataset is valid")
    }

    /// The parts, unbuilt, so a fixture that needs one field different can set it
    /// rather than every caller of [`dataset`] passing a value it does not care
    /// about.
    fn parts(matchups: Matrix, disputed: Vec<bool>) -> DatasetParts {
        let heroes = vec![
            hero("reinhardt", "Reinhardt", Role::Tank),
            hero("pharah", "Pharah", Role::Damage),
        ];
        let n = heroes.len();

        DatasetParts {
            heroes,
            maps: Vec::new(),
            matchups,
            synergy: Matrix::unrated(n),
            map_affinity: Vec::new(),
            base_strength: vec![0; n],
            rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
            prevalence: vec![[0; Rank::CHOICES.len()]; n],
            win_rate: vec![None; n],
            side_lean: vec![0; n],
            side_note: vec![String::new(); n],
            shape: vec![[0; 3]; n],
            shape_note: vec![String::new(); n],
            reasons: vec![String::new(); n * n],
            disputed,
            generated: String::new(),
            patch: String::new(),
        }
    }

    /// A roster with a map and published win rates, which the shared fixture
    /// deliberately has neither of. Reinhardt is above the ladder average and
    /// Pharah below it, so one dataset covers both signs of the base term.
    fn phrasing_fixture() -> Dataset {
        let mut parts = parts(Matrix::unrated(2), vec![false; 4]);
        parts.maps = vec![GameMap {
            key: "kings-row".to_owned(),
            name: "King's Row".to_owned(),
            mode: GameMode::Hybrid,
            aliases: Vec::new(),
        }];
        // One row per hero per map, and the values do not matter: the sign a
        // wording is chosen under is passed in, not read back out of the data.
        parts.map_affinity = vec![0; parts.maps.len() * parts.heroes.len()];
        parts.win_rate = vec![Some(50.7), Some(45.6)];
        // Reinhardt is curated and Pharah is not, so one dataset covers both the
        // composed line and the head standing alone. `SideFit` is the arm that
        // needs it most: it is the only `Signed` kind that gains a note, and a
        // sentence that reads correctly after "suits attack" and wrongly after
        // "leans defend" is exactly what that variant exists to catch.
        parts.side_note = vec![
            "Wall denies a choke the attackers must clear.".to_owned(),
            String::new(),
        ];
        parts.shape_note = vec![
            "A held barrier is what a dive has to go through or around.".to_owned(),
            String::new(),
        ];
        // One axis each, so a board of both ties them — rated, and committed to
        // nothing, which is the `mixed` read the `why` panel exists to name.
        // Reaches nothing else here: every other test passes an `Archetype` to
        // `phrasing` directly rather than deriving one from a board.
        parts.shape = vec![[0, 0, 95], [0, 95, 0]];
        Dataset::new(parts).expect("a two-hero dataset with a map is valid")
    }

    fn threat(enemy: HeroId, severity: f32, text: &str) -> Threat {
        Threat {
            enemy,
            severity,
            text: text.to_owned(),
            disputed: false,
        }
    }

    /// The same threat, with the sources contradicting each other about it.
    /// Its own constructor rather than a parameter on [`threat`], which would
    /// put a `false` on six calls that are not about this.
    fn disputed_threat(enemy: HeroId, severity: f32, text: &str) -> Threat {
        Threat {
            disputed: true,
            ..threat(enemy, severity, text)
        }
    }

    /// A disclosure that has quietly stopped listing one of its sources is worse
    /// than no disclosure, and nothing else in the crate can see this table.
    /// The precedent is `keys::SHORTCUTS` against the key handler.
    #[test]
    fn the_how_it_works_sheet_names_every_source_the_dataset_is_built_from() {
        for expected in [
            "OverFast API",
            "counterpickgg",
            "counterwatch",
            "Blizzard hero rates",
            "overpicker",
        ] {
            assert!(
                SOURCES.iter().any(|(name, _, _)| *name == expected),
                "the panel does not name {expected}, which the dataset is built from"
            );
        }

        for (name, url, what) in SOURCES {
            assert!(
                url.starts_with("https://"),
                "{name} is listed without a link"
            );
            assert!(!what.is_empty(), "{name} does not say what it provides");
        }

        // The excluded source is the most persuasive row in the list, and only
        // while it says it is excluded. Without that clause it reads as a fifth
        // input to the numbers.
        let (_, _, overpicker) = SOURCES
            .iter()
            .find(|(name, _, _)| *name == "overpicker")
            .expect("overpicker is listed");
        assert!(
            overpicker.contains("not used"),
            "overpicker's row has to say it is not used: {overpicker}"
        );
    }

    /// The panel claims nothing talks to these sites while you draft. That claim
    /// is only as good as the bundle, so this checks the bundle: no module in
    /// this crate may name a source host, which is the shape a live fetch would
    /// have to take. `ui.rs` is exempt because it holds the table itself.
    #[test]
    fn no_source_row_claims_the_app_talks_to_it_while_you_draft() {
        const MODULES: [(&str, &str); 7] = [
            ("main.rs", include_str!("main.rs")),
            ("sync.rs", include_str!("sync.rs")),
            ("session.rs", include_str!("session.rs")),
            ("board.rs", include_str!("board.rs")),
            ("profile.rs", include_str!("profile.rs")),
            ("matchlog.rs", include_str!("matchlog.rs")),
            ("icons.rs", include_str!("icons.rs")),
        ];

        for (module, text) in MODULES {
            for (name, url, _) in SOURCES {
                let host = url.trim_start_matches("https://");
                let host = host.split('/').next().unwrap_or(host);
                assert!(
                    !text.contains(host),
                    "{module} names {name} ({host}) \u{2014} the panel says the app never \
                     talks to it while you draft"
                );
            }
        }
    }

    #[test]
    fn an_enemy_beating_you_reads_below_zero_like_every_other_column() {
        let ds = fixture();
        let row = ThreatRow::build(&threat(PHARAH, 0.9, ""), REINHARDT, &ds);

        assert_eq!(row.score, "-90", "a hard counter has to read as a loss");
        assert!(!row.favourable);
        assert!(!row.even);
    }

    #[test]
    fn an_enemy_you_beat_keeps_its_place_on_the_list_and_reads_above_zero() {
        let ds = fixture();
        let row = ThreatRow::build(&threat(PHARAH, -0.4, ""), REINHARDT, &ds);

        // The panel is the whole enemy team, not only the half that is winning:
        // being +40 into four of them is what makes a -90 into the fifth
        // survivable, and a filtered list could not say that.
        assert_eq!(row.score, "+40");
        assert!(row.favourable);
    }

    /// `format!("{:+.0}", -0.4)` prints `-0`, which would render a red minus
    /// sign over a matchup nothing measured as negative.
    #[test]
    fn a_severity_that_rounds_to_nothing_never_prints_a_signed_zero() {
        let ds = fixture();

        for severity in [0.004_f32, -0.004, 0.0] {
            let row = ThreatRow::build(&threat(PHARAH, severity, ""), REINHARDT, &ds);
            assert_eq!(row.score, "+0", "severity {severity} printed {}", row.score);
            assert!(row.even, "and must take neither tint");
            assert!(!row.favourable);
        }
    }

    #[test]
    fn the_mirror_says_so_rather_than_looking_like_missing_data() {
        let ds = fixture();
        let row = ThreatRow::build(&threat(REINHARDT, 0.0, ""), REINHARDT, &ds);

        assert_eq!(row.score, "+0");
        assert_eq!(row.text, "the mirror — even by definition");
    }

    /// Only ~40% of pairs carry a scraped sentence, and there is no `kind` here
    /// to phrase a fallback from as `RecRow::build` does — the only thing left
    /// to say would be the number again in words.
    #[test]
    fn an_unexplained_matchup_is_left_bare_rather_than_padded() {
        let ds = fixture();

        let bare = ThreatRow::build(&threat(PHARAH, 0.5, ""), REINHARDT, &ds);
        assert_eq!(bare.text, "");

        let scraped = ThreatRow::build(
            &threat(
                PHARAH,
                0.5,
                "Reinhardt is very weak against airborne targets.",
            ),
            REINHARDT,
            &ds,
        );
        assert_eq!(
            scraped.text,
            "Reinhardt is very weak against airborne targets."
        );
    }

    #[test]
    fn a_hero_the_roster_cannot_name_still_renders_a_row() {
        let ds = fixture();
        let row = ThreatRow::build(&threat(HeroId(99), 0.3, ""), REINHARDT, &ds);

        assert_eq!(row.name, "?", "a dataset mismatch must not blank the panel");
        assert_eq!(row.score, "-30");
    }

    /// The flag has to survive the last hop. `blend` writes it, the loader now
    /// carries it and the scorer resolves it — and for a long time the whole
    /// chain ended in a struct the screen never read.
    #[test]
    fn a_disputed_matchup_says_so_on_the_row() {
        let ds = fixture();

        let row = ThreatRow::build(&disputed_threat(PHARAH, 0.5, ""), REINHARDT, &ds);
        assert!(row.disputed);

        let ordinary = ThreatRow::build(&threat(PHARAH, 0.5, ""), REINHARDT, &ds);
        assert!(
            !ordinary.disputed,
            "an agreed matchup must not be marked, or the marker means nothing"
        );
    }

    /// Only the counter terms read the matchup matrix, so only they can be in
    /// dispute. A marker on the patch-strength line would be pointing at a
    /// number that has no second source behind it to disagree.
    #[test]
    fn only_a_counter_line_can_be_marked_as_disputed() {
        let ds = disputed_fixture();

        let rec = scored(
            REINHARDT,
            -0.3,
            vec![
                Reason {
                    kind: ReasonKind::LosesToEnemy(PHARAH),
                    contribution: -0.3,
                    text: String::new(),
                },
                Reason {
                    kind: ReasonKind::BaseStrength,
                    contribution: 0.1,
                    text: String::new(),
                },
            ],
        );

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        assert!(row.reasons[0].disputed, "the counter line reads the matrix");
        assert!(!row.reasons[0].positive);
        assert!(
            !row.reasons[1].disputed,
            "patch strength has one source and nothing to disagree with"
        );
    }

    fn tile(name: &str, state: TileState, comfort: Option<ComfortStep>) -> HeroTile {
        HeroTile {
            hero: REINHARDT,
            name: name.to_owned(),
            icon: String::new(),
            state,
            owner: None,
            comfort,
        }
    }

    /// The one line telling anyone that this board cycles rather than toggles.
    /// It is also the only place the ladder's three words appear together, so a
    /// rung renamed in core has to be renamed here too.
    #[test]
    fn the_pool_note_names_every_rung_of_the_ladder_and_says_a_click_cycles() {
        for step in ComfortStep::LADDER {
            assert!(
                POOL_NOTE.contains(step.label()),
                "the note does not name {:?}: {POOL_NOTE:?}",
                step
            );
        }
        assert!(POOL_NOTE.contains("cycle"));
        // Written in this app's voice, like every other line it wrote itself.
        assert!(!POOL_NOTE.starts_with(char::is_uppercase));
        // And no line continuation baked its own indentation into the copy,
        // which is how the first version of this shipped to the screen.
        assert!(!POOL_NOTE.contains("  "), "double space in {POOL_NOTE:?}");
    }

    /// The level is drawn by class and by nothing else,    /// The level is drawn by class and by nothing else, so this is what a reader
    /// who cannot tell two ambers apart is actually depending on. One pip per
    /// rung, and every rung distinct.
    #[test]
    fn every_comfort_step_draws_a_different_number_of_pips() {
        let classes: Vec<String> = ComfortStep::LADDER
            .iter()
            .map(|step| tile_class(&tile("Reinhardt", TileState::Free, Some(*step))))
            .collect();

        assert_eq!(classes, vec!["tile c1", "tile c2", "tile c3"]);
        assert_eq!(
            classes
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3,
            "two rungs drawing the same class is two rungs nobody can tell apart"
        );
        assert_eq!(
            tile_class(&tile("Reinhardt", TileState::Free, None)),
            "tile",
            "and an unclaimed tile is bare"
        );
        // The comfort class rides *beside* the state's rather than replacing it,
        // so a board that has both keeps both.
        assert_eq!(
            tile_class(&tile("Reinhardt", TileState::Picked, Some(ComfortStep::Ok))),
            "tile selected c1"
        );
    }

    /// The level in words, through the label the board already had. It fires on
    /// hover, focus-visible and `:active`, so this is the only cue that reaches
    /// every pointer kind — the pips are the count, this is the name.
    #[test]
    fn a_pool_tile_names_its_comfort_level_in_the_label_it_already_had() {
        assert_eq!(
            tile_label(&tile("Reinhardt", TileState::Free, Some(ComfortStep::Main))),
            "Reinhardt \u{b7} main"
        );
        assert_eq!(
            tile_label(&tile("Reinhardt", TileState::Free, Some(ComfortStep::Ok))),
            "Reinhardt \u{b7} ok"
        );
        assert_eq!(
            tile_label(&tile("Reinhardt", TileState::Free, None)),
            "Reinhardt",
            "an unclaimed tile is still just the hero"
        );

        // The other arm, unchanged: a teammate's pick names them instead. The
        // two cannot collide, because the pool board never produces `Theirs`.
        let theirs = HeroTile {
            owner: Some("mika".to_owned()),
            ..tile("Ana", TileState::Theirs, None)
        };
        assert_eq!(tile_label(&theirs), "Ana \u{b7} mika");
    }

    /// The whole point of the slice: a line carrying somebody else's sentence
    /// The whole point of the slice: a line carrying somebody else's sentence
    /// says so, and a line the app wrote about its own arithmetic does not.
    #[test]
    fn a_scraped_sentence_is_marked_as_quoted_and_a_generated_line_is_not() {
        let ds = phrasing_fixture();

        let rec = scored(
            REINHARDT,
            0.2,
            vec![
                Reason {
                    kind: ReasonKind::LosesToEnemy(PHARAH),
                    contribution: -0.3,
                    text: "Nothing Reinhardt does reaches the air.".to_owned(),
                },
                Reason {
                    kind: ReasonKind::BaseStrength,
                    contribution: 0.1,
                    text: String::new(),
                },
            ],
        );

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        assert!(row.reasons[0].cited, "the site wrote that sentence");
        assert_eq!(
            row.reasons[0].text,
            "Nothing Reinhardt does reaches the air."
        );
        assert!(
            !row.reasons[1].cited,
            "a win rate this app formatted is nobody's quotation"
        );
    }

    /// The forward guard, and the only test that fails if the kind gate in
    /// [`RecRow::build`] is "simplified" into a bare `!text.is_empty()`.
    ///
    /// `side.toml` and `archetype.toml` are hand-written in this repository and
    /// their notes are capitalised prose exactly like the site's sentences, so the
    /// typographic register cannot tell the two apart and the moment those notes
    /// reach `Reason.text` the only thing standing between them and a false
    /// attribution is the kind.
    #[test]
    fn a_hand_written_note_is_not_attributed_to_the_site_that_did_not_write_it() {
        let ds = phrasing_fixture();

        let rec = scored(
            REINHARDT,
            0.2,
            vec![Reason {
                kind: ReasonKind::SideFit(Side::Attack),
                contribution: 0.1,
                text: "Wall denies a choke the attackers must clear.".to_owned(),
            }],
        );

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        assert_eq!(
            row.reasons[0].text,
            "Wall denies a choke the attackers must clear."
        );
        assert!(
            !row.reasons[0].cited,
            "this repository wrote that note, and crediting counterpickgg with it \
             is the failure the kind gate exists to prevent"
        );
    }

    /// One reason on one row, resolved. Three tests below differ only in the kind
    /// and the sign, and spelling `Recommendation` out three times would bury
    /// that.
    fn one_reason(hero: HeroId, kind: ReasonKind, contribution: f32, ds: &Dataset) -> String {
        let rec = scored(
            hero,
            contribution,
            vec![Reason {
                kind,
                contribution,
                text: String::new(),
            }],
        );
        RecRow::build(&rec, ds, false, 0, Rank::All)
            .reasons
            .remove(0)
            .text
    }

    /// The head names *their* leading axis and the note is about *this* kit, so
    /// replacing one with the other loses half the sentence either way. Both
    /// survive, in that order.
    #[test]
    fn a_shape_reason_names_their_shape_before_saying_why_this_kit_answers_it() {
        let ds = phrasing_fixture();

        let line = one_reason(
            REINHARDT,
            ReasonKind::CountersShape(Archetype::Dive),
            0.2,
            &ds,
        );
        assert_eq!(
            line,
            "answers their dive \u{2014} A held barrier is what a dive has to go through or around."
        );
        assert!(
            line.starts_with("answers their dive"),
            "the head has to come first: it is what the number is about"
        );
    }

    /// A hero nobody has written a note for reads exactly as it did before this
    /// existed. The dataset says nothing and the row says the head, rather than
    /// the head plus a dangling dash.
    #[test]
    fn a_shape_line_keeps_its_head_when_there_is_no_note_behind_it() {
        let ds = phrasing_fixture();

        let line = one_reason(PHARAH, ReasonKind::LosesToShape(Archetype::Dive), -0.2, &ds);
        assert_eq!(line, "walks into their dive");
        assert!(
            !line.contains('\u{2014}'),
            "an empty note must not leave its separator behind"
        );
    }

    /// `SideFit` is the only `Signed` kind that gains prose, so it is the only one
    /// where the note has to read correctly after two different heads. A sentence
    /// that argues for attacking, printed after "leans defend", would be the row
    /// contradicting itself in the middle.
    #[test]
    fn a_side_note_reads_the_same_under_both_wordings_of_its_sign() {
        let ds = phrasing_fixture();
        const NOTE: &str = "Wall denies a choke the attackers must clear.";

        let up = one_reason(REINHARDT, ReasonKind::SideFit(Side::Attack), 0.2, &ds);
        let down = one_reason(REINHARDT, ReasonKind::SideFit(Side::Attack), -0.2, &ds);

        assert_eq!(up, format!("suits attack \u{2014} {NOTE}"));
        assert_eq!(down, format!("leans defend \u{2014} {NOTE}"));
        assert_ne!(up, down, "the head still turns with the sign");
    }

    /// The mirror line is the app's own, and it is the reason this panel is
    /// marked per row rather than carrying one note over the column.
    /// The mirror line is the app's own, and it is the reason this panel is
    /// marked per row rather than carrying one note over the column.
    #[test]
    fn the_mirror_line_is_the_apps_own_and_is_never_attributed() {
        let ds = fixture();

        let row = ThreatRow::build(&threat(REINHARDT, 0.0, ""), REINHARDT, &ds);
        assert_eq!(row.text, "the mirror \u{2014} even by definition");
        assert!(!row.cited, "no site wrote that line");

        let quoted = ThreatRow::build(&threat(PHARAH, 0.5, "Pharah flies."), REINHARDT, &ds);
        assert!(quoted.cited, "that sentence came out of the matrix");
    }

    /// The patch rung shows a win rate this app formatted, not a claim about a
    /// pair, so the marker must not follow it. The bug it forecloses is one
    /// `match` growing a second reader that decides the attribution separately
    /// from the line.
    #[test]
    fn the_ban_panels_win_rate_figure_is_never_attributed_to_counterpickgg() {
        let (text, cited) = ban_text(Some(52.4), true, "Something the site said.", Rank::All);
        assert_eq!(text, "52.4% win rate");
        assert!(!cited, "the figure is ours and the wording is ours");

        let (text, cited) = ban_text(Some(52.4), false, "Something the site said.", Rank::All);
        assert_eq!(text, "Something the site said.");
        assert!(cited);

        // No pair rated, no figure to fall back to: a bare row, and an empty
        // string is not a quotation of anything.
        let (text, cited) = ban_text(None, true, "", Rank::Master);
        assert!(text.is_empty());
        assert!(!cited);
    }

    /// Locks the typographic channel the attribution rests on. Every wording this
    /// app generates is lowercase-initial with no terminal full stop, and every
    /// one of the 1,066 committed sentences is the opposite — so a reader who has
    /// met the legend once can tell the registers apart even where a marker has
    /// scrolled out of view.
    ///
    /// Iterates the same table [`every_reason_kind_renders_a_line_under_both_signs`]
    /// does, which is what makes the compiler's exhaustive `match` carry this test
    /// too: a new [`ReasonKind`] cannot skip the arm, and the arm cannot skip
    /// this.
    #[test]
    fn no_generated_line_is_capitalised_or_ends_in_a_full_stop() {
        let ds = phrasing_fixture();
        let map = MapId(0);

        let kinds = [
            ReasonKind::BeatsEnemy(PHARAH),
            ReasonKind::LosesToEnemy(PHARAH),
            ReasonKind::CountersShape(Archetype::Dive),
            ReasonKind::LosesToShape(Archetype::Dive),
            ReasonKind::BaseStrength,
            ReasonKind::RankFit(Rank::Master),
            ReasonKind::SideFit(Side::Attack),
            ReasonKind::MapFit(map),
            ReasonKind::PairsWithAlly(PHARAH),
            ReasonKind::Comfort(ComfortStep::Main.value()),
        ];

        let mut lines: Vec<String> = Vec::new();
        for kind in kinds {
            lines.push(phrasing(kind, REINHARDT, Rank::All, &ds).under(true));
            lines.push(phrasing(kind, REINHARDT, Rank::All, &ds).under(false));
        }
        // The two the table above cannot reach: the arm taken when a hero has no
        // published win rate, and the line the matchups panel writes for a mirror.
        lines.push(phrasing(ReasonKind::BaseStrength, HeroId(99), Rank::All, &ds).under(true));
        lines.push(ThreatRow::build(&threat(REINHARDT, 0.0, ""), REINHARDT, &ds).text);

        for line in &lines {
            assert!(
                !line.starts_with(char::is_uppercase),
                "{line:?} opens like a quotation, and this app writes in lowercase"
            );
            assert!(
                !line.ends_with('.'),
                "{line:?} closes like a quotation, and this app does not punctuate its own labels"
            );
        }
    }

    /// Every kind, both signs. The compiler is the real guard — a kind added to
    /// [`ReasonKind`] cannot skip the match — so what this catches is an arm
    /// wired to the wrong variant: a claim left `Symmetric`, or a label given two
    /// wordings it does not need.
    #[test]
    fn every_reason_kind_renders_a_line_under_both_signs() {
        let ds = phrasing_fixture();
        let map = MapId(0);

        // `true` where the phrase makes a claim of its own and the negative
        // reading is a different sentence rather than the same one negated.
        let kinds = [
            (ReasonKind::BeatsEnemy(PHARAH), false),
            (ReasonKind::LosesToEnemy(PHARAH), false),
            (ReasonKind::CountersShape(Archetype::Dive), false),
            (ReasonKind::LosesToShape(Archetype::Dive), false),
            (ReasonKind::BaseStrength, false),
            (ReasonKind::RankFit(Rank::Master), false),
            (ReasonKind::SideFit(Side::Attack), true),
            (ReasonKind::MapFit(map), true),
            (ReasonKind::PairsWithAlly(PHARAH), true),
            (ReasonKind::Comfort(ComfortStep::Main.value()), true),
        ];

        for (kind, signed) in kinds {
            let up = phrasing(kind, REINHARDT, Rank::All, &ds).under(true);
            let down = phrasing(kind, REINHARDT, Rank::All, &ds).under(false);

            assert!(!up.is_empty(), "{kind:?} renders nothing when positive");
            assert!(!down.is_empty(), "{kind:?} renders nothing when negative");
            assert!(
                !up.contains('?'),
                "{kind:?} could not resolve a name: {up:?}"
            );
            if signed {
                assert_ne!(
                    up, down,
                    "{kind:?} claims something, so the minus needs its own wording"
                );
            } else {
                assert_eq!(
                    up, down,
                    "{kind:?} names a thing rather than claiming one, so both signs read it"
                );
            }
        }
    }

    /// The counter templates are rendered *only* where no source published a
    /// sentence, and that is overwhelmingly where the edge is smallest: over the
    /// committed matrix the 1,416 rows reaching a template top out at |term| 47,
    /// while all 530 rows at 50 or more carry prose that replaces it. So the app
    /// called a matchup strong on exactly the set that has no strength in it.
    ///
    /// The absent words are asserted rather than only the new string, because
    /// they are what the slice is about: a future rewording should have to meet
    /// this argument again rather than slip past a string comparison.
    #[test]
    fn a_slim_matchup_edge_is_not_described_as_strong() {
        let ds = phrasing_fixture();

        for kind in [
            ReasonKind::BeatsEnemy(PHARAH),
            ReasonKind::LosesToEnemy(PHARAH),
        ] {
            let line = phrasing(kind, REINHARDT, Rank::All, &ds).under(true);
            assert!(
                line.contains("Pharah"),
                "{kind:?} has to name the enemy: {line:?}"
            );
            assert!(
                !line.contains("strong"),
                "{kind:?} claims a size the reading behind it never reaches: {line:?}"
            );
            assert!(
                !line.contains("struggles"),
                "{kind:?} claims a size the reading behind it never reaches: {line:?}"
            );
            assert!(
                line.starts_with("rated "),
                "the app has a number from a source, and the verb should say so: {line:?}"
            );
        }
    }

    /// Dropping the claim must not drop the direction. The number beside the row
    /// is the candidate's whole score rather than this pair's, so the line is the
    /// only thing on screen saying which way one matchup goes.
    #[test]
    fn a_matchup_with_no_published_note_still_says_which_way_it_goes() {
        let ds = phrasing_fixture();

        let ahead = one_reason(REINHARDT, ReasonKind::BeatsEnemy(PHARAH), 0.2, &ds);
        let behind = one_reason(REINHARDT, ReasonKind::LosesToEnemy(PHARAH), -0.2, &ds);

        assert_eq!(ahead, "rated ahead of Pharah");
        assert_eq!(behind, "rated behind Pharah");
        assert_ne!(
            ahead, behind,
            "the two kinds have to read differently, or the row says nothing at all"
        );
    }

    /// The reported bug. Twenty-two of the fifty-three heroes have a negative
    /// base term, and every one of them was described as strong.
    /// The reported bug. Twenty-two of the fifty-three heroes have a negative
    /// base term, and every one of them was described as strong.
    #[test]
    fn a_hero_below_the_ladder_average_shows_its_win_rate_rather_than_being_called_strong() {
        let ds = phrasing_fixture();

        let line = phrasing(ReasonKind::BaseStrength, PHARAH, Rank::All, &ds).under(false);
        assert_eq!(line, "45.6% win rate");
        assert!(
            !line.contains("strong"),
            "a minus in front of 'strong' is the bug this closes"
        );

        // The figure is the hero's either way: the sign says what the term did to
        // the score, not what the number is.
        let above = phrasing(ReasonKind::BaseStrength, REINHARDT, Rank::All, &ds).under(true);
        assert_eq!(above, "50.7% win rate");
    }

    /// The published rate is the whole ladder's at every rung, so a chosen rung
    /// has to be told apart from it. Same words as the ban panel, from the same
    /// function, because a qualifier on one of them and not the other would be
    /// worse than neither.
    #[test]
    fn the_win_rate_on_a_reason_line_is_qualified_once_a_rank_is_chosen() {
        let ds = phrasing_fixture();

        assert_eq!(
            phrasing(ReasonKind::BaseStrength, REINHARDT, Rank::Grandmaster, &ds).under(true),
            "50.7% win rate across the ladder"
        );
        assert_eq!(
            win_rate_text(50.7, Rank::Grandmaster),
            phrasing(ReasonKind::BaseStrength, REINHARDT, Rank::Grandmaster, &ds).under(true),
            "the ban panel and the reason line must print one string"
        );
    }

    /// Unreachable on the committed data, where all 53 heroes carry a published
    /// rate — but the field is an `Option` and every other fixture in this module
    /// passes `None`, so the row still has to say something.
    #[test]
    fn a_hero_with_no_published_win_rate_still_gets_a_line() {
        let ds = fixture();

        for positive in [true, false] {
            assert_eq!(
                phrasing(ReasonKind::BaseStrength, REINHARDT, Rank::All, &ds).under(positive),
                "patch strength",
                "a label makes no claim, so it needs no second wording"
            );
        }
    }

    /// A rung has no opposite, which is why this one line may not become a
    /// comparative however the sign in front of it reads. The argument is in
    /// [`phrasing`] and this is what holds it.
    #[test]
    fn a_rung_is_never_phrased_as_a_comparative_because_a_rung_has_no_opposite() {
        let ds = phrasing_fixture();
        let kind = ReasonKind::RankFit(Rank::Master);

        let up = phrasing(kind, REINHARDT, Rank::Master, &ds).under(true);
        assert_eq!(
            up,
            phrasing(kind, REINHARDT, Rank::Master, &ds).under(false)
        );
        for word in ["stronger", "weaker", "better", "worse"] {
            assert!(!up.contains(word), "{up:?} reads as a comparison");
        }
    }

    /// A side does have an opposite, so the negative reading names it rather than
    /// leaving the reader to work out that "not attack" is a place.
    #[test]
    fn a_hero_leaning_the_other_way_names_the_side_it_actually_wants() {
        let ds = phrasing_fixture();

        let on_attack = ReasonKind::SideFit(Side::Attack);
        assert_eq!(
            phrasing(on_attack, REINHARDT, Rank::All, &ds).under(true),
            "suits attack"
        );
        assert_eq!(
            phrasing(on_attack, REINHARDT, Rank::All, &ds).under(false),
            "leans defend",
            "a defend-leaning hero read on attack has to name defend"
        );

        // And symmetrically from the other side, because the payload is the side
        // you are on rather than the one the kit wants.
        assert_eq!(
            phrasing(ReasonKind::SideFit(Side::Defend), REINHARDT, Rank::All, &ds).under(false),
            "leans attack"
        );
    }

    /// No committed map affinity is negative — 159 rows, all of them positive —
    /// so this arm is unreachable from the data and testable only here. It stops
    /// being unreachable the day the worst-map figures land.
    #[test]
    fn a_negative_map_affinity_would_read_as_a_poor_fit_rather_than_performing_well() {
        let ds = phrasing_fixture();
        let kind = ReasonKind::MapFit(MapId(0));

        assert_eq!(
            phrasing(kind, REINHARDT, Rank::All, &ds).under(true),
            "performs well on King's Row"
        );
        assert_eq!(
            phrasing(kind, REINHARDT, Rank::All, &ds).under(false),
            "a poor fit for King's Row"
        );
    }

    /// Same again for duos: all 441 committed synergy ratings are positive, so
    /// only this can check the other half.
    #[test]
    fn a_negative_synergy_value_would_read_as_a_poor_pair_rather_than_pairing_well() {
        let ds = phrasing_fixture();
        let kind = ReasonKind::PairsWithAlly(PHARAH);

        assert_eq!(
            phrasing(kind, REINHARDT, Rank::All, &ds).under(true),
            "pairs well with Pharah"
        );
        assert_eq!(
            phrasing(kind, REINHARDT, Rank::All, &ds).under(false),
            "a poor pair with Pharah"
        );
    }

    /// The pool board writes the three positive rungs and nothing else, so a
    /// negative still needs a hand-edited stored profile to reach. Worth keeping:
    /// a level you set yourself is the last place the app should argue with
    /// itself, and this is the one wording the ladder does not supply.
    #[test]
    fn a_hero_you_rated_down_never_reads_as_one_of_your_comfort_picks() {
        let ds = phrasing_fixture();

        // A negative is off the ladder by construction - there is no negative
        // step - so this is the hand-edited stored profile the arm exists for.
        let line = phrasing(ReasonKind::Comfort(-40), REINHARDT, Rank::All, &ds).under(false);
        assert_eq!(line, "one you rated down");
        assert!(!line.contains("comfort pick"));
    }

    /// Three rungs, three claims. The pool board has always been able to say "you
    /// play it", "you play it well" and "this is your hero", and every one of them
    /// arrived on the row as the same sentence.
    ///
    /// Driven off `LADDER` rather than off three literals, so a fourth rung cannot
    /// be added without an answer for it here.
    #[test]
    fn each_rung_of_the_comfort_ladder_reads_as_a_different_claim() {
        let ds = phrasing_fixture();

        let mut seen: Vec<String> = Vec::new();
        for step in ComfortStep::LADDER {
            let line =
                phrasing(ReasonKind::Comfort(step.value()), REINHARDT, Rank::All, &ds).under(true);
            assert!(
                !seen.contains(&line),
                "{} reads exactly as a rung below it, which is the whole bug",
                step.label()
            );
            seen.push(line);
        }
    }

    /// The top of the ladder at 0.60 is the heaviest single claim the app makes
    /// about a hero, and it used to be worded identically to the 0.12 below it.
    #[test]
    fn the_top_of_the_ladder_is_named_a_main_rather_than_a_comfort_pick() {
        let ds = phrasing_fixture();

        let line = phrasing(
            ReasonKind::Comfort(ComfortStep::Main.value()),
            REINHARDT,
            Rank::All,
            &ds,
        )
        .under(true);
        assert_eq!(line, "one of your mains");
    }

    /// [`ComfortStep::of`] is an exact match on purpose, so the sentence the row
    /// carried before the ladder had words is the honest answer for a value the
    /// ladder cannot name. That is why the old string is kept rather than deleted:
    /// it is reachable, through a hand-edited profile, and now tested.
    #[test]
    fn a_comfort_value_the_ladder_cannot_name_keeps_the_wording_it_always_had() {
        let ds = phrasing_fixture();

        assert_eq!(
            ComfortStep::of(21),
            None,
            "21 has to be off the ladder or this test asserts nothing"
        );
        let line = phrasing(ReasonKind::Comfort(21), REINHARDT, Rank::All, &ds).under(true);
        assert_eq!(line, "one of your comfort picks");
    }

    /// The star and the bar ask "is this one of yours", which is `> 0` — the same
    /// predicate `Profile::pool` derives the pool from, the one the mode chip
    /// counts and the one the ban list defends on.
    ///
    /// Were membership read off `ComfortStep::of(..).is_some()` instead, this row
    /// would go out unstarred and unbarred while its own reason line called the
    /// hero one of your comfort picks and the mode chip beside it counted a hero
    /// the list said was not yours.
    #[test]
    fn a_comfort_value_the_ladder_cannot_name_still_marks_the_row_as_yours() {
        let ds = phrasing_fixture();
        let rec = scored(REINHARDT, 0.4, Vec::new());

        assert!(RecRow::build(&rec, &ds, false, 21, Rank::All).claimed());
        assert!(!RecRow::build(&rec, &ds, false, 0, Rank::All).claimed());
        assert!(
            !RecRow::build(&rec, &ds, false, -40, Rank::All).claimed(),
            "a hero you rated down is not one of yours at a low level, it is not \
             one of yours"
        );
    }

    /// Two channels, one sentence. The star exists because the reason list is
    /// impact-sorted and truncated, so at the lowest rung it is regularly the only
    /// survivor — and a star saying something the line does not is worse than no
    /// star at all.
    #[test]
    fn the_star_names_the_same_rung_the_reason_line_does() {
        let ds = phrasing_fixture();

        for value in [
            ComfortStep::Ok.value(),
            ComfortStep::Good.value(),
            ComfortStep::Main.value(),
            21,
        ] {
            assert_eq!(
                comfort_claim(value),
                phrasing(ReasonKind::Comfort(value), REINHARDT, Rank::All, &ds).under(true),
                "the header and the reasons disagree about {value}"
            );
        }
    }

    /// The row carries the number rather than the fact that there is one.
    /// Everything above depends on it still being there when the components read
    /// it.
    #[test]
    fn a_row_carries_the_comfort_value_and_not_merely_that_there_is_one() {
        let ds = phrasing_fixture();

        let row = RecRow::build(
            &scored(REINHARDT, 0.4, Vec::new()),
            &ds,
            false,
            ComfortStep::Good.value(),
            Rank::All,
        );
        assert_eq!(row.comfort, ComfortStep::Good.value());
        assert!(row.claimed());
    }

    /// One scored recommendation, for the tests that are about how a row is
    /// rendered rather than about how it was ranked.
    ///
    /// Six of these differed only in the hero, the number and the reasons, and
    /// spelling out the four fields that never varied buried the three that did.
    /// It is also what made `breakdown` and `place` one edit instead of six.
    ///
    /// `Breakdown::default()` because none of these rows is about the arithmetic:
    /// the ledger is exercised where it is built, in the scorer's own tests.
    /// The scale is the half a newcomer needs, so it leads whenever there is a
    /// scale to state. `+41` beside a portrait reads as a percentage otherwise,
    /// and nothing else on the screen corrects that.
    #[test]
    fn the_note_states_the_scale_when_nothing_is_locked() {
        let note = score_note(None, Some("Winston"), Some(6), 0, 8, false, 0.15);

        assert_eq!(
            note,
            "weighted sum, not a percentage \u{2014} Winston leads the next by 6"
        );
    }

    /// Locked, the column stops being a score and becomes a gain over one hero,
    /// so the sentence changes rather than gaining a clause. Naming that hero is
    /// the whole point: "the gain" over an unnamed something is not a scale.
    #[test]
    fn the_note_names_the_hero_the_column_is_measured_against() {
        let note = score_note(
            Some("Reinhardt"),
            Some("Winston"),
            Some(6),
            0,
            8,
            true,
            0.15,
        );

        assert!(
            note.starts_with("the column is the gain over Reinhardt"),
            "{note}"
        );
        assert!(
            !note.contains("weighted sum"),
            "the column is not a weighted sum any more, so saying it is is worse \
             than saying nothing: {note}"
        );
        assert!(
            !note.contains("Winston"),
            "the leader is not what this column is about"
        );
    }

    /// The two locked arms are different answers to "is there anything here", and
    /// a reader who cannot tell them apart has to compare eight numbers against a
    /// threshold by hand.
    #[test]
    fn the_note_says_when_nothing_clears_the_swap_bar() {
        let nothing = score_note(
            Some("Reinhardt"),
            Some("Winston"),
            Some(6),
            0,
            8,
            false,
            0.15,
        );
        let something = score_note(
            Some("Reinhardt"),
            Some("Winston"),
            Some(6),
            0,
            8,
            true,
            0.15,
        );

        assert!(nothing.contains("nothing here clears +15"), "{nothing}");
        assert_ne!(nothing, something);
    }

    /// The first time `swap_threshold` is a number on screen, so it has to be
    /// *the* number: a literal 15 would go on lying the moment a stored profile
    /// moved the weight `worth_swapping` is actually measured against.
    #[test]
    fn the_swap_bar_on_the_note_is_read_off_the_stored_weight() {
        let moved = score_note(Some("Reinhardt"), None, None, 0, 8, true, 0.25);

        assert!(moved.contains("+25"), "{moved}");
        assert!(
            !moved.contains("+15"),
            "the default leaked past the argument"
        );
    }

    /// The margin is the difference of the figures the rows print, so two heroes
    /// inside 0.005 of each other give a lead of zero — often, not rarely. "leads
    /// the next by 0" is a sentence that reads as a bug.
    #[test]
    fn the_top_two_reading_the_same_is_said_rather_than_printed_as_a_lead_of_zero() {
        let note = score_note(None, Some("Winston"), Some(0), 0, 8, false, 0.15);

        assert!(note.ends_with("the top two are level"), "{note}");
        assert!(
            !note.contains(" 0"),
            "a margin of nothing is not a margin: {note}"
        );
    }

    /// One candidate has no next row to lead, and `by 0` would be as wrong there
    /// as a margin against a hero that is not on the list.
    #[test]
    fn a_role_with_one_candidate_left_says_so_rather_than_naming_a_margin() {
        let alone = score_note(None, Some("Winston"), None, 0, 1, false, 0.15);
        assert!(
            alone.ends_with("the only hero left in this role"),
            "{alone}"
        );

        // And with nothing at all, the scale still stands on its own — the panel
        // below says why the list is empty; this says what its numbers meant.
        assert_eq!(
            score_note(None, None, None, 0, 0, false, 0.15),
            "weighted sum, not a percentage"
        );
    }

    /// Every line this app writes itself is lowercase and unpunctuated, and this
    /// one is assembled from pieces rather than written out, which is exactly how
    /// a stray capital or a doubled space gets in.
    #[test]
    fn the_score_note_is_written_in_this_apps_own_voice_under_every_state() {
        let states = [
            score_note(None, Some("Winston"), Some(6), 0, 8, false, 0.15),
            score_note(None, Some("Winston"), Some(0), 0, 8, false, 0.15),
            score_note(None, Some("Winston"), None, 0, 1, false, 0.15),
            score_note(None, None, None, 0, 0, false, 0.15),
            score_note(Some("Reinhardt"), None, None, 0, 8, true, 0.15),
            score_note(Some("Reinhardt"), None, None, 0, 8, false, 0.15),
            score_note(None, Some("Winston"), Some(0), 3, 8, false, 0.15),
            score_note(None, Some("Winston"), Some(0), 8, 8, false, 0.15),
        ];
        for note in states {
            assert!(!note.starts_with(char::is_uppercase), "{note}");
            assert!(!note.ends_with('.'), "{note}");
            assert!(!note.contains("  "), "double space in {note:?}");
        }
    }

    /// The claim the panel's own subtitle makes out loud, and the reason the
    /// ledger exists at all: the three sentences on the row never could.
    #[test]
    fn the_breakdown_rows_add_up_to_the_number_on_the_row() {
        let ds = phrasing_fixture();
        let rec = ledger_rec();

        let view = WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds);
        let summed: i32 = view
            .terms
            .iter()
            .map(|term| term.value.parse::<i32>().expect("a signed integer"))
            .sum();

        assert_eq!(format!("{summed:+}"), view.total);
        assert_eq!(
            view.total,
            RecRow::build(&rec, &ds, false, 0, Rank::All).score
        );
    }

    /// Showing the zeros is the entire point. A term at nothing is a real reading
    /// with no sentence to it, and the row is forbidden from mentioning it.
    #[test]
    fn a_term_that_came_to_nothing_is_still_a_row_and_takes_neither_tint() {
        let ds = phrasing_fixture();
        let view = WhyView::build(&ledger_rec(), None, 0, 0.15, Rank::All, &ds);

        let side = view
            .terms
            .iter()
            .find(|term| term.label == TermKind::Side.label())
            .expect("every term is a row");

        assert_eq!(side.value, "+0");
        assert!(side.even, "a dead flat term is not an argument either way");
        assert!(!side.positive);
    }

    /// Never sorted by contribution: that is the reason list below it. A table
    /// whose rows move between heroes is one you re-read every time.
    #[test]
    fn the_ledger_reads_in_the_order_the_score_is_summed() {
        let ds = phrasing_fixture();
        let view = WhyView::build(&ledger_rec(), None, 0, 0.15, Rank::All, &ds);

        let labels: Vec<&str> = view.terms.iter().map(|term| term.label).collect();
        let expected: Vec<&str> = TermKind::ALL.into_iter().map(TermKind::label).collect();
        assert_eq!(labels, expected);
    }

    /// The one term that can move a score with no reason line behind it. A mixed
    /// board is rated and has committed to nothing, so `CountersShape` has no
    /// archetype to name and the panel is the only place this can be said.
    #[test]
    fn a_mixed_enemy_shape_is_named_in_the_panel_even_though_no_reason_line_can_carry_it() {
        let ds = phrasing_fixture();
        let mut rec = ledger_rec();
        rec.breakdown.shape = mixed_shape(&ds);

        let view = WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds);
        let shape = view.shape.expect("a mixed board is worth saying");
        assert!(shape.contains("mixed"), "{shape}");
        assert!(
            !view
                .reasons
                .iter()
                .any(|line| line.text.contains("answers their")),
            "a mixed board has no axis for a reason line to name"
        );
    }

    /// The other half. An axis that leads has a reason line that can name it, so
    /// the panel says nothing and the sentence carries it.
    #[test]
    fn an_enemy_shape_that_leads_is_left_to_the_reason_line_that_can_name_it() {
        let ds = phrasing_fixture();
        let mut rec = ledger_rec();
        rec.breakdown.shape = leading_shape(&ds);

        assert!(WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds)
            .shape
            .is_none());
    }

    /// What dropping `take(MAX_REASONS)` buys, and the answer to "which enemies
    /// made that -32".
    #[test]
    fn the_panel_lists_every_reason_rather_than_the_three_the_row_has_room_for() {
        let ds = phrasing_fixture();
        let rec = many_reasons_rec();
        assert!(rec.reasons.len() > MAX_REASONS, "the fixture tests nothing");

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        let view = WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds);

        assert_eq!(row.reasons.len(), MAX_REASONS);
        assert_eq!(view.reasons.len(), rec.reasons.len());
    }

    /// One builder, so a sentence cannot read one way on the row and another in
    /// the panel — and, more sharply, so a quoted sentence cannot lose its
    /// attribution on the way down the page.
    #[test]
    fn a_reason_reads_the_same_in_the_panel_as_it_does_on_the_row() {
        let ds = phrasing_fixture();
        let rec = many_reasons_rec();

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        let view = WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds);

        assert_eq!(row.reasons, view.reasons[..MAX_REASONS]);
        assert!(
            view.reasons.iter().any(|line| line.cited),
            "the fixture no longer carries a quoted sentence, so this tests nothing"
        );
    }

    /// The row goes quiet on a complete read because a fraction on every row is
    /// noise. Here the question is "how much of this did you know", so silence
    /// would be the one answer that does not answer it.
    #[test]
    fn the_panel_states_its_coverage_even_where_the_row_stays_silent() {
        let ds = phrasing_fixture();
        let mut rec = ledger_rec();
        rec.breakdown.counter = Coverage {
            rated: 4,
            entered: 4,
        };

        assert_eq!(coverage_note(rec.breakdown.counter), None);
        let view = WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds);
        assert_eq!(view.coverage, "read against all 4 of their picks");
    }

    /// The top hero is inside the band of itself, always, so the flag alone would
    /// print a tie on every leading row in the app.
    #[test]
    fn only_a_real_tie_puts_the_tie_line_in_the_panel() {
        let ds = phrasing_fixture();
        let mut rec = ledger_rec();
        rec.tied_with_top = true;

        assert!(
            WhyView::build(&rec, None, 1, 0.15, Rank::All, &ds)
                .tie
                .is_none(),
            "one is not a tie"
        );
        let tied = WhyView::build(&rec, None, 3, 0.15, Rank::All, &ds)
            .tie
            .expect("three rows the scorer could not separate");
        assert!(
            tied.contains("15"),
            "the band is read off the weight: {tied}"
        );
    }

    /// The claim the panel makes out loud, on terms that do not land on whole
    /// points — which is every real draft. Eight independently rounded values are
    /// not the rounding of their sum, and the subtitle promises they are.
    ///
    /// `ledger_rec`'s terms all come out at exact points, so the test above it
    /// agreed with the implementation for a reason unrelated to the property.
    #[test]
    fn the_ledger_adds_up_when_its_terms_do_not_land_on_whole_points() {
        let ds = phrasing_fixture();
        let rec = fractional_rec();

        let view = WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds);
        let summed: i32 = view
            .terms
            .iter()
            .map(|term| term.value.parse::<i32>().expect("a signed integer"))
            .sum();

        assert_eq!(
            format!("{summed:+}"),
            view.total,
            "the rows do not come to the total the panel prints above them"
        );
    }

    /// The second column balances for the same reason the first one does, and it
    /// is a second set of roundings, so it needs saying separately.
    #[test]
    fn the_delta_column_adds_up_the_same_way() {
        let ds = phrasing_fixture();
        let rec = fractional_rec();
        let other = ledger_rec_for(PHARAH);

        let view = WhyView::build(&rec, Some(&other), 0, 0.15, Rank::All, &ds);
        let summed: i32 = view
            .terms
            .iter()
            .map(|term| {
                term.delta
                    .as_deref()
                    .expect("every row carries the column")
                    .parse::<i32>()
                    .expect("a signed integer")
            })
            .sum();

        assert_eq!(
            format!("{summed:+}"),
            view.delta_total.expect("a column has a footer")
        );
    }

    /// And the footer is the difference of the two totals, which is what makes it
    /// the same number the row shows in swap mode.
    #[test]
    fn the_delta_column_totals_to_the_same_number_the_row_shows() {
        let ds = phrasing_fixture();
        let rec = fractional_rec();
        let other = ledger_rec_for(PHARAH);

        let view = WhyView::build(&rec, Some(&other), 0, 0.15, Rank::All, &ds);
        assert_eq!(
            view.delta_total.as_deref(),
            Some(format!("{:+}", points(rec.score - other.score)).as_str())
        );
        assert_eq!(view.against.as_deref(), Some("Pharah"));
    }

    /// The identity the whole column rests on: `delta_vs_locked` is
    /// `score - locked_score`, and the locked hero's own row carries that same
    /// score, so the panel's footer and the row's number are one subtraction.
    /// Asserted on the formatted strings, because that is where it is claimed.
    #[test]
    fn in_swap_mode_the_column_is_measured_against_the_hero_you_are_on() {
        let ds = phrasing_fixture();
        let locked = ledger_rec_for(PHARAH);
        let mut rec = fractional_rec();
        rec.delta_vs_locked = Some(rec.score - locked.score);

        let row = RecRow::build(&rec, &ds, true, 0, Rank::All);
        let view = WhyView::build(&rec, Some(&locked), 0, 0.15, Rank::All, &ds);

        assert_eq!(view.delta_total.as_deref(), Some(row.score.as_str()));
    }

    /// A hero is not compared with itself. A column of `+0`s would answer a
    /// question nobody asked, and the caption would name the row it is on.
    #[test]
    fn the_delta_column_is_absent_when_the_hero_being_read_is_the_one_it_would_compare_against() {
        let ds = phrasing_fixture();
        let rec = fractional_rec();

        let view = WhyView::build(&rec, Some(&rec.clone()), 0, 0.15, Rank::All, &ds);

        assert!(view.against.is_none());
        assert!(view.delta_total.is_none());
        assert!(view.terms.iter().all(|term| term.delta.is_none()));
    }

    /// What makes largest-remainder honest rather than merely tidy: no row is
    /// moved more than a point from what it would have said on its own.
    #[test]
    fn an_apportioned_value_is_never_more_than_a_point_from_its_own_rounding() {
        let values = [0.0351, -0.324, 0.15175, 0.3318, 0.0049, -0.0049, 0.5, -0.5];
        let total: f32 = values.iter().sum();

        let given = apportion(&values, total);
        assert_eq!(given.iter().sum::<i32>(), points(total));
        for (value, given) in values.iter().zip(&given) {
            assert!(
                (given - points(*value)).abs() <= 1,
                "{value} was rounded to {given} against its own {}",
                points(*value)
            );
        }
    }

    /// Largest remainder, and not merely *a* remainder. Handing the spare unit to
    /// the row that came *closest* to earning it is the whole rule; reversing it
    /// still balances the table, and still leaves every row within a point of its
    /// own rounding, so neither test above can see the difference. What it does
    /// is move a point onto the row that wanted it least.
    #[test]
    fn the_point_goes_to_the_row_that_was_closest_to_earning_it() {
        // Nine tenths of a point and one tenth, with a single unit to give away.
        // Their own roundings are already `+1` and `+0`, and apportioning should
        // agree with that rather than invert it.
        let given = apportion(&[0.009, 0.001], 0.010);

        assert_eq!(given, vec![1, 0]);
        assert_eq!(given, vec![points(0.009), points(0.001)]);
    }

    /// One rule for every number this column compares against another. The
    /// threat column has had `a_severity_that_rounds_to_nothing_never_prints_a
    /// _signed_zero` since it was written; the pick column used the idiom that
    /// test exists to forbid.
    #[test]
    fn one_rounding_rule_decides_every_number_the_pick_column_shows() {
        let ds = phrasing_fixture();
        let mut rec = scored(REINHARDT, -0.004, Vec::new());
        rec.delta_vs_locked = Some(-0.004);

        assert_eq!(RecRow::build(&rec, &ds, false, 0, Rank::All).score, "+0");
        assert_eq!(RecRow::build(&rec, &ds, true, 0, Rank::All).score, "+0");
        assert_eq!(
            WhyView::build(&rec, None, 0, 0.15, Rank::All, &ds).total,
            "+0"
        );
    }

    /// A ledger on a named hero, for the tests that need two of them.
    fn ledger_rec_for(hero: HeroId) -> Recommendation {
        let mut rec = ledger_rec();
        rec.hero = hero;
        rec
    }

    /// A ledger whose terms carry fractions of a point, so the rounding is the
    /// thing under test rather than an accident of the numbers.
    fn fractional_rec() -> Recommendation {
        let mut rec = scored(REINHARDT, 0.0, Vec::new());
        let live = [
            (TermKind::Base, 0.15, 0.234),
            (TermKind::Counter, 1.0, -0.324),
            (TermKind::Map, 0.25, 0.607),
            (TermKind::Personal, 0.60, 0.553),
        ];
        for (kind, weight, value) in live {
            let term = &mut rec.breakdown.terms[kind.index()];
            term.weight = weight;
            term.value = value;
        }
        rec.score = rec.breakdown.total();
        rec
    }

    /// A scored recommendation with a live ledger, for the panel tests. Side is
    /// left at nothing on purpose — the zero row is half of what is under test.
    fn ledger_rec() -> Recommendation {
        let mut rec = scored(REINHARDT, 0.0, Vec::new());
        let live = [
            (TermKind::Base, 0.15, 0.20),
            (TermKind::Counter, 1.0, -0.32),
            (TermKind::Map, 0.25, 0.60),
            (TermKind::Personal, 0.60, 0.55),
        ];
        for (kind, weight, value) in live {
            let term = &mut rec.breakdown.terms[kind.index()];
            term.weight = weight;
            term.value = value;
        }
        rec.breakdown.counter = Coverage {
            rated: 3,
            entered: 5,
        };
        rec.score = rec.breakdown.total();
        rec
    }

    /// Four reasons, one of them a quoted sentence, so the row's cut to three is
    /// visible against the panel's completeness.
    fn many_reasons_rec() -> Recommendation {
        let reason = |kind, contribution, text: &str| Reason {
            kind,
            contribution,
            text: text.to_owned(),
        };
        scored(
            REINHARDT,
            0.2,
            vec![
                reason(
                    ReasonKind::LosesToEnemy(PHARAH),
                    -0.4,
                    "Nothing Reinhardt does reaches the air.",
                ),
                reason(ReasonKind::BaseStrength, 0.3, ""),
                reason(ReasonKind::MapFit(MapId(0)), 0.2, ""),
                reason(ReasonKind::Comfort(55), 0.1, ""),
            ],
        )
    }

    /// Two dive and two poke, which ties the axes: rated, and committed to
    /// nothing. `Shape` has no constructor, so it is built the only way it can
    /// be — out of a board.
    fn mixed_shape(ds: &Dataset) -> overwatch_core::Shape {
        overwatch_core::shape_of(ds, &[REINHARDT, REINHARDT, PHARAH, PHARAH])
    }

    fn leading_shape(ds: &Dataset) -> overwatch_core::Shape {
        overwatch_core::shape_of(ds, &[REINHARDT, REINHARDT])
    }

    /// What a keyboard user is told when they land on the row. Until this slice
    /// they could not land on it: the strip's top three were the only heroes on
    /// the screen a tab could reach.
    #[test]
    fn the_pick_label_names_the_hero_the_number_and_the_place() {
        let mut row = row_at(0);
        row.name = "Winston".to_owned();
        row.score = "+34".to_owned();

        assert_eq!(pick_label(&row), "pick Winston, +34, 1st");
    }

    /// The tie is drawn as a hairline and said in a sentence above the list.
    /// Neither reaches somebody who is hearing one row at a time.
    #[test]
    fn a_tied_row_says_so_in_its_accessible_name() {
        let mut row = row_at(1);
        row.name = "Sigma".to_owned();
        row.score = "+33".to_owned();
        row.tied_with_top = true;

        assert!(pick_label(&row).ends_with(", too close to call"));
    }

    /// Pressing the current row re-locks the hero it already holds — `Seat::lock`
    /// assigns rather than toggles — so naming it as an action would promise
    /// something that does not happen.
    #[test]
    fn the_row_you_are_on_is_named_as_current_rather_than_as_a_pick() {
        let mut row = row_at(0);
        row.name = "Reinhardt".to_owned();
        row.score = "+0".to_owned();
        row.is_locked = true;

        let label = pick_label(&row);
        assert_eq!(label, "Reinhardt, the hero you are on, +0, 1st");
        assert!(!label.starts_with("pick"), "{label}");
    }

    /// The `swap` tag has no other carrier. Leaving it out would hand a
    /// screen-reader user strictly less than the person beside them.
    #[test]
    fn a_swap_candidate_says_so_in_its_name_because_the_tag_beside_it_does() {
        let mut row = row_at(2);
        row.name = "D.Va".to_owned();
        row.score = "+19".to_owned();
        row.worth_swapping = true;

        assert!(pick_label(&row).contains(", worth swapping"), "{row:?}");
    }

    /// Same for the star, which is a glyph and a `title` — and a `title` is not
    /// read at all by most of the software this is for.
    #[test]
    fn a_claimed_row_names_the_pool_in_its_accessible_name() {
        let mut row = row_at(3);
        row.name = "Ana".to_owned();
        row.score = "+12".to_owned();
        row.comfort = 55;

        assert!(pick_label(&row).ends_with(", one of yours"));
    }

    /// Every mark the row can carry, at once and in the order the eye meets them
    /// along it. The guard against a fifth state being added to the row and drawn
    /// without ever being spoken.
    #[test]
    fn every_clause_the_row_carries_reaches_its_name() {
        let mut row = row_at(4);
        row.name = "Orisa".to_owned();
        row.score = "+22".to_owned();
        row.worth_swapping = true;
        row.tied_with_top = true;
        row.comfort = 100;

        assert_eq!(
            pick_label(&row),
            "pick Orisa, +22, 5th, worth swapping, too close to call, one of yours"
        );
    }

    /// The list is cut to eight, so nothing here can reach a screen today. The
    /// rule is written because the cap is a `take(8)` somewhere else entirely.
    #[test]
    fn the_ordinal_reads_as_english_past_the_tenth_row() {
        let got: Vec<String> = (0..14).map(ordinal).collect();

        assert_eq!(
            got,
            [
                "1st", "2nd", "3rd", "4th", "5th", "6th", "7th", "8th", "9th", "10th", "11th",
                "12th", "13th", "14th"
            ]
        );
    }

    /// The hairline marks the boundary, so it belongs on the first row the scorer
    /// *could* separate — one place further down than the eye expects, because an
    /// inset top shadow draws along that row's own top edge.
    #[test]
    fn the_hairline_falls_below_the_last_tied_row_rather_than_on_it() {
        let rows: Vec<RecRow> = (0..4).map(row_at).collect();

        assert!(
            !rec_class(&rows[1], 2).contains("after-tie"),
            "the last tied row"
        );
        assert!(
            rec_class(&rows[2], 2).contains("after-tie"),
            "the first row after it"
        );
        assert!(
            !rec_class(&rows[3], 2).contains("after-tie"),
            "and nowhere else"
        );
    }

    /// One is not a tie, so there is no boundary to draw. Without the guard every
    /// list with a clear leader would get a rule under its top row.
    #[test]
    fn a_list_with_no_tie_draws_no_boundary_anywhere() {
        let rows: Vec<RecRow> = (0..4).map(row_at).collect();

        for tied in [0, 1] {
            for row in &rows {
                assert!(
                    !rec_class(row, tied).contains("after-tie"),
                    "row {} drew a boundary at a tie of {tied}",
                    row.place
                );
            }
        }
    }

    /// The boundary is one class among four and must not disturb the three that
    /// were already there — a pooled row still owns its left border.
    #[test]
    fn the_boundary_class_joins_the_row_states_rather_than_replacing_them() {
        let mut row = row_at(2);
        row.is_locked = true;
        row.worth_swapping = true;
        row.comfort = 55;

        let class = rec_class(&row, 2);
        for state in ["rec", "locked", "swap", "pooled", "after-tie"] {
            assert!(class.contains(state), "{state} missing from {class:?}");
        }
    }

    fn row_at(place: usize) -> RecRow {
        RecRow {
            hero: HeroId(place as u16),
            name: String::new(),
            icon: String::new(),
            score: String::new(),
            is_locked: false,
            worth_swapping: false,
            place,
            tied_with_top: false,
            comfort: 0,
            reasons: Vec::new(),
            coverage: None,
        }
    }

    /// The whole point: two rows at nearly the same number, one read against the
    /// whole enemy board and one against a fifth of it, used to look alike.
    #[test]
    fn a_candidate_read_against_two_of_five_enemies_says_so_on_its_row() {
        let note = coverage_note(Coverage {
            rated: 2,
            entered: 5,
        });

        assert_eq!(note.as_deref(), Some("read against 2 of their 5 picks"));
    }

    /// Silence is what a complete row looks like. `5 of 5` on every row is a
    /// fraction nobody reads twice, and the rows that stay quiet are then exactly
    /// the ones with nothing to admit.
    #[test]
    fn a_complete_read_says_nothing_because_silence_is_what_a_full_row_looks_like() {
        assert_eq!(
            coverage_note(Coverage {
                rated: 5,
                entered: 5
            }),
            None
        );
    }

    /// A fraction of zero is a worse sentence than the one the app already uses
    /// for this silence a panel over — and it is the same silence, so it gets the
    /// same words.
    #[test]
    fn a_candidate_no_source_has_rated_says_that_rather_than_printing_a_fraction() {
        let note = coverage_note(Coverage {
            rated: 0,
            entered: 4,
        })
        .expect("nothing rated is worth saying");

        assert_eq!(note, "no source has rated it against any of them");
        assert!(
            !note.contains("0 of"),
            "the fraction this arm exists to avoid"
        );
    }

    /// Nothing entered is not thin coverage, it is no question yet.
    #[test]
    fn an_empty_enemy_board_has_no_coverage_to_report() {
        assert_eq!(
            coverage_note(Coverage {
                rated: 0,
                entered: 0
            }),
            None
        );
    }

    /// The claim the doc makes by construction, pinned rather than left as a
    /// comment: reaching the fraction needs `0 < rated < entered`, so `entered` is
    /// at least two and the sentence never reads `their 1 picks`. It is why this
    /// carries none of the singular/plural split `p.threat-note` needs.
    #[test]
    fn the_coverage_line_never_describes_a_single_pick() {
        for entered in 0..=5usize {
            for rated in 0..=entered {
                let Some(note) = coverage_note(Coverage { rated, entered }) else {
                    continue;
                };
                assert!(
                    !note.contains("their 1 picks"),
                    "{rated} of {entered} produced {note:?}"
                );
            }
        }
    }

    /// Computable is not the same as wired. This is the only test that fails if
    /// the builder stops reading the ledger the scorer filled in.
    #[test]
    fn the_coverage_line_reaches_the_row_through_the_builder() {
        let ds = phrasing_fixture();
        let mut rec = scored(REINHARDT, 0.2, Vec::new());
        rec.breakdown.counter = Coverage {
            rated: 1,
            entered: 3,
        };

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        assert_eq!(
            row.coverage.as_deref(),
            Some("read against 1 of their 3 picks")
        );
    }

    /// The direct answer to "there are a lot of acceptable answers": when the
    /// scorer cannot separate the top rows, the panel stops explaining a scale and
    /// starts saying which rows it cannot tell apart.
    #[test]
    fn a_tied_top_replaces_the_scale_clause_rather_than_appending_to_it() {
        let note = score_note(None, Some("Winston"), Some(6), 3, 8, false, 0.15);

        assert_eq!(
            note,
            "top 3 too close to call \u{2014} take the one you are comfortable on"
        );
        assert!(
            !note.contains("weighted sum"),
            "there is one line, and what the number is measured in is the less \
             useful half of it once the answer is any of these three: {note}"
        );
        assert!(!note.contains("leads the next"), "{note}");
    }

    /// "top 8" implies a ninth row the reader cannot see. When the tie is the
    /// whole visible list, the sentence has to say so instead.
    #[test]
    fn the_whole_visible_list_being_tied_says_so_rather_than_naming_a_top_n() {
        let all = score_note(None, Some("Winston"), Some(0), 8, 8, false, 0.15);

        assert!(all.starts_with("all 8 here are too close to call"), "{all}");
        assert!(
            !all.contains("top 8"),
            "there is no ninth row for a top eight to be the top of: {all}"
        );
    }

    /// The count comes from the rows the caller is about to draw, so the note can
    /// never name a hero that is not on screen — but the arm still has to exist,
    /// because the two wordings turn on exactly that comparison.
    #[test]
    fn the_tie_note_never_claims_more_heroes_than_the_list_shows() {
        let three_of_three = score_note(None, Some("Winston"), Some(0), 3, 3, false, 0.15);
        assert!(three_of_three.starts_with("all 3 here"), "{three_of_three}");

        let three_of_eight = score_note(None, Some("Winston"), Some(0), 3, 8, false, 0.15);
        assert!(three_of_eight.starts_with("top 3"), "{three_of_eight}");
    }

    /// One is not a tie. The top hero is inside the band of itself by definition,
    /// so a count of one has to read as the clear leader it is.
    #[test]
    fn a_top_that_is_tied_with_nothing_still_reports_its_margin() {
        let note = score_note(None, Some("Winston"), Some(6), 1, 8, false, 0.15);

        assert_eq!(
            note,
            "weighted sum, not a percentage \u{2014} Winston leads the next by 6"
        );
    }

    /// Swap mode has one line and the threshold statement is what it is for. The
    /// hairline still draws — that is placement rather than a claim — but the
    /// sentence stays the one about whether anything is worth the swap.
    #[test]
    fn swap_mode_keeps_the_threshold_statement_when_the_top_is_tied() {
        let note = score_note(
            Some("Reinhardt"),
            Some("Winston"),
            Some(0),
            4,
            8,
            true,
            0.15,
        );

        assert!(
            note.starts_with("the column is the gain over Reinhardt"),
            "{note}"
        );
        assert!(!note.contains("too close to call"), "{note}");
    }

    /// The row you are on used to be the one exception in the column: a delta
    /// everywhere else and an absolute score here, under a heading asking whether
    /// to swap. Its own delta is zero by construction, so this is what the column
    /// meaning one thing looks like.
    #[test]
    fn in_swap_mode_the_row_you_are_on_reads_zero_rather_than_its_own_score() {
        let ds = phrasing_fixture();
        let mut rec = scored(REINHARDT, 0.41, Vec::new());
        rec.is_locked = true;
        rec.delta_vs_locked = Some(0.0);

        assert_eq!(RecRow::build(&rec, &ds, true, 0, Rank::All).score, "+0");
        // And out of swap mode the same row is still its own score, because there
        // is nothing for it to be a gain over.
        assert_eq!(RecRow::build(&rec, &ds, false, 0, Rank::All).score, "+41");
    }

    fn scored(hero: HeroId, score: f32, reasons: Vec<Reason>) -> Recommendation {
        Recommendation {
            hero,
            score,
            delta_vs_locked: None,
            worth_swapping: false,
            is_locked: false,
            breakdown: Breakdown::default(),
            place: 0,
            tied_with_top: false,
            reasons,
        }
    }

    /// The whole chain, once, through the real builder: a negative base term on a
    /// row reaches the screen as the figure and as the `bad` tint that draws the
    /// minus.
    #[test]
    fn a_negative_term_reaches_the_row_as_the_wording_its_sign_allows() {
        let ds = phrasing_fixture();
        let rec = scored(
            PHARAH,
            -0.2,
            vec![Reason {
                kind: ReasonKind::BaseStrength,
                contribution: -0.2,
                text: String::new(),
            }],
        );

        let row = RecRow::build(&rec, &ds, false, 0, Rank::All);
        assert!(!row.reasons[0].positive, "the sign is what draws the minus");
        assert_eq!(row.reasons[0].text, "45.6% win rate");
    }
}
