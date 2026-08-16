//! Draft sessions.
//!
//! A session is a team looking at the same draft. Whoever sees the enemy team
//! first types it once and it appears on everybody's screen.
//!
//! The state is split by who owns it. The board — map, side, enemies, and the
//! allies nobody in the session is playing — belongs to everyone, and the last
//! write wins. A seat belongs to exactly one person and holds the only thing
//! that is genuinely theirs: their own pick. That split is the whole reason
//! this scales past the two people it was originally written for. A single
//! shared `Draft` has one `locked` field, so a third person joining meant three
//! clients fighting over one slot.
//!
//! The critical property is what a session does *not* do: it never computes or
//! sends recommendations. Each client holds the whole dataset and scores
//! locally, so the only thing crossing the network is the state itself. That
//! keeps the network off the path between a keystroke and an answer, and it
//! means a client that loses the connection carries on working alone.
//!
//! There is no authentication — see the note in `main.rs`. The one rule the
//! server does enforce is that a seat may only be moved by the socket it
//! arrived on, which is not a security boundary so much as a guarantee that two
//! clients cannot corrupt each other's picks by accident.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use overwatch_core::{Board, Draft, Seat, SessionState};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// How many messages a slow client may fall behind before it is dropped.
///
/// Sessions are small and absolute, so a client that falls behind is better off
/// resubscribing and taking the current state than replaying history.
const CHANNEL_CAPACITY: usize = 32;

/// How long an emptied session is kept before it is forgotten.
///
/// Zero would mean a page reload destroys the draft, which is exactly what
/// happens at the least forgivable moment: someone's browser hiccups thirty
/// seconds into hero select and the team loses the enemy comp they had just
/// finished entering. Ten minutes covers a reload, a laptop lid, and a router
/// blip, and is short enough that codes do not accumulate all evening.
const GRACE: Duration = Duration::from_secs(10 * 60);

/// What travels over the socket.
///
/// The board is sent whole rather than as deltas: it is a handful of small
/// integers, and sending the complete thing makes conflict resolution trivially
/// last-writer-wins instead of an ordering problem during a 30-second window.
/// Seats need no conflict resolution at all — each has exactly one writer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomMessage {
    /// Client to server, and fanned back out to everyone else.
    Board {
        board: Board,
        /// Identifies the sender so it can ignore its own echo.
        from: String,
    },
    /// A client changing its own seat. The server overwrites `seat.id` with the
    /// id of the socket it arrived on, so `from` is the authority and the id in
    /// the payload is only a hint.
    Seat { seat: Seat, from: String },
    /// Server to a client that has just joined.
    Snapshot { state: SessionState },
    /// Server to everyone when the roster changes: someone joined, left, picked
    /// or renamed themselves.
    Roster { seats: Vec<Seat> },
    /// Server to a client that asked for a session nobody had started.
    Rejected { reason: String },

    /// A client saying it is done with the session, as opposed to its socket
    /// dropping. Client to server; never sent back out — what the others see is
    /// the [`RoomMessage::Roster`] the removal produces.
    ///
    /// The two are deliberately different events. A seat outlives its
    /// connection so that a reload does not empty a slot mid-draft, but
    /// somebody who has actually left is not coming back to fill theirs, and a
    /// reservation nobody will ever spend is one the rest of the team should
    /// get back. `from` is the socket's id, like everywhere else here.
    Leave { from: String },

    /// A whole-draft update from a client that predates sessions.
    ///
    /// Accepted and folded into a board plus a seat; never sent. The service
    /// worker can hold an old bundle across a rebuild, and a client that talks
    /// to this server with the old vocabulary should degrade rather than sit
    /// there silently desynced while its user keeps picking.
    Update { draft: Draft, from: String },
}

struct Room {
    state: SessionState,
    tx: broadcast::Sender<RoomMessage>,
    members: usize,
    /// When the last member left, or `None` while anyone is still here.
    /// Drives the lazy sweep in [`Rooms::join`].
    empty_since: Option<Instant>,
}

impl Room {
    fn new() -> Self {
        Self {
            state: SessionState::new(),
            tx: broadcast::channel(CHANNEL_CAPACITY).0,
            members: 0,
            // Created empty and unattended: a session minted by `POST
            // /api/session` has to survive long enough for the person who
            // asked for it to actually open the link.
            empty_since: Some(Instant::now()),
        }
    }

    fn expired(&self, now: Instant) -> bool {
        self.empty_since
            .is_some_and(|since| now.duration_since(since) >= GRACE)
    }
}

