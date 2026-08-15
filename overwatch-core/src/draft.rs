use serde::{Deserialize, Serialize};

use crate::dataset::Dataset;
use crate::format::{Capacity, Format};
use crate::hero::{HeroId, Role};
use crate::map::{MapId, Side};

/// The state of one hero-select, and the only thing the sync socket carries.
///
/// Entry is order-free: enemy picks go into a flat list in whatever order they
/// are typed, because assigning them to slots is a decision that costs time and
/// buys nothing — the scorer treats the enemy team as a set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// The lobby this draft is in: how big both teams are, and whether the queue
    /// holds them to a role split.
    ///
    /// A `Draft` enforces only what it can see on its own — no duplicates, and
    /// no more bodies than a team has. The per-role caps need the roster to know
    /// what role a pick even is, and the reservations need the seats beside you,
    /// so those are applied by [`crate::session::SessionState::draft_for`],
    /// which has both. Both layers count through [`Capacity`], so there is still
    /// one rule.
    #[serde(default)]
    pub format: Format,
    pub map: Option<MapId>,
    /// Which half of a payload map you are on. Only meaningful when the map's
    /// mode [`has_sides`](crate::map::GameMode::has_sides).
    #[serde(default)]
    pub side: Option<Side>,
    #[serde(default)]
    pub enemies: Vec<HeroId>,
    /// Your team's picks, excluding your own. Used for synergy and to stop the
    /// app suggesting a hero a teammate already took.
    #[serde(default)]
    pub allies: Vec<HeroId>,
    /// Your current hero, once you have locked in. Its presence switches the
    /// recommendation list into swap mode.
    pub locked: Option<HeroId>,
}

impl Draft {
    pub fn new() -> Self {
        Self::default()
    }

    /// A draft of a given format, for the callers that know one before they know
    /// anything else.
    pub fn in_format(format: Format) -> Self {
        Self {
            format,
            ..Self::default()
        }
    }

    /// Adds an enemy pick, ignoring duplicates and refusing to overfill the
    /// team. Returns whether the draft actually changed.
    pub fn add_enemy(&mut self, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) || self.enemies.len() >= self.format.team_size() {
            return false;
        }
        self.enemies.push(hero);
        true
    }

    /// Your own team cannot run duplicates, so an ally pick is rejected if it
    /// is already taken by another ally or by you.
    pub fn add_ally(&mut self, hero: HeroId) -> bool {
        let taken = self.allies.contains(&hero) || self.locked == Some(hero);
        if taken || self.allies.len() >= self.format.team_size() - 1 {
            return false;
        }
        self.allies.push(hero);
        true
    }

    /// What the enemy team can still take, per role.
    ///
    /// The enemy has no seats and no declared roles, so their picks are the
    /// whole story and there is nothing to hold open — which is what makes this
    /// a plain count where the ally side needs the roster.
    pub fn enemy_capacity(&self, dataset: &Dataset) -> Capacity {
        capacity_after(dataset, self.format, &self.enemies)
    }

    pub fn remove_enemy(&mut self, hero: HeroId) {
        self.enemies.retain(|h| *h != hero);
    }

    pub fn remove_ally(&mut self, hero: HeroId) {
        self.allies.retain(|h| *h != hero);
    }

    /// Adds an enemy pick, or takes it back if it is already there. Returns
    /// whether the hero is on the enemy team afterwards.
    ///
    /// This is what a click on the enemy board does: one gesture both ways, so
    /// correcting a misread portrait costs the same as entering it did. A team
    /// that is already full simply reports `false` — the tile is drawn disabled,
    /// so a click that cannot land also does not look like it should.
    pub fn toggle_enemy(&mut self, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) {
            self.remove_enemy(hero);
            return false;
        }
        self.add_enemy(hero)
    }

    /// Adds an ally pick, or takes it back if it is already there. Returns
    /// whether the hero is on your team afterwards.
    pub fn toggle_ally(&mut self, hero: HeroId) -> bool {
        if self.allies.contains(&hero) {
            self.remove_ally(hero);
            return false;
        }
        self.add_ally(hero)
    }

    pub fn pop_enemy(&mut self) -> Option<HeroId> {
        self.enemies.pop()
    }

    pub fn pop_ally(&mut self) -> Option<HeroId> {
        self.allies.pop()
    }

    /// Clears the picks but keeps the map and side: a match is played on one map
    /// across many rounds of re-picking, so re-entering it every time is wasted
    /// input. The side does swap between rounds, but flipping it here would be
    /// guessing at which round just ended.
    pub fn clear_picks(&mut self) {
        self.enemies.clear();
        self.allies.clear();
        self.locked = None;
    }

    /// Clears everything, map and side included. This is the "new match" reset,
    /// as opposed to [`Self::clear_picks`]'s "next round".
    ///
    /// The format survives even this. It describes the queue you are sitting in
    /// rather than the match in front of you: it changes when you leave the
    /// queue, which is not something a reset between matches can know.
    pub fn clear_all(&mut self) {
        self.clear_picks();
        self.map = None;
        self.side = None;
    }

    pub fn is_empty(&self) -> bool {
        self.enemies.is_empty() && self.allies.is_empty() && self.locked.is_none()
    }
}

