//! The complete arithmetic behind one score, as a ledger rather than as prose.
//!
//! [`crate::score::Reason`] is the other half and the two are not the same job.
//! Reasons are *sentences about terms that said something*: a term of exactly
//! zero never becomes one, because "pairs well with" built out of an empty cell
//! is a claim the data does not make. That rule is load-bearing and this module
//! does not touch it.
//!
//! What it means, though, is that the reason list can never add up to the score.
//! Two terms move a score while emitting nothing at all — a zero one by the rule
//! above, and the shape term against an enemy team whose axes do not commit,
//! because [`crate::Shape::leading`] has no archetype for the sentence to name.
//! Trying to close that gap inside the reason list ends in inventing reason
//! kinds for silence, which is the rule being protected.
//!
//! So: two lists, two jobs. Reasons are sorted by impact, cut to what fits a
//! column, and worded. This is the ledger — every term, in a fixed order, with
//! its zeros, no prose, summing to exactly the number on the row.

use crate::archetype::Shape;

/// One weighted term of the pick score.
///
/// The eight variants are every term in the sum and nothing else.
/// `Weights::prevalence` is not here because it reaches only the ban list, and
/// `Weights::swap_threshold` is not a term at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermKind {
    /// All-ranks patch strength.
    Base,
    /// The shift away from that at the rung you chose. Splits with [`Self::Base`]
    /// rather than doubling it — see [`crate::score::Weights::rank`].
    Rank,
    /// The weighted mean of this hero's matchups against the enemy board.
    Counter,
    /// The mean of the duo ratings with the allies already picked.
    Synergy,
    /// This hero's affinity for the map.
    Map,
    /// The attack/defend lean, on a map that has sides.
    Side,
    /// How this kit fares against the shape the enemy team is building.
    Shape,
    /// The comfort you declared for this hero.
    Personal,
}

impl TermKind {
    /// Every term, **in the order the score is summed**.
    ///
    /// The order is load-bearing twice over. [`Breakdown::total`] folds it, so it
    /// has to match the sum this replaced operand for operand or the arithmetic
    /// moves in the last bit; and the `why` panel renders it, where a table whose
    /// rows reorder between heroes is one you re-read every time.
    pub const ALL: [TermKind; 8] = [
        TermKind::Base,
        TermKind::Rank,
        TermKind::Counter,
        TermKind::Synergy,
        TermKind::Map,
        TermKind::Side,
        TermKind::Shape,
        TermKind::Personal,
    ];

    /// This kind's slot in [`Self::ALL`], so [`Breakdown::term`] can index rather
    /// than search.
    pub const fn index(self) -> usize {
        match self {
            TermKind::Base => 0,
            TermKind::Rank => 1,
            TermKind::Counter => 2,
            TermKind::Synergy => 3,
            TermKind::Map => 4,
            TermKind::Side => 5,
            TermKind::Shape => 6,
            TermKind::Personal => 7,
        }
    }

    /// The word for this term on screen.
    ///
    /// **No `as_str()` beside it**, unlike [`crate::Role`] and friends. That pair
    /// exists for values that go on the wire or into a stored profile, where the
    /// key has to survive a restyling of the label. Nothing persists a term kind,
    /// so a second string would be an alias with one caller.
    ///
    /// The words are the ones already on screen rather than the field names:
    /// the panel says `matchups` and `duos` because that is what the boards
    /// beside it say, and `your rung` because the rank picker does.
    pub const fn label(self) -> &'static str {
        match self {
            TermKind::Base => "patch",
            TermKind::Rank => "your rung",
            TermKind::Counter => "matchups",
            TermKind::Synergy => "duos",
            TermKind::Map => "map",
            TermKind::Side => "side",
            TermKind::Shape => "their shape",
            TermKind::Personal => "comfort",
        }
    }
}

/// One term, kept as the two numbers that made it rather than as their product.
///
/// The panel renders the product, so this could have been a single
/// `contribution`. It is not, for two reasons. A weight a stored profile changed
/// is a different explanation from a value the data supplied, and only these two
/// fields can tell them apart — a term at zero because you turned it off looks
/// identical to one at zero because nothing rated it. And keeping them apart is
/// what lets [`Breakdown::total`] multiply the same operands the sum it replaced
/// did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Term {
    pub kind: TermKind,
    /// The weight from [`crate::score::Weights`], as it was when this was scored.
    pub weight: f32,
    /// What the data said, before weighting, on roughly -1.0..=1.0.
    pub value: f32,
}

impl Term {
    /// What this term moved the score by.
    pub fn contribution(&self) -> f32 {
        self.weight * self.value
    }
}

/// How much of what was on the board actually fed a term.
///
/// Two counts and not a ratio, because the difference between them is the
/// sentence: "read against 2 of their 5 picks" is a statement about the draft,
/// and 0.4 is not. `entered == 0` is its own case — nothing to be read against
/// is not thin coverage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    /// How many carried a reading.
    pub rated: usize,
    /// How many were on the board at all.
    pub entered: usize,
}