/// All active sessions, keyed by code.
#[derive(Clone, Default)]
pub struct Rooms {
    inner: Arc<Mutex<HashMap<String, Room>>>,
}

/// A client's handle on one session.
pub struct Membership {
    rooms: Rooms,
    code: String,
    id: String,
    /// Taken exactly once by the socket loop.
    ///
    /// An `Option` rather than a plain field because `Membership` has a `Drop`
    /// impl, so the receiver cannot simply be moved out — and it must be moved
    /// rather than `resubscribe`d, since a fresh subscription silently misses
    /// everything sent between joining and subscribing, including the join's
    /// own roster message.
    receiver: Option<broadcast::Receiver<RoomMessage>>,
    /// The state as of joining, so a late joiner catches up immediately.
    pub snapshot: SessionState,
}

impl Membership {
    pub fn take_receiver(&mut self) -> Option<broadcast::Receiver<RoomMessage>> {
        self.receiver.take()
    }
}

impl Rooms {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mints an unused code and reserves an empty session behind it.
    ///
    /// Retries on collision rather than trusting the odds: the code space is
    /// small enough to read aloud, which means it is small enough to collide.
    /// After enough failures it gives up and disambiguates by hand, because
    /// looping forever on a full map would hang the request instead.
    pub fn create(&self) -> String {
        let mut rooms = self.lock();
        sweep(&mut rooms, Instant::now());

        for _ in 0..16 {
            let code = crate::code::mint();
            if !rooms.contains_key(&code) {
                rooms.insert(code.clone(), Room::new());
                return code;
            }
        }

        let code = format!("{}-{}", crate::code::mint(), rooms.len());
        rooms.insert(code.clone(), Room::new());
        code
    }

    #[cfg(test)]
    fn exists(&self, code: &str) -> bool {
        let mut rooms = self.lock();
        sweep(&mut rooms, Instant::now());
        rooms.contains_key(code)
    }

    /// Joins `code`, taking a seat. Returns `None` if no such session exists.
    ///
    /// Joining deliberately does *not* create. A session you can fall into by
    /// mistyping a code is worse than an error message: everyone sits in their
    /// own private room wondering why the other four are not showing up.
    /// Creation is an explicit act, through [`Rooms::create`].
    pub fn join(&self, code: &str, id: &str, name: &str) -> Option<Membership> {
        let mut rooms = self.lock();
        sweep(&mut rooms, Instant::now());

        let room = rooms.get_mut(code)?;
        room.members += 1;
        room.empty_since = None;

        // A returning seat keeps its pick. Someone who reloads mid-draft should
        // come back to the hero they had locked, not an empty slot their team
        // has to be told about twice.
        match room.state.seat_mut(id) {
            Some(existing) => {
                existing.connected = true;
                // An empty name means "I have not named myself", which must not
                // erase the name this seat already answered to.
                if !name.is_empty() {
                    existing.name = name.to_owned();
                }
            }
            None => room.state.seats.push(Seat {
                id: id.to_owned(),
                name: name.to_owned(),
                connected: true,
                ..Seat::default()
            }),
        }

        let membership = Membership {
            rooms: self.clone(),
            code: code.to_owned(),
            id: id.to_owned(),
            receiver: Some(room.tx.subscribe()),
            snapshot: room.state.clone(),
        };
        let tx = room.tx.clone();
        let seats = room.state.seats.clone();
        drop(rooms);

        // Send after releasing the lock: `broadcast::send` runs subscriber
        // bookkeeping, and holding a std Mutex across it invites contention.
        let _ = tx.send(RoomMessage::Roster { seats });
        Some(membership)
    }

    /// Records a new shared board and fans it out.
    pub fn publish_board(&self, code: &str, board: Board, from: &str) {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(code) else {
            return;
        };
        room.state.board = board.clone();
        let tx = room.tx.clone();
        drop(rooms);

        let _ = tx.send(RoomMessage::Board {
            board,
            from: from.to_owned(),
        });
    }

    /// Records a seat and fans the roster out.
    ///
    /// `from` is the id of the socket the message arrived on and it overwrites
    /// whatever the payload claimed. That is the only ownership rule the server
    /// can enforce without authentication, and it is enough: a client can move
    /// its own seat and no other.
    pub fn publish_seat(&self, code: &str, seat: Seat, from: &str) {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(code) else {
            return;
        };

        let seat = Seat {
            id: from.to_owned(),
            connected: true,
            ..seat
        };
        room.state.upsert_seat(seat);

        let tx = room.tx.clone();
        let seats = room.state.seats.clone();
        drop(rooms);

        let _ = tx.send(RoomMessage::Roster { seats });
    }

