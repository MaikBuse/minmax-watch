use serde::{Deserialize, Serialize};

use crate::archetype::{shape_of, Archetype, Shape};
use crate::dataset::Dataset;
use crate::draft::Draft;
use crate::error::CoreError;
use crate::hero::{HeroId, Role};
use crate::map::{MapId, Side};
use crate::rank::Rank;

/// How much one enemy pick counts towards the counter term, by its role and by
/// the role you are playing. Indexed `[your role][their role]` via
/// [`Role::index`].
///
/// Two things set these numbers.
///
/// *Measured discrimination.* Across the committed matchup data, one enemy pick
/// spreads your own candidates by an average `max - min` of: 1.34 (your tank vs
/// their tank), 1.22 (tank vs support), 1.19 (tank vs damage), 1.26 (support vs
/// tank), 1.20 (support vs damage), and 0.83 (support vs support). So the data
/// says role barely matters — a 13% span across the first five — *except* that
/// the enemy supports hardly change which support you should play, hence the 0.6
/// on that one cell.
///
/// *Slot multiplicity, which the data cannot see.* A 5v5 team fields one tank
/// and two of everything else, so under a plain average the single most decisive
/// duel in the game is outvoted 2-to-1 by the damage pair purely because there is
/// only one of it. The 2.2 on the tank-vs-tank cell buys the enemy tank about a
/// third of the counter signal instead of a fifth.
///
/// *And these numbers assume that team.* [`crate::format::Format`] can now say
/// 6v6, where there are two enemy tanks and each still carries the 2.2 — taking
/// the tank share of the counter term from roughly a third to roughly a half.
/// That may well be right, since two tank duels really are two duels, but it is
/// untested and it was not what the figure was chosen for. Left alone
/// deliberately: retuning it means inventing a second table with no 6v6 corpus
/// behind it. [`Weights::enemy_roles`] is already per-user and serde-defaulted,
/// so a format-aware table stays an additive change whenever there is evidence
/// for one.
///
/// These are judgement calls informed by the matchup spread, not fits to a
/// win-rate corpus — there is no match log large enough to fit against yet.
/// All-ones reproduces a uniform average, which is what this replaced.
///
/// Worth knowing before touching them: this is the largest single lever in the
/// scorer. Against random full enemy teams, switching to [`Self::uniform`]
/// changes the top recommendation for 31% of tank drafts and 25% of support
/// drafts.
///
/// The damage row is the one the measured spread above says nothing about — the
/// figures were gathered while damage had no pick mode. It is reasoned from the
/// same two ingredients as the others rather than left at all-ones; see the row
/// itself for the argument.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnemyRoleWeights([[f32; 3]; 3]);

impl EnemyRoleWeights {
    pub const fn new(rows: [[f32; 3]; 3]) -> Self {
        Self(rows)
    }

    /// A weight of 1.0 everywhere: every enemy counts the same, which is how
    /// the counter term behaved before role weighting existed.
    pub const fn uniform() -> Self {
        Self([[1.0; 3]; 3])
    }

    pub fn get(&self, yours: Role, theirs: Role) -> f32 {
        self.0[yours.index()][theirs.index()]
    }

    /// The same table, rescaled so that the three columns are comparable — for
    /// the one caller that compares them.
    ///
    /// [`Self::get`] is right wherever the thing being ranked shares a column:
    /// `score_hero` normalises it into shares over one enemy board, and
    /// [`threats`] orders the enemies themselves, which is the question these
    /// numbers answer. `ban_recommendations` is the exception. Its candidates
    /// span all three roles and its denominator sums only certainty, so the raw
    /// cell survives into the score as a bare multiplier — and the columns are
    /// not the same size. Weighted by the roles a 5v5 actually fields, they come
    /// out at tank 1.56, damage 1.32 and support 0.80, so a support candidate was
    /// read through a column worth **1.95x less than a tank's** and could not
    /// reach the drawn top eight whatever the matchups said. Measured over 300
    /// random legal comps: 0.67 supports per top eight against a roster-neutral
    /// 2.0, and no support at all in half of them.
    ///
    /// That was never what the magnitudes were for. The doc comment above sizes
    /// the 2.2 as buying the enemy tank "about a third of the counter signal
    /// instead of a fifth" — a share of a mean, not a claim that a tank ban is
    /// worth 2.2 of a support ban.
    ///
    /// Dividing by the column's own mean fixes exactly that and nothing else.
    /// Every column then averages 1.0, so the cross-role scale is gone, while the
    /// ratios *within* a column survive untouched — a tank teammate still counts
    /// 1.200 against a support candidate where a support teammate counts 0.720,
    /// which is the slot-multiplicity correction doing its real work.
    ///
    /// What that buys, on the committed data over the same 300 comps: the drawn
    /// top eight goes from 3.25 tanks / 4.08 damage / 0.67 supports to
    /// 2.44 / 3.92 / 1.64, against a roster-neutral 2.33 / 3.67 / 2.00, and the
    /// share of drafts showing a support player no supports at all falls from
    /// **49% to 12%**. It changes the top ban for **30%** of comps and the top-eight
    /// set for 79% — the same order as this table's own headline figure, which is
    /// the right size for correcting the largest lever in the scorer.
    ///
    /// **A flat mean of the three cells, deliberately, and not one weighted by
    /// the team in front of you.** The alternative fix is to put the weight in
    /// the denominator too, which normalises against the actual roster — and
    /// that costs the property
    /// `a_second_tank_on_the_team_raises_what_an_enemy_tank_is_worth` pins,
    /// because a second tank then moves the reference as well as the numerator.
    /// A fixed reference scales each column by a constant, so every comparison
    /// of one candidate across team compositions is arithmetically unchanged:
    /// that test rises on `b > 0.727a` before and after, where the denominator
    /// fix would make it `b > a`. Only cross-role ordering moves, which is the
    /// thing that was wrong.
    ///
    /// Derived from the table rather than tabulated, so editing `enemy_roles`
    /// keeps the ban list consistent with the pick list.
    pub fn ban_weight(&self, theirs: Role, candidate: Role) -> f32 {
        let mean = self.column_mean(candidate);
        if mean.abs() < f32::EPSILON {
            return 1.0;
        }
        self.get(theirs, candidate) / mean
    }

    /// The mean of one column, over the three roles a teammate can be playing.
    fn column_mean(&self, candidate: Role) -> f32 {
        let column = candidate.index();
        let total: f32 = self.0.iter().map(|row| row[column]).sum();
        total / self.0.len() as f32
    }
}

