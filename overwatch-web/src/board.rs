//! Which state each tile on a roster board is in.
//!
//! The boards are the whole interface — which board you click *is* the answer
//! to "which team" — so the rule deciding what a tile means, and therefore what
//! a click on it does, is the densest logic in the client. It lives here as
//! plain functions with tests rather than inline in the view, for the same
//! reason `SessionState::draft_for` lives in the core: it is the one place the
//! screen can be wrong about the draft, and nothing can reach it inside a
//! component.
//!
//! The ally board is the interesting one. Three different things share it —
//! your own pick, a teammate's pick arriving from their seat, and a name
//! somebody typed for a teammate who is not in the session — and telling them
//! apart is the difference between a board you can read and one that quietly
//! writes shared state when you click a portrait that was never yours.

use overwatch_core::{Capacity, HeroId, Role, Seat};

use crate::ui::TileState;

/// A teammate's pick, and who to credit it to.
pub struct SeatedPick {
    pub hero: HeroId,
    pub owner: String,
}

/// What one tile on the ally board is, and whose.
///
/// The order of these branches is the rule, and it mirrors the precedence
/// `SessionState::assemble_allies` already applies — seats before typed names.
/// A disagreement between the two is a bug in one of them, not a difference of
/// opinion: this decides what you can click, that decides what reaches the
/// scorer, and a board offering a click the derivation drops is exactly the
/// failure per-role caps invite.
///
/// 1. **Mine first**, so my own pick stays takeable-back even when its row is
///    otherwise full. It is what fills my own reservation, so the row being
///    full is a description of my pick rather than a reason to refuse it.
/// 2. **A teammate's pick**, which is inert. Not mine to change and not the
///    board's to un-pick — theirs arrives from their seat.
/// 3. **A typed name**, tested against the raw shared list rather than against
///    the derived team. The two differ when a typed name is crowded out of the
///    team it was typed onto, and reading the derived one would draw such a
///    pick unlit while it still sat in shared state — visible to nobody, and
///    still eating the click that would remove it.
/// 4. **My own role's row, while I have no pick of my own.** Nothing is blocked
///    there, because this click is about to *become* my pick and it spends the
///    reservation my own seat is holding. Refusing a tile on the strength of a
///    slot that the same click would spend is the board arguing with itself —
///    which is what a 5v5 tank saw: their own held slot greyed out the entire
///    tank row.
///
///    Only that row, though. Every other row is a teammate being typed in, and
///    reading a click on one as my pick is how a tank marking the enemy-of-the-
///    moment's Genji ended up *as* Genji: `Seat::lock` moves the declared role
///    to follow the hero, so the pick column, the pool board and the roster all
///    left tank behind on a click that never meant to say anything about me.
///    The mode is mine to declare, and the mode switch is where it is declared.
/// 5. Otherwise the ordinary cap, which governs typing a teammate in.
pub fn ally_tile_state(
    hero: HeroId,
    role: Role,
    my_lock: Option<HeroId>,
    my_role: Role,
    seated: &[SeatedPick],
    extras: &[HeroId],
    room: &Capacity,
) -> (TileState, Option<String>) {
    if my_lock == Some(hero) {
        return (TileState::Mine, None);
    }
    if let Some(pick) = seated.iter().find(|pick| pick.hero == hero) {
        return (TileState::Theirs, Some(pick.owner.clone()));
    }
    if extras.contains(&hero) {
        return (TileState::Picked, None);
    }
    if (my_lock.is_none() && role == my_role) || room.fits(Some(role)) {
        return (TileState::Free, None);
    }
    (TileState::Blocked, None)
}

/// What a click on an ally tile means.
///
/// The same ladder [`ally_tile_state`] draws, seen from the other side, and the
/// reason both live here rather than in the view: what a tile *says* it will do
/// and what a click on it *does* are two statements of one rule, and the only
/// way to keep them from drifting is to write them next to each other and test
/// them together. The view's own copy of this ladder is exactly where the role
/// test went missing.
///
/// [`TileState::Theirs`] has no meaning here and never arrives: the component
/// drops a tile it drew as unclickable before the click leaves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllyClick {
    /// My own pick, handed back.
    TakeBack,
    /// A name somebody typed, taken off the board.
    RemoveExtra,
    /// The hero I am playing. Only ever one of my own role.
    Claim,
    /// A teammate who is not in the session, typed in for the scorer.
    AddExtra,
}