    /// Drops a seat outright and fans the roster out.
    ///
    /// For [`RoomMessage::Leave`] only. [`Rooms::leave`] — the socket-dropped
    /// path — deliberately does not do this: the seat is kept there so that a
    /// reload comes back to its pick.
    ///
    /// Removing a seat that is not there is not an error. It is the ordinary
    /// case: the socket closes right behind the message that already removed
    /// it, and `Drop` runs anyway.
    pub fn remove_seat(&self, code: &str, id: &str) {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(code) else {
            return;
        };
        if !room.state.remove_seat(id) {
            return;
        }

        let tx = room.tx.clone();
        let seats = room.state.seats.clone();
        drop(rooms);

        let _ = tx.send(RoomMessage::Roster { seats });
    }

    /// Folds a pre-session client's whole-draft update into the split state.
    ///
    /// Its `allies` cannot be told apart from anyone else's picks, so they land
    /// in the board's hand-typed list — which is what they effectively were on
    /// a client that had no idea seats existed.
    pub fn publish_legacy_draft(&self, code: &str, draft: Draft, from: &str) {
        let board = Board {
            // A client old enough to send a whole draft sends no format either,
            // so this is 5v5 — which is the only shape that client ever had.
            format: draft.format,
            map: draft.map,
            side: draft.side,
            enemies: draft.enemies,
            extra_allies: draft.allies,
        };
        self.publish_board(code, board, from);

        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(code) else {
            return;
        };
        let seat = match room.state.seat(from) {
            Some(existing) => Seat {
                locked: draft.locked,
                ..existing.clone()
            },
            None => Seat {
                id: from.to_owned(),
                locked: draft.locked,
                connected: true,
                ..Seat::default()
            },
        };
        room.state.upsert_seat(seat);
        let tx = room.tx.clone();
        let seats = room.state.seats.clone();
        drop(rooms);

        let _ = tx.send(RoomMessage::Roster { seats });
    }

    #[cfg(test)]
    pub fn member_count(&self, code: &str) -> usize {
        self.lock().get(code).map(|room| room.members).unwrap_or(0)
    }

    #[cfg(test)]
    fn state_of(&self, code: &str) -> Option<SessionState> {
        self.lock().get(code).map(|room| room.state.clone())
    }

    /// Drops sessions whose grace period has run out. Exposed for the tests,
    /// which cannot wait ten minutes.
    #[cfg(test)]
    fn sweep_at(&self, now: Instant) {
        sweep(&mut self.lock(), now);
    }

    pub fn room_count(&self) -> usize {
        self.lock().len()
    }

    /// Marks a seat disconnected and starts the grace clock if that was the
    /// last member.
    ///
    /// The seat itself stays. A teammate who drops should read as "Mika,
    /// Winston, offline" rather than vanishing from the roster — the draft
    /// still has to account for the hero they are on.
    fn leave(&self, code: &str, id: &str) {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(code) else {
            return;
        };
        room.members = room.members.saturating_sub(1);
        if let Some(seat) = room.state.seat_mut(id) {
            seat.connected = false;
        }
        if room.members == 0 {
            room.empty_since = Some(Instant::now());
        }

        let tx = room.tx.clone();
        let seats = room.state.seats.clone();
        drop(rooms);

        let _ = tx.send(RoomMessage::Roster { seats });
    }

