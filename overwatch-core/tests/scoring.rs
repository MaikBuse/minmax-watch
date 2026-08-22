//! Golden cases for the scoring engine, built on a hand-made seven-hero
//! fixture whose matchup values are the real counterpickgg difficulties
//! (Pharah is 9/10 into Reinhardt, D.Va is 2/10 into Pharah, and so on).
//!
//! The ratings are copied from the cached pages in `data/sources`, so every
//! mirrored pair sums to 10 the way the site's own data does. That is worth
//! preserving when these are refreshed: a fixture whose pairs sum to anything
//! else is describing a scale the source does not use, and cannot catch a
//! conversion that gets the midpoint wrong.

use overwatch_core::{
    ban_recommendations, difficulty_to_value, recommend, threats, Archetype, BanBoard, BanSubject,
    ComfortStep, Dataset, DatasetParts, Defended, DefendedTeam, Draft, EnemyRoleWeights, GameMap,
    GameMode, Hero, HeroId, Knowledge, MapId, Matrix, Rank, ReasonKind, Role, Side, TermKind,
    UserContext, Weights,
};

const REINHARDT: HeroId = HeroId(0);
const SIGMA: HeroId = HeroId(1);
const DVA: HeroId = HeroId(2);
const PHARAH: HeroId = HeroId(3);
const WIDOWMAKER: HeroId = HeroId(4);
const ANA: HeroId = HeroId(5);
const LUCIO: HeroId = HeroId(6);

const KINGS_ROW: MapId = MapId(0);

fn hero(key: &str, name: &str, role: Role) -> Hero {
    Hero {
        key: key.to_owned(),
        name: name.to_owned(),
        role,
        subrole: None,
        aliases: Vec::new(),
    }
}

/// Seven heroes, with the tank-vs-DPS matchups populated from real difficulty
/// ratings and everything else left neutral.
fn fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("sigma", "Sigma", Role::Tank),
        hero("dva", "D.Va", Role::Tank),
        hero("pharah", "Pharah", Role::Damage),
        hero("widowmaker", "Widowmaker", Role::Damage),
        hero("ana", "Ana", Role::Support),
        hero("lucio", "Lúcio", Role::Support),
    ];
    let n = heroes.len();

    let maps = vec![GameMap {
        key: "kings-row".to_owned(),
        name: "King's Row".to_owned(),
        mode: GameMode::Hybrid,
        aliases: Vec::new(),
    }];

    let mut matchups = Matrix::unrated(n);
    // (row hero, column hero, difficulty the row hero faces)
    let ratings = [
        (REINHARDT, PHARAH, 9.0),
        (PHARAH, REINHARDT, 1.0),
        (REINHARDT, WIDOWMAKER, 7.0),
        (WIDOWMAKER, REINHARDT, 3.0),
        (SIGMA, PHARAH, 6.0),
        (PHARAH, SIGMA, 4.0),
        (SIGMA, WIDOWMAKER, 5.0),
        (WIDOWMAKER, SIGMA, 5.0),
        (DVA, PHARAH, 2.0),
        (PHARAH, DVA, 8.0),
        (DVA, WIDOWMAKER, 1.0),
        (WIDOWMAKER, DVA, 9.0),
    ];
    for (a, b, difficulty) in ratings {
        matchups
            .set(a, b, difficulty_to_value(difficulty))
            .expect("fixture indices are in range");
    }

    let mut reasons = vec![String::new(); n * n];
    reasons[REINHARDT.index() * n + PHARAH.index()] =
        "Reinhardt is very weak against airborne targets.".to_owned();
    reasons[DVA.index() * n + PHARAH.index()] =
        "Defense Matrix eats Pharah's rockets and D.Va can follow her into the air.".to_owned();

    Dataset::new(DatasetParts {
        heroes,
        maps,
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: vec![0; n],
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons,
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

fn tank_context(ds: &Dataset) -> UserContext {
    UserContext::new(Role::Tank, ds.hero_count())
}

fn rank_of(recs: &[overwatch_core::Recommendation], hero: HeroId) -> usize {
    recs.iter()
        .position(|r| r.hero == hero)
        .expect("hero should be ranked")
}

#[test]
fn dive_tanks_outrank_reinhardt_into_pharah_and_widowmaker() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.add_enemy(PHARAH);
    draft.add_enemy(WIDOWMAKER);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert_eq!(recs.len(), 3, "three tanks in the fixture");
    assert!(
        rank_of(&recs, DVA) < rank_of(&recs, REINHARDT),
        "D.Va must outrank Reinhardt into an air comp"
    );
    assert!(
        rank_of(&recs, SIGMA) < rank_of(&recs, REINHARDT),
        "Sigma must outrank Reinhardt into an air comp"
    );
    assert_eq!(recs[0].hero, DVA, "D.Va is the strongest answer");
    assert!(recs[0].score > 0.0 && recs[2].score < 0.0);
}

#[test]
fn support_mode_never_offers_a_tank() {
    let ds = fixture();
    let ctx = UserContext::new(Role::Support, ds.hero_count());

    let recs = recommend(&ds, &Draft::new(), &ctx).expect("scoring succeeds");

    let picked: Vec<_> = recs.iter().map(|r| r.hero).collect();
    assert_eq!(picked, vec![ANA, LUCIO]);
}

#[test]
fn a_hero_a_teammate_already_took_is_not_suggested() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    // Contrived for the fixture's small roster: the point is that an ally's
    // hero drops out of your candidate list.
    draft.add_ally(DVA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert!(recs.iter().all(|r| r.hero != DVA));
}

#[test]
fn mirroring_an_enemy_pick_stays_allowed() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.add_enemy(SIGMA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert!(
        recs.iter().any(|r| r.hero == SIGMA),
        "both teams may field the same hero"
    );
}

#[test]
fn reasons_cite_the_enemy_and_carry_the_scraped_text() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.add_enemy(PHARAH);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    let rein = &recs[rank_of(&recs, REINHARDT)];
    let losing = rein
        .reasons
        .iter()
        .find(|r| r.kind == ReasonKind::LosesToEnemy(PHARAH))
        .expect("Reinhardt should be flagged as losing to Pharah");
    assert!(losing.contribution < 0.0);
    assert_eq!(
        losing.text,
        "Reinhardt is very weak against airborne targets."
    );

    let dva = &recs[rank_of(&recs, DVA)];
    assert!(dva
        .reasons
        .iter()
        .any(|r| r.kind == ReasonKind::BeatsEnemy(PHARAH) && r.contribution > 0.0));
}

#[test]
fn swap_mode_fires_on_a_real_upgrade() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.add_enemy(PHARAH);
    draft.add_enemy(WIDOWMAKER);
    draft.locked = Some(REINHARDT);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    let dva = &recs[rank_of(&recs, DVA)];
    assert!(dva.worth_swapping, "D.Va is a large upgrade over Reinhardt");
    assert!(dva.delta_vs_locked.is_some_and(|d| d > 1.0));

    let rein = &recs[rank_of(&recs, REINHARDT)];
    assert!(rein.is_locked);
    assert!(!rein.worth_swapping, "you never swap to yourself");
    assert_eq!(rein.delta_vs_locked, Some(0.0));
}

#[test]
fn swap_mode_stays_quiet_on_a_marginal_gain() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);
    // A small comfort nudge worth 0.6 * 0.10 = 0.06, well under the 0.15
    // threshold: not enough to justify abandoning a working hero.
    ctx.overrides[DVA.index()] = 10;

    let mut draft = Draft::new();
    draft.locked = Some(REINHARDT);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    let dva = &recs[rank_of(&recs, DVA)];
    let delta = dva.delta_vs_locked.expect("locked hero present");
    assert!(delta > 0.0, "D.Va does score higher");
    assert!(
        delta < ctx.weights.swap_threshold,
        "but by less than the hysteresis threshold"
    );
    assert!(!dva.worth_swapping, "so no swap is suggested");
}

/// The top of the ladder against a counter argument, and the claim the whole
/// term exists to make: a hero you actually play beats the technically correct
/// pick you cannot.
///
/// Reads `ComfortStep::Main.value()` rather than a literal `100` so the ladder
/// and the scorer cannot drift apart — if the top step ever moves, this is where
/// it is felt rather than somewhere downstream.
///
/// Against Widowmaker, Reinhardt reads -0.50 and Sigma 0.00 on the fixture's own
/// numbers, so 0.60 of comfort clears the gap by 0.10. That margin is the reason
/// the top step stays a *claim*: two counter losses of this size still beat it.
#[test]
fn the_top_comfort_step_outranks_a_hero_you_are_countered_by() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.add_enemy(WIDOWMAKER);

    let neutral = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(rank_of(&neutral, REINHARDT) > rank_of(&neutral, SIGMA));

    ctx.overrides[REINHARDT.index()] = ComfortStep::Main.value();
    let tuned = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(rank_of(&tuned, REINHARDT) < rank_of(&tuned, SIGMA));
}

/// The margin the ladder rests on, and the property that makes Slice 15's
/// migration safe to run unattended: every legacy pool entry becomes `ok`, and
/// `ok` alone must never tell somebody to abandon a hero that is working.
///
/// **This must fail loudly if either number moves**, which is why it names both
/// rather than asserting a hard-coded 0.12. `comfort.rs` carries the arithmetic
/// half beside the values; this is the behavioural half, through the real
/// scorer, and the two are deliberately not one test.
///
/// The closest neighbour is `swap_mode_stays_quiet_on_a_marginal_gain` above,
/// which does this at `10` and calls the result "well under". This is the
/// tightest the ladder ever gets: 0.12 against 0.15.
#[test]
fn the_lowest_comfort_step_cannot_on_its_own_argue_for_a_swap() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);

    let contribution = f32::from(ComfortStep::Ok.value()) / 100.0 * ctx.weights.personal;
    assert!(
        contribution < ctx.weights.swap_threshold,
        "the lowest step is worth {contribution} against a swap threshold of {}",
        ctx.weights.swap_threshold
    );

    // No enemies, so comfort is the only thing separating anybody.
    ctx.overrides[DVA.index()] = ComfortStep::Ok.value();
    let mut draft = Draft::new();
    draft.locked = Some(REINHARDT);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let dva = &recs[rank_of(&recs, DVA)];

    let delta = dva.delta_vs_locked.expect("locked hero present");
    assert!(delta > 0.0, "marking a hero does move it up the list");
    assert!(
        !dva.worth_swapping,
        "but never far enough to abandon a working pick: {delta} would have to          clear {}",
        ctx.weights.swap_threshold
    );
}

/// The middle step, sized against the counter term rather than against the other
/// two steps: it should win a close matchup argument and lose a decisive one.
///
/// Both figures come off the fixture's own difficulty table and have to be
/// re-derived if it changes. With one enemy the share is 1.0, and Reinhardt reads
/// -0.50 into Widowmaker against Sigma's 0.00 — a 0.50 gap that 0.33 of comfort
/// does not close. Adding Ana halves both shares without adding a term, because
/// she is unrated against every tank here and an unrated enemy still counts in
/// the denominator. That leaves a 0.25 gap, which 0.33 does close.
#[test]
fn the_middle_comfort_step_loses_to_a_decisive_counter_and_wins_a_close_one() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);
    ctx.overrides[REINHARDT.index()] = ComfortStep::Good.value();

    let mut decisive = Draft::new();
    decisive.add_enemy(WIDOWMAKER);
    let recs = recommend(&ds, &decisive, &ctx).expect("scoring succeeds");
    assert!(
        rank_of(&recs, REINHARDT) > rank_of(&recs, SIGMA),
        "a 0.50 counter gap outlasts 0.33 of comfort"
    );

    let mut close = decisive.clone();
    close.add_enemy(ANA);
    let recs = recommend(&ds, &close, &ctx).expect("scoring succeeds");
    assert!(
        rank_of(&recs, REINHARDT) < rank_of(&recs, SIGMA),
        "a 0.25 gap does not"
    );
}

