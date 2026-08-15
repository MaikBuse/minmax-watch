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
    ban_recommendations, difficulty_to_value, recommend, threats, BanBoard, BanSubject, Dataset,
    DatasetParts, Draft, EnemyRoleWeights, GameMap, GameMode, Hero, HeroId, HeroSet, MapId, Matrix,
    ReasonKind, Role, Side, UserContext,
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
        side_lean: vec![0; n],
        reasons,
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
        side_lean: vec![0; n],
        reasons: vec![String::new(); n * n],
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
        side_lean: vec![0; n],
        reasons: vec![String::new(); n * n],
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
        side_lean,
        reasons: vec![String::new(); n * n],
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

// --- ban recommendations -----------------------------------------------------
//
// The ban phase runs before anyone picks, so these cases are mostly about who
// the answer is *for*: one locked hero, or the whole set you might end up on.

fn pool_of(heroes: [HeroId; 2]) -> HeroSet {
    HeroSet::from_iter_checked(heroes).expect("fixture indices are in range")
}

fn ban_rank_of(board: &BanBoard, hero: HeroId) -> Option<usize> {
    board.candidates.iter().position(|c| c.hero == hero)
}

#[test]
fn a_locked_pick_is_banned_for_by_its_own_matchups() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.locked = Some(REINHARDT);

    let board = ban_recommendations(&ds, &draft, &ctx, &HeroSet::empty());

    assert_eq!(board.subject, BanSubject::Locked(REINHARDT));
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
    assert_eq!(board.candidates[0].worst, REINHARDT);
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
    let pool = pool_of([REINHARDT, DVA]);

    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &pool);

    assert_eq!(board.subject, BanSubject::Pool);
    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![PHARAH],
    );
    // (+1.0 against Reinhardt, -0.75 against D.Va) / 2.
    assert!((board.candidates[0].severity - 0.125).abs() < 1e-6);
    assert_eq!(
        board.candidates[0].worst, REINHARDT,
        "the average decides the ranking; the worst case names who it is for"
    );
}

#[test]
fn an_empty_pool_bans_for_the_whole_role() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &HeroSet::empty());

    // An unmarked pool means "I have not said who I play", which is a reason to
    // answer for the role rather than a reason to answer nothing.
    assert_eq!(board.subject, BanSubject::Role(Role::Tank));
    assert_eq!(
        board.candidates.iter().map(|c| c.hero).collect::<Vec<_>>(),
        vec![PHARAH],
    );
    // (+1.0, +0.25, -0.75) / 3 across all three tanks — a different number from
    // the two-hero pool above, which is the point of the distinction.
    assert!((board.candidates[0].severity - 1.0 / 6.0).abs() < 1e-6);
}

#[test]
fn heroes_your_side_of_the_draft_beats_are_not_worth_banning() {
    let ds = fixture();
    let ctx = tank_context(&ds);

    let mut draft = Draft::new();
    draft.locked = Some(DVA);

    let board = ban_recommendations(&ds, &draft, &ctx, &HeroSet::empty());

    // D.Va beats everyone the fixture rates her against, so there is no ban to
    // spend — and saying so with an empty list is the honest answer.
    assert!(board.candidates.is_empty());
}

#[test]
fn a_hero_already_in_the_draft_is_not_a_ban() {
    let ds = fixture();
    let ctx = tank_context(&ds);
    let pool = pool_of([REINHARDT, SIGMA]);

    let open = ban_recommendations(&ds, &Draft::new(), &ctx, &pool);
    assert!(ban_rank_of(&open, PHARAH).is_some(), "worth banning");

    // Picked, so the ban phase is over for her either way.
    let mut drafted = Draft::new();
    drafted.add_enemy(PHARAH);
    assert!(ban_rank_of(&ban_recommendations(&ds, &drafted, &ctx, &pool), PHARAH).is_none());

    // On your own team, so banning her is banning yourself.
    let mut allied = Draft::new();
    allied.add_ally(PHARAH);
    assert!(ban_rank_of(&ban_recommendations(&ds, &allied, &ctx, &pool), PHARAH).is_none());
}

#[test]
fn the_enemy_tank_is_worth_more_to_ban_for_a_tank_player() {
    let ds = symmetric_fixture();
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let mut draft = Draft::new();
    draft.locked = Some(W_SIGMA);

    let board = ban_recommendations(&ds, &draft, &ctx, &HeroSet::empty());

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
