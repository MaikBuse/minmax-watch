//! What kind of fight a team is built for, and which kind beats which.
//!
//! Everything else in the scorer is pairwise: this hero against that hero. But
//! a draft is read as a shape long before it is read as five names, and the
//! shape is its own argument — a dive comp does not beat a poke comp because
//! each of its five duels happens to be favourable, it beats it because five
//! people arriving at once is the answer to five people standing apart.
//!
//! Kept apart from [`crate::hero::Role`], which is what a hero *is*. This is
//! how it wants to play, and the two vary independently: every role has heroes
//! on all three axes.

use serde::{Deserialize, Serialize};

use crate::dataset::Dataset;
use crate::hero::HeroId;

/// The three ways a team can want a fight to go.
///
/// Three rather than five. "Rush" and "bunker" are the two names usually added,
/// and both are positions on these axes rather than axes of their own — rush is
/// brawl that engages first, bunker is poke that cannot be moved. Naming them
/// separately would buy a finer label at the cost of a triangle with no
/// self-consistent answer for what beats what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Archetype {
    /// Close the distance, isolate one target, kill it, leave.
    Dive,
    /// Hold an angle, chip from range, refuse the close fight.
    Poke,
    /// Hold ground at short range and win the sustained fight.
    Brawl,
}

impl Archetype {
    /// Every axis, in the order a hero's three values are stored.
    pub const ALL: [Archetype; 3] = [Archetype::Dive, Archetype::Poke, Archetype::Brawl];

    /// Position in [`Archetype::ALL`], and the index into a hero's stored axes.
    pub const fn index(self) -> usize {
        match self {
            Archetype::Dive => 0,
            Archetype::Poke => 1,
            Archetype::Brawl => 2,
        }
    }

    /// The stable key, and also what the screen shows. The two coincide here,
    /// but they are still named apart so that restyling the chip can never
    /// change what a stored value means.
    pub const fn as_str(self) -> &'static str {
        match self {
            Archetype::Dive => "dive",
            Archetype::Poke => "poke",
            Archetype::Brawl => "brawl",
        }
    }

    pub const fn label(self) -> &'static str {
        self.as_str()
    }
}

/// What a team with no single leading axis is called.
///
/// Deliberately not "hybrid": [`crate::map::GameMode::Hybrid`] already owns that
/// word and it is on screen on the map board at the same time as this one. One
/// word doing two jobs on one screen is how a glance goes wrong.
pub const MIXED: &str = "mixed";

/// What an empty or entirely unrated team reads as.
///
/// Drawn rather than left blank, because nothing appears or disappears
/// mid-draft — the chip has to occupy its space before there is anything to put
/// in it, or the header reflows on the first pick.
pub const UNREAD: &str = "—";

/// How far the leading axis must clear the runner-up before the team is called
/// by it rather than [`MIXED`].
///
/// On the same 0.0..=1.0 scale as the axes themselves. Two forces set it, and
/// both were measured against the committed roster over 4000 random legal 5v5
/// comps rather than guessed at.
///
/// *A margin has to exist.* Most heroes are honestly two-axis — Zarya brawls but
/// travels with a dive, Lúcio heals a group and keeps up with a flanker — so the
/// top two axes of a full team are often within a rounding error of each other.
/// With no margin at all, **the leading axis changes on 20% of fifth picks**: a
/// label that renames the team you already read is worse than no label.
///
/// *But `mixed` says nothing.* The gap distribution is smooth, with no natural
/// cliff to sit in, so every extra point of caution is paid for directly in
/// labels that decline to answer: 0.04 reports `mixed` for 19% of those comps,
/// 0.06 for 28%, 0.08 for 37%, 0.10 for 44%.
///
/// 0.06 is where those meet. It is also lower than it would have to be if the
/// chip had to carry all the doubt on its own — [`Shape::confident`] already
/// draws an early read as tentative, so the margin is not the only thing
/// standing between a two-pick guess and a confident-looking answer.
///
/// Random comps are the pessimistic case: they are far less coherent than what
/// anybody actually drafts, so the real `mixed` rate on a played comp is lower
/// than any figure above. These are still judgement calls informed by a spread,
/// not a fit to a win-rate corpus — there is no match log large enough yet.
pub const MIXED_MARGIN: f32 = 0.06;