/// The payload has to be the value the player set, not something plausible
/// derived from the contribution. Dividing `personal` back out would read wrong
/// the moment a stored profile carries a different weight.
#[test]
fn a_comfort_reason_carries_the_value_that_produced_it() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);
    ctx.overrides[REINHARDT.index()] = ComfortStep::Good.value();

    let recs = recommend(&ds, &Draft::new(), &ctx).expect("scoring succeeds");
    let rein = &recs[rank_of(&recs, REINHARDT)];

    let comfort = rein
        .reasons
        .iter()
        .find(|r| matches!(r.kind, ReasonKind::Comfort(_)))
        .expect("a declared comfort produces a line");

    assert_eq!(comfort.kind, ReasonKind::Comfort(55));
    assert!(
        (comfort.contribution - 0.33).abs() < 1e-5,
        "0.55 * 0.60, and it came out {}",
        comfort.contribution
    );
}

#[test]
fn the_map_term_moves_the_ranking() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.map = Some(KINGS_ROW);

    // The fixture has neutral affinity everywhere, so the map must not perturb
    // an otherwise tied field.
    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(recs.iter().all(|r| r.score.abs() < f32::EPSILON));
}

#[test]
fn threat_board_ranks_the_worst_enemy_first() {
    let ds = fixture();

    let mut draft = Draft::new();
    draft.add_enemy(WIDOWMAKER);
    draft.add_enemy(PHARAH);

    let board = threats(&ds, &draft, &tank_context(&ds), REINHARDT);

    assert_eq!(board.len(), 2);
    assert_eq!(board[0].enemy, PHARAH, "Pharah is the bigger problem");
    assert!(board[0].severity > board[1].severity);
    assert!(board[0].severity > 0.0);
    assert_eq!(
        board[0].text,
        "Reinhardt is very weak against airborne targets."
    );
}

#[test]
fn partial_input_already_produces_a_useful_answer() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    // One enemy pick is enough to rank; you never wait for a full team.
    let mut draft = Draft::new();
    draft.add_enemy(PHARAH);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert_eq!(recs[0].hero, DVA);
}

#[test]
fn a_mismatched_override_vector_is_rejected() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);
    ctx.overrides.truncate(2);

    assert!(recommend(&ds, &Draft::new(), &ctx).is_err());
}

// --- role weighting -------------------------------------------------------
//
// A separate fixture from the one above, deliberately symmetric: each candidate
// wins one matchup and loses the mirror of it by exactly the same margin. Under
// a plain average every candidate ties at zero, so anything that separates them
// is the role weighting and nothing else.

const W_REINHARDT: HeroId = HeroId(0);
const W_DVA: HeroId = HeroId(1);
const W_SIGMA: HeroId = HeroId(2);
const W_ANA: HeroId = HeroId(3);
const W_BAPTISTE: HeroId = HeroId(4);
// Appended rather than interleaved, so the ids above — and every assertion
// built on them — keep their meaning.
const W_TRACER: HeroId = HeroId(5);
const W_SOJOURN: HeroId = HeroId(6);
const W_GENJI: HeroId = HeroId(7);

/// Three tanks, two supports and three damage heroes, with `±60` matchups
/// arranged so that every candidate is as far ahead against one enemy as it is
/// behind against another.
fn symmetric_fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("dva", "D.Va", Role::Tank),
        hero("sigma", "Sigma", Role::Tank),
        hero("ana", "Ana", Role::Support),
        hero("baptiste", "Baptiste", Role::Support),
        hero("tracer", "Tracer", Role::Damage),
        hero("sojourn", "Sojourn", Role::Damage),
        hero("genji", "Genji", Role::Damage),
    ];
    let n = heroes.len();

    let mut matchups = Matrix::unrated(n);
    // Reinhardt beats the enemy tank and loses to the enemy support by the same
    // margin; D.Va is the exact mirror. Same for Ana and Baptiste, and for
    // Tracer and Sojourn across the enemy damage pick and the enemy tank.
    let edges = [
        (W_REINHARDT, W_SIGMA, 60),
        (W_REINHARDT, W_ANA, -60),
        (W_DVA, W_SIGMA, -60),
        (W_DVA, W_ANA, 60),
        (W_ANA, W_SIGMA, 60),
        (W_ANA, W_BAPTISTE, -60),
        (W_BAPTISTE, W_SIGMA, -60),
        (W_BAPTISTE, W_ANA, 60),
        (W_TRACER, W_GENJI, 60),
        (W_TRACER, W_SIGMA, -60),
        (W_SOJOURN, W_GENJI, -60),
        (W_SOJOURN, W_SIGMA, 60),
    ];
    for (attacker, defender, value) in edges {
        matchups
            .set(attacker, defender, value)
            .expect("fixture indices are in range");
        matchups
            .set(defender, attacker, -value)
            .expect("fixture indices are in range");
    }

    Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

#[test]
fn the_enemy_tank_outweighs_an_enemy_support_for_a_tank_player() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    // Reinhardt's edge is against the tank, D.Va's against the support. The two
    // edges are numerically identical, so only the weighting can break the tie.
    let mut draft = Draft::new();
    draft.add_enemy(W_SIGMA);
    draft.add_enemy(W_ANA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert_eq!(
        recs[0].hero, W_REINHARDT,
        "winning the tank duel beats winning the support matchup"
    );
    assert!(rank_of(&recs, W_REINHARDT) < rank_of(&recs, W_DVA));
}

#[test]
fn a_support_player_all_but_ignores_the_enemy_supports() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Support, ds.hero_count());

    // Ana's edge is against the enemy tank, Baptiste's against the enemy
    // support. Which healers they run barely changes a support's answer.
    let mut draft = Draft::new();
    draft.add_enemy(W_SIGMA);
    draft.add_enemy(W_ANA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert_eq!(recs[0].hero, W_ANA);
    assert!(rank_of(&recs, W_ANA) < rank_of(&recs, W_BAPTISTE));
}

#[test]
fn a_damage_player_answers_the_enemy_damage_before_the_enemy_tank() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Damage, ds.hero_count());

    // Tracer's edge is against the enemy damage pick, Sojourn's against the
    // enemy tank, and the two edges are numerically identical. The duel you are
    // in every fight is the one that should break the tie.
    let mut draft = Draft::new();
    draft.add_enemy(W_SIGMA);
    draft.add_enemy(W_GENJI);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert_eq!(recs[0].hero, W_TRACER);
    assert!(rank_of(&recs, W_TRACER) < rank_of(&recs, W_SOJOURN));

    // And it is the weighting doing it, not the fixture: under a plain average
    // the two mirror images tie exactly.
    let mut flat = UserContext::new(Role::Damage, ds.hero_count());
    flat.weights.enemy_roles = EnemyRoleWeights::uniform();
    let even = recommend(&ds, &draft, &flat).expect("scoring succeeds");
    let score_of = |hero| even.iter().find(|r| r.hero == hero).expect("ranked").score;
    assert!(
        (score_of(W_TRACER) - score_of(W_SOJOURN)).abs() < 1e-6,
        "mirrored candidates tie under a plain average, got {} and {}",
        score_of(W_TRACER),
        score_of(W_SOJOURN)
    );
}

#[test]
fn uniform_weights_reproduce_the_plain_average() {
    let ds = symmetric_fixture();
    let mut ctx = UserContext::new(Role::Tank, ds.hero_count());
    ctx.weights.enemy_roles = EnemyRoleWeights::uniform();

    let mut draft = Draft::new();
    draft.add_enemy(W_SIGMA);
    draft.add_enemy(W_ANA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    // Reinhardt and D.Va are mirror images of each other, so without role
    // weighting nothing separates them. (Sigma is both a candidate and an enemy
    // here — mirrors are allowed — and is genuinely behind, so it is not part of
    // the tie.) This is the guard that the change is opt-in arithmetic rather
    // than a new bias baked into the counter term.
    let score_of = |hero| recs.iter().find(|r| r.hero == hero).expect("ranked").score;
    assert!(
        (score_of(W_REINHARDT) - score_of(W_DVA)).abs() < 1e-6,
        "mirrored candidates tie under a plain average, got {} and {}",
        score_of(W_REINHARDT),
        score_of(W_DVA)
    );

    // ...whereas the defaults do separate them.
    let weighted = recommend(&ds, &draft, &UserContext::new(Role::Tank, ds.hero_count()))
        .expect("scoring succeeds");
    let weighted_score_of = |hero| {
        weighted
            .iter()
            .find(|r| r.hero == hero)
            .expect("ranked")
            .score
    };
    assert!(weighted_score_of(W_REINHARDT) > weighted_score_of(W_DVA));
}

#[test]
fn a_lone_first_pick_scores_the_same_weighted_or_not() {
    let ds = symmetric_fixture();

    let mut draft = Draft::new();
    draft.add_enemy(W_SIGMA);

    let weighted = UserContext::new(Role::Tank, ds.hero_count());
    let mut plain = UserContext::new(Role::Tank, ds.hero_count());
    plain.weights.enemy_roles = EnemyRoleWeights::uniform();

    let a = recommend(&ds, &draft, &weighted).expect("scoring succeeds");
    let b = recommend(&ds, &draft, &plain).expect("scoring succeeds");

    // With one enemy on the board the denominator is that enemy's own weight, so
    // it cancels: `w/w == 1`, and the answer from the first pick alone is
    // untouched whatever the role table says. Still true now that the denominator
    // counts every enemy rather than every *rated* enemy, because with a single
    // enemy those are the same set - and it is why the fix costs the
    // partial-input answer this app is built around nothing.
    for (weighted, plain) in a.iter().zip(b.iter()) {
        assert_eq!(weighted.hero, plain.hero);
        assert!((weighted.score - plain.score).abs() < 1e-6);
    }
}

// --- partial coverage ------------------------------------------------------
//
// The matrix has an unrated state and the counter term is a mean, so what a
// missing reading does to that mean is worth pinning down precisely. It adds
// nothing to the numerator - no term, no reason line, no invented dead even - and
// it still takes its share of the denominator, so a hero the sources have barely
// rated cannot read as more certain than one they have covered.
//
// The second half is newer than the first, and the tests below say which is
// which. These use their own fixture because the ones above are deliberately
// fully rated over the pairs they exercise.

const C_REINHARDT: HeroId = HeroId(0);
const C_DVA: HeroId = HeroId(1);
const C_PHARAH: HeroId = HeroId(2);
const C_MIZUKI: HeroId = HeroId(3);

/// Two tanks against a rated damage hero and a support nothing has an opinion
/// on — the shape the newest heroes actually arrive in.
fn sparse_fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("dva", "D.Va", Role::Tank),
        hero("pharah", "Pharah", Role::Damage),
        hero("mizuki", "Mizuki", Role::Support),
    ];
    let n = heroes.len();

    let mut matchups = Matrix::unrated(n);
    // Reinhardt/Pharah is rated from both sides, as the primary source gives it.
    matchups.set(C_REINHARDT, C_PHARAH, -80).expect("in range");
    matchups.set(C_PHARAH, C_REINHARDT, 80).expect("in range");
    // D.Va/Pharah is rated from one side only, as the secondary source gives it
    // where the primary has no card. Nothing rates anyone against Mizuki.
    matchups.set(C_DVA, C_PHARAH, 40).expect("in range");

    Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

fn score_of(recs: &[overwatch_core::Recommendation], hero: HeroId) -> f32 {
    recs.iter().find(|r| r.hero == hero).expect("ranked").score
}

#[test]
fn a_one_sided_matchup_counts_at_its_full_magnitude() {
    let ds = sparse_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut draft = Draft::new();
    draft.add_enemy(C_PHARAH);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    // D.Va has one reading of +40 and no reverse. Averaging it against a missing
    // direction would report +0.20; the reading itself is +0.40.
    assert!(
        (score_of(&recs, C_DVA) - 0.40).abs() < 1e-6,
        "expected the full reading, got {}",
        score_of(&recs, C_DVA)
    );
    // Reinhardt is rated both ways and still averages the two.
    assert!((score_of(&recs, C_REINHARDT) + 0.80).abs() < 1e-6);
}

