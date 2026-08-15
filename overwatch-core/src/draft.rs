use serde::{Deserialize, Serialize};

use crate::dataset::Dataset;
use crate::hero::{HeroId, Role};
use crate::map::{MapId, Side};

/// The state of one hero-select, and the only thing the sync socket carries.
///
/// Entry is order-free: enemy picks go into a flat list in whatever order they
/// are typed, because assigning them to slots is a decision that costs time and
/// buys nothing — the scorer treats the enemy team as a set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
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
    pub const TEAM_SIZE_5V5: usize = 5;

    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an enemy pick, ignoring duplicates and refusing to overfill the
    /// team. Returns whether the draft actually changed.
    pub fn add_enemy(&mut self, hero: HeroId) -> bool {
        if self.enemies.contains(&hero) || self.enemies.len() >= Self::TEAM_SIZE_5V5 {
            return false;
        }
        self.enemies.push(hero);
        true
    }

    /// Your own team cannot run duplicates, so an ally pick is rejected if it
    /// is already taken by another ally or by you.
    pub fn add_ally(&mut self, hero: HeroId) -> bool {
        let taken = self.allies.contains(&hero) || self.locked == Some(hero);
        if taken || self.allies.len() >= Self::TEAM_SIZE_5V5 - 1 {
            return false;
        }
        self.allies.push(hero);
        true
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
    pub fn clear_all(&mut self) {
        self.clear_picks();
        self.map = None;
        self.side = None;
    }

    pub fn is_empty(&self) -> bool {
        self.enemies.is_empty() && self.allies.is_empty() && self.locked.is_none()
    }
}

/// An enemy id's role, or `None` if the roster has no such hero.
fn role_of(dataset: &Dataset, hero: HeroId) -> Option<Role> {
    dataset.hero(hero).ok().map(|entry| entry.role)
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

    #[test]
    fn enemy_entry_is_deduplicated_and_capped() {
        let mut draft = Draft::new();
        assert!(draft.add_enemy(HeroId(1)));
        assert!(!draft.add_enemy(HeroId(1)), "duplicate rejected");

        for id in 2..=5 {
            assert!(draft.add_enemy(HeroId(id)));
        }
        assert!(!draft.add_enemy(HeroId(6)), "sixth enemy rejected in 5v5");
        assert_eq!(draft.enemies.len(), 5);
    }

    #[test]
    fn ally_entry_reserves_your_own_slot() {
        let mut draft = Draft::new();
        for id in 1..=4 {
            assert!(draft.add_ally(HeroId(id)));
        }
        assert!(!draft.add_ally(HeroId(5)), "your slot stays free");
        assert_eq!(draft.allies.len(), 4);
    }

    #[test]
    fn ally_cannot_duplicate_your_locked_hero() {
        let mut draft = Draft::new();
        draft.locked = Some(HeroId(7));
        assert!(!draft.add_ally(HeroId(7)));
    }

    #[test]
    fn clearing_picks_keeps_the_map() {
        let mut draft = Draft::new();
        draft.map = Some(MapId(3));
        draft.add_enemy(HeroId(1));
        draft.locked = Some(HeroId(2));

        draft.clear_picks();

        assert_eq!(draft.map, Some(MapId(3)));
        assert!(draft.is_empty());
    }

    #[test]
    fn toggling_a_pick_puts_it_back_the_way_it_was() {
        let mut draft = Draft::new();

        assert!(draft.toggle_enemy(HeroId(1)));
        assert_eq!(draft.enemies, vec![HeroId(1)]);
        assert!(!draft.toggle_enemy(HeroId(1)));
        assert!(draft.enemies.is_empty());

        assert!(draft.toggle_ally(HeroId(2)));
        assert_eq!(draft.allies, vec![HeroId(2)]);
        assert!(!draft.toggle_ally(HeroId(2)));
        assert!(draft.allies.is_empty());
    }

    #[test]
    fn toggling_onto_a_full_team_changes_nothing() {
        let mut draft = Draft::new();
        for id in 1..=5 {
            draft.add_enemy(HeroId(id));
        }

        // Refused, and — the part worth asserting — it must not evict anyone to
        // make room, which is what a naive toggle would do.
        assert!(!draft.toggle_enemy(HeroId(6)));
        assert_eq!(draft.enemies.len(), 5);
        assert!(!draft.enemies.contains(&HeroId(6)));
    }

    #[test]
    fn enemies_are_grouped_by_their_own_role() {
        let ds = fixture();
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
        draft.map = Some(MapId(3));
        draft.side = Some(Side::Attack);
        draft.add_enemy(HeroId(1));

        // `clear_picks` is the next round; `clear_all` is a new match.
        draft.clear_picks();
        assert_eq!(draft.map, Some(MapId(3)));
        assert_eq!(draft.side, Some(Side::Attack));
        assert!(draft.is_empty());

        draft.clear_all();
        assert_eq!(draft.map, None);
        assert_eq!(draft.side, None);
    }

    // --- fixture ---------------------------------------------------------

    const SIGMA: HeroId = HeroId(1);
    const TRACER: HeroId = HeroId(2);
    const ANA: HeroId = HeroId(4);
    const LUCIO: HeroId = HeroId(5);

    fn hero(key: &str, role: Role) -> crate::hero::Hero {
        crate::hero::Hero {
            key: key.to_owned(),
            name: key.to_owned(),
            role,
            aliases: Vec::new(),
        }
    }

    fn fixture() -> Dataset {
        let heroes = vec![
            hero("reinhardt", Role::Tank),
            hero("sigma", Role::Tank),
            hero("tracer", Role::Damage),
            hero("sojourn", Role::Damage),
            hero("ana", Role::Support),
            hero("lucio", Role::Support),
        ];
        let n = heroes.len();

        Dataset::new(crate::dataset::DatasetParts {
            heroes,
            maps: vec![crate::map::GameMap {
                key: "kings-row".to_owned(),
                name: "King's Row".to_owned(),
                mode: crate::map::GameMode::Hybrid,
                aliases: Vec::new(),
            }],
            matchups: crate::matrix::Matrix::unrated(n),
            synergy: crate::matrix::Matrix::unrated(n),
            map_affinity: vec![0; n],
            base_strength: vec![0; n],
            side_lean: vec![0; n],
            reasons: vec![String::new(); n * n],
            generated: "test".to_owned(),
            patch: "test".to_owned(),
        })
        .expect("fixture is well formed")
    }
}
