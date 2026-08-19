//! Presentational components.
//!
//! These take plain values rather than reaching into shared state, so what each
//! panel depends on is visible in its signature and the whole screen stays
//! re-renderable from one recomputed frame.

use dioxus::prelude::*;
use overwatch_core::{
    Dataset, Format, HeroId, MapId, Queue, Rank, ReasonKind, Recommendation, Role, Side, TeamSize,
    Threat,
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
    /// How many heroes of this role you have marked as yours. A zero is honest
    /// now: the pool highlights rather than restricts, so an empty one costs
    /// you nothing but the highlight.
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
            a {
                href: "https://github.com/MaikBuse/minmax-watch",
                rel: "noopener",
                "source"
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
}

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
                                class: format!("tile{}", tile.state.class()),
                                style: art(&tile.icon),
                                aria_label: "{tile.name}",
                                // Whose it is, where that is the reason it
                                // cannot be clicked.
                                "data-name": match &tile.owner {
                                    Some(owner) => format!("{} · {}", tile.name, owner),
                                    None => tile.name.clone(),
                                },
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
                            p { class: "ban-text", "{ban.text}" }
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
            name: dataset
                .hero(threat.enemy)
                .map(|h| h.name.clone())
                .unwrap_or_else(|_| "?".to_owned()),
            icon: dataset
                .hero(threat.enemy)
                .map(|h| crate::icons::hero(&h.key))
                .unwrap_or_default(),
            score: format!("{points:+}"),
            favourable: points > 0,
            even: points == 0,
            text,
            disputed: threat.disputed,
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
                "three quarters of matchups are one site's rating \u{2014} \u{201c}disputed\u{201d} means the second source disagrees"
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
                                    title: "the two sources disagree about this matchup",
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
                            p { class: "threat-text", "{threat.text}" }
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
    /// One of yours, from the pool board. A highlight and nothing else — the
    /// hero is ranked and the score untouched either way, because the comfort
    /// overrides are already the lever for "I like this hero" and two levers for
    /// one job would fight.
    pub in_pool: bool,
    pub reasons: Vec<ReasonLine>,
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
}

impl RecRow {
    /// Resolves one scored recommendation into display form.
    pub fn build(rec: &Recommendation, dataset: &Dataset, swap_mode: bool, in_pool: bool) -> Self {
        let name_of = |hero: HeroId| {
            dataset
                .hero(hero)
                .map(|h| h.name.clone())
                .unwrap_or_else(|_| "?".to_owned())
        };
        let map_name_of = |map: MapId| {
            dataset
                .map(map)
                .map(|m| m.name.clone())
                .unwrap_or_else(|_| "?".to_owned())
        };

        // Once you are locked in, the absolute score is noise: the only
        // question is whether a swap gains you anything.
        let score = if swap_mode && !rec.is_locked {
            match rec.delta_vs_locked {
                Some(delta) => format!("{:+.0}", delta * 100.0),
                None => String::new(),
            }
        } else {
            format!("{:+.0}", rec.score * 100.0)
        };

        let reasons = rec
            .reasons
            .iter()
            .map(|reason| {
                let text = if reason.text.is_empty() {
                    // Only ~40% of matchups carry a scraped sentence; the rest
                    // get phrasing generated from the reason kind, because a
                    // bare number explains nothing.
                    match reason.kind {
                        ReasonKind::BeatsEnemy(hero) => format!("strong into {}", name_of(hero)),
                        ReasonKind::LosesToEnemy(hero) => {
                            format!("struggles against {}", name_of(hero))
                        }
                        ReasonKind::PairsWithAlly(hero) => {
                            format!("pairs well with {}", name_of(hero))
                        }
                        ReasonKind::MapFit(map) => {
                            format!("performs well on {}", map_name_of(map))
                        }
                        ReasonKind::SideFit(side) => {
                            format!("suits {}", side.as_str())
                        }
                        // Their shape, not this hero's: the portrait beside the
                        // line already says what the candidate is.
                        ReasonKind::CountersShape(theirs) => {
                            format!("answers their {}", theirs.label())
                        }
                        ReasonKind::LosesToShape(theirs) => {
                            format!("walks into their {}", theirs.label())
                        }
                        ReasonKind::BaseStrength => "strong in the current patch".to_owned(),
                        // The only line on the whole screen that names a rung.
                        // Never produced at `Rank::All`, where the term is zero
                        // and a zero term is never explained — so an unset rank
                        // leaves the panel exactly as it has always read.
                        //
                        // Phrased like `SideFit` above, and for the same reason:
                        // the leading sign has to carry it. This term goes
                        // negative as often as positive — half the roster is
                        // worse at any given rung than across the ladder — and a
                        // comparative like "stronger at master" reads as a
                        // contradiction the moment it is prefixed with a minus.
                        //
                        // "right now" is what keeps it about the patch. Without
                        // it the line reads as a claim about matchups at that
                        // rung, which nothing behind this feature measured.
                        ReasonKind::RankFit(rank) => {
                            format!("suits {} right now", rank.label())
                        }
                        ReasonKind::Comfort => "one of your comfort picks".to_owned(),
                    }
                } else {
                    reason.text.clone()
                };
                // Only the counter terms read the matchup matrix, so only they
                // can be in dispute. Asked of the pair rather than of the row
                // the scorer happened to average, which is what
                // `sources_disagree` is for.
                let disputed = match reason.kind {
                    ReasonKind::BeatsEnemy(enemy) | ReasonKind::LosesToEnemy(enemy) => {
                        dataset.sources_disagree(rec.hero, enemy)
                    }
                    _ => false,
                };
                ReasonLine {
                    positive: reason.contribution >= 0.0,
                    text,
                    disputed,
                }
            })
            .collect();

        Self {
            hero: rec.hero,
            name: name_of(rec.hero),
            icon: dataset
                .hero(rec.hero)
                .map(|h| crate::icons::hero(&h.key))
                .unwrap_or_default(),
            score,
            is_locked: rec.is_locked,
            worth_swapping: rec.worth_swapping,
            in_pool,
            reasons,
        }
    }
}