    /// A poisoned lock means another thread panicked while holding it. The map
    /// is still structurally sound, so recovering beats taking the server down
    /// in the middle of a draft.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Room>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Forgets sessions that have been empty longer than [`GRACE`].
///
/// Called on the paths that already hold the lock rather than from a timer.
/// A background task would need a runtime handle, a shutdown path and a test
/// that can control the clock, to reclaim a few hundred bytes a session; doing
/// it lazily costs one pass over a map that never holds more than a handful of
/// entries, and nothing accumulates unless somebody is actively joining.
fn sweep(rooms: &mut HashMap<String, Room>, now: Instant) {
    rooms.retain(|_, room| !room.expired(now));
}

impl Drop for Membership {
    fn drop(&mut self) {
        self.rooms.leave(&self.code, &self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use overwatch_core::{HeroId, MapId, Role};

    /// Creating and joining in one step, which is what every test that is not
    /// about the create/join split actually wants.
    fn open(rooms: &Rooms, id: &str) -> (String, Membership) {
        let code = rooms.create();
        let membership = rooms.join(&code, id, id).expect("the session just created");
        (code, membership)
    }

    #[test]
    fn a_late_joiner_gets_the_whole_session() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");

        let mut board = Board::new();
        board.enemies.push(HeroId(3));
        board.map = Some(MapId(2));
        rooms.publish_board(&code, board.clone(), "era");
        rooms.publish_seat(
            &code,
            Seat {
                locked: Some(HeroId(9)),
                ..Seat::new("era")
            },
            "era",
        );

        let second = rooms.join(&code, "mika", "mika").expect("joins");
        assert_eq!(
            second.snapshot.board, board,
            "joining mid-draft catches you up"
        );
        assert_eq!(
            second.snapshot.seat("era").and_then(|s| s.locked),
            Some(HeroId(9)),
            "and shows what everyone else is already on"
        );
    }

    #[test]
    fn joining_a_code_nobody_minted_is_rejected() {
        let rooms = Rooms::new();

        assert!(
            rooms.join("brave-otter-41", "era", "era").is_none(),
            "a mistyped code must not quietly become a session of one"
        );
        assert_eq!(rooms.room_count(), 0, "and must not leave anything behind");
    }

    #[test]
    fn a_created_session_can_be_joined_by_its_code() {
        let rooms = Rooms::new();
        let code = rooms.create();

        assert!(rooms.exists(&code));
        assert!(rooms.join(&code, "era", "era").is_some());
    }

    #[test]
    fn each_created_session_gets_its_own_code() {
        let rooms = Rooms::new();
        let first = rooms.create();
        let second = rooms.create();

        assert_ne!(first, second);
        assert_eq!(rooms.room_count(), 2);
    }

    #[tokio::test]
    async fn a_board_update_reaches_the_other_clients() {
        let rooms = Rooms::new();
        let (code, mut first) = open(&rooms, "era");
        let mut first_rx = first.take_receiver().expect("receiver");
        let _second = rooms.join(&code, "mika", "mika").expect("joins");

        let mut board = Board::new();
        board.enemies.push(HeroId(7));
        rooms.publish_board(&code, board.clone(), "mika");

        // Both joins announced themselves, so skip the roster traffic rather
        // than assuming how much of it is queued.
        let message = loop {
            match first_rx.recv().await.expect("delivered") {
                RoomMessage::Roster { .. } => continue,
                other => break other,
            }
        };
        assert_eq!(
            message,
            RoomMessage::Board {
                board,
                from: "mika".to_owned()
            }
        );
    }

    #[test]
    fn a_seat_update_only_moves_its_own_seat() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");
        let _second = rooms.join(&code, "mika", "mika").expect("joins");

        rooms.publish_seat(
            &code,
            Seat {
                locked: Some(HeroId(4)),
                role: Role::Support,
                ..Seat::new("mika")
            },
            "mika",
        );

        let state = rooms.state_of(&code).expect("the session");
        assert_eq!(state.seat("mika").and_then(|s| s.locked), Some(HeroId(4)));
        assert_eq!(
            state.seat("era").and_then(|s| s.locked),
            None,
            "one person picking must not disturb anyone else's slot"
        );
    }

    /// The one ownership rule the server can enforce without authentication.
    /// A client that claims someone else's id in the payload gets its own seat
    /// moved instead, because the socket it arrived on is the authority.
    #[test]
    fn a_client_cannot_move_someone_elses_seat() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");
        let _second = rooms.join(&code, "mika", "mika").expect("joins");

        rooms.publish_seat(
            &code,
            Seat {
                locked: Some(HeroId(6)),
                ..Seat::new("era")
            },
            // ...but it actually arrived on mika's socket.
            "mika",
        );

        let state = rooms.state_of(&code).expect("the session");
        assert_eq!(
            state.seat("era").and_then(|s| s.locked),
            None,
            "era's pick is era's alone"
        );
        assert_eq!(
            state.seat("mika").and_then(|s| s.locked),
            Some(HeroId(6)),
            "the write lands on the seat that actually sent it"
        );
    }

    #[test]
    fn sessions_are_isolated_from_each_other() {
        let rooms = Rooms::new();
        let (ours, _us) = open(&rooms, "era");
        let (theirs, _them) = open(&rooms, "someone");

        let mut board = Board::new();
        board.enemies.push(HeroId(1));
        rooms.publish_board(&ours, board, "era");

        let other = rooms.state_of(&theirs).expect("the other session");
        assert_eq!(other.board, Board::new());
        assert_eq!(rooms.member_count(&ours), 1);
        assert_eq!(rooms.member_count(&theirs), 1);
    }