impl Default for EnemyRoleWeights {
    fn default() -> Self {
        // Columns are the enemy's role in `Role::ALL` order: tank, damage, support.
        Self([
            // You play tank: the tank duel decides your game, and it needs the
            // 2.2 just to out-weigh the enemy damage pair.
            [2.2, 1.0, 1.0],
            // You play damage: the duel you are in every fight is the enemy
            // damage pair, so the 1.5 buys them about half the counter signal
            // once the team is full. The tank still decides whether your kit
            // does anything at all, and their supports shape a dive pick's life
            // more than they shape a support's — hence 0.9 rather than the 0.6
            // on the row below.
            [1.2, 1.5, 0.9],
            // You play support: what kills you is the divers and the tank that
            // enables them. Which healers they run barely moves your answer.
            [1.6, 1.3, 0.6],
        ])
    }
}

/// Relative importance of each scoring term. Exposed as sliders in the UI and
/// persisted per user, because "how much do I care about counters vs. comfort"
/// is a genuine preference rather than something to hard-code.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights {
    pub base: f32,
    pub counter: f32,
    pub synergy: f32,
    pub map: f32,
    pub personal: f32,
    /// How much the attack/defend lean counts, on the payload maps that have
    /// sides. Low, because the leans behind it are hand-curated judgement rather
    /// than anything measured.
    #[serde(default = "default_side")]
    pub side: f32,
    /// How much the enemy team's *shape* counts, on top of the individual
    /// matchups against it. Same band as the map and side terms, and for the
    /// same reason: the axes behind it are hand-curated judgement, so it is
    /// there to argue at the margin rather than to overrule the measured
    /// matchup data above it.
    ///
    /// What 0.25 actually buys, measured against random full enemy teams on the
    /// committed data: the term changes the top tank recommendation for **11%**
    /// of drafts, and moves any single score by at most 0.18. Worth comparing
    /// against [`EnemyRoleWeights`], the largest lever in the scorer, which
    /// moves the top pick for 31% — this is deliberately the smaller of the two.
    ///
    /// It bites hardest early, which is when it is most use: a full enemy team
    /// usually has all three axes populated and the term largely cancels, while
    /// the two or three picks you are actually guessing from often do not.
    #[serde(default = "default_shape")]
    pub shape: f32,
    /// How much the rung of the ladder you selected counts, on top of the
    /// all-ranks patch strength above.
    ///
    /// This weights the *shift* away from the ladder average — [`Dataset::rank_shift`]
    /// — rather than the strength itself, so the two decompose instead of
    /// double-counting: no single number ever mixes "strong on the ladder" with
    /// "strong where you play", which is the same rule
    /// [`ban_by_strength`] follows for a different pair of arguments.
    ///
    /// 0.15 because that is [`Weights::base`]. At equal weights the two terms sum
    /// to exactly the rank-sliced strength, so the default behaviour is "read the
    /// column for the rung you picked" and the knob exists without a number
    /// nobody measured behind it. Raising it is a claim that where you play
    /// matters more than the ladder average does, and wants a corpus first.
    ///
    /// What 0.15 actually buys, measured against random enemy teams on the
    /// committed data: selecting a rung changes the top recommendation for
    /// **21% of drafts at Bronze, 19% at Diamond, 24% at Master and 28% at
    /// Grandmaster+**, and moves any single score by at most 0.13. Gold is 8%,
    /// which is the sanity check rather than a disappointment — Gold sits near
    /// the ladder average, so there is little there to move.
    ///
    /// Worth comparing against the rest of the scorer: [`EnemyRoleWeights`], the
    /// largest lever here, moves the top pick for 31%; [`Weights::shape`] 11%;
    /// [`Weights::map`] 8.3%. So this lands between the two, which is where a
    /// term backed by measured win rates but reaching only one signal belongs.
    ///
    /// Per hero it is the tails that carry it: Reinhardt moves 0.10 of score
    /// between Bronze and Grandmaster, Sierra 0.17 and D.Mon 0.20, against a
    /// typical counter contribution around 0.25 and a
    /// [`Weights::swap_threshold`] of 0.15.
    ///
    /// The only other rank-aware thing in the scorer is [`Weights::prevalence`],
    /// which reads a different rank-sliced file and reaches only the ban list. No
    /// source publishes per-rung matchups or duos, so nothing about a *pair* is or
    /// can be rank-aware. See [`crate::Rank`].
    #[serde(default = "default_rank")]
    pub rank: f32,
    /// How much a hero's pick rate discounts the case for banning it.
    ///
    /// **A multiplier, not a term**, and the only one in here — the last
    /// multiplicative field this struct had was `focus_multiplier`, and it was
    /// removed. The reason it is not additive is the whole point: prevalence is
    /// not an argument *for* a ban, it is a discount on one. Added, a hero
    /// everybody picks and nobody loses to would climb the list on popularity
    /// alone, which is the failure this exists to fix rather than to cause.
    ///
    /// At 0.40 the factor lives in `[0.60, 1.40]` — a 2.3:1 spread against the
    /// 20:1 the raw pick rates would give, and strictly positive, so it can never
    /// flip a sign or promote a hero the sources say your team beats. It sits just
    /// under [`EnemyRoleWeights`], the largest lever here, which is the same
    /// argument that holds `map` and `synergy` down: this is a prior on who turns
    /// up, not a reading of the matchup the ban is about.
    ///
    /// **It reaches the ban list and nothing else.** [`threats`] reads heroes
    /// already on the board, where the probability of appearing is 1 and
    /// multiplying by a prior would be arithmetic about an event that has already
    /// happened; [`recommend`] is excused by the same argument, because the
    /// candidate there is you.
    #[serde(default = "default_prevalence")]
    pub prevalence: f32,
    /// Minimum advantage before swap mode suggests leaving a working hero.
    /// Without this the list churns every time the enemy team twitches.
    pub swap_threshold: f32,
    /// Defaulted rather than required, so a profile stored before this field
    /// existed still loads with its pool and weights intact.
    #[serde(default)]
    pub enemy_roles: EnemyRoleWeights,
}

// `#[serde(default)]` on a bare `f32` resolves to 0.0, not to the value in
// `Weights::default()`, so a profile written before this field existed would
// silently load with the side term switched off. A named default is the only
// way to keep an old profile behaving like a new one.
fn default_side() -> f32 {
    0.20
}

/// Same trap as [`default_side`], and it bites harder here: this term is new, so
/// a bare `#[serde(default)]` would resolve to 0.0 and ship the feature switched
/// off for everybody who already has a stored profile — which is everybody who
/// has used the app.
fn default_shape() -> f32 {
    0.25
}

/// Same trap again, and the same reason it matters: this term is new, so a bare
/// `#[serde(default)]` would resolve to 0.0 and ship the rank picker connected to
/// nothing for everybody who already has a stored profile.
///
/// The value is [`Weights::base`] rather than a number of its own, deliberately —
/// see [`Weights::rank`].
fn default_rank() -> f32 {
    0.15
}

