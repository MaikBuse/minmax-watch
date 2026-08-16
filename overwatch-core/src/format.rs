//! The shape of the two teams: how many players, and whether the queue holds
//! them to a role split.
//!
//! Kept apart from [`crate::map::GameMode`], which is what the *map* is played
//! as. This is what the *lobby* is, and the two vary independently — every map
//! mode exists in both team sizes and both queues.

use serde::{Deserialize, Serialize};

use crate::hero::Role;

/// How many players a team fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TeamSize {
    /// The default, and a compatibility choice rather than a claim about which
    /// queue people play: it is what every board written before formats existed
    /// meant, and what an older client still means when it sends none.
    #[default]
    #[serde(rename = "5v5")]
    FiveVFive,
    #[serde(rename = "6v6")]
    SixVSix,
}

impl TeamSize {
    /// Both sizes, in the order the switch walks them.
    pub const BOTH: [TeamSize; 2] = [TeamSize::FiveVFive, TeamSize::SixVSix];

    pub const fn players(self) -> usize {
        match self {
            TeamSize::FiveVFive => 5,
            TeamSize::SixVSix => 6,
        }
    }

    /// The stable key, and also what the switch shows. The two coincide here,
    /// unlike [`Role::as_str`] and [`Role::label`], but they are still named
    /// apart so that restyling the chip can never change what is on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            TeamSize::FiveVFive => "5v5",
            TeamSize::SixVSix => "6v6",
        }
    }

    pub const fn label(self) -> &'static str {
        self.as_str()
    }
}

/// Whether the queue holds each team to a role split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Queue {
    #[default]
    Role,
    Open,
}

impl Queue {
    pub const BOTH: [Queue; 2] = [Queue::Role, Queue::Open];

    pub const fn as_str(self) -> &'static str {
        match self {
            Queue::Role => "role",
            Queue::Open => "open",
        }
    }

    /// What the switch shows. Short, because it sits beside the size in a header
    /// that is already carrying the map, the side, the sync light and a reset.
    pub const fn label(self) -> &'static str {
        self.as_str()
    }

    /// The long form, for the tooltip that has room to say what the choice does.
    pub const fn description(self) -> &'static str {
        match self {
            Queue::Role => "role queue — each team holds to its role split",
            Queue::Open => {
                "open queue — no damage or support split, but 6v6 still caps tanks at two"
            }
        }
    }
}

/// The shape of both teams in the match being drafted.
///
/// Two axes rather than one four-way choice, because the game asks them
/// separately and they are chosen independently. A flattened
/// `enum { FiveVFive, FiveVFiveOpen, .. }` would make each of the two switches
/// reconstruct the other axis to build its answer — a product type written by
/// hand. It also gives each half its own serde default, so a board carrying only
/// `{"size":"6v6"}` reads as 6v6 role queue rather than as nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Format {
    #[serde(default)]
    pub size: TeamSize,
    #[serde(default)]
    pub queue: Queue,
}

impl Format {
    pub const fn new(size: TeamSize, queue: Queue) -> Self {
        Self { size, queue }
    }

    pub const fn team_size(self) -> usize {
        self.size.players()
    }

    /// How many of `role` one team can field.
    ///
    /// Open queue mostly returns the whole team rather than a special "no limit"
    /// value. That is what keeps every caller on one code path: a per-role cap
    /// equal to the team size can never bind before the team size itself does,
    /// so those cells need no second rule and no branch anywhere else.
    ///
    /// The one exception is 6v6 open queue, which is the only open queue the
    /// game actually ships as a competitive playlist and which caps tanks at
    /// two. Without that cell a board would happily accept six enemy tanks and
    /// score a draft that cannot be queued for.
    pub const fn slots(self, role: Role) -> usize {
        match (self.queue, self.size, role) {
            // 6v6 open queue drops the damage and support split but keeps the
            // tank limit: any number of damage and supports, never more than two
            // tanks. The 5v5 cell is left uncapped because no source says
            // otherwise — it is not a live competitive playlist to check against.
            (Queue::Open, TeamSize::SixVSix, Role::Tank) => 2,
            (Queue::Open, size, _) => size.players(),
            // The one asymmetry in the role queue: 5v5 dropped the second tank,
            // and 6v6 is that tank coming back.
            (Queue::Role, TeamSize::FiveVFive, Role::Tank) => 1,
            (Queue::Role, TeamSize::SixVSix, Role::Tank) => 2,
            (Queue::Role, _, Role::Damage) | (Queue::Role, _, Role::Support) => 2,
        }
    }
}

