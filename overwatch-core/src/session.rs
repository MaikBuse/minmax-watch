//! A draft shared by a whole team.
//!
//! The two-person room this grew out of synced one [`Draft`] whole, which works
//! precisely because a `Draft` is written from one seat: `locked` means "my
//! hero" and `allies` means "the rest of my team". Put five people behind that
//! one struct and both fields become contested — everyone's lock overwrites
//! everyone else's.
//!
//! So the shared state is split by who owns it. The [`Board`] — map, side, enemy
//! team — belongs to nobody and anybody: whoever reads the enemy comp first
//! types it once and it lands on every screen, which is the entire point of the
//! feature. A [`Seat`] belongs to exactly one person, and the only thing they
//! own is their own pick.
//!
//! Nobody syncs a `Draft` any more. Each client *derives* one for itself with
//! [`SessionState::draft_for`], and because that derivation is the one place the
//! feature can get its arithmetic wrong, it lives here as a pure function with
//! tests rather than inside the wasm UI where nothing can reach it.

use serde::{Deserialize, Serialize};

use crate::draft::Draft;
use crate::hero::{HeroId, Role};
use crate::map::{MapId, Side};

/// The half of the draft that everyone in a session shares.
///
/// Every field is something exactly one person needs to enter. That is the
/// whole feature: four people stop retyping the enemy team.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    #[serde(default)]
    pub map: Option<MapId>,
    /// Only meaningful when the map's mode
    /// [`has_sides`](crate::map::GameMode::has_sides).
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default)]
    pub enemies: Vec<HeroId>,
    /// Teammates who are not in the session.
    ///
    /// A session rarely covers the whole team — someone is not running the app,
    /// or you are in a group of three with two randoms. Those picks still matter
    /// to synergy, so they are entered by hand exactly as allies always were,
    /// and shared like everything else on the board.
    #[serde(default)]
    pub extra_allies: Vec<HeroId>,
}

impl Board {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an enemy pick, ignoring duplicates and refusing to overfill the
    /// team. Returns whether the board actually changed.
    pub fn add_enemy(&mut self, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) || self.enemies.len() >= Draft::TEAM_SIZE_5V5 {
            return false;
        }
        self.enemies.push(hero);
        true
    }

    pub fn remove_enemy(&mut self, hero: HeroId) {
        self.enemies.retain(|h| *h != hero);
    }

    /// Adds an enemy pick, or takes it back if it is already there. Returns
    /// whether the hero is on the enemy team afterwards.
    pub fn toggle_enemy(&mut self, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) {
            self.remove_enemy(hero);
            return false;
        }
        self.add_enemy(hero)
    }

    /// Adds an unseated ally, ignoring duplicates.
    ///
    /// Deliberately *not* capped here. The real limit is on the derived draft,
    /// where seated picks and typed ones compete for the same four slots, and
    /// capping in both places would mean a hand-typed ally silently vanishing
    /// the moment a teammate joined.
    pub fn add_extra_ally(&mut self, hero: HeroId) -> bool {
        if self.extra_allies.contains(&hero) {
            return false;
        }
        self.extra_allies.push(hero);
        true
    }

    pub fn remove_extra_ally(&mut self, hero: HeroId) {
        self.extra_allies.retain(|h| *h != hero);
    }

    pub fn toggle_extra_ally(&mut self, hero: HeroId) -> bool {
        if self.extra_allies.contains(&hero) {
            self.remove_extra_ally(hero);
            return false;
        }
        self.add_extra_ally(hero)
    }

    /// Clears the picks but keeps the map and side, matching
    /// [`Draft::clear_picks`]: one map is played across many rounds of
    /// re-picking.
    pub fn clear_picks(&mut self) {
        self.enemies.clear();
        self.extra_allies.clear();
    }

    /// Clears everything, map and side included — the "new match" reset.
    pub fn clear_all(&mut self) {
        self.clear_picks();
        self.map = None;
        self.side = None;
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_none() && self.enemies.is_empty() && self.extra_allies.is_empty()
    }
}