/// The same trap once more, and it needs saying explicitly because this one is a
/// multiplier and the arithmetic looks like it should be safe.
///
/// It is not: the stored number is the *weight*, so a bare `#[serde(default)]`
/// resolving to 0.0 makes `prevalence_factor` return exactly 1.0 for every hero.
/// That is the term shipped switched off for everybody with a stored profile —
/// the identical failure the three defaults above guard against, arrived at
/// through a factor of one instead of a term of zero.
fn default_prevalence() -> f32 {
    0.40
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            base: 0.15,
            rank: default_rank(),
            prevalence: default_prevalence(),
            counter: 1.0,
            // Lowered from 0.30 when the term stopped being hypothetical:
            // `synergy.toml` shipped empty for a long time, so 0.30 was a number
            // multiplying nothing and was never measured against real data.
            //
            // What 0.20 buys, against random enemy teams on the committed data:
            // the term changes the top recommendation for 11% of drafts with one
            // ally entered, rising to 18% with four, and moves any single score
            // by at most 0.20. That puts it alongside the shape term (11%, 0.18)
            // and well under `EnemyRoleWeights` (31%), which is where a
            // supporting argument belongs.
            //
            // It is held below `map` for the reason `map` is held low: the
            // source publishes only a short top-N of partners per hero, so like
            // map affinity the signal is positive-only. A pair can be a reason
            // to pick a hero and never a reason not to, and a term that can only
            // ever add deserves less room than one that can argue both ways.
            synergy: 0.20,
            // Higher than it looks, because the signal it multiplies is sparse
            // enough to bound itself. Only 8.1% of the hero/map grid is
            // populated — the source publishes each hero's three best maps and
            // nothing else — so the term is silent for most candidates, and the
            // best cell it can read is 60, capping any single score at 0.15.
            //
            // Measured against random enemy teams on a random map: the term
            // changes the top recommendation for 8.3% of drafts, which is less
            // than the shape term's 11% despite the larger weight. Lowering it
            // would buy nothing except a quieter term on the maps where the
            // sources actually have something to say.
            //
            // The asymmetry to keep in mind is that a zero here means "nothing
            // known", not "average", so this term can argue for a hero and never
            // against one.
            map: 0.25,
            // Comfort is weighted heavily on purpose: a hero you play well but
            // is countered usually beats the "correct" pick you cannot play.
            personal: 0.60,
            side: default_side(),
            shape: default_shape(),
            swap_threshold: 0.15,
            enemy_roles: EnemyRoleWeights::default(),
        }
    }
}

/// Per-user state layered on top of the shared [`Dataset`].
#[derive(Debug, Clone)]
pub struct UserContext {
    /// Which role you are picking for. This is what the mode switch sets.
    ///
    /// The only thing that narrows the candidate list. Your hero pool used to
    /// narrow it further, but a pool that hides heroes is a pool that hides the
    /// answer on the draft where you needed it — it marks what is yours now and
    /// leaves the ranking to say the rest.
    pub role: Role,
    /// Which rung of the ladder the patch-strength numbers are read on.
    ///
    /// [`Rank::All`] rather than an `Option`, because the whole-ladder aggregate
    /// is a real bucket both sources publish and the one this app has always
    /// scored on — "unset" and "all ranks" are the same reading of the same
    /// number. That is not the situation [`crate::Matrix::rating`] is in, where
    /// "nothing known" and "dead even" are genuinely different claims.
    ///
    /// It reaches exactly two things, and neither of them is a matchup: the
    /// [`Weights::rank`] term on the pick list, and [`Weights::prevalence`] on the
    /// ban list. Nothing else in here is rank-aware and nothing else can be — no
    /// source publishes a *pair* per rung. See [`Rank`].
    pub rank: Rank,
    /// Personal nudges on the -100..=100 scale, indexed by hero. Never written
    /// by the ingest.
    pub overrides: Vec<i8>,
    pub weights: Weights,
}

impl UserContext {
    pub fn new(role: Role, hero_count: usize) -> Self {
        Self {
            role,
            rank: Rank::All,
            overrides: vec![0; hero_count],
            weights: Weights::default(),
        }
    }

    fn override_for(&self, hero: HeroId) -> f32 {
        f32::from(self.overrides.get(hero.index()).copied().unwrap_or(0)) / 100.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonKind {
    /// This candidate beats an enemy pick.
    BeatsEnemy(HeroId),
    /// An enemy pick beats this candidate.
    LosesToEnemy(HeroId),
    PairsWithAlly(HeroId),
    MapFit(MapId),
    /// This candidate suits the half of the map you are playing.
    SideFit(Side),
    /// This candidate's own playstyle is the answer to the shape the enemy team
    /// is building. The archetype carried is *theirs* — what is being beaten —
    /// because that is the half of the sentence the screen does not already say.
    CountersShape(Archetype),
    /// The enemy's shape is the answer to this candidate.
    LosesToShape(Archetype),
    BaseStrength,
    /// This candidate is stronger, or weaker, at the rung of the ladder you
    /// selected than it is across the ladder as a whole.
    ///
    /// Its own kind rather than a payload on [`Self::BaseStrength`], because the
    /// two are separate terms in the score and the panel should be able to say so
    /// separately. Never produced at [`Rank::All`]: the shift there is zero by
    /// construction, and a zero term is never explained.
    ///
    /// The only kind that names a rung. The counter, synergy, map, side and shape
    /// terms read the same numbers at every rung, so a second line implying
    /// otherwise would be a claim no source behind this supports.
    RankFit(Rank),
    Comfort,
}

/// One line of the "why" panel, already weighted so the UI can sort by impact.
#[derive(Debug, Clone, PartialEq)]
pub struct Reason {
    pub kind: ReasonKind,
    /// Signed contribution to the final score.
    pub contribution: f32,
    /// Scraped rationale where one exists, otherwise empty and the UI renders
    /// its own phrasing from `kind`.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    pub hero: HeroId,
    pub score: f32,
    /// Score minus your locked hero's score. `None` when nothing is locked.
    pub delta_vs_locked: Option<f32>,
    /// True only in swap mode, and only when the gain clears `swap_threshold`.
    pub worth_swapping: bool,
    pub is_locked: bool,
    pub reasons: Vec<Reason>,
}

/// How much of the enemy team is pressuring one candidate, used for the threat
/// board that tells you *why* you are losing rather than just what to pick.
#[derive(Debug, Clone, PartialEq)]
pub struct Threat {
    pub enemy: HeroId,
    /// Positive means this enemy is winning the matchup against you.
    pub severity: f32,
    pub text: String,
    /// The two trusted sources contradicted each other about this pair.
    ///
    /// Resolved here rather than left for the panel to look up, for the same
    /// reason `text` is: the screen renders what the scorer read, and a second
    /// resolution of the same question is a build away from disagreeing with the
    /// first.
    pub disputed: bool,
}

/// How sure the ban list is about what one member of your team will be on.
///
/// The ban phase runs before anyone picks, so "who am I defending" is usually
/// unanswerable in the strict sense. It is not, however, unanswerable in
/// practice: people mark pools, and a pool is a claim about what they might end
/// up on. These are the three grades of that claim, worst to best.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Knowledge {
    /// They have locked in. One hero, and it is a fact rather than a claim.
    Locked(HeroId),
    /// They have marked a pool for the role they declared. Any of it could be
    /// the hero they end up on, so a ban has to defend all of it at once.
    Pool,
    /// They have said only what they queued as. Their heroes are the whole
    /// role, which averages to nearly nothing — this is what
    /// [`UNKNOWN_CERTAINTY`] discounts.
    Unknown,
}

/// One person the ban list is defending, resolved to the heroes they might play.
///
/// Built by [`crate::session::SessionState::defended_team`], which is where the
/// roster arithmetic lives. The scorer takes it as given so that it stays a pure
/// function of an explicit input — the same reason `draft_for` is in the session
/// module rather than the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defended {
    /// The roster label, so a row can say whose hero takes the worst of it.
    pub who: String,
    /// Whether this is the person reading the screen. What lets the panel say
    /// "hardest on Reinhardt" for you and "hardest on Ana · Mika" for a
    /// teammate, rather than crediting you to yourself.
    pub is_me: bool,
    /// A hero typed onto the ally board rather than a seat, in which case `who`
    /// is the hero's own name because there is no person behind it.
    ///
    /// The other reason not to print a credit. Without it the row reads
    /// "hardest on Genji · Genji", which is the same word twice pretending to
    /// be an attribution.
    pub is_typed: bool,
    /// The role they hold a slot in: their locked hero's, or the one they
    /// declared. It picks the [`EnemyRoleWeights`] row this member reads a
    /// candidate through.
    pub role: Role,
    pub knowledge: Knowledge,
    /// Every hero they might end up on: one when locked, their pool when they
    /// have one, the whole role when they have said only what they queued as.
    pub heroes: Vec<HeroId>,
}