/// An unrated enemy contributes no reading and still takes its share of the
/// denominator, which is two claims and both matter.
///
/// This test used to assert the opposite of its second half — that adding an
/// unrated enemy could not move a score at all — and that was the shape of a real
/// defect. Normalising over the rated enemies only is unbiased for the *mean* and
/// wrong for the *ranking*: it made a candidate rated against one of five enemies
/// divide by that one enemy's weight while a fully-rated candidate divided by all
/// five, so thin coverage read as conviction. See
/// `a_thinly_rated_hero_no_longer_outranks_a_fully_rated_one_on_coverage_alone`
/// for the consequence, which is what the change was made for.
///
/// What has not changed, and is the first half: the numerator. An unrated pair
/// still contributes no term and no reason line, so this is dilution rather than
/// a fabricated dead-even reading.
#[test]
fn an_unrated_enemy_takes_its_share_of_the_denominator_without_adding_a_reading() {
    let ds = sparse_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut rated_only = Draft::new();
    rated_only.add_enemy(C_PHARAH);

    let mut with_unknown = Draft::new();
    with_unknown.add_enemy(C_PHARAH);
    with_unknown.add_enemy(C_MIZUKI);

    let a = recommend(&ds, &rated_only, &ctx).expect("scoring succeeds");
    let b = recommend(&ds, &with_unknown, &ctx).expect("scoring succeeds");

    // Both enemies weigh 1.0 against a tank candidate, so adding the unrated one
    // exactly halves the share the rated one carries. Reinhardt goes -0.80 to
    // -0.40, D.Va +0.40 to +0.20.
    for hero in [C_REINHARDT, C_DVA] {
        assert!(
            (score_of(&a, hero) / 2.0 - score_of(&b, hero)).abs() < 1e-6,
            "expected {} to halve, got {}",
            score_of(&a, hero),
            score_of(&b, hero)
        );
    }

    // Uniformly, because the denominator does not depend on the candidate. An
    // unrated enemy costs every hero the same fraction, so it cannot reorder a
    // list on its own — it can only stop a thin candidate being flattered.
    let before = score_of(&a, C_DVA) - score_of(&a, C_REINHARDT);
    let after = score_of(&b, C_DVA) - score_of(&b, C_REINHARDT);
    assert!(
        (before / 2.0 - after).abs() < 1e-6,
        "the gap between the two candidates did not scale with the denominator"
    );
}

/// The defect the denominator change exists for, at its smallest.
///
/// Both candidates are read against the same two enemies. Reinhardt is rated
/// against both at +40; D.Va is rated against one at +60 and is unrated against
/// the other. The honest reading is that Reinhardt has the better case — it is
/// +40 against this team, where D.Va is +60 against half of it and unmeasured
/// against the rest.
///
/// Dividing by the rated enemies only gave D.Va 0.60/1.0 against Reinhardt's
/// 0.40, so the hero with half the evidence led the list. Dividing by the whole
/// team gives D.Va 0.60/2.0 = 0.30 and leaves Reinhardt at 0.40.
///
/// Measured on the committed data this was worth up to eleven places: Emre, on 24
/// rated rows of 52, reached the drawn top eight of the ban list 68 times in 300
/// random comps against 16 with this denominator.
#[test]
fn a_thinly_rated_hero_no_longer_outranks_a_fully_rated_one_on_coverage_alone() {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("dva", "D.Va", Role::Tank),
        hero("pharah", "Pharah", Role::Damage),
        hero("ashe", "Ashe", Role::Damage),
    ];
    let n = heroes.len();
    let (rein, dva, pharah, ashe) = (HeroId(0), HeroId(1), HeroId(2), HeroId(3));

    let mut matchups = Matrix::unrated(n);
    // Reinhardt: rated against both enemies, +40 each.
    matchups.set(rein, pharah, 40).expect("in range");
    matchups.set(pharah, rein, -40).expect("in range");
    matchups.set(rein, ashe, 40).expect("in range");
    matchups.set(ashe, rein, -40).expect("in range");
    // D.Va: a larger reading against one of them, and silence about the other.
    matchups.set(dva, pharah, 60).expect("in range");
    matchups.set(pharah, dva, -60).expect("in range");

    let ds = Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent");

    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let mut draft = Draft::new();
    draft.add_enemy(pharah);
    draft.add_enemy(ashe);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    assert!(
        (score_of(&recs, rein) - 0.40).abs() < 1e-6,
        "Reinhardt should read +0.40, got {}",
        score_of(&recs, rein)
    );
    assert!(
        (score_of(&recs, dva) - 0.30).abs() < 1e-6,
        "D.Va should read +0.30 - its one reading over the whole team - got {}",
        score_of(&recs, dva)
    );
    assert!(
        score_of(&recs, rein) > score_of(&recs, dva),
        "the fully-rated hero must lead the thinly-rated one"
    );
}

#[test]
fn an_unrated_enemy_produces_no_reasoning_and_no_threat() {
    let ds = sparse_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut draft = Draft::new();
    draft.add_enemy(C_MIZUKI);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let rein = recs.iter().find(|r| r.hero == C_REINHARDT).expect("ranked");

    // The old arithmetic gave this a term of exactly 0.0, which classified as
    // `BeatsEnemy` and rendered as "rated ahead of Mizuki" — a line naming a
    // reading, on a pair no source has rated at all.
    assert!(
        !rein
            .reasons
            .iter()
            .any(|r| r.kind == ReasonKind::BeatsEnemy(C_MIZUKI)),
        "claimed strength against a hero nothing has rated"
    );
    assert!(threats(&ds, &draft, &ctx, C_REINHARDT).is_empty());
}

#[test]
fn a_rated_even_matchup_stays_in_the_mean_but_claims_nothing() {
    let ds = sparse_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    // The mirror is the one matchup guaranteed to be a rated 0.0.
    let mut draft = Draft::new();
    draft.add_enemy(C_PHARAH);
    draft.add_enemy(C_DVA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let dva = recs.iter().find(|r| r.hero == C_DVA).expect("ranked");

    // It counts: without it D.Va would keep the full +0.40 Pharah reading.
    assert!(
        score_of(&recs, C_DVA) < 0.40,
        "the even matchup dropped out of the mean"
    );
    // It says nothing: a 0.0 is not an argument for the pick, and rendering it
    // as one puts "rated ahead of D.Va" under D.Va's own portrait, over a
    // reading that says neither hero is.
    assert!(
        !dva.reasons
            .iter()
            .any(|r| r.kind == ReasonKind::BeatsEnemy(C_DVA)),
        "a dead-even matchup rendered as a claim of strength"
    );
}

#[test]
fn mirroring_an_enemy_still_counts_as_an_even_matchup() {
    let ds = sparse_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    // No source rates D.Va against D.Va, but the duel is even by definition. If
    // the mirror dropped out of the mean, Pharah would carry all the weight and
    // D.Va would score its full +0.40 here too.
    let mut draft = Draft::new();
    draft.add_enemy(C_PHARAH);
    draft.add_enemy(C_DVA);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    let dva = score_of(&recs, C_DVA);
    assert!(
        dva < 0.40 && dva > 0.0,
        "the mirror should dilute the Pharah reading, got {dva}"
    );
}

// --- synergy ---------------------------------------------------------------

const Y_REINHARDT: HeroId = HeroId(0);
const Y_DVA: HeroId = HeroId(1);
const Y_LUCIO: HeroId = HeroId(2);
const Y_MIZUKI: HeroId = HeroId(3);

/// One ally the sources have paired a candidate with, and one nobody has.
///
/// The shape the real file has: duo data is published as a short top-N per
/// hero, so most of the grid is silence rather than a measured "these two do
/// nothing for each other".
fn synergy_fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("dva", "D.Va", Role::Tank),
        hero("lucio", "Lúcio", Role::Support),
        hero("mizuki", "Mizuki", Role::Support),
    ];
    let n = heroes.len();

    let mut synergy = Matrix::unrated(n);
    synergy.set(Y_REINHARDT, Y_LUCIO, 60).expect("in range");
    synergy.set(Y_LUCIO, Y_REINHARDT, 60).expect("in range");
    // Nothing pairs D.Va with anyone, and nothing pairs anyone with Mizuki.

    Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups: Matrix::unrated(n),
        synergy,
        map_affinity: Vec::new(),
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

#[test]
fn an_unrated_ally_is_left_out_of_the_synergy_mean_rather_than_counted_as_even() {
    let ds = synergy_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut rated_only = Draft::new();
    rated_only.add_ally(Y_LUCIO);

    let mut with_unknown = Draft::new();
    with_unknown.add_ally(Y_LUCIO);
    with_unknown.add_ally(Y_MIZUKI);

    let a = recommend(&ds, &rated_only, &ctx).expect("scoring succeeds");
    let b = recommend(&ds, &with_unknown, &ctx).expect("scoring succeeds");

    // Reinhardt/Lúcio is the only rated pair in the file. Dividing by every ally
    // instead of the rated ones would report it at half its strength the moment
    // a hero nobody has paired him with joins the team.
    assert!(
        score_of(&a, Y_REINHARDT) > 0.0,
        "the rated pair should score at all"
    );
    assert!(
        (score_of(&a, Y_REINHARDT) - score_of(&b, Y_REINHARDT)).abs() < 1e-6,
        "an unrated ally moved the score from {} to {}",
        score_of(&a, Y_REINHARDT),
        score_of(&b, Y_REINHARDT)
    );
    // And a candidate nobody has any reading for stays flat rather than being
    // dragged toward the middle of the field.
    assert!((score_of(&b, Y_DVA)).abs() < 1e-6);
}

// --- side ------------------------------------------------------------------

const S_REINHARDT: HeroId = HeroId(0);
const S_WINSTON: HeroId = HeroId(1);
const S_BASTION: HeroId = HeroId(2);
const S_KINGS_ROW: MapId = MapId(0);
const S_ILIOS: MapId = MapId(1);