/// Which of the four a click on `hero` is.
///
/// `role` is the row the tile was drawn in, which is the hero's own role — the
/// rows are built by walking [`Dataset::heroes_in_role`]. A hero the roster
/// cannot name has no row to be on and so cannot be the one I am claiming;
/// passing `None` types it in, which is the same conservative reading
/// `Seat::lock` takes when it leaves a declared role alone rather than guessing.
///
/// [`Dataset::heroes_in_role`]: overwatch_core::Dataset::heroes_in_role
pub fn ally_click(
    hero: HeroId,
    role: Option<Role>,
    my_lock: Option<HeroId>,
    my_role: Role,
    extras: &[HeroId],
) -> AllyClick {
    if my_lock == Some(hero) {
        return AllyClick::TakeBack;
    }
    if extras.contains(&hero) {
        return AllyClick::RemoveExtra;
    }
    if my_lock.is_none() && role == Some(my_role) {
        return AllyClick::Claim;
    }
    AllyClick::AddExtra
}

/// A teammate's pick as the board should credit it.
///
/// Someone who has dropped keeps their pick — the team is still playing around
/// the hero they are on — so the label says so rather than leaving a tile that
/// cannot be clicked with no account of why.
pub fn seated_picks(seats: &[Seat], me: &str) -> Vec<SeatedPick> {
    seats
        .iter()
        .filter(|seat| seat.id != me)
        .filter_map(|seat| {
            seat.locked.map(|hero| SeatedPick {
                hero,
                owner: if seat.connected {
                    seat.display_name().to_owned()
                } else {
                    format!("{} · offline", seat.display_name())
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use overwatch_core::{Format, Queue, TeamSize};

    const SIGMA: HeroId = HeroId(0);
    const REIN: HeroId = HeroId(1);
    const TRACER: HeroId = HeroId(2);
    const ANA: HeroId = HeroId(3);
    const KIRIKO: HeroId = HeroId(4);
    const SOJOURN: HeroId = HeroId(5);

    fn seat(id: &str, hero: HeroId) -> SeatedPick {
        SeatedPick {
            hero,
            owner: id.to_owned(),
        }
    }

    /// A 5v5 role-queue team with `taken` of `role` already spoken for.
    fn room_with(role: Role, taken: usize) -> Capacity {
        let mut room = Capacity::of(Format::default());
        for _ in 0..taken {
            room.take(Some(role));
        }
        room
    }

    /// The reported bug, pinned dead. My own pick used to fall through every
    /// branch and come out as the most greyed-out thing on the board.
    #[test]
    fn my_own_pick_draws_as_mine_rather_than_as_a_blocked_tile() {
        // My own tank fills the only 5v5 tank slot, so the row is full — which
        // must not be read as a reason to refuse me my own hero back.
        let (state, owner) = ally_tile_state(
            SIGMA,
            Role::Tank,
            Some(SIGMA),
            Role::Tank,
            &[],
            &[],
            &room_with(Role::Tank, 1),
        );

        assert_eq!(state, TileState::Mine);
        assert!(state.is_clickable(), "clicking it takes the pick back");
        assert_eq!(owner, None, "it is mine; there is nobody to credit");
    }

    /// The other reported bug. A teammate's pick used to come out selected and
    /// clickable, and the click wrote the shared board — leaving a name nobody
    /// typed, which reappeared as a real pick when they later swapped off.
    #[test]
    fn a_teammates_pick_is_theirs_and_cannot_be_clicked() {
        let seated = vec![seat("mika", ANA)];
        let (state, owner) = ally_tile_state(
            ANA,
            Role::Support,
            None,
            Role::Support,
            &seated,
            &[],
            &Capacity::of(Format::default()),
        );

        assert_eq!(state, TileState::Theirs);
        assert!(!state.is_clickable(), "a click here has nothing to write");
        assert_eq!(owner.as_deref(), Some("mika"), "and it says whose it is");
    }

    /// Seats outrank typed names, the same way they do in `assemble_allies`.
    /// A hero that is both must read as the teammate's, or the board offers to
    /// un-pick something it does not own.
    #[test]
    fn a_seated_pick_outranks_the_same_hero_typed_as_an_extra() {
        let seated = vec![seat("mika", ANA)];
        let (state, _) = ally_tile_state(
            ANA,
            Role::Support,
            None,
            Role::Support,
            &seated,
            &[ANA],
            &Capacity::of(Format::default()),
        );

        assert_eq!(state, TileState::Theirs, "the seat is the better evidence");
    }

    /// The whole of the rule, stated over a full board: before I have picked,
    /// my own row is about me, and a pick of mine is never refused by a cap.
    #[test]
    fn nothing_in_my_own_row_is_blocked_while_i_have_no_pick() {
        let full = room_with(Role::Support, 2);
        assert_eq!(full.free_in(Role::Support), 0, "the row really is full");

        let (state, _) = ally_tile_state(ANA, Role::Support, None, Role::Support, &[], &[], &full);
        assert_eq!(state, TileState::Free);
        assert!(state.is_clickable());
    }

    /// The reported bug. In tank mode, clicking a damage hero on the ally board
    /// took it as *my* pick and dragged my declared role to damage with it —
    /// the pick column, the pool board and the roster all following a click
    /// that only ever meant "a teammate is on this".
    #[test]
    fn a_hero_outside_my_role_is_a_teammate_rather_than_my_pick() {
        assert_eq!(
            ally_click(TRACER, Some(Role::Damage), None, Role::Tank, &[]),
            AllyClick::AddExtra,
            "clicking a dps in tank mode says nothing about what I am playing"
        );
        assert_eq!(
            ally_click(SIGMA, Some(Role::Tank), None, Role::Tank, &[]),
            AllyClick::Claim,
            "my own row is still the one click that takes a hero for me"
        );
    }

    /// The other half of the same fix, and something that was impossible
    /// before: every first click was claimed as mine, so the only way to type a
    /// teammate in was to pick myself first.
    #[test]
    fn an_out_of_role_teammate_can_be_typed_in_before_i_have_picked() {
        let (state, _) = ally_tile_state(
            TRACER,
            Role::Damage,
            None,
            Role::Tank,
            &[],
            &[],
            &Capacity::of(Format::default()),
        );

        assert_eq!(state, TileState::Free, "the dps row is open to be typed in");
        assert_eq!(
            ally_click(TRACER, Some(Role::Damage), None, Role::Tank, &[]),
            AllyClick::AddExtra
        );
    }

    /// A hero the roster cannot name has no row to be on, so it cannot be the
    /// one I am claiming. Typed in rather than guessed at, for the same reason
    /// `Seat::lock` leaves a declared role alone when it cannot name the pick.
    #[test]
    fn a_hero_the_roster_cannot_name_is_typed_in_rather_than_claimed() {
        assert_eq!(
            ally_click(HeroId(99), None, None, Role::Tank, &[]),
            AllyClick::AddExtra
        );
    }

    /// The two ladders are one rule written twice, and the whole reason they
    /// live in the same file. A tile the board draws as clickable must have a
    /// meaning, and the meaning must be the one the tile promised.
    #[test]
    fn the_click_ladder_and_the_tile_ladder_agree() {
        let room = room_with(Role::Support, 2);
        let seated = vec![seat("mika", KIRIKO)];
        let extras = [TRACER];

        // I am a tank who has not picked, so the tank row is mine to spend and
        // the support row is the one the caps have closed.
        // (hero, its role, my lock, what the tile is, what a click means)
        let cases = [
            (
                SIGMA,
                Role::Tank,
                Some(SIGMA),
                TileState::Mine,
                Some(AllyClick::TakeBack),
            ),
            (KIRIKO, Role::Support, None, TileState::Theirs, None),
            (
                TRACER,
                Role::Damage,
                None,
                TileState::Picked,
                Some(AllyClick::RemoveExtra),
            ),
            (
                REIN,
                Role::Tank,
                None,
                TileState::Free,
                Some(AllyClick::Claim),
            ),
            (
                SOJOURN,
                Role::Damage,
                None,
                TileState::Free,
                Some(AllyClick::AddExtra),
            ),
            (ANA, Role::Support, None, TileState::Blocked, None),
        ];

        for (hero, role, my_lock, want_state, want_click) in cases {
            let (state, _) =
                ally_tile_state(hero, role, my_lock, Role::Tank, &seated, &extras, &room);
            assert_eq!(state, want_state, "tile for {hero:?}");
            assert_eq!(
                state.is_clickable(),
                want_click.is_some(),
                "a tile with a meaning must be clickable, and one without must not be: {hero:?}"
            );
            if let Some(want) = want_click {
                assert_eq!(
                    ally_click(hero, Some(role), my_lock, Role::Tank, &extras),
                    want,
                    "click for {hero:?}"
                );
            }
        }
    }

    /// The case that made the old board argue with itself: in 5v5 your own
    /// unspent tank reservation closed the tank row against you.
    #[test]
    fn my_own_held_slot_does_not_block_me_out_of_my_own_role() {
        // What `ally_capacity` hands back for an unlocked tank in 5v5.
        let room = room_with(Role::Tank, 1);
        assert_eq!(room.free_in(Role::Tank), 0);

        let (state, _) = ally_tile_state(SIGMA, Role::Tank, None, Role::Tank, &[], &[], &room);
        assert_eq!(
            state,
            TileState::Free,
            "the slot is not gone, it is mine, and this click spends it"
        );
        assert_eq!(
            ally_click(SIGMA, Some(Role::Tank), None, Role::Tank, &[]),
            AllyClick::Claim,
            "and spending it is what the click does"
        );
    }

    /// The other half: once my own pick is made, the caps mean what they always
    /// did, because now they are about typing in a teammate.
    #[test]
    fn a_full_role_blocks_the_rest_of_its_row_once_i_am_locked() {
        let room = room_with(Role::Support, 2);

        let (state, _) =
            ally_tile_state(ANA, Role::Support, Some(SIGMA), Role::Tank, &[], &[], &room);
        assert_eq!(state, TileState::Blocked);
        assert!(!state.is_clickable());

        // And my own tile in that same board is still mine to take back.
        let (mine, _) =
            ally_tile_state(SIGMA, Role::Tank, Some(SIGMA), Role::Tank, &[], &[], &room);
        assert_eq!(mine, TileState::Mine);
    }

    /// A typed name the derived team had no room for still sits in the shared
    /// board. Drawing it as picked is what makes it removable — the old board
    /// drew it unlit and disabled, so it could neither be seen nor taken back.
    #[test]
    fn a_typed_name_the_team_had_no_room_for_is_still_there_to_take_back() {
        let full = room_with(Role::Support, 2);

        let (state, _) = ally_tile_state(
            ANA,
            Role::Support,
            Some(SIGMA),
            Role::Tank,
            &[],
            &[ANA],
            &full,
        );
        assert_eq!(state, TileState::Picked);
        assert!(state.is_clickable(), "or it is stuck there for good");
    }

    /// Deliberate, and worth pinning because it is the one place the rules
    /// cost a click: one tile means one thing, and taking a pick back is the
    /// meaning every board on this screen already has.
    #[test]
    fn clicking_a_typed_name_takes_it_back_rather_than_claiming_it() {
        // In damage mode, so the row is the very one a click would otherwise
        // claim: a typed name outranks that, and is still there to take back.
        let (state, _) = ally_tile_state(
            TRACER,
            Role::Damage,
            None,
            Role::Damage,
            &[],
            &[TRACER],
            &Capacity::of(Format::default()),
        );

        assert_eq!(
            state,
            TileState::Picked,
            "even with no pick of my own, the typed name is what the tile is"
        );
        assert_eq!(
            ally_click(TRACER, Some(Role::Damage), None, Role::Damage, &[TRACER]),
            AllyClick::RemoveExtra
        );
    }

    /// Drafting alone there are no other seats, so the board is only ever about
    /// me and what I have typed. This is the dominant path and it must not need
    /// a session to work.
    #[test]
    fn drafting_alone_the_board_still_tells_my_pick_from_a_typed_name() {
        let room = Capacity::of(Format::default());

        let (mine, _) = ally_tile_state(
            SIGMA,
            Role::Tank,
            Some(SIGMA),
            Role::Tank,
            &[],
            &[TRACER],
            &room,
        );
        assert_eq!(mine, TileState::Mine);

        let (typed, _) = ally_tile_state(
            TRACER,
            Role::Damage,
            Some(SIGMA),
            Role::Tank,
            &[],
            &[TRACER],
            &room,
        );
        assert_eq!(typed, TileState::Picked);
    }

    /// Two people can put the same hero up before the game stops them. My own
    /// state wins the tile — it has to, or my pick would vanish from my screen
    /// — which is exactly why the roster has to say the two of us collided.
    #[test]
    fn my_own_pick_shadows_a_teammate_who_picked_the_same_hero() {
        let seated = vec![seat("mika", SIGMA)];
        let (state, _) = ally_tile_state(
            SIGMA,
            Role::Tank,
            Some(SIGMA),
            Role::Tank,
            &seated,
            &[],
            &Capacity::of(Format::default()),
        );

        assert_eq!(state, TileState::Mine, "my own pick is never shadowed");
    }

    /// Open queue caps only bodies, so a role row fills only when the team
    /// does. The branch order is the same either way.
    #[test]
    fn open_queue_blocks_on_the_team_size_rather_than_the_role() {
        let mut room = Capacity::of(Format::new(TeamSize::FiveVFive, Queue::Open));
        for _ in 0..4 {
            room.take(Some(Role::Support));
        }

        let (state, _) =
            ally_tile_state(ANA, Role::Support, Some(SIGMA), Role::Tank, &[], &[], &room);
        assert_eq!(state, TileState::Free, "open queue fields four supports");

        room.take(Some(Role::Damage));
        let (full, _) =
            ally_tile_state(ANA, Role::Support, Some(SIGMA), Role::Tank, &[], &[], &room);
        assert_eq!(full, TileState::Blocked, "but not a sixth body");
    }

    /// 6v6 brings the second tank back, so the row that 5v5 closes stays open.
    #[test]
    fn six_v_six_leaves_room_for_a_second_ally_tank() {
        let mut room = Capacity::of(Format::new(TeamSize::SixVSix, Queue::Role));
        room.take(Some(Role::Tank));

        let (state, _) = ally_tile_state(
            REIN,
            Role::Tank,
            Some(TRACER),
            Role::Damage,
            &[],
            &[],
            &room,
        );
        assert_eq!(state, TileState::Free);
    }

    #[test]
    fn a_teammate_who_dropped_still_holds_their_pick_and_says_so() {
        let seats = vec![
            Seat {
                name: "mika".to_owned(),
                locked: Some(ANA),
                connected: false,
                ..Seat::new("mika")
            },
            Seat {
                name: "me".to_owned(),
                locked: Some(SIGMA),
                connected: true,
                ..Seat::new("me")
            },
        ];

        let picks = seated_picks(&seats, "me");
        assert_eq!(picks.len(), 1, "my own pick is not one of theirs");
        assert_eq!(picks[0].hero, ANA);
        assert!(
            picks[0].owner.contains("offline"),
            "a tile you cannot click has to account for why: {}",
            picks[0].owner
        );
    }

    #[test]
    fn a_seat_that_has_not_picked_yet_puts_nothing_on_the_board() {
        let seats = vec![Seat {
            name: "mika".to_owned(),
            connected: true,
            ..Seat::new("mika")
        }];

        assert!(seated_picks(&seats, "me").is_empty());
    }
}