impl Defended {
    /// How much this member's opinion counts. See [`UNKNOWN_CERTAINTY`].
    fn certainty(&self) -> f32 {
        match self.knowledge {
            Knowledge::Locked(_) | Knowledge::Pool => 1.0,
            Knowledge::Unknown => UNKNOWN_CERTAINTY,
        }
    }

    /// Whether this member has actually said anything. An `Unknown` member is a
    /// role and nothing else, which is not a claim about any hero.
    fn is_known(&self) -> bool {
        !matches!(self.knowledge, Knowledge::Unknown)
    }
}

/// Your whole team, and what is known about each of them.
///
/// One entry per person, in roster order, including you. Drafting alone is the
/// one-member case rather than a separate path, which is what keeps the solo
/// answer identical to what a one-person session produces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefendedTeam {
    pub members: Vec<Defended>,
}

impl DefendedTeam {
    /// Whether anybody on the team plays `hero`.
    ///
    /// Deliberately blind to `Unknown` members, whose heroes are an entire role:
    /// counting those would answer "yes" for most of the roster and empty the
    /// ban list of everything it exists to rank.
    pub fn plays(&self, hero: HeroId) -> bool {
        self.members
            .iter()
            .filter(|member| member.is_known())
            .any(|member| member.heroes.contains(&hero))
    }

    /// Whether anybody has said anything at all. False is the rung where the
    /// list falls back to the patch.
    pub fn anyone_known(&self) -> bool {
        self.members.iter().any(Defended::is_known)
    }
}

/// Whose matchups the ban list is reading, and therefore what a ban defends.
///
/// The panel names whichever one is live, because the number on every row means
/// something different in each case and a column that does not say which is a
/// column that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BanSubject {
    /// Nobody has marked a pool or locked a hero, so there is nothing about
    /// *this* team to answer with. Ranked by patch strength instead.
    ///
    /// The only rung where the score is about the patch rather than about your
    /// team, and it is deliberately the only one: "strong right now" and "bad
    /// for us" are two different arguments for a ban, and mixing them in one
    /// number gives a figure that reads as neither. Here there is no second
    /// argument to mix it with.
    Patch,
    /// Exactly one person has said anything. Named, because "vs your pool" is
    /// only right when that person is you.
    One {
        who: String,
        is_me: bool,
        /// Whether that one thing was a lock rather than a pool.
        locked: bool,
        /// How many heroes they might end up on.
        heroes: usize,
    },
    /// Two or more people have said something, so the list is a team answer.
    Team {
        known: usize,
        /// How many of the known are locked in, which is how far through the
        /// draft the answer has got.
        locked: usize,
    },
}

/// One hero worth denying the enemy.
#[derive(Debug, Clone, PartialEq)]
pub struct BanCandidate {
    pub hero: HeroId,
    /// Role-weighted danger to the team. Higher is more worth banning, and this
    /// is what the list is sorted by. On the [`BanSubject::Patch`] rung it is
    /// patch strength instead, on the same scale.
    pub score: f32,
    /// The same certainty-weighted mean without the role weight, on the
    /// -1.0..=1.0 scale everything else uses. Positive means it beats the team.
    ///
    /// Not what the panel displays. [`threats`] can show its raw severity
    /// because it weights across one enemy team, where the two orderings barely
    /// differ; a ban list ranks the whole roster through a 2.2× role weight, so
    /// a column sorted by `score` while showing `severity` would visibly
    /// disagree with itself.
    pub severity: f32,
    /// How often this hero is picked relative to its role's fair share, at the
    /// rung the list was read on. Positive means more often than its share.
    ///
    /// Already spent on `score`. It is carried anyway for the reason `severity`
    /// is: the panel says out loud that a hero is rare, and re-deriving that from
    /// the dataset on the other side of the wall is a build away from disagreeing
    /// with the number that actually moved the row.
    pub prevalence: i8,
    /// Whichever of the team's heroes it hurts most.
    ///
    /// `None` only on the [`BanSubject::Patch`] rung, where the score comes from
    /// no pair at all and naming one would invent a claim.
    pub worst: Option<HeroId>,
    /// Who plays `worst`, when that is somebody other than you. `None` when the
    /// hero is one of yours, so the panel does not credit you to yourself.
    pub worst_owner: Option<String>,
    /// The scraped sentence for the `worst` pair, where one exists.
    pub text: String,
}

/// The ban list and the question it answers.
#[derive(Debug, Clone, PartialEq)]
pub struct BanBoard {
    pub subject: BanSubject,
    /// Best ban first. Only heroes that actually beat the team are here, and
    /// never one the team plays.
    pub candidates: Vec<BanCandidate>,
}