/// A payload map and a control map, and two tanks that lean opposite ways.
fn side_fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("winston", "Winston", Role::Tank),
        hero("bastion", "Bastion", Role::Damage),
    ];
    let n = heroes.len();

    let maps = vec![
        GameMap {
            key: "kings-row".to_owned(),
            name: "King's Row".to_owned(),
            mode: GameMode::Hybrid,
            aliases: Vec::new(),
        },
        GameMap {
            key: "ilios".to_owned(),
            name: "Ilios".to_owned(),
            mode: GameMode::Control,
            aliases: Vec::new(),
        },
    ];

    let mut side_lean = vec![0i8; n];
    side_lean[S_WINSTON.index()] = 50; // attack
    side_lean[S_BASTION.index()] = -70; // defend

    Dataset::new(DatasetParts {
        heroes,
        maps,
        matchups: Matrix::unrated(n),
        synergy: Matrix::unrated(n),
        map_affinity: vec![0; 2 * n],
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean,
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

fn tank_score(ds: &Dataset, draft: &Draft, hero: HeroId) -> f32 {
    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    recommend(ds, draft, &ctx)
        .expect("scoring succeeds")
        .iter()
        .find(|r| r.hero == hero)
        .expect("ranked")
        .score
}

#[test]
fn the_side_term_flips_between_attack_and_defend() {
    let ds = side_fixture();

    let mut attacking = Draft::new();
    attacking.map = Some(S_KINGS_ROW);
    attacking.side = Some(Side::Attack);

    let mut defending = attacking.clone();
    defending.side = Some(Side::Defend);

    // One curated number per hero, read forwards on attack and backwards on
    // defence, so the two sides are exact opposites.
    let on_attack = tank_score(&ds, &attacking, S_WINSTON);
    let on_defence = tank_score(&ds, &defending, S_WINSTON);
    assert!(on_attack > 0.0, "Winston leans attack, got {on_attack}");
    assert!((on_attack + on_defence).abs() < 1e-6);

    // A hero with no lean is untouched by the choice.
    assert!(tank_score(&ds, &attacking, S_REINHARDT).abs() < 1e-6);
}

#[test]
fn a_symmetric_mode_ignores_the_side_entirely() {
    let ds = side_fixture();

    // Control has no attack half, so even a side left over from a previous map
    // must not contribute. Nothing in the UI can set this, but the sync socket
    // can deliver it.
    let mut draft = Draft::new();
    draft.map = Some(S_ILIOS);
    draft.side = Some(Side::Attack);

    assert!(tank_score(&ds, &draft, S_WINSTON).abs() < 1e-6);

    // And with no map at all there is nothing to have a side of.
    let mut mapless = Draft::new();
    mapless.side = Some(Side::Attack);
    assert!(tank_score(&ds, &mapless, S_WINSTON).abs() < 1e-6);
}

#[test]
fn the_side_shows_up_in_the_reasoning_only_when_it_says_something() {
    let ds = side_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut draft = Draft::new();
    draft.map = Some(S_KINGS_ROW);
    draft.side = Some(Side::Attack);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let reasons_for = |hero: HeroId| {
        recs.iter()
            .find(|r| r.hero == hero)
            .expect("ranked")
            .reasons
            .clone()
    };

    assert!(reasons_for(S_WINSTON)
        .iter()
        .any(|r| r.kind == ReasonKind::SideFit(Side::Attack)));
    assert!(
        reasons_for(S_REINHARDT).is_empty(),
        "a zero lean is not a reason"
    );
}

// --- the patch rung ----------------------------------------------------------

const P_REINHARDT: HeroId = HeroId(0);
const P_SIGMA: HeroId = HeroId(1);
const P_PHARAH: HeroId = HeroId(2);
const P_ANA: HeroId = HeroId(3);

/// A roster with strengths that differ, and one rated pair.
///
/// The other fixtures leave every strength at zero, which is right for them —
/// nothing above this point scores on it. The patch rung does, so it needs a
/// roster where "strongest" is a different answer from "worst matchup", or the
/// test cannot tell which one produced the order.
/// [`patch_fixture`] with the two heroes it ranks pulled apart by how often they
/// are picked: Ana leads on strength and is rare, Sigma trails and is everywhere.
///
/// Built so the two orderings disagree by construction, which is the only way to
/// tell which of them produced the list.
fn prevalence_fixture() -> Dataset {
    let ds = patch_fixture();
    let n = ds.hero_count();

    let mut prevalence = vec![[0i8; Rank::CHOICES.len()]; n];
    prevalence[P_ANA.index()] = [-100; Rank::CHOICES.len()];
    prevalence[P_SIGMA.index()] = [100; Rank::CHOICES.len()];

    let mut matchups = Matrix::unrated(n);
    matchups.set(P_PHARAH, P_REINHARDT, 100).expect("in range");
    matchups.set(P_REINHARDT, P_PHARAH, -100).expect("in range");

    Dataset::new(DatasetParts {
        heroes: (0..n)
            .map(|index| ds.hero(HeroId(index as u16)).expect("in range").clone())
            .collect(),
        maps: Vec::new(),
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![-20, 40, -60, 80],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence,
        win_rate: vec![Some(48.5), Some(52.0), Some(46.0), Some(54.0)],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

fn patch_fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("sigma", "Sigma", Role::Tank),
        hero("pharah", "Pharah", Role::Damage),
        hero("ana", "Ana", Role::Support),
    ];
    let n = heroes.len();

    // Pharah beats Reinhardt, and is the weakest hero on the patch. Ana is the
    // strongest and beats nobody. The two orders disagree by construction.
    let mut matchups = Matrix::unrated(n);
    matchups.set(P_PHARAH, P_REINHARDT, 100).expect("in range");
    matchups.set(P_REINHARDT, P_PHARAH, -100).expect("in range");

    Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![-20, 40, -60, 80],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![Some(48.5), Some(52.0), Some(46.0), Some(54.0)],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

// --- ban recommendations -----------------------------------------------------
//
// The ban phase runs before anyone picks, so these cases are mostly about who
// the answer is *for*. A ban is spent once for the whole team, so that is
// everyone on it — and the interesting part is how the answer sharpens as more
// of them say what they play.

fn locked(who: &str, is_me: bool, role: Role, hero: HeroId) -> Defended {
    Defended {
        who: who.to_owned(),
        is_me,
        is_typed: false,
        role,
        knowledge: Knowledge::Locked(hero),
        heroes: vec![hero],
    }
}

fn pooled(who: &str, is_me: bool, role: Role, heroes: Vec<HeroId>) -> Defended {
    Defended {
        who: who.to_owned(),
        is_me,
        is_typed: false,
        role,
        knowledge: Knowledge::Pool,
        heroes,
    }
}

/// Somebody who has said only what they queued as, which resolves to the whole
/// role — the state everybody is in before they touch the pool board.
fn unknown(ds: &Dataset, who: &str, is_me: bool, role: Role) -> Defended {
    Defended {
        who: who.to_owned(),
        is_me,
        is_typed: false,
        role,
        knowledge: Knowledge::Unknown,
        heroes: ds.heroes_in_role(role).collect(),
    }
}

fn team(members: Vec<Defended>) -> DefendedTeam {
    DefendedTeam { members }
}

/// Drafting alone, having marked a pool. The one-member case, which is what the
/// solo screen actually passes.
/// Two people who have said only their role, which is the state the patch rung
/// answers for.
fn quiet_team(ds: &Dataset) -> DefendedTeam {
    team(vec![
        unknown(ds, "me", true, Role::Tank),
        unknown(ds, "mika", false, Role::Support),
    ])
}

fn solo_pool(heroes: Vec<HeroId>) -> DefendedTeam {
    team(vec![pooled("me", true, Role::Tank, heroes)])
}

fn ban_rank_of(board: &BanBoard, hero: HeroId) -> Option<usize> {
    board.candidates.iter().position(|c| c.hero == hero)
}

fn ban_score_of(board: &BanBoard, hero: HeroId) -> Option<f32> {
    board
        .candidates
        .iter()
        .find(|c| c.hero == hero)
        .map(|c| c.score)
}

#[test]
fn a_locked_pick_is_banned_for_by_its_own_matchups() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.locked = Some(REINHARDT);

    let board = ban_recommendations(
        &ds,
        &draft,
        &ctx,
        &team(vec![locked("me", true, Role::Tank, REINHARDT)]),
    );

    assert_eq!(
        board.subject,
        BanSubject::One {
            who: "me".to_owned(),
            is_me: true,
            locked: true,
            heroes: 1,
        }
    );
    // Pharah is a 9/10 for Reinhardt and Widowmaker a 7/10, so the order is the
    // order of the two matchups and nothing else.
    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![PHARAH, WIDOWMAKER],
        "the two heroes rated against Reinhardt, worst first"
    );
    // Every other hero in the fixture is unrated against him. Ranking them at a
    // flat zero would put four heroes with no argument behind them on a list
    // whose whole content is the argument.
    assert!((board.candidates[0].severity - 1.0).abs() < 1e-6);
    assert_eq!(board.candidates[0].worst, Some(REINHARDT));
    assert_eq!(
        board.candidates[0].worst_owner, None,
        "your own hero is not credited back to you"
    );
    assert!(!board.candidates[0].text.is_empty(), "the scraped sentence");
}

#[test]
fn an_unpicked_draft_bans_for_the_average_of_your_pool() {
    let ds = fixture();
    let ctx = tank_context(&ds);
    // Reinhardt is helpless into both; D.Va eats both. Widowmaker is the
    // interesting one: she beats Reinhardt outright (+0.5) and so tops a list
    // built from the worst case, but D.Va beats her by twice that, so across
    // the pool she is not a problem at all.
    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &solo_pool(vec![REINHARDT, DVA]));

    assert_eq!(
        board.subject,
        BanSubject::One {
            who: "me".to_owned(),
            is_me: true,
            locked: false,
            heroes: 2,
        }
    );
    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![PHARAH],
    );
    // (+1.0 against Reinhardt, -0.75 against D.Va) / 2.
    assert!((board.candidates[0].severity - 0.125).abs() < 1e-6);
    assert_eq!(
        board.candidates[0].worst,
        Some(REINHARDT),
        "the average decides the ranking; the worst case names who it is for"
    );
}

#[test]
fn heroes_your_side_of_the_draft_beats_are_not_worth_banning() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.locked = Some(DVA);

    let board = ban_recommendations(
        &ds,
        &draft,
        &ctx,
        &team(vec![locked("me", true, Role::Tank, DVA)]),
    );

    // D.Va beats everyone the fixture rates her against, so there is no ban to
    // spend — and saying so with an empty list is the honest answer.
    assert!(board.candidates.is_empty());
}

#[test]
fn a_hero_already_in_the_draft_is_not_a_ban() {
    let ds = fixture();
    let ctx = tank_context(&ds);
    let mine = solo_pool(vec![REINHARDT, SIGMA]);

    let open = ban_recommendations(&ds, &Draft::new(), &ctx, &mine);
    assert!(ban_rank_of(&open, PHARAH).is_some(), "worth banning");

    // Picked, so the ban phase is over for her either way.
    let mut drafted = Draft::new();
    drafted.add_enemy(PHARAH);
    assert!(ban_rank_of(&ban_recommendations(&ds, &drafted, &ctx, &mine), PHARAH).is_none());

    // On your own team, so banning her is banning yourself.
    let mut allied = Draft::new();
    allied.add_ally(PHARAH);
    assert!(ban_rank_of(&ban_recommendations(&ds, &allied, &ctx, &mine), PHARAH).is_none());
}

#[test]
fn the_enemy_tank_is_worth_more_to_ban_for_a_tank_player() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut draft = Draft::new();
    draft.locked = Some(W_SIGMA);

    let board = ban_recommendations(
        &ds,
        &draft,
        &ctx,
        &team(vec![locked("me", true, Role::Tank, W_SIGMA)]),
    );

    // Reinhardt, Ana and Sojourn all beat Sigma by exactly 60, so the matchups
    // cannot break the tie — only the role weighting can, and for a tank player
    // it is the enemy tank that decides the game.
    let severities: Vec<f32> = board.candidates.iter().map(|c| c.severity).collect();
    assert!(
        severities.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-6),
        "every candidate wins by the same margin: {severities:?}"
    );
    assert_eq!(board.candidates[0].hero, W_REINHARDT);
    assert!(ban_rank_of(&board, W_REINHARDT) < ban_rank_of(&board, W_ANA));
}

// --- what the team adds ------------------------------------------------------
//
// The ladder the panel climbs: nobody has said anything, then somebody marks a
// pool, then somebody else does, then people lock in. Each rung has to move the
// answer, or the panel is claiming to use information it is throwing away.

/// The Torbjörn case, in miniature. Ana is the strongest hero here and almost
/// nobody picks her; Sigma is weaker and everywhere. A ban is spent on a hero the
/// enemy might actually take, so the list has to weigh that.
#[test]
fn a_rarely_picked_hero_falls_down_the_ban_list() {
    let ds = prevalence_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &quiet_team(&ds));

    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![P_SIGMA, P_ANA],
        "strength alone would have put Ana first"
    );
    // 0.40 x 1.40 against 0.80 x 0.60.
    assert!((ban_score_of(&board, P_SIGMA).expect("listed") - 0.56).abs() < 1e-6);
    assert!((ban_score_of(&board, P_ANA).expect("listed") - 0.48).abs() < 1e-6);
}

/// Membership is decided by the argument and only the order by the prior. The
/// discount is applied after the `score <= 0.0` gate and is strictly positive, so
/// there is no weight at which it can put a hero on this list or take one off.
#[test]
fn prevalence_reorders_the_ban_list_without_changing_who_is_on_it() {
    let ds = prevalence_fixture();
    let mut ctx = UserContext::new(Role::Tank, ds.hero_count());

    let discounted = ban_recommendations(&ds, &Draft::new(), &ctx, &quiet_team(&ds));
    ctx.weights.prevalence = 0.0;
    let plain = ban_recommendations(&ds, &Draft::new(), &ctx, &quiet_team(&ds));

    let membership = |board: &BanBoard| {
        let mut heroes: Vec<HeroId> = board.candidates.iter().map(|c| c.hero).collect();
        heroes.sort_by_key(|hero| hero.index());
        heroes
    };
    assert_eq!(membership(&discounted), membership(&plain));
    assert_ne!(
        discounted.candidates[0].hero, plain.candidates[0].hero,
        "and the order really did move, or this test proves nothing"
    );

    // Zeroing the weight makes the factor exactly 1.0, which is patch strength
    // read straight — the behaviour everybody had before this term existed.
    assert!((ban_score_of(&plain, P_ANA).expect("listed") - 0.80).abs() < 1e-6);
}

