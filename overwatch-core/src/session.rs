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

use crate::dataset::Dataset;
use crate::draft::{capacity_after, fit_to_format, role_of, Draft};
use crate::format::{Capacity, Format};
use crate::hero::{HeroId, Role};
use crate::map::{MapId, Side};

/// The half of the draft that everyone in a session shares.
///
/// Every field is something exactly one person needs to enter. That is the
/// whole feature: four people stop retyping the enemy team.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Board {
    /// Which queue the room is in. Shared for the same reason the map is:
    /// everybody in a session is in one lobby, so one person setting it has set
    /// it for all of them.
    ///
    /// Absent from a board written before formats existed, and
    /// [`Format::default`] — 5v5 role queue — is what such a board meant. That
    /// is a compatibility reading, not a claim about which queue people play.
    #[serde(default)]
    pub format: Format,
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

    /// What the enemy team can still take, per role.
    ///
    /// The board the screen draws from, so that a tile is disabled exactly when
    /// [`Self::add_enemy`] would refuse it.
    pub fn enemy_capacity(&self, dataset: &Dataset) -> Capacity {
        capacity_after(dataset, self.format, &self.enemies)
    }

    /// Adds an enemy pick, ignoring duplicates and refusing to overfill the team
    /// or any one of its roles. Returns whether the board actually changed.
    ///
    /// Unlike the ally side this is capped here, at entry. An enemy pick has
    /// nowhere else to be decided: there are no seats holding enemy slots open
    /// and nothing arrives about them from anybody else, so what is typed is the
    /// whole story.
    pub fn add_enemy(&mut self, dataset: &Dataset, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) {
            return false;
        }
        if !self.enemy_capacity(dataset).fits(role_of(dataset, hero)) {
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
    pub fn toggle_enemy(&mut self, dataset: &Dataset, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) {
            self.remove_enemy(hero);
            return false;
        }
        self.add_enemy(dataset, hero)
    }

    /// Switches the format, dropping the picks the new one has no room for.
    ///
    /// The pruning is not tidiness. The boards draw their picked tiles from the
    /// *derived* draft, so a pick left here that the derivation refuses would
    /// render as an unlit tile that nonetheless un-picks when clicked — state
    /// you cannot see that still eats a click. Shrinking the format is a
    /// deliberate act, so the loss is attributable and happens under the cursor;
    /// growing one never drops anything.
    pub fn set_format(&mut self, dataset: &Dataset, format: Format) {
        self.format = format;
        self.enemies = fit_to_format(dataset, format, &self.enemies);
        // Best-effort for the typed allies: this ignores the seats, so it can
        // only ever be too generous, and the derived draft stays the authority
        // on which of them actually land.
        self.extra_allies = fit_to_format(dataset, format, &self.extra_allies);
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

    /// Clears the picks but keeps the map, the side and the format, matching
    /// [`Draft::clear_picks`]: one map is played across many rounds of
    /// re-picking, and the queue outlasts even the match.
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

    /// Whether the board holds any of the draft itself.
    ///
    /// Deliberately blind to the format: a room that has only said "6v6" holds
    /// nothing worth adopting over a draft already in progress, which is what
    /// this is asked. The format still has to reach a joiner — see the snapshot
    /// handling in the client, which takes it separately for exactly this
    /// reason.
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

    /// Takes `hero`, moving the declared role to the one that hero plays.
    ///
    /// The role follows the pick rather than the other way round because the
    /// pick is the fact: a hero locked in game is what you are playing whatever
    /// you queued as. Keeping the two in step is what makes a seat cost exactly
    /// one slot of `role` whether or not it has locked — see
    /// [`SessionState::assemble_allies`], which charges a lock its hero's role
    /// and an empty seat its declared one, and can only agree with itself if
    /// they are the same role.
    ///
    /// A hero the dataset cannot name a role for leaves the role alone. There
    /// is nothing to follow, and guessing would move the slot this seat holds
    /// on the strength of a dataset mismatch.
    /// Returns the role the seat holds afterwards, which is what a caller
    /// keeping its own copy of "what am I playing" needs in order to follow.
    pub fn lock(&mut self, dataset: &Dataset, hero: HeroId) -> Role {
        self.locked = Some(hero);
        if let Some(role) = role_of(dataset, hero) {
            self.role = role;
        }
        self.role
    }

    /// Declares `role`, dropping a pick the new role cannot hold. Returns
    /// whether a lock was actually given up.
    ///
    /// The counterpart to [`Self::lock`], and the same invariant seen from the
    /// other side. Dropping the pick rather than keeping it is the one place
    /// this file takes something back, and it is deliberate for the reason
    /// [`Board::set_format`] gives: the loss is attributable and happens under
    /// the cursor. The alternative — a seat declared dps while locked on a tank
    /// — reads as a tank to the derivation and as a dps to the roster, and
    /// makes the pick panel argue about swapping away from your own hero.
    pub fn set_role(&mut self, dataset: &Dataset, role: Role) -> bool {
        self.role = role;
        let stale = self
            .locked
            .is_some_and(|hero| role_of(dataset, hero) != Some(role));
        if stale {
            self.locked = None;
        }
        stale
    }

    /// Gives the pick back, leaving the declared role alone.
    ///
    /// Un-picking says nothing about what you are queued as, so the slot this
    /// seat holds does not move — it goes back to being a reservation in the
    /// same role.
    pub fn unlock(&mut self) {
        self.locked = None;
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

    /// Drops a seat and the slot it was holding. Returns whether there was one.
    ///
    /// For somebody saying they are done, which is a different event from their
    /// socket dropping: a seat outlives its connection precisely so a reload
    /// does not empty a slot mid-draft, but somebody who has actually left is
    /// not coming back to fill it, and a reservation nobody will ever spend is
    /// one the rest of the team should get back.
    pub fn remove_seat(&mut self, id: &str) -> bool {
        let before = self.seats.len();
        self.seats.retain(|seat| seat.id != id);
        self.seats.len() != before
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
    /// Seated picks come first deliberately. The ally slots are contested once a
    /// team is more than half seated, and a teammate who is actually in the
    /// session and has actually locked in is better evidence than a name
    /// somebody typed. Both lists are filtered through [`Draft::add_ally`], so
    /// the duplicate and size rules are the same ones a solo draft obeys rather
    /// than a second implementation that can drift from them; the per-role caps
    /// come from [`Self::assemble_allies`], which is the only thing that can see
    /// them.
    ///
    /// An `me` that matches no seat is not an error: it is what a spectator, or
    /// a client whose own seat has not yet come back from the server, looks
    /// like. They get every lock as an ally and no `locked` of their own, and
    /// they hold no slot open, since there is no seat of theirs to hold one for.
    pub fn draft_for(&self, dataset: &Dataset, me: &str) -> Draft {
        let (allies, _) = self.assemble_allies(dataset, me);

        Draft {
            format: self.board.format,
            map: self.board.map,
            side: self.board.side,
            // Trimmed rather than copied. A board arrives from whoever typed it,
            // which may be a client in a bigger format than this one.
            enemies: fit_to_format(dataset, self.board.format, &self.board.enemies),
            allies,
            locked: self.seat(me).and_then(|seat| seat.locked),
        }
    }

    /// Room left on my team for one more hand-typed ally, per role.
    ///
    /// What the ally board disables its tiles from. It comes out of the same
    /// pass as the ally list itself, so the tiles you can click and the picks
    /// that reach the scorer cannot disagree.
    pub fn ally_capacity(&self, dataset: &Dataset, me: &str) -> Capacity {
        self.assemble_allies(dataset, me).1
    }

    /// My team as it stands, and the room left for one more typed ally.
    ///
    /// The one place the ally arithmetic lives, and the order of it is the rule:
    ///
    /// 1. **Locked seats, mine included.** A hero picked in game costs its own
    ///    role's slot whatever anybody declared, falling back to the declared
    ///    role only when the roster cannot name the hero's. These are never
    ///    gated — a teammate's pick is a fact, and a cap that could refuse it
    ///    would delete a hero from a team that really has it.
    /// 2. **Typed extras, in entry order**, each refused if its role is full.
    ///    This is the only gate, so it can only ever turn away the newest.
    /// 3. **Reservations**: every seat that has *not* locked, mine included,
    ///    holds a slot in the role it declared. This is what closes the ally
    ///    tank row in 5v5 when you are the tank, and what stops a typed name
    ///    taking the slot a seated teammate is about to fill.
    ///
    /// Reservations come last, and spend saturating, so that a role switch
    /// mid-draft can never evict a pick that already landed: the reservation
    /// that no longer has room is simply absorbed and the row reads as full.
    /// Caps gate what can be added; they never take anything back. The same
    /// saturation is why an over-full room — six people in a 5v5 lobby — reads
    /// as full rather than going wrong.
    ///
    /// A seat holds its slot whether or not it is `connected`. A seat already
    /// outlives its socket so that a reload does not empty a slot mid-draft, and
    /// handing its slot to somebody else's typing the moment the wifi blinks
    /// would only take it back again.
    fn assemble_allies(&self, dataset: &Dataset, me: &str) -> (Vec<HeroId>, Capacity) {
        let mut draft = Draft::in_format(self.board.format);
        draft.locked = self.seat(me).and_then(|seat| seat.locked);
        let mut room = Capacity::of(self.board.format);

        for seat in &self.seats {
            let Some(hero) = seat.locked else { continue };
            // `add_ally` refuses a hero equal to `locked`, which is what keeps
            // my own pick from also counting as one of my allies — and what
            // stops two people who put the same hero up before the game stopped
            // them being charged for it twice.
            let landed = if seat.id == me {
                true
            } else {
                draft.add_ally(hero)
            };
            if landed {
                // Falling back to the declared role is what keeps a seat
                // costing one slot of `seat.role` either way. A hero the roster
                // cannot name would otherwise spend a body and no role column,
                // so locking one would quietly hand the seat's own role slot
                // back to the team — the reservation below no longer runs.
                // A hero it *can* name still overrides the declaration: the
                // pick is the fact.
                room.take(role_of(dataset, hero).or(Some(seat.role)));
            }
        }

        for hero in &self.board.extra_allies {
            let role = role_of(dataset, *hero);
            if room.fits(role) && draft.add_ally(*hero) {
                room.take(role);
            }
        }

        for seat in &self.seats {
            if seat.locked.is_none() {
                room.take(Some(seat.role));
            }
        }

        (draft.allies, room)
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
    use crate::dataset::Dataset;
    use crate::fixture::{
        self, ANA, ASHE, KIRIKO, LUCIO, NOBODY, REINHARDT, SIGMA, SOJOURN, TRACER, WINSTON,
    };
    use crate::format::{Queue, TeamSize};

    fn seated(id: &str, role: Role, locked: Option<HeroId>) -> Seat {
        Seat {
            id: id.to_owned(),
            name: id.to_owned(),
            role,
            locked,
            connected: true,
        }
    }

    fn session(seats: Vec<Seat>) -> SessionState {
        SessionState {
            board: Board::new(),
            seats,
        }
    }

    fn six_v_six() -> Format {
        Format::new(TeamSize::SixVSix, Queue::Role)
    }

    /// The room left for one more typed ally, as a triple in `Role::ALL` order.
    /// Reads as the shape of the team, which is what these tests are about.
    fn room(state: &SessionState, ds: &Dataset, me: &str) -> [usize; 3] {
        let room = state.ally_capacity(ds, me);
        [
            room.free_in(Role::Tank),
            room.free_in(Role::Damage),
            room.free_in(Role::Support),
        ]
    }

    #[test]
    fn a_members_allies_are_the_other_seats_locks() {
        let ds = fixture::dataset();
        let state = session(vec![
            seated("me", Role::Tank, Some(SIGMA)),
            seated("mika", Role::Damage, Some(TRACER)),
            seated("sam", Role::Support, Some(ANA)),
        ]);

        let draft = state.draft_for(&ds, "me");
        assert_eq!(draft.locked, Some(SIGMA), "my own seat is my lock");
        assert_eq!(draft.allies, vec![TRACER, ANA]);
    }

    #[test]
    fn my_own_lock_is_never_also_one_of_my_allies() {
        let ds = fixture::dataset();
        let state = session(vec![
            seated("me", Role::Tank, Some(SIGMA)),
            seated("mika", Role::Support, Some(ANA)),
        ]);

        let draft = state.draft_for(&ds, "me");
        assert!(
            !draft.allies.contains(&SIGMA),
            "your own pick belongs in `locked` and nowhere else"
        );
        assert_eq!(draft.allies, vec![ANA]);
    }

    #[test]
    fn a_seat_that_has_not_picked_yet_is_not_one_of_your_allies() {
        let ds = fixture::dataset();
        let state = session(vec![
            seated("me", Role::Tank, Some(SIGMA)),
            seated("mika", Role::Damage, None),
            seated("sam", Role::Support, Some(ANA)),
        ]);

        assert_eq!(state.draft_for(&ds, "me").allies, vec![ANA]);
    }

    #[test]
    fn extra_allies_fill_in_behind_the_seated_ones() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Tank, Some(SIGMA)),
            seated("mika", Role::Damage, Some(TRACER)),
        ]);
        state.board.extra_allies = vec![ANA, LUCIO];

        assert_eq!(
            state.draft_for(&ds, "me").allies,
            vec![TRACER, ANA, LUCIO],
            "a teammate who actually locked in outranks a typed-in name"
        );
    }

    /// The ally slots are contested once seats and typed names together exceed
    /// them. The seated picks are the ones that survive.
    #[test]
    fn a_full_team_stops_taking_allies() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Tank, Some(REINHARDT)),
            seated("a", Role::Damage, Some(TRACER)),
            seated("b", Role::Damage, Some(SOJOURN)),
            seated("c", Role::Support, Some(ANA)),
            seated("d", Role::Support, Some(LUCIO)),
        ]);
        state.board.extra_allies = vec![KIRIKO];

        let draft = state.draft_for(&ds, "me");
        assert_eq!(
            draft.allies,
            vec![TRACER, SOJOURN, ANA, LUCIO],
            "four allies plus yourself is a full 5v5 team"
        );
        assert!(
            !draft.allies.contains(&KIRIKO),
            "the typed-in name is the one that loses the contested slot"
        );
    }

    #[test]
    fn a_duplicate_pick_appears_once() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Tank, Some(SIGMA)),
            seated("mika", Role::Support, Some(ANA)),
        ]);
        // Two people can put the same hero up before the game stops them, and
        // the board can name someone a seat already covers.
        state.board.extra_allies = vec![ANA];

        assert_eq!(state.draft_for(&ds, "me").allies, vec![ANA]);
        assert_eq!(
            room(&state, &ds, "me")[Role::Support.index()],
            1,
            "and it costs the team one support, not two"
        );
    }

    #[test]
    fn the_board_reaches_every_member_unchanged() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Tank, None),
            seated("mika", Role::Damage, None),
        ]);
        state.board.format = six_v_six();
        state.board.map = Some(fixture::KINGS_ROW);
        state.board.side = Some(Side::Attack);
        state.board.enemies = vec![SIGMA, TRACER];

        for who in ["me", "mika"] {
            let draft = state.draft_for(&ds, who);
            assert_eq!(draft.format, six_v_six(), "one lobby, one format");
            assert_eq!(draft.map, Some(fixture::KINGS_ROW));
            assert_eq!(draft.side, Some(Side::Attack));
            assert_eq!(draft.enemies, vec![SIGMA, TRACER]);
        }
    }

    /// The feature must cost nothing when nobody else is there. A session of
    /// one has to score exactly like the single-player app it replaces.
    #[test]
    fn a_session_of_one_scores_exactly_like_a_solo_draft() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, Some(SIGMA))]);
        state.board.map = Some(fixture::KINGS_ROW);
        state.board.enemies = vec![TRACER];
        state.board.extra_allies = vec![ANA, LUCIO];

        let mut expected = Draft::new();
        expected.map = Some(fixture::KINGS_ROW);
        expected.locked = Some(SIGMA);
        expected.add_enemy(TRACER);
        expected.add_ally(ANA);
        expected.add_ally(LUCIO);

        assert_eq!(state.draft_for(&ds, "me"), expected);
    }

    /// The compatibility contract for the whole change: one unlocked seat holds
    /// exactly one slot, so the count that reaches the scorer is the four it has
    /// always been.
    #[test]
    fn a_session_of_one_caps_allies_exactly_as_it_always_did() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, None)]);
        state.board.extra_allies = vec![TRACER, SOJOURN, ANA, LUCIO, KIRIKO];

        assert_eq!(
            state.draft_for(&ds, "me").allies,
            vec![TRACER, SOJOURN, ANA, LUCIO],
            "your own slot stays free, and the fifth name has nowhere to go"
        );
    }

    #[test]
    fn a_stranger_sees_every_lock_as_an_ally_and_none_as_their_own() {
        let ds = fixture::dataset();
        let state = session(vec![
            seated("a", Role::Tank, Some(SIGMA)),
            seated("b", Role::Damage, Some(TRACER)),
        ]);

        let draft = state.draft_for(&ds, "nobody");
        assert_eq!(draft.locked, None);
        assert_eq!(draft.allies, vec![SIGMA, TRACER]);
    }

    #[test]
    fn the_enemy_team_is_capped_the_way_a_solo_draft_caps_it() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, None)]);
        state.board.enemies = vec![SIGMA, WINSTON, TRACER, SOJOURN, ASHE, ANA, LUCIO, KIRIKO];

        assert_eq!(
            state.draft_for(&ds, "me").enemies,
            vec![SIGMA, TRACER, SOJOURN, ANA, LUCIO],
            "a board carrying junk must not produce an illegal draft"
        );
    }

    // --- the room your own team has left ----------------------------------

    /// The rule the feature was asked for: in 5v5 role queue you are the tank,
    /// so no teammate of yours is.
    #[test]
    fn my_own_role_holds_a_slot_open_on_my_team() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, None)]);

        assert_eq!(room(&state, &ds, "me"), [0, 2, 2]);

        // The same seat in another pick mode holds another slot.
        state.seats[0].role = Role::Damage;
        assert_eq!(room(&state, &ds, "me"), [1, 1, 2]);
    }

    #[test]
    fn a_teammate_who_has_not_picked_still_holds_their_role_open() {
        let ds = fixture::dataset();
        let state = session(vec![
            seated("me", Role::Tank, None),
            seated("mika", Role::Support, None),
        ]);

        assert_eq!(
            room(&state, &ds, "me"),
            [0, 2, 1],
            "the support they are about to pick is not a slot you can type into"
        );
    }

    /// A seat whose declaration disagrees with its pick, which [`Seat::lock`]
    /// no longer produces on this build. It still arrives: the server holds no
    /// dataset and so cannot repair one, `publish_legacy_draft` keeps the old
    /// declaration while writing a new lock, and a client on an older build
    /// never had the rule. The hero is the fact, so the hero is what it spends.
    #[test]
    fn a_teammate_who_has_locked_spends_their_hero_and_not_their_declared_role() {
        let ds = fixture::dataset();
        // Declared tank, actually picked a support: it happens, and the hero is
        // the fact.
        let state = session(vec![
            seated("me", Role::Damage, None),
            seated("mika", Role::Tank, Some(ANA)),
        ]);

        assert_eq!(
            room(&state, &ds, "me"),
            [1, 1, 1],
            "their tank slot went back to the team; their support slot went"
        );
    }

    /// A cap must never delete a hero from a team that really has it.
    #[test]
    fn a_seated_lock_is_never_refused_by_a_cap() {
        let ds = fixture::dataset();
        let state = session(vec![
            seated("me", Role::Tank, Some(REINHARDT)),
            seated("mika", Role::Tank, Some(SIGMA)),
        ]);

        let draft = state.draft_for(&ds, "me");
        assert_eq!(
            draft.allies,
            vec![SIGMA],
            "two tanks in 5v5 cannot happen, but if it has, it has"
        );
        assert_eq!(room(&state, &ds, "me"), [0, 2, 2], "and it saturates");
    }

    /// Caps gate what can be added. They never take back something that landed.
    #[test]
    fn a_pick_already_entered_survives_a_role_switch_beside_it() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Damage, None)]);
        state.board.extra_allies = vec![SIGMA];

        assert_eq!(state.draft_for(&ds, "me").allies, vec![SIGMA]);

        // I switch to tank with a tank already typed on my team.
        state.seats[0].role = Role::Tank;
        assert_eq!(
            state.draft_for(&ds, "me").allies,
            vec![SIGMA],
            "the pick stays; my reservation is the thing that gives way"
        );
        assert_eq!(
            room(&state, &ds, "me")[Role::Tank.index()],
            0,
            "and the row simply reads as full"
        );
    }

    #[test]
    fn a_typed_ally_loses_the_contested_slot_to_a_seated_lock_in_its_own_role() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Tank, None),
            seated("mika", Role::Support, Some(ANA)),
        ]);
        state.board.extra_allies = vec![LUCIO, KIRIKO];

        let draft = state.draft_for(&ds, "me");
        assert_eq!(draft.allies, vec![ANA, LUCIO]);
        assert!(
            !draft.allies.contains(&KIRIKO),
            "a team fields two supports, and both are spoken for"
        );
    }

    #[test]
    fn six_v_six_takes_a_second_ally_tank_and_5v5_does_not() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Damage, None)]);
        state.board.extra_allies = vec![SIGMA, WINSTON];

        assert_eq!(state.draft_for(&ds, "me").allies, vec![SIGMA]);

        state.board.format = six_v_six();
        assert_eq!(state.draft_for(&ds, "me").allies, vec![SIGMA, WINSTON]);
    }

    #[test]
    fn open_queue_fields_whatever_it_likes_up_to_the_team_size() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, None)]);
        state.board.extra_allies = vec![ANA, LUCIO, KIRIKO, TRACER];

        assert_eq!(
            state.draft_for(&ds, "me").allies,
            vec![ANA, LUCIO, TRACER],
            "role queue fields two supports"
        );

        state.board.format = Format::new(TeamSize::FiveVFive, Queue::Open);
        assert_eq!(
            state.draft_for(&ds, "me").allies,
            vec![ANA, LUCIO, KIRIKO, TRACER],
            "open queue only counts bodies"
        );
    }

    /// The invariant that keeps the screen honest: every pick the board offers
    /// actually lands. Checked over the whole roster, because it is the
    /// disagreement that is the bug, not any one hero.
    ///
    /// A tile you can click for a pick that then quietly vanishes is the failure
    /// this rules out. The converse is deliberately *not* asserted — see below.
    #[test]
    fn every_pick_the_board_offers_actually_lands() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Tank, None),
            seated("mika", Role::Support, Some(ANA)),
        ]);
        state.board.extra_allies = vec![LUCIO];

        let offered = state.ally_capacity(&ds, "me");
        for hero in (0..ds.hero_count()).map(|i| HeroId(i as u16)) {
            if hero == ANA || hero == LUCIO {
                continue; // already on the team; the duplicate rule owns these
            }
            let role = ds.hero(hero).map(|entry| entry.role).ok();

            let mut with_hero = state.clone();
            with_hero.board.extra_allies.push(hero);
            let landed = with_hero.draft_for(&ds, "me").allies.contains(&hero);

            assert!(
                !offered.fits(role) || landed,
                "the board offered {hero:?} and the draft dropped it"
            );
        }
    }

    /// The one direction the two are allowed to differ, and it is the
    /// no-eviction rule seen from the other side.
    ///
    /// The room the board offers holds a slot for every seat that has not
    /// picked, so it is stricter than the derivation, which charges reservations
    /// last and will not throw away a pick that is already there. Nothing can
    /// reach that gap by clicking — the tile is disabled on every screen in the
    /// session, because the reservation is charged for all of the seats, not
    /// just yours. It is reachable by switching your role with the pick already
    /// entered, which is exactly when keeping the pick is the right answer.
    #[test]
    fn a_held_slot_closes_the_board_without_evicting_what_is_already_there() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, None)]);

        assert_eq!(
            room(&state, &ds, "me")[Role::Tank.index()],
            0,
            "my own seat holds the tank slot, so the board offers none"
        );

        state.board.extra_allies = vec![SIGMA];
        assert_eq!(
            state.draft_for(&ds, "me").allies,
            vec![SIGMA],
            "but a tank that got there anyway stays on the team"
        );
    }

    #[test]
    fn the_enemy_board_refuses_exactly_what_its_capacity_says() {
        let ds = fixture::dataset();
        let mut board = Board::new();
        board.enemies = vec![SIGMA, TRACER];

        let offered = board.enemy_capacity(&ds);
        for hero in (0..ds.hero_count()).map(|i| HeroId(i as u16)) {
            if board.enemies.contains(&hero) {
                continue;
            }
            let role = ds.hero(hero).map(|entry| entry.role).ok();
            assert_eq!(
                offered.fits(role),
                board.clone().add_enemy(&ds, hero),
                "the board and its own capacity disagree about {hero:?}"
            );
        }
    }

    #[test]
    fn more_seats_than_slots_does_not_underflow() {
        let ds = fixture::dataset();
        let state = session(
            ["a", "b", "c", "d", "e", "f"]
                .into_iter()
                .map(|id| seated(id, Role::Tank, None))
                .collect(),
        );

        assert_eq!(
            room(&state, &ds, "a"),
            [0, 0, 0],
            "an over-full room is full"
        );
        assert!(state.ally_capacity(&ds, "a").is_full());
    }

    // --- the board --------------------------------------------------------

    #[test]
    fn upserting_a_seat_keeps_its_place_in_the_roster() {
        let mut state = session(vec![
            seated("a", Role::Tank, None),
            seated("b", Role::Damage, None),
            seated("c", Role::Support, None),
        ]);
        state.upsert_seat(seated("b", Role::Damage, Some(TRACER)));

        let ids: Vec<&str> = state.seats.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "the roster must not reorder");
        assert_eq!(state.seat("b").and_then(|s| s.locked), Some(TRACER));
    }

    // --- a seat's pick and the role it holds --------------------------------

    /// The invariant, from the picking side: what you locked is what you are
    /// playing, whatever you queued as.
    #[test]
    fn locking_a_hero_of_another_role_moves_the_seat_to_that_role() {
        let ds = fixture::dataset();
        let mut seat = seated("me", Role::Damage, None);

        seat.lock(&ds, REINHARDT);
        assert_eq!(seat.locked, Some(REINHARDT));
        assert_eq!(seat.role, Role::Tank, "the role follows the pick");
    }

    #[test]
    fn locking_a_hero_of_your_own_role_leaves_the_role_alone() {
        let ds = fixture::dataset();
        let mut seat = seated("me", Role::Support, None);

        seat.lock(&ds, ANA);
        assert_eq!(seat.role, Role::Support);
    }

    /// The invariant from the other side. A pick the new role cannot hold is
    /// the one thing this file takes back, and it happens under the cursor.
    #[test]
    fn switching_role_drops_a_pick_the_new_role_cannot_hold() {
        let ds = fixture::dataset();
        let mut seat = seated("me", Role::Tank, Some(SIGMA));

        assert!(seat.set_role(&ds, Role::Damage), "the tank had to go");
        assert_eq!(seat.locked, None);
        assert_eq!(seat.role, Role::Damage);
    }

    #[test]
    fn switching_to_the_role_you_are_already_playing_keeps_the_pick() {
        let ds = fixture::dataset();
        let mut seat = seated("me", Role::Damage, Some(TRACER));

        assert!(!seat.set_role(&ds, Role::Damage), "nothing was given up");
        assert_eq!(seat.locked, Some(TRACER));
    }

    /// A dataset mismatch must not move the slot this seat is holding: there is
    /// no role to follow, so the declared one stands.
    #[test]
    fn locking_a_hero_the_roster_cannot_name_keeps_the_declared_role() {
        let ds = fixture::dataset();
        let mut seat = seated("me", Role::Support, None);

        seat.lock(&ds, NOBODY);
        assert_eq!(seat.locked, Some(NOBODY));
        assert_eq!(seat.role, Role::Support, "nothing to follow");

        // And it is not silently kept across a role switch either — the same
        // rule applies, since its role is not the one being declared.
        assert!(seat.set_role(&ds, Role::Tank));
        assert_eq!(seat.locked, None);
    }

    #[test]
    fn giving_a_pick_back_leaves_the_role_where_it_was() {
        let ds = fixture::dataset();
        let mut seat = seated("me", Role::Damage, None);
        seat.lock(&ds, REINHARDT);

        seat.unlock();
        assert_eq!(seat.locked, None);
        assert_eq!(
            seat.role,
            Role::Tank,
            "un-picking says nothing about what you are queued as"
        );
    }

    /// What the invariant buys, stated as the arithmetic the boards draw from:
    /// a seat costs one slot of its declared role either way, so locking in
    /// never moves the team's shape — only which of its slots is spoken for.
    #[test]
    fn a_seat_costs_one_slot_of_its_role_whether_or_not_it_has_locked() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Damage, None),
            seated("mika", Role::Support, None),
        ]);

        let before = room(&state, &ds, "me");
        assert_eq!(before, [1, 1, 1]);

        // Both lock in, each keeping the invariant.
        state.seats[0].lock(&ds, TRACER);
        state.seats[1].lock(&ds, ANA);

        assert_eq!(
            room(&state, &ds, "me"),
            before,
            "the shape of the team does not move when people lock in"
        );
    }

    /// The one hole the invariant would otherwise leave. [`Seat::lock`] keeps
    /// the declared role for a hero it cannot name, so the derivation has to
    /// keep charging that role — otherwise locking an unknown hero would hand
    /// the seat's own slot back to the team, since the reservation pass skips
    /// a seat that has locked.
    #[test]
    fn an_unknown_locked_hero_still_holds_the_seats_declared_role() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Damage, None),
            seated("mika", Role::Support, None),
        ]);

        let before = room(&state, &ds, "me");
        state.seats[1].lock(&ds, NOBODY);

        assert_eq!(
            room(&state, &ds, "me"),
            before,
            "an unnameable pick must not free the slot its seat was holding"
        );
    }

    /// The role switch the feature was reported over: the slot you were holding
    /// goes back to the team and the new one is taken, on every screen.
    #[test]
    fn switching_role_frees_the_slot_you_held_and_reserves_another() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, Some(SIGMA))]);

        assert_eq!(room(&state, &ds, "me"), [0, 2, 2]);

        state.seats[0].set_role(&ds, Role::Damage);
        assert_eq!(
            room(&state, &ds, "me"),
            [1, 1, 2],
            "the tank slot came back and a dps slot went"
        );
    }

    // --- leaving ------------------------------------------------------------

    #[test]
    fn removing_a_seat_gives_its_slot_back_to_the_team() {
        let ds = fixture::dataset();
        let mut state = session(vec![
            seated("me", Role::Damage, None),
            seated("mika", Role::Tank, None),
        ]);

        assert_eq!(room(&state, &ds, "me"), [0, 1, 2]);

        assert!(state.remove_seat("mika"));
        assert_eq!(
            room(&state, &ds, "me"),
            [1, 1, 2],
            "a reservation nobody will spend is one the team gets back"
        );
    }

    #[test]
    fn removing_a_seat_that_is_not_there_reports_so_and_changes_nothing() {
        let mut state = session(vec![seated("me", Role::Tank, None)]);

        assert!(!state.remove_seat("nobody"));
        assert_eq!(state.seats.len(), 1);
        // Leaving twice is the ordinary way this happens: the socket drops
        // right behind the message that already removed the seat.
        assert!(state.remove_seat("me"));
        assert!(!state.remove_seat("me"));
    }

    #[test]
    fn an_unnamed_seat_still_has_something_to_call_it() {
        assert_eq!(Seat::new("c0ffee").display_name(), "c0ffee");
        assert_eq!(seated("mika", Role::Tank, None).display_name(), "mika");

        let blank = Seat {
            name: "   ".to_owned(),
            ..Seat::new("c0ffee")
        };
        assert_eq!(blank.display_name(), "c0ffee", "whitespace is not a name");
    }

    #[test]
    fn a_session_nobody_has_touched_is_empty() {
        let mut state = session(vec![seated("me", Role::Tank, None)]);
        assert!(state.is_empty(), "seats alone are not state worth adopting");

        state.board.format = six_v_six();
        assert!(
            state.is_empty(),
            "nor is a room that has only named its queue"
        );

        state.board.map = Some(fixture::KINGS_ROW);
        assert!(!state.is_empty());
    }

    #[test]
    fn the_board_toggles_the_way_the_boards_it_replaces_did() {
        let ds = fixture::dataset();
        let mut board = Board::new();
        assert!(board.toggle_enemy(&ds, SIGMA));
        assert!(
            !board.toggle_enemy(&ds, SIGMA),
            "a second click takes it back"
        );
        assert!(board.enemies.is_empty());

        assert!(board.toggle_extra_ally(TRACER));
        assert!(!board.toggle_extra_ally(TRACER));
        assert!(board.extra_allies.is_empty());
    }

    #[test]
    fn the_enemy_board_refuses_a_second_tank_in_5v5() {
        let ds = fixture::dataset();
        let mut board = Board::new();

        assert!(board.add_enemy(&ds, SIGMA));
        assert!(!board.add_enemy(&ds, WINSTON), "5v5 fields one tank");
        assert!(board.add_enemy(&ds, TRACER), "and the other rows are open");

        board.format = six_v_six();
        assert!(board.add_enemy(&ds, WINSTON), "6v6 has room for the second");
    }

    #[test]
    fn switching_format_drops_the_picks_the_new_one_cannot_hold() {
        let ds = fixture::dataset();
        let mut board = Board {
            format: six_v_six(),
            enemies: vec![SIGMA, WINSTON, TRACER, ANA],
            extra_allies: vec![REINHARDT, LUCIO],
            ..Board::new()
        };

        board.set_format(&ds, Format::default());
        assert_eq!(board.format, Format::default());
        assert_eq!(
            board.enemies,
            vec![SIGMA, TRACER, ANA],
            "the second tank goes, and the one entered first stays"
        );
        assert_eq!(board.extra_allies, vec![REINHARDT, LUCIO]);

        // Growing costs nothing.
        let before = board.clone();
        board.set_format(&ds, six_v_six());
        assert_eq!(board.enemies, before.enemies);
        assert_eq!(board.extra_allies, before.extra_allies);
    }

    #[test]
    fn clearing_picks_keeps_the_map_and_the_format() {
        let mut board = Board::new();
        board.format = six_v_six();
        board.map = Some(fixture::KINGS_ROW);
        board.side = Some(Side::Defend);
        board.enemies = vec![SIGMA];
        board.extra_allies = vec![TRACER];

        board.clear_picks();
        assert_eq!(board.map, Some(fixture::KINGS_ROW), "one map, many rounds");
        assert_eq!(board.side, Some(Side::Defend));
        assert!(board.enemies.is_empty());
        assert!(board.extra_allies.is_empty());

        board.clear_all();
        assert_eq!(board.map, None);
        assert_eq!(board.side, None);
        assert_eq!(
            board.format,
            six_v_six(),
            "the queue outlasts even a new match"
        );
    }

    /// Both screens in a session are separate installs and can be on different
    /// builds mid-draft, so a field this end has not heard of has to arrive as
    /// something to ignore rather than something to reject. A parse error here
    /// is a session that silently stops syncing while everyone keeps picking.
    #[test]
    fn state_from_a_newer_client_still_parses() {
        let newer = r#"{
            "board": {
                "format": {"size": "6v6", "queue": "open", "ranked": true},
                "map": 0,
                "side": "attack",
                "enemies": [1, 3],
                "extra_allies": [6],
                "bans": [9]
            },
            "seats": [
                {"id": "a", "name": "era", "role": "tank", "locked": 1,
                 "connected": true, "ready": false}
            ],
            "phase": "picking"
        }"#;

        let state: SessionState =
            serde_json::from_str(newer).expect("an unknown field must not break the session");
        assert_eq!(
            state.board.format,
            Format::new(TeamSize::SixVSix, Queue::Open),
            "including one inside the format"
        );
        assert_eq!(state.board.enemies, vec![SIGMA, TRACER]);
        assert_eq!(state.seats.len(), 1);
        assert_eq!(state.seats[0].locked, Some(SIGMA));
    }

    /// The other direction: a board written by a client that predates the
    /// format has to read as the shape it was drafted in.
    #[test]
    fn a_board_from_a_client_that_had_no_format_reads_as_5v5() {
        let older = r#"{"map": 0, "enemies": [1], "extra_allies": []}"#;

        let board: Board = serde_json::from_str(older).expect("the format is optional");
        assert_eq!(board.format, Format::default());
        assert_eq!(board.format.team_size(), 5);
    }

    /// A seat written by a client that predates a field this build now expects
    /// has to load with the rest of it intact.
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
        let mut state = session(vec![
            seated("me", Role::Tank, Some(SIGMA)),
            seated("mika", Role::Damage, None),
        ]);
        state.board.format = six_v_six();
        state.board.map = Some(fixture::KINGS_ROW);
        state.board.enemies = vec![ANA];

        let json = serde_json::to_string(&state).expect("serialises");
        let back: SessionState = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, state);
    }

    #[test]
    fn an_unknown_hero_on_the_board_still_costs_the_team_a_body() {
        let ds = fixture::dataset();
        let mut state = session(vec![seated("me", Role::Tank, None)]);
        state.board.extra_allies = vec![NOBODY, TRACER, SOJOURN, ANA, LUCIO];

        let draft = state.draft_for(&ds, "me");
        assert_eq!(
            draft.allies,
            vec![NOBODY, TRACER, SOJOURN, ANA],
            "it takes a slot it cannot name a role for, and the last name loses"
        );
    }
}
