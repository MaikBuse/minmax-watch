//! LAN sync server.
//!
//! Two jobs, both deliberately small:
//!
//! 1. Serve the compiled wasm bundle.
//! 2. Relay draft state between the clients in a session.
//!
//! It does **not** score anything. Recommendations are computed on each client
//! from a compiled-in dataset, so the network never sits between a keystroke
//! and an answer, and a client whose connection drops keeps working alone.
//! That is the whole reason the sync feature does not cost any latency.
//!
//! There is no authentication: it is meant for a home network. A session code
//! is a convenience for finding the right draft, not a secret — anyone who can
//! reach the port and knows a code is in that session.
//!
//! The match log is the one part that is genuinely personal rather than merely
//! unguarded, so it can be switched off: an empty `OVERWATCH_MATCH_LOG` makes
//! `/api/matches` a 404 in both directions. The public deployment runs that
//! way, which is what makes exposing the rest of this defensible.

mod code;
mod matchlog;
mod room;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_http::services::{ServeDir, ServeFile};

use crate::matchlog::{MatchLog, MatchRecord};
use crate::room::{RoomMessage, Rooms};

#[derive(Clone)]
struct AppState {
    rooms: Rooms,
    /// `None` disables the match log entirely — see `Config::from_env`.
    matches: Option<MatchLog>,
}

#[derive(Debug, Deserialize)]
struct ClientQuery {
    /// Distinguishes the people in a session, so a client can ignore the echo
    /// of its own update and so its seat can be found again after a reload.
    /// Absent means "tell me about everything".
    #[serde(default)]
    id: Option<String>,
    /// What the roster should call this person. Optional: a client that has not
    /// been named yet still gets a seat, shown by its id.
    #[serde(default)]
    name: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env();

    let state = AppState {
        rooms: Rooms::new(),
        matches: config.match_log.clone().map(MatchLog::new),
    };

    let app = router(state, &config.assets);

    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .with_context(|| format!("binding {}", config.addr))?;

    print_banner(&config);

    axum::serve(listener, app)
        .await
        .context("running the server")?;
    Ok(())
}

/// The route table.
///
/// Split out from `main` so the end-to-end test can serve the real thing on an
/// ephemeral port rather than testing a reimplementation of it.
fn router(state: AppState, assets: &std::path::Path) -> Router {
    let index = assets.join("index.html");
    // `precompressed_*` serves `foo.wasm.br` in place of `foo.wasm` when the
    // client accepts it, and falls straight back to the plain file when no
    // sibling exists. So the container build can brotli the bundle once at
    // -q 11 and every request is a plain file read, where compressing on the
    // fly would spend CPU per request to get a worse ratio — and `just serve`,
    // which precompresses nothing, is unaffected either way.
    //
    // The wasm is the reason this is here: ~1.2M raw, roughly a third of that
    // compressed, on the critical path of every first visit.
    let static_files = ServeDir::new(assets)
        .precompressed_br()
        .precompressed_gzip()
        // Unknown paths fall back to the SPA shell rather than a bare 404.
        .not_found_service(
            ServeFile::new(&index)
                .precompressed_br()
                .precompressed_gzip(),
        );

    Router::new()
        .route("/health", get(health))
        .route("/ws/{room}", get(ws_handler))
        .route("/api/session", post(post_session))
        // Registered even when the match log is off, in which case the handlers
        // answer 404. Dropping the route instead would hand `/api/matches` to
        // the SPA fallback below, which answers 200 with the index page — a
        // disabled endpoint that returns the app is worse than either a working
        // one or an honest 404.
        .route("/api/matches", post(post_match).get(get_matches))
        .fallback_service(static_files)
        .with_state(state)
}

struct Config {
    addr: SocketAddr,
    assets: PathBuf,
    /// `None` when the match log is switched off.
    match_log: Option<PathBuf>,
}

impl Config {
    fn from_env() -> Self {
        // Binds all interfaces by default: the entire point is that the other
        // person reaches it from their own machine.
        let addr = std::env::var("OVERWATCH_ADDR")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 8080)));

        let assets = std::env::var("OVERWATCH_ASSETS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target/dx/overwatch-web/release/web/public"));

        // Unset keeps the local default, so `just serve` is unaffected. Set but
        // empty means off — the deployment switch. Spelling it that way rather
        // than adding a second variable keeps the two states that matter (where
        // is it / is it on) in one place, and an empty string is the one value
        // that could never be a real path.
        let match_log = match std::env::var("OVERWATCH_MATCH_LOG") {
            Ok(raw) if raw.is_empty() => None,
            Ok(raw) => Some(PathBuf::from(raw)),
            Err(_) => Some(PathBuf::from("data/matches.jsonl")),
        };

        Self {
            addr,
            assets,
            match_log,
        }
    }
}