const MAX_REASONS: usize = 3;

/// Averaged matchup value for `hero` against `enemy`, on -1.0..=1.0, or `None`
/// when no source has rated the pair in either direction.
///
/// The two directions are stored separately and can disagree, because the
/// secondary source covers only part of each row and so moves one direction
/// without the other. Taking the difference folds them back into a single
/// symmetric answer, and cancels any constant offset either direction carries.
///
/// It does not, however, average two *independent* readings: the primary source
/// derives both directions from one rating, so for the pairs only it covers the
/// two halves are the same number stated twice.
///
/// Only the readings that exist are averaged. Folding a missing direction in as a
/// zero would report a known matchup at half its magnitude, which is what this
/// used to do for the pairs the secondary source covers alone.
///
/// A mirror is a rated dead even rather than an unknown. No source rates a hero
/// against itself, but the answer is not in doubt, and leaving it out would let
/// mirroring the enemy tank inflate the weight of everyone else on their team.
fn matchup_term(ds: &Dataset, hero: HeroId, enemy: HeroId) -> Option<f32> {
    if hero == enemy {
        return Some(0.0);
    }
    let forward = ds.matchups().rating(hero, enemy).map(f32::from);
    let reverse = ds.matchups().rating(enemy, hero).map(f32::from);
    let value = match (forward, reverse) {
        (Some(f), Some(r)) => (f - r) / 2.0,
        (Some(f), None) => f,
        (None, Some(r)) => -r,
        (None, None) => return None,
    };
    Some(value / 100.0)
}

/// How much this enemy counts, given who you are playing. An id with no hero
/// behind it counts as an ordinary pick rather than vanishing from the average.
fn enemy_weight(ds: &Dataset, ctx: &UserContext, enemy: HeroId) -> f32 {
    match ds.hero(enemy) {
        Ok(entry) => ctx.weights.enemy_roles.get(ctx.role, entry.role),
        Err(_) => 1.0,
    }
}

/// The attack/defend term, and the side it is for.
///
/// Zero unless the map is one that actually has sides *and* you have said which
/// one you are on. A symmetric mode has no answer to the question, so it must not
/// contribute a value either way.
fn side_term(ds: &Dataset, draft: &Draft, hero: HeroId) -> Option<(Side, f32)> {
    let side = draft.side?;
    let map = draft.map?;
    if !ds.map(map).ok()?.mode.has_sides() {
        return None;
    }
    Some((side, f32::from(ds.side_lean(hero)) / 100.0 * side.sign()))
}