#[component]
pub fn Recommendations(
    items: Vec<RecRow>,
    swap_mode: bool,
    /// The rung patch strength is read on. The control lives here rather than in
    /// the header because this is the list it reorders: selecting one changes the
    /// top row for a fifth to well over a quarter of drafts, depending on the
    /// rung, and a chip among the header's match facts claimed none of that.
    rank: Rank,
    rank_open: bool,
    on_lock: EventHandler<HeroId>,
    on_rank: EventHandler<Rank>,
    on_rank_open: EventHandler<()>,
) -> Element {
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
            if items.is_empty() {
                p { class: "empty", "every hero in this role is already on your team" }
            }
            for (index, rec) in items.iter().enumerate() {
                div {
                    key: "{rec.hero.0}",
                    class: format!(
                        "rec{}{}{}",
                        if rec.is_locked { " locked" } else { "" },
                        if rec.worth_swapping { " swap" } else { "" },
                        if rec.in_pool { " pooled" } else { "" },
                    ),
                    onclick: {
                        let hero = rec.hero;
                        move |_| on_lock.call(hero)
                    },
                    span { class: "rank", "{index + 1}" }
                    // The portrait spans the name and its reasons, so a row
                    // reads as one block rather than two stacked lines.
                    span { class: "rec-portrait", style: art(&rec.icon) }
                    div { class: "rec-body",
                        div { class: "rec-head",
                            span { class: "rec-name", "{rec.name}" }
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
                            if rec.in_pool {
                                span { class: "star on", title: "one of yours", aria_label: "in your pool", "★" }
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
                                            title: "the two sources disagree about this matchup",
                                            "disputed"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
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
                for (index, rec) in items.iter().take(3).enumerate() {
                    button {
                        key: "{rec.hero.0}",
                        class: format!(
                            "strip-pick{}{}{}",
                            if rec.is_locked { " locked" } else { "" },
                            if rec.worth_swapping { " swap" } else { "" },
                            if rec.in_pool { " pooled" } else { "" },
                        ),
                        // The rank is the reading order here rather than a
                        // column of its own — there is no room for one, and
                        // three items left to right is already an order.
                        aria_label: "{index + 1}. {rec.name}, {rec.score}",
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

/// The one part of this module that can be wrong in a way a test can catch.
///
/// Everything else here is markup, but [`ThreatRow::build`] inverts a sign, and
/// a silently un-inverted threat column would read as the exact opposite of what
/// it means while looking entirely plausible.
#[cfg(test)]
mod tests {
    use super::*;
    use overwatch_core::{DatasetParts, Hero, Matrix, Reason};

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
        let heroes = vec![
            hero("reinhardt", "Reinhardt", Role::Tank),
            hero("pharah", "Pharah", Role::Damage),
        ];
        let n = heroes.len();

        Dataset::new(DatasetParts {
            heroes,
            maps: Vec::new(),
            matchups,
            synergy: Matrix::unrated(n),
            map_affinity: Vec::new(),
            base_strength: vec![0; n],
            rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
            win_rate: vec![None; n],
            side_lean: vec![0; n],
            shape: vec![[0; 3]; n],
            reasons: vec![String::new(); n * n],
            disputed,
            generated: String::new(),
            patch: String::new(),
        })
        .expect("a two-hero dataset is valid")
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

        let rec = Recommendation {
            hero: REINHARDT,
            score: -0.3,
            delta_vs_locked: None,
            worth_swapping: false,
            is_locked: false,
            reasons: vec![
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
        };

        let row = RecRow::build(&rec, &ds, false, false);
        assert!(row.reasons[0].disputed, "the counter line reads the matrix");
        assert!(!row.reasons[0].positive);
        assert!(
            !row.reasons[1].disputed,
            "patch strength has one source and nothing to disagree with"
        );
    }
}
