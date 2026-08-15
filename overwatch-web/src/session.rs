//! Joining, sharing and leaving a session.
//!
//! The browser-facing half of this is three lines of `web-sys`; everything
//! interesting is string handling, and string handling is where a share link
//! quietly stops working. So the parsing and formatting live here as plain
//! functions with tests, and the parts that need a `window` are kept down to
//! the thinnest possible wrappers around them.
//!
//! The rule the parser follows: **anything a person could plausibly paste
//! should join the right session.** They will paste the whole URL, or the code
//! with a trailing full stop, or the code as it was capitalised in a chat
//! message. Making them clean it up first would be a worse feature than not
//! having one.

use overwatch_core::Seat;
use wasm_bindgen::JsCast;

/// The query parameter a share link carries.
const PARAM: &str = "s";

/// Where this client stands with respect to a session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Membership {
    /// Drafting alone. The app's original mode, and still a first-class one.
    #[default]
    Alone,
    In(String),
}

impl Membership {
    pub fn code(&self) -> Option<&str> {
        match self {
            Membership::Alone => None,
            Membership::In(code) => Some(code),
        }
    }
}

/// Folds anything paste-shaped into a session code.
///
/// Mirrors `overwatch_server::code::normalise`, and then some: the server sees
/// only what this sends, so the URL handling belongs on this side.
pub fn parse_code(raw: &str) -> Option<String> {
    let raw = raw.trim();

    // A pasted share link. Taken apart by hand rather than with `Url`, which
    // would be a dependency for one `split`, and which would reject the
    // half-links people actually paste ("192.168.1.5:8080/?s=brave-otter-41").
    let candidate = match raw.split_once(&format!("{PARAM}=")) {
        Some((_, tail)) => tail.split(['&', '#']).next().unwrap_or(tail),
        None => raw,
    };

    let code: String = candidate
        .trim()
        .trim_start_matches('#')
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();

    // Trailing punctuation survives the filter above only as a dash, which is
    // legal mid-code but never terminal.
    let code = code.trim_matches('-').to_owned();

    (!code.is_empty() && code.len() <= 64).then_some(code)
}

/// The link to hand a teammate.
///
/// `origin` is the app's own origin, so the link points wherever this client
/// reached the server from — which on a LAN is the one address that is known to
/// work from another machine.
pub fn share_url(origin: &str, code: &str) -> String {
    format!("{}/?{PARAM}={}", origin.trim_end_matches('/'), code)
}

/// The code in the address bar, if the page was opened from a share link.
pub fn code_from_location() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    if search.is_empty() {
        return None;
    }
    parse_code(&search)
}

/// This page's origin, for building a share link.
pub fn origin() -> Option<String> {
    web_sys::window()?.location().origin().ok()
}

/// Drops the query string from the address bar without reloading.
///
/// A share link that stays in the URL is a small trap: the code outlives the
/// session, so a refresh next week tries to rejoin something long swept away,
/// and a bookmark quietly becomes wrong. Joining is recorded in the profile
/// instead, which is where the rest of the sticky state already lives.
pub fn clear_query() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(history) = window.history() else {
        return;
    };
    let path = window
        .location()
        .pathname()
        .unwrap_or_else(|_| "/".to_owned());
    let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&path));
}

/// Copies text to the clipboard, best-effort.
///
/// Fire-and-forget: the clipboard API is asynchronous and permission-gated, and
/// there is nothing useful to do when it says no. The link is on screen and
/// selectable either way, which is the actual fallback.
pub fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let _ = window.navigator().clipboard().write_text(text);
}

/// Asks the server for a new session code.
///
/// Returns `None` when there is no server — which is the ordinary case for
/// `just dev`, and not an error worth interrupting anyone over.
pub async fn create() -> Option<String> {
    let window = web_sys::window()?;
    let init = web_sys::RequestInit::new();
    init.set_method("POST");

    let request = web_sys::Request::new_with_str_and_init("/api/session", &init).ok()?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request))
        .await
        .ok()?;
    let response: web_sys::Response = response.dyn_into().ok()?;
    if !response.ok() {
        return None;
    }

    let json = wasm_bindgen_futures::JsFuture::from(response.json().ok()?)
        .await
        .ok()?;
    let minted: Minted = serde_wasm_bindgen::from_value(json).ok()?;
    parse_code(&minted.code)
}

