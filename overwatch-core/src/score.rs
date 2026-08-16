use serde::{Deserialize, Serialize};

use crate::archetype::{shape_of, Archetype, Shape};
use crate::dataset::Dataset;
use crate::draft::Draft;
use crate::error::CoreError;
use crate::hero::{HeroId, Role};
use crate::map::{MapId, Side};

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

impl Default for Weights {
    fn default() -> Self {
        Self {
            base: 0.15,
            counter: 1.0,
            synergy: 0.30,
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
    /// Personal nudges on the -100..=100 scale, indexed by hero. Never written
    /// by the ingest.
    pub overrides: Vec<i8>,
    pub weights: Weights,
}

impl UserContext {
    pub fn new(role: Role, hero_count: usize) -> Self {
        Self {
            role,
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

    // A weighted mean rather than a plain one, so the enemy tank is not outvoted
    // by the damage pair just because a team fields two of them. Normalising over
    // the enemies actually entered means a lone first pick weighs `w/w == 1`:
    // role weighting only starts to bite as the team fills, and never distorts
    // the partial-input answer this app is built around.
    //
    // Enemies this candidate has no reading against are left out of the mean
    // entirely rather than folded in as a zero. Counting them would be averaging
    // in the absence of evidence — it drags a hero the sources have barely rated
    // toward the middle of the field and, worse, produces a "strong into X"
    // reason for a matchup nobody has an opinion on. The cost is that a candidate
    // with one known matchup out of five enemies now leans entirely on that one;
    // the sources' coverage, not this mean, is the thing to fix there.
    let rated: Vec<(HeroId, f32)> = draft
        .enemies
        .iter()
        .filter_map(|enemy| matchup_term(ds, hero, *enemy).map(|term| (*enemy, term)))
        .collect();

    let mut counter_total = 0.0;
    let total_weight: f32 = rated
        .iter()
        .map(|(enemy, _)| enemy_weight(ds, ctx, *enemy))
        .sum();
    if total_weight > 0.0 {
        for (enemy, term) in &rated {
            let share = enemy_weight(ds, ctx, *enemy) / total_weight;
            counter_total += share * term;

            let contribution = w.counter * share * term;
            let kind = if *term >= 0.0 {
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

    // Terms of exactly zero are added to the score but never explained. A pair
    // with no synergy entry, a hero with no affinity for this map and a hero
    // with no side lean all compute to 0.0, and a zero-contribution reason
    // renders as "pairs well with", "performs well on" or "suits attack" — a
    // positive claim built out of an empty cell. Most of these files are mostly
    // empty, so this is the common case rather than the edge one.
    let mut synergy_total = 0.0;
    if !draft.allies.is_empty() {
        for ally in &draft.allies {
            let term = f32::from(ds.synergy().get(hero, *ally)) / 100.0;
            synergy_total += term;
            if term != 0.0 {
                reasons.push(Reason {
                    kind: ReasonKind::PairsWithAlly(*ally),
                    contribution: w.synergy * term / draft.allies.len() as f32,
                    text: String::new(),
                });
            }
        }
        synergy_total /= draft.allies.len() as f32;
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

    let score = base_contribution
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
fn ban_by_strength(ds: &Dataset, draft: &Draft) -> Vec<BanCandidate> {
    let mut candidates: Vec<BanCandidate> = (0..ds.hero_count())
        .map(|index| HeroId(index as u16))
        .filter(|hero| bannable(draft, *hero))
        .filter_map(|hero| {
            // On the same -1.0..=1.0 scale as every other score here, so the
            // column reads the same way it does on the other rungs.
            let score = f32::from(ds.base_strength(hero)) / 100.0;
            // Base strength is symmetric about zero by construction — see
            // `crate::normalize` — so this keeps the above-average half of the
            // roster, which is the half a ban has an argument for.
            if score <= 0.0 {
                return None;
            }
            Some(BanCandidate {
                hero,
                score,
                severity: score,
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
            candidates: ban_by_strength(ds, draft),
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
                // Pairs nobody has rated are left out of the mean rather than
                // folded in as a zero, for the same reason the counter term
                // leaves them out: averaging in the absence of evidence drags a
                // barely-rated hero toward the middle and invents a reading for
                // a matchup the sources have no opinion on. A candidate rated
                // against nobody on the team drops out entirely — silence is
                // not safety.
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
                let certainty = member.certainty();
                weighted += certainty * ctx.weights.enemy_roles.get(member.role, role) * danger;
                plain += certainty * danger;
                total += certainty;

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
            // This is not tidiness about the missing "hardest on" line. `total`
            // sums the certainty of the members that actually had a rating, so
            // a candidate rated *only* by an `Unknown` member divides its 0.25
            // straight back out again and lands at full weight — the discount
            // evaporates exactly where it was supposed to bite. Ranking a hero
            // off a role nobody claimed, above one your own pool is measured
            // against, is the failure that guards against.
            let (worst_hero, owner, _) = worst?;

            Some(BanCandidate {
                hero,
                score,
                severity: plain / total,
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
