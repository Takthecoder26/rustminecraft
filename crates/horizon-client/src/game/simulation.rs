//! The level's mob simulation state, separate from window state and immutable
//! `Resources`. Dimension swaps move this owner as a unit, including death
//! bookkeeping and caches; none of it belongs to the currently viewed level.
use super::{Mob, mob_index::MobIndex, natural_spawning::SpawnScratch};
use horizon_client::{
    fast_hash::FxHashSet,
    join_seq::JoinList,
    server_regions::{Regions, Workers},
};

#[derive(Default)]
pub(super) struct LevelSimulation {
    /// Stable level order remains authoritative, including across migrations.
    pub mobs: JoinList<Mob>,
    pub mob_index: MobIndex,
    pub died: FxHashSet<u32>,
    pub spawn_scratch: SpawnScratch,
    pub regions: Regions,
    pub workers: Workers,
}
