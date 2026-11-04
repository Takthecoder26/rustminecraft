//! Immutable, thread-shareable mob tick inputs. Native callbacks stay on the
//! coordinator and are never hidden inside a shared world reference.
use super::*;

#[derive(Clone)]
pub struct TickResources<'a> {
    pub world: &'a World,
    pub registry: &'a horizon_world::registry::BlockRegistry,
    pub facts: &'a BlockFacts,
    pub fluid_tables: &'a crate::motion::FluidResources,
    pub movement: &'a horizon_world::movement_properties::BoundMovementProperties,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub data: &'a MobData,
    pub player: Option<PlayerView>,
    pub controlling_player_movement: Option<Vec3>,
    pub nearest_player: Option<Vec3>,
    pub dragon: Option<(u32, TargetInfo)>,
    pub leash_boats: &'a [(u32, leash::HolderState)],
    pub daylight: bool,
    pub monsters_burn: bool,
    pub game_time: i64,
    pub day_time: i64,
    pub raining: bool,
    pub thundering: bool,
    pub nether: bool,
    pub block_tags: &'a HashMap<String, std::collections::HashSet<String>>,
    pub landing: &'a horizon_world::block_landing::BlockLandingRegistry,
    pub biomes: &'a crate::biome::Biomes,
    pub mob_griefing: bool,
    pub universal_anger: bool,
    pub mob_drops: bool,
    pub max_entity_cramming: i32,
    pub difficulty: spawning::LocalDifficulty,
    pub end: bool,
    pub world_seed: i64,
    pub sea_level: i32,
    pub teams: std::sync::Arc<Teams<'a>>,
    pub items: Option<&'a crate::items::Items>,
    pub lid_openers: &'a HashMap<(i32, i32, i32), i32>,
    pub border: Option<&'a horizon_core::geometry::border::BorderGeometry>,
}
impl<'a> TickResources<'a> {
    pub fn new(world: &MobWorld<'a>) -> Self {
        Self {
            world: world.world,
            registry: world.registry,
            facts: world.facts,
            fluid_tables: world.fluid_tables,
            movement: world.movement,
            collision: world.collision,
            data: world.data,
            player: world.player.clone(),
            controlling_player_movement: world.controlling_player_movement,
            nearest_player: world.nearest_player,
            dragon: world.dragon,
            leash_boats: world.leash_boats,
            daylight: world.daylight,
            monsters_burn: world.monsters_burn,
            game_time: world.game_time,
            day_time: world.day_time,
            raining: world.raining,
            thundering: world.thundering,
            nether: world.nether,
            block_tags: world.block_tags,
            landing: world.landing,
            biomes: world.biomes,
            mob_griefing: world.mob_griefing,
            universal_anger: world.universal_anger,
            mob_drops: world.mob_drops,
            max_entity_cramming: world.max_entity_cramming,
            difficulty: world.difficulty,
            end: world.end,
            world_seed: world.world_seed,
            sea_level: world.sea_level,
            teams: world.teams.clone(),
            items: world.items,
            lid_openers: world.lid_openers,
            border: world.border,
        }
    }
    pub fn view<'s>(
        &'s self,
        others: snapshot::SeenView<'s>,
        grid: &'s dyn seen_grid::SnapshotGrid,
        index: &'s crate::fast_hash::FxHashMap<u32, u32>,
    ) -> MobWorld<'s> {
        MobWorld {
            others,
            others_grid: Some(grid),
            others_index: Some(index),
            mod_effects: None,
            world: self.world,
            registry: self.registry,
            facts: self.facts,
            fluid_tables: self.fluid_tables,
            movement: self.movement,
            collision: self.collision,
            data: self.data,
            player: self.player.clone(),
            controlling_player_movement: self.controlling_player_movement,
            nearest_player: self.nearest_player,
            dragon: self.dragon,
            leash_boats: self.leash_boats,
            daylight: self.daylight,
            monsters_burn: self.monsters_burn,
            game_time: self.game_time,
            day_time: self.day_time,
            raining: self.raining,
            thundering: self.thundering,
            nether: self.nether,
            block_tags: self.block_tags,
            landing: self.landing,
            biomes: self.biomes,
            mob_griefing: self.mob_griefing,
            universal_anger: self.universal_anger,
            mob_drops: self.mob_drops,
            max_entity_cramming: self.max_entity_cramming,
            difficulty: self.difficulty,
            end: self.end,
            world_seed: self.world_seed,
            sea_level: self.sea_level,
            teams: self.teams.clone(),
            items: self.items,
            lid_openers: self.lid_openers,
            border: self.border,
        }
    }
}

