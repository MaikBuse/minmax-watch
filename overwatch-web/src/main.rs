//! The draft screen.
//!
//! Everything here is built around one constraint: hero select is seconds long,
//! so nothing waits on a network round trip and nothing is more than one click
//! away. The dataset is compiled in, scoring runs locally on every pick, and the
//! sync socket (when connected) only carries the session state between the
//! people drafting — it is never in the path between a click and seeing an
//! answer.
//!
//! The draft this screen scores is *derived*, not stored. In a session the
//! shared board and the roster are the state; `SessionState::draft_for` turns
//! them into the [`Draft`] the scorer takes, so that everyone's teammates show
//! up as their allies without anybody having to type them in. Alone, the same
//! path runs over a one-seat session and produces exactly what the app produced
//! before sessions existed.
//!
//! There are no modes. Every selectable thing is on screen at once, on a board
//! that says what it is: your pool, the maps, the ally roster, the enemy roster.
//! Which board you click *is* the answer to "which team", so there is nothing to
//! switch between and nothing appears or disappears as picks land — a portrait
//! stays where your hand learned it.

mod board;
mod icons;
mod keys;
mod matchlog;
mod profile;
mod session;
mod sync;
mod ui;

use std::rc::Rc;

use dioxus::prelude::*;
use overwatch_core::{
    ban_recommendations, recommend, search_maps, shape_of, threats, Archetype, BanBoard,
    BanSubject, Board, Capacity, ComfortStep, Dataset, Draft, Format, HeroId, MapId, Rank,
    Recommendation, Role, Seat, SessionState, Side, Threat,
};

use crate::profile::Profile;
use crate::session::Membership;
use crate::ui::{
    BanRow, BoardRow, HeroChip, HeroTile, MapChip, MapTile, ModeChip, RosterRow, ShapeChip,
    TileState,
};

/// The stylesheet, put in the head by `dx` at build time rather than by the app.
///
/// `with_static_head` is the load-bearing part. Rendering `document::Stylesheet`
/// — which is what this used to do — inserts the `<link>` from an effect that
/// only runs after the first render, so the CSS could not even begin
/// downloading until the whole wasm bundle had arrived and run once. The
/// browser filled that window with its own defaults: a white page, and then the
/// draft screen as unstyled black-on-white text. Written into `index.html`
/// instead, the stylesheet is discovered in the initial HTML parse and blocks
/// the first paint, so there is no unstyled window at all.
///
/// Never read. The `asset!()` call still has to exist: it is what registers the
/// file with the bundler and pins its content-hashed name.
#[allow(dead_code)]
static CSS: Asset = asset!(
    "/assets/style.css",
    AssetOptions::css().with_static_head(true)
);

fn main() {
    dioxus::launch(App);
}

/// How many of somebody's pool the roster draws before it counts the rest.
///
/// The roster is a row of pills that wraps, so a strip that grew with the pool
/// behind it would move everybody else's row. Six is about what fits beside a
/// name and a role without the pill outgrowing the one holding a locked hero.
const ROSTER_POOL_SHOWN: usize = 6;

/// Everything the screen needs, recomputed on every pick.
///
/// For a 53-hero roster this is a few thousand `i8` lookups — microseconds — so
/// there is no memoisation to get stale or wrong.
struct Frame {
    recommendations: Vec<Recommendation>,
    bans: BanBoard,
    /// The enemy team ranked by how hard each one is beating your locked hero.
    ///
    /// Empty until you lock in, which is a state the panel renders rather than
    /// an absence worth encoding: `draft.locked` is in scope at the render site,
    /// so an `Option` here would be the same fact written twice.
    threats: Vec<Threat>,
}

/// The roster with this client's own seat as *it* believes it to be.
///
/// The server's copy of your seat is always at least one round trip behind your
/// last click, and the derived draft is what the screen renders. Taking the
/// local copy is what keeps locking a hero instant: the alternative is a pick
/// that appears when the network says so, which is precisely the latency this
/// app exists to avoid.
///
/// Alone — no session, so an empty roster — this yields a one-seat session, and
/// `draft_for` over it reproduces exactly the solo behaviour.
fn roster_including_me(seats: &[Seat], me: &Seat) -> Vec<Seat> {
    let mut roster: Vec<Seat> = seats
        .iter()
        .filter(|seat| seat.id != me.id)
        .cloned()
        .collect();
    roster.push(me.clone());
    roster
}

/// Resolves a hero into the form the panels render: name and portrait together.
///
/// Every display site needs both, and both come from the same lookup, so they
/// are produced together rather than threaded separately.
fn hero_chip(dataset: &Dataset, hero: HeroId) -> HeroChip {
    match dataset.hero(hero) {
        Ok(entry) => HeroChip {
            hero,
            name: entry.name.clone(),
            icon: icons::hero(&entry.key),
        },
        // An id with no hero behind it is a bug, not a draft state. Render it
        // visibly rather than silently dropping a pick from the board.
        Err(_) => HeroChip {
            hero,
            name: "?".to_owned(),
            icon: String::new(),
        },
    }
}

/// Resolves a team's picks into the word its board header shows.
///
/// The hint is the half a two-syllable chip cannot carry: what the word means,
/// and — while the team is still filling — that the read is provisional. Both
/// belong in a title rather than on screen, because the header is already
/// carrying a name and a reset.
fn shape_chip(dataset: &Dataset, picks: &[HeroId]) -> ShapeChip {
    let shape = shape_of(dataset, picks);
    let confident = shape.confident();

    let hint = match shape.leading() {
        Some(axis) => {
            let what = match axis {
                Archetype::Dive => "close the distance and isolate a target",
                Archetype::Poke => "hold an angle and chip from range",
                Archetype::Brawl => "hold ground and win the close fight",
            };
            if confident {
                format!("{}: {what}", axis.label())
            } else {
                format!("{}: {what} — still filling", axis.label())
            }
        }
        None if shape.is_rated() => "no one shape yet — the picks pull both ways".to_owned(),
        None => "nothing picked yet".to_owned(),
    };

    ShapeChip {
        label: shape.label().to_owned(),
        confident,
        hint,
    }
}

fn map_chip(dataset: &Dataset, map: overwatch_core::MapId) -> Option<MapChip> {
    dataset.map(map).ok().map(|entry| MapChip {
        map,
        name: entry.name.clone(),
        icon: icons::map(&entry.key),
    })
}

/// How far the top pick leads the next, in the points the rows actually print.
///
/// **Rounded before the subtraction, not after**, and that is the whole reason
/// this is a function rather than a line at the call site. The note sits directly
/// above the two figures it is subtracting, so it has to be *their* difference:
/// two scores of 0.414 and 0.408 both print `+41`, and `round((a - b) * 100)`
/// would put "leads the next by 1" under them. A note that cannot subtract the
/// two numbers beneath it undoes the thing it is there to do.
///
/// The cost is that a lead of zero is ordinary rather than rare, which
/// [`ui::score_note`] words rather than printing as a margin of nothing.
///
/// `None` when there is no second row to lead — one candidate, or none.
fn printed_lead(recs: &[Recommendation]) -> Option<i32> {
    let printed = |rec: &Recommendation| (rec.score * 100.0).round() as i32;
    match recs {
        [top, next, ..] => Some(printed(top) - printed(next)),
        _ => None,
    }
}

