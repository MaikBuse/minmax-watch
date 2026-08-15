use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::hero::HeroId;

/// A dense `n x n` table of hero-vs-hero readings on the canonical -100..=100
/// scale, stored row-major in one flat allocation.
///
/// `rating(a, b)` reads "how well does `a` do against `b`", from `a`'s point of
/// view: positive is good for `a`. A cell is `None` until some source rates it,
/// which is *not* the same as a rating of zero — a quarter of the committed
/// matchups are a measured dead even, and telling those apart from "nobody has an
/// opinion" is the difference between averaging evidence and averaging its
/// absence. Cells are `Option` rather than a parallel presence bitmap so the two
/// cannot drift apart.
///
/// The table is deliberately *not* forced to be antisymmetric. The primary source
/// already is (it rates every pair from both sides such that the two readings sum
/// to a fixed total), but the secondary one is not, and it only covers part of
/// each row — so `(a, b)` and `(b, a)` can disagree by however much that source
/// moved one direction and not the other, and either one can be rated while the
/// other is not. Keeping both is what lets the scorer average whichever readings
/// exist and land back on a symmetric answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Matrix {
    n: usize,
    cells: Vec<Option<i8>>,
}

impl Matrix {
    /// An `n x n` table with nothing rated yet.
    pub fn unrated(n: usize) -> Self {
        Self {
            n,
            cells: vec![None; n * n],
        }
    }

    pub fn n(&self) -> usize {
        self.n
    }

    /// The reading for this pair, or `None` if no source rated it.
    ///
    /// Out-of-range lookups read as `None` rather than panicking. A draft in
    /// progress must never take down the UI over a bad index.
    pub fn rating(&self, attacker: HeroId, defender: HeroId) -> Option<i8> {
        let (a, d) = (attacker.index(), defender.index());
        if a >= self.n || d >= self.n {
            return None;
        }
        self.cells.get(a * self.n + d).copied().flatten()
    }

    /// The reading, treating an unrated pair as neutral.
    ///
    /// For callers that want a number and for which "nothing known" and "even" are
    /// genuinely the same answer. Anything that averages readings wants
    /// [`Self::rating`] instead, so that a missing one can be left out rather than
    /// counted as a zero.
    pub fn get(&self, attacker: HeroId, defender: HeroId) -> i8 {
        self.rating(attacker, defender).unwrap_or(0)
    }

    pub fn set(&mut self, attacker: HeroId, defender: HeroId, value: i8) -> Result<(), CoreError> {
        let (a, d) = (attacker.index(), defender.index());
        if a >= self.n {
            return Err(CoreError::UnknownHero(attacker.0, self.n));
        }
        if d >= self.n {
            return Err(CoreError::UnknownHero(defender.0, self.n));
        }
        let idx = a * self.n + d;
        match self.cells.get_mut(idx) {
            Some(cell) => {
                *cell = Some(value);
                Ok(())
            }
            None => Err(CoreError::MatrixShape {
                n: self.n,
                expected: self.n * self.n,
                actual: self.cells.len(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_and_set_round_trip() {
        let mut m = Matrix::unrated(4);
        m.set(HeroId(1), HeroId(2), -78).expect("in range");
        assert_eq!(m.get(HeroId(1), HeroId(2)), -78);
        // The reverse direction is stored independently and stays untouched.
        assert_eq!(m.get(HeroId(2), HeroId(1)), 0);
    }

    /// The distinction the scorer's arithmetic rests on: a pair nobody rated is
    /// not a pair rated as even.
    #[test]
    fn an_unrated_cell_is_distinguishable_from_a_rated_zero() {
        let mut m = Matrix::unrated(4);
        assert_eq!(m.rating(HeroId(1), HeroId(2)), None);

        m.set(HeroId(1), HeroId(2), 0).expect("in range");
        assert_eq!(m.rating(HeroId(1), HeroId(2)), Some(0));
        // Both still read as neutral through the lossy accessor.
        assert_eq!(m.get(HeroId(1), HeroId(2)), 0);
        assert_eq!(m.get(HeroId(3), HeroId(0)), 0);
    }

    #[test]
    fn out_of_range_reads_are_neutral_not_panics() {
        let m = Matrix::unrated(4);
        assert_eq!(m.rating(HeroId(99), HeroId(0)), None);
        assert_eq!(m.rating(HeroId(0), HeroId(99)), None);
        assert_eq!(m.get(HeroId(99), HeroId(0)), 0);
        assert_eq!(m.get(HeroId(0), HeroId(99)), 0);
    }

    #[test]
    fn out_of_range_writes_are_errors() {
        let mut m = Matrix::unrated(4);
        assert_eq!(
            m.set(HeroId(9), HeroId(0), 10),
            Err(CoreError::UnknownHero(9, 4))
        );
    }
}
