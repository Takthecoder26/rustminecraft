//! Mutable level simulation, separate from window state, immutable Resources,
//! and server-wide scheduling. Dimension swaps move world data, entity families,
//! tick queues, bookkeeping and replication buffers together as one owner.
use super::*;
use super::{mob_index::MobIndex, natural_spawning::SpawnScratch};
use horizon_client::{
    fast_hash::FxHashSet,
    join_seq::JoinList,
    server_regions::{Regions, Workers},
};

pub(super) struct LevelSimulation {
    pub armor_stands: horizon_client::join_seq::JoinList<horizon_client::entities::ArmorStand>,
    pub block_ticks: Ticks,
    pub boats: horizon_client::join_seq::JoinList<boats::Boat>,
    pub bolts: Vec<lightning::Bolt>,
    pub breath_clouds: Vec<dragon::Breath>,
    pub carts: horizon_client::join_seq::JoinList<minecarts::Minecart>,
    pub crystals: Vec<dragon::Crystal>,
    /// `Cushion` decoration entities (game/cushions.rs).
    pub cushions: Vec<horizon_client::entities::Cushion>,
    pub dragon: Option<dragon::Dragon>,
    pub dragon_fireballs: Vec<dragon::DragonFireball>,
    pub dropped: horizon_client::join_seq::JoinList<ItemEntity>,
    pub falling: horizon_client::join_seq::JoinList<FallingBlock>,
    pub fangs: fangs::FangState,
    pub hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    pub item_frames: Vec<horizon_client::entities::ItemFrame>,
    /// The entity ids a hosted server gives the knots its mobs hang on, by
    /// block, while a lead is tied there.
    pub knot_ids: HashMap<[i32; 3], u32>,
    /// Block events (position, b0, b1, state) and entity events (id, event,
    /// position) for the guests.
    pub lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    /// Successful crits (attacking guest, hit entity), queued while the hub is taken.
    pub lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    pub lan_entity_events: Vec<(i32, i8, [f64; 3])>,
    /// Level events of this tick for the guests, and the guest acting now.
    pub lan_events: Vec<(i32, [i32; 3], i32, Option<u32>)>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    pub lectern_pulses: Vec<((i32, i32, i32), i32)>,
    pub level_events: Vec<LevelEvent>,
    /// `ContainerOpenersCounter`: how many players (the host and the guests)
    /// have each chest open.
    pub lid_openers: HashMap<(i32, i32, i32), i32>,
    /// Ominous trial spawners' item spawners.
    pub ominous_spawners: Vec<ominous_item_spawner::OminousItemSpawner>,
    pub orbs: horizon_client::join_seq::JoinList<ExperienceOrb>,
    /// Entities of types Horizon does not simulate.
    pub others: Vec<commands::OtherEntity>,
    pub paintings: Vec<horizon_client::entities::Painting>,
    pub persist: persist::EntityMemory,
    pub rockets: horizon_client::join_seq::JoinList<fireworks::Rocket>,
    pub tick_chunks: horizon_client::tick_chunk_membership::TickChunkMembership,
    pub tickets: horizon_client::ticket_levels::ChunkTickets,
    pub tnts: horizon_client::join_seq::JoinList<tnt::PrimedTnt>,
    pub tridents: horizon_client::join_seq::JoinList<trident::ThrownTrident>,
    pub world: horizon_client::world::World,
    pub projectiles: horizon_client::join_seq::JoinList<horizon_client::projectiles::Projectile>,
    /// Stable level order remains authoritative, including across migrations.
    pub mobs: JoinList<Mob>,
    pub mob_index: MobIndex,
    pub died: FxHashSet<u32>,
    pub spawn_scratch: SpawnScratch,
    pub regions: Regions,
    pub section_order: horizon_client::mobs::region_tick::SectionOrder,
    pub workers: Workers,
}

