//! Network-distributed modifier store.
//!
//! Completed research is distributed across a faction's structure network as a
//! "software patch": a set of typed [`Modifier`] contributions that later systems
//! query through a single evaluation API. No system ever stores a pre-multiplied
//! stat, so a tier-3 turret is the same archetype as a tier-1 turret with a
//! different modifier set applied.
//!
//! # Stacking rule (authoritative and deterministic)
//!
//! 1. Every modifier declares a [`ModifierKind`] (what it affects) and a
//!    [`ModifierGroup`] (which stacking bucket it belongs to).
//! 2. **Within a group, contributions add.** `delta_milli` values are summed with
//!    exact integer addition, which is commutative and associative, so insertion
//!    order can never change the sum.
//! 3. **Across groups, buckets multiply.** Group factors are combined as the exact
//!    rational product `prod(1000 + sum_g) / 1000^n`, evaluated in ascending
//!    [`ModifierGroup`] order in `i128` with a single rounding step at the very
//!    end. Integer multiplication is also commutative and associative, so the
//!    result is bit-identical regardless of the order modifiers were inserted.
//! 4. All arithmetic is fixed-point in thousandths (`1000` == `x1.0`,
//!    `+100` == `+10%`). No floating point is used to accumulate, which removes
//!    every source of ordering-dependent rounding drift.
//!
//! Rounding uses round-half-away-from-zero, applied exactly once per query.

use game_types::{FactionId, TechId};
use std::collections::BTreeMap;

/// Fixed-point scale: `1000` represents a neutral `x1.0` multiplier.
pub const MODIFIER_SCALE: i64 = 1000;

/// Category of simulation value a modifier patches.
///
/// Kinds are intentionally declared ahead of the systems that consume them so
/// that later milestones (combat, robots, reinforcement) only have to call
/// [`ModifierStore::value_for`] instead of inventing a new upgrade pipeline.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ModifierKind {
    /// Outgoing weapon damage per shot (combat, Milestone 13).
    WeaponDamage,
    /// Weapon rate of fire (combat, Milestone 13).
    WeaponFireRate,
    /// Weapon accuracy / spread tightening (combat, Milestone 13).
    WeaponAccuracy,
    /// Ore units recovered per mining cycle.
    MiningYield,
    /// Mining drill cycle rate.
    MiningSpeed,
    /// Refinery and fabricator craft rate.
    RefiningSpeed,
    /// Logistics dock service throughput (units moved per tick).
    TransportThroughput,
    /// Powered depot logistics coverage radius.
    LogisticsCoverage,
    /// Robot fabrication speed (robots, Milestone 12/18).
    RobotFabricationSpeed,
    /// Robot fabrication material cost (robots, Milestone 12/18).
    RobotFabricationCost,
    /// Electrical generation output of generators.
    PowerGeneration,
    /// Electrical demand efficiency of consumers (higher == less demand).
    PowerEfficiency,
    /// Hit points restored per unit of repair material.
    RepairRate,
    /// Reinforcement / respawn wave strength (reinforcements, Milestone 21).
    ReinforcementRate,
    /// Structure maximum integrity.
    StructureIntegrity,
    /// Research progress rate.
    ResearchSpeed,
}

/// Every modifier kind in canonical (sorted) order, for iteration and UI.
pub const ALL_MODIFIER_KINDS: &[ModifierKind] = &[
    ModifierKind::WeaponDamage,
    ModifierKind::WeaponFireRate,
    ModifierKind::WeaponAccuracy,
    ModifierKind::MiningYield,
    ModifierKind::MiningSpeed,
    ModifierKind::RefiningSpeed,
    ModifierKind::TransportThroughput,
    ModifierKind::LogisticsCoverage,
    ModifierKind::RobotFabricationSpeed,
    ModifierKind::RobotFabricationCost,
    ModifierKind::PowerGeneration,
    ModifierKind::PowerEfficiency,
    ModifierKind::RepairRate,
    ModifierKind::ReinforcementRate,
    ModifierKind::StructureIntegrity,
    ModifierKind::ResearchSpeed,
];