    #[test]
    fn leaving_marks_a_seat_disconnected_rather_than_deleting_it() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");
        let second = rooms.join(&code, "mika", "mika").expect("joins");
        rooms.publish_seat(
            &code,
            Seat {
                locked: Some(HeroId(8)),
                ..Seat::new("mika")
            },
            "mika",
        );

        drop(second);

        let state = rooms.state_of(&code).expect("the session");
        let seat = state.seat("mika").expect("mika's seat outlives the socket");
        assert!(!seat.connected, "the roster has to show they dropped");
        assert_eq!(
            seat.locked,
            Some(HeroId(8)),
            "the team is still playing around the hero they are on"
        );
        assert_eq!(rooms.member_count(&code), 1);
    }

    #[test]
    fn a_reload_keeps_the_seat_it_left() {
        let rooms = Rooms::new();
        let (code, first) = open(&rooms, "era");
        rooms.publish_seat(
            &code,
            Seat {
                locked: Some(HeroId(5)),
                ..Seat::new("era")
            },
            "era",
        );

        drop(first);
        let back = rooms
            .join(&code, "era", "era")
            .expect("the session survives");

        assert_eq!(
            back.snapshot.seat("era").and_then(|s| s.locked),
            Some(HeroId(5)),
            "reloading must not cost you the hero you had locked"
        );
        assert_eq!(
            back.snapshot.seats.len(),
            1,
            "and must not duplicate a seat"
        );
    }

    #[test]
    fn an_empty_session_survives_a_reload_but_not_the_grace_period() {
        let rooms = Rooms::new();
        let (_code, first) = open(&rooms, "era");
        drop(first);

        rooms.sweep_at(Instant::now());
        assert_eq!(
            rooms.room_count(),
            1,
            "an empty session has to outlive a page reload"
        );

        rooms.sweep_at(Instant::now() + GRACE + Duration::from_secs(1));
        assert_eq!(
            rooms.room_count(),
            0,
            "but must not linger all evening once nobody comes back"
        );
    }

    #[test]
    fn a_session_still_in_use_is_never_swept() {
        let rooms = Rooms::new();
        let (code, _held) = open(&rooms, "era");

        rooms.sweep_at(Instant::now() + GRACE * 10);
        assert!(
            rooms.exists(&code),
            "a long draft must not be swept out from under the people in it"
        );
    }

    #[test]
    fn publishing_to_a_session_nobody_started_is_harmless() {
        let rooms = Rooms::new();
        rooms.publish_board("nobody", Board::new(), "era");
        rooms.publish_seat("nobody", Seat::new("era"), "era");
        rooms.remove_seat("nobody", "era");
        assert_eq!(rooms.room_count(), 0);
    }

    /// The counterpart to `leaving_marks_a_seat_disconnected_rather_than_
    /// deleting_it`, and the two are meant to be read as a pair: a socket that
    /// drops keeps its seat, and somebody who says they are done does not.
    #[test]
    fn an_explicit_leave_removes_the_seat_rather_than_marking_it_offline() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");
        let second = rooms.join(&code, "mika", "mika").expect("joins");
        rooms.publish_seat(
            &code,
            Seat {
                role: Role::Tank,
                locked: Some(HeroId(8)),
                ..Seat::new("mika")
            },
            "mika",
        );

        rooms.remove_seat(&code, "mika");

        let state = rooms.state_of(&code).expect("the session");
        assert!(
            state.seat("mika").is_none(),
            "somebody who left is not holding a slot open"
        );
        assert!(state.seat("era").is_some(), "and nobody else was touched");

        // The socket closing behind the message is the ordinary case, not an
        // error: `Drop` runs regardless and must find nothing to do.
        drop(second);
        let state = rooms.state_of(&code).expect("the session");
        assert!(state.seat("mika").is_none(), "and it stays gone");
    }

    #[test]
    fn leaving_a_session_twice_is_not_an_error() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");

        rooms.remove_seat(&code, "era");
        rooms.remove_seat(&code, "era");
        rooms.remove_seat(&code, "nobody-was-here");

        let state = rooms.state_of(&code).expect("the session survives");
        assert!(state.seats.is_empty());
    }

    /// The wire shape the client's own test pins from the other side. The
    /// envelope is duplicated on purpose, so this pair is the only thing
    /// stopping the two halves drifting.
    #[test]
    fn leave_messages_use_the_agreed_shape() {
        let message = RoomMessage::Leave {
            from: "c1234".to_owned(),
        };
        let json = serde_json::to_string(&message).expect("serialises");

        assert!(json.contains(r#""type":"leave""#), "{json}");
        assert!(json.contains(r#""from":"c1234""#), "{json}");

        let back: RoomMessage = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, message);
    }

    #[test]
    fn messages_round_trip_as_json() {
        let mut board = Board::new();
        board.enemies.push(HeroId(2));

        let message = RoomMessage::Board {
            board,
            from: "era".to_owned(),
        };
        let json = serde_json::to_string(&message).expect("serialises");
        let back: RoomMessage = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, message);
    }

    /// The screens in a session are separate installs and can be on different
    /// builds mid-draft. A field this end has dropped therefore has to arrive
    /// as something to ignore rather than something to reject: a parse error
    /// here is a draft that silently stops syncing while everyone keeps
    /// picking.
    #[test]
    fn a_message_from_a_newer_client_still_parses() {
        let newer = r#"{
            "type": "board",
            "from": "them",
            "board": {
                "map": 3,
                "side": "attack",
                "enemies": [1, 2],
                "extra_allies": [4],
                "bans": [7]
            }
        }"#;

        let RoomMessage::Board { board, from } =
            serde_json::from_str(newer).expect("an unknown field must not break sync")
        else {
            panic!("should have parsed as a board");
        };

        assert_eq!(from, "them");
        assert_eq!(board.enemies, vec![HeroId(1), HeroId(2)]);
        assert_eq!(board.extra_allies, vec![HeroId(4)]);
    }

    /// The service worker can serve a bundle from before sessions existed. Such
    /// a client speaks whole drafts, and the server has to keep understanding
    /// it rather than logging a parse error while its user keeps drafting.
    #[test]
    fn a_draft_from_a_pre_session_client_still_parses() {
        let old = r#"{
            "type": "update",
            "from": "them",
            "draft": {
                "map": 3,
                "side": "attack",
                "enemies": [1, 2],
                "allies": [4],
                "focused": [1],
                "locked": 5
            }
        }"#;

        let RoomMessage::Update { draft, from } =
            serde_json::from_str(old).expect("an older client's update still parses")
        else {
            panic!("should have parsed as an update");
        };

        assert_eq!(from, "them");
        assert_eq!(draft.enemies, vec![HeroId(1), HeroId(2)]);
        assert_eq!(draft.allies, vec![HeroId(4)]);
        assert_eq!(draft.locked, Some(HeroId(5)));
    }

    #[test]
    fn a_pre_session_clients_draft_lands_where_a_session_can_read_it() {
        let rooms = Rooms::new();
        let (code, _first) = open(&rooms, "era");

        let mut draft = Draft::new();
        draft.map = Some(MapId(4));
        draft.add_enemy(HeroId(1));
        draft.add_ally(HeroId(2));
        draft.locked = Some(HeroId(3));
        rooms.publish_legacy_draft(&code, draft, "era");

        let state = rooms.state_of(&code).expect("the session");
        assert_eq!(state.board.map, Some(MapId(4)));
        assert_eq!(state.board.enemies, vec![HeroId(1)]);
        assert_eq!(
            state.board.extra_allies,
            vec![HeroId(2)],
            "an old client's allies are indistinguishable from typed-in ones"
        );
        assert_eq!(
            state.seat("era").and_then(|s| s.locked),
            Some(HeroId(3)),
            "but its own pick still belongs to its seat"
        );
    }

    /// Regression: the socket loop used to `resubscribe()`, which starts a
    /// fresh subscription and therefore missed the roster message the join
    /// itself had already sent. A client alone in a session then sat on
    /// "connecting" forever, because nothing else was ever going to tell it.
    #[tokio::test]
    async fn a_joiner_sees_its_own_roster_message() {
        let rooms = Rooms::new();
        let (_code, mut membership) = open(&rooms, "era");
        let mut receiver = membership.take_receiver().expect("receiver");

        let RoomMessage::Roster { seats } = receiver
            .try_recv()
            .expect("the roster was queued at join time")
        else {
            panic!("should have been a roster");
        };
        assert_eq!(seats.len(), 1);
        assert_eq!(
            membership.snapshot.seats.len(),
            1,
            "and is also available without waiting for the broadcast"
        );
    }
}
