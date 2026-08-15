//! Live draft sync between the people in a session.
//!
//! Whoever reads the enemy team first types it once and it lands on every
//! screen. What crosses the socket is *only* the session state: recommendations
//! are computed locally on each machine from the compiled-in dataset, so this
//! feature adds no latency to the thing that actually matters, and losing the
//! connection degrades the app to single-player rather than breaking it.
//!
//! The state is split by ownership rather than sent as one blob. The board —
//! map, side, enemies, typed-in allies — is shared and last-writer-wins; it is
//! a handful of small integers, so it goes whole rather than as a delta and
//! conflict resolution stays trivial instead of becoming an ordering problem
//! inside a thirty-second window. A seat is written by exactly one client, so
//! it cannot conflict at all.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;
use overwatch_core::{Board, Seat, SessionState};
use serde::{Deserialize, Serialize};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

const CLIENT_ID_KEY: &str = "overwatch-picker.client-id";

/// First reconnect delay, in milliseconds.
const BACKOFF_MIN_MS: i32 = 500;
/// Ceiling on the reconnect delay.
///
/// Eight seconds is about as long as someone will stare at "offline" during a
/// draft before deciding the feature is broken. The app is fully usable
/// throughout, but the light still has to come back on its own.
const BACKOFF_MAX_MS: i32 = 8_000;

/// Mirrors `overwatch_server::room::RoomMessage`.
///
/// Duplicated rather than shared through a crate: it is a handful of variants,
/// and a shared wire crate would drag the server's dependencies into the wasm
/// build for no real gain. The payload types themselves are *not* duplicated —
/// they live in `overwatch-core`, which both sides already depend on and which
/// is wasm-clean, so the only thing that can drift is the envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomMessage {
    Board { board: Board, from: String },
    Seat { seat: Seat, from: String },
    Snapshot { state: SessionState },
    Roster { seats: Vec<Seat> },
    Rejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Not in a session at all. The normal state for one person drafting alone,
    /// and not a failure of anything.
    Solo,
    Connecting,
    /// Connected, with the number of people in the session.
    Live(usize),
    /// The connection dropped and is being retried. The app is fully usable in
    /// this state.
    Reconnecting,
    /// The code does not name a session anybody started.
    Rejected,
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::Solo => "solo".to_owned(),
            Status::Connecting => "connecting".to_owned(),
            // One member is just you, which is worth distinguishing from an
            // actual shared session.
            Status::Live(0 | 1) => "session of 1".to_owned(),
            Status::Live(n) => format!("synced ×{n}"),
            Status::Reconnecting => "reconnecting".to_owned(),
            Status::Rejected => "no such session".to_owned(),
        }
    }

    /// Whether the socket is currently carrying anything.
    pub fn is_live(&self) -> bool {
        matches!(self, Status::Live(_))
    }
}

/// The delay before the nth reconnect attempt.
///
/// Split out as a plain function because it is the one piece of the reconnect
/// logic that can be wrong in a way tests can catch — everything else is
/// browser callbacks.
pub fn backoff_ms(attempt: u32) -> i32 {
    // Clamped before the shift rather than after: `1 << 32` is undefined for
    // `i32`, and a long outage reaches that in under a minute.
    let factor = 1_i32.checked_shl(attempt.min(16)).unwrap_or(i32::MAX);
    BACKOFF_MIN_MS
        .saturating_mul(factor)
        .clamp(BACKOFF_MIN_MS, BACKOFF_MAX_MS)
}

/// A stable per-browser identity, so a client can recognise and ignore the echo
/// of its own update — and so its seat can be found again after a reload.
pub fn client_id() -> String {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());

    if let Some(existing) = storage
        .as_ref()
        .and_then(|s| s.get_item(CLIENT_ID_KEY).ok().flatten())
    {
        if !existing.is_empty() {
            return existing;
        }
    }

    // Uniqueness only has to hold between the browsers on one LAN.
    let id = format!("c{:08x}", (js_sys::Math::random() * 4_294_967_296.0) as u32);
    if let Some(storage) = storage {
        let _ = storage.set_item(CLIENT_ID_KEY, &id);
    }
    id
}

fn socket_url(code: &str, id: &str, name: &str) -> Option<String> {
    let location = web_sys::window()?.location();
    let host = location.host().ok()?;
    let scheme = match location.protocol().ok()?.as_str() {
        "https:" => "wss",
        _ => "ws",
    };
    let code = encode(code);
    let id = encode(id);
    let name = encode(name);
    Some(format!("{scheme}://{host}/ws/{code}?id={id}&name={name}"))
}

/// Percent-encodes the few characters that would otherwise change what a URL
/// means. Display names are free text, so a name with a `&` or a `#` in it must
/// not be able to forge a query parameter.
fn encode(raw: &str) -> String {
    js_sys::encode_uri_component(raw).into()
}

