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
/// 4. **Anything at all, while I have no pick of my own.** Nothing is blocked
///    here, because this click is about to *become* my pick, and the caps are
///    a function of the role I am declaring by making it. Refusing a tile on
///    the strength of a reservation that the same click would spend is the
///    board arguing with itself — which is what a 5v5 tank saw: their own held
///    slot greyed out the entire tank row.
/// 5. Otherwise the ordinary cap, which governs typing a teammate in.
pub fn ally_tile_state(
    hero: HeroId,
    role: Role,
    my_lock: Option<HeroId>,
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
    if my_lock.is_none() || room.fits(Some(role)) {
        return (TileState::Free, None);
    }
    (TileState::Blocked, None)
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
            &seated,
            &[ANA],
            &Capacity::of(Format::default()),
        );

        assert_eq!(state, TileState::Theirs, "the seat is the better evidence");
    }

    /// The whole of the rule, stated over a full board: before I have picked,
    /// every click is about me, and a pick of mine is never refused by a cap.
    #[test]
    fn nothing_on_the_ally_board_is_blocked_while_i_have_no_pick() {
        let full = room_with(Role::Support, 2);
        assert_eq!(full.free_in(Role::Support), 0, "the row really is full");

        let (state, _) = ally_tile_state(ANA, Role::Support, None, &[], &[], &full);
        assert_eq!(state, TileState::Free);
        assert!(state.is_clickable());
    }

    /// The case that made the old board argue with itself: in 5v5 your own
    /// unspent tank reservation closed the tank row against you.
    #[test]
    fn my_own_held_slot_does_not_block_me_out_of_my_own_role() {
        // What `ally_capacity` hands back for an unlocked tank in 5v5.
        let room = room_with(Role::Tank, 1);
        assert_eq!(room.free_in(Role::Tank), 0);

        let (state, _) = ally_tile_state(SIGMA, Role::Tank, None, &[], &[], &room);
        assert_eq!(
            state,
            TileState::Free,
            "the slot is not gone, it is mine, and this click spends it"
        );
    }

    /// The other half: once my own pick is made, the caps mean what they always
    /// did, because now they are about typing in a teammate.
    #[test]
    fn a_full_role_blocks_the_rest_of_its_row_once_i_am_locked() {
        let room = room_with(Role::Support, 2);

        let (state, _) = ally_tile_state(ANA, Role::Support, Some(SIGMA), &[], &[], &room);
        assert_eq!(state, TileState::Blocked);
        assert!(!state.is_clickable());

        // And my own tile in that same board is still mine to take back.
        let (mine, _) = ally_tile_state(SIGMA, Role::Tank, Some(SIGMA), &[], &[], &room);
        assert_eq!(mine, TileState::Mine);
    }

    /// A typed name the derived team had no room for still sits in the shared
    /// board. Drawing it as picked is what makes it removable — the old board
    /// drew it unlit and disabled, so it could neither be seen nor taken back.
    #[test]
    fn a_typed_name_the_team_had_no_room_for_is_still_there_to_take_back() {
        let full = room_with(Role::Support, 2);

        let (state, _) = ally_tile_state(ANA, Role::Support, Some(SIGMA), &[], &[ANA], &full);
        assert_eq!(state, TileState::Picked);
        assert!(state.is_clickable(), "or it is stuck there for good");
    }

    /// Deliberate, and worth pinning because it is the one place the rules
    /// cost a click: one tile means one thing, and taking a pick back is the
    /// meaning every board on this screen already has.
    #[test]
    fn clicking_a_typed_name_takes_it_back_rather_than_claiming_it() {
        let (state, _) = ally_tile_state(
            TRACER,
            Role::Damage,
            None,
            &[],
            &[TRACER],
            &Capacity::of(Format::default()),
        );

        assert_eq!(
            state,
            TileState::Picked,
            "even with no pick of my own, the typed name is what the tile is"
        );
    }

    /// Drafting alone there are no other seats, so the board is only ever about
    /// me and what I have typed. This is the dominant path and it must not need
    /// a session to work.
    #[test]
    fn drafting_alone_the_board_still_tells_my_pick_from_a_typed_name() {
        let room = Capacity::of(Format::default());

        let (mine, _) = ally_tile_state(SIGMA, Role::Tank, Some(SIGMA), &[], &[TRACER], &room);
        assert_eq!(mine, TileState::Mine);

        let (typed, _) = ally_tile_state(TRACER, Role::Damage, Some(SIGMA), &[], &[TRACER], &room);
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

        let (state, _) = ally_tile_state(ANA, Role::Support, Some(SIGMA), &[], &[], &room);
        assert_eq!(state, TileState::Free, "open queue fields four supports");

        room.take(Some(Role::Damage));
        let (full, _) = ally_tile_state(ANA, Role::Support, Some(SIGMA), &[], &[], &room);
        assert_eq!(full, TileState::Blocked, "but not a sixth body");
    }

    /// 6v6 brings the second tank back, so the row that 5v5 closes stays open.
    #[test]
    fn six_v_six_leaves_room_for_a_second_ally_tank() {
        let mut room = Capacity::of(Format::new(TeamSize::SixVSix, Queue::Role));
        room.take(Some(Role::Tank));

        let (state, _) = ally_tile_state(REIN, Role::Tank, Some(TRACER), &[], &[], &room);
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
