//! The chord table.
//!
//! Four keys and an Escape, and one rule that makes them a table rather than a
//! list to memorise: **ctrl builds, alt costs.** `^L` adds a pick and takes
//! nothing, so it sits on ctrl. Everything behind alt gives something up —
//! `⌥W`/`⌥L` clear the draft to record a result, and `⌥R` gives up a pick the
//! role it moves you to cannot hold. Reaching for alt is the guard against a
//! stray keypress costing you the picks you just entered.
//!
//! Matching is on [`Code`] — the *physical* key — rather than on the character
//! the key produced. That is not a detail. `evt.key()` gives what the layout
//! and the modifiers composed, so on macOS `⌥W` arrives as `"∑"` and `⌥L` as
//! `"¬"`, and on a non-US layout the letters are somewhere else entirely; a
//! table written against characters is dead on both. Caps Lock breaks it too.
//! `Code::KeyW` is the key under the finger whatever it prints.
//!
//! The table lives here as a pure function with tests rather than inline in the
//! handler, for the same reason the ally board's rules do: it is the part that
//! can be wrong in a way a test can catch, and nothing can reach it inside an
//! event closure.

use dioxus::prelude::{Code, Modifiers};

/// What a chord asks for, separate from what it takes to carry out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Clear the picks — the shared board's and your own — keeping the map.
    Clear,
    /// Take the hero at the top of the pick column.
    LockTop,
    /// Move to the next pick mode, giving up a pick it cannot hold.
    NextRole,
    /// Write the result down and start the next draft.
    Record { won: bool },
}

/// The chord this key press asks for, or `None` for every key that is not ours.
///
/// Alt is tested before ctrl, and demands that ctrl and meta are *absent*. Both
/// halves matter: `^L` and `⌥L` are the same physical key, so without the order
/// they would be ambiguous, and without the exclusion `^⌥L` would silently
/// record a loss when it was meant to do nothing at all.
pub fn command_for(code: Code, mods: Modifiers) -> Option<Command> {
    if code == Code::Escape {
        return Some(Command::Clear);
    }

    // Everything that costs you something.
    if mods.alt() && !mods.ctrl() && !mods.meta() {
        return match code {
            Code::KeyW => Some(Command::Record { won: true }),
            Code::KeyL => Some(Command::Record { won: false }),
            Code::KeyR => Some(Command::NextRole),
            _ => None,
        };
    }

    // And the one that does not. Meta as well as ctrl, so the chord is the one
    // the hand already makes on whichever machine it is on — but alt excluded
    // here too, so that a chord is exactly its modifiers in both directions.
    // `^⌥L` is nothing rather than a lock: a combination nobody meant to press
    // should do nothing, not guess at the nearest thing.
    if (mods.ctrl() || mods.meta()) && !mods.alt() {
        return match code {
            Code::KeyL => Some(Command::LockTop),
            _ => None,
        };
    }

    None
}