/// The signals a live connection writes into.
///
/// Passed as one struct rather than five arguments because every reconnect has
/// to rewire exactly the same set, and a dropped one would show up as a screen
/// that stops updating rather than as a compile error.
#[derive(Clone, Copy)]
pub struct Sinks {
    pub board: Signal<Board>,
    pub seats: Signal<Vec<Seat>>,
    /// Mirrors the last board the server sent, so the outbound effect can tell
    /// "this came from another screen" from "this came from my keyboard".
    pub synced_board: Signal<Board>,
    pub status: Signal<Status>,
}

/// An open connection to one session, or a placeholder when there is none.
///
/// The socket lives behind an `Rc<RefCell<_>>` so that a reconnect can swap it
/// in place, and so that joining or leaving a session does not need a page
/// reload — which is what pinning the socket inside a `use_hook` used to force.
#[derive(Clone)]
pub struct Connection {
    socket: Rc<RefCell<Option<WebSocket>>>,
    id: String,
}

impl Connection {
    /// A connection attached to nothing. Publishing through it is a no-op, which
    /// is exactly what drafting alone should cost.
    pub fn idle(id: String) -> Self {
        Self {
            socket: Rc::new(RefCell::new(None)),
            id,
        }
    }

    fn send(&self, message: &RoomMessage) {
        let socket = self.socket.borrow();
        let Some(socket) = socket.as_ref() else {
            return;
        };
        if socket.ready_state() != WebSocket::OPEN {
            return;
        }
        if let Ok(text) = serde_json::to_string(message) {
            let _ = socket.send_with_str(&text);
        }
    }

    /// Publishes the shared board. Silently does nothing when offline — a failed
    /// send must never interrupt what the player is doing.
    pub fn publish_board(&self, board: &Board) {
        self.send(&RoomMessage::Board {
            board: board.clone(),
            from: self.id.clone(),
        });
    }

    /// Publishes this client's own seat.
    pub fn publish_seat(&self, seat: &Seat) {
        self.send(&RoomMessage::Seat {
            seat: seat.clone(),
            from: self.id.clone(),
        });
    }

    /// Closes the socket without reopening it. The reconnect loop checks for
    /// this before rescheduling, so leaving a session is not mistaken for a
    /// dropped connection.
    pub fn close(&self) {
        if let Some(socket) = self.socket.borrow_mut().take() {
            // Unhook first: `close()` fires `onclose`, which would otherwise
            // schedule a reconnect to the session we are deliberately leaving.
            socket.set_onclose(None);
            socket.set_onerror(None);
            socket.set_onmessage(None);
            socket.set_onopen(None);
            let _ = socket.close();
        }
    }
}

/// Opens a session socket and keeps it open.
///
/// Returns immediately with a `Connection` whose socket may still be
/// connecting; everything on the send path tolerates that. Drops are retried
/// with backoff until [`Connection::close`] is called.
pub fn connect(code: &str, name: &str, sinks: Sinks) -> Connection {
    let connection = Connection::idle(client_id());
    open(
        connection.clone(),
        code.to_owned(),
        name.to_owned(),
        sinks,
        0,
    );
    connection
}

/// One connection attempt, which reschedules itself on failure.
fn open(connection: Connection, code: String, name: String, mut sinks: Sinks, attempt: u32) {
    let id = connection.id.clone();

    let Some(url) = socket_url(&code, &id, &name) else {
        sinks.status.set(Status::Rejected);
        return;
    };
    let Ok(socket) = WebSocket::new(&url) else {
        // No server on this LAN is a perfectly normal way to use the app.
        sinks.status.set(Status::Reconnecting);
        retry(connection, code, name, sinks, attempt);
        return;
    };

    if attempt == 0 {
        sinks.status.set(Status::Connecting);
    }

    // A rejection is terminal: retrying a code the server does not know would
    // hammer it forever over a typo. Tracked here rather than in a signal so it
    // is not affected by whatever the UI does with the status.
    let rejected = Rc::new(RefCell::new(false));

    let on_message = {
        let rejected = rejected.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let Some(text) = event.data().as_string() else {
                return;
            };
            // An unparseable message is ignored rather than fatal: the other
            // screens can be on a newer build, and a message this one does not
            // understand must not stop the ones it does.
            let Ok(message) = serde_json::from_str::<RoomMessage>(&text) else {
                return;
            };

            match message {
                RoomMessage::Board {
                    board: incoming,
                    from,
                } => {
                    if from == id {
                        return;
                    }
                    sinks.synced_board.set(incoming.clone());
                    sinks.board.set(incoming);
                }
                RoomMessage::Snapshot { state } => {
                    sinks.seats.set(state.seats.clone());
                    sinks
                        .status
                        .set(Status::Live(connected_count(&state.seats)));
                    // Only adopt a board that has something in it, so joining a
                    // stale empty session cannot wipe a draft already in
                    // progress.
                    if !state.board.is_empty() {
                        sinks.synced_board.set(state.board.clone());
                        sinks.board.set(state.board);
                    }
                }
                RoomMessage::Roster { seats } => {
                    sinks.status.set(Status::Live(connected_count(&seats)));
                    sinks.seats.set(seats);
                }
                RoomMessage::Rejected { .. } => {
                    *rejected.borrow_mut() = true;
                    sinks.status.set(Status::Rejected);
                }
                // Client-to-server only.
                RoomMessage::Seat { .. } => {}
            }
        })
    };
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let on_close = {
        let connection = connection.clone();
        let rejected = rejected.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if *rejected.borrow() {
                return;
            }
            // A close for a socket we already replaced or deliberately dropped
            // is not something to reconnect from.
            if connection.socket.borrow().is_none() {
                return;
            }
            sinks.status.set(Status::Reconnecting);
            retry(
                connection.clone(),
                code.clone(),
                name.clone(),
                sinks,
                attempt + 1,
            );
        })
    };
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    socket.set_onerror(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    *connection.socket.borrow_mut() = Some(socket);
}

