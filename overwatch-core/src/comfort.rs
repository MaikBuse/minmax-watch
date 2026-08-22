//! How well you play a hero, as a thing you say rather than a thing inferred.
//!
//! A property of the *player's relationship to* a hero, which is why it lives
//! here rather than in [`crate::hero`]: nothing about the roster changes when you
//! say you can play Reinhardt.
//!
//! This is the vocabulary behind [`crate::score::Weights::personal`], the
//! second-heaviest term in the pick score and the one that has been zero for
//! every user the app has ever had, because nothing wrote
//! [`crate::UserContext::overrides`]. The ladder is what closes that.
//!
//! **Declared, not inferred.** The two `matchlog` module docs carry the full
//! argument for why match results are not an input: a result belongs to the team
//! rather than to your hero, the per-hero sample is single digits for months,
//! inferring from drafts the app itself steered is a closed loop with no external
//! check, and you already know which heroes you can play.

/// One rung of the comfort ladder.
///
/// Three positive steps and no negative one. Two would put 20 next to 100 with
/// nothing between; a negative is a *different statement* — "never suggest this"
/// — that would tax every clear-out, and "I am bad at this hero" is already said
/// by leaving the hero off the board.
///
/// **Deliberately not `Serialize`/`Deserialize`, and deliberately no
/// `as_str()`.** The repo's rule elsewhere is that a type on the wire carries a
/// stable `as_str()` key beside its display `label()`; this is the one vocabulary
/// that never goes on the wire. What is stored is the **value**, as an `i8` in
/// `StoredProfile.overrides`, and this enum is a view onto that number. Adding
/// the pair by reflex would invent a second representation of a thing that
/// already has one.
///
/// That view-onto-an-`i8` framing is also why the two lookups behave as they do:
/// [`Self::of`] is an exact match, so an off-ladder value reads honestly as "not
/// a named step", and [`Self::cycle`] climbs by comparison rather than by table
/// lookup, so every off-ladder value has somewhere sane to go on one click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComfortStep {
    /// You play it.
    Ok,
    /// You play it well.
    Good,
    /// This is your hero.
    Main,
}

impl ComfortStep {
    /// The ladder, ascending. [`Self::cycle`] walks this in order, so the
    /// ordering is load-bearing rather than cosmetic — `the_comfort_ladder_climbs
    /// _one_step_per_click_and_wraps_back_to_nothing` pins it.
    pub const LADDER: [ComfortStep; 3] = [ComfortStep::Ok, ComfortStep::Good, ComfortStep::Main];

    /// What this step puts on the canonical -100..=100 scale.
    ///
    /// Every number is chosen against a figure already in the scorer, at the
    /// shipped `personal` of 0.60:
    ///
    /// | step | value | contribution | what it claims |
    /// | --- | --- | --- | --- |
    /// | `ok` | 20 | 0.12 | you play it |
    /// | `good` | 55 | 0.33 | you play it well |
    /// | `main` | 100 | 0.60 | this is your hero |
    ///
    /// **`ok` at 0.12 sits below `swap_threshold` (0.15) on purpose**, by 0.03.
    /// The lowest step can mark a hero and can never on its own tell you to
    /// abandon a working pick, which is what makes it safe to migrate a whole
    /// legacy pool onto it unattended.
    /// `the_lowest_comfort_step_cannot_on_its_own_argue_for_a_swap` fails if
    /// either number moves, and it names both.
    ///
    /// `good` at 0.33 is about one strong counter contribution — the counter docs
    /// cite ~0.25 — so it wins a close matchup argument and loses a decisive one.
    /// `main` at 0.60 is the claim at full strength: it flips a hero across a 0.50
    /// counter gap by 0.10, so it is still a claim rather than an override.
    pub const fn value(self) -> i8 {
        match self {
            ComfortStep::Ok => 20,
            ComfortStep::Good => 55,
            ComfortStep::Main => 100,
        }
    }