/// Prints URLs that can actually be opened.
///
/// The bind address is usually `0.0.0.0`, which is a wildcard meaning "every
/// interface" — it is not something a browser can connect to. Printing it
/// verbatim invites exactly one bug report, so it never appears here.
fn print_banner(config: &Config) {
    let port = config.addr.port();
    println!("minmax is up");
    println!("  this machine   http://localhost:{port}");

    match primary_local_ip() {
        Some(ip) => println!("  this network   http://{ip}:{port}"),
        None => println!("  this network   (could not determine a local IP)"),
    }

    if is_wsl() {
        // WSL2's default NAT puts the distro on its own private network, so the
        // address above is reachable from Windows but not from another machine.
        let wsl_ip = primary_local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "<wsl-ip>".to_owned());

        println!();
        println!("  Running under WSL. The network address above is WSL's own, so");
        println!("  another machine on your LAN cannot reach it as-is.");
        println!();
        println!("  Easiest fix - mirrored networking. Put this in %USERPROFILE%\\.wslconfig:");
        println!("      [wsl2]");
        println!("      networkingMode=mirrored");
        println!("  then run `wsl --shutdown` and start again.");
        println!();
        println!("  Or forward the port. In an admin PowerShell, one line each:");
        // Written out in full rather than with shell substitution: these are
        // PowerShell, and bash syntax pasted into it silently does the wrong
        // thing.
        println!(
            "      netsh interface portproxy add v4tov4 listenport={port} listenaddress=0.0.0.0 connectport={port} connectaddress={wsl_ip}"
        );
        println!(
            "      New-NetFirewallRule -DisplayName 'MinMax' -Direction Inbound -LocalPort {port} -Protocol TCP -Action Allow"
        );
        println!();
        println!("  Note the forwarded address changes when WSL restarts; mirrored mode does not.");
    }

    println!();
    println!("  serving   {}", config.assets.display());
    match &config.match_log {
        Some(path) => println!("  match log {}", path.display()),
        None => println!("  match log off (OVERWATCH_MATCH_LOG is empty)"),
    }
    if !config.assets.is_dir() {
        println!(
            "  note: {} does not exist yet - run `just build-web` first",
            config.assets.display()
        );
    }
}

/// The address this machine would use to reach the outside world.
///
/// Found by asking the OS which local address it would route a UDP datagram
/// from. Nothing is actually sent — `connect` on UDP only sets a default peer —
/// and it beats guessing at interface names.
fn primary_local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // Any routable address works; this one is never contacted.
    socket.connect("192.0.2.1:80").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip())
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|version| {
            let version = version.to_ascii_lowercase();
            version.contains("microsoft") || version.contains("wsl")
        })
        .unwrap_or(false)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "rooms": state.rooms.room_count(),
    }))
}

/// Mints a session and returns its code.
///
/// Creating is a separate act from joining so that a mistyped code fails
/// loudly. If the socket created on demand, a typo would put you alone in a
/// session of one that looks exactly like a working one — the failure would not
/// surface until the rest of the team wondered where you were.
async fn post_session(State(state): State<AppState>) -> impl IntoResponse {
    Json(serde_json::json!({ "code": state.rooms.create() }))
}