/// One person in the session, and the only state they own.
///
/// `id` is the client id the browser already generates for echo suppression. It
/// is client-asserted and not a credential — the server overwrites it with the
/// id of the socket the seat arrived on, which is what stops one client moving
/// another's pick. There is no authentication here and none is implied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub id: String,
    /// What the roster calls this person. Free text, and theirs to set.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: Role,
    /// Their hero, once they have locked in.
    #[serde(default)]
    pub locked: Option<HeroId>,
    /// Whether their socket is currently attached. A seat outlives its
    /// connection so that a reload does not empty a slot mid-draft.
    #[serde(default)]
    pub connected: bool,
}

impl Seat {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    /// What to show when someone has not named themselves.
    ///
    /// Falls back to the id rather than to "anonymous", because a roster of
    /// four anonymouses is worse than useless — it actively misleads about who
    /// has picked.
    pub fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// Everything a session holds: one shared board and one seat per member.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionState {
    #[serde(default)]
    pub board: Board,
    #[serde(default)]
    pub seats: Vec<Seat>,
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seat(&self, id: &str) -> Option<&Seat> {
        self.seats.iter().find(|seat| seat.id == id)
    }

    pub fn seat_mut(&mut self, id: &str) -> Option<&mut Seat> {
        self.seats.iter_mut().find(|seat| seat.id == id)
    }

    /// Inserts or replaces a seat, keeping its position in the roster so the
    /// list does not reorder itself under someone mid-draft.
    pub fn upsert_seat(&mut self, seat: Seat) {
        match self.seat_mut(&seat.id) {
            Some(existing) => *existing = seat,
            None => self.seats.push(seat),
        }
    }

    /// The scoring view for one member.
    ///
    /// This is the function the whole feature turns on. From one shared board
    /// and a roster it produces the same [`Draft`] a solo player would have
    /// typed by hand:
    ///
    /// - the board's map, side and enemies, unchanged;
    /// - `locked` from *my* seat;
    /// - `allies` from everyone *else's* locks, then the hand-typed extras.
    ///
    /// Seated picks come first deliberately. The four ally slots are contested
    /// once a team is more than half seated, and a teammate who is actually in
    /// the session and has actually locked in is better evidence than a name
    /// somebody typed. Both lists are filtered through [`Draft::add_ally`], so
    /// the duplicate and capacity rules are the same ones a solo draft obeys
    /// rather than a second implementation that can drift from them.
    ///
    /// An `me` that matches no seat is not an error: it is what a spectator, or
    /// a client whose own seat has not yet come back from the server, looks
    /// like. They get every lock as an ally and no `locked` of their own.
    pub fn draft_for(&self, me: &str) -> Draft {
        let mut draft = Draft {
            map: self.board.map,
            side: self.board.side,
            enemies: Vec::new(),
            allies: Vec::new(),
            locked: self.seat(me).and_then(|seat| seat.locked),
        };

        for enemy in &self.board.enemies {
            draft.add_enemy(*enemy);
        }

        // `add_ally` already refuses a hero equal to `locked`, which is what
        // keeps my own pick from also counting as one of my allies.
        let seated = self
            .seats
            .iter()
            .filter(|seat| seat.id != me)
            .filter_map(|seat| seat.locked);
        for ally in seated.chain(self.board.extra_allies.iter().copied()) {
            draft.add_ally(ally);
        }

        draft
    }

