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
    Dataset, DatasetParts, Defended, DefendedTeam, Draft, EnemyRoleWeights, GameMap, GameMode,
    Hero, HeroId, Knowledge, MapId, Matrix, Rank, ReasonKind, Role, Side, UserContext,
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
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
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

#[test]
fn comfort_can_outweigh_a_bad_matchup() {
    let ds = fixture();
    let mut ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.add_enemy(WIDOWMAKER);

    let neutral = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(rank_of(&neutral, REINHARDT) > rank_of(&neutral, SIGMA));

    // Max out comfort on Reinhardt: a hero you actually play well beats the
    // technically correct pick you cannot.
    ctx.overrides[REINHARDT.index()] = 100;
    let tuned = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert!(rank_of(&tuned, REINHARDT) < rank_of(&tuned, SIGMA));
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
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
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

    // Normalising over the enemies actually entered means one enemy weighs
    // `w/w == 1`: the answer you get from the first pick alone is untouched.
    for (weighted, plain) in a.iter().zip(b.iter()) {
        assert_eq!(weighted.hero, plain.hero);
        assert!((weighted.score - plain.score).abs() < 1e-6);
    }
}

// --- partial coverage ------------------------------------------------------
//
// The matrix has an unrated state, and the counter term is a mean, so the thing
// worth pinning down is what a missing reading does to that mean: nothing. These
// use their own fixture because the ones above are deliberately fully rated over
// the pairs they exercise.

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
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
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

#[test]
fn an_unrated_enemy_is_left_out_of_the_mean_rather_than_counted_as_even() {
    let ds = sparse_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut rated_only = Draft::new();
    rated_only.add_enemy(C_PHARAH);

    let mut with_unknown = Draft::new();
    with_unknown.add_enemy(C_PHARAH);
    with_unknown.add_enemy(C_MIZUKI);

    let a = recommend(&ds, &rated_only, &ctx).expect("scoring succeeds");
    let b = recommend(&ds, &with_unknown, &ctx).expect("scoring succeeds");

    // Adding an enemy nobody has rated tells us nothing, so it must not move the
    // answer. Folding it in as a zero would have halved both scores.
    for hero in [C_REINHARDT, C_DVA] {
        assert!(
            (score_of(&a, hero) - score_of(&b, hero)).abs() < 1e-6,
            "an unrated enemy moved the score from {} to {}",
            score_of(&a, hero),
            score_of(&b, hero)
        );
    }
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
    // `BeatsEnemy` and rendered as "strong into Mizuki" on no evidence at all.
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
    // as one puts "strong into D.Va" under D.Va's own portrait.
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
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
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
        win_rate: vec![None; n],
        side_lean,
        shape: vec![[0; 3]; n],
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
        win_rate: vec![Some(48.5), Some(52.0), Some(46.0), Some(54.0)],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
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
    // no-op and the score is exactly `severity * enemy_roles[tank][support]`.
    let ana = board.candidates.first().expect("Ana is ranked");
    assert_eq!(ana.hero, W_ANA);
    assert!((ana.severity - 0.60).abs() < 1e-6);
    let weight = ctx.weights.enemy_roles.get(Role::Tank, Role::Support);
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
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![
            [95, 0, 0], // winston: dive
            [0, 0, 95], // reinhardt: brawl
            [0, 95, 0], // sigma: poke
            [0, 95, 0], // widowmaker: poke
            [95, 0, 0], // tracer: dive
            [0, 0, 0],  // unread: nobody has curated this one
        ],
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
        win_rate: vec![None; n],
        side_lean: vec![0; n],
        shape: vec![[0; 3]; n],
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