/// `severity` is the reading and `score` is the reading times a prior. The panel
/// sorts on and displays the second; the first has to survive un-multiplied, or
/// nothing on screen can say what the matchup alone was.
#[test]
fn the_discount_reaches_the_score_and_leaves_the_severity_alone() {
    let ds = prevalence_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &quiet_team(&ds));
    let ana = board
        .candidates
        .iter()
        .find(|c| c.hero == P_ANA)
        .expect("listed");

    assert!((ana.severity - 0.80).abs() < 1e-6, "the raw reading");
    assert!((ana.score - 0.48).abs() < 1e-6, "and the discounted one");
    assert_eq!(
        ana.prevalence, -100,
        "carried, so the panel says the same thing"
    );
}

/// Prevalence reaches a hero the enemy has not picked yet, and nothing else. The
/// pick list is about you, and the threat board is about heroes already on the
/// board — where the probability of turning up is 1 and a prior about it would be
/// arithmetic on an event that has already happened.
#[test]
fn prevalence_never_reaches_the_pick_list_or_the_threat_board() {
    let ds = prevalence_fixture();
    let mut draft = Draft::new();
    draft.enemies.push(P_PHARAH);

    let mut discounted = UserContext::new(Role::Tank, ds.hero_count());
    discounted.weights.prevalence = 0.90;
    let mut plain = UserContext::new(Role::Tank, ds.hero_count());
    plain.weights.prevalence = 0.0;

    let scores = |ctx: &UserContext| {
        recommend(&ds, &draft, ctx)
            .expect("scoring succeeds")
            .into_iter()
            .map(|rec| (rec.hero, rec.score))
            .collect::<Vec<_>>()
    };
    assert_eq!(scores(&discounted), scores(&plain));

    let severities = |ctx: &UserContext| {
        threats(&ds, &draft, ctx, P_REINHARDT)
            .into_iter()
            .map(|threat| threat.severity)
            .collect::<Vec<_>>()
    };
    assert_eq!(severities(&discounted), severities(&plain));
}

/// Nobody has said anything, so there is no team to answer about. Ranked by
/// patch strength instead — a role-wide matchup average is nearly flat, and a
/// flat list presented as an answer is worse than one that says what it is.
#[test]
fn with_nobody_known_the_list_is_the_patch() {
    let ds = patch_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(
        &ds,
        &Draft::new(),
        &ctx,
        &team(vec![
            unknown(&ds, "me", true, Role::Tank),
            unknown(&ds, "mika", false, Role::Support),
        ]),
    );

    assert_eq!(board.subject, BanSubject::Patch);
    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![P_ANA, P_SIGMA],
        "strongest first, and only the above-average half"
    );
    assert!((board.candidates[0].score - 0.80).abs() < 1e-6);
    assert_eq!(
        board.candidates[0].worst, None,
        "no pair produced this, so naming one would invent a claim"
    );
}

/// The moment anybody says anything, strength stops being consulted at all.
/// "Strong right now" and "bad for us" are two different arguments for a ban,
/// and a number that mixes them can only be read as neither.
#[test]
fn one_pool_anywhere_leaves_the_patch_rung_for_good() {
    let ds = patch_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(
        &ds,
        &Draft::new(),
        &ctx,
        &team(vec![
            pooled("me", true, Role::Tank, vec![P_REINHARDT]),
            unknown(&ds, "mika", false, Role::Support),
        ]),
    );

    assert!(!matches!(board.subject, BanSubject::Patch));
    // Ana is the strongest hero in this fixture by a distance and beats nobody
    // on the team, so a list still reading strength would have her top. It is
    // Pharah, who is the only hero rated against anything the team plays.
    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![P_PHARAH],
    );
}

/// The reported bug, as a test: adding an ally used to remove one candidate and
/// change nothing else. A ban is spent for the team, so a teammate arriving has
/// to be able to change the whole answer.
#[test]
fn a_teammate_locking_in_changes_what_is_worth_banning() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    // Reinhardt beats Sigma and loses to Ana; D.Va is the exact mirror. Across
    // the two of them every candidate averages out to dead even, so there is
    // nothing here to spend a ban on.
    let alone = team(vec![pooled(
        "me",
        true,
        Role::Tank,
        vec![W_REINHARDT, W_DVA],
    )]);
    let solo = ban_recommendations(&ds, &Draft::new(), &ctx, &alone);
    assert!(
        solo.candidates.is_empty(),
        "nothing beats the pool on average: {:?}",
        solo.candidates
    );

    // A support teammate locks Ana, who loses to Baptiste by 60. That is a real
    // argument for a ban and it was not on the board a moment ago — nobody in
    // the pool is rated against Baptiste at all.
    let mut with_mika = alone.clone();
    with_mika
        .members
        .push(locked("mika", false, Role::Support, W_ANA));
    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &with_mika);

    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![W_BAPTISTE],
    );
    assert_eq!(board.candidates[0].worst, Some(W_ANA));
    assert_eq!(
        board.candidates[0].worst_owner.as_deref(),
        Some("mika"),
        "whose hero takes the worst of it, since it is not yours"
    );
}

/// The ban list's half of the coverage fix, which is where the defect did the most
/// damage: `score <= 0.0` discards the unfavourable tail and the panel draws only
/// the first eight, so a thin candidate's wider spread reached the user in one
/// direction only.
///
/// Two members, and a candidate rated against just one of them. Dividing by the
/// contributing members alone gave it that member's reading at full strength —
/// `certainty * w * danger / certainty` — so half the evidence produced the whole
/// score, and a hero rated against nobody but one teammate could lead a list over
/// one measured against the entire team. Dividing by both halves it.
///
/// Baptiste beats Ana by 60 and is unrated against everything in my own pool, so
/// it is the one candidate this fixture can express the case with.
#[test]
fn a_candidate_rated_against_one_member_of_two_scores_over_the_whole_team() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut both = team(vec![pooled(
        "me",
        true,
        Role::Tank,
        vec![W_REINHARDT, W_DVA],
    )]);
    both.members
        .push(locked("mika", false, Role::Support, W_ANA));

    let with_two = ban_recommendations(&ds, &Draft::new(), &ctx, &both);
    let baptiste = with_two
        .candidates
        .iter()
        .find(|c| c.hero == W_BAPTISTE)
        .expect("Baptiste is the one candidate with an argument here");

    // Mika alone: Ana is a support member, so the ban weight for a support
    // candidate is `0.6 / 0.8333 = 0.72`, and the reading is +0.60 of danger.
    // Over both members the score is `1.0 * 0.72 * 0.60 / 2.0`; over the
    // contributing member only it was `/ 1.0`, twice as large on the same single
    // reading, which is what this test is about.
    let expected = ctx
        .weights
        .enemy_roles
        .ban_weight(Role::Support, Role::Support)
        * 0.60
        / 2.0;
    assert!(
        (baptiste.severity - 0.60 / 2.0).abs() < 1e-6,
        "severity should be the danger spread over the whole team, got {}",
        baptiste.severity
    );
    // `symmetric_fixture` leaves prevalence at zero everywhere, so the ordering
    // multiplier is exactly 1.0 and `score` is the weighted mean unmodified.
    assert_eq!(baptiste.prevalence, 0, "the fixture rates no pick rates");
    assert!(
        (baptiste.score - expected).abs() < 1e-6,
        "score should be {expected}, got {}",
        baptiste.score
    );

    // And the sorted column still agrees with the shown one: both took the same
    // denominator, which is what `BanCandidate::severity` promises.
    assert!(
        baptiste.severity > 0.0 && baptiste.score > 0.0,
        "a real argument, just a smaller one"
    );
}

/// The defect the ban weight exists for: the list has to rank by how much a
/// candidate beats your team, not by which role it happens to be.
///
/// One tank teammate, two candidates. The support beats them by 60, the tank by
/// only 40 — so the support is plainly the bigger problem. Reading the table's raw
/// cell put the tank first anyway, because `[tank][tank]` is 2.2 against
/// `[tank][support]` at 1.0: `2.2 x 0.40 = 0.88` against `1.0 x 0.60 = 0.60`. That
/// is the whole bug, and it is not a close call — a 50% larger matchup lost to a
/// column that is twice the size.
///
/// The ban weight divides each column by its own mean, so the comparison becomes
/// `1.320 x 0.40 = 0.528` against `1.200 x 0.60 = 0.72` and the bigger problem
/// wins. Note what is *not* claimed: the tank column still outweighs the support
/// column for a tank teammate, 1.320 against 1.200. It just no longer outweighs it
/// by enough to overturn the matchups.
#[test]
fn a_support_that_is_the_bigger_problem_outranks_a_tank_that_is_not() {
    let heroes = vec![
        hero("winston", "Winston", Role::Tank),
        hero("sigma", "Sigma", Role::Tank),
        hero("ana", "Ana", Role::Support),
    ];
    let n = heroes.len();
    let (winston, sigma, ana) = (HeroId(0), HeroId(1), HeroId(2));

    let mut matchups = Matrix::unrated(n);
    for (attacker, defender, value) in [(sigma, winston, 40), (ana, winston, 60)] {
        matchups.set(attacker, defender, value).expect("in range");
        matchups.set(defender, attacker, -value).expect("in range");
    }

    let ds = Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups,
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent");

    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let board = ban_recommendations(
        &ds,
        &Draft::new(),
        &ctx,
        &team(vec![locked("me", true, Role::Tank, winston)]),
    );

    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![ana, sigma],
        "the support beats this team by more, so it is the better ban"
    );

    // And the column now agrees with itself. `severity` is the unweighted mean
    // danger and is what the panel prints, so a list sorted by `score` that put
    // the smaller `severity` on top was visibly contradicting the number beside
    // it.
    let severities: Vec<f32> = board.candidates.iter().map(|c| c.severity).collect();
    assert!(
        (severities[0] - 0.60).abs() < 1e-6 && (severities[1] - 0.40).abs() < 1e-6,
        "expected 0.60 then 0.40, got {severities:?}"
    );
}

/// Same again for a pool rather than a lock — the rung before anybody has
/// picked, which is where a ban actually lands.
#[test]
fn a_teammates_pool_changes_what_is_worth_banning() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let alone = team(vec![pooled("me", true, Role::Tank, vec![W_REINHARDT])]);
    let solo = ban_recommendations(&ds, &Draft::new(), &ctx, &alone);
    assert_eq!(
        solo.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![W_ANA],
        "Ana is the only hero that beats Reinhardt"
    );

    // Mika plays Ana. She stops being a candidate — banning her would cost Mika
    // the pick — and what beats *her* takes over the list instead: D.Va, who
    // nothing in the draft had a reason to care about a moment ago, then
    // Baptiste. D.Va leads because Mika reads an enemy tank through the 1.6 a
    // support pays one, while Baptiste is a support against a support at 0.6.
    let mut with_mika = alone.clone();
    with_mika
        .members
        .push(pooled("mika", false, Role::Support, vec![W_ANA]));
    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &with_mika);

    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![W_DVA, W_BAPTISTE],
    );
    assert_eq!(
        board.subject,
        BanSubject::Team {
            known: 2,
            locked: 0
        }
    );
}

/// A ban takes the hero off the table for everyone, so recommending one the team
/// plays is recommending they lose a pick to deny it. The pool highlights rather
/// than restricts everywhere else in this app; here it is the cost.
#[test]
fn nothing_the_team_plays_is_ever_a_candidate() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(
        &ds,
        &Draft::new(),
        &ctx,
        &team(vec![
            pooled("me", true, Role::Tank, vec![W_REINHARDT, W_DVA]),
            pooled("mika", false, Role::Support, vec![W_ANA]),
            locked("sam", false, Role::Damage, W_TRACER),
        ]),
    );

    for hero in [W_REINHARDT, W_DVA, W_ANA, W_TRACER] {
        assert!(
            ban_rank_of(&board, hero).is_none(),
            "{hero:?} is one of ours"
        );
    }
}