/// A hero id's role, or `None` if the roster has no such hero.
pub(crate) fn role_of(dataset: &Dataset, hero: HeroId) -> Option<Role> {
    dataset.hero(hero).ok().map(|entry| entry.role)
}

/// The room a team of `format` has left once `picks` are on it.
pub(crate) fn capacity_after(dataset: &Dataset, format: Format, picks: &[HeroId]) -> Capacity {
    let mut room = Capacity::of(format);
    for hero in picks {
        room.take(role_of(dataset, *hero));
    }
    room
}

/// The part of `picks` a `format` can actually hold: the first `slots(role)` of
/// each role, in entry order, and never more than a team.
///
/// First-entered wins, which is the same rule every refusal in this file already
/// follows — [`Draft::add_enemy`] declines a sixth pick rather than evicting the
/// first — applied after the fact, for when the format shrinks under picks that
/// were legal when they were made.
pub fn fit_to_format(dataset: &Dataset, format: Format, picks: &[HeroId]) -> Vec<HeroId> {
    let mut room = Capacity::of(format);
    picks
        .iter()
        .copied()
        .filter(|hero| {
            let role = role_of(dataset, *hero);
            let fits = room.fits(role);
            if fits {
                room.take(role);
            }
            fits
        })
        .collect()
}

/// The enemy picks of one role, in entry order.
///
/// Enemy slots are *derived* from each hero's own role rather than stored, so
/// entry stays order-free while the board can still show the shape of what is
/// still unknown — an empty tank slot, one dps of two.
pub fn enemies_in_role(dataset: &Dataset, draft: &Draft, role: Role) -> Vec<HeroId> {
    draft
        .enemies
        .iter()
        .copied()
        .filter(|enemy| role_of(dataset, *enemy) == Some(role))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{
        self, ANA, ASHE, KIRIKO, LUCIO, NOBODY, REINHARDT, SIGMA, SOJOURN, TRACER, WINSTON,
    };
    use crate::format::{Queue, TeamSize};

    fn six_v_six() -> Format {
        Format::new(TeamSize::SixVSix, Queue::Role)
    }

    #[test]
    fn enemy_entry_is_deduplicated_and_capped() {
        let mut draft = Draft::new();
        assert!(draft.add_enemy(SIGMA));
        assert!(!draft.add_enemy(SIGMA), "duplicate rejected");

        for hero in [TRACER, SOJOURN, ANA, LUCIO] {
            assert!(draft.add_enemy(hero));
        }
        assert!(!draft.add_enemy(WINSTON), "sixth enemy rejected in 5v5");
        assert_eq!(draft.enemies.len(), 5);
    }

    /// The size cap is the format's now, not a constant. A sixth body is the
    /// whole difference between the two, so it is asserted from both sides.
    #[test]
    fn a_sixth_enemy_is_refused_in_5v5_and_taken_in_6v6() {
        let five = [SIGMA, TRACER, SOJOURN, ANA, LUCIO];

        let mut draft = Draft::new();
        for hero in five {
            assert!(draft.add_enemy(hero));
        }
        assert!(!draft.add_enemy(WINSTON));

        let mut draft = Draft::in_format(six_v_six());
        for hero in five {
            assert!(draft.add_enemy(hero));
        }
        assert!(draft.add_enemy(WINSTON), "6v6 has room for the second tank");
        assert!(!draft.add_enemy(KIRIKO), "and not for a seventh");
    }

    #[test]
    fn ally_entry_reserves_your_own_slot() {
        let mut draft = Draft::new();
        for hero in [SIGMA, TRACER, SOJOURN, ANA] {
            assert!(draft.add_ally(hero));
        }
        assert!(!draft.add_ally(LUCIO), "your slot stays free");
        assert_eq!(draft.allies.len(), 4);

        // The same rule one body wider.
        let mut draft = Draft::in_format(six_v_six());
        for hero in [SIGMA, TRACER, SOJOURN, ANA, LUCIO] {
            assert!(draft.add_ally(hero));
        }
        assert!(!draft.add_ally(WINSTON));
        assert_eq!(draft.allies.len(), 5);
    }

    #[test]
    fn ally_cannot_duplicate_your_locked_hero() {
        let mut draft = Draft::new();
        draft.locked = Some(ANA);
        assert!(!draft.add_ally(ANA));
    }

    #[test]
    fn clearing_picks_keeps_the_map() {
        let mut draft = Draft::new();
        draft.map = Some(fixture::KINGS_ROW);
        draft.add_enemy(SIGMA);
        draft.locked = Some(TRACER);

        draft.clear_picks();

        assert_eq!(draft.map, Some(fixture::KINGS_ROW));
        assert!(draft.is_empty());
    }

    /// The format outlives even the "new match" reset: it says which queue you
    /// are sitting in, and that does not change between matches.
    #[test]
    fn clearing_a_draft_keeps_the_format() {
        let mut draft = Draft::in_format(six_v_six());
        draft.add_enemy(SIGMA);

        draft.clear_picks();
        assert_eq!(draft.format, six_v_six());

        draft.clear_all();
        assert_eq!(draft.format, six_v_six());
    }

    #[test]
    fn toggling_a_pick_puts_it_back_the_way_it_was() {
        let mut draft = Draft::new();

        assert!(draft.toggle_enemy(SIGMA));
        assert_eq!(draft.enemies, vec![SIGMA]);
        assert!(!draft.toggle_enemy(SIGMA));
        assert!(draft.enemies.is_empty());

        assert!(draft.toggle_ally(TRACER));
        assert_eq!(draft.allies, vec![TRACER]);
        assert!(!draft.toggle_ally(TRACER));
        assert!(draft.allies.is_empty());
    }

    #[test]
    fn toggling_onto_a_full_team_changes_nothing() {
        let mut draft = Draft::new();
        for hero in [SIGMA, TRACER, SOJOURN, ANA, LUCIO] {
            draft.add_enemy(hero);
        }

        // Refused, and — the part worth asserting — it must not evict anyone to
        // make room, which is what a naive toggle would do.
        assert!(!draft.toggle_enemy(WINSTON));
        assert_eq!(draft.enemies.len(), 5);
        assert!(!draft.enemies.contains(&WINSTON));
    }

    #[test]
    fn enemies_are_grouped_by_their_own_role() {
        let ds = fixture::dataset();
        let mut draft = Draft::new();
        // Entered in whatever order they appeared, not in slot order.
        for hero in [LUCIO, SIGMA, TRACER, ANA] {
            draft.add_enemy(hero);
        }

        assert_eq!(enemies_in_role(&ds, &draft, Role::Tank), vec![SIGMA]);
        assert_eq!(enemies_in_role(&ds, &draft, Role::Damage), vec![TRACER]);
        assert_eq!(
            enemies_in_role(&ds, &draft, Role::Support),
            vec![LUCIO, ANA],
            "entry order is preserved within a role"
        );
    }

    #[test]
    fn clearing_everything_drops_the_map_and_side_too() {
        let mut draft = Draft::new();
        draft.map = Some(fixture::KINGS_ROW);
        draft.side = Some(Side::Attack);
        draft.add_enemy(SIGMA);

        // `clear_picks` is the next round; `clear_all` is a new match.
        draft.clear_picks();
        assert_eq!(draft.map, Some(fixture::KINGS_ROW));
        assert_eq!(draft.side, Some(Side::Attack));
        assert!(draft.is_empty());

        draft.clear_all();
        assert_eq!(draft.map, None);
        assert_eq!(draft.side, None);
    }

    // --- room, per role --------------------------------------------------

    #[test]
    fn one_enemy_tank_closes_the_tank_row_in_5v5_and_not_in_6v6() {
        let ds = fixture::dataset();

        let mut draft = Draft::new();
        draft.add_enemy(SIGMA);
        assert!(!draft.enemy_capacity(&ds).fits(Some(Role::Tank)));

        let mut draft = Draft::in_format(six_v_six());
        draft.add_enemy(SIGMA);
        assert!(draft.enemy_capacity(&ds).fits(Some(Role::Tank)));
        draft.add_enemy(WINSTON);
        assert!(
            !draft.enemy_capacity(&ds).fits(Some(Role::Tank)),
            "and a third is too many even in 6v6"
        );
    }

    /// The whole point of a per-role cap: the rows are independent. A team that
    /// has both its dps can still take a tank.
    #[test]
    fn a_full_dps_row_does_not_block_the_tank_row() {
        let ds = fixture::dataset();
        let mut draft = Draft::new();
        draft.add_enemy(TRACER);
        draft.add_enemy(SOJOURN);

        let room = draft.enemy_capacity(&ds);
        assert!(!room.fits(Some(Role::Damage)));
        assert!(room.fits(Some(Role::Tank)));
        assert!(room.fits(Some(Role::Support)));
    }

    #[test]
    fn open_queue_takes_five_of_one_role_and_then_stops() {
        let ds = fixture::dataset();
        let open = Format::new(TeamSize::FiveVFive, Queue::Open);
        let mut draft = Draft::in_format(open);

        for hero in [ANA, LUCIO, KIRIKO] {
            assert!(draft.add_enemy(hero));
        }
        assert!(
            draft.enemy_capacity(&ds).fits(Some(Role::Support)),
            "open queue does not care that these are all supports"
        );

        for hero in [REINHARDT, SIGMA] {
            assert!(draft.add_enemy(hero));
        }
        assert!(
            !draft.enemy_capacity(&ds).fits(Some(Role::Support)),
            "the team size still binds"
        );
    }

    #[test]
    fn a_hero_the_roster_does_not_know_still_counts_against_the_team_size() {
        let ds = fixture::dataset();
        let mut draft = Draft::new();
        draft.add_enemy(NOBODY);

        let room = draft.enemy_capacity(&ds);
        assert_eq!(room.total_free(), 4, "it is still a body on their team");
        assert_eq!(room.free_in(Role::Tank), 1, "but it took no role slot");
    }

    // --- shrinking the format --------------------------------------------

    #[test]
    fn switching_to_a_smaller_format_keeps_the_picks_entered_first() {
        let ds = fixture::dataset();
        let entered = vec![SIGMA, WINSTON, TRACER, SOJOURN, ANA, LUCIO];

        assert_eq!(
            fit_to_format(&ds, six_v_six(), &entered),
            entered,
            "a legal 6v6 comp survives itself"
        );
        assert_eq!(
            fit_to_format(&ds, Format::default(), &entered),
            vec![SIGMA, TRACER, SOJOURN, ANA, LUCIO],
            "5v5 drops the second tank, not the first"
        );
    }

    #[test]
    fn fitting_a_format_that_already_holds_everything_changes_nothing() {
        let ds = fixture::dataset();
        let entered = vec![SIGMA, TRACER, ANA];

        assert_eq!(fit_to_format(&ds, Format::default(), &entered), entered);
        assert_eq!(fit_to_format(&ds, six_v_six(), &entered), entered);
    }

    #[test]
    fn fitting_a_format_drops_the_bodies_it_has_no_room_for_at_all() {
        let ds = fixture::dataset();
        // Junk from a client that thinks the roster is bigger than it is.
        let entered = vec![ASHE, NOBODY, NOBODY, NOBODY, NOBODY, NOBODY];

        assert_eq!(
            fit_to_format(&ds, Format::default(), &entered).len(),
            5,
            "unknown ids are bodies, and a team holds five of them"
        );
    }
}