/// The chords, and what they do, for the sheet behind the header button.
///
/// The one place either is written down. A handler and a help text that are
/// maintained apart drift, and a shortcut list that lies is worse than none —
/// which is what the test at the bottom of this file is for.
pub const SHORTCUTS: [(&str, &str); 5] = [
    ("^L", "take the top pick"),
    ("⌥R", "next role"),
    ("⌥W", "record a win"),
    ("⌥L", "record a loss"),
    ("Esc", "clear the picks"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers::CONTROL
    }
    fn meta() -> Modifiers {
        Modifiers::META
    }
    fn alt() -> Modifiers {
        Modifiers::ALT
    }
    fn none() -> Modifiers {
        Modifiers::empty()
    }

    #[test]
    fn every_chord_resolves_to_its_own_command() {
        assert_eq!(command_for(Code::Escape, none()), Some(Command::Clear));
        assert_eq!(command_for(Code::KeyL, ctrl()), Some(Command::LockTop));
        assert_eq!(command_for(Code::KeyR, alt()), Some(Command::NextRole));
        assert_eq!(
            command_for(Code::KeyW, alt()),
            Some(Command::Record { won: true })
        );
        assert_eq!(
            command_for(Code::KeyL, alt()),
            Some(Command::Record { won: false })
        );
    }

    /// The rule the table is built on. Everything that gives something up sits
    /// behind alt, so reaching for it is the guard.
    #[test]
    fn everything_that_costs_you_something_sits_behind_alt() {
        for code in [Code::KeyW, Code::KeyL, Code::KeyR] {
            let behind_alt = command_for(code, alt());
            assert!(
                matches!(behind_alt, Some(Command::Record { .. } | Command::NextRole)),
                "{code:?} with alt should cost something, got {behind_alt:?}"
            );
        }
        // And the one that costs nothing is the one that does not need it.
        assert_eq!(command_for(Code::KeyL, ctrl()), Some(Command::LockTop));
    }

    /// A chord is the key under the finger, not the character it printed.
    ///
    /// This is the macOS case: `⌥W` composes `"∑"` there and `⌥L` composes
    /// `"¬"`, so a table written against `evt.key()` answers neither. Matching
    /// `Code` means the layout never enters into it.
    #[test]
    fn a_chord_is_the_physical_key_so_a_composed_character_cannot_break_it() {
        assert_eq!(
            command_for(Code::KeyW, alt()),
            Some(Command::Record { won: true }),
            "the key that prints ∑ on a Mac is still the win key"
        );
        assert_eq!(
            command_for(Code::KeyL, alt()),
            Some(Command::Record { won: false }),
            "and the one that prints ¬ is still the loss key"
        );
    }

    /// `Code` carries no case, so the shift state cannot reach the table. The
    /// character-based version matched only lowercase on the ctrl chords, which
    /// meant Caps Lock silently turned them off.
    #[test]
    fn caps_lock_does_not_stop_a_chord_resolving() {
        // Shift held is the same physical key and the same command.
        assert_eq!(
            command_for(Code::KeyL, ctrl() | Modifiers::SHIFT),
            Some(Command::LockTop)
        );
        assert_eq!(
            command_for(Code::KeyR, alt() | Modifiers::SHIFT),
            Some(Command::NextRole)
        );
    }

    /// `^L` and `⌥L` are one key and two commands. Nothing may collapse them.
    #[test]
    fn ctrl_and_alt_on_one_key_are_two_different_commands() {
        assert_eq!(command_for(Code::KeyL, ctrl()), Some(Command::LockTop));
        assert_eq!(
            command_for(Code::KeyL, alt()),
            Some(Command::Record { won: false })
        );
        assert_ne!(
            command_for(Code::KeyL, ctrl()),
            command_for(Code::KeyL, alt())
        );
    }

    /// Holding both is a chord nobody meant to press, and recording a loss is
    /// far too expensive a thing to do on a guess.
    #[test]
    fn ctrl_and_alt_together_is_not_mistaken_for_alt() {
        assert_eq!(command_for(Code::KeyL, ctrl() | alt()), None);
        assert_eq!(command_for(Code::KeyW, ctrl() | alt()), None);
        assert_eq!(command_for(Code::KeyR, meta() | alt()), None);
    }

    /// The map filter sits under the same root handler, so a bare letter has to
    /// mean nothing at all — typing "route 66" must not walk the pick modes.
    #[test]
    fn a_bare_letter_is_never_a_chord() {
        for code in [Code::KeyW, Code::KeyL, Code::KeyR, Code::KeyA, Code::Digit6] {
            assert_eq!(command_for(code, none()), None, "{code:?} unmodified");
        }
    }

    /// Moving the role switch to alt gives the browser its reload back. Pinned
    /// because losing it again would be silent — the page would simply stop
    /// reloading and nobody would connect it to this table.
    #[test]
    fn ctrl_r_is_not_a_chord_so_the_browser_keeps_its_reload() {
        assert_eq!(command_for(Code::KeyR, ctrl()), None);
        assert_eq!(command_for(Code::KeyR, meta()), None);
    }

    /// The sheet is documentation, and documentation that names a key the
    /// handler ignores is worse than none. Every row has to be real.
    #[test]
    fn the_help_sheet_lists_exactly_the_chords_the_handler_answers_to() {
        let resolved = |chord: &str| match chord {
            "^L" => command_for(Code::KeyL, ctrl()),
            "⌥R" => command_for(Code::KeyR, alt()),
            "⌥W" => command_for(Code::KeyW, alt()),
            "⌥L" => command_for(Code::KeyL, alt()),
            "Esc" => command_for(Code::Escape, none()),
            other => panic!("the sheet names {other}, which this test cannot press"),
        };

        let mut seen = Vec::new();
        for (chord, what) in SHORTCUTS {
            let command = resolved(chord);
            assert!(
                command.is_some(),
                "the sheet promises {chord} ({what}) and the table ignores it"
            );
            seen.push(command);
        }

        seen.sort_by_key(|c| format!("{c:?}"));
        seen.dedup();
        assert_eq!(seen.len(), SHORTCUTS.len(), "two rows share one command");
    }
}