/// How many of the shown rows the scorer could not separate — and none at all
/// while the enemy board is empty.
///
/// **That second half is the whole reason this is a function.** With nothing
/// entered, only patch strength, the rung and your own comfort are live, so the
/// top eight are flat by construction: median `best - eighth` is 0.114 against a
/// band of 0.15, and the note would read "all 8 here are too close to call" on
/// **74% of opening screens**. That is a true sentence about the dataset and a
/// useless one about your draft, offered before you have told the app anything —
/// and it is not a number problem, because even a band of 0.05 ties something on
/// the empty board 76% of the time.
///
/// So the tie is reported once there is a draft for it to be about. The rows keep
/// their `tied_with_top` either way: the hairline is a placement rather than a
/// claim, and `ui::score_note` reads a count of zero as no tie.
fn tie_count(rows: &[ui::RecRow], enemies: usize) -> usize {
    if enemies == 0 {
        return 0;
    }
    rows.iter().filter(|row| row.tied_with_top).count()
}

/// What the ban list is defending, named rather than left to be inferred: the
/// number on every row means a different thing in each case, and on the patch
/// rung it is not about this team at all.
///
/// `rank` reaches exactly one arm *of the caption*, and that is the whole point of
/// it being here. `Patch` is the one subject `ban_recommendations` scores with
/// `base_strength_at(rank, ..)` — every other arm is threat, where the rung does
/// not enter the matchup and naming it would be a claim the score does not make.
///
/// The rung does now reach every arm's *ordering*, through the prevalence
/// discount, which is why the other arms still say nothing about it: a prior on
/// who turns up is not a reading of the matchup the row is about, and a caption
/// that named the rung would suggest it was.
fn ban_subject(subject: &BanSubject, rank: Rank, role: Role) -> String {
    match subject {
        // The rung that sorted the list, said in the caption. Without it this
        // reads as a claim about the whole ladder while ranking on one division
        // of it — and the row underneath already qualifies its win rate as the
        // ladder's, so the two lines would contradict each other.
        BanSubject::Patch if rank == Rank::All => "strongest right now".to_owned(),
        BanSubject::Patch => format!("strongest at {} right now", rank.label()),
        BanSubject::One {
            is_me: true,
            locked: true,
            ..
        } => "vs your pick".to_owned(),
        BanSubject::One {
            who, locked: true, ..
        } => format!("vs {who}'s pick"),
        BanSubject::One {
            is_me: true,
            heroes,
            ..
        } => format!("vs your pool · {heroes} {}", role.label()),
        BanSubject::One { who, heroes, .. } => format!("vs {who}'s pool · {heroes}"),
        BanSubject::Team { known, locked } if *locked > 0 => {
            format!("vs your team · {known} known, {locked} in")
        }
        BanSubject::Team { known, .. } => format!("vs your team · {known} known"),
    }
}