/// Scores one candidate and collects the reasoning behind it.
///
/// `enemy_shape` is passed in rather than derived here because it is a property
/// of the draft, not of the candidate: every hero in the list would otherwise
/// recompute the same answer from the same five picks.
fn score_hero(
    ds: &Dataset,
    draft: &Draft,
    ctx: &UserContext,
    enemy_shape: &Shape,
    hero: HeroId,
) -> (f32, Vec<Reason>) {
    let w = &ctx.weights;
    let mut reasons: Vec<Reason> = Vec::new();

    let base = f32::from(ds.base_strength(hero)) / 100.0;
    let base_contribution = w.base * base;

    // How far this hero moves from that on the rung the user picked. Zero at
    // `Rank::All` and zero for a hero no source rated there, so an unset rank
    // reproduces the ranking this app has always produced, term for term.
    let rank_shift = f32::from(ds.rank_shift(ctx.rank, hero)) / 100.0;
    let rank_contribution = w.rank * rank_shift;

    // A weighted mean rather than a plain one, so the enemy tank is not outvoted
    // by the damage pair just because a team fields two of them.
    //
    // Enemies this candidate has no reading against contribute **nothing to the
    // numerator but their weight to the denominator**, and the difference between
    // that and dropping them from both is the whole point. An unrated pair still
    // says nothing about this hero — no term, and no "strong into X" reason for a
    // matchup nobody has an opinion on. What it must not do is make the readings
    // that *do* exist count for more.
    //
    // Normalising over the rated enemies only, as this did, is unbiased for the
    // mean and wrong for the ranking. Measured over 300 random five-enemy drafts:
    // mean counter term is flat across coverage, but the standard deviation at one
    // rated enemy is 1.88x the deviation at five, and a list is chosen by its
    // maximum. A candidate rated against one enemy won the counter term 3.5x as
    // often as one rated against all five (P(top) 0.200 against 0.057); with this
    // denominator, 0.000 against 0.067. Concretely, Emre — 24 rated rows of 52 and
    // a *below-average* mean counter — topped the damage list in 19 of 276 drafts
    // while Soldier: 76, better mean and full coverage, topped it none.
    //
    // The fold in `matchup_term` is deliberately left alone: a lone reading is
    // still worth its full magnitude, because that is a statement about one pair.
    // This is a statement about comparing estimates of unequal precision.
    let rated: Vec<(HeroId, f32)> = draft
        .enemies
        .iter()
        .filter_map(|enemy| matchup_term(ds, hero, *enemy).map(|term| (*enemy, term)))
        .collect();

    let mut counter_total = 0.0;
    let total_weight: f32 = draft
        .enemies
        .iter()
        .map(|enemy| enemy_weight(ds, ctx, *enemy))
        .sum();
    if total_weight > 0.0 {
        for (enemy, term) in &rated {
            let share = enemy_weight(ds, ctx, *enemy) / total_weight;
            counter_total += share * term;

            // A rated dead even is a real reading, but "strong into X" is not
            // what it says — and neither is the `Some(0.0)` a mirror match
            // returns. Both stay in the mean and out of the panel, which is the
            // same rule every term below follows.
            if *term != 0.0 {
                let contribution = w.counter * share * term;
                let kind = if *term > 0.0 {
                    ReasonKind::BeatsEnemy(*enemy)
                } else {
                    ReasonKind::LosesToEnemy(*enemy)
                };
                reasons.push(Reason {
                    kind,
                    contribution,
                    text: ds.reason(hero, *enemy).unwrap_or_default().to_owned(),
                });
            }
        }
    }

    // Terms of exactly zero are added to the score but never explained. A pair
    // with no synergy entry, a hero with no affinity for this map and a hero
    // with no side lean all compute to 0.0, and a zero-contribution reason
    // renders as "pairs well with", "performs well on" or "suits attack" — a
    // positive claim built out of an empty cell. Most of these files are mostly
    // empty, so this is the common case rather than the edge one.
    //
    // Allies this candidate has no synergy reading against are left out of the
    // mean entirely — and **unlike the counter mean above, they are left out of
    // the denominator too.** The two look like the same normalisation and are not,
    // so this is deliberate rather than an inconsistency to tidy up.
    //
    // The counter mean can divide by every enemy because matchup values are signed
    // and centred: over the committed matrix, 39% positive, 43% negative, mean
    // -0.63. The expected contribution of a pair nobody rated is therefore about
    // zero, so counting it in the denominator costs a hero nothing it was owed —
    // it only stops thin coverage reading as conviction.
    //
    // Synergy is one-signed. All 441 committed pairs are positive, mean +35, and
    // the file covers 21% of the roster's ordered pairs against the matrix's 92%.
    // Divide by every ally here and the term stops measuring how well a hero pairs
    // and starts measuring how many of your allies the source happened to list it
    // with: the expected value of an absent reading is +35, not 0, so omitting it
    // from the numerator while charging it to the denominator is a penalty rather
    // than a neutral dilution. It would land hardest on the heroes the source
    // lists least, which are the tanks — Roadhog has 6 rated partners of 52,
    // Reinhardt 8.
    //
    // Measured: the mean term is flat across coverage (+0.318 at one rated ally,
    // +0.323 at three), so the same variance inflation exists here as in the
    // counter mean — the standard deviation runs 0.172 at one against 0.106 at
    // three. But the cure available there is worse than the disease here. If this
    // ever needs fixing, it wants a shrink toward the file's own positive mean —
    // the shape of `selection_shrink` in the ingest — and not this denominator.
    //
    // `rating` rather than `get`, because here "nothing known" and "even" are
    // emphatically not the same answer.
    let mut synergy_total = 0.0;
    let rated_allies: Vec<(HeroId, f32)> = draft
        .allies
        .iter()
        .filter_map(|ally| {
            ds.synergy()
                .rating(hero, *ally)
                .map(|value| (*ally, f32::from(value) / 100.0))
        })
        .collect();
    if !rated_allies.is_empty() {
        for (ally, term) in &rated_allies {
            synergy_total += term;
            if *term != 0.0 {
                reasons.push(Reason {
                    kind: ReasonKind::PairsWithAlly(*ally),
                    contribution: w.synergy * term / rated_allies.len() as f32,
                    text: String::new(),
                });
            }
        }
        synergy_total /= rated_allies.len() as f32;
    }

    let map_term = match draft.map {
        Some(map) => {
            let term = f32::from(ds.map_affinity(map, hero)) / 100.0;
            if term != 0.0 {
                reasons.push(Reason {
                    kind: ReasonKind::MapFit(map),
                    contribution: w.map * term,
                    text: String::new(),
                });
            }
            term
        }
        None => 0.0,
    };

    // Zero for most of the roster, and for every symmetric mode, so this only
    // shows up as a reason where it actually says something.
    let side = match side_term(ds, draft, hero) {
        Some((side, term)) if term != 0.0 => {
            reasons.push(Reason {
                kind: ReasonKind::SideFit(side),
                contribution: w.side * term,
                text: String::new(),
            });
            term
        }
        _ => 0.0,
    };

    // What the enemy is building, rather than who is on it. Zero whenever
    // either side is unread — an uncurated candidate, an enemy team nobody has
    // picked into yet, or an enemy team with no committed shape — and, like
    // every term above, a zero is added to the score but never turned into a
    // sentence.
    //
    // Named for *their* leading axis rather than the candidate's own, because
    // "dive" next to Winston's portrait says nothing the portrait did not; the
    // half worth reading is what it is dive *into*.
    let shape = enemy_shape.against(ds.shape(hero));
    if shape != 0.0 {
        if let Some(theirs) = enemy_shape.leading() {
            reasons.push(Reason {
                kind: if shape > 0.0 {
                    ReasonKind::CountersShape(theirs)
                } else {
                    ReasonKind::LosesToShape(theirs)
                },
                contribution: w.shape * shape,
                text: String::new(),
            });
        }
    }

    let personal = ctx.override_for(hero);
    if personal != 0.0 {
        reasons.push(Reason {
            kind: ReasonKind::Comfort,
            contribution: w.personal * personal,
            text: String::new(),
        });
    }
    if base_contribution.abs() > f32::EPSILON {
        reasons.push(Reason {
            kind: ReasonKind::BaseStrength,
            contribution: base_contribution,
            text: String::new(),
        });
    }
    if rank_contribution.abs() > f32::EPSILON {
        reasons.push(Reason {
            kind: ReasonKind::RankFit(ctx.rank),
            contribution: rank_contribution,
            text: String::new(),
        });
    }

    let score = base_contribution
        + rank_contribution
        + w.counter * counter_total
        + w.synergy * synergy_total
        + w.map * map_term
        + w.side * side
        + w.shape * shape
        + w.personal * personal;

    // Biggest movers first, in either direction — being told what is about to
    // go wrong matters as much as being told what works.
    reasons.sort_by(|a, b| {
        b.contribution
            .abs()
            .partial_cmp(&a.contribution.abs())
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    reasons.truncate(MAX_REASONS);

    (score, reasons)
}

/// Ranks every eligible hero for the current draft, best first.
///
/// Runs in a few microseconds for a 53-hero roster, which is why the client
/// scores locally and the sync socket only ever carries [`Draft`] deltas.
pub fn recommend(
    ds: &Dataset,
    draft: &Draft,
    ctx: &UserContext,
) -> Result<Vec<Recommendation>, CoreError> {
    if ctx.overrides.len() != ds.hero_count() {
        return Err(CoreError::RosterLengthMismatch {
            what: "overrides",
            expected: ds.hero_count(),
            actual: ctx.overrides.len(),
        });
    }

    // One read of the enemy team, shared by every candidate below.
    let enemy_shape = shape_of(ds, &draft.enemies);

    let locked_score = draft
        .locked
        .map(|hero| score_hero(ds, draft, ctx, &enemy_shape, hero).0);

    let mut out: Vec<Recommendation> = ds
        .heroes_in_role(ctx.role)
        // Your team cannot run duplicates. Enemies are not filtered: both teams
        // may field the same hero, and mirroring is often the right answer.
        .filter(|hero| !draft.allies.contains(hero))
        .map(|hero| {
            let (score, reasons) = score_hero(ds, draft, ctx, &enemy_shape, hero);
            let delta = locked_score.map(|locked| score - locked);
            Recommendation {
                hero,
                score,
                delta_vs_locked: delta,
                worth_swapping: draft.locked.is_some_and(|l| l != hero)
                    && delta.is_some_and(|d| d > ctx.weights.swap_threshold),
                is_locked: draft.locked == Some(hero),
                reasons,
            }
        })
        .collect();

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    Ok(out)
}

/// Ranks the enemy team by how hard each one is beating `hero`.
///
/// This is the signal for *whether* to swap, which the recommendation list
/// alone does not give you.
///
/// Ordering uses the same role weights as the scorer, so a support sees the
/// diver above a marginally worse tank matchup. `severity` itself stays the raw
/// matchup value: it is rendered as a bare number next to the reason text, and a
/// weighted one would be a number nothing on screen explains.
///
/// Enemies nobody has rated are left off. A hero the sources have no opinion on
/// is not a known problem, and listing it at a flat `+0` with no rationale reads
/// as a measurement rather than as the silence it is.
pub fn threats(ds: &Dataset, draft: &Draft, ctx: &UserContext, hero: HeroId) -> Vec<Threat> {
    let mut out: Vec<(f32, Threat)> = draft
        .enemies
        .iter()
        .filter_map(|enemy| {
            let severity = -matchup_term(ds, hero, *enemy)?;
            Some((
                severity * enemy_weight(ds, ctx, *enemy),
                Threat {
                    enemy: *enemy,
                    severity,
                    text: ds.reason(hero, *enemy).unwrap_or_default().to_owned(),
                    disputed: ds.sources_disagree(hero, *enemy),
                },
            ))
        })
        .collect();

    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(core::cmp::Ordering::Equal));
    out.into_iter().map(|(_, threat)| threat).collect()
}

/// How much a team member who has said only their role counts, against the 1.0
/// of one who has locked in or marked a pool.
///
/// Their heroes are an entire role, so their danger for most candidates averages
/// close to zero: including them at full weight would let four silent teammates
/// flatten the one real signal on the board, and excluding them entirely would
/// throw away the one thing they have said — that somebody is holding a slot in
/// that role, which is why a candidate that beats the role at all matters more
/// than one that does not.
///
/// A quarter lets the shape of the team register without letting it drown the
/// people who have actually answered. It is a judgement call, like
/// [`EnemyRoleWeights`], and unlike those numbers it has no measured spread
/// behind it — but it only ever scales an already near-zero term, so the room it
/// has to be wrong in is small.
const UNKNOWN_CERTAINTY: f32 = 0.25;

/// Names the question the ban list is answering, from who has said what.
fn ban_subject(team: &DefendedTeam) -> BanSubject {
    let known: Vec<&Defended> = team.members.iter().filter(|m| m.is_known()).collect();
    match known.as_slice() {
        [] => BanSubject::Patch,
        [only] => BanSubject::One {
            who: only.who.clone(),
            is_me: only.is_me,
            locked: matches!(only.knowledge, Knowledge::Locked(_)),
            heroes: only.heroes.len(),
        },
        many => BanSubject::Team {
            known: many.len(),
            locked: many
                .iter()
                .filter(|m| matches!(m.knowledge, Knowledge::Locked(_)))
                .count(),
        },
    }
}

/// Ranks the roster by how strong it is in the current patch.
///
/// The rung the list falls back to when nobody has marked a pool and nobody has
/// picked. There is no team to answer about yet, and the honest thing to say
/// then is not a role-wide matchup average — which is nearly flat, and reads as
/// an answer while carrying almost no information — but the one fact that does
/// exist: these are the heroes winning right now.
///
/// This is the *only* rung that consults patch strength. The moment anybody says
/// anything, the score goes back to pure threat, so no single number ever mixes
/// "strong right now" with "bad for us" — two different arguments for a ban that
/// a reader cannot separate again once they are added together.
fn ban_by_strength(ds: &Dataset, draft: &Draft, ctx: &UserContext) -> Vec<BanCandidate> {
    let mut candidates: Vec<BanCandidate> = (0..ds.hero_count())
        .map(|index| HeroId(index as u16))
        .filter(|hero| bannable(draft, *hero))
        .filter_map(|hero| {
            // On the same -1.0..=1.0 scale as every other score here, so the
            // column reads the same way it does on the other rungs.
            //
            // Read at the rung the user picked, because the claim this rung makes
            // is "these are the heroes winning right now" — and once you have
            // said which ladder you are on, "right now" means there. Whatever
            // this sorts by is also what the column displays; the two cannot
            // disagree.
            let score = f32::from(ds.base_strength_at(ctx.rank, hero)) / 100.0;
            // Base strength is symmetric about zero by construction — see
            // `crate::normalize` — so this keeps the above-average half of the
            // roster, which is the half a ban has an argument for.
            if score <= 0.0 {
                return None;
            }
            let prevalence = ds.prevalence_at(ctx.rank, hero);
            Some(BanCandidate {
                hero,
                // Discounted by who actually turns up. Applied *after* the gate
                // above, so the prior decides the order and never the membership.
                score: score * prevalence_factor(ctx, prevalence),
                // Raw, which on this rung deliberately breaks the identity these
                // two used to have: `severity` is the reading and `score` is the
                // reading times a prior, and only one of them is what the column
                // is sorted by.
                severity: score,
                prevalence,
                // No pair produced this, and naming one would be a claim
                // nothing here made.
                worst: None,
                worst_owner: None,
                text: String::new(),
            })
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    candidates
}

/// Discounts a ban by how rarely the hero turns up. See [`Weights::prevalence`].
///
/// A multiplier and not a term, because prevalence is not an argument for a ban —
/// it is a discount on one. Strictly positive at any sane weight, so it reorders
/// the list and can never put a hero on it or take one off: that is decided above,
/// by whether the sources say the hero beats your side.
///
/// Takes the reading rather than the hero, so the caller has already asked
/// [`Dataset::prevalence_at`] once and the number it multiplies by is the same one
/// it hands to the panel.
fn prevalence_factor(ctx: &UserContext, prevalence: i8) -> f32 {
    1.0 + ctx.weights.prevalence * f32::from(prevalence) / 100.0
}

/// Whether `hero` is still a hero anybody could ban.
///
/// One already on the board cannot be banned any more, and one on your own team
/// is a hero you would be banning from yourself.
fn bannable(draft: &Draft, hero: HeroId) -> bool {
    !draft.enemies.contains(&hero) && !draft.allies.contains(&hero) && draft.locked != Some(hero)
}

/// Ranks every hero by how much denying them would help *the team*.
///
/// This is the ban phase's question, and it is not the pick phase's question
/// inverted: a ban lands before anyone has picked, so the enemy team is usually
/// empty and there is nothing to counter *into*. What there is is your team —
/// or, more often, the sets of heroes each of them might end up on, which is why
/// an unpicked draft averages across pools rather than declining to answer.
///
/// A ban is spent once for five people, so it is scored for five people. Each
/// member contributes the mean matchup against the heroes they might play, read
/// through the [`EnemyRoleWeights`] row for *their* role rather than yours — an
/// enemy tank is the tank player's problem whoever is looking at the screen.
/// Members are then averaged by [`Knowledge`], so somebody who has marked a pool
/// counts fully and somebody who has only queued counts a quarter.
///
/// Averaging the role weight across the team is also what makes this format-aware
/// for free: a 6v6 roster holds two tank slots, so enemy tanks weigh more,
/// arrived at from the shape of the team rather than from a second hand-written
/// weight table there is no corpus for.
///
/// **Heroes the team plays are not candidates.** A ban takes the hero off the
/// table for everyone, so recommending one of your own is recommending you lose
/// a pick to deny it — and one in a teammate's pool is worse, because it costs
/// somebody else the hero on your say-so. The pool highlights rather than
/// restricts everywhere else in this app; here it excludes, because here it is
/// the cost rather than a preference.
pub fn ban_recommendations(
    ds: &Dataset,
    draft: &Draft,
    ctx: &UserContext,
    team: &DefendedTeam,
) -> BanBoard {
    let subject = ban_subject(team);
    if !team.anyone_known() {
        return BanBoard {
            subject,
            candidates: ban_by_strength(ds, draft, ctx),
        };
    }

    let mut candidates: Vec<BanCandidate> = (0..ds.hero_count())
        .map(|index| HeroId(index as u16))
        .filter(|hero| bannable(draft, *hero) && !team.plays(*hero))
        .filter_map(|hero| {
            let role = ds.hero(hero).ok()?.role;

            let mut weighted = 0.0f32;
            let mut plain = 0.0f32;
            let mut total = 0.0f32;
            let mut worst: Option<(HeroId, &Defended, f32)> = None;

            for member in &team.members {
                let certainty = member.certainty();
                // Every member divides, including one this candidate is unrated
                // against. That member contributes no danger — an unrated pair
                // still says nothing, and still produces no reason line — but it
                // must not make the members who *did* rate the candidate count
                // for more than their share of the team.
                //
                // Dividing by the contributing members only, as this did, is what
                // let a barely-rated hero take the top row: rated against 1 of 5
                // it divided by 1.0, rated against all five it divided by 5.0, so
                // one strong pair beat five moderate ones. Mizuki led a Baptiste
                // player's list at 0.5000 off a single pair — 23 rated rows of 52 —
                // above a fully-rated Ramattra at 0.3464, whose numerator was three
                // and a half times larger. Over 300 random comps, Emre reached the
                // drawn top eight 68 times against 16 with this denominator.
                //
                // It is variance, not bias: the mean danger is flat across
                // coverage, but `score <= 0` below discards the unfavourable tail
                // and the panel draws only the first eight, so a thin candidate's
                // wider spread reaches the user in one direction.
                total += certainty;

                let rated: Vec<(HeroId, f32)> = member
                    .heroes
                    .iter()
                    .filter_map(|theirs| {
                        matchup_term(ds, *theirs, hero).map(|term| (*theirs, -term))
                    })
                    .collect();
                if rated.is_empty() {
                    continue;
                }

                let danger = rated.iter().map(|(_, term)| term).sum::<f32>() / rated.len() as f32;
                // `ban_weight` and not `get`: this is the one place candidates of
                // different roles are ranked against each other, and the raw cell
                // carries a cross-role scale that was only ever meant to
                // redistribute shares within one column. See `ban_weight`.
                weighted +=
                    certainty * ctx.weights.enemy_roles.ban_weight(member.role, role) * danger;
                plain += certainty * danger;

                // Only somebody who has actually said what they play can have a
                // hero take the worst of it. An `Unknown` member's worst case is
                // an arbitrary hero of a role nobody claimed.
                if member.is_known() {
                    for (theirs, term) in rated {
                        if worst.is_none_or(|(_, _, best)| term > best) {
                            worst = Some((theirs, member, term));
                        }
                    }
                }
            }

            // An empty team, which `BanSubject` cannot actually produce — kept as
            // the guard against dividing by zero rather than as a rule about
            // coverage. Coverage is now the `score <= 0.0` gate's job: a candidate
            // nobody on the team is rated against contributes nothing to
            // `weighted`, so it scores exactly zero and drops out just below.
            // Silence is still not safety; it is just stated one line later.
            if total <= 0.0 {
                return None;
            }
            let score = weighted / total;
            // A hero your side of the draft already beats is not a ban. Saying
            // so by omission keeps the list to heroes there is an argument for.
            if score <= 0.0 {
                return None;
            }
            // Nobody who has said what they play is rated against this hero, so
            // the only thing behind its score is a role somebody is queued in.
            // It drops out, and the `?` is the whole rule.
            //
            // This used to carry a second job it was bad at. `total` summed the
            // certainty of the members that had a rating, so a candidate rated
            // *only* by an `Unknown` member divided its 0.25 straight back out and
            // landed at full weight — the discount evaporated exactly where it was
            // supposed to bite, and this gate was the patch. The denominator above
            // now counts every member, so the discount survives on its own and
            // this is back to being about the missing "hardest on" line: there is
            // no hero to name, so there is no row to draw.
            let (worst_hero, owner, _) = worst?;

            let prevalence = ds.prevalence_at(ctx.rank, hero);
            Some(BanCandidate {
                hero,
                // As on the patch rung: the argument decided membership above, and
                // the prior only decides where in the list it lands.
                score: score * prevalence_factor(ctx, prevalence),
                severity: plain / total,
                prevalence,
                worst: Some(worst_hero),
                worst_owner: (!owner.is_me && !owner.is_typed).then(|| owner.who.clone()),
                text: ds.reason(worst_hero, hero).unwrap_or_default().to_owned(),
            })
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    BanBoard {
        subject,
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{Format, Queue, TeamSize};

    /// What the counter term actually gives the enemy tanks, in each format.
    fn tank_share(format: Format, yours: Role) -> f32 {
        let weights = EnemyRoleWeights::default();
        let total: f32 = Role::ALL
            .iter()
            .map(|theirs| weights.get(yours, *theirs) * format.slots(*theirs) as f32)
            .sum();
        weights.get(yours, Role::Tank) * format.slots(Role::Tank) as f32 / total
    }

    /// Pins the arithmetic the format changes, so that re-tuning for 6v6 is a
    /// decision somebody makes rather than a number that moved.
    ///
    /// The 2.2 on the tank cell was chosen for a team with *one* tank in it: it
    /// buys the enemy tank about a third of the counter signal instead of the
    /// fifth a plain average would give it. In 6v6 the same weight is paid twice
    /// and the tanks take about half. See [`EnemyRoleWeights`] for why that is
    /// left alone rather than guessed at.
    #[test]
    fn the_enemy_tanks_take_half_the_counter_signal_in_6v6() {
        let five = tank_share(Format::new(TeamSize::FiveVFive, Queue::Role), Role::Tank);
        let six = tank_share(Format::new(TeamSize::SixVSix, Queue::Role), Role::Tank);

        assert!((five - 0.355).abs() < 0.005, "5v5 tank share was {five}");
        assert!((six - 0.524).abs() < 0.005, "6v6 tank share was {six}");
    }
}