/// Every term behind one hero's score, complete and adding up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Breakdown {
    /// In [`TermKind::ALL`] order, always all eight, zeros included.
    pub terms: [Term; 8],
    /// Enemies with a matchup reading, over enemies on the board.
    pub counter: Coverage,
    /// Allies with a duo reading, over allies on the board.
    pub synergy: Coverage,
    /// The shape of the enemy team this was scored against.
    ///
    /// Carried whole rather than as its label, because the panel has to
    /// distinguish [`Shape::is_mixed`] — rated, and committed to nothing — from
    /// unrated. The mixed case is the one term that can move a score with no
    /// reason line behind it, and this is the only thing that can say so.
    pub shape: Shape,
}

impl Default for Breakdown {
    /// The zero ledger: every term present, none of them computed.
    ///
    /// Every term still carries its own `kind`, so an empty breakdown is still
    /// in [`TermKind::ALL`] order and [`Self::term`] still answers.
    fn default() -> Self {
        Self {
            terms: TermKind::ALL.map(|kind| Term {
                kind,
                weight: 0.0,
                value: 0.0,
            }),
            counter: Coverage::default(),
            synergy: Coverage::default(),
            shape: Shape::default(),
        }
    }
}

impl Breakdown {
    /// One term by kind.
    pub fn term(&self, kind: TermKind) -> &Term {
        &self.terms[kind.index()]
    }

    /// The score.
    ///
    /// Not `.map(Term::contribution).sum()`, and not a fold from `0.0`, and the
    /// difference is not style. This replaced a hand-written chain of eight
    /// `+`, and f32 addition is not associative — so the sum is seeded from the
    /// first term and added left to right, exactly as that chain did, and every
    /// golden in the scoring tests keeps its value to the last bit.
    ///
    /// A fold from zero agrees on all but one input, and it is worth naming
    /// exactly because it is narrower than it first looks: a ledger where **every
    /// one of the eight** is `-0.0`, which a stored profile with negative weights
    /// reaches on a hero with no data. `0.0 + -0.0` is `+0.0` while
    /// `-0.0 + -0.0` is `-0.0`, so the fold loses the sign the old chain kept and
    /// the row prints `+0` where it printed `-0`. A single negative zero among
    /// positives is *not* the case — that one is `+0.0` either way.
    pub fn total(&self) -> f32 {
        let mut total = self.terms[0].contribution();
        for term in &self.terms[1..] {
            total += term.contribution();
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The array and the lookup have to agree, or `term()` silently answers with
    /// its neighbour.
    #[test]
    fn every_term_kind_sits_where_its_index_says_it_does() {
        for (slot, kind) in TermKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.index(), slot, "{kind:?} is out of place");
        }
    }

    /// The ledger is rendered as a table of labelled rows, so two terms sharing a
    /// word would be two rows nobody can tell apart.
    #[test]
    fn no_two_terms_answer_to_the_same_word() {
        let mut seen: Vec<&str> = Vec::new();
        for kind in TermKind::ALL {
            let label = kind.label();
            assert!(!label.is_empty(), "{kind:?} has no word");
            assert!(!seen.contains(&label), "{label} names two terms");
            seen.push(label);
        }
    }

    /// The whole promise of the type, on numbers small enough to check by hand.
    #[test]
    fn the_terms_are_summed_in_the_order_they_are_declared() {
        let mut breakdown = Breakdown::default();
        for (slot, term) in breakdown.terms.iter_mut().enumerate() {
            term.weight = 0.5;
            term.value = slot as f32;
        }

        // 0.5 * (0 + 1 + ... + 7), assembled the way the score is.
        assert_eq!(breakdown.total(), 14.0);
        assert_eq!(breakdown.term(TermKind::Shape).value, 6.0);
    }

    /// The one input where seeding from the first term and folding from zero
    /// disagree, which is the whole argument in [`Breakdown::total`]'s comment.
    ///
    /// Every term at `-0.0` — a stored profile with negative weights, on a hero
    /// nothing has rated. The chain this replaced kept the sign; a fold from
    /// `0.0` does not, and the row would print `+0` where it used to print `-0`.
    #[test]
    fn a_ledger_of_negative_zeroes_keeps_the_sign_the_old_sum_gave_it() {
        let mut breakdown = Breakdown::default();
        for term in &mut breakdown.terms {
            term.weight = -0.15;
            term.value = 0.0;
        }

        assert!(breakdown.terms[0].contribution().is_sign_negative());
        assert!(
            breakdown.total().is_sign_negative(),
            "the sum was re-associated through a zero seed"
        );
    }

    /// A ledger nothing has been written into still has all eight rows, because
    /// the panel draws them whether or not the draft filled them.
    #[test]
    fn an_empty_ledger_still_carries_every_term_and_comes_to_nothing() {
        let breakdown = Breakdown::default();

        assert_eq!(breakdown.total(), 0.0);
        for kind in TermKind::ALL {
            assert_eq!(breakdown.term(kind).kind, kind);
        }
        assert!(!breakdown.shape.is_rated());
    }
}