impl ModifierKind {
    /// Human readable label for debug overlays and reports.
    pub const fn display_name(&self) -> &'static str {
        match self {
            ModifierKind::WeaponDamage => "Weapon Damage",
            ModifierKind::WeaponFireRate => "Weapon Fire Rate",
            ModifierKind::WeaponAccuracy => "Weapon Accuracy",
            ModifierKind::MiningYield => "Mining Yield",
            ModifierKind::MiningSpeed => "Mining Speed",
            ModifierKind::RefiningSpeed => "Refining Speed",
            ModifierKind::TransportThroughput => "Transport Throughput",
            ModifierKind::LogisticsCoverage => "Logistics Coverage",
            ModifierKind::RobotFabricationSpeed => "Robot Fabrication Speed",
            ModifierKind::RobotFabricationCost => "Robot Fabrication Cost",
            ModifierKind::PowerGeneration => "Power Generation",
            ModifierKind::PowerEfficiency => "Power Efficiency",
            ModifierKind::RepairRate => "Repair Rate",
            ModifierKind::ReinforcementRate => "Reinforcement Rate",
            ModifierKind::StructureIntegrity => "Structure Integrity",
            ModifierKind::ResearchSpeed => "Research Speed",
        }
    }
}

/// Stacking bucket. Contributions add within a group and groups multiply.
///
/// The set is deliberately small and fixed so the across-group product can never
/// overflow `i128` and so the evaluation order is a stable, total order.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum ModifierGroup {
    /// Permanent faction-wide software patches from completed research.
    SoftwarePatch = 0,
    /// Infrastructure/efficiency retrofits (kept separate so they compound).
    Efficiency = 1,
    /// Temporary overclocks and power surges.
    Overclock = 2,
    /// Commander doctrine choices (Milestone 22).
    Doctrine = 3,
    /// Localized field auras from support units or structures.
    FieldAura = 4,
    /// Emergency / last-stand effects (Milestone 21).
    Emergency = 5,
}

/// Every stacking group in canonical ascending order.
pub const ALL_MODIFIER_GROUPS: &[ModifierGroup] = &[
    ModifierGroup::SoftwarePatch,
    ModifierGroup::Efficiency,
    ModifierGroup::Overclock,
    ModifierGroup::Doctrine,
    ModifierGroup::FieldAura,
    ModifierGroup::Emergency,
];

impl ModifierGroup {
    pub const fn display_name(&self) -> &'static str {
        match self {
            ModifierGroup::SoftwarePatch => "Software Patch",
            ModifierGroup::Efficiency => "Efficiency",
            ModifierGroup::Overclock => "Overclock",
            ModifierGroup::Doctrine => "Doctrine",
            ModifierGroup::FieldAura => "Field Aura",
            ModifierGroup::Emergency => "Emergency",
        }
    }
}

/// A single typed contribution to the faction-wide modifier network.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Modifier {
    pub kind: ModifierKind,
    pub group: ModifierGroup,
    /// Signed contribution in thousandths. `+100` == `+10%`, `-50` == `-5%`.
    pub delta_milli: i32,
}

impl Modifier {
    /// Declare a software-patch modifier (the standard research contribution).
    pub const fn patch(kind: ModifierKind, delta_milli: i32) -> Self {
        Modifier {
            kind,
            group: ModifierGroup::SoftwarePatch,
            delta_milli,
        }
    }

    /// Declare a modifier in an explicit stacking group.
    pub const fn new(kind: ModifierKind, group: ModifierGroup, delta_milli: i32) -> Self {
        Modifier {
            kind,
            group,
            delta_milli,
        }
    }
}

/// Hard bound on a single stacking bucket's factor (`x10.0`).
///
/// Bounding every bucket keeps the across-group `i128` product well inside range
/// for the fixed six-group set, so evaluation can never overflow or panic even on
/// malformed content data.
const MAX_GROUP_FACTOR_MILLI: i64 = 10_000;