#[component]
fn App() -> Element {
    // The dataset is immutable and shared by every panel.
    let dataset = use_hook(|| match overwatch_data::load() {
        Ok(dataset) => Rc::new(dataset),
        // A dataset that will not parse is a build-time mistake, not something
        // to paper over at runtime — say so plainly instead of showing an empty
        // hero list that looks like a bug in the game.
        Err(err) => panic!("the compiled-in dataset is invalid: {err}"),
    });

    let mut profile = use_signal(|| Profile::load(&dataset));
    // With no text input on the screen there is no natural focus sink, so the
    // shortcuts hang off the root element and this is what keeps them reachable
    // after a stray click.
    let mut root_ref = use_signal(|| None::<Rc<MountedData>>);
    // `Some(won)` after a result is recorded, so the keystroke has visible
    // confirmation. Cleared by the next pick.
    let mut logged = use_signal(|| None::<bool>);
    // The map filter, and whether it currently holds focus. The shortcuts hang
    // off the root element, so both root handlers have to stand down while it
    // does or typing a map name fires them.
    let mut map_query = use_signal(String::new);
    let mut search_focused = use_signal(|| false);

    // --- session ----------------------------------------------------------
    //
    // The state is split by who owns it. `board` is shared with everyone and
    // last-writer-wins; `me` is this client's own seat and nobody else ever
    // writes it; `seats` is the roster as the server last described it. The
    // draft the scorer sees is derived from all three, so nothing here is
    // stored twice.
    let my_id = use_hook(sync::client_id);
    // Seeded from the profile, which remembers the format you were last in. A
    // session you join overwrites it with the room's, the same way the map
    // arrives.
    let mut board = use_signal(|| Board {
        format: profile.peek().format,
        ..Board::new()
    });
    let mut seats = use_signal(Vec::<Seat>::new);
    let mut me = use_signal(|| {
        let profile = profile.peek();
        Seat {
            id: my_id.clone(),
            name: profile.name.clone(),
            role: profile.role,
            locked: None,
            pool: profile.pool(&dataset, profile.role).iter().collect(),
            rank: profile.rank,
            connected: true,
        }
    });

    // Mirrors of what the server last told us, so the outbound effects can tell
    // a local edit from the echo of a remote one.
    let synced_board = use_signal(Board::new);
    let synced_seat = use_signal(Seat::default);

    let mut status = use_signal(|| sync::Status::Solo);
    let mut membership = use_signal(Membership::default);
    // Held in a signal rather than a hook so that joining and leaving can swap
    // the socket without a page reload.
    let mut connection = use_signal(|| sync::Connection::idle(my_id.clone()));
    let mut qr_open = use_signal(|| false);
    // Whether the rank sheet is showing. A disclosure, not draft state: it never
    // survives a pick and nothing but a click on the chip opens it.
    let mut rank_open = use_signal(|| false);
    let mut join_entry = use_signal(String::new);

    let sinks = sync::Sinks {
        board,
        seats,
        synced_board,
        status,
    };

    // Joining, in one place because three things reach for it: the deep link,
    // the code box, and the create button.
    let join = {
        let ds = dataset.clone();
        use_callback(move |code: String| {
            let name = profile.peek().name.clone();
            // Moving to another session is a departure from this one, and the
            // seat left behind would otherwise hold a slot nobody will spend.
            connection.peek().leave();
            connection.set(sync::connect(&code, &name, sinks));
            membership.set(Membership::In(code.clone()));
            status.set(sync::Status::Connecting);

            // Persisted so a team drafting all evening types the code once.
            let mut p = profile.write();
            p.session = Some(code);
            p.save(&ds);
        })
    };

    // The three things that move your own seat, in one place each.
    //
    // `profile.role` and `me.role` are the same fact stored twice — one drives
    // the pool board, the recommendations and the ban subject, the other holds
    // your slot and shows on everyone's roster — and every site that moved one
    // had to remember to move the other. Now that locking a hero can *change*
    // your role, forgetting would put the header on tank while the roster reads
    // "support · Ana" and the pick column ranks tanks. So they move together,
    // here, and nowhere else.
    let lock_hero = {
        let ds = dataset.clone();
        use_callback(move |hero: HeroId| {
            let role = me.write().lock(&ds, hero);
            // A name somebody typed for this hero is now answered by a real
            // pick. Dropped by the one client that acted, rather than by
            // everyone noticing at once and racing to write the shared board.
            board.write().remove_extra_ally(hero);
            if profile.peek().role != role {
                let mut p = profile.write();
                p.role = role;
                p.save(&ds);
            }
        })
    };

    // Written to the profile *and* the seat: it is a sticky personal setting,
    // and it rides on the seat so the rest of the roster can see what bracket
    // each person is answering for. Nothing about anybody else's scoring moves —
    // each client reads its own seat.
    let set_rank = {
        let ds = dataset.clone();
        use_callback(move |next: Rank| {
            rank_open.set(false);
            me.write().rank = next;
            let mut p = profile.write();
            p.rank = next;
            p.save(&ds);
        })
    };

    let set_role = {
        let ds = dataset.clone();
        use_callback(move |next: Role| {
            me.write().set_role(&ds, next);
            let mut p = profile.write();
            p.role = next;
            p.save(&ds);
        })
    };

    let unlock = use_callback(move |()| me.write().unlock());

    let leave = {
        let ds = dataset.clone();
        use_callback(move |()| {
            // Saying so, rather than just dropping the socket: a seat outlives
            // its connection on purpose, so a silent close would leave the slot
            // held for the ten minutes the room takes to expire.
            connection.peek().leave();
            connection.set(sync::Connection::idle(sync::client_id()));
            membership.set(Membership::Alone);
            status.set(sync::Status::Solo);
            seats.set(Vec::new());
            qr_open.set(false);

            let mut p = profile.write();
            p.session = None;
            p.save(&ds);
        })
    };

    // A session the server will not have is not one to sit in.
    //
    // Being rejected used to leave the bar showing the dead code with only a
    // `leave` button to escape it, which is a trap rather than an error: the
    // remembered code has been swept after its grace period, or was never
    // minted at all, and neither is something the user did. Dropping to
    // `Alone` puts `start a session` back on screen. The status pill keeps
    // saying "no such session", so the failure is still stated rather than
    // silently swallowed.
    {
        let ds = dataset.clone();
        use_effect(move || {
            if *status.read() != sync::Status::Rejected {
                return;
            }
            if matches!(*membership.peek(), Membership::Alone) {
                return;
            }
            connection.peek().close();
            membership.set(Membership::Alone);
            seats.set(Vec::new());
            qr_open.set(false);

            let mut p = profile.write();
            p.session = None;
            p.save(&ds);
        });
    }

    // Open the session the URL or the profile names, once, on mount. A share
    // link wins over the remembered session: following a link is an explicit
    // act and the remembered one is not.
    use_hook(|| {
        let from_link = session::code_from_location();
        if from_link.is_some() {
            // The code outlives the session it names, so leaving it in the
            // address bar would turn a bookmark into a broken rejoin later.
            session::clear_query();
        }
        if let Some(code) = from_link.or_else(|| profile.peek().session.clone()) {
            join.call(code);
        }
    });

    // Take back the seat the server kept for us, once per page.
    //
    // A seat outlives its socket precisely so that a reload does not empty a
    // slot mid-draft, and the server hands it back on the snapshot — but `me`
    // is seeded from the profile with nothing locked, and `roster_including_me`
    // prefers the local copy. So without this the pick survives on every screen
    // except the one it belongs to, and the next local edit publishes the empty
    // seat over it.
    //
    // A one-shot latch rather than a test on `locked`, because "never picked"
    // and "just un-picked" look identical from the value alone: the reconnect
    // loop re-delivers a snapshot on every successful retry, and pressing Esc
    // during an outage would otherwise be silently undone by a stale seat
    // arriving behind it. The server's copy is a backup for a page with no
    // memory of one, and a page only lacks that memory once.
    let mut seat_restored = use_signal(|| false);
    {
        let ds = dataset.clone();
        let my_id = my_id.clone();
        use_effect(move || {
            let mine = seats.read().iter().find(|seat| seat.id == my_id).cloned();
            // Nothing about us on the roster yet — not our one chance.
            let Some(mine) = mine else { return };
            if *seat_restored.peek() {
                return;
            }
            seat_restored.set(true);

            if let Some(hero) = mine.locked {
                if me.peek().locked.is_none() {
                    let role = me.write().lock(&ds, hero);
                    let mut p = profile.write();
                    p.role = role;
                    p.save(&ds);
                }
            }
        });
    }

    // Publish local board edits only. A board that already equals what we last
    // received came from another screen, and echoing it would fight with
    // whatever they are typing.
    //
    // Both of these read `status` so that they run again when the light comes
    // back on, and both move their shadow only on a send that actually went.
    // A socket that is connecting or reconnecting drops what it is handed; a
    // shadow advanced anyway would record the edit as published and never offer
    // it again, losing it to the team for good while looking sent on this
    // screen.
    use_effect(move || {
        // Read, not peeked: this is what makes the effect run again when the
        // light comes back on, and so what gets an edit made during an outage
        // finally sent.
        let live = status.read().is_live();
        let current = board.read().clone();
        let mut synced_board = synced_board;
        if live && current != *synced_board.peek() && connection.peek().publish_board(&current) {
            synced_board.set(current);
        }
    });

    // Your pool travels with your seat, so the rest of the team can ban for you
    // rather than only for themselves.
    //
    // Mirrored into the seat rather than published from the profile directly,
    // because a seat is the only thing the socket accepts and it has exactly one
    // writer. This runs whenever the pool or the role changes — the pool that
    // matters is the one for the role you are actually queued as — and the seat
    // effect below does the sending.
    let ds_pool = dataset.clone();
    use_effect(move || {
        let mine: Vec<HeroId> = {
            let profile = profile.read();
            profile.pool(&ds_pool, profile.role).iter().collect()
        };
        // Peeked, so this effect does not subscribe to the signal it writes.
        if me.peek().pool != mine {
            me.write().pool = mine;
        }
    });

    // The same for this client's own seat. A seat has exactly one writer, so
    // this can never conflict — the shadow exists only to keep the socket quiet
    // when nothing actually changed.
    use_effect(move || {
        let live = status.read().is_live();
        let current = me.read().clone();
        let mut synced_seat = synced_seat;
        if live && current != *synced_seat.peek() && connection.peek().publish_seat(&current) {
            synced_seat.set(current);
        }
    });

    // --- the derived draft -------------------------------------------------
    //
    // Rebuilt from scratch every render, like the frame below it: one pass over
    // at most five seats is far cheaper than the scoring it feeds, and there is
    // no cached copy to go stale.
    let state = SessionState {
        board: board.read().clone(),
        seats: roster_including_me(&seats.read(), &me.read()),
    };
    let ds = dataset.clone();
    let draft = state.draft_for(&ds, &my_id);
    // A ban is spent once for the whole team, so it is scored for the whole
    // team: everyone's pool and everyone's pick, out of the same pass that
    // decides who the allies are.
    let team = state.defended_team(&ds, &my_id);

    let frame = {
        let profile = profile.read();

        let ctx = profile.context(ds.hero_count());

        let recommendations = recommend(&ds, &draft, &ctx).unwrap_or_default();
        let bans = ban_recommendations(&ds, &draft, &ctx, &team);
        // Named apart from the function it calls, which is imported under the
        // obvious name and would otherwise be shadowed out of reach.
        let threat_board = draft
            .locked
            .map(|hero| threats(&ds, &draft, &ctx, hero))
            .unwrap_or_default();

        Frame {
            recommendations,
            bans,
            threats: threat_board,
        }
    };

    // What each team is building, for the chip on each board header.
    //
    // Your own locked hero is not in `draft.allies` — that list is everyone
    // else — so it is unioned back in here. A read of "your team" that leaves
    // out the hero you are playing is wrong in exactly the case you are looking
    // at it for.
    let ally_shape = {
        let mut mine = draft.allies.clone();
        mine.extend(draft.locked);
        shape_chip(&ds, &mine)
    };
    let enemy_shape = shape_chip(&ds, &draft.enemies);

    // --- actions ----------------------------------------------------------

    let ds_keys = dataset.clone();
    // The derived draft as of this render. Event handlers are rebuilt every
    // render, so this is never the stale copy it looks like — and taking it by
    // value avoids borrowing a signal inside a handler that also writes one.
    let draft_now = draft.clone();
    let on_key = move |evt: Event<KeyboardData>| {
        // Typing "route 66" must not walk the pick modes, and Escape belongs to
        // the filter box while it has the caret.
        if *search_focused.read() {
            return;
        }

        // Which chord this is belongs to `keys`, where it is a pure function
        // over the physical key and has tests. What it costs belongs here.
        let Some(command) = keys::command_for(evt.code(), evt.modifiers()) else {
            return;
        };
        evt.prevent_default();

        match command {
            // Clears the picks but deliberately keeps the map, which does not
            // change between rounds of the same match. In a session this clears
            // it for everyone — which is what "next round" means when five
            // people are looking at the same board — but it only ever unlocks
            // *your own* hero. Reaching across and clearing a teammate's pick is
            // not a thing one key should do.
            keys::Command::Clear => {
                board.write().clear_picks();
                unlock.call(());
            }
            // The hero you just picked in game. It switches the pick column into
            // swap mode, and it is your seat rather than the board: locking in
            // is the one thing in a session that is nobody else's.
            keys::Command::LockTop => {
                let top = frame_top(&ds_keys, &draft_now, &profile.read());
                if let Some(hero) = top {
                    lock_hero.call(hero);
                }
            }
            // Walk the pick modes in order, wrapping at the end. One key for
            // three modes rather than three keys, because the hand already knows
            // this one and a mode switch mid-draft is rare.
            //
            // Behind alt rather than ctrl because it gives up a pick the new
            // mode cannot hold — the same reason recording a result is. The
            // roster shows what everyone is playing, so the switch is also news
            // to the rest of the session.
            keys::Command::NextRole => {
                let at = Role::PLAYABLE_MODES
                    .iter()
                    .position(|mode| *mode == profile.peek().role)
                    .unwrap_or(0);
                set_role.call(Role::PLAYABLE_MODES[(at + 1) % Role::PLAYABLE_MODES.len()]);
            }
            keys::Command::Record { won } => {
                let role = profile.read().role;
                // Credited to the display name now that there is one. A log of
                // random client ids is unreadable the moment more than one
                // person is recording into it.
                let who = me.read().display_name().to_owned();
                let entry =
                    matchlog::MatchRecord::from_draft(&ds_keys, &draft_now, role, &who, won);
                match entry {
                    Some(entry) => {
                        matchlog::record(&entry);
                        // The draft is over; the map usually is not, so it
                        // survives into the next round.
                        board.write().clear_picks();
                        unlock.call(());
                        logged.set(Some(won));
                    }
                    // Nothing was locked, so there is no hero to credit.
                    None => logged.set(None),
                }
            }
        }
    };

    let ds_view = dataset.clone();
    let chip_of = move |hero: HeroId| hero_chip(&ds_view, hero);

    let role = profile.read().role;
    let map = draft.map.and_then(|m| map_chip(&dataset, m));

    // Every mode carries its own pool count, so how much of a role you have
    // marked as yours is legible for the modes you are not currently in.
    let modes: Vec<ModeChip> = Role::PLAYABLE_MODES
        .into_iter()
        .map(|mode| ModeChip {
            role: mode,
            label: mode.label().to_owned(),
            pool_size: profile.read().pool(&dataset, mode).len(),
            roster_size: dataset.heroes_in_role(mode).count(),
        })
        .collect();

    // --- the boards -------------------------------------------------------

    let picked_map = draft.map;
    // `search_maps` returns everything for an empty query and is already ranked
    // best-first, so the board reads as a filtered board rather than a result
    // list — same tiles, fewer of them.
    let query = map_query.read().clone();
    let map_tiles: Vec<MapTile> = search_maps(&dataset, &query, dataset.maps().len())
        .into_iter()
        .filter_map(|m| {
            let entry = dataset.map(m.map).ok()?;
            map_chip(&dataset, m.map).map(|chip| MapTile {
                map: m.map,
                name: chip.name,
                icon: chip.icon,
                selected: picked_map == Some(m.map),
                // The tile carries this so the board can draw the side toggle on
                // the map it belongs to; the components never see the dataset.
                has_sides: entry.mode.has_sides(),
            })
        })
        .collect();

    // Attack/defend only means something on the payload modes.
    let sides_apply = picked_map
        .and_then(|id| dataset.map(id).ok())
        .is_some_and(|m| m.mode.has_sides());

    // Both boards block per role rather than per team: a team with both its dps
    // can still take a tank, and a board that greyed out everything the moment
    // one row filled would be lying about four of the five slots.
    let enemy_board = {
        let enemies = &draft.enemies;
        // Counted off the derived draft rather than the board, so that the room
        // the tiles offer is the room the picks they draw come from.
        let room = draft.enemy_capacity(&dataset);
        roster(
            &dataset,
            |role, hero| plain_tile(enemies.contains(&hero), !room.fits(Some(role))),
            Some(&room),
            None,
        )
    };

    // Which picks came from a teammate's seat rather than from this board.
    // Theirs to change, not yours, so the board draws them as such.
    let seated_allies = board::seated_picks(&state.seats, &my_id);
    let my_lock = draft.locked;
    let my_role = me.read().role;

    let ally_board = {
        // Holds a slot for every seat that has not picked yet, your own
        // included — in 5v5 role queue as tank, nobody else on your team is one.
        let room = state.ally_capacity(&dataset, &my_id);
        // The extras as *shared*, not as derived. See `ally_tile_state`: a name
        // the team had no room for still has to be takeable back.
        let extras = board.read().extra_allies.clone();
        // Before you have picked, the slot your row is short is the one you are
        // about to spend, so the row says "you" rather than counting zero.
        let held = my_lock.is_none().then_some(my_role);
        roster(
            &dataset,
            |role, hero| {
                board::ally_tile_state(hero, role, my_lock, my_role, &seated_allies, &extras, &room)
            },
            Some(&room),
            held,
        )
    };

    // The roster panel, resolved into plain values for the view.
    let roster_rows: Vec<RosterRow> = session::order_roster(&state.seats, &my_id)
        .into_iter()
        .map(|seat| {
            let chip = seat.locked.map(|hero| hero_chip(&dataset, hero));
            // Two people can put the same hero up before the game stops them.
            // The boards can only draw it once — it is one hero and the team
            // fields one of it — so this is the only place the collision can
            // be seen at all.
            let contested = seat.locked.is_some_and(|hero| {
                state
                    .seats
                    .iter()
                    .filter(|other| other.locked == Some(hero))
                    .count()
                    > 1
            });
            // Only for a seat still choosing: once they lock, the pick is the
            // answer and the pool is history.
            let pool_icons: Vec<String> = match seat.locked {
                Some(_) => Vec::new(),
                None => seat
                    .pool
                    .iter()
                    .take(ROSTER_POOL_SHOWN)
                    .map(|hero| hero_chip(&dataset, *hero).icon)
                    .collect(),
            };
            RosterRow {
                name: seat.display_name().to_owned(),
                role_label: seat.role.label().to_owned(),
                hero: chip.as_ref().map(|c| c.name.clone()),
                icon: chip.map(|c| c.icon),
                connected: seat.connected,
                is_me: seat.id == my_id,
                contested,
                pool_extra: seat.pool.len().saturating_sub(pool_icons.len()),
                pool: pool_icons,
                rank: seat.rank,
            }
        })
        .collect();

    // Only the role you are picking for: the pool marks the heroes the list
    // will highlight, and nothing recommends a hero outside your role.
    let pool_board = vec![BoardRow {
        role,
        label: role.label().to_owned(),
        // Your pool is not a team, so there is nothing for it to be out of.
        capacity: None,
        mine: false,
        tiles: dataset
            .heroes_in_role(role)
            .map(|hero| pool_tile(&dataset, hero, profile.read().comfort(hero)))
            .collect(),
    }];

    let ban_subject = ban_subject(&frame.bans.subject, profile.read().rank, role);
    // With one hero to defend, "hardest on" can only name it back at you.
    let locked_subject = matches!(frame.bans.subject, BanSubject::One { locked: true, .. });
    let patch_subject = matches!(frame.bans.subject, BanSubject::Patch);

    // Three different silences, said three different ways. The last one is the
    // one that has to be careful: an unrated pair is not a clean bill of
    // health, and copy that read "nothing beats you" would turn the sources
    // having no opinion into a measurement — the same mistake `threats` itself
    // refuses to make by leaving those enemies off the list.
    let threat_empty = match (draft.locked.is_some(), draft.enemies.is_empty()) {
        (false, _) => "nothing locked — this reads your pick against their team",
        (true, true) => "nothing on their side yet",
        (true, false) => "no source has rated your pick against any of them",
    }
    .to_owned();

    // The link to hand a teammate, and its QR. Both derived from the code, and
    // both `None` when drafting alone. The QR is only built while the panel is
    // open — it is a few hundred modules of string formatting, which is nothing
    // next to scoring, but there is no reason to do it on every keystroke.
    let share_link = membership
        .read()
        .code()
        .zip(session::origin())
        .map(|(code, origin)| session::share_url(&origin, code));
    let qr_image = qr_open()
        .then(|| share_link.as_deref().and_then(session::qr_data_url))
        .flatten();

    // Resolved once and rendered twice — the pick column, and the strip pinned
    // to the foot of a phone. Built here rather than inline at either call site
    // so the two cannot end up ranking, formatting or truncating differently:
    // the strip takes its three from the front of this same list.
    // Hoisted out of the closure: the patch-strength line prints a ladder-wide
    // win rate and has to say so once a rung is chosen, and eight rows each
    // taking their own read of the signal would be eight borrows for one value
    // that cannot change between them.
    let rank = profile.read().rank;
    let rec_rows: Vec<ui::RecRow> = frame
        .recommendations
        .iter()
        .take(8)
        .map(|rec| {
            ui::RecRow::build(
                rec,
                &dataset,
                draft.locked.is_some(),
                // The value, not membership in a derived `HeroSet`: the row draws
                // its star from `> 0` and words that star from the rung, and a
                // `bool` throws the rung away at this boundary. One read per row,
                // exactly as the pool board takes one per tile.
                profile.read().comfort(rec.hero),
                rank,
            )
        })
        .collect();

    // What the score column means, said once under the list. The margin comes
    // off `printed_lead`, which is where the reason it rounds when it does is
    // written down.
    let locked_name = draft.locked.map(|hero| chip_of(hero).name);
    let top_name = frame
        .recommendations
        .first()
        .map(|rec| chip_of(rec.hero).name);
    let score_note = ui::score_note(
        locked_name.as_deref(),
        top_name.as_deref(),
        printed_lead(&frame.recommendations),
        // Both counted over the rows about to be drawn rather than over the whole
        // role, so neither sentence can name a hero that is not on screen.
        tie_count(&rec_rows, draft.enemies.len()),
        rec_rows.len(),
        // The eight rows on screen, not the whole role: the line says "nothing
        // *here* clears +15", and here is what you can see.
        rec_rows.iter().any(|row| row.worth_swapping),
        profile.read().weights.swap_threshold,
    );

    rsx! {
        div {
            class: "app",
            // There is no text input to hold focus any more, so the shortcuts
            // live on the root and this keeps them reachable: a click on a
            // board button moves focus to that button, and any click at all
            // hands it straight back.
            tabindex: "0",
            onmounted: move |evt| {
                let node = evt.data();
                root_ref.set(Some(node.clone()));
                spawn(async move { let _ = node.set_focus(true).await; });
            },
            onkeydown: on_key,
            onclick: move |_| {
                // Grabbing focus back is what keeps the shortcuts alive after a
                // stray click, but doing it mid-word would empty the filter box
                // of its caret.
                if *search_focused.read() {
                    return;
                }
                if let Some(node) = root_ref.read().clone() {
                    spawn(async move { let _ = node.set_focus(true).await; });
                }
            },

            ui::Header {
                role,
                format: draft.format,
                map: map.clone(),
                sides_apply,
                side: draft.side,
                modes,
                generated: dataset.generated.clone(),
                sync_status: status.read().label(),
                logged: *logged.read(),
                on_role: move |next: Role| set_role.call(next),
                // One handler for both halves of the switch, so there is exactly
                // one place that moves the room, remembers the choice and drops
                // what the new format has no room for.
                on_format: {
                    let ds = dataset.clone();
                    move |next: Format| {
                        logged.set(None);
                        board.write().set_format(&ds, next);
                        let mut p = profile.write();
                        p.format = next;
                        p.save(&ds);
                    }
                },
                on_reset_all: move |_| {
                    logged.set(None);
                    map_query.set(String::new());
                    // "New match" clears the shared board for everyone, and
                    // your own pick with it — but not anyone else's, which is
                    // theirs to take back.
                    board.write().clear_all();
                    unlock.call(());
                },
            }

            // The draft itself. A landmark around it and not around the
            // whole of .app, so that "skip to main content" actually skips
            // the header — the mark, the mode switch and the match context
            // are navigation and status, not the content being navigated to.
            // A plain block box inside a plain block box, so it changes no
            // layout.
            main { class: "app-main",
                ui::SessionBar {
                    code: membership.read().code().map(str::to_owned),
                    share_url: share_link.clone(),
                    qr: qr_image.clone(),
                    qr_open: qr_open(),
                    status: status.read().label(),
                    name: profile.read().name.clone(),
                    entry: join_entry(),
                    on_entry: move |next: String| join_entry.set(next),
                    on_name: {
                        let ds = dataset.clone();
                        move |next: String| {
                            {
                                let mut p = profile.write();
                                p.name = next.clone();
                                p.save(&ds);
                            }
                            // The roster is showing this to four other people, so it
                            // travels with the seat rather than waiting for a reload.
                            me.write().name = next;
                        }
                    },
                    on_focus: move |focused: bool| search_focused.set(focused),
                    on_create: move |_| {
                        spawn(async move {
                            if let Some(code) = session::create().await {
                                join.call(code);
                            }
                            // No server means no session, and nothing to say about
                            // it: the app carries on exactly as it did before.
                        });
                    },
                    on_join: move |_| {
                        if let Some(code) = session::parse_code(&join_entry()) {
                            join_entry.set(String::new());
                            join.call(code);
                        }
                    },
                    on_leave: move |_| leave.call(()),
                    on_copy: {
                        let share = share_link.clone();
                        move |_| {
                            if let Some(url) = &share {
                                session::copy_to_clipboard(url);
                            }
                        }
                    },
                    on_qr: move |_| {
                        let open = qr_open();
                        qr_open.set(!open);
                    },
                }

                // Shown when drafting alone too. It is one row, and it is the
                // legend for the ally board's amber tile: this is you, this is what
                // you are on. Hiding it solo left the board's one "mine" marker
                // with nothing on screen to explain it.
                if !roster_rows.is_empty() {
                    ui::Roster { rows: roster_rows }
                }

                // Above the map, because this is the one board that is not about the
                // match in front of you: it is who you play, set once and then left
                // alone, so it sits where you configure rather than where you draft.
                ui::HeroBoard {
                    title: format!("my pool · {}", role.label()),
                    side: "pool".to_owned(),
                    rows: pool_board,
                    // Your pool is weeks of accumulated configuration, not draft
                    // state, so this one asks before it throws it away.
                    reset_confirm: true,
                    // The only board whose click is not a toggle, and the only
                    // one that moves a scoring term. Both worth saying once,
                    // where the clicking happens.
                    note: ui::POOL_NOTE.to_owned(),
                    on_toggle: {
                        let ds = dataset.clone();
                        move |hero: HeroId| {
                            let mut p = profile.write();
                            p.cycle_comfort(hero);
                            p.save(&ds);
                        }
                    },
                    on_reset: {
                        let ds = dataset.clone();
                        move |_| {
                            let mut p = profile.write();
                            p.clear_pool(&ds, role);
                            p.save(&ds);
                        }
                    },
                }

                ui::MapBoard {
                    maps: map_tiles,
                    query: map_query.read().clone(),
                    side: draft.side,
                    on_query: move |next: String| map_query.set(next),
                    on_submit: {
                        let ds = dataset.clone();
                        move |_| {
                            // Enter takes the best match, which is the whole point
                            // of typing rather than hunting for the tile.
                            let Some(id) = overwatch_core::resolve_map(&ds, &map_query.read()) else {
                                return;
                            };
                            logged.set(None);
                            let mut b = board.write();
                            b.map = Some(id);
                            // Same invariant the click path holds: a side belongs to
                            // the map it was picked on, and typing your way to a
                            // symmetric one has to drop it too.
                            if !ds.map(id).is_ok_and(|m| m.mode.has_sides()) {
                                b.side = None;
                            }
                            drop(b);
                            map_query.set(String::new());
                        }
                    },
                    on_focus: move |focused: bool| search_focused.set(focused),
                    on_side: move |next: Option<Side>| { board.write().side = next; },
                    on_pick: {
                        let ds = dataset.clone();
                        move |id: MapId| {
                            logged.set(None);
                            let mut b = board.write();
                            // Clicking the map you are on clears it, the same way
                            // clicking a picked hero takes the pick back.
                            b.map = if b.map == Some(id) { None } else { Some(id) };
                            // A side is a property of the map you are on; carrying
                            // one over to a mode that has no sides would leave it
                            // set and invisible.
                            let keeps_side = b
                                .map
                                .and_then(|id| ds.map(id).ok())
                                .is_some_and(|m| m.mode.has_sides());
                            if !keeps_side {
                                b.side = None;
                            }
                        }
                    },
                    on_reset: move |_| {
                        map_query.set(String::new());
                        let mut b = board.write();
                        b.map = None;
                        b.side = None;
                    },
                }

                // Ally on the left, enemy on the right. Swapped in source order
                // rather than with a CSS `order`, so tab order still follows what
                // you see.
                div { class: "boards",
                    ui::HeroBoard {
                        // The board is your whole team, you included, so it says
                        // which of the two the next click is about — and, since
                        // only your own row takes a hero for you, which row.
                        title: if my_lock.is_none() {
                            format!("ally · click a {} to take yours", my_role.label())
                        } else {
                            "ally".to_owned()
                        },
                        side: "ally".to_owned(),
                        rows: ally_board,
                        claiming: my_lock.is_none(),
                        shape: ally_shape,
                        // Dispatches on the same ladder the tiles were drawn from,
                        // so what a click does is what the tile said it would —
                        // which is why the ladder itself is `board::ally_click`,
                        // beside the one that drew them. A teammate's pick never
                        // reaches here; the component drops those before they
                        // leave it.
                        on_toggle: {
                            let ds = dataset.clone();
                            move |hero: HeroId| {
                                logged.set(None);
                                let hero_role = ds.hero(hero).ok().map(|entry| entry.role);
                                // Read out and dropped before the arms run: a
                                // borrow held across them would meet the
                                // `board.write()` two of them make.
                                let typed = board.peek().extra_allies.clone();
                                match board::ally_click(hero, hero_role, my_lock, my_role, &typed) {
                                    board::AllyClick::TakeBack => unlock.call(()),
                                    board::AllyClick::RemoveExtra => {
                                        board.write().remove_extra_ally(hero);
                                    }
                                    // Your own row, and nothing of yours on it
                                    // yet, so this is you.
                                    board::AllyClick::Claim => lock_hero.call(hero),
                                    // A teammate who is not in the session and
                                    // whom somebody has to type in.
                                    board::AllyClick::AddExtra => {
                                        board.write().add_extra_ally(hero);
                                    }
                                }
                            }
                        },
                        // Your own pick goes with the typed names. A reset that
                        // visibly left one tile lit would read as broken.
                        on_reset: move |_| {
                            board.write().extra_allies.clear();
                            unlock.call(());
                        },
                    }

                    ui::HeroBoard {
                        title: "enemy".to_owned(),
                        side: "enemy".to_owned(),
                        rows: enemy_board,
                        shape: enemy_shape,
                        on_toggle: {
                            let ds = dataset.clone();
                            move |hero: HeroId| {
                                logged.set(None);
                                board.write().toggle_enemy(&ds, hero);
                            }
                        },
                        on_reset: move |_| { board.write().enemies.clear(); },
                    }
                }

                div { class: "columns",
                    // Ban and matchups share a column because they are the two
                    // halves of one question, split by whether the hero is on the
                    // enemy board yet: the ban list drains as picks land and this
                    // one fills with those same heroes, so the column is never
                    // dead. Wrapped rather than left to grid auto-placement, which
                    // would drop the second panel below the *tallest* row and leave
                    // a hole under the ban list.
                    div { class: "column-stack",
                    ui::BanPanel {
                        subject: ban_subject,
                        items: frame.bans.candidates
                            .iter()
                            .take(8)
                            .map(|ban| {
                                let chip = chip_of(ban.hero);
                                // The patch rung ranks on strength rather than on
                                // any pair, so it shows a figure instead of a
                                // rationale there is none of. Resolved with the
                                // claim about whose words those are, in
                                // `ui::ban_text`, so a row can never print a site's
                                // sentence unattributed or our own figure credited
                                // to them.
                                let (text, cited) = ui::ban_text(
                                    dataset.win_rate(ban.hero),
                                    patch_subject,
                                    &ban.text,
                                    profile.read().rank,
                                );
                                BanRow {
                                    hero: ban.hero,
                                    name: chip.name,
                                    icon: chip.icon,
                                    // The weighted score rather than the raw
                                    // matchup, because this is the number the list
                                    // is *sorted* by and a column ordered by one
                                    // figure while displaying another reads as
                                    // broken. Negated so it carries the same sign
                                    // as every other number on screen: below zero
                                    // is losing, exactly as in the pick column,
                                    // whose score is a weighted sum too.
                                    score: format!("{:+.0}", ban.score * -100.0),
                                    worst: (!locked_subject)
                                        .then(|| ban.worst.map(|hero| chip_of(hero).name))
                                        .flatten(),
                                    worst_owner: ban.worst_owner.clone(),
                                    text,
                                    cited,
                                    prevalence: ui::prevalence_note(
                                        ban.prevalence,
                                        profile.read().rank,
                                    ),
                                }
                            })
                            .collect::<Vec<_>>(),
                    }

                    ui::ThreatPanel {
                        // Named for the hero, not for the relation: the ban panel
                        // right above already spends "vs" on the opposite one, and
                        // two adjacent headers reading "vs" about inverse things
                        // would be worse than no label.
                        subject: my_lock.map(|hero| format!("as {}", chip_of(hero).name)),
                        items: frame.threats
                            .iter()
                            .filter_map(|threat| {
                                my_lock.map(|locked| ui::ThreatRow::build(threat, locked, &dataset))
                            })
                            .collect::<Vec<_>>(),
                        unrated: draft.enemies.len().saturating_sub(frame.threats.len()),
                        empty: threat_empty,
                    }
                    }

                    ui::Recommendations {
                        items: rec_rows.clone(),
                        swap_mode: draft.locked.is_some(),
                        note: score_note,
                        rank: profile.read().rank,
                        rank_open: rank_open(),
                        on_lock: move |hero: HeroId| lock_hero.call(hero),
                        on_rank: move |next: Rank| set_rank.call(next),
                        // Picking a rung closes the sheet: it is a one-shot choice,
                        // not something to sit comparing, and leaving it open would
                        // cover the list the change was made to reorder.
                        on_rank_open: move |_| {
                            let next = !rank_open();
                            rank_open.set(next);
                        },
                    }
                }
            }

            // Between the draft and the footer, so opening it grows the page
            // downward and moves nothing above it. The counts come off the
            // dataset rather than the copy, so the coverage sentence is a
            // measurement of the tables in this bundle.
            ui::HowItWorks {
                generated: dataset.generated.clone(),
                with_note: dataset.notes_published(),
                rated: dataset.pairs_rated(),
            }

            ui::Footer {}

            // Inside .app rather than beside it, so the root's keyboard handler
            // and its click-to-refocus still cover it — the strip's own buttons
            // stop the click before it gets there. Last, because it is fixed:
            // source order is what a screen reader follows, and the pick column
            // it mirrors has already been read by this point.
            ui::AnswerStrip {
                items: rec_rows,
                swap_mode: draft.locked.is_some(),
                on_lock: move |hero: HeroId| lock_hero.call(hero),
            }
        }
    }
}