// This compile-time contract rejects thread-affine caches added to immutable
// world inputs. Actual state mutation belongs to a region-owned transaction.
fn _shareable(resources: TickResources<'_>) {
    fn require<T: Send + Sync>(_: T) {}
    require(resources);
}

/// A region's immutable sources, lent until the worker barrier. Each worker
/// creates its exclusively owned mutable inputs; the coordinator retains the
/// originals for ordered replay. World/chunks are never cloned.
pub struct RegionInput<'a> {
    pub owner: usize,
    pub mobs: Vec<(usize, &'a Mob)>,
}

#[derive(Clone)]
pub struct MobContext {
    pub bound_player: Option<u32>,
    pub nearest_player: Option<Vec3>,
    pub difficulty: spawning::LocalDifficulty,
    pub section_order: u64,
}

pub struct TickPrediction {
    pub slot: usize,
    pub owner: usize,
    pub mob: Mob,
    pub kept: bool,
    pub events: Vec<MobEvent>,
    pub reads: snapshot::ReadSet,
    pub random_before: LegacyRandom,
    pub random_after: LegacyRandom,
}
impl TickPrediction {
    pub fn rng_valid(&self, current: &LegacyRandom) -> bool {
        !self.random_after.was_used() || current.same_stream(&self.random_before)
    }
}

pub struct RegionTick<'a> {
    pub resources: &'a TickResources<'a>,
    pub snapshot: &'a [Seen],
    pub grid: &'a seen_grid::SeenGrid,
    pub species: &'a snapshot::SpeciesIndex,
    pub index: &'a crate::fast_hash::FxHashMap<u32, u32>,
    pub contexts: &'a [MobContext],
    pub players: &'a players::PlayerDirectory<'a>,
    pub path_changes: &'a [[i32; 3]],
    pub random: &'a LegacyRandom,
    pub player_uuid: Option<[i32; 4]>,
    pub tnt_explodes: bool,
}
impl RegionTick<'_> {
    fn predict(&self, input: RegionInput<'_>) -> Vec<TickPrediction> {
        use std::cell::RefCell;
        // This rule was historically thread-local. Set it per level job, not
        // just when a persistent worker starts (different levels may differ).
        sulfur_cube::set_tnt_explodes(self.tnt_explodes);
        let mut overlay = snapshot::SnapshotOverlay::default();
        let mut grid = seen_grid::GridOverlay::new(self.grid);
        let mut pending = PendingPushes::default();
        let mut random = self.random.clone();
        let mut out = Vec::with_capacity(input.mobs.len());
        let owned_slots: crate::fast_hash::FxHashMap<_, _> = input
            .mobs
            .iter()
            .map(|(slot, mob)| (mob.id, *slot))
            .collect();
        let mut pending_brains: crate::fast_hash::FxHashMap<u32, Vec<MobEvent>> =
            Default::default();
        for (slot, original) in input.mobs {
            // Active raiders' hurt ordering still uses a global sequence. No
            // speculative execution may consume it, including rejected work.
            if raider::is_raider(original.species()) || original.extra.raider.is_some() {
                break;
            }
            let mut mob = original.clone();
            if let Some(mut events) = pending_brains.remove(&mob.id) {
                apply_brain_events(std::slice::from_mut(&mut mob), &mut events, 0);
            }
            apply_mob_pushes(&mut mob, &mut pending);
            blocks_changed(&mut mob, self.path_changes);
            let reads = RefCell::new(snapshot::ReadSet::default());
            let mut events = Vec::new();
            let random_before = random.clone();
            random.clear_usage();
            let kept = {
                let context = &self.contexts[slot];
                let mut world = self.resources.view(
                    snapshot::SeenView::observed(self.snapshot, &overlay, &reads)
                        .with_species(self.species),
                    &grid,
                    self.index,
                );
                let conn = self
                    .players
                    .player_for(input.owner, &mob, context.bound_player);
                world.player = conn
                    .and_then(|conn| self.players.guest(conn).cloned())
                    .or_else(|| self.resources.player.clone());
                world.controlling_player_movement = self.players.controlling_movement(&mob);
                world.nearest_player = context.nearest_player;
                world.difficulty = context.difficulty;
                mob.tick_with_order(&world, &mut random, &mut events, context.section_order)
            };
            // Preserve push order while keeping the original events intact for
            // the coordinator's authoritative global-order application.
            for event in &events {
                if let MobEvent::Push {
                    target: Who::Mob(id),
                    velocity,
                } = event
                {
                    pending.push(*id, Vec3::new(velocity[0], velocity[1], velocity[2]));
                }
            }
            let mut current = mob.seen(self.player_uuid);
            current.alive &= kept;
            grid.update(slot, current.bounds);
            overlay.update(slot, current);
            // Replay known single-entity brain writes into the corresponding
            // future owned inputs, preserving their original event order.
            let mut writes = if events.is_empty() {
                BrainWrites::default()
            } else {
                apply_brain_events(&mut [], &mut events.clone(), 0)
            };
            writes.targets.sort_unstable();
            writes.targets.dedup();
            for target in writes.targets {
                if owned_slots
                    .get(&target)
                    .is_some_and(|&target_slot| target_slot > slot)
                {
                    pending_brains
                        .entry(target)
                        .or_default()
                        .extend(events.iter().cloned());
                }
            }
            let global_conflict = writes.global;
            out.push(TickPrediction {
                slot,
                owner: input.owner,
                mob,
                kept,
                events,
                reads: reads.into_inner(),
                random_before,
                random_after: random.clone(),
            });
            if global_conflict {
                break;
            }
        }
        out
    }
}