/// Divide with round-half-away-from-zero. Deterministic on every platform and
/// saturating rather than panicking on pathological inputs.
#[inline]
fn div_round(numerator: i128, denominator: i128) -> i128 {
    if denominator == 0 {
        return 0;
    }
    let sign = if (numerator < 0) != (denominator < 0) {
        -1i128
    } else {
        1i128
    };
    let n = numerator.unsigned_abs();
    let d = denominator.unsigned_abs();
    let quotient = n.saturating_mul(2).saturating_add(d) / d.saturating_mul(2);
    sign * quotient.min(i128::MAX as u128) as i128
}

/// Faction-wide modifier network state, replicated to consuming systems as patches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModifierStore {
    /// Per faction, per contributing tech, the raw declared modifiers.
    sources: BTreeMap<FactionId, BTreeMap<TechId, Vec<Modifier>>>,
    /// Per faction, summed `delta_milli` per (kind, group) stacking bucket.
    totals: BTreeMap<FactionId, BTreeMap<(ModifierKind, ModifierGroup), i64>>,
    /// Monotonic patch version, bumped on every mutation.
    version: u64,
}

impl ModifierStore {
    pub fn new() -> Self {
        ModifierStore::default()
    }

    /// Monotonic version identifying the current distributed patch set.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// True when no faction has any contribution registered.
    pub fn is_empty(&self) -> bool {
        self.totals.is_empty()
    }

    /// Factions with at least one registered contribution, in ascending order.
    pub fn factions(&self) -> Vec<FactionId> {
        self.totals.keys().copied().collect()
    }

    /// Register (or replace) the contributions of one source technology.
    pub fn add_source(&mut self, faction: FactionId, source: TechId, modifiers: &[Modifier]) {
        self.sources
            .entry(faction)
            .or_default()
            .insert(source, modifiers.to_vec());
        self.recompute_faction(faction);
        self.version += 1;
    }

    /// Remove a source technology's contributions entirely.
    pub fn remove_source(&mut self, faction: FactionId, source: TechId) -> bool {
        let removed = self
            .sources
            .get_mut(&faction)
            .map(|m| m.remove(&source).is_some())
            .unwrap_or(false);
        if removed {
            self.recompute_faction(faction);
            self.version += 1;
        }
        removed
    }

    /// True if the given source technology currently contributes modifiers.
    pub fn contains_source(&self, faction: FactionId, source: TechId) -> bool {
        self.sources
            .get(&faction)
            .map(|m| m.contains_key(&source))
            .unwrap_or(false)
    }

    /// Number of distinct contributing sources for a faction.
    pub fn source_count(&self, faction: FactionId) -> usize {
        self.sources.get(&faction).map(|m| m.len()).unwrap_or(0)
    }

    /// Discard everything belonging to a faction.
    pub fn clear_faction(&mut self, faction: FactionId) {
        self.sources.remove(&faction);
        self.totals.remove(&faction);
        self.version += 1;
    }

    /// Rebuild the summed stacking buckets for one faction from its raw sources.
    ///
    /// Summation is plain integer addition over a `BTreeMap`, so the result is
    /// independent of the order sources were registered.
    fn recompute_faction(&mut self, faction: FactionId) {
        let mut totals: BTreeMap<(ModifierKind, ModifierGroup), i64> = BTreeMap::new();
        if let Some(by_source) = self.sources.get(&faction) {
            for mods in by_source.values() {
                for m in mods {
                    *totals.entry((m.kind, m.group)).or_insert(0) += m.delta_milli as i64;
                }
            }
        }
        totals.retain(|_, v| *v != 0);
        if totals.is_empty() {
            self.totals.remove(&faction);
        } else {
            self.totals.insert(faction, totals);
        }
    }

    /// Summed contribution of a single (kind, group) stacking bucket, in milli.
    pub fn group_total_milli(
        &self,
        faction: FactionId,
        kind: ModifierKind,
        group: ModifierGroup,
    ) -> i64 {
        self.totals
            .get(&faction)
            .and_then(|m| m.get(&(kind, group)))
            .copied()
            .unwrap_or(0)
    }