async fn post_match(
    State(state): State<AppState>,
    Json(record): Json<MatchRecord>,
) -> impl IntoResponse {
    let Some(matches) = &state.matches else {
        return StatusCode::NOT_FOUND;
    };
    match matches.append(&record).await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(err) => {
            eprintln!("failed to record a match: {err:#}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

async fn get_matches(State(state): State<AppState>) -> impl IntoResponse {
    let Some(matches) = &state.matches else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match matches.read_all().await {
        Ok(records) => Json(records).into_response(),
        Err(err) => {
            eprintln!("failed to read the match log: {err:#}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    AxumPath(room): AxumPath<String>,
    Query(query): Query<ClientQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let client_id = query.id.unwrap_or_else(|| "anonymous".to_owned());
    let name = query.name.unwrap_or_default();
    ws.on_upgrade(move |socket| handle_socket(socket, room, client_id, name, state))
}

async fn handle_socket(
    mut socket: WebSocket,
    room: String,
    client_id: String,
    name: String,
    state: AppState,
) {
    let code = crate::code::normalise(&room);

    // A code nobody started is a typo, and saying so is the whole point of
    // separating create from join. Silently opening a session here would hand
    // the user something that looks like it is working.
    //
    // The plausibility check comes first so that a URL carrying nothing usable
    // — `/ws/` with an empty segment, or a pasted paragraph — is refused before
    // it can become a map key.
    let joined = crate::code::is_plausible(&code)
        .then(|| state.rooms.join(&code, &client_id, &name))
        .flatten();

    let Some(mut membership) = joined else {
        let _ = send(
            &mut socket,
            &RoomMessage::Rejected {
                reason: "no such session".to_owned(),
            },
        )
        .await;
        return;
    };
    let Some(mut receiver) = membership.take_receiver() else {
        return;
    };

    // Catch the newcomer up before anything else, so joining mid-draft shows
    // the current picks rather than a blank screen. The snapshot carries the
    // roster too, so a client alone in a session learns it is connected without
    // waiting for anyone else to arrive.
    let snapshot = RoomMessage::Snapshot {
        state: membership.snapshot.clone(),
    };
    if send(&mut socket, &snapshot).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            // Something happened elsewhere in the session.
            broadcast = receiver.recv() => {
                match broadcast {
                    Ok(RoomMessage::Board { board, from }) => {
                        // Do not echo a client's own update back at it: that
                        // would fight with whatever they are typing right now.
                        if from == client_id {
                            continue;
                        }
                        if send(&mut socket, &RoomMessage::Board { board, from }).await.is_err() {
                            break;
                        }
                    }
                    Ok(message) => {
                        // The roster goes to everyone, sender included: a seat
                        // is single-writer, so it cannot fight anybody, and the
                        // sender still needs to see the server's version of its
                        // own id.
                        if send(&mut socket, &message).await.is_err() {
                            break;
                        }
                    }
                    // Lagged: the state is small and absolute, so the next
                    // update supersedes whatever was missed.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            // Something arrived from this client.
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<RoomMessage>(&text) {
                            Ok(RoomMessage::Board { board, .. }) => {
                                // `from` comes from the socket rather than the
                                // payload, so a client cannot impersonate
                                // another and suppress its echo.
                                state.rooms.publish_board(&code, board, &client_id);
                            }
                            Ok(RoomMessage::Seat { seat, .. }) => {
                                state.rooms.publish_seat(&code, seat, &client_id);
                            }
                            // Done with the session, as opposed to the socket
                            // dropping — the seat goes, and the slot it held
                            // goes back to the team.
                            Ok(RoomMessage::Leave { .. }) => {
                                state.rooms.remove_seat(&code, &client_id);
                            }
                            // A client from before sessions existed.
                            Ok(RoomMessage::Update { draft, .. }) => {
                                state.rooms.publish_legacy_draft(&code, draft, &client_id);
                            }
                            // Snapshot, Roster and Rejected are server-to-client
                            // only.
                            Ok(_) => {}
                            Err(err) => eprintln!("unparseable message in session {code}: {err}"),
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        eprintln!("socket error in session {code}: {err}");
                        break;
                    }
                }
            }
        }
    }
    // Dropping the membership marks the seat disconnected and announces it.
    drop(membership);
}

async fn send(socket: &mut WebSocket, message: &RoomMessage) -> Result<()> {
    let text = serde_json::to_string(message).context("serialising a room message")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .context("sending on the socket")?;
    Ok(())
}

/// End-to-end tests over a real socket.
///
/// `room.rs` covers the session logic directly; what only shows up here is the
/// wiring around it — the route table, the create-then-join split, and the rule
/// that a seat is owned by the socket it arrived on rather than by whatever the
/// payload claims. Those are exactly the parts a unit test cannot reach, and
/// exactly the parts whose failure looks like "sync just does not work".
#[cfg(test)]
mod e2e {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use overwatch_core::{Board, HeroId, Seat};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Starts the real router on an ephemeral port and returns its base URL.
    async fn serve() -> String {
        // These tests never post a match, but pointing six concurrent servers
        // at one path is a trap waiting for the first test that does.
        let matches = std::env::temp_dir().join(format!(
            "overwatch-e2e-matches-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or(0),
        ));
        serve_with(Some(MatchLog::new(matches))).await
    }

    async fn serve_with(matches: Option<MatchLog>) -> String {
        let state = AppState {
            rooms: Rooms::new(),
            matches,
        };
        let app = router(state, std::path::Path::new("/nonexistent"));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral port");
        let addr = listener.local_addr().expect("its address");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("127.0.0.1:{}", addr.port())
    }

    async fn create_session(base: &str) -> String {
        let body = reqwest_post(&format!("http://{base}/api/session")).await;
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        value["code"].as_str().expect("a code").to_owned()
    }

    /// A one-shot POST, hand-rolled: pulling in an HTTP client as a dev
    /// dependency to send eleven bytes would be a poor trade.
    async fn reqwest_post(url: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let rest = url.trim_start_matches("http://");
        let (host, path) = rest.split_once('/').expect("a path");
        let mut stream = tokio::net::TcpStream::connect(host)
            .await
            .expect("connects");
        stream
            .write_all(
                format!("POST /{path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("sends");

        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.expect("reads");
        raw.split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .expect("a body")
    }

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn join(base: &str, code: &str, id: &str, name: &str) -> Socket {
        let url = format!("ws://{base}/ws/{code}?id={id}&name={name}");
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("the socket connects");
        socket
    }

    async fn send(socket: &mut Socket, message: &RoomMessage) {
        let text = serde_json::to_string(message).expect("serialises");
        socket.send(WsMessage::Text(text)).await.expect("sends");
    }

    /// The next message of interest, skipping the roster chatter that joins and
    /// departures generate.
    async fn next_of_interest(socket: &mut Socket) -> RoomMessage {
        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                .await
                .expect("the server answers in time")
                .expect("a frame")
                .expect("a good frame");
            let WsMessage::Text(text) = frame else {
                continue;
            };
            let message: RoomMessage = serde_json::from_str(&text).expect("parses");
            if !matches!(message, RoomMessage::Roster { .. }) {
                return message;
            }
        }
    }

    async fn next_roster(socket: &mut Socket) -> Vec<Seat> {
        roster_until(socket, |_| true).await
    }

    /// The next roster satisfying `wanted`.
    ///
    /// Every join and departure broadcasts one, so a socket that has been open
    /// for two of those has two queued before the one the test is about. Waiting
    /// for the state rather than counting frames is what keeps these tests from
    /// depending on broadcast ordering.
    async fn roster_until(socket: &mut Socket, wanted: impl Fn(&[Seat]) -> bool) -> Vec<Seat> {
        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                .await
                .expect("the server answers in time")
                .expect("a frame")
                .expect("a good frame");
            let WsMessage::Text(text) = frame else {
                continue;
            };
            if let Ok(RoomMessage::Roster { seats }) = serde_json::from_str(&text) {
                if wanted(&seats) {
                    return seats;
                }
            }
        }
    }

    #[tokio::test]
    async fn a_session_is_created_then_joined_and_the_enemy_team_reaches_everyone() {
        let base = serve().await;
        let code = create_session(&base).await;

        let mut era = join(&base, &code, "era", "era").await;
        // The joiner is caught up before anything else.
        let first = next_of_interest(&mut era).await;
        assert!(matches!(first, RoomMessage::Snapshot { .. }), "{first:?}");

        let mut mika = join(&base, &code, "mika", "mika").await;
        let _ = next_of_interest(&mut mika).await;

        // Whoever reads the enemy comp first types it once...
        let mut board = Board::new();
        board.enemies.push(HeroId(3));
        board.enemies.push(HeroId(4));
        send(
            &mut era,
            &RoomMessage::Board {
                board: board.clone(),
                from: "era".to_owned(),
            },
        )
        .await;

        // ...and it lands on the other screen.
        let RoomMessage::Board { board: got, .. } = next_of_interest(&mut mika).await else {
            panic!("the board should have arrived");
        };
        assert_eq!(got.enemies, vec![HeroId(3), HeroId(4)]);
    }

    #[tokio::test]
    async fn joining_a_code_nobody_created_is_refused() {
        let base = serve().await;
        let mut lost = join(&base, "brave-otter-99", "era", "era").await;

        let message = next_of_interest(&mut lost).await;
        assert!(
            matches!(message, RoomMessage::Rejected { .. }),
            "a typo must say so rather than opening an empty session: {message:?}"
        );
    }

    /// The ownership rule, over a real socket: the id in the payload is a hint,
    /// and the connection is the authority.
    #[tokio::test]
    async fn a_seat_belongs_to_the_socket_it_arrived_on() {
        let base = serve().await;
        let code = create_session(&base).await;

        let mut era = join(&base, &code, "era", "era").await;
        let _ = next_of_interest(&mut era).await;
        let mut mika = join(&base, &code, "mika", "mika").await;
        let _ = next_of_interest(&mut mika).await;

        // mika claims era's seat.
        send(
            &mut mika,
            &RoomMessage::Seat {
                seat: Seat {
                    locked: Some(HeroId(7)),
                    ..Seat::new("era")
                },
                from: "era".to_owned(),
            },
        )
        .await;

        let seats = roster_until(&mut era, |seats| {
            seats.iter().any(|s| s.id == "mika" && s.locked.is_some())
        })
        .await;
        let era_seat = seats.iter().find(|s| s.id == "era").expect("era's seat");
        let mika_seat = seats.iter().find(|s| s.id == "mika").expect("mika's seat");
        assert_eq!(era_seat.locked, None, "era's pick is era's alone");
        assert_eq!(
            mika_seat.locked,
            Some(HeroId(7)),
            "the write lands on the seat that actually sent it"
        );
    }

    #[tokio::test]
    async fn a_teammates_pick_arrives_as_a_roster_update() {
        let base = serve().await;
        let code = create_session(&base).await;

        let mut era = join(&base, &code, "era", "era").await;
        let _ = next_of_interest(&mut era).await;
        let mut mika = join(&base, &code, "mika", "mika").await;
        let _ = next_of_interest(&mut mika).await;

        send(
            &mut mika,
            &RoomMessage::Seat {
                seat: Seat {
                    locked: Some(HeroId(11)),
                    ..Seat::new("mika")
                },
                from: "mika".to_owned(),
            },
        )
        .await;

        // The point of the whole feature: era did not type this, and can now
        // score against it as an ally.
        let seats = roster_until(&mut era, |seats| seats.iter().any(|s| s.locked.is_some())).await;
        let state = overwatch_core::SessionState {
            board: Board::new(),
            seats,
        };
        let dataset = overwatch_data::load().expect("the committed dataset loads");
        assert_eq!(
            state.draft_for(&dataset, "era").allies,
            vec![HeroId(11)],
            "a teammate locking in becomes your ally without you typing it"
        );
    }

    #[tokio::test]
    async fn a_reload_keeps_the_seat_and_the_board() {
        let base = serve().await;
        let code = create_session(&base).await;

        let mut era = join(&base, &code, "era", "era").await;
        let _ = next_of_interest(&mut era).await;

        let mut board = Board::new();
        board.enemies.push(HeroId(2));
        send(
            &mut era,
            &RoomMessage::Board {
                board,
                from: "era".to_owned(),
            },
        )
        .await;
        send(
            &mut era,
            &RoomMessage::Seat {
                seat: Seat {
                    locked: Some(HeroId(5)),
                    ..Seat::new("era")
                },
                from: "era".to_owned(),
            },
        )
        .await;
        // Let the writes land before dropping the socket.
        let _ = next_roster(&mut era).await;
        drop(era);

        let mut back = join(&base, &code, "era", "era").await;
        let RoomMessage::Snapshot { state } = next_of_interest(&mut back).await else {
            panic!("a rejoin should be caught up");
        };

        assert_eq!(state.board.enemies, vec![HeroId(2)], "the draft survives");
        assert_eq!(
            state.seat("era").and_then(|s| s.locked),
            Some(HeroId(5)),
            "and so does the hero you had locked"
        );
    }

    #[tokio::test]
    async fn the_health_endpoint_still_answers() {
        let base = serve().await;
        let code = create_session(&base).await;
        let mut _era = join(&base, &code, "era", "era").await;

        let body = reqwest_get(&format!("http://{base}/health")).await;
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["rooms"], 1);
    }

    /// Departing on purpose is a different event from a socket dropping, and
    /// only this end-to-end path proves the distinction survives the wire: the
    /// `from` the server acts on is the socket's, not the payload's.
    #[tokio::test]
    async fn an_explicit_leave_takes_the_seat_off_everyone_elses_roster() {
        let base = serve().await;
        let code = create_session(&base).await;

        let mut era = join(&base, &code, "era", "era").await;
        let _ = next_of_interest(&mut era).await;

        let mut mika = join(&base, &code, "mika", "mika").await;
        let _ = next_of_interest(&mut mika).await;
        mika_locks_in(&mut mika).await;

        // era waits until the roster actually holds both, so the assertion
        // below is about the departure rather than about arrival ordering.
        let both = roster_until(&mut era, |seats| seats.len() == 2).await;
        assert!(both.iter().any(|seat| seat.id == "mika"));

        send(
            &mut mika,
            &RoomMessage::Leave {
                // Deliberately a lie: the server must use the socket's id.
                from: "era".to_owned(),
            },
        )
        .await;

        let seats = roster_until(&mut era, |seats| seats.len() == 1).await;
        assert_eq!(seats[0].id, "era", "the wrong seat was taken");
    }

    async fn mika_locks_in(socket: &mut Socket) {
        send(
            socket,
            &RoomMessage::Seat {
                seat: Seat {
                    locked: Some(HeroId(8)),
                    ..Seat::new("mika")
                },
                from: "mika".to_owned(),
            },
        )
        .await;
    }

    /// The compatibility contract for the new message, from the other side: a
    /// client on a newer build talking to this server sends things it has never
    /// heard of, and a parse failure must not take the session down with it.
    #[tokio::test]
    async fn a_message_the_server_cannot_parse_does_not_end_the_session() {
        let base = serve().await;
        let code = create_session(&base).await;

        let mut era = join(&base, &code, "era", "era").await;
        let _ = next_of_interest(&mut era).await;
        let mut mika = join(&base, &code, "mika", "mika").await;
        let _ = next_of_interest(&mut mika).await;

        mika.send(WsMessage::Text(
            r#"{"type":"something-this-build-never-heard-of","from":"mika"}"#.to_owned(),
        ))
        .await
        .expect("sends");

        // The session carries on: the very next message still lands.
        let mut board = Board::new();
        board.enemies.push(HeroId(3));
        send(
            &mut mika,
            &RoomMessage::Board {
                board,
                from: "mika".to_owned(),
            },
        )
        .await;

        let RoomMessage::Board { board, .. } = next_of_interest(&mut era).await else {
            panic!("the board should still have arrived");
        };
        assert_eq!(board.enemies, vec![HeroId(3)]);
    }

    async fn reqwest_get(url: &str) -> String {
        raw_get(url)
            .await
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .expect("a body")
    }

    /// The status code of a bare GET, for the cases where the code is the
    /// whole point and the body is not.
    async fn status_of(url: &str) -> u16 {
        let raw = raw_get(url).await;
        let status = raw
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("a status line");
        status.parse().expect("a numeric status")
    }

    async fn raw_get(url: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let rest = url.trim_start_matches("http://");
        let (host, path) = rest.split_once('/').expect("a path");
        let mut stream = tokio::net::TcpStream::connect(host)
            .await
            .expect("connects");
        stream
            .write_all(
                format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("sends");

        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.expect("reads");
        raw
    }

    /// Switching the match log off has to actually close the endpoint.
    ///
    /// The failure this guards against is not a misbehaving handler but a
    /// disappearing route: drop `/api/matches` from the table instead of
    /// answering 404 on it and the SPA fallback picks it up, so the endpoint
    /// "works" — it returns 200 and the index page — and nothing looks wrong
    /// until someone checks what the public deployment is actually serving.
    #[tokio::test]
    async fn a_disabled_match_log_closes_the_endpoint() {
        let base = serve_with(None).await;
        assert_eq!(status_of(&format!("http://{base}/api/matches")).await, 404);

        // Only the match log goes: sync is the reason the server is reachable
        // at all, and it has to survive the switch.
        let code = create_session(&base).await;
        let mut era = join(&base, &code, "era", "era").await;
        assert!(matches!(
            next_of_interest(&mut era).await,
            RoomMessage::Snapshot { .. }
        ));
    }

    /// The other half of the pair: without this, the test above would still
    /// pass if `/api/matches` were broken for everyone.
    #[tokio::test]
    async fn an_enabled_match_log_serves_the_endpoint() {
        let base = serve().await;
        assert_eq!(status_of(&format!("http://{base}/api/matches")).await, 200);
    }
}