/// Somebody who has only queued still holds a slot, and a candidate that beats
/// their whole role matters more than one that does not — but they have made no
/// claim about any hero, so they must not out-vote the people who have.
#[test]
fn a_member_who_has_said_nothing_dilutes_without_deciding() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let alone = team(vec![pooled("me", true, Role::Tank, vec![W_REINHARDT])]);
    let mut with_quiet = alone.clone();
    with_quiet
        .members
        .push(unknown(&ds, "mika", false, Role::Support));

    let solo = ban_recommendations(&ds, &Draft::new(), &ctx, &alone);
    let joined = ban_recommendations(&ds, &Draft::new(), &ctx, &with_quiet);

    assert_eq!(
        solo.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        joined.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        "a quiet teammate cannot reorder the answer"
    );
    let before = ban_score_of(&solo, W_ANA).expect("ranked alone");
    let after = ban_score_of(&joined, W_ANA).expect("still ranked");
    assert!(
        after < before,
        "but they do dilute it: {before} then {after}"
    );
    // Their own worst case is a hero of a role nobody claimed, so it must not be
    // what the row names.
    assert_eq!(joined.candidates[0].worst, Some(W_REINHARDT));
    assert_eq!(joined.candidates[0].worst_owner, None);
}

/// The other half of the same rule, and the sharper one.
///
/// D.Va beats Ana, and Ana is only on the board because somebody queued support.
/// Nothing in the pool is rated against D.Va at all — so putting her on the list
/// would rank a hero off a role nobody claimed, above one the pool is actually
/// measured against. Worse, the certainty discount cannot stop it: the mean
/// divides by the certainty of the members that contributed, so a candidate
/// carried by one quiet member divides its 0.25 straight back out.
#[test]
fn a_quiet_member_alone_cannot_put_a_hero_on_the_list() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(
        &ds,
        &Draft::new(),
        &ctx,
        &team(vec![
            pooled("me", true, Role::Tank, vec![W_REINHARDT]),
            unknown(&ds, "mika", false, Role::Support),
        ]),
    );

    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![W_ANA],
        "only the hero the pool is rated against"
    );
    assert!(
        board.candidates.iter().all(|c| c.worst.is_some()),
        "every row on a team rung names whose hero takes the worst of it"
    );
}

/// The compatibility pin. Drafting alone is the one-member case rather than a
/// separate path, so the answer a solo player gets must be the one the old
/// per-person scorer gave: the pool's mean, through their own role's weights.
#[test]
fn drafting_alone_still_scores_the_pool_through_your_own_role() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let board = ban_recommendations(
        &ds,
        &Draft::new(),
        &ctx,
        &team(vec![pooled("me", true, Role::Tank, vec![W_REINHARDT])]),
    );

    // Ana beats Reinhardt by 60. One member, so the certainty average is a
    // no-op and the score is exactly `severity * ban_weight[tank][support]`.
    //
    // The *ban* weight, not the raw cell: the ban list is the one caller that
    // ranks candidates of different roles against each other, so it reads the
    // table rescaled to make its columns comparable. Read from the context
    // rather than written out, so this keeps holding if the table is retuned.
    let ana = board.candidates.first().expect("Ana is ranked");
    assert_eq!(ana.hero, W_ANA);
    assert!((ana.severity - 0.60).abs() < 1e-6);
    let weight = ctx
        .weights
        .enemy_roles
        .ban_weight(Role::Tank, Role::Support);
    assert!((ana.score - 0.60 * weight).abs() < 1e-6);
}

/// Averaging the role weight across the team is what makes the ban list
/// format-aware without a second weight table: a 6v6 roster holds two tank
/// slots, so the enemy tank is read through the tank row twice.
#[test]
fn a_second_tank_on_the_team_raises_what_an_enemy_tank_is_worth() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    // Ana beats Sigma, and so does Reinhardt, so Sigma is nobody's problem —
    // Baptiste is the one who loses to him. One tank alongside, then two.
    let base = vec![
        pooled("me", true, Role::Support, vec![W_BAPTISTE]),
        pooled("mika", false, Role::Tank, vec![W_DVA]),
    ];
    let five = ban_recommendations(&ds, &Draft::new(), &ctx, &team(base.clone()));

    let mut six = base;
    six.push(pooled("sam", false, Role::Tank, vec![W_DVA]));
    let six = ban_recommendations(&ds, &Draft::new(), &ctx, &team(six));

    let before = ban_score_of(&five, W_SIGMA).expect("ranked");
    let after = ban_score_of(&six, W_SIGMA).expect("still ranked");
    assert!(
        after > before,
        "a second tank slot reads the enemy tank through the tank row twice: {before} then {after}"
    );
}

// --- team shape ------------------------------------------------------------

const T_WINSTON: HeroId = HeroId(0);
const T_REINHARDT: HeroId = HeroId(1);
const T_SIGMA: HeroId = HeroId(2);
const T_WIDOWMAKER: HeroId = HeroId(3);
const T_TRACER: HeroId = HeroId(4);
const T_UNREAD: HeroId = HeroId(5);