/// One roster board's rows: every hero the game has, grouped by role.
///
/// `state` is asked per board rather than per hero, because the same hero can
/// be a live enemy pick, an untaken ally slot, and one of yours at the same
/// time — which is the whole reason the boards are separate rather than one
/// list with a mode. It takes the row's role as well as the hero, because with
/// per-role caps the answer is a property of the row: a full dps row says
/// nothing about whether a tank can still be entered. Taking it here also means
/// the roles are free — the rows are built by walking them — rather than one
/// lookup per tile.
///
/// One closure returning one state, rather than the two independent predicates
/// this used to take. Those could describe a tile that was both picked and
/// unclickable, and the caller had to remember to rule it out by hand; it did
/// not, and a teammate's pick came out clickable for months. There is nothing
/// left here to get wrong.
///
/// `room` is what the row's count is drawn from, and `None` on a board that has
/// no cap to count against. `mine` names the row, if any, whose remaining slot
/// is one the viewer is holding open themselves.
fn roster(
    dataset: &Dataset,
    state: impl Fn(Role, HeroId) -> (TileState, Option<String>),
    room: Option<&Capacity>,
    mine: Option<Role>,
) -> Vec<BoardRow> {
    Role::ALL
        .into_iter()
        .map(|role| BoardRow {
            role,
            label: role.label().to_owned(),
            capacity: room.map(|room| room.free_in(role)),
            mine: mine == Some(role),
            tiles: dataset
                .heroes_in_role(role)
                .map(|hero| {
                    let (state, owner) = state(role, hero);
                    tile_of(dataset, hero, state, owner)
                })
                .collect(),
        })
        .collect()
}