/// How many rated picks a team needs before its shape is stated plainly rather
/// than tentatively.
///
/// A shape is a property of a team, and two heroes are not a team: a lone tank
/// pick genuinely does not say what the other four will be. Three is the point
/// where a majority of a five-player team has committed, so it is the point
/// where the read stops being a guess about what is coming.
pub const CONFIDENT_PICKS: usize = 3;

/// Which archetype beats which, indexed `[yours][theirs]` via
/// [`Archetype::index`].
///
/// The canonical triangle, and the reasoning rather than the assertion:
///
/// - **Dive beats poke.** Poke's damage is paid for by standing still on a
///   sightline. Mobility that closes that distance collects on it, and the
///   immobile hero at the end has no answer once it arrives.
/// - **Poke beats brawl.** Brawl needs to reach you before its damage exists.
///   Chip on the walk in means the fight starts with one team already down.
/// - **Brawl beats dive.** A diver's whole design is arriving alone at somebody
///   who is also alone. Five people standing together turn the isolated kill
///   into an isolated death.
///
/// Mirrors are zero, and the table is antisymmetric: an advantage for one side
/// is exactly the disadvantage for the other. That is the property a transposed
/// table would fail, and it is asserted rather than trusted.
///
/// Magnitudes are all 1.0 because there is no evidence for ranking one edge of
/// the triangle above another. [`crate::score::Weights::shape`] is where the
/// strength of the whole term lives, which is the lever to move if this is
/// pushing too hard or too little.
const TRIANGLE: [[f32; 3]; 3] = [
    //        vs dive  vs poke  vs brawl
    /* dive */ [0.0, 1.0, -1.0],
    /* poke */ [-1.0, 0.0, 1.0],
    /* brawl */ [1.0, -1.0, 0.0],
];

/// What one team wants the fight to be.
///
/// Holds the per-axis mean rather than just the winner, because the counter term
/// needs the whole distribution: a team that is 60% dive and 40% brawl is a
/// different thing to counter than a team that is 95% dive, and collapsing both
/// to "dive" before scoring would throw that away.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shape {
    /// Mean of each axis over the rated picks, on 0.0..=1.0.
    weight: [f32; 3],
    /// How many picks carried any archetype reading at all.
    rated: usize,
}

/// The shape of a team, from its picks.
///
/// Picks with no archetype data are left out of the mean rather than folded in
/// as zeroes, for the same reason the counter term leaves unrated matchups out
/// (see [`crate::score`]): averaging in the absence of evidence drags a team
/// toward no-shape-at-all and would report a real dive comp as `mixed` because
/// one of its five heroes has not been curated yet.
pub fn shape_of(ds: &Dataset, picks: &[HeroId]) -> Shape {
    let mut total = [0.0f32; 3];
    let mut rated = 0usize;

    for hero in picks {
        let axes = ds.shape(*hero);
        // All-zero is "nobody has read this kit", not "this hero wants nothing".
        // The two are indistinguishable in the file by design — an absent hero
        // and an absent axis both mean the same silence — so they are treated
        // the same here.
        if axes == [0; 3] {
            continue;
        }
        for (slot, value) in total.iter_mut().zip(axes) {
            *slot += f32::from(value) / 100.0;
        }
        rated += 1;
    }

    if rated == 0 {
        return Shape::default();
    }
    for slot in &mut total {
        *slot /= rated as f32;
    }
    Shape {
        weight: total,
        rated,
    }
}

