use serde::{Deserialize, Serialize};

use crate::dataset::Dataset;
use crate::draft::Draft;
use crate::error::CoreError;
use crate::hero::{HeroId, HeroSet, Role};
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

/// Whose matchups the ban list is reading, and therefore what a ban defends.
///
/// The ban phase runs before anyone picks, so most of the time there is no "you"
/// yet — only the set of heroes you might end up on. That is what the two
/// unlocked variants are for, and the UI names whichever one is live so the
/// number on screen is never ambiguous about who it is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BanSubject {
    /// Your locked pick. One hero, so the numbers are simply its own matchups.
    Locked(HeroId),
    /// Your pool for the role you are picking, averaged. Any of them could be
    /// the hero you end up on, so a ban has to defend all of them at once.
    Pool,
    /// The whole role, averaged, because no pool is marked yet. An empty pool
    /// means "I have not said who I play", which is a reason to answer for the
    /// role rather than a reason to answer nothing.
    Role(Role),
}

/// One hero worth denying the enemy.
#[derive(Debug, Clone, PartialEq)]
pub struct BanCandidate {
    pub hero: HeroId,
    /// Role-weighted danger. Higher is more worth banning, and this is what the
    /// list is sorted by.
    pub score: f32,
    /// The unweighted mean matchup against the subject set, on the same
    /// -1.0..=1.0 scale as everything else. Positive means it beats you.
    ///
    /// Not what the panel displays. [`threats`] can show its raw severity
    /// because it weights across one enemy team, where the two orderings barely
    /// differ; a ban list ranks the whole roster through a 2.2× role weight, so
    /// a column sorted by `score` while showing `severity` would visibly
    /// disagree with itself.
    pub severity: f32,
    /// Whichever of your heroes it hurts most. Always set — the sort needs a
    /// worst case to exist before a candidate can rank at all — and equal to the
    /// subject itself when you are locked.
    pub worst: HeroId,
    /// The scraped sentence for the `worst` pair, where one exists.
    pub text: String,
}

/// The ban list and the question it answers.
#[derive(Debug, Clone, PartialEq)]
pub struct BanBoard {
    pub subject: BanSubject,
    /// Best ban first. Only heroes that actually beat you are here.
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
fn score_hero(ds: &Dataset, draft: &Draft, ctx: &UserContext, hero: HeroId) -> (f32, Vec<Reason>) {
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

    let locked_score = draft.locked.map(|hero| score_hero(ds, draft, ctx, hero).0);

    let mut out: Vec<Recommendation> = ds
        .heroes_in_role(ctx.role)
        // Your team cannot run duplicates. Enemies are not filtered: both teams
        // may field the same hero, and mirroring is often the right answer.
        .filter(|hero| !draft.allies.contains(hero))
        .map(|hero| {
            let (score, reasons) = score_hero(ds, draft, ctx, hero);
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

/// What a ban is defending, and the heroes that resolves to.
///
/// The pool is intersected with the role you are picking rather than trusted
/// wholesale. It is stored per role already, so the intersection is normally the
/// whole set — but a stored profile is user data, and one that has drifted must
/// not quietly average a tank into a support's answer.
fn ban_subject(
    ds: &Dataset,
    draft: &Draft,
    ctx: &UserContext,
    pool: &HeroSet,
) -> (BanSubject, Vec<HeroId>) {
    if let Some(hero) = draft.locked {
        return (BanSubject::Locked(hero), vec![hero]);
    }

    let pooled: Vec<HeroId> = ds
        .heroes_in_role(ctx.role)
        .filter(|hero| pool.contains(*hero))
        .collect();
    if pooled.is_empty() {
        (
            BanSubject::Role(ctx.role),
            ds.heroes_in_role(ctx.role).collect(),
        )
    } else {
        (BanSubject::Pool, pooled)
    }
}

/// Ranks every hero by how much denying them would help you.
///
/// This is the ban phase's question, and it is not the pick phase's question
/// inverted: a ban lands before anyone has picked, so the enemy team is usually
/// empty and there is nothing to counter *into*. What there is is you — or, more
/// often, the set of heroes you might end up on, which is why an unpicked draft
/// averages across your pool rather than declining to answer.
///
/// The score is pure threat: how hard the candidate beats the heroes you are
/// defending, scaled by [`EnemyRoleWeights`] so an enemy tank counts for a tank
/// player the way it does everywhere else in the scorer. Patch strength is
/// deliberately not folded in — "strong right now" and "bad for me" are two
/// different arguments for a ban, and a single number that mixes them can only
/// be read as neither.
///
/// Heroes you have marked as your own are ranked like any other. The pool
/// highlights rather than restricts here as everywhere else, and a hero in it
/// scores a rated dead even against itself, so being yours never inflates your
/// own danger.
pub fn ban_recommendations(
    ds: &Dataset,
    draft: &Draft,
    ctx: &UserContext,
    pool: &HeroSet,
) -> BanBoard {
    let (subject, defends) = ban_subject(ds, draft, ctx, pool);

    let mut candidates: Vec<BanCandidate> = (0..ds.hero_count())
        .map(|index| HeroId(index as u16))
        // A hero already on the board cannot be banned any more, and one on your
        // own team is a hero you would be banning from yourself.
        .filter(|hero| {
            !draft.enemies.contains(hero)
                && !draft.allies.contains(hero)
                && draft.locked != Some(*hero)
        })
        .filter_map(|hero| {
            let role = ds.hero(hero).ok()?.role;

            // Pairs nobody has rated are left out of the mean rather than folded
            // in as a zero, for the same reason the counter term leaves them out:
            // averaging in the absence of evidence drags a barely-rated hero
            // toward the middle and invents a reading for a matchup the sources
            // have no opinion on. A candidate rated against none of your heroes
            // drops out entirely — silence is not safety.
            let rated: Vec<(HeroId, f32)> = defends
                .iter()
                .filter_map(|mine| matchup_term(ds, *mine, hero).map(|term| (*mine, -term)))
                .collect();
            let (worst, _) =
                rated.iter().copied().reduce(
                    |worst, next| {
                        if next.1 > worst.1 {
                            next
                        } else {
                            worst
                        }
                    },
                )?;

            let severity = rated.iter().map(|(_, term)| term).sum::<f32>() / rated.len() as f32;
            let score = severity * ctx.weights.enemy_roles.get(ctx.role, role);
            // A hero your side of the draft already beats is not a ban. Saying
            // so by omission keeps the list to heroes there is an argument for.
            if score <= 0.0 {
                return None;
            }

            Some(BanCandidate {
                hero,
                score,
                severity,
                worst,
                text: ds.reason(worst, hero).unwrap_or_default().to_owned(),
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