/// One persistent bounded pool per server, shared by all of its levels. The
/// install call lends immutable tick data safely; it joins all region jobs
/// before returning predictions or propagating a worker panic.
pub struct RegionExecutor {
    pool: Option<rayon::ThreadPool>,
}
impl RegionExecutor {
    pub fn new(workers: usize) -> Result<Self, rayon::ThreadPoolBuildError> {
        if workers <= 1 {
            return Ok(Self { pool: None });
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers.clamp(1, 8))
            .thread_name(|index| format!("Horizon region {index}"))
            .build()?;
        Ok(Self { pool: Some(pool) })
    }
    pub fn predict(
        &self,
        inputs: Vec<RegionInput<'_>>,
        tick: &RegionTick<'_>,
    ) -> Vec<Vec<TickPrediction>> {
        use rayon::prelude::*;
        match &self.pool {
            Some(pool) => pool.install(|| {
                inputs
                    .into_par_iter()
                    .map(|input| tick.predict(input))
                    .collect()
            }),
            None => inputs
                .into_iter()
                .map(|input| tick.predict(input))
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RegionStats {
    pub regions: usize,
    pub predicted: usize,
    pub committed: usize,
    pub replayed: usize,
    pub rng_conflicts: usize,
    pub read_conflicts: usize,
    pub read_full: usize,
    pub read_slot: usize,
    pub read_shape: usize,
    pub read_cell: usize,
    pub read_species: usize,
}

/// The authoritative ordered commit. A failed region prefix can never provide
/// another prediction this tick. Conflicts replay against the live snapshot
/// and RNG, before any dependent entity or event is published.
pub struct TickRound {
    predictions: Vec<Option<TickPrediction>>,
    owners: Vec<usize>,
    failed: Vec<bool>,
    changes: snapshot::SnapshotChanges,
    pub stats: RegionStats,
    section_base: u64,
}
impl TickRound {
    pub fn new(
        owners: Vec<usize>,
        predictions: Vec<Vec<TickPrediction>>,
        section_base: u64,
    ) -> Self {
        let active_regions = predictions
            .iter()
            .filter(|predictions| !predictions.is_empty())
            .count();
        let regions = owners.iter().copied().max().map_or(0, |owner| owner + 1);
        let mut slots: Vec<_> = (0..owners.len()).map(|_| None).collect();
        let mut predicted = 0;
        for prediction in predictions.into_iter().flatten() {
            let slot = prediction.slot;
            assert_eq!(owners[slot], prediction.owner, "prediction owner");
            assert!(slots[slot].is_none(), "one prediction per entity");
            slots[slot] = Some(prediction);
            predicted += 1;
        }
        Self {
            changes: snapshot::SnapshotChanges::new(owners.clone()),
            owners,
            failed: vec![false; regions],
            predictions: slots,
            section_base,
            stats: RegionStats {
                regions: active_regions,
                predicted,
                ..Default::default()
            },
        }
    }
    pub fn tick(
        &mut self,
        slot: usize,
        mob: &mut Mob,
        world: &MobWorld<'_>,
        grid: &seen_grid::SeenGrid,
        random: &mut LegacyRandom,
        events: &mut Vec<MobEvent>,
    ) -> bool {
        let owner = self.owners[slot];
        if let Some(prediction) = self.predictions[slot].take()
            && !self.failed[owner]
        {
            let rng_valid = prediction.rng_valid(random);
            let read_conflict = self.changes.conflict(&prediction.reads, owner, grid);
            if rng_valid && read_conflict.is_none() {
                *mob = prediction.mob;
                if prediction.random_after.was_used() {
                    *random = prediction.random_after;
                }
                events.extend(prediction.events);
                self.stats.committed += 1;
                return prediction.kept;
            }
            self.stats.rng_conflicts += usize::from(!rng_valid);
            if let Some(conflict) = read_conflict {
                self.stats.read_conflicts += 1;
                match conflict {
                    snapshot::ReadConflict::FullScan => self.stats.read_full += 1,
                    snapshot::ReadConflict::EntitySlot => self.stats.read_slot += 1,
                    snapshot::ReadConflict::GridShape => self.stats.read_shape += 1,
                    snapshot::ReadConflict::SpatialCell => self.stats.read_cell += 1,
                    snapshot::ReadConflict::Species => self.stats.read_species += 1,
                }
            }
        }
        self.failed[owner] = true;
        self.stats.replayed += 1;
        mob.tick_with_order(
            world,
            random,
            events,
            self.section_base.wrapping_add(slot as u64),
        )
    }
    pub fn snapshot_changed(&mut self, slot: usize, before: &Seen, after: &Seen) {
        self.changes.touch(slot, before.bounds, after.bounds);
        self.changes
            .species_changed(slot, before.species, after.species);
    }
    pub fn despawn_removed(&mut self, slot: usize, bounds: Aabb, species: Species) {
        // Non-entity-ticking chunks run their cheap despawn check serially;
        // workers therefore cannot rely on that owner's predicted prefix.
        self.failed[self.owners[slot]] = true;
        self.changes.touch(slot, bounds, bounds);
        self.changes.species_changed(slot, species, species);
    }
    pub fn events(
        &mut self,
        slot: usize,
        events: &[MobEvent],
        index: &crate::fast_hash::FxHashMap<u32, u32>,
    ) {
        let owner = self.owners[slot];
        for event in events {
            if let MobEvent::Push {
                target: Who::Mob(id),
                ..
            } = event
                && let Some(&target) = index.get(id)
            {
                let target_owner = self.owners[target as usize];
                if target_owner != owner {
                    self.failed[target_owner] = true;
                }
            }
        }
    }
    pub fn immediate_brain_write(
        &mut self,
        source: usize,
        writes: &BrainWrites,
        index: &crate::fast_hash::FxHashMap<u32, u32>,
    ) {
        if writes.global {
            self.failed.fill(true);
            return;
        }
        let owner = self.owners[source];
        for target in &writes.targets {
            if let Some(&slot) = index.get(target) {
                let target_owner = self.owners[slot as usize];
                if target_owner != owner {
                    self.failed[target_owner] = true;
                }
            }
        }
    }
}

/// Per-level EntitySection insertion order. Reserving a slot for every entity
/// keeps relative order correct even if an earlier immediate interaction makes
/// a later mob change sections. Unused reservations are harmless gaps.
pub struct SectionOrder {
    next: u64,
}
impl Default for SectionOrder {
    fn default() -> Self {
        Self { next: 1 }
    }
}
impl SectionOrder {
    pub fn reserve(&mut self, mobs: &[Mob]) -> u64 {
        let after_existing = mobs
            .iter()
            .map(|mob| mob.extra.section_seq)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        let start = self.next.max(after_existing);
        self.next = start.wrapping_add(mobs.len() as u64);
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fast_hash::FxHashMap;
    use std::sync::Arc;

    fn advance(
        mobs: &mut Vec<Mob>,
        base: &MobWorld<'_>,
        random: &mut LegacyRandom,
        executor: Option<&RegionExecutor>,
    ) -> (Vec<MobEvent>, RegionStats) {
        let section_base = mobs
            .iter()
            .map(|mob| mob.extra.section_seq)
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        let mut snapshot: Vec<_> = mobs.iter().map(|mob| mob.seen(None)).collect();
        let mut grid = seen_grid::SeenGrid::new(snapshot.iter().map(|seen| seen.bounds));
        let species_index = snapshot::SpeciesIndex::new(&snapshot);
        let index: FxHashMap<_, _> = mobs
            .iter()
            .enumerate()
            .map(|(slot, mob)| (mob.id, slot as u32))
            .collect();
        let contexts: Vec<_> = mobs
            .iter()
            .enumerate()
            .map(|(slot, _)| MobContext {
                bound_player: None,
                nearest_player: base.nearest_player,
                difficulty: base.difficulty,
                section_order: section_base.wrapping_add(slot as u64),
            })
            .collect();
        let player_directory = players::PlayerDirectory::new(base.player.as_ref(), &[]);
        let mut round = executor.map(|executor| {
            let owners: Vec<_> = mobs
                .iter()
                .map(|mob| {
                    if mob.id <= 4 {
                        (mob.id as usize - 1) % 2
                    } else {
                        2
                    }
                })
                .collect();
            let inputs = (0..3)
                .map(|owner| RegionInput {
                    owner,
                    mobs: mobs
                        .iter()
                        .enumerate()
                        .filter(|&(slot, _)| owners[slot] == owner)
                        .collect(),
                })
                .collect();
            let resources = TickResources::new(base);
            let tick = RegionTick {
                resources: &resources,
                snapshot: &snapshot,
                grid: &grid,
                species: &species_index,
                index: &index,
                contexts: &contexts,
                players: &player_directory,
                path_changes: &[],
                random,
                player_uuid: None,
                tnt_explodes: true,
            };
            TickRound::new(owners, executor.predict(inputs, &tick), section_base)
        });
        let mut events = Vec::new();
        let mut pushes = PendingPushes::default();
        let mut keep = Vec::new();
        for slot in 0..mobs.len() {
            let world = MobWorld {
                others: snapshot::SeenView::from(&snapshot).with_species(&species_index),
                others_grid: executor.map(|_| &grid as &dyn seen_grid::SnapshotGrid),
                others_index: Some(&index),
                ..base.clone()
            };
            let mob = &mut mobs[slot];
            apply_mob_pushes(mob, &mut pushes);
            let from = events.len();
            let kept = match round.as_mut() {
                Some(round) => round.tick(slot, mob, &world, &grid, random, &mut events),
                None => mob.tick_with_order(
                    &world,
                    random,
                    &mut events,
                    section_base.wrapping_add(slot as u64),
                ),
            };
            if let Some(round) = round.as_mut() {
                round.events(slot, &events[from..], &index);
            }
            take_mob_pushes(&mut events, from, &mut pushes);
            let mut after = mob.seen(None);
            after.alive &= kept;
            let before = std::mem::replace(&mut snapshot[slot], after);
            grid.update(slot, snapshot[slot].bounds);
            if let Some(round) = round.as_mut() {
                round.snapshot_changed(slot, &before, &snapshot[slot]);
            }
            let changed = apply_brain_events(mobs, &mut events, from);
            if let Some(round) = round.as_mut() {
                round.immediate_brain_write(slot, &changed, &index);
            }
            keep.push(kept);
        }
        for mob in mobs.iter_mut() {
            apply_mob_pushes(mob, &mut pushes);
        }
        let mut keep = keep.into_iter();
        mobs.retain(|_| keep.next().unwrap());
        (
            events,
            round.map_or_else(Default::default, |round| round.stats),
        )
    }

    fn assert_same_mobs(
        actual: &[Mob],
        expected: &[Mob],
        species: &str,
        near: bool,
        tick: usize,
        mode: &str,
    ) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            let a = format!("{actual:?}");
            let b = format!("{expected:?}");
            if a != b {
                let offset = a
                    .bytes()
                    .zip(b.bytes())
                    .position(|(a, b)| a != b)
                    .unwrap_or(a.len().min(b.len()));
                let lo = offset.saturating_sub(100);
                let hi_a = (offset + 180).min(a.len());
                let hi_b = (offset + 180).min(b.len());
                panic!(
                    "{mode} {species}, near={near}, tick={tick}, mob={}: first difference at {offset}\nactual: {}\nserial: {}",
                    actual.id,
                    &a[lo..hi_a],
                    &b[lo..hi_b]
                );
            }
        }
    }

    #[test]
    fn full_mob_transactions_match_serial_for_independence_and_conflicts() {
        // Existing resource-backed mob tests use this helper and skip when the
        // owner's prepared vanilla data is unavailable. No substitute oracle
        // or weakened expected values are used when resources are present.
        let ran = super::super::world::with_test_world(&[], |world, types| {
            let serial_regions = RegionExecutor::new(1).unwrap();
            let parallel = RegionExecutor::new(3).unwrap();
            for (species, near, breeding) in [
                ("cow", false, false),
                ("cow", true, false),
                ("cow", false, true),
                ("cow", true, true),
                ("zombie", false, false),
                ("zombie", true, false),
                ("villager", false, false),
                ("bee", false, false),
            ] {
                let kind = Arc::new(types.types[&format!("minecraft:{species}")].clone());
                let dummy = super::super::sensor_scan_tests::kind();
                let mut initial = Vec::new();
                for id in 1..=132 {
                    let mut mob = Mob::new(
                        id,
                        if id <= 4 { kind.clone() } else { dummy.clone() },
                        Vec3::ZERO,
                        0.0,
                    );
                    mob.persistent = true;
                    mob.no_gravity = true;
                    mob.position.y = 80.0;
                    if id <= 4 {
                        let owner = (id - 1) % 2;
                        mob.position.x = if near {
                            f64::from(id) * 0.1
                        } else {
                            f64::from(owner) * 1024.0 + f64::from(id / 2) * 0.1
                        };
                    } else {
                        mob.position.x = 4096.0 + f64::from(id) * 8.0;
                        mob.no_ai = true;
                    }
                    mob.previous = mob.position;
                    if id <= 4 && breeding {
                        mob.extra.in_love = 600;
                    }
                    if id <= 4 && species == "bee" {
                        bee::set_has_nectar(&mut mob, true);
                    }
                    initial.push(mob);
                }
                let mut expected = initial.clone();
                let mut serial = initial.clone();
                let mut concurrent = initial;
                let mut expected_rng = LegacyRandom::new(1234);
                let mut serial_rng = expected_rng.clone();
                let mut concurrent_rng = expected_rng.clone();
                let mut accepted = 0;
                let mut replayed = 0;
                for tick in 0..if breeding { 80 } else { 24 } {
                    let (events, _) = advance(&mut expected, world, &mut expected_rng, None);
                    let (serial_events, _) =
                        advance(&mut serial, world, &mut serial_rng, Some(&serial_regions));
                    let (parallel_events, stats) =
                        advance(&mut concurrent, world, &mut concurrent_rng, Some(&parallel));
                    assert_same_mobs(&serial, &expected, species, near, tick, "serial");
                    assert_same_mobs(&concurrent, &expected, species, near, tick, "parallel");
                    assert_eq!(format!("{serial_events:?}"), format!("{events:?}"));
                    assert_eq!(format!("{parallel_events:?}"), format!("{events:?}"));
                    assert!(serial_rng.same_stream(&expected_rng));
                    assert!(concurrent_rng.same_stream(&expected_rng));
                    accepted += stats.committed;
                    replayed += stats.replayed;
                }
                assert!(accepted > 0, "actual predictions must be committed");
                if species == "bee" {
                    assert!(
                        !expected_rng.same_stream(&LegacyRandom::new(1234)),
                        "nectar drips must exercise the shared level RNG"
                    );
                }
                if matches!(species, "cow" | "zombie") && !near {
                    assert_eq!(
                        replayed, 0,
                        "independent real mob ticks should commit without replay"
                    );
                }
                if near {
                    assert!(replayed > 0, "interacting owners must exercise replay");
                }
            }
        });
        if !ran {
            eprintln!(
                "prepared vanilla resources unavailable; resource-backed transaction test skipped"
            );
        }
    }
}