impl LevelSimulation {
    pub fn new(world: horizon_client::world::World, tick_seed: i32) -> Self {
        Self {
            projectiles: Default::default(),
            armor_stands: Default::default(),
            block_ticks: Ticks::new(tick_seed),
            boats: Default::default(),
            bolts: Vec::new(),
            breath_clouds: Vec::new(),
            carts: Default::default(),
            crystals: Vec::new(),
            cushions: Vec::new(),
            dragon: None,
            dragon_fireballs: Vec::new(),
            dropped: Default::default(),
            falling: Default::default(),
            fangs: Default::default(),
            hoppers: HashMap::new(),
            item_frames: Vec::new(),
            knot_ids: Default::default(),
            lan_block_events: Vec::new(),
            lan_critical_hits: Vec::new(),
            lan_entity_events: Vec::new(),
            lan_events: Vec::new(),
            lectern_pulses: Vec::new(),
            level_events: Vec::new(),
            lid_openers: HashMap::new(),
            ominous_spawners: Vec::new(),
            orbs: Default::default(),
            others: Vec::new(),
            paintings: Vec::new(),
            persist: Default::default(),
            rockets: Default::default(),
            tick_chunks: Default::default(),
            tickets: Default::default(),
            tnts: Default::default(),
            tridents: Default::default(),
            world,
            mobs: Default::default(),
            mob_index: Default::default(),
            died: Default::default(),
            spawn_scratch: Default::default(),
            regions: Default::default(),
            section_order: Default::default(),
            workers: Default::default(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Execution {
    Auto,
    Legacy,
    SerialRegions,
    Parallel,
}

/// Server-wide scheduling, shared by dimensions and separate from their owned
/// mutable level state. The pool is lazy and persists across tick barriers.
pub(super) struct ServerCoordination {
    /// A server's levels other than the one in place (game/levels.rs).
    pub levels: levels::Levels,
    pub next_entity: u32,
    /// The last entity id the running server tick found before its levels
    /// ticked: an item with a later one was added this tick and first ticks
    /// on the next (`EntityTickList`). `u32::MAX` until a tick has run.
    pub ticked_entities_through: u32,
    execution: Execution,
    workers: usize,
    executor: std::sync::OnceLock<Option<horizon_client::mobs::region_tick::RegionExecutor>>,
    pub stats: TickStats,
    log_stats: bool,
}

pub(super) use horizon_client::mobs::region_tick::RegionStats as TickStats;

impl Default for ServerCoordination {
    fn default() -> Self {
        let execution = match std::env::var("HORIZON_SIMULATION_MODE").as_deref() {
            Ok("legacy") => Execution::Legacy,
            Ok("serial") => Execution::SerialRegions,
            Ok("parallel") => Execution::Parallel,
            _ => Execution::Auto,
        };
        Self {
            levels: Default::default(),
            next_entity: 0,
            ticked_entities_through: u32::MAX,
            execution,
            workers: std::env::var("HORIZON_SIMULATION_WORKERS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or_else(horizon_threads::worker_count)
                .clamp(1, 8),
            executor: Default::default(),
            stats: Default::default(),
            log_stats: std::env::var_os("HORIZON_REGION_METRICS").is_some(),
        }
    }
}
impl ServerCoordination {
    pub fn executor(
        &self,
        regions: usize,
        mobs: usize,
    ) -> Option<&horizon_client::mobs::region_tick::RegionExecutor> {
        // This threshold controls scheduling cost, never interaction reach.
        // RNG tracing itself is a side effect and remains in original order.
        if self.execution == Execution::Legacy
            || regions == 0
            || mobs == 0
            || (self.execution == Execution::Auto && (regions < 2 || mobs < 128))
            || cfg!(feature = "rng-trace")
        {
            return None;
        }
        self.executor
            .get_or_init(|| {
                let workers = if self.execution == Execution::SerialRegions {
                    1
                } else {
                    self.workers
                };
                match horizon_client::mobs::region_tick::RegionExecutor::new(workers) {
                    Ok(executor) => Some(executor),
                    Err(error) => {
                        log::warn!("could not start region workers: {error}");
                        None
                    }
                }
            })
            .as_ref()
    }
    pub fn publish(&mut self, stats: TickStats) {
        self.stats = stats;
        if self.log_stats {
            log::info!(
                "region tick: regions={} predicted={} committed={} replayed={} rng_conflicts={} read_conflicts={} read_full={} read_slot={} read_shape={} read_cell={} read_species={}",
                stats.regions,
                stats.predicted,
                stats.committed,
                stats.replayed,
                stats.rng_conflicts,
                stats.read_conflicts,
                stats.read_full,
                stats.read_slot,
                stats.read_shape,
                stats.read_cell,
                stats.read_species
            );
        }
    }
}

// The complete mutable level owner is transferable without a window or GPU.
fn _level_state_is_send(state: LevelSimulation) {
    fn require<T: Send>(_: T) {}
    require(state);
}