#[derive(serde::Deserialize)]
struct Minted {
    code: String,
}

/// Orders the roster so it reads the same on every screen.
///
/// You first, because you are looking for your own row; then everyone still
/// connected; then the people who have dropped, who are still on the team but
/// are not doing anything. Within each group the join order is kept, so the
/// list does not reshuffle itself every time somebody locks in.
pub fn order_roster(seats: &[Seat], me: &str) -> Vec<Seat> {
    let mut ordered: Vec<Seat> = seats.to_vec();
    ordered.sort_by_key(|seat| {
        let rank = if seat.id == me {
            0
        } else if seat.connected {
            1
        } else {
            2
        };
        (
            rank,
            seats.iter().position(|s| s.id == seat.id).unwrap_or(0),
        )
    });
    ordered
}

/// A QR of `data`, as an SVG document.
///
/// Rendered as one `<path>` rather than a rect per module: a 25×25 symbol is
/// several hundred dark modules, and that many elements is a real cost to lay
/// out for something displayed at 160 pixels across. `shape-rendering:
/// crispEdges` keeps the modules square at any scale.
///
/// Returns `None` for input the encoder cannot fit, which for a LAN URL should
/// never happen — but a panicking QR would take the whole draft screen with it.
pub fn qr_svg(data: &str) -> Option<String> {
    use qrcodegen::{QrCode, QrCodeEcc};

    // Low correction: this is read from a bright screen at arm's length, not
    // off a crumpled receipt, and less correction means a coarser grid that
    // phones lock onto faster.
    let qr = QrCode::encode_text(data, QrCodeEcc::Low).ok()?;
    let size = qr.size();
    // One module of quiet zone all round. The spec asks for four; at this
    // display size the panel's own padding is doing that job, and four would
    // shrink the modules for no gain.
    let margin = 1;
    let extent = size + margin * 2;

    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{} {}h1v1h-1z", x + margin, y + margin));
            }
        }
    }

    // `r##"..."##`: the fill colours contain `"#`, which would close a plain
    // raw string.
    Some(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {extent} {extent}" shape-rendering="crispEdges"><rect width="{extent}" height="{extent}" fill="#fff"/><path d="{path}" fill="#000"/></svg>"##
    ))
}

/// A QR as a `data:` URL, ready for an `<img src>` or a CSS background.
pub fn qr_data_url(data: &str) -> Option<String> {
    let svg = qr_svg(data)?;
    // Percent-encoded rather than base64: there is no base64 encoder in the
    // tree, and an SVG this small is barely larger this way.
    Some(format!("data:image/svg+xml,{}", percent_encode(&svg)))
}