/// How many more heroes a team can take, per role and in total.
///
/// Handed to everything that has to answer "can this pick land": the derived
/// draft, the enemy board, and the tiles the screen draws. One value rather than
/// two implementations, because a board that disables a click the draft would
/// have accepted — or accepts one the draft then silently drops — is exactly the
/// bug per-role caps invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capacity {
    free: [usize; Role::ALL.len()],
    total: usize,
}

impl Capacity {
    /// An empty team: every slot the format allows.
    pub fn of(format: Format) -> Self {
        let mut free = [0; Role::ALL.len()];
        for role in Role::ALL {
            free[role.index()] = format.slots(role);
        }
        Self {
            free,
            total: format.team_size(),
        }
    }

    /// Whether one more hero of `role` fits.
    ///
    /// `None` is a hero the roster does not know. It still costs a body, so only
    /// the total can refuse it — dropping it outright would silently shrink a
    /// team over a dataset mismatch.
    pub fn fits(&self, role: Option<Role>) -> bool {
        if self.total == 0 {
            return false;
        }
        match role {
            Some(role) => self.free[role.index()] > 0,
            None => true,
        }
    }

    /// Spends a slot, saturating at zero.
    ///
    /// The saturation is load-bearing rather than defensive. Reservations are
    /// charged after the picks they compete with, precisely so that a role
    /// switch mid-draft cannot evict a pick that already landed; the reservation
    /// that no longer has room is absorbed here. Six people in a five-player
    /// room have to read as full for the same reason.
    pub fn take(&mut self, role: Option<Role>) {
        self.total = self.total.saturating_sub(1);
        if let Some(role) = role {
            self.free[role.index()] = self.free[role.index()].saturating_sub(1);
        }
    }

    /// How many of `role` are still open, bounded by the room the team has left
    /// overall — a team with one body left has one slot open, whatever the role
    /// column says.
    pub fn free_in(&self, role: Role) -> usize {
        self.free[role.index()].min(self.total)
    }

    pub fn total_free(&self) -> usize {
        self.total
    }