/// One board tile, with the portrait and the name already resolved.
fn tile_of(dataset: &Dataset, hero: HeroId, state: TileState, owner: Option<String>) -> HeroTile {
    let chip = hero_chip(dataset, hero);
    HeroTile {
        hero,
        name: chip.name,
        icon: chip.icon,
        state,
        owner,
        comfort: None,
    }
}

/// A tile on the pool board, which is the one board that is not about a team.
///
/// Its own constructor rather than a third case inside [`plain_tile`], because
/// that one is shared with the enemy roster and the pool has nothing in common
/// with it: no seats, no capacity, and a level rather than a picked/not-picked
/// bit. The state stays [`TileState::Free`] on every tile, claimed or not —
/// the level is what says so, and it rides beside the state rather than inside
/// it. See `HeroTile::comfort`.
fn pool_tile(dataset: &Dataset, hero: HeroId, comfort: i8) -> HeroTile {
    HeroTile {
        comfort: ComfortStep::of(comfort),
        ..tile_of(dataset, hero, TileState::Free, None)
    }
}

/// The state of a tile on a board that only ever holds picks and refusals —
/// the enemy roster and your own pool. Neither has seats, so neither can have
/// a hero that is somebody else's.
fn plain_tile(picked: bool, blocked: bool) -> (TileState, Option<String>) {
    let state = match (picked, blocked) {
        (true, _) => TileState::Picked,
        (false, true) => TileState::Blocked,
        (false, false) => TileState::Free,
    };
    (state, None)
}