/// Percent-encodes the characters that would end a `data:` URL early or be
/// read as markup.
///
/// Hand-rolled rather than `js_sys::encode_uri_component`, which panics outside
/// a browser and would therefore make every function that touches it untestable
/// — including this one, where a missed character is exactly the kind of bug
/// that only shows up as an image that will not render.
fn percent_encode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            // Unreserved, plus the punctuation an SVG path is made of. Leaving
            // these literal keeps the URL readable in devtools.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => out.push(byte as char),
            b'-' | b'_' | b'.' | b'~' | b'/' | b':' | b'=' | b'(' | b')' | b',' => {
                out.push(byte as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_link_round_trips_through_its_own_parser() {
        let url = share_url("http://192.168.1.5:8080", "brave-otter-41");
        assert_eq!(url, "http://192.168.1.5:8080/?s=brave-otter-41");
        assert_eq!(parse_code(&url).as_deref(), Some("brave-otter-41"));
    }

    #[test]
    fn a_trailing_slash_on_the_origin_does_not_double_up() {
        assert_eq!(
            share_url("http://localhost:8080/", "x-y-12"),
            "http://localhost:8080/?s=x-y-12"
        );
    }

    #[test]
    fn a_pasted_link_joins_the_same_session_as_a_pasted_code() {
        let from_link = parse_code("http://192.168.1.5:8080/?s=brave-otter-41");
        let from_code = parse_code("brave-otter-41");

        assert_eq!(from_link, from_code);
        assert_eq!(from_code.as_deref(), Some("brave-otter-41"));
    }

    #[test]
    fn a_bare_query_string_is_understood() {
        // What `location.search()` actually hands back.
        assert_eq!(
            parse_code("?s=brave-otter-41").as_deref(),
            Some("brave-otter-41")
        );
    }

    #[test]
    fn other_query_parameters_do_not_confuse_it() {
        assert_eq!(
            parse_code("?debug=1&s=brave-otter-41&x=2").as_deref(),
            Some("brave-otter-41")
        );
        assert_eq!(
            parse_code("http://host/?s=brave-otter-41#top").as_deref(),
            Some("brave-otter-41")
        );
    }

    /// The three ways a code arrives mangled from a chat window.
    #[test]
    fn a_code_survives_being_read_off_a_screen() {
        assert_eq!(
            parse_code("  Brave-Otter-41  ").as_deref(),
            Some("brave-otter-41")
        );
        assert_eq!(
            parse_code("brave-otter-41.").as_deref(),
            Some("brave-otter-41")
        );
        assert_eq!(
            parse_code("#brave-otter-41").as_deref(),
            Some("brave-otter-41")
        );
    }

    #[test]
    fn nothing_paste_shaped_is_not_a_code() {
        assert_eq!(parse_code(""), None);
        assert_eq!(parse_code("   "), None);
        assert_eq!(parse_code("?s="), None);
        assert_eq!(parse_code(&"x".repeat(65)), None);
    }

    #[test]
    fn being_alone_is_a_state_with_no_code() {
        assert_eq!(Membership::default(), Membership::Alone);
        assert_eq!(Membership::Alone.code(), None);
        assert_eq!(
            Membership::In("brave-otter-41".to_owned()).code(),
            Some("brave-otter-41")
        );
    }

    fn seat(id: &str, connected: bool) -> Seat {
        Seat {
            connected,
            ..Seat::new(id)
        }
    }

    #[test]
    fn the_roster_puts_you_first_and_the_dropped_last() {
        let seats = vec![
            seat("a", true),
            seat("gone", false),
            seat("me", true),
            seat("b", true),
        ];

        let ids: Vec<String> = order_roster(&seats, "me")
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["me", "a", "b", "gone"]);
    }

    /// The list sits next to a draft in progress, so it must not reshuffle
    /// itself every time somebody picks — only when someone joins or drops.
    #[test]
    fn the_roster_keeps_join_order_within_a_group() {
        let seats = vec![seat("c", true), seat("a", true), seat("b", true)];

        let ids: Vec<String> = order_roster(&seats, "nobody")
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec!["c", "a", "b"], "join order, not alphabetical");
    }

    #[test]
    fn a_qr_encodes_a_share_link() {
        let svg = qr_svg("http://192.168.1.5:8080/?s=brave-otter-41").expect("encodes");

        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.contains("viewBox"), "{svg}");
        assert!(
            svg.contains("<path"),
            "the modules have to actually be drawn"
        );
        assert!(svg.ends_with("</svg>"));
    }

    #[test]
    fn a_qr_becomes_something_an_img_can_show() {
        let url = qr_data_url("http://192.168.1.5:8080/?s=brave-otter-41").expect("encodes");

        assert!(url.starts_with("data:image/svg+xml,"), "{}", &url[..40]);
        // The characters that would end the URL early or be read as markup.
        // A raw one of these is an image that silently will not render.
        for bad in ['<', '>', '"', '#', '&'] {
            assert!(!url.contains(bad), "unencoded {bad:?} in the data URL");
        }
    }

    #[test]
    fn the_encoder_escapes_what_would_break_a_data_url() {
        assert_eq!(percent_encode("<svg>"), "%3Csvg%3E");
        // `=` is in the safe set and stays literal; the quotes and the hash do
        // not, and those are the ones that would break the URL.
        assert_eq!(percent_encode(r##"fill="#fff""##), "fill=%22%23fff%22");
        assert_eq!(percent_encode("M1 2h1v1z"), "M1%202h1v1z");
        assert_eq!(
            percent_encode("abcXYZ019-_.~/:=(),"),
            "abcXYZ019-_.~/:=(),",
            "the safe set must survive untouched, or the URL doubles in size"
        );
    }
}