    pub fn is_full(&self) -> bool {
        self.total == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role_queue(size: TeamSize) -> Format {
        Format::new(size, Queue::Role)
    }

    /// The table has to add up to a team, or the boards would offer a comp that
    /// cannot be fielded. Asserted rather than encoded, so the numbers stay
    /// readable.
    #[test]
    fn a_role_queue_team_is_the_sum_of_its_slots() {
        for size in TeamSize::BOTH {
            let format = role_queue(size);
            let filled: usize = Role::ALL.iter().map(|role| format.slots(*role)).sum();
            assert_eq!(
                filled,
                format.team_size(),
                "{} does not add up",
                size.as_str()
            );
        }
    }

    #[test]
    fn five_v_five_fields_one_tank_and_six_v_six_two() {
        let five = role_queue(TeamSize::FiveVFive);
        assert_eq!(five.slots(Role::Tank), 1);
        assert_eq!(five.slots(Role::Damage), 2);
        assert_eq!(five.slots(Role::Support), 2);

        let six = role_queue(TeamSize::SixVSix);
        assert_eq!(six.slots(Role::Tank), 2);
        assert_eq!(six.slots(Role::Damage), 2);
        assert_eq!(six.slots(Role::Support), 2);
    }

    /// The property the single code path rests on, everywhere it still holds: in
    /// open queue the role cap is the team itself, so it can never be what
    /// refuses a pick. Tanks in 6v6 are the one cell where it does not — see
    /// below.
    #[test]
    fn open_queue_lets_a_whole_team_play_one_role() {
        for size in TeamSize::BOTH {
            let format = Format::new(size, Queue::Open);
            for role in Role::ALL {
                if size == TeamSize::SixVSix && role == Role::Tank {
                    continue;
                }
                assert_eq!(format.slots(role), format.team_size());
            }
        }
    }

    /// 6v6 open queue is the only open queue the game ships as a competitive
    /// playlist, and it keeps a two-tank limit while dropping the damage and
    /// support split. Without this the boards would accept a comp nobody can
    /// queue for.
    #[test]
    fn six_v_six_open_queue_drops_every_role_split_except_the_tank_limit() {
        let format = Format::new(TeamSize::SixVSix, Queue::Open);

        assert_eq!(format.slots(Role::Tank), 2, "two tanks, not six");
        assert_eq!(format.slots(Role::Damage), 6);
        assert_eq!(format.slots(Role::Support), 6);

        let mut room = Capacity::of(format);
        room.take(Some(Role::Tank));
        room.take(Some(Role::Tank));

        assert!(!room.fits(Some(Role::Tank)), "a third tank cannot land");
        assert!(room.fits(Some(Role::Damage)), "but the team is not full");
        assert_eq!(room.free_in(Role::Tank), 0);
        assert_eq!(room.total_free(), 4);
    }

    /// Pinned exactly. These strings are on the wire between clients and in
    /// everybody's stored profile, so a rename here would desync a mixed room
    /// and quietly reset the setting for everyone who upgraded.
    #[test]
    fn a_format_round_trips_through_the_names_it_is_stored_under() {
        let six_open = Format::new(TeamSize::SixVSix, Queue::Open);
        let json = serde_json::to_string(&six_open).expect("a format serialises");
        assert_eq!(json, r#"{"size":"6v6","queue":"open"}"#);
        assert_eq!(
            serde_json::from_str::<Format>(&json).expect("and reads back"),
            six_open
        );
    }

    #[test]
    fn a_format_missing_either_half_reads_as_todays_default() {
        let default = Format::default();
        assert_eq!(default.size, TeamSize::FiveVFive);
        assert_eq!(default.queue, Queue::Role);

        assert_eq!(
            serde_json::from_str::<Format>("{}").expect("an absent format is not an error"),
            default
        );
        assert_eq!(
            serde_json::from_str::<Format>(r#"{"size":"6v6"}"#).expect("half of one is not either"),
            Format::new(TeamSize::SixVSix, Queue::Role),
        );
    }

    #[test]
    fn capacity_refuses_a_role_that_is_full_while_the_others_stay_open() {
        let mut room = Capacity::of(role_queue(TeamSize::FiveVFive));
        room.take(Some(Role::Tank));

        assert!(!room.fits(Some(Role::Tank)), "5v5 fields one tank");
        assert!(room.fits(Some(Role::Damage)));
        assert!(room.fits(Some(Role::Support)));
        assert_eq!(room.free_in(Role::Tank), 0);
        assert_eq!(room.free_in(Role::Damage), 2);
    }

    #[test]
    fn capacity_charges_an_unknown_hero_a_body_but_no_role_slot() {
        let mut room = Capacity::of(role_queue(TeamSize::FiveVFive));
        room.take(None);

        assert_eq!(room.total_free(), 4);
        assert_eq!(room.free_in(Role::Tank), 1, "no role column moved");
    }

    /// A team with one body left cannot take two of anything, whatever the role
    /// column still says.
    #[test]
    fn a_role_column_never_offers_more_than_the_team_has_left() {
        let mut room = Capacity::of(role_queue(TeamSize::SixVSix));
        for _ in 0..5 {
            room.take(None);
        }

        assert_eq!(room.total_free(), 1);
        assert_eq!(room.free_in(Role::Damage), 1, "two slots, one body");
    }

    #[test]
    fn spending_more_than_a_team_has_saturates_rather_than_wrapping() {
        let mut room = Capacity::of(role_queue(TeamSize::FiveVFive));
        for _ in 0..9 {
            room.take(Some(Role::Tank));
        }

        assert!(room.is_full());
        assert_eq!(room.total_free(), 0);
        assert!(!room.fits(Some(Role::Damage)));
        assert!(!room.fits(None));
    }
}