/// The hero the "lock" shortcut should snap to: the current best suggestion.
fn frame_top(dataset: &Dataset, draft: &Draft, profile: &Profile) -> Option<HeroId> {
    // Deliberately the same assembly the pick column uses, not a second copy of
    // it: this is the hero the lock shortcut takes, and a context that disagreed
    // would lock one the visible list never put first.
    let ctx = profile.context(dataset.hero_count());

    recommend(dataset, draft, &ctx)
        .ok()?
        .first()
        .map(|rec| rec.hero)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one decision behind the margin, and the one no test above `main` can
    /// reach: it is the difference of the printed figures, not the printing of
    /// the difference. Both heroes here round to `+41`, so the honest answer is
    /// zero — `round((0.414 - 0.408) * 100)` is 1, and a note claiming a lead of
    /// 1 over two identical numbers is the failure this forecloses.
    #[test]
    fn the_margin_is_the_difference_of_the_printed_figures_not_the_printed_difference() {
        let recs = vec![rec_at(HeroId(0), 0.414), rec_at(HeroId(1), 0.408)];

        assert_eq!(
            format!("{:+.0}", recs[0].score * 100.0),
            format!("{:+.0}", recs[1].score * 100.0),
            "the fixture no longer prints two equal figures, so it tests nothing"
        );
        assert_eq!(printed_lead(&recs), Some(0));
    }

    /// A real gap still reads as one, and still agrees with the column.
    #[test]
    fn a_clear_leader_reports_the_gap_the_two_rows_show() {
        let recs = vec![rec_at(HeroId(0), 0.41), rec_at(HeroId(1), 0.35)];
        assert_eq!(printed_lead(&recs), Some(6));
    }

    /// The half of the tie rule that is not a number, and the one the measurement
    /// forced: on an empty enemy board the top eight are flat by construction, so
    /// the note would decline to call the draft on 74% of opening screens — before
    /// the reader has told the app anything at all.
    #[test]
    fn nothing_is_called_a_tie_until_the_draft_says_something() {
        let rows = vec![tied_row(0), tied_row(1), tied_row(2)];

        assert_eq!(
            tie_count(&rows, 0),
            0,
            "an empty enemy board reports no tie"
        );
        assert_eq!(tie_count(&rows, 1), 3, "one enemy is a draft to be about");
    }

    /// And the count itself is the rows that carry the flag, not the rows there
    /// are — the scorer decides who is tied, this only decides whether to say so.
    #[test]
    fn the_tie_count_is_the_rows_the_scorer_could_not_separate() {
        let mut rows = vec![tied_row(0), tied_row(1), tied_row(2)];
        rows[2].tied_with_top = false;

        assert_eq!(tie_count(&rows, 5), 2);
    }

    fn tied_row(place: usize) -> ui::RecRow {
        ui::RecRow {
            hero: HeroId(place as u16),
            name: String::new(),
            icon: String::new(),
            score: String::new(),
            is_locked: false,
            worth_swapping: false,
            place,
            tied_with_top: true,
            comfort: 0,
            reasons: Vec::new(),
            coverage: None,
        }
    }

    /// Nothing to lead. Worded by `score_note` rather than printed as a margin.
    #[test]
    fn one_candidate_or_none_has_no_margin_to_report() {
        assert_eq!(printed_lead(&[rec_at(HeroId(0), 0.41)]), None);
        assert_eq!(printed_lead(&[]), None);
    }

    fn rec_at(hero: HeroId, score: f32) -> Recommendation {
        Recommendation {
            hero,
            score,
            delta_vs_locked: None,
            worth_swapping: false,
            is_locked: false,
            breakdown: Default::default(),
            place: 0,
            tied_with_top: false,
            reasons: Vec::new(),
        }
    }

    // The caption is the only place the ban list says which rung sorted it, and
    // it is one arm of a match on a subject with four other shapes. A branch on
    // a string nothing asserts is how `synergy.toml` shipped empty behind a
    // weighted term for months.
    #[test]
    fn the_patch_ban_caption_names_the_rung_that_sorted_it() {
        assert_eq!(
            ban_subject(&BanSubject::Patch, Rank::Grandmaster, Role::Tank),
            "strongest at grandmaster+ right now"
        );
    }

    // The aggregate is not a division, and saying "strongest at all ranks"
    // would invent a bracket for somebody who never opened the picker.
    #[test]
    fn the_whole_ladder_is_not_named_as_though_it_were_a_division() {
        assert_eq!(
            ban_subject(&BanSubject::Patch, Rank::All, Role::Tank),
            "strongest right now"
        );
    }

    // Every other subject scores on threat, where the rung does not enter. A
    // rung in these captions would be a claim the number does not make.
    #[test]
    fn no_other_ban_subject_mentions_a_rung_at_any_rank() {
        let others = [
            BanSubject::One {
                who: "Sam".to_owned(),
                is_me: true,
                locked: true,
                heroes: 1,
            },
            BanSubject::One {
                who: "Sam".to_owned(),
                is_me: false,
                locked: true,
                heroes: 1,
            },
            BanSubject::One {
                who: "Sam".to_owned(),
                is_me: true,
                locked: false,
                heroes: 3,
            },
            BanSubject::One {
                who: "Sam".to_owned(),
                is_me: false,
                locked: false,
                heroes: 3,
            },
            BanSubject::Team {
                known: 3,
                locked: 0,
            },
            BanSubject::Team {
                known: 3,
                locked: 2,
            },
        ];

        for subject in &others {
            for rank in Rank::CHOICES {
                let text = ban_subject(subject, rank, Role::Tank);
                assert!(
                    !text.contains(rank.label()) && !text.contains("strongest"),
                    "{subject:?} at {rank:?} named a rung: {text:?}"
                );
            }
        }
    }

    // The rung is the only thing that moved. Pinned so a future edit to the
    // caption cannot quietly reword the five subjects that carry the draft.
    #[test]
    fn the_threat_captions_still_read_as_they_did() {
        let at = |subject: &BanSubject| ban_subject(subject, Rank::Diamond, Role::Support);

        assert_eq!(
            at(&BanSubject::One {
                who: "Sam".to_owned(),
                is_me: true,
                locked: true,
                heroes: 1,
            }),
            "vs your pick"
        );
        assert_eq!(
            at(&BanSubject::One {
                who: "Sam".to_owned(),
                is_me: false,
                locked: true,
                heroes: 1,
            }),
            "vs Sam's pick"
        );
        assert_eq!(
            at(&BanSubject::One {
                who: "Sam".to_owned(),
                is_me: true,
                locked: false,
                heroes: 3,
            }),
            "vs your pool · 3 support"
        );
        assert_eq!(
            at(&BanSubject::One {
                who: "Sam".to_owned(),
                is_me: false,
                locked: false,
                heroes: 3,
            }),
            "vs Sam's pool · 3"
        );
        assert_eq!(
            at(&BanSubject::Team {
                known: 3,
                locked: 0,
            }),
            "vs your team · 3 known"
        );
        assert_eq!(
            at(&BanSubject::Team {
                known: 3,
                locked: 2,
            }),
            "vs your team · 3 known, 2 in"
        );
    }
}