/// Three tanks, one per axis, and a damage roster to build enemy comps out of.
///
/// Every matchup is unrated and every other term zero, so the only thing that
/// can move a score here is the shape term. That is the point: the claim under
/// test is that team shape ranks candidates *on its own*, and a fixture with
/// live matchup data could pass it for the wrong reason.
fn shape_fixture() -> Dataset {
    let heroes = vec![
        hero("winston", "Winston", Role::Tank),
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("sigma", "Sigma", Role::Tank),
        hero("widowmaker", "Widowmaker", Role::Damage),
        hero("tracer", "Tracer", Role::Damage),
        hero("unread", "Unread", Role::Damage),
    ];
    let n = heroes.len();

    Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups: Matrix::unrated(n),
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        base_strength: vec![0; n],
        rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![
            [95, 0, 0], // winston: dive
            [0, 0, 95], // reinhardt: brawl
            [0, 95, 0], // sigma: poke
            [0, 95, 0], // widowmaker: poke
            [95, 0, 0], // tracer: dive
            [0, 0, 0],  // unread: nobody has curated this one
        ],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

/// The whole argument for the term, on a full poke enemy team: the dive answer
/// ranks above the mirror, and the brawl one ranks below it.
#[test]
fn a_tank_that_answers_the_enemy_shape_outranks_one_that_walks_into_it() {
    let ds = shape_fixture();
    let mut draft = Draft::new();
    draft.add_enemy(T_SIGMA);
    draft.add_enemy(T_WIDOWMAKER);

    let winston = tank_score(&ds, &draft, T_WINSTON);
    let sigma = tank_score(&ds, &draft, T_SIGMA);
    let reinhardt = tank_score(&ds, &draft, T_REINHARDT);

    assert!(winston > 0.0, "dive answers poke, got {winston}");
    assert!(reinhardt < 0.0, "brawl walks into poke, got {reinhardt}");
    assert!(
        sigma.abs() < 1e-6,
        "the mirror is a rated dead even, got {sigma}"
    );
    assert!(winston > sigma && sigma > reinhardt);
}

/// The triangle, asserted end to end through the scorer rather than on the
/// table it is built from. Each enemy comp must promote exactly one of the
/// three tanks and demote exactly one.
#[test]
fn each_enemy_shape_promotes_the_answer_to_it() {
    let ds = shape_fixture();

    // Two picks a side rather than one, so each case is a comp rather than a
    // lone hero: the mean has to survive being taken over more than one body.
    //
    // (enemy comp, the tank that beats it, the tank that loses to it)
    for (enemies, answer, victim) in [
        // poke: dive beats it, brawl walks into it
        ([T_SIGMA, T_WIDOWMAKER], T_WINSTON, T_REINHARDT),
        // dive: brawl beats it, poke walks into it
        ([T_WINSTON, T_TRACER], T_REINHARDT, T_SIGMA),
        // brawl: poke beats it, dive walks into it
        ([T_REINHARDT, T_REINHARDT], T_SIGMA, T_WINSTON),
    ] {
        let mut draft = Draft::new();
        for enemy in enemies {
            draft.add_enemy(enemy);
        }

        let good = tank_score(&ds, &draft, answer);
        let bad = tank_score(&ds, &draft, victim);
        assert!(
            good > 0.0 && bad < 0.0,
            "against {enemies:?} the answer scored {good} and the victim {bad}"
        );
    }
}

/// A roster nobody has curated must score exactly as it did before this term
/// existed. Silence has to cost nothing, or the term is a tax on every hero the
/// file has not reached yet.
#[test]
fn an_unread_enemy_team_leaves_every_score_untouched() {
    let ds = shape_fixture();
    let mut draft = Draft::new();
    draft.add_enemy(T_UNREAD);

    for tank in [T_WINSTON, T_REINHARDT, T_SIGMA] {
        let score = tank_score(&ds, &draft, tank);
        assert!(score.abs() < 1e-6, "{tank:?} moved to {score}");
    }
}

/// And an enemy team with no committed shape is the same silence: the two picks
/// cancel, so there is nothing for a candidate to lean into either way.
#[test]
fn a_mixed_enemy_team_ranks_every_shape_the_same() {
    let ds = shape_fixture();
    let mut draft = Draft::new();
    draft.add_enemy(T_WINSTON); // dive
    draft.add_enemy(T_REINHARDT); // brawl
    draft.add_enemy(T_SIGMA); // poke

    for tank in [T_WINSTON, T_REINHARDT, T_SIGMA] {
        let score = tank_score(&ds, &draft, tank);
        assert!(score.abs() < 1e-6, "{tank:?} moved to {score}");
    }
}

/// The term explains itself, and names the enemy's shape rather than the
/// candidate's — the portrait beside the line already says what the candidate
/// is. A zero-valued term must stay silent, the way every other term does.
#[test]
fn the_shape_shows_up_in_the_reasoning_only_when_it_says_something() {
    let ds = shape_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut draft = Draft::new();
    draft.add_enemy(T_SIGMA);
    draft.add_enemy(T_WIDOWMAKER);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let reasons_for = |hero: HeroId| {
        recs.iter()
            .find(|r| r.hero == hero)
            .expect("ranked")
            .reasons
            .clone()
    };

    assert!(reasons_for(T_WINSTON)
        .iter()
        .any(|r| r.kind == ReasonKind::CountersShape(Archetype::Poke)));
    assert!(reasons_for(T_REINHARDT)
        .iter()
        .any(|r| r.kind == ReasonKind::LosesToShape(Archetype::Poke)));

    // The mirror computes to exactly zero, and a zero is never dressed up as a
    // claim about anything.
    assert!(!reasons_for(T_SIGMA).iter().any(|r| matches!(
        r.kind,
        ReasonKind::CountersShape(_) | ReasonKind::LosesToShape(_)
    )));
}

/// The weight is the lever, and it has to be able to switch the term off
/// entirely — which is also what an old stored profile is asserting when it
/// carries a weight this field did not exist for.
#[test]
fn zeroing_the_shape_weight_removes_the_term() {
    let ds = shape_fixture();
    let mut ctx = UserContext::new(Role::Tank, ds.hero_count());
    ctx.weights.shape = 0.0;

    let mut draft = Draft::new();
    draft.add_enemy(T_SIGMA);
    draft.add_enemy(T_WIDOWMAKER);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    for rec in &recs {
        assert!(
            rec.score.abs() < 1e-6,
            "{:?} scored {}",
            rec.hero,
            rec.score
        );
    }
}

// --- rank ------------------------------------------------------------------

const R_ANCHOR: HeroId = HeroId(0);
const R_CLIMBER: HeroId = HeroId(1);
const R_SMURF: HeroId = HeroId(2);

/// Three tanks whose patch strength says one thing and whose rank curve says the
/// opposite, so a test can tell which one the scorer read.
///
/// Everything else is zeroed for the same reason `shape_fixture` zeroes it: the
/// claim under test is that rank ranks candidates on its own, and a fixture with
/// live matchup data could pass it for the wrong reason.
///
/// The curves are deliberately not parallel. `climber` is worst at the bottom of
/// the ladder and best at the top, `smurf` the reverse, and `anchor` never moves
/// — which is the case the feature must leave alone.
fn rank_fixture() -> Dataset {
    let heroes = vec![
        hero("anchor", "Anchor", Role::Tank),
        hero("climber", "Climber", Role::Tank),
        hero("smurf", "Smurf", Role::Tank),
    ];
    let n = heroes.len();

    Dataset::new(DatasetParts {
        heroes,
        maps: Vec::new(),
        matchups: Matrix::unrated(n),
        synergy: Matrix::unrated(n),
        map_affinity: Vec::new(),
        // Across the ladder as a whole, this is the order. The spread is 30
        // rather than the 60 the real data reaches, so that a shift inside the
        // -100..=100 scale can actually overturn it — the point of the fixture
        // is the arithmetic, and a gap no legal shift can close would only test
        // the clamp.
        base_strength: vec![30, 0, -30],
        rank_shift: vec![
            // anchor: the same everywhere. Picking a rung must not move it.
            [0; Rank::DIVISIONS.len()],
            // climber: nothing at Bronze, enough by Grandmaster to lead.
            [0, 10, 20, 40, 60, 80, 90, 100],
            // smurf: the mirror.
            [100, 90, 80, 60, 40, 20, 10, 0],
        ],
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        side_note: vec![String::new(); n],
        shape: vec![[0; 3]; n],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

fn ranked_context(ds: &Dataset, rank: Rank) -> UserContext {
    let mut ctx = UserContext::new(Role::Tank, ds.hero_count());
    ctx.rank = rank;
    ctx
}

/// The promise the whole feature rests on: somebody who never opens the picker
/// gets the app they had. Value for value, not merely in the same order.
#[test]
fn an_unset_rank_scores_exactly_as_the_all_ranks_column_does() {
    let ds = rank_fixture();
    let draft = Draft::new();

    // A context built the way every caller builds one, against a dataset where
    // every hero has a large rank shift waiting to be applied.
    let recs = recommend(&ds, &draft, &UserContext::new(Role::Tank, ds.hero_count()))
        .expect("scoring succeeds");

    for rec in &recs {
        let expected = 0.15 * f32::from(ds.base_strength(rec.hero)) / 100.0;
        assert!(
            (rec.score - expected).abs() < 1e-6,
            "{:?} scored {} rather than its all-ranks strength {expected}",
            rec.hero,
            rec.score
        );
        assert!(
            !rec.reasons
                .iter()
                .any(|reason| matches!(reason.kind, ReasonKind::RankFit(_))),
            "an unset rank must not put a rung on the panel"
        );
    }
}

/// And the other half: picking a rung has to actually change the answer.
#[test]
fn choosing_a_division_reorders_the_list_it_is_about() {
    let ds = rank_fixture();
    let draft = Draft::new();

    let ladder = recommend(&ds, &draft, &ranked_context(&ds, Rank::All)).expect("scores");
    assert_eq!(rank_of(&ladder, R_ANCHOR), 0, "60 leads across the ladder");
    assert_eq!(rank_of(&ladder, R_SMURF), 2);

    let bronze = recommend(&ds, &draft, &ranked_context(&ds, Rank::Bronze)).expect("scores");
    assert_eq!(
        rank_of(&bronze, R_SMURF),
        0,
        "a hero that only works low down leads at the bottom of the ladder"
    );

    let grandmaster =
        recommend(&ds, &draft, &ranked_context(&ds, Rank::Grandmaster)).expect("scores");
    assert_eq!(
        rank_of(&grandmaster, R_CLIMBER),
        0,
        "and the one that only works high up leads at the top"
    );
}

/// Rank reaches exactly one term. Nothing else in the scorer is rank-aware,
/// because no source publishes matchups or duos per rung — so if this ever fails,
/// something has claimed evidence that does not exist.
#[test]
fn zeroing_the_rank_weight_removes_the_term() {
    let ds = rank_fixture();
    let draft = Draft::new();

    let mut ctx = ranked_context(&ds, Rank::Grandmaster);
    ctx.weights.rank = 0.0;

    let silenced = recommend(&ds, &draft, &ctx).expect("scores");
    let unset = recommend(&ds, &draft, &ranked_context(&ds, Rank::All)).expect("scores");

    for (a, b) in silenced.iter().zip(&unset) {
        assert_eq!(a.hero, b.hero);
        assert!(
            (a.score - b.score).abs() < 1e-6,
            "{:?} scored {} with the term off and {} at all ranks",
            a.hero,
            a.score,
            b.score
        );
    }
}

/// At equal weights the two terms sum to exactly the rank-sliced strength. That
/// is what lets `Weights::rank` default to `Weights::base` without inventing a
/// number: the out-of-the-box behaviour is "read the column for the rung you
/// picked", and the knob is there for anybody who wants to argue with it.
#[test]
fn the_two_patch_terms_decompose_the_rank_sliced_strength_rather_than_double_counting() {
    let ds = rank_fixture();
    let draft = Draft::new();
    let ctx = ranked_context(&ds, Rank::Diamond);
    assert_eq!(
        ctx.weights.rank, ctx.weights.base,
        "the default this identity depends on"
    );

    for rec in recommend(&ds, &draft, &ctx).expect("scores") {
        let sliced = f32::from(ds.base_strength_at(Rank::Diamond, rec.hero)) / 100.0;
        assert!(
            (rec.score - ctx.weights.base * sliced).abs() < 1e-6,
            "{:?} scored {} rather than the sliced strength {sliced}",
            rec.hero,
            rec.score
        );
    }
}

/// The rung has to travel with the reason, because the panel is the only place
/// on screen that says which ladder the number came from.
#[test]
fn the_rank_reason_names_the_division_it_was_read_from() {
    let ds = rank_fixture();
    let draft = Draft::new();

    let recs = recommend(&ds, &draft, &ranked_context(&ds, Rank::Master)).expect("scores");
    let climber = recs
        .iter()
        .find(|rec| rec.hero == R_CLIMBER)
        .expect("in the list");

    assert!(
        climber
            .reasons
            .iter()
            .any(|reason| reason.kind == ReasonKind::RankFit(Rank::Master)),
        "got {:?}",
        climber.reasons
    );
}

/// A hero the sources agree is the same everywhere gets no line, the same way a
/// hero with no map affinity gets no map line. A zero term is a real reading and
/// "suits master right now" is not what it says.
#[test]
fn a_hero_with_no_rank_effect_is_left_out_of_the_panel_rather_than_explained() {
    let ds = rank_fixture();
    let draft = Draft::new();

    let recs = recommend(&ds, &draft, &ranked_context(&ds, Rank::Master)).expect("scores");
    let anchor = recs
        .iter()
        .find(|rec| rec.hero == R_ANCHOR)
        .expect("in the list");

    assert!(
        !anchor
            .reasons
            .iter()
            .any(|reason| matches!(reason.kind, ReasonKind::RankFit(_))),
        "got {:?}",
        anchor.reasons
    );
    // But it is still ranked on its all-ranks strength, which is the point of a
    // zero shift rather than a missing one.
    assert!(anchor.score > 0.0);
}

/// The patch rung of the ban list argues "these are the heroes winning right
/// now", and once you have said which ladder you are on, "right now" means
/// there. Two panels on one screen must not read patch strength from two
/// different populations.
#[test]
fn the_ban_lists_patch_rung_ranks_on_the_bracket_you_chose() {
    let ds = rank_fixture();
    let draft = Draft::new();
    let team = DefendedTeam::default();

    let ladder = ban_recommendations(&ds, &draft, &ranked_context(&ds, Rank::All), &team);
    assert_eq!(ladder.subject, BanSubject::Patch);
    assert_eq!(
        ban_rank_of(&ladder, R_ANCHOR),
        Some(0),
        "across the ladder the strongest hero leads the ban list"
    );

    let bronze = ban_recommendations(&ds, &draft, &ranked_context(&ds, Rank::Bronze), &team);
    assert_eq!(
        ban_rank_of(&bronze, R_SMURF),
        Some(0),
        "and at Bronze it is the one that is strongest at Bronze"
    );
}

// ---------------------------------------------------------------------------
// The breakdown: every term at once, and the arithmetic adding up.
// ---------------------------------------------------------------------------

// Reinhardt is first in the roster and last in the answer, deliberately: a list
// whose sorted order is already its roster order cannot tell `place` assigned
// after the sort from `place` assigned before it.
const A_REINHARDT: HeroId = HeroId(0);
const A_WINSTON: HeroId = HeroId(1);
const A_PHARAH: HeroId = HeroId(2);
const A_WIDOWMAKER: HeroId = HeroId(3);
const A_LUCIO: HeroId = HeroId(4);

/// The one fixture where **all eight terms are live at once**.
///
/// Every other fixture in this file deliberately zeroes everything but the term
/// it is about, which is right for testing a term and useless for testing a sum:
/// a total that drops a term nothing populated is still correct. Winston is the
/// hero carrying all eight; Reinhardt is beside him carrying most but not all,
/// which is what makes the zero rows visible.
///
/// A new fixture rather than an extension of an existing one. `sparse_fixture`,
/// `shape_fixture` and `rank_fixture` each carry goldens keyed to their exact
/// contents, so adding a hero or an axis to any of them moves numbers in tests
/// that are about something else entirely.
fn every_term_fixture() -> Dataset {
    let heroes = vec![
        hero("reinhardt", "Reinhardt", Role::Tank),
        hero("winston", "Winston", Role::Tank),
        hero("pharah", "Pharah", Role::Damage),
        hero("widowmaker", "Widowmaker", Role::Damage),
        hero("lucio", "Lúcio", Role::Support),
    ];
    let n = heroes.len();

    // Hybrid, because a side term needs a map that has sides at all.
    let maps = vec![GameMap {
        key: "kings-row".to_owned(),
        name: "King's Row".to_owned(),
        mode: GameMode::Hybrid,
        aliases: Vec::new(),
    }];

    // Winston is rated against one of the two enemies and not the other, so the
    // counter coverage on his row is a real fraction rather than a full house.
    let mut matchups = Matrix::unrated(n);
    matchups.set(A_WINSTON, A_PHARAH, 60).expect("in range");
    matchups.set(A_PHARAH, A_WINSTON, -60).expect("in range");
    matchups.set(A_REINHARDT, A_PHARAH, -40).expect("in range");
    matchups.set(A_PHARAH, A_REINHARDT, 40).expect("in range");

    let mut synergy = Matrix::unrated(n);
    synergy.set(A_WINSTON, A_LUCIO, 50).expect("in range");
    synergy.set(A_LUCIO, A_WINSTON, 50).expect("in range");

    let mut map_affinity = vec![0i8; maps.len() * n];
    map_affinity[A_WINSTON.index()] = 40;
    // Reinhardt's is left at zero on purpose: it is the row that proves a term
    // can be in the ledger and absent from the panel at the same time.

    let mut side_lean = vec![0i8; n];
    side_lean[A_WINSTON.index()] = 50; // attack

    let mut base_strength = vec![0i8; n];
    base_strength[A_WINSTON.index()] = 40;
    base_strength[A_REINHARDT.index()] = -20;

    let mut rank_shift = vec![[0i8; Rank::DIVISIONS.len()]; n];
    rank_shift[A_WINSTON.index()] = [30; Rank::DIVISIONS.len()];
    rank_shift[A_REINHARDT.index()] = [-10; Rank::DIVISIONS.len()];

    Dataset::new(DatasetParts {
        heroes,
        maps,
        matchups,
        synergy,
        map_affinity,
        base_strength,
        rank_shift,
        prevalence: vec![[0; Rank::CHOICES.len()]; n],
        win_rate: vec![None; n],
        side_lean,
        side_note: vec![String::new(); n],
        // Both enemies poke, so their shape has a leader well clear of
        // MIXED_MARGIN and the term has an axis to name.
        shape: vec![
            [0, 0, 95], // reinhardt: brawl, which walks into poke
            [95, 0, 0], // winston: dive, which answers it
            [0, 95, 0], // pharah: poke
            [0, 95, 0], // widowmaker: poke
            [0, 0, 0],  // lucio: nobody has read this one
        ],
        shape_note: vec![String::new(); n],
        reasons: vec![String::new(); n * n],
        disputed: vec![false; n * n],
        generated: "fixture".to_owned(),
        patch: "fixture".to_owned(),
    })
    .expect("fixture is internally consistent")
}

/// A draft and a context that light every term up at once.
fn every_term_draft(ds: &Dataset) -> (Draft, UserContext) {
    let mut draft = Draft::new();
    draft.map = Some(KINGS_ROW);
    draft.side = Some(Side::Attack);
    draft.add_enemy(A_PHARAH);
    draft.add_enemy(A_WIDOWMAKER);
    draft.add_ally(A_LUCIO);

    let mut ctx = ranked_context(ds, Rank::Master);
    ctx.overrides[A_WINSTON.index()] = ComfortStep::Good.value();
    ctx.overrides[A_REINHARDT.index()] = ComfortStep::Ok.value();

    (draft, ctx)
}

/// The headline. Every term the panel can show is in the ledger, and the ledger
/// is the number on the row — not close to it.
///
/// `assert_eq!` on `f32` and not a tolerance, deliberately. This is exact by
/// construction: `recommend` sets `score` to `breakdown.total()` and nothing
/// else computes it, and `total()` adds the eight in declaration order the way
/// the hand-written chain it replaced did. The test exists to keep it that way —
/// a tolerance here would pass a re-associated sum, which is the one thing worth
/// catching.
#[test]
fn the_terms_shown_add_up_to_the_score() {
    let ds = every_term_fixture();
    let (draft, ctx) = every_term_draft(&ds);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(!recs.is_empty());

    for rec in &recs {
        assert_eq!(
            rec.breakdown.total(),
            rec.score,
            "the ledger for {:?} does not come to its own score",
            rec.hero
        );
    }

    // And the fixture is doing its job: on Winston, not one of the eight is
    // silent. Without this the assertion above would hold on an empty draft.
    let winston = &recs[rank_of(&recs, A_WINSTON)].breakdown;
    for kind in TermKind::ALL {
        assert!(
            winston.term(kind).contribution() != 0.0,
            "{kind:?} came to nothing, so this fixture no longer tests a full sum"
        );
    }
}

/// The whole reason the ledger exists beside the reasons rather than inside
/// them. Reinhardt has no affinity for this map, and a zero term is a real
/// reading with no sentence to it — so it belongs in the arithmetic and not in
/// the panel.
#[test]
fn a_term_that_came_to_nothing_is_in_the_breakdown_but_never_in_the_reasons() {
    let ds = every_term_fixture();
    let (draft, ctx) = every_term_draft(&ds);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let rein = &recs[rank_of(&recs, A_REINHARDT)];

    assert_eq!(rein.breakdown.term(TermKind::Map).value, 0.0);
    assert!(
        !rein
            .reasons
            .iter()
            .any(|reason| matches!(reason.kind, ReasonKind::MapFit(_))),
        "a zero map affinity is not a reason to say the hero performs well there"
    );
    // Present in the ledger all the same, in its own slot rather than missing.
    assert_eq!(rein.breakdown.term(TermKind::Map).kind, TermKind::Map);
}

/// The term that moves a score while saying nothing at all, which no test could
/// see before there was a ledger to look in.
///
/// `a_mixed_enemy_team_ranks_every_shape_the_same` puts one pure axis on each of
/// three enemies, so the triangle cancels to exactly zero and the test passes
/// without ever distinguishing "no term" from "a term that came to zero". Two
/// dive and two poke is the case it cannot reach: the axes tie, so `leading()`
/// is `None` and there is no archetype for `CountersShape` to name — and yet the
/// candidate's own axes still have something to say about the half of the team
/// they answer.
#[test]
fn the_shape_a_mixed_enemy_team_produces_still_counts_and_still_has_no_reason_line() {
    let ds = shape_fixture();
    let mut draft = Draft::new();
    draft.add_enemy(T_WINSTON); // dive
    draft.add_enemy(T_TRACER); // dive
    draft.add_enemy(T_SIGMA); // poke
    draft.add_enemy(T_WIDOWMAKER); // poke

    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let rec = &recs[rank_of(&recs, T_WINSTON)];

    assert!(
        rec.breakdown.shape.is_mixed(),
        "the fixture no longer produces a mixed read, so this tests nothing"
    );
    assert!(
        rec.breakdown.term(TermKind::Shape).value != 0.0,
        "the term cancelled, which is the case the older test already covers"
    );
    assert!(
        !rec.reasons.iter().any(|reason| matches!(
            reason.kind,
            ReasonKind::CountersShape(_) | ReasonKind::LosesToShape(_)
        )),
        "a mixed team has no axis to name, so there is no sentence to be had"
    );
}

/// What Slice 21's coverage line is built on: the counter mean divides by every
/// enemy, so how many of them actually fed it is not recoverable from the number.
#[test]
fn the_breakdown_counts_how_many_enemies_actually_fed_the_counter_term() {
    let ds = sparse_fixture();
    let mut draft = Draft::new();
    draft.add_enemy(C_PHARAH); // rated against both tanks
    draft.add_enemy(C_MIZUKI); // nothing rates anyone against this one

    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let dva = &recs[rank_of(&recs, C_DVA)].breakdown;

    assert_eq!(dva.counter.entered, 2, "both enemies are on the board");
    assert_eq!(dva.counter.rated, 1, "only one of them has been rated");
    assert_eq!(dva.synergy.entered, 0, "nobody has picked an ally");
}

/// The cap moved to the view, so the scorer hands over everything it worked out
/// and the `why` panel can show the rest. The sort stays here, because that is a
/// claim about the terms rather than about a column.
#[test]
fn every_reason_survives_the_sort_rather_than_the_first_three() {
    let ds = every_term_fixture();
    let (draft, ctx) = every_term_draft(&ds);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let winston = &recs[rank_of(&recs, A_WINSTON)];

    assert!(
        winston.reasons.len() > 3,
        "got {} reasons, so the truncation is still somewhere",
        winston.reasons.len()
    );
    let mut previous = f32::INFINITY;
    for reason in &winston.reasons {
        let size = reason.contribution.abs();
        assert!(size <= previous, "reasons are out of order at {size}");
        previous = size;
    }
}

/// Two screens render this list and both used to number it by their own position
/// in their own copy of it. The number is a property of the answer now.
#[test]
fn the_place_on_each_recommendation_is_the_position_the_list_sorted_it_into() {
    let ds = every_term_fixture();
    let (draft, ctx) = every_term_draft(&ds);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(recs.len() > 1, "one row cannot be out of order");

    // The fixture puts the winner second in the roster, so this is the assertion
    // that separates "numbered after the sort" from "numbered before it". Without
    // it the test passes on any list whose roster order is already its answer.
    assert_ne!(
        recs[0].hero, A_REINHARDT,
        "the answer is in roster order, so this test cannot see the bug it is for"
    );

    for (index, rec) in recs.iter().enumerate() {
        assert_eq!(rec.place, index, "{:?} is numbered wrong", rec.hero);
    }
    // And the order it is numbering is the sorted one.
    for pair in recs.windows(2) {
        assert!(pair[0].score >= pair[1].score, "the list is not sorted");
    }
}

// ---------------------------------------------------------------------------
// Ties: when the list stops claiming an order it cannot defend.
// ---------------------------------------------------------------------------

/// Three tanks whose scores differ, but by less than one term's range.
///
/// `fixture()` leaves every base strength at zero, so on an empty board comfort
/// is the only live term and the gaps are exactly what this sets: D.Va 0.06,
/// Sigma 0.03, Reinhardt 0.00, against a band of 0.15. Deliberately *not* an
/// exactly flat field — scores that are bit-identical are tied at any band at
/// all, including zero, so a test built on one cannot tell a band from a bug.
fn near_flat(ds: &Dataset) -> UserContext {
    let mut ctx = tank_context(ds);
    ctx.overrides[DVA.index()] = 10;
    ctx.overrides[SIGMA.index()] = 5;
    ctx
}

/// A field with nothing much between its top rows says so, rather than
/// presenting an order the numbers do not support.
#[test]
fn a_flat_field_reports_its_top_as_a_tie_rather_than_an_order() {
    let ds = fixture();
    let ctx = near_flat(&ds);
    let recs = recommend(&ds, &Draft::new(), &ctx).expect("scoring succeeds");

    // The premise: these are three different numbers. The claim is not that they
    // are equal, it is that the differences are too small to rank on.
    assert!(
        recs[0].score > recs[1].score && recs[1].score > recs[2].score,
        "the fixture went flat, so this no longer tests a band"
    );
    assert!(
        recs.iter().take(3).all(|rec| rec.tied_with_top),
        "0.06 is less than half of what the rank term alone can move a score"
    );
}

/// The property the whole rendering rests on. The set is drawn as a boundary
/// between two rows rather than as a mark on each, and a boundary is only
/// meaningful if the set is a prefix — which it is, because the list is sorted
/// and `best - score` only grows down it.
#[test]
fn the_tied_set_is_always_a_prefix_of_the_list() {
    let ds = fixture();
    let ctx = tank_context(&ds);
    let mut draft = Draft::new();
    draft.add_enemy(PHARAH);
    draft.add_enemy(WIDOWMAKER);

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    let mut ended = false;
    for rec in &recs {
        if !rec.tied_with_top {
            ended = true;
        } else {
            assert!(
                !ended,
                "{:?} is tied with the top after the tie already ended",
                rec.hero
            );
        }
    }
}

/// The other half: a real gap is still a real gap, and the top hero is tied with
/// nothing but itself. Counting these has to treat one as no tie at all.
#[test]
fn a_clear_best_pick_is_tied_with_nothing_but_itself() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);
    // Comfort at the top rung is 0.60, four times the band, so this is a gap
    // nothing about the measurement's resolution can explain away.
    ctx.overrides[DVA.index()] = ComfortStep::Main.value();

    let recs = recommend(&ds, &Draft::new(), &ctx).expect("scoring succeeds");

    assert_eq!(recs[0].hero, DVA);
    assert!(recs[0].tied_with_top, "the top is always tied with itself");
    assert_eq!(
        recs.iter().filter(|rec| rec.tied_with_top).count(),
        1,
        "a 0.60 lead is not a tie at a 0.15 band"
    );
}

/// The design decision, pinned. These are different quantities — one prices the
/// cost of abandoning a hero you are playing, the other is the resolution the
/// measurement is read at — and sharing a field would mean anybody who raised the
/// swap bar because they hate switching also collapsed their top six into "too
/// close to call".
///
/// Both directions, and both against a field whose gaps sit *between* the two
/// values being set, because that is the only arrangement in which an alias shows
/// up at all: with both at their defaults the two are indistinguishable by
/// construction.
#[test]
fn the_tie_band_and_the_swap_threshold_move_independently() {
    let ds = fixture();
    let tied = |ctx: &UserContext| {
        recommend(&ds, &Draft::new(), ctx)
            .expect("scoring succeeds")
            .iter()
            .filter(|rec| rec.tied_with_top)
            .count()
    };

    let base = near_flat(&ds);
    assert_eq!(tied(&base), 3, "0.06 and 0.03 are both inside the band");

    // Move the swap bar under every gap on the board. If the flag read it, the
    // tie would collapse to the top hero alone.
    let mut swap_moved = near_flat(&ds);
    swap_moved.weights.swap_threshold = 0.01;
    assert_eq!(
        tied(&swap_moved),
        3,
        "the swap bar reached the tie band, which is the coupling this forbids"
    );

    // And the other way: move the band alone and the tie does collapse, while the
    // swap bar goes on pricing what it always priced.
    let mut band_moved = near_flat(&ds);
    band_moved.weights.tie_band = 0.01;
    assert_eq!(tied(&band_moved), 1, "a band under every gap ties nothing");
    assert_eq!(
        band_moved.weights.swap_threshold,
        Weights::default().swap_threshold,
        "and moving the band did not move the bar"
    );
}