    /// Whether the session holds anything worth adopting.
    ///
    /// Used to decide if an incoming snapshot should overwrite what is already
    /// on screen: joining a stale, empty session must not wipe a draft that is
    /// already in progress.
    pub fn is_empty(&self) -> bool {
        self.board.is_empty() && self.seats.iter().all(|seat| seat.locked.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seated(id: &str, locked: Option<u16>) -> Seat {
        Seat {
            id: id.to_owned(),
            name: id.to_owned(),
            role: Role::Tank,
            locked: locked.map(HeroId),
            connected: true,
        }
    }

    fn session(seats: Vec<Seat>) -> SessionState {
        SessionState {
            board: Board::new(),
            seats,
        }
    }

    #[test]
    fn a_members_allies_are_the_other_seats_locks() {
        let state = session(vec![
            seated("me", Some(1)),
            seated("mika", Some(2)),
            seated("sam", Some(3)),
        ]);

        let draft = state.draft_for("me");
        assert_eq!(draft.locked, Some(HeroId(1)), "my own seat is my lock");
        assert_eq!(draft.allies, vec![HeroId(2), HeroId(3)]);
    }

    #[test]
    fn my_own_lock_is_never_also_one_of_my_allies() {
        let state = session(vec![seated("me", Some(7)), seated("mika", Some(9))]);

        let draft = state.draft_for("me");
        assert!(
            !draft.allies.contains(&HeroId(7)),
            "your own pick belongs in `locked` and nowhere else"
        );
        assert_eq!(draft.allies, vec![HeroId(9)]);
    }

    #[test]
    fn a_seat_that_has_not_picked_yet_takes_up_no_ally_slot() {
        let state = session(vec![
            seated("me", Some(1)),
            seated("mika", None),
            seated("sam", Some(3)),
        ]);

        assert_eq!(state.draft_for("me").allies, vec![HeroId(3)]);
    }

    #[test]
    fn extra_allies_fill_in_behind_the_seated_ones() {
        let mut state = session(vec![seated("me", Some(1)), seated("mika", Some(2))]);
        state.board.extra_allies = vec![HeroId(5), HeroId(6)];

        assert_eq!(
            state.draft_for("me").allies,
            vec![HeroId(2), HeroId(5), HeroId(6)],
            "a teammate who actually locked in outranks a typed-in name"
        );
    }

    /// The four ally slots are contested once seats and typed names together
    /// exceed them. The seated picks are the ones that survive.
    #[test]
    fn a_full_team_stops_taking_allies() {
        let mut state = session(vec![
            seated("me", Some(1)),
            seated("a", Some(2)),
            seated("b", Some(3)),
            seated("c", Some(4)),
            seated("d", Some(5)),
        ]);
        state.board.extra_allies = vec![HeroId(9)];

        let draft = state.draft_for("me");
        assert_eq!(
            draft.allies,
            vec![HeroId(2), HeroId(3), HeroId(4), HeroId(5)],
            "four allies plus yourself is a full 5v5 team"
        );
        assert!(
            !draft.allies.contains(&HeroId(9)),
            "the typed-in name is the one that loses the contested slot"
        );
    }

    #[test]
    fn a_duplicate_pick_appears_once() {
        let mut state = session(vec![seated("me", Some(1)), seated("mika", Some(4))]);
        // Two people can put the same hero up before the game stops them, and
        // the board can name someone a seat already covers.
        state.board.extra_allies = vec![HeroId(4)];

        assert_eq!(state.draft_for("me").allies, vec![HeroId(4)]);
    }

    #[test]
    fn the_board_reaches_every_member_unchanged() {
        let mut state = session(vec![seated("me", None), seated("mika", None)]);
        state.board.map = Some(MapId(3));
        state.board.side = Some(Side::Attack);
        state.board.enemies = vec![HeroId(10), HeroId(11)];

        for who in ["me", "mika"] {
            let draft = state.draft_for(who);
            assert_eq!(draft.map, Some(MapId(3)));
            assert_eq!(draft.side, Some(Side::Attack));
            assert_eq!(draft.enemies, vec![HeroId(10), HeroId(11)]);
        }
    }

    /// The feature must cost nothing when nobody else is there. A session of
    /// one has to score exactly like the single-player app it replaces.
    #[test]
    fn a_session_of_one_scores_exactly_like_a_solo_draft() {
        let mut state = session(vec![seated("me", Some(2))]);
        state.board.map = Some(MapId(1));
        state.board.enemies = vec![HeroId(8)];
        state.board.extra_allies = vec![HeroId(3), HeroId(4)];

        let mut expected = Draft::new();
        expected.map = Some(MapId(1));
        expected.locked = Some(HeroId(2));
        expected.add_enemy(HeroId(8));
        expected.add_ally(HeroId(3));
        expected.add_ally(HeroId(4));

        assert_eq!(state.draft_for("me"), expected);
    }

    #[test]
    fn a_stranger_sees_every_lock_as_an_ally_and_none_as_their_own() {
        let state = session(vec![seated("a", Some(1)), seated("b", Some(2))]);

        let draft = state.draft_for("nobody");
        assert_eq!(draft.locked, None);
        assert_eq!(draft.allies, vec![HeroId(1), HeroId(2)]);
    }

    #[test]
    fn the_enemy_team_is_capped_the_way_a_solo_draft_caps_it() {
        let mut state = session(vec![seated("me", None)]);
        state.board.enemies = (1..=8).map(HeroId).collect();

        assert_eq!(
            state.draft_for("me").enemies.len(),
            Draft::TEAM_SIZE_5V5,
            "a board carrying junk must not produce an illegal draft"
        );
    }

    #[test]
    fn upserting_a_seat_keeps_its_place_in_the_roster() {
        let mut state = session(vec![
            seated("a", None),
            seated("b", None),
            seated("c", None),
        ]);
        state.upsert_seat(seated("b", Some(4)));

        let ids: Vec<&str> = state.seats.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "the roster must not reorder");
        assert_eq!(state.seat("b").and_then(|s| s.locked), Some(HeroId(4)));
    }

    #[test]
    fn an_unnamed_seat_still_has_something_to_call_it() {
        assert_eq!(Seat::new("c0ffee").display_name(), "c0ffee");
        assert_eq!(seated("mika", None).display_name(), "mika");

        let blank = Seat {
            name: "   ".to_owned(),
            ..Seat::new("c0ffee")
        };
        assert_eq!(blank.display_name(), "c0ffee", "whitespace is not a name");
    }

    #[test]
    fn a_session_nobody_has_touched_is_empty() {
        let mut state = session(vec![seated("me", None)]);
        assert!(state.is_empty(), "seats alone are not state worth adopting");

        state.board.map = Some(MapId(1));
        assert!(!state.is_empty());
    }

    #[test]
    fn the_board_toggles_the_way_the_boards_it_replaces_did() {
        let mut board = Board::new();
        assert!(board.toggle_enemy(HeroId(1)));
        assert!(
            !board.toggle_enemy(HeroId(1)),
            "a second click takes it back"
        );
        assert!(board.enemies.is_empty());

        assert!(board.toggle_extra_ally(HeroId(2)));
        assert!(!board.toggle_extra_ally(HeroId(2)));
        assert!(board.extra_allies.is_empty());
    }

    #[test]
    fn clearing_picks_keeps_the_map() {
        let mut board = Board::new();
        board.map = Some(MapId(2));
        board.side = Some(Side::Defend);
        board.enemies = vec![HeroId(1)];
        board.extra_allies = vec![HeroId(2)];

        board.clear_picks();
        assert_eq!(board.map, Some(MapId(2)), "one map, many rounds");
        assert_eq!(board.side, Some(Side::Defend));
        assert!(board.enemies.is_empty());
        assert!(board.extra_allies.is_empty());

        board.clear_all();
        assert_eq!(board.map, None);
        assert_eq!(board.side, None);
    }

    /// Both screens in a session are separate installs and can be on different
    /// builds mid-draft, so a field this end has not heard of has to arrive as
    /// something to ignore rather than something to reject. A parse error here
    /// is a session that silently stops syncing while everyone keeps picking.
    #[test]
    fn state_from_a_newer_client_still_parses() {
        let newer = r#"{
            "board": {
                "map": 3,
                "side": "attack",
                "enemies": [1, 2],
                "extra_allies": [4],
                "bans": [9]
            },
            "seats": [
                {"id": "a", "name": "era", "role": "tank", "locked": 5,
                 "connected": true, "ready": false}
            ],
            "phase": "picking"
        }"#;

        let state: SessionState =
            serde_json::from_str(newer).expect("an unknown field must not break the session");
        assert_eq!(state.board.enemies, vec![HeroId(1), HeroId(2)]);
        assert_eq!(state.seats.len(), 1);
        assert_eq!(state.seats[0].locked, Some(HeroId(5)));
    }

    /// The other direction: a seat written by a client that predates a field
    /// this build now expects has to load with the rest of it intact.
    #[test]
    fn a_seat_missing_every_optional_field_still_parses() {
        let sparse = r#"{"id": "c1234"}"#;

        let seat: Seat = serde_json::from_str(sparse).expect("only the id is required");
        assert_eq!(seat.id, "c1234");
        assert_eq!(seat.locked, None);
        assert!(!seat.connected);
    }

    #[test]
    fn a_session_round_trips_as_json() {
        let mut state = session(vec![seated("me", Some(1)), seated("mika", None)]);
        state.board.map = Some(MapId(4));
        state.board.enemies = vec![HeroId(7)];

        let json = serde_json::to_string(&state).expect("serialises");
        let back: SessionState = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, state);
    }
}