    /// Non-zero stacking buckets for a kind, in ascending group order (UI/debug).
    pub fn group_breakdown(
        &self,
        faction: FactionId,
        kind: ModifierKind,
    ) -> Vec<(ModifierGroup, i64)> {
        let mut out = Vec::new();
        if let Some(m) = self.totals.get(&faction) {
            for group in ALL_MODIFIER_GROUPS {
                if let Some(v) = m.get(&(kind, *group))
                    && *v != 0
                {
                    out.push((*group, *v));
                }
            }
        }
        out
    }

    /// All kinds with a non-neutral multiplier for a faction, ascending (UI/debug).
    pub fn active_kinds(&self, faction: FactionId) -> Vec<(ModifierKind, i64)> {
        let mut out = Vec::new();
        for kind in ALL_MODIFIER_KINDS {
            let mult = self.multiplier_milli(faction, *kind);
            if mult != MODIFIER_SCALE {
                out.push((*kind, mult));
            }
        }
        out
    }

    /// Exact rational factor for a (faction, kind) as `(numerator, denominator)`.
    ///
    /// `numerator = prod(1000 + sum_g)` over groups with a non-zero sum, and
    /// `denominator = 1000^n`. Both accumulate with exact `i128` integer
    /// multiplication, so the pair is order-independent by construction.
    fn factor(&self, faction: FactionId, kind: ModifierKind) -> (i128, i128) {
        let mut numerator: i128 = 1;
        let mut denominator: i128 = 1;
        if let Some(m) = self.totals.get(&faction) {
            for group in ALL_MODIFIER_GROUPS {
                if let Some(sum) = m.get(&(kind, *group)) {
                    // A bucket may never drive a value below zero nor above the cap.
                    let factor = (MODIFIER_SCALE + *sum).clamp(0, MAX_GROUP_FACTOR_MILLI) as i128;
                    numerator = numerator.saturating_mul(factor);
                    denominator = denominator.saturating_mul(MODIFIER_SCALE as i128);
                }
            }
        }
        (numerator, denominator)
    }