    /// What the screen calls it. No `as_str()` beside this — see the type doc.
    pub const fn label(self) -> &'static str {
        match self {
            ComfortStep::Ok => "ok",
            ComfortStep::Good => "good",
            ComfortStep::Main => "main",
        }
    }

    /// The step this value *is*, or `None` for a value the ladder does not name.
    ///
    /// An exact match, deliberately. Rounding `21` onto `Ok` would be inventing a
    /// claim the stored profile does not make; `None` says what is true, and
    /// [`Self::cycle`] is what does something about it.
    pub fn of(value: i8) -> Option<Self> {
        Self::LADDER.into_iter().find(|step| step.value() == value)
    }

    /// One click: the first step strictly above `current`, or 0 past the top.
    ///
    /// A comparison and not a table lookup, so every value has somewhere to go
    /// rather than only the three the ladder names. `21` climbs to `55`, `120`
    /// falls off the top to `0`, and a negative — which no step produces, but a
    /// hand-edited profile can hold — climbs onto the bottom rung rather than
    /// being preserved as something the ladder cannot express.
    pub fn cycle(current: i8) -> i8 {
        Self::LADDER
            .into_iter()
            .find(|step| step.value() > current)
            .map_or(0, Self::value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::Weights;

    /// The whole interaction, in one line per click.
    #[test]
    fn the_comfort_ladder_climbs_one_step_per_click_and_wraps_back_to_nothing() {
        assert_eq!(ComfortStep::cycle(0), 20);
        assert_eq!(ComfortStep::cycle(20), 55);
        assert_eq!(ComfortStep::cycle(55), 100);
        assert_eq!(ComfortStep::cycle(100), 0, "the top step clears the hero");

        // Ascending, because `cycle` walks the array in order and takes the
        // first match. A ladder written out of order would skip steps.
        let values: Vec<i8> = ComfortStep::LADDER.iter().map(|s| s.value()).collect();
        let mut sorted = values.clone();
        sorted.sort_unstable();
        assert_eq!(values, sorted, "LADDER has to ascend for `cycle` to work");

        for step in ComfortStep::LADDER {
            assert_eq!(ComfortStep::of(step.value()), Some(step));
            assert!(!step.label().is_empty());
        }
    }

    /// A stored profile can hold anything — the values load unclamped, and a
    /// future ladder edit leaves old numbers behind. One click has to normalise
    /// them rather than getting stuck.
    #[test]
    fn a_comfort_value_the_ladder_does_not_name_still_climbs_to_the_next_step_above_it() {
        assert_eq!(ComfortStep::of(21), None, "21 is not a step, and says so");

        assert_eq!(ComfortStep::cycle(21), 55, "the next step strictly above");
        assert_eq!(ComfortStep::cycle(99), 100);
        assert_eq!(ComfortStep::cycle(120), 0, "past the top clears it");
        assert_eq!(
            ComfortStep::cycle(-50),
            20,
            "a negative climbs onto the ladder rather than being preserved as a \
             step that does not exist"
        );
        assert_eq!(ComfortStep::cycle(i8::MIN), 20);
        assert_eq!(ComfortStep::cycle(i8::MAX), 0);
    }

    /// The margin the whole ladder rests on, stated where the numbers are.
    ///
    /// The behavioural half of this lives in `tests/scoring.rs` as
    /// `the_lowest_comfort_step_cannot_on_its_own_argue_for_a_swap`, which drives
    /// it through `recommend`. This half is the arithmetic, and it is here so a
    /// reader editing `value()` meets it in the same file.
    #[test]
    fn the_lowest_step_is_worth_less_than_the_swap_threshold() {
        let w = Weights::default();
        let lowest = f32::from(ComfortStep::Ok.value()) / 100.0 * w.personal;

        assert!(
            lowest < w.swap_threshold,
            "the lowest comfort step is worth {lowest}, which is not under the \
             swap threshold of {} - marking a hero would start telling people to \
             abandon working picks",
            w.swap_threshold
        );
    }
}
