//! A small roster for tests, shared by the modules that need one.
//!
//! Three heroes per role rather than the minimum, so a test can fill a role and
//! still have a hero of it left over to be refused — which is the whole subject
//! of the per-role caps. The ids are named, because `HeroId(4)` in an assertion
//! about tank slots says nothing about whether 4 is a tank.
//!
//! Every rating is unrated and every strength zero: nothing here is about
//! scoring, and a fixture that leaned would invite tests that pass for the wrong
//! reason.

use crate::dataset::{Dataset, DatasetParts};
use crate::hero::{Hero, HeroId, Role};
use crate::map::{GameMap, GameMode, MapId};
use crate::matrix::Matrix;
use crate::rank::Rank;

pub const REINHARDT: HeroId = HeroId(0);
pub const SIGMA: HeroId = HeroId(1);
pub const WINSTON: HeroId = HeroId(2);
pub const TRACER: HeroId = HeroId(3);
pub const SOJOURN: HeroId = HeroId(4);
pub const ASHE: HeroId = HeroId(5);
pub const ANA: HeroId = HeroId(6);
pub const LUCIO: HeroId = HeroId(7);
pub const KIRIKO: HeroId = HeroId(8);

/// An id no roster entry answers to. Its own case, because a hero the dataset
/// cannot resolve still costs a body on the team.
pub const NOBODY: HeroId = HeroId(99);

pub const KINGS_ROW: MapId = MapId(0);

/// The heroes above, in id order.
pub fn roster() -> Vec<Hero> {
    [
        ("reinhardt", Role::Tank),
        ("sigma", Role::Tank),
        ("winston", Role::Tank),
        ("tracer", Role::Damage),
        ("sojourn", Role::Damage),
        ("ashe", Role::Damage),
        ("ana", Role::Support),
        ("lucio", Role::Support),
        ("kiriko", Role::Support),
    ]
    .into_iter()
    .map(|(key, role)| Hero {
        key: key.to_owned(),
        name: key.to_owned(),
        role,
        subrole: None,
        aliases: Vec::new(),
    })
    .collect()
}

pub fn dataset() -> Dataset {
    let heroes = roster();
    let n = heroes.len();

    Dataset::new(DatasetParts {
        heroes,
        maps: vec![GameMap {
            key: "kings-row".to_owned(),
            name: "King's Row".to_owned(),
            mode: GameMode::Hybrid,
            aliases: Vec::new(),
        }],
        matchups: Matrix::unrated(n),
        synergy: Matrix::unrated(n),
        map_affinity: vec![0; n],
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "test".to_owned(),
        patch: "test".to_owned(),
    })
    .expect("the fixture is well formed")
}