    /// Combined multiplier in milli. `1000` == neutral `x1.0`.
    pub fn multiplier_milli(&self, faction: FactionId, kind: ModifierKind) -> i64 {
        let (numerator, denominator) = self.factor(faction, kind);
        div_round(
            numerator.saturating_mul(MODIFIER_SCALE as i128),
            denominator,
        )
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Apply the modifier network to a fixed-point base value (single rounding).
    pub fn value_for_milli(&self, faction: FactionId, kind: ModifierKind, base_milli: i64) -> i64 {
        let (numerator, denominator) = self.factor(faction, kind);
        div_round((base_milli as i128).saturating_mul(numerator), denominator)
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Apply the modifier network to an integer base value (single rounding).
    pub fn value_for_u32(&self, faction: FactionId, kind: ModifierKind, base: u32) -> u32 {
        let (numerator, denominator) = self.factor(faction, kind);
        let scaled = div_round((base as i128).saturating_mul(numerator), denominator);
        scaled.clamp(0, u32::MAX as i128) as u32
    }

    /// Apply the modifier network to an integer base value with a minimum floor.
    pub fn value_for_u32_min(
        &self,
        faction: FactionId,
        kind: ModifierKind,
        base: u32,
        floor: u32,
    ) -> u32 {
        self.value_for_u32(faction, kind, base).max(floor)
    }

    /// Apply the modifier network to a float base value.
    ///
    /// The multiplier itself is computed in exact integer arithmetic; only the
    /// final scaling is floating point, so the result cannot depend on the order
    /// modifiers were inserted.
    pub fn value_for(&self, faction: FactionId, kind: ModifierKind, base: f32) -> f32 {
        let mult = self.multiplier_milli(faction, kind);
        base * (mult as f32) / (MODIFIER_SCALE as f32)
    }

    /// Divide a duration by a rate modifier: a `+50%` speed patch makes a
    /// 100-tick job take 67 ticks. Never returns zero.
    pub fn duration_ticks_for(&self, faction: FactionId, kind: ModifierKind, base: u32) -> u32 {
        let (numerator, denominator) = self.factor(faction, kind);
        if numerator == 0 {
            return base.max(1);
        }
        let scaled = div_round((base as i128).saturating_mul(denominator), numerator);
        scaled.clamp(1, u32::MAX as i128) as u32
    }

    /// Replace this store's contents with another patch set (network distribution).
    ///
    /// Returns `true` when the local replica actually changed.
    pub fn install_patch(&mut self, patch: &ModifierStore) -> bool {
        if self.version == patch.version && self.totals == patch.totals {
            return false;
        }
        self.sources = patch.sources.clone();
        self.totals = patch.totals.clone();
        self.version = patch.version;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F1: FactionId = FactionId(1);

    fn techs(n: u32) -> Vec<TechId> {
        (1..=n).map(TechId::new).collect()
    }

    #[test]
    fn test_neutral_store_returns_base_values() {
        let store = ModifierStore::new();
        assert_eq!(store.multiplier_milli(F1, ModifierKind::WeaponDamage), 1000);
        assert_eq!(store.value_for_u32(F1, ModifierKind::MiningYield, 17), 17);
        assert_eq!(store.value_for(F1, ModifierKind::RepairRate, 50.0), 50.0);
        assert_eq!(
            store.duration_ticks_for(F1, ModifierKind::ResearchSpeed, 120),
            120
        );
    }

    #[test]
    fn test_additive_stacking_within_group_exact_values() {
        let mut store = ModifierStore::new();
        let ids = techs(3);
        store.add_source(
            F1,
            ids[0],
            &[Modifier::patch(ModifierKind::WeaponDamage, 100)],
        );
        store.add_source(
            F1,
            ids[1],
            &[Modifier::patch(ModifierKind::WeaponDamage, 50)],
        );
        store.add_source(
            F1,
            ids[2],
            &[Modifier::patch(ModifierKind::WeaponDamage, 25)],
        );

        // +10% +5% +2.5% add within SoftwarePatch => x1.175
        assert_eq!(store.multiplier_milli(F1, ModifierKind::WeaponDamage), 1175);
        assert_eq!(
            store.value_for_u32(F1, ModifierKind::WeaponDamage, 100),
            118
        );
    }

    #[test]
    fn test_multiplicative_stacking_across_groups_exact_values() {
        let mut store = ModifierStore::new();
        store.add_source(
            F1,
            TechId::new(1),
            &[Modifier::new(
                ModifierKind::MiningYield,
                ModifierGroup::SoftwarePatch,
                200,
            )],
        );
        store.add_source(
            F1,
            TechId::new(2),
            &[Modifier::new(
                ModifierKind::MiningYield,
                ModifierGroup::Efficiency,
                500,
            )],
        );

        // 1.2 * 1.5 = 1.8
        assert_eq!(store.multiplier_milli(F1, ModifierKind::MiningYield), 1800);
        assert_eq!(store.value_for_u32(F1, ModifierKind::MiningYield, 10), 18);
    }

    /// ACCEPTANCE: modifier stacking is deterministic (order-independent).
    #[test]
    fn test_acceptance_modifier_stacking_is_order_independent() {
        // A deliberately awkward mix spanning several kinds and stacking groups.
        let catalog: Vec<(TechId, Vec<Modifier>)> = vec![
            (
                TechId::new(11),
                vec![
                    Modifier::patch(ModifierKind::WeaponDamage, 130),
                    Modifier::patch(ModifierKind::MiningYield, 70),
                ],
            ),
            (
                TechId::new(12),
                vec![Modifier::new(
                    ModifierKind::WeaponDamage,
                    ModifierGroup::Doctrine,
                    77,
                )],
            ),
            (
                TechId::new(13),
                vec![
                    Modifier::new(ModifierKind::WeaponDamage, ModifierGroup::Efficiency, 33),
                    Modifier::new(ModifierKind::MiningYield, ModifierGroup::Overclock, 210),
                ],
            ),
            (
                TechId::new(14),
                vec![Modifier::patch(ModifierKind::WeaponDamage, -41)],
            ),
            (
                TechId::new(15),
                vec![
                    Modifier::new(ModifierKind::MiningYield, ModifierGroup::Emergency, 13),
                    Modifier::new(ModifierKind::WeaponDamage, ModifierGroup::FieldAura, 9),
                ],
            ),
        ];

        // Reference order.
        let mut reference = ModifierStore::new();
        for (tech, mods) in &catalog {
            reference.add_source(F1, *tech, mods);
        }

        // Every rotation of the insertion order must produce identical results.
        for rotation in 1..catalog.len() {
            let mut shuffled = ModifierStore::new();
            for offset in 0..catalog.len() {
                let (tech, mods) = &catalog[(offset + rotation) % catalog.len()];
                shuffled.add_source(F1, *tech, mods);
            }

            for kind in ALL_MODIFIER_KINDS {
                assert_eq!(
                    reference.multiplier_milli(F1, *kind),
                    shuffled.multiplier_milli(F1, *kind),
                    "multiplier differs for {kind:?} at rotation {rotation}"
                );
                assert_eq!(
                    reference.value_for_milli(F1, *kind, 987_654),
                    shuffled.value_for_milli(F1, *kind, 987_654),
                    "fixed-point value differs for {kind:?} at rotation {rotation}"
                );
                // Bit-identical floating point results, not merely approximate.
                assert_eq!(
                    reference.value_for(F1, *kind, 1234.5678).to_bits(),
                    shuffled.value_for(F1, *kind, 1234.5678).to_bits(),
                    "float bits differ for {kind:?} at rotation {rotation}"
                );
            }
        }

        // Also pin the exact expected value so a silent rule change fails loudly.
        // WeaponDamage: patch (130 - 41) = +89 -> 1089; Efficiency 1033;
        // Doctrine 1077; FieldAura 1009.
        // 1089 * 1033 * 1077 * 1009 / 1000^4 = 1.222461... -> 1222 (half away from zero)
        assert_eq!(
            reference.multiplier_milli(F1, ModifierKind::WeaponDamage),
            1222
        );
        // MiningYield: patch 1070, Overclock 1210, Emergency 1013 -> 1.311531 -> 1312
        assert_eq!(
            reference.multiplier_milli(F1, ModifierKind::MiningYield),
            1312
        );
    }

    #[test]
    fn test_reversed_insertion_matches_forward_insertion() {
        let mods_a = [Modifier::patch(ModifierKind::PowerGeneration, 250)];
        let mods_b = [Modifier::new(
            ModifierKind::PowerGeneration,
            ModifierGroup::Efficiency,
            125,
        )];
        let mods_c = [Modifier::patch(ModifierKind::PowerGeneration, -75)];

        let mut forward = ModifierStore::new();
        forward.add_source(F1, TechId::new(1), &mods_a);
        forward.add_source(F1, TechId::new(2), &mods_b);
        forward.add_source(F1, TechId::new(3), &mods_c);

        let mut backward = ModifierStore::new();
        backward.add_source(F1, TechId::new(3), &mods_c);
        backward.add_source(F1, TechId::new(2), &mods_b);
        backward.add_source(F1, TechId::new(1), &mods_a);

        assert_eq!(
            forward.multiplier_milli(F1, ModifierKind::PowerGeneration),
            backward.multiplier_milli(F1, ModifierKind::PowerGeneration)
        );
        // (1000 + 250 - 75) = 1175, Efficiency 1125 -> 1.175 * 1.125 = 1.321875 -> 1322
        assert_eq!(
            forward.multiplier_milli(F1, ModifierKind::PowerGeneration),
            1322
        );
    }

    #[test]
    fn test_factions_are_isolated() {
        let mut store = ModifierStore::new();
        store.add_source(
            FactionId::new(1),
            TechId::new(1),
            &[Modifier::patch(ModifierKind::RepairRate, 500)],
        );
        assert_eq!(
            store.multiplier_milli(FactionId::new(1), ModifierKind::RepairRate),
            1500
        );
        assert_eq!(
            store.multiplier_milli(FactionId::new(2), ModifierKind::RepairRate),
            1000
        );
    }

    #[test]
    fn test_remove_source_reverts_contribution() {
        let mut store = ModifierStore::new();
        store.add_source(
            F1,
            TechId::new(1),
            &[Modifier::patch(ModifierKind::MiningSpeed, 300)],
        );
        store.add_source(
            F1,
            TechId::new(2),
            &[Modifier::patch(ModifierKind::MiningSpeed, 200)],
        );
        assert_eq!(store.multiplier_milli(F1, ModifierKind::MiningSpeed), 1500);

        assert!(store.remove_source(F1, TechId::new(2)));
        assert_eq!(store.multiplier_milli(F1, ModifierKind::MiningSpeed), 1300);
        assert!(!store.remove_source(F1, TechId::new(2)));

        assert!(store.remove_source(F1, TechId::new(1)));
        assert_eq!(store.multiplier_milli(F1, ModifierKind::MiningSpeed), 1000);
        assert!(store.is_empty());
    }

    #[test]
    fn test_negative_modifier_cannot_drive_value_below_zero() {
        let mut store = ModifierStore::new();
        store.add_source(
            F1,
            TechId::new(1),
            &[Modifier::patch(ModifierKind::RobotFabricationCost, -5000)],
        );
        assert_eq!(
            store.multiplier_milli(F1, ModifierKind::RobotFabricationCost),
            0
        );
        assert_eq!(
            store.value_for_u32(F1, ModifierKind::RobotFabricationCost, 100),
            0
        );
    }

    #[test]
    fn test_duration_ticks_shrink_with_speed_modifier() {
        let mut store = ModifierStore::new();
        store.add_source(
            F1,
            TechId::new(1),
            &[Modifier::patch(ModifierKind::ResearchSpeed, 500)],
        );
        // 120 ticks at x1.5 speed = 80 ticks
        assert_eq!(
            store.duration_ticks_for(F1, ModifierKind::ResearchSpeed, 120),
            80
        );
        // Never collapses to an instant job
        assert_eq!(
            store.duration_ticks_for(F1, ModifierKind::ResearchSpeed, 1),
            1
        );
    }

    #[test]
    fn test_version_increments_and_patch_installs() {
        let mut authoritative = ModifierStore::new();
        let v0 = authoritative.version();
        authoritative.add_source(
            F1,
            TechId::new(1),
            &[Modifier::patch(ModifierKind::TransportThroughput, 400)],
        );
        assert!(authoritative.version() > v0);

        let mut replica = ModifierStore::new();
        assert!(replica.install_patch(&authoritative));
        assert_eq!(
            replica.multiplier_milli(F1, ModifierKind::TransportThroughput),
            1400
        );
        // Re-installing the identical patch is a no-op.
        assert!(!replica.install_patch(&authoritative));
    }

    #[test]
    fn test_group_breakdown_is_sorted_and_non_zero() {
        let mut store = ModifierStore::new();
        store.add_source(
            F1,
            TechId::new(1),
            &[
                Modifier::new(ModifierKind::WeaponFireRate, ModifierGroup::Emergency, 20),
                Modifier::new(
                    ModifierKind::WeaponFireRate,
                    ModifierGroup::SoftwarePatch,
                    50,
                ),
                Modifier::new(ModifierKind::WeaponFireRate, ModifierGroup::Doctrine, 30),
            ],
        );
        let breakdown = store.group_breakdown(F1, ModifierKind::WeaponFireRate);
        assert_eq!(
            breakdown,
            vec![
                (ModifierGroup::SoftwarePatch, 50),
                (ModifierGroup::Doctrine, 30),
                (ModifierGroup::Emergency, 20),
            ]
        );
    }

    #[test]
    fn test_div_round_half_away_from_zero() {
        assert_eq!(div_round(5, 2), 3);
        assert_eq!(div_round(-5, 2), -3);
        assert_eq!(div_round(4, 2), 2);
        assert_eq!(div_round(1, 3), 0);
        assert_eq!(div_round(7, 0), 0);
    }
}