/// Schedules the next attempt.
///
/// `setTimeout` rather than a tokio timer: there is no async runtime in the
/// wasm build, and adding one to wait half a second would be an odd trade.
fn retry(connection: Connection, code: String, name: String, sinks: Sinks, attempt: u32) {
    // Drop the dead socket so the guard in `on_close` can tell a retry we
    // scheduled from a close we should ignore.
    *connection.socket.borrow_mut() = None;

    let Some(window) = web_sys::window() else {
        return;
    };
    let again = Closure::once_into_js(move || {
        open(connection, code, name, sinks, attempt);
    });
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        again.unchecked_ref(),
        backoff_ms(attempt),
    );
}

/// How many people are actually attached.
///
/// A seat outlives its socket so that a teammate who reloads does not vanish
/// mid-draft, which means the roster length is the size of the team and *not*
/// the number of live connections. The status light wants the latter.
fn connected_count(seats: &[Seat]) -> usize {
    seats.iter().filter(|seat| seat.connected).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use overwatch_core::HeroId;

    /// The wire format must match the server's, which is the one thing a
    /// duplicated type can get wrong.
    #[test]
    fn board_messages_use_the_agreed_shape() {
        let mut board = Board::new();
        board.add_enemy(HeroId(4));

        let json = serde_json::to_string(&RoomMessage::Board {
            board,
            from: "c1234".to_owned(),
        })
        .expect("serialises");

        assert!(json.contains(r#""type":"board""#), "{json}");
        assert!(json.contains(r#""from":"c1234""#), "{json}");
        assert!(json.contains(r#""enemies":[4]"#), "{json}");
    }

    #[test]
    fn seat_messages_use_the_agreed_shape() {
        let json = serde_json::to_string(&RoomMessage::Seat {
            seat: Seat {
                locked: Some(HeroId(9)),
                ..Seat::new("c1234")
            },
            from: "c1234".to_owned(),
        })
        .expect("serialises");

        assert!(json.contains(r#""type":"seat""#), "{json}");
        assert!(json.contains(r#""locked":9"#), "{json}");
    }

    #[test]
    fn a_snapshot_from_the_server_parses() {
        let wire = r#"{
            "type": "snapshot",
            "state": {
                "board": {"map": 2, "enemies": [1]},
                "seats": [{"id": "a", "name": "era", "locked": 3, "connected": true}]
            }
        }"#;

        let RoomMessage::Snapshot { state } = serde_json::from_str(wire).expect("parses") else {
            panic!("should have parsed as a snapshot");
        };
        assert_eq!(state.board.enemies, vec![HeroId(1)]);
        assert_eq!(state.seats[0].locked, Some(HeroId(3)));
    }

    #[test]
    fn status_labels_distinguish_the_ways_of_being_alone() {
        assert_eq!(Status::Solo.label(), "solo");
        assert_eq!(Status::Live(1).label(), "session of 1");
        assert_eq!(Status::Live(3).label(), "synced ×3");
        assert_eq!(Status::Reconnecting.label(), "reconnecting");
        assert_eq!(Status::Rejected.label(), "no such session");
    }

    #[test]
    fn only_a_live_session_counts_as_live() {
        assert!(Status::Live(2).is_live());
        assert!(!Status::Solo.is_live());
        assert!(!Status::Reconnecting.is_live());
    }

    #[test]
    fn backoff_grows_and_then_stops_growing() {
        assert_eq!(backoff_ms(0), BACKOFF_MIN_MS);
        assert_eq!(backoff_ms(1), BACKOFF_MIN_MS * 2);
        assert!(backoff_ms(3) > backoff_ms(2), "it has to actually back off");
        assert_eq!(
            backoff_ms(30),
            BACKOFF_MAX_MS,
            "and has to stop, without overflowing on the way"
        );
    }

    /// A seat outlives its socket, so the light must count connections rather
    /// than seats — otherwise a teammate who closed their laptop still reads as
    /// synced.
    #[test]
    fn the_status_counts_live_connections_not_seats() {
        let seats = vec![
            Seat {
                connected: true,
                ..Seat::new("a")
            },
            Seat {
                connected: false,
                ..Seat::new("b")
            },
        ];
        assert_eq!(connected_count(&seats), 1);
    }
}
