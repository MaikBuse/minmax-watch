use thiserror::Error;

/// Every fallible path in `overwatch-core` returns one of these.
///
/// This crate compiles to `wasm32` and runs inside the draft UI, so it never
/// panics: no `unwrap`, no `expect`, no indexing that can go out of bounds.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoreError {
    #[error("hero id {0} is out of range (roster has {1} heroes)")]
    UnknownHero(u16, usize),

    #[error("map id {0} is out of range ({1} maps known)")]
    UnknownMap(u16, usize),

    #[error("no hero matches the key {0:?}")]
    UnknownHeroKey(String),

    #[error("no map matches the key {0:?}")]
    UnknownMapKey(String),

    #[error("matrix needs {expected} cells for a {n}-hero roster but got {actual}")]
    MatrixShape {
        n: usize,
        expected: usize,
        actual: usize,
    },

    #[error("{what} has {actual} entries but the roster has {expected} heroes")]
    RosterLengthMismatch {
        what: &'static str,
        expected: usize,
        actual: usize,
    },

    #[error("roster of {0} heroes exceeds the {1}-hero limit of HeroSet")]
    RosterTooLarge(usize, usize),

    #[error("{0:?} is not a role that can be picked in this app")]
    UnknownRole(String),

    #[error("{0:?} is not a rung of the competitive ladder")]
    UnknownRank(String),

    #[error("{0:?} is not a sub-role this app knows")]
    UnknownSubrole(String),

    #[error("{0:?} is not a competitive game mode")]
    UnknownGameMode(String),
}