impl Shape {
    /// Whether anything on this team carried an archetype reading.
    ///
    /// Distinct from [`Self::is_mixed`], and the distinction is the point: an
    /// unrated team is a question nobody has answered, a mixed team is an answer.
    pub fn is_rated(&self) -> bool {
        self.rated > 0
    }

    /// How much this team leans on one axis, on 0.0..=1.0.
    pub fn weight(&self, axis: Archetype) -> f32 {
        self.weight[axis.index()]
    }

    /// The axis this team is built on, or `None` when there is no answer —
    /// either because nothing is rated or because no axis leads by
    /// [`MIXED_MARGIN`].
    pub fn leading(&self) -> Option<Archetype> {
        if !self.is_rated() {
            return None;
        }

        let mut ranked = Archetype::ALL;
        ranked.sort_by(|a, b| {
            self.weight(*b)
                .partial_cmp(&self.weight(*a))
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let (first, second) = (ranked[0], ranked[1]);

        if self.weight(first) - self.weight(second) < MIXED_MARGIN {
            return None;
        }
        Some(first)
    }

    /// Whether this team is rated but has committed to nothing.
    pub fn is_mixed(&self) -> bool {
        self.is_rated() && self.leading().is_none()
    }

    /// Whether enough of the team is rated for the read to be stated plainly.
    pub fn confident(&self) -> bool {
        self.rated >= CONFIDENT_PICKS
    }

    /// The word for this shape: an axis, [`MIXED`], or [`UNREAD`].
    pub fn label(&self) -> &'static str {
        match self.leading() {
            Some(axis) => axis.label(),
            None if self.is_rated() => MIXED,
            None => UNREAD,
        }
    }

    /// How well a hero's own axes fare against this shape, on roughly
    /// -1.0..=1.0.
    ///
    /// Every one of the hero's axes is weighed against every one of the team's,
    /// rather than collapsing both to their leader first. A hero that is 70%
    /// dive and 60% poke into a team that is 80% poke and 30% brawl is making
    /// two arguments at once, and the sum of them is the answer.
    ///
    /// Zero when either side is unrated, which is what keeps the term silent
    /// rather than confidently neutral — see [`crate::score`] on why a
    /// zero-valued term is never turned into a reason line.
    pub fn against(&self, hero: [i8; 3]) -> f32 {
        if !self.is_rated() {
            return 0.0;
        }

        let mut total = 0.0;
        for mine in Archetype::ALL {
            let theirs_beaten = f32::from(hero[mine.index()]) / 100.0;
            if theirs_beaten == 0.0 {
                continue;
            }
            for theirs in Archetype::ALL {
                total +=
                    theirs_beaten * self.weight(theirs) * TRIANGLE[mine.index()][theirs.index()];
            }
        }
        total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::DatasetParts;
    use crate::hero::{Hero, Role};
    use crate::matrix::Matrix;

    const WINSTON: HeroId = HeroId(0);
    const REINHARDT: HeroId = HeroId(1);
    const WIDOWMAKER: HeroId = HeroId(2);
    /// Present on the roster, absent from `archetype.toml`.
    const UNCURATED: HeroId = HeroId(3);
    /// Leads dive by less than [`MIXED_MARGIN`].
    const NARROW: HeroId = HeroId(4);
    /// Leads dive by more than it.
    const CLEAR: HeroId = HeroId(5);

    /// A leaning roster, unlike [`crate::fixture`], which is deliberately flat.
    /// Shapes are the entire subject here, so the fixture has to have some.
    fn dataset() -> Dataset {
        let heroes: Vec<Hero> = [
            ("winston", Role::Tank),
            ("reinhardt", Role::Tank),
            ("widowmaker", Role::Damage),
            ("uncurated", Role::Damage),
            ("narrow", Role::Damage),
            ("clear", Role::Damage),
        ]
        .into_iter()
        .map(|(key, role)| Hero {
            key: key.to_owned(),
            name: key.to_owned(),
            role,
            subrole: None,
            aliases: Vec::new(),
        })
        .collect();
        let n = heroes.len();

        Dataset::new(DatasetParts {
            heroes,
            maps: Vec::new(),
            matchups: Matrix::unrated(n),
            synergy: Matrix::unrated(n),
            map_affinity: Vec::new(),
            base_strength: vec![0; n],
            win_rate: vec![None; n],
            side_lean: vec![0; n],
            // One pure hero per axis, at equal strength, so a team of all three
            // is an exact three-way tie and `mixed` is not resting on the
            // margin happening to swallow a lopsided fixture.
            shape: vec![
                [95, 0, 0], // winston: dive
                [0, 0, 95], // reinhardt: brawl
                [0, 95, 0], // widowmaker: poke
                [0, 0, 0],  // uncurated: nothing
                // Two heroes that lean the same way by different amounts, for
                // pinning MIXED_MARGIN from both sides: a 0.05 gap is inside it
                // and a 0.10 gap is outside.
                [50, 45, 0], // narrow
                [50, 40, 0], // clear
            ],
            reasons: vec![String::new(); n * n],
            generated: "test".to_owned(),
            patch: "test".to_owned(),
        })
        .expect("the fixture is well formed")
    }

    #[test]
    fn a_team_of_one_axis_reads_as_that_axis() {
        let ds = dataset();

        assert_eq!(shape_of(&ds, &[WINSTON]).leading(), Some(Archetype::Dive));
        assert_eq!(
            shape_of(&ds, &[REINHARDT]).leading(),
            Some(Archetype::Brawl)
        );
        assert_eq!(
            shape_of(&ds, &[WIDOWMAKER]).leading(),
            Some(Archetype::Poke)
        );
    }

    #[test]
    fn an_evenly_split_team_reads_as_mixed() {
        let ds = dataset();
        let shape = shape_of(&ds, &[WINSTON, REINHARDT, WIDOWMAKER]);

        assert!(shape.is_mixed());
        assert_eq!(shape.leading(), None);
        assert_eq!(shape.label(), MIXED);
    }

    /// The margin itself, pinned from both sides. A team can lead on an axis and
    /// still be called `mixed` — that gap is the whole reason the constant
    /// exists, and a change to it should have to move this test rather than
    /// slip past it.
    #[test]
    fn a_lead_narrower_than_the_margin_is_not_a_shape() {
        let ds = dataset();

        assert_eq!(
            shape_of(&ds, &[NARROW]).leading(),
            None,
            "0.05 ahead is inside the margin"
        );
        assert!(shape_of(&ds, &[NARROW]).is_mixed());

        assert_eq!(
            shape_of(&ds, &[CLEAR]).leading(),
            Some(Archetype::Dive),
            "0.10 ahead is outside it"
        );
    }

    /// The distinction the label rests on: `mixed` is a read, and an unrated
    /// team is the absence of one. Both have no leading axis, so `leading()`
    /// alone cannot tell them apart and the two must not render the same.
    #[test]
    fn an_unread_team_is_not_the_same_as_a_mixed_one() {
        let ds = dataset();

        let empty = shape_of(&ds, &[]);
        assert!(!empty.is_rated());
        assert!(!empty.is_mixed());
        assert_eq!(empty.label(), UNREAD);

        let uncurated = shape_of(&ds, &[UNCURATED, UNCURATED]);
        assert!(!uncurated.is_rated(), "no pick carried a reading");
        assert_eq!(uncurated.label(), UNREAD);

        let mixed = shape_of(&ds, &[WINSTON, REINHARDT, WIDOWMAKER]);
        assert!(mixed.is_rated());
        assert_ne!(mixed.label(), uncurated.label());
    }

    /// An uncurated hero must not drag its team toward no-shape-at-all: a real
    /// dive comp with one unread pick in it is still a dive comp.
    #[test]
    fn an_uncurated_pick_is_left_out_of_the_mean_rather_than_counted_as_zero() {
        let ds = dataset();

        let pure = shape_of(&ds, &[WINSTON, WINSTON]);
        let diluted = shape_of(&ds, &[WINSTON, WINSTON, UNCURATED]);

        assert_eq!(
            pure.weight(Archetype::Dive),
            diluted.weight(Archetype::Dive)
        );
        assert_eq!(diluted.leading(), Some(Archetype::Dive));
    }

    #[test]
    fn confidence_arrives_with_the_third_rated_pick() {
        let ds = dataset();

        assert!(!shape_of(&ds, &[WINSTON]).confident());
        assert!(!shape_of(&ds, &[WINSTON, WINSTON]).confident());
        assert!(shape_of(&ds, &[WINSTON, WINSTON, WINSTON]).confident());

        assert!(
            !shape_of(&ds, &[WINSTON, WINSTON, UNCURATED]).confident(),
            "an unread pick is not a pick this read rests on"
        );
    }

    /// The property a transposed table would fail. Every edge of the triangle
    /// has to cost the other side exactly what it gains this one, or the scorer
    /// would find free value in a mirror match.
    #[test]
    fn the_triangle_is_antisymmetric() {
        for a in Archetype::ALL {
            assert_eq!(
                TRIANGLE[a.index()][a.index()],
                0.0,
                "{a:?} mirrors itself for nothing"
            );
            for b in Archetype::ALL {
                assert_eq!(
                    TRIANGLE[a.index()][b.index()],
                    -TRIANGLE[b.index()][a.index()],
                    "{a:?} vs {b:?} is not the opposite of {b:?} vs {a:?}"
                );
            }
        }
    }

    /// The claim the whole term exists to make, asserted on the shapes rather
    /// than on the table it is built from.
    #[test]
    fn dive_beats_poke_beats_brawl_beats_dive() {
        let ds = dataset();
        let (dive, poke, brawl) = (ds.shape(WINSTON), ds.shape(WIDOWMAKER), ds.shape(REINHARDT));

        let poke_comp = shape_of(&ds, &[WIDOWMAKER]);
        let brawl_comp = shape_of(&ds, &[REINHARDT]);
        let dive_comp = shape_of(&ds, &[WINSTON]);

        assert!(poke_comp.against(dive) > 0.0, "dive answers poke");
        assert!(brawl_comp.against(poke) > 0.0, "poke answers brawl");
        assert!(dive_comp.against(brawl) > 0.0, "brawl answers dive");

        assert!(poke_comp.against(brawl) < 0.0, "brawl walks into poke");
        assert!(brawl_comp.against(dive) < 0.0, "dive walks into brawl");
        assert!(dive_comp.against(poke) < 0.0, "poke walks into dive");
    }

    /// Silence in either direction has to produce silence, not a confident
    /// neutral — the scorer turns a non-zero term into a sentence on screen.
    #[test]
    fn an_unrated_side_contributes_nothing() {
        let ds = dataset();

        assert_eq!(shape_of(&ds, &[]).against(ds.shape(WINSTON)), 0.0);
        assert_eq!(
            shape_of(&ds, &[WIDOWMAKER]).against(ds.shape(UNCURATED)),
            0.0
        );
    }

    /// A mirror is a rated dead even. Worth pinning separately from the
    /// antisymmetry check: that one is about the table, this is about a real
    /// comp facing itself, where the off-axis weights also have to cancel.
    #[test]
    fn a_comp_facing_itself_scores_nothing() {
        let ds = dataset();

        for hero in [WINSTON, REINHARDT, WIDOWMAKER] {
            let comp = shape_of(&ds, &[hero]);
            let term = comp.against(ds.shape(hero));
            assert!(term.abs() < 1e-6, "a mirror scored {term}");
        }
    }
}
