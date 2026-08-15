//! Presentational components.
//!
//! These take plain values rather than reaching into shared state, so what each
//! panel depends on is visible in its signature and the whole screen stays
//! re-renderable from one recomputed frame.

use dioxus::prelude::*;
use overwatch_core::{
    Dataset, Format, HeroId, MapId, Queue, ReasonKind, Recommendation, Role, Side, TeamSize,
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
/// carries the mode switch, the map, the side toggle, the sync light, the
/// ingest date and a reset, and a screen whose entire argument is density
/// cannot spend a hundred pixels of it naming the app you already opened. The
/// wordmark does its work on the tab, the install prompt and the link preview.
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

/// The one place the app says who it is and whose artwork it is borrowing.
///
/// The portraits and map shots in this bundle are Blizzard's. Serving them
/// across a LAN and serving them to the open internet are different postures,
/// and the second one should say so out loud rather than leave it to be
/// inferred from a licence file nobody opens.
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
            span {
                "not affiliated with or endorsed by Blizzard Entertainment. \
                 Overwatch, hero and map artwork are Blizzard's."
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
}

/// One hero on one of the roster boards.
///
/// `selected` and `disabled` are computed per board, not per hero: the same
/// hero is picked on the enemy board, unpicked on the ally board, and in your
/// pool all at once.
#[derive(Debug, Clone, PartialEq)]
pub struct HeroTile {
    pub hero: HeroId,
    pub name: String,
    pub icon: String,
    pub selected: bool,
    /// The team is full and this hero is not on it, so a click cannot land.
    pub disabled: bool,
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
    pub tiles: Vec<HeroTile>,
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
    on_query: EventHandler<String>,
    on_submit: EventHandler<()>,
    on_focus: EventHandler<bool>,
    on_pick: EventHandler<MapId>,
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
                    button {
                        key: "{tile.map.0}",
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
    on_toggle: EventHandler<HeroId>,
    on_reset: EventHandler<()>,
) -> Element {
    rsx! {
        section { class: "board board-{side}",
            div { class: "board-head",
                h3 { class: "board-title {side}", "{title}" }
                ResetButton { confirm: reset_confirm, on_reset }
            }
            for row in rows.iter() {
                div { key: "{row.label}", class: "board-row",
                    span { class: format!("board-role {}", role_class(row.role)),
                        "{row.label}"
                        // A zero here is the answer to "why can I not click
                        // this", which a disabled tile alone does not give.
                        if let Some(free) = row.capacity {
                            span { class: "board-free", "{free}" }
                        }
                    }
                    div { class: "tiles",
                        for tile in row.tiles.iter() {
                            button {
                                key: "{tile.hero.0}",
                                class: format!(
                                    "tile{}{}",
                                    if tile.selected { " selected" } else { "" },
                                    if tile.disabled { " disabled" } else { "" },
                                ),
                                style: art(&tile.icon),
                                aria_label: "{tile.name}",
                                "data-name": "{tile.name}",
                                // Disabled rather than hidden: a full team must
                                // still show the whole roster, or the positions
                                // everything else relies on would move.
                                disabled: tile.disabled,
                                onclick: {
                                    let hero = tile.hero;
                                    move |_| on_toggle.call(hero)
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
/// No keyboard shortcut, deliberately. The four that exist are all things done
/// repeatedly inside a draft; this changes when you change queue, once an
/// evening. A fifth chord would dilute the four that matter and lengthen a hint
/// line already at capacity.
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

/// Attack or defend, for the payload modes that have both.
///
/// Rendered only where the question has an answer — Push, Control, Flashpoint
/// and Clash start both teams in the same posture, so the caller passes `None`
/// and this draws nothing rather than a disabled control nobody can use.
#[component]
pub fn SideToggle(side: Option<Side>, on_side: EventHandler<Option<Side>>) -> Element {
    rsx! {
        div { class: "sides",
            for option in Side::BOTH {
                button {
                    key: "{option.as_str()}",
                    class: if side == Some(option) { "side active" } else { "side" },
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
    /// `None` on a symmetric mode, or when no map is picked yet.
    sides_apply: bool,
    side: Option<Side>,
    /// One per playable role, in switch order.
    modes: Vec<ModeChip>,
    generated: String,
    sync_status: String,
    on_role: EventHandler<Role>,
    on_format: EventHandler<Format>,
    on_side: EventHandler<Option<Side>>,
    on_reset_all: EventHandler<()>,
) -> Element {
    let sync_class = format!("sync sync-{}", sync_status.replace(' ', "-"));

    rsx! {
        header { class: "header",
            a {
                class: "brand",
                href: "/",
                "aria-label": "MinMax — minmax.watch",
                title: "minmax.watch",
                {brand_mark()}
            }
            ModeSwitch { role, modes, on_role }
            div { class: "context",
                // First, because it is the widest-scope fact about the match —
                // queue, then map, then side — and because unlike the side
                // toggle it never disappears, so the cluster keeps a stable
                // left edge as picks land.
                FormatSwitch { format, on_format }
                match map {
                    Some(map) => rsx! {
                        span { class: "map-thumb", style: art(&map.icon) }
                        span { class: "map", "{map.name}" }
                    },
                    None => rsx! { span { class: "map unset", "no map" } },
                }
                if sides_apply {
                    SideToggle { side, on_side }
                }
                // The pool count used to sit here, adrift between the map and
                // the sync light. It lives on the mode segment it describes now,
                // where it is also legible for the modes you are not in.
                // Whether the other screen is actually attached. Scoring is
                // local either way, so "offline" costs sync, not function.
                span { class: "{sync_class}", "{sync_status}" }
                // Counter data ages with every patch; showing when it was last
                // pulled is the difference between trusting it and trusting it
                // blindly.
                span { class: "generated", title: "counter data last ingested", "{generated}" }
                // Everything, map and side included — the "new match" reset, as
                // opposed to Esc, which keeps the map for the next round.
                ResetButton { confirm: true, on_reset: on_reset_all }
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
    /// Which of your heroes this hurts most. `None` when you are locked in, in
    /// which case the answer is "you" and saying so would be noise.
    pub worst: Option<String>,
    /// The scraped sentence for that pair, where one exists.
    pub text: String,
    /// One of yours. The same read-out as the pick list's star, and it means
    /// something sharper here: a ban costs you the hero too.
    pub in_pool: bool,
}

/// Who to deny the enemy, before anyone has picked.
///
/// The counterpart to [`Recommendations`], and the one panel that is about the
/// phase *before* the draft. `subject` is spelled out in the heading rather than
/// left to be inferred, because the number in each row means something different
/// depending on it — your locked hero's own matchup, or an average over every
/// hero you might still end up on.
#[component]
pub fn BanPanel(subject: String, items: Vec<BanRow>) -> Element {
    rsx! {
        section { class: "panel bans",
            div { class: "panel-head",
                h2 { "ban" }
                span { class: "subject", "{subject}" }
            }
            if items.is_empty() {
                p { class: "empty", "nothing here beats you — no ban worth spending" }
            }
            for (index, ban) in items.iter().enumerate() {
                div {
                    key: "{ban.hero.0}",
                    class: if ban.in_pool { "ban pooled" } else { "ban" },
                    span { class: "rank", "{index + 1}" }
                    span { class: "rec-portrait", style: art(&ban.icon) }
                    div { class: "rec-body",
                        div { class: "rec-head",
                            span { class: "rec-name", "{ban.name}" }
                            span { class: "score", "{ban.score}" }
                            // A ban takes the hero off the table for everyone,
                            // so one of yours landing here is a real cost and
                            // not just a highlight.
                            if ban.in_pool {
                                span { class: "star on", title: "one of yours — banning it costs you the pick too", aria_label: "in your pool", "★" }
                            }
                        }
                        // Two separate claims, so they are two lines: which of
                        // your heroes takes the worst of it, and whatever the
                        // sources actually say about that pair.
                        if let Some(worst) = &ban.worst {
                            p { class: "ban-worst", "hardest on {worst}" }
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
    /// `(is_positive, text)` per reason.
    pub reasons: Vec<(bool, String)>,
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
                        ReasonKind::BaseStrength => "strong in the current patch".to_owned(),
                        ReasonKind::Comfort => "one of your comfort picks".to_owned(),
                    }
                } else {
                    reason.text.clone()
                };
                (reason.contribution >= 0.0, text)
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
    on_lock: EventHandler<HeroId>,
) -> Element {
    rsx! {
        section { class: "panel recommendations",
            div { class: "panel-head",
                h2 {
                    if swap_mode { "should you swap?" } else { "pick" }
                }
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
                            for (index, (positive, text)) in rec.reasons.iter().enumerate() {
                                li {
                                    key: "{index}",
                                    class: if *positive { "reason good" } else { "reason bad" },
                                    "{text}"
                                }
                            }
                        }
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
                        match (row.icon, row.hero) {
                            (Some(icon), Some(hero)) => rsx! {
                                span { class: "roster-hero",
                                    span { class: "roster-portrait", style: art(&icon) }
                                    "{hero}"
                                }
                            },
                            // An empty slot is information: they are still
                            // choosing, which is worth seeing during a draft.
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
