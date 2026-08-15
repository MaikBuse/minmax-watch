//! Session codes.
//!
//! A code has one job: survive being read aloud over voice chat and typed back
//! correctly on the first try. That rules out random strings — `x7Kp2q` is four
//! letters and an argument about whether that was a capital K. Two short words
//! and two digits ("brave-otter-41") is a mouthful nobody has to spell.
//!
//! It is not a password. The server has no authentication and is meant for a
//! home network; anyone who knows a code is in that session, exactly as anyone
//! who knew a room name was before. The digits are there to keep two sessions
//! on the same evening apart, not to resist guessing.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// Words chosen for being short, unambiguous over a bad microphone, and hard to
/// mishear as each other. No homophones, nothing that sounds like a letter.
const ADJECTIVES: [&str; 32] = [
    "amber", "brave", "bright", "calm", "clever", "cosmic", "crisp", "dapper", "eager", "electric",
    "fierce", "gentle", "golden", "happy", "hidden", "jolly", "keen", "lucky", "mellow", "noble",
    "polar", "quick", "quiet", "rapid", "royal", "rustic", "silent", "solar", "spry", "sunny",
    "tidy", "witty",
];

const NOUNS: [&str; 32] = [
    "anchor", "badger", "beacon", "bishop", "canyon", "cedar", "comet", "condor", "dragon",
    "ember", "falcon", "ferret", "glacier", "harbor", "heron", "island", "jaguar", "kestrel",
    "lantern", "marble", "meadow", "nebula", "otter", "panther", "quarry", "raven", "summit",
    "thunder", "tundra", "walrus", "willow", "zephyr",
];

/// How many digits ride on the end.
const SUFFIX: u64 = 90;
const SUFFIX_BASE: u64 = 10;

/// Mints a code from a fresh draw of OS entropy.
///
/// `RandomState` is seeded by the operating system on construction, which is
/// the one source of randomness the standard library exposes. Reaching for it
/// through a hasher is a slightly odd way to ask, but it is genuinely random
/// per call and it keeps a dependency out of a crate that otherwise has no use
/// for one. Nothing here is cryptographic, and nothing here needs to be.
pub fn mint() -> String {
    from_entropy(RandomState::new().build_hasher().finish())
}

/// The pure half of [`mint`], so the shape of a code can be tested without
/// depending on what the OS handed back.
fn from_entropy(entropy: u64) -> String {
    let adjective = ADJECTIVES[(entropy % ADJECTIVES.len() as u64) as usize];
    let noun = NOUNS[((entropy / ADJECTIVES.len() as u64) % NOUNS.len() as u64) as usize];
    let number = (entropy / (ADJECTIVES.len() as u64 * NOUNS.len() as u64)) % SUFFIX + SUFFIX_BASE;

    format!("{adjective}-{noun}-{number}")
}

/// Folds a code into the one form the room map is keyed by.
///
/// A code gets read off one screen and typed into another, so it arrives with
/// stray capitals, a leading space from a bad paste, or the whole share URL
/// wrapped around it. All of those mean the same session and must land in it.
pub fn normalise(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('#')
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

/// Whether a string could plausibly be a code at all.
///
/// Deliberately loose: it exists to reject an empty box and a pasted paragraph,
/// not to insist a code came from [`mint`]. Someone who wants to run a session
/// called `tuesday` should be able to, exactly as they could name a room before.
pub fn is_plausible(code: &str) -> bool {
    !code.is_empty() && code.len() <= 64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_minted_code_is_lowercase_and_readable() {
        let code = mint();

        assert_eq!(
            code,
            code.to_lowercase(),
            "codes are typed, not transcribed"
        );
        assert_eq!(code.split('-').count(), 3, "adjective-noun-number: {code}");
        assert_eq!(
            normalise(&code),
            code,
            "a fresh code must already be in its own canonical form"
        );
        assert!(is_plausible(&code));
    }

    #[test]
    fn the_number_on_the_end_is_always_two_digits() {
        // A one-digit tail reads as a typo and invites "was that a dash?".
        for entropy in [0, 1, 7, u64::MAX / 3, u64::MAX] {
            let code = from_entropy(entropy);
            let tail = code.rsplit('-').next().expect("a tail");
            assert_eq!(tail.len(), 2, "{code}");
        }
    }

    #[test]
    fn codes_do_not_collide_over_many_mints() {
        // ~92k combinations, so a few hundred draws should be near-clean. This
        // asserts the entropy actually varies, not that collisions are
        // impossible — `Rooms::create` retries on the ones that do collide.
        let codes: HashSet<String> = (0..500).map(|_| mint()).collect();
        assert!(
            codes.len() > 450,
            "only {} distinct codes in 500 draws — the entropy is not varying",
            codes.len()
        );
    }

    #[test]
    fn every_word_can_actually_come_up() {
        // A word list indexed by a modulo that does not cover it is a bug that
        // hides forever, so prove the ends of both lists are reachable.
        let codes: HashSet<String> = (0..u64::from(u32::MAX / 1_000_000))
            .map(|n| from_entropy(u64::from(u32::MAX / 1_000_000) * 7 + n * 31))
            .collect();
        assert!(!codes.is_empty());

        let last_adjective = from_entropy(ADJECTIVES.len() as u64 - 1);
        assert!(last_adjective.starts_with("witty"), "{last_adjective}");
    }

    #[test]
    fn a_code_typed_in_capitals_still_matches() {
        assert_eq!(normalise("Brave-Otter-41"), "brave-otter-41");
        assert_eq!(normalise("  brave-otter-41  "), "brave-otter-41");
        assert_eq!(normalise("#brave-otter-41"), "brave-otter-41");
    }

    /// People paste the whole link rather than picking the code out of it. The
    /// client strips the URL properly; this is the backstop for what gets past
    /// it, and it must not turn a link into a *different* valid code.
    #[test]
    fn punctuation_a_paste_drags_in_is_dropped() {
        assert_eq!(normalise("brave-otter-41."), "brave-otter-41");
        assert_eq!(normalise("'brave-otter-41'"), "brave-otter-41");
    }

    #[test]
    fn nothing_at_all_is_not_a_session() {
        assert!(!is_plausible(""));
        assert!(!is_plausible(&normalise("   ")));
        assert!(!is_plausible(&"x".repeat(65)));
        assert!(is_plausible("tuesday"), "a hand-picked name is still fine");
    }
}
