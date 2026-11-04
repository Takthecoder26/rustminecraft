//! Game events and sculk on the game side: dispatching events to sculk
//! sensors, shriekers and wardens (`GameEventDispatcher`), the vibration
//! ticker, redstone output, resonance, shrieks and warden summoning
//! (`SculkShriekerBlockEntity`, `SpawnUtil`), catalysts feeding the sculk
//! spreader from deaths, and the darkness wardens and shriekers cast.
use super::{Game, Result};
use horizon_client::join_seq::ListStamp;
use horizon_client::{
    containers::BlockEntity,
    effects::{EffectInstance, PlayerEffects},
    fast_hash::{FxHashMap, FxHashSet},
    mobs::{Mob, Species, Who, allay, warden},
    particles::{ParticleOptions, ParticleType},
    sculk::{self, Context, Info, SculkEntity, SculkKind, WardenSpawnTracker},
};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};
use horizon_world::registry::BlockStateId;
use std::collections::HashSet;
use winit::keyboard::KeyCode;

pub const HELP: &[&str] = &[
    "sculk                          sculk listeners, the warning level and the darkness blend",
    "sculk charge X Y Z AMOUNT      a mob dying there with that much experience (the nearest catalyst takes it)",
    "sculk warn LEVEL               set the player's warden warning level",
    "sculk dig                      end the minute of calm of every warden",
];

/// The sculk block kinds by state id, built once.
#[derive(Default)]
struct Tables {
    built: bool,
    /// 0 none, 1 sensor, 2 calibrated sensor, 3 shrieker, 4 catalyst.
    kinds: Vec<u8>,
    occludes: Vec<bool>,
    dampens: Vec<bool>,
    resonator: Vec<bool>,
    /// The `minecraft:climbable` tag, and the two redstone ores: read for
    /// every mob on every tick.
    climbable: Vec<bool>,
    redstone_ore: Vec<bool>,
}
/// `SculkSensorBlock.getAnalogOutputSignal`: the last vibration's frequency
/// while the sensor is active.
pub(super) fn sensor_signal(
    world: &horizon_client::world::World,
    behaviours: &horizon_world::level::Behaviours,
    state: BlockStateId,
    pos: BlockPos,
) -> Option<i32> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
        Some(BlockEntity::Sculk(e))
            if matches!(e.kind, SculkKind::Sensor | SculkKind::Calibrated) =>
        {
            Some(if behaviours.sculk_sensor_active(state) {
                e.last_frequency
            } else {
                0
            })
        }
        _ => None,
    }
}
/// `Entity.moveDist` / `nextStep` and the last state, per entity.
#[derive(Clone, Copy, Debug)]
struct Steps {
    move_dist: f32,
    next_step: f32,
    last: [f64; 3],
    on_ground: bool,
    in_water: bool,
}
impl Steps {
    /// An entity's first entry: nothing has moved it yet.
    fn first(at: [f64; 3], on_ground: bool, in_water: bool) -> Self {
        Self {
            move_dist: 0.0,
            next_step: 1.0,
            last: at,
            on_ground,
            in_water,
        }
    }
}
/// `Entity.checkFallDamage`: touching down after falling.
fn touches_down(on_ground: bool, was_on_ground: bool, dy: f64, in_water: bool) -> bool {
    on_ground && (!was_on_ground || dy < -1.0e-6) && dy < 0.0 && !in_water
}
/// `Entity.move`'s distance for `moveDist`: a climber counts its height too.
fn travelled(dx: f64, dy: f64, dz: f64, climbable: bool) -> f32 {
    if climbable {
        ((dx * dx + dy * dy + dz * dz).sqrt() * f64::from(0.6f32)) as f32
    } else {
        ((dx * dx + dz * dz).sqrt() * f64::from(0.6f32)) as f32
    }
}
/// Whether a sculk state table holds `state`.
fn table_holds(table: &[bool], state: BlockStateId) -> bool {
    table.get(state.0 as usize).copied().unwrap_or(false)
}
/// A living mob as the sculk walk reads it, taken before the walk.
#[derive(Clone, Copy)]
struct MobMove {
    id: u32,
    at: [f64; 3],
    on_ground: bool,
    in_water: bool,
    species: Species,
    damaged: Option<i64>,
    vehicle: Option<u32>,
    age: u32,
    resting: bool,
}
impl MobMove {
    fn of(m: &Mob) -> Self {
        Self {
            id: m.id,
            at: [m.position.x, m.position.y, m.position.z],
            on_ground: m.on_ground,
            in_water: m.fluids.in_water,
            species: m.species(),
            damaged: m.last_damage.map(|(_, t)| t),
            vehicle: m.extra.mount,
            age: m.age,
            resting: m.species() == Species::Bat && horizon_client::mobs::bat::resting(m),
        }
    }
}
/// What classifying a mob reads: nothing of `Game` itself, so workers can
/// share it.
#[derive(Clone, Copy)]
struct QuietView<'a> {
    world: &'a horizon_client::world::World,
    facts: &'a horizon_client::blocks::BlockFacts,
    tables: &'a Tables,
    steps: &'a FxHashMap<Who, Steps>,
    mounts: &'a FxHashMap<Who, (u32, [f64; 3])>,
    damaged: &'a FxHashMap<u32, i64>,
}
impl QuietView<'_> {
    /// A mob's new `Steps` when all its turn in the walk would do is store
    /// them: no redstone ore underfoot, no vehicle or mounts entry, no
    /// hit_ground, splash, step, swim or flap, and unchanged damage. None
    /// leaves it to the walk's own code. Each answer holds only until an event
    /// writes a block.
    fn quiet_steps(&self, m: &MobMove) -> Option<Steps> {
        let who = Who::Mob(m.id);
        // A shulker keeps no steps; a flap is decided by the walk.
        if m.vehicle.is_some()
            || self.mounts.contains_key(&who)
            || m.species == Species::Shulker
            || flaps(m.species, m.resting, m.on_ground, m.age)
            || m.damaged
                .is_some_and(|t| self.damaged.get(&m.id) != Some(&t))
        {
            return None;
        }
        let previous = self
            .steps
            .get(&who)
            .copied()
            .unwrap_or_else(|| Steps::first(m.at, m.on_ground, m.in_water));
        let (dx, dy, dz) = (
            m.at[0] - previous.last[0],
            m.at[1] - previous.last[1],
            m.at[2] - previous.last[2],
        );
        if touches_down(m.on_ground, previous.on_ground, dy, m.in_water)
            || (m.in_water && !previous.in_water)
        {
            return None;
        }
        // getOnPosLegacy: what `step_on_redstone_ore` and the footstep read.
        let state = self.world.block(
            m.at[0].floor() as i32,
            (m.at[1] - 0.2).floor() as i32,
            m.at[2].floor() as i32,
        );
        if m.on_ground && table_holds(&self.tables.redstone_ore, state) {
            return None;
        }
        let climbable = table_holds(&self.tables.climbable, state);
        let move_dist = previous.move_dist + travelled(dx, dy, dz, climbable);
        if move_dist > previous.next_step
            && !self.facts.is_air(state)
            && (m.on_ground || climbable || m.in_water)
        {
            return None;
        }
        Some(Steps {
            move_dist,
            next_step: previous.next_step,
            last: m.at,
            on_ground: m.on_ground,
            in_water: m.in_water,
        })
    }
}
#[derive(Default)]
pub(super) struct PlayerEvents {
    steps: Option<Steps>,
    last_health: Option<f32>,
    mount: Option<(u32, [f64; 3])>,
}

#[derive(Default)]
pub(super) struct SculkState {
    /// The player's warning level toward a warden.
    pub tracker: WardenSpawnTracker,
    tables: Tables,
    /// The sculk block entities in the order they were added.
    index: Index,
    pub known: HashSet<(i32, i32)>,
    /// The chunk map's membership revision `known` was taken at.
    pub known_revision: Option<u64>,
    // Keyed by mob ids the server gives out: the fast hasher is safe.
    steps: FxHashMap<Who, Steps>,
    /// The vehicle each rider had last tick and where it stood.
    mounts: FxHashMap<Who, (u32, [f64; 3])>,
    damaged: FxHashMap<u32, i64>,
    last_health: Option<f32>,
    /// The darkness effect's blend factor, previous and current tick.
    pub darkness: (f32, f32),
    /// Who causes the block changes being processed (the player's actions).
    pub actor: Option<Who>,
    ticks: u32,
    pub tracker_loaded: bool,
    /// Where the wardens and allays are in the mob list, and the list they
    /// were read from: see `listener_slots`.
    listeners: Vec<u32>,
    listeners_of: Option<ListStamp>,
}

impl SculkState {
    pub(super) fn swap_player(&mut self, other: &mut PlayerEvents) {
        let steps = self.steps.remove(&Who::Player);
        if let Some(guest) = other.steps.take() {
            self.steps.insert(Who::Player, guest);
        }
        other.steps = steps;
        let mount = self.mounts.remove(&Who::Player);
        if let Some(guest) = other.mount.take() {
            self.mounts.insert(Who::Player, guest);
        }
        other.mount = mount;
        std::mem::swap(&mut self.last_health, &mut other.last_health);
    }

    /// Exchange the level-held parts (its sculk block entities and the
    /// chunks scanned for them) with another level's (game/levels.rs).
    pub(super) fn swap_level(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.index, &mut other.index);
        std::mem::swap(&mut self.known, &mut other.known);
    }
}
/// An insertion-ordered set of block positions (`Level.blockEntityTickers`
/// keeps the order block entities were added in).
#[derive(Default)]
struct Index {
    order: Vec<(i32, i32, i32)>,
    set: HashSet<(i32, i32, i32)>,
    /// The positions by 32-block cell, each with its rank: ranks only grow,
    /// so sorting by them gives `order`'s order. Every mob's step posts a
    /// game event, and the listeners near it are read from at most eight
    /// cells instead of the whole level's.
    cells: FxHashMap<(i32, i32, i32), Vec<(u64, (i32, i32, i32))>>,
    next: u64,
}
fn index_cell(key: (i32, i32, i32)) -> (i32, i32, i32) {
    (key.0 >> 5, key.1 >> 5, key.2 >> 5)
}
impl Index {
    fn insert(&mut self, key: (i32, i32, i32)) {
        if self.set.insert(key) {
            self.order.push(key);
            self.cells
                .entry(index_cell(key))
                .or_default()
                .push((self.next, key));
            self.next += 1;
        }
    }
    fn remove(&mut self, key: &(i32, i32, i32)) {
        if self.set.remove(key) {
            self.order.retain(|k| k != key);
            let cell = index_cell(*key);
            if let Some(list) = self.cells.get_mut(&cell) {
                list.retain(|(_, k)| k != key);
                if list.is_empty() {
                    self.cells.remove(&cell);
                }
            }
        }
    }
    /// The positions whose `dist_sqr` to `at` is at most `radius` squared,
    /// in `order`'s order: what filtering `iter` gives.
    fn within(&self, at: [i32; 3], radius: i32) -> Vec<(i32, i32, i32)> {
        let limit = i64::from(radius * radius);
        let span = |v: i32| {
            let cell = |w: i64| (w >> 5).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            cell(i64::from(v) - i64::from(radius))..=cell(i64::from(v) + i64::from(radius))
        };
        let mut found: Vec<(u64, (i32, i32, i32))> = Vec::new();
        for x in span(at[0]) {
            for y in span(at[1]) {
                for z in span(at[2]) {
                    if let Some(list) = self.cells.get(&(x, y, z)) {
                        found.extend(list.iter().filter(|(_, k)| dist_sqr(at, *k) <= limit));
                    }
                }
            }
        }
        found.sort_unstable_by_key(|(rank, _)| *rank);
        found.into_iter().map(|(_, k)| k).collect()
    }
    fn contains(&self, key: &(i32, i32, i32)) -> bool {
        self.set.contains(key)
    }
    fn iter(&self) -> impl Iterator<Item = &(i32, i32, i32)> {
        self.order.iter()
    }
    fn retain(&mut self, mut keep: impl FnMut(&(i32, i32, i32)) -> bool) {
        self.order.retain(|k| keep(k));
        self.set.retain(|k| keep(k));
        self.cells.retain(|_, list| {
            list.retain(|(_, k)| keep(k));
            !list.is_empty()
        });
    }
}
const KIND_SENSOR: u8 = 1;
const KIND_CALIBRATED: u8 = 2;
const KIND_SHRIEKER: u8 = 3;
const KIND_CATALYST: u8 = 4;

fn center(p: (i32, i32, i32)) -> [f64; 3] {
    [
        f64::from(p.0) + 0.5,
        f64::from(p.1) + 0.5,
        f64::from(p.2) + 0.5,
    ]
}
fn block_of(p: [f64; 3]) -> [i32; 3] {
    [
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32,
    ]
}
fn dist_sqr(a: [i32; 3], b: (i32, i32, i32)) -> i64 {
    let d = |x: i32, y: i32| i64::from(x) - i64::from(y);
    d(a[0], b.0).pow(2) + d(a[1], b.1).pow(2) + d(a[2], b.2).pow(2)
}

impl Game<'_> {
    /// Run `f` with the player as the cause of the block changes it makes.
    pub(super) fn with_player_actor<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = self.sculk.actor.replace(Who::Player);
        let out = f(self);
        self.sculk.actor = prior;
        out
    }

    /// The `sculk` control command: listeners and their vibrations, or a
    /// test charge for the nearest catalyst.
    pub(super) fn sculk_command(&mut self, words: &[&str]) -> Result<serde_json::Value> {
        match words.get(1).copied() {
            Some("charge") => {
                let num = |i: usize| -> Result<f64> {
                    Ok(words.get(i).ok_or("missing argument")?.parse::<f64>()?)
                };
                let at = [num(2)?, num(3)?, num(4)?];
                let amount = num(5)? as u32;
                let taken = self.catalyst_takes_experience(at, |_| amount);
                Ok(serde_json::json!({ "ok": true, "taken": taken }))
            }
            Some("dig") => {
                // Let every warden's minute of calm run out.
                for m in self
                    .simulation
                    .mobs
                    .iter_mut()
                    .filter(|m| m.species() == Species::Warden)
                {
                    warden::expire_dig_cooldown(m);
                }
                Ok(serde_json::json!({ "ok": true }))
            }
            Some("warn") => {
                let level = words.get(2).ok_or("missing level")?.parse::<i32>()?;
                self.sculk.tracker.set_warning_level(level);
                Ok(serde_json::json!({ "ok": true }))
            }
            _ => Ok(self.sculk_status()),
        }
    }
    /// The `sculk` control command's listing.
    pub(super) fn sculk_status(&self) -> serde_json::Value {
        let listeners: Vec<serde_json::Value> = self
            .sculk
            .index
            .iter()
            .filter_map(|key| match self.simulation.world.block_entities.get(key) {
                Some(BlockEntity::Sculk(e)) => Some(serde_json::json!({
                    "pos": [key.0, key.1, key.2],
                    "kind": e.id(),
                    "current": e.vibration.current.map(|i| i.event),
                    "travel": e.vibration.travel,
                    "last_frequency": e.last_frequency,
                    "warning_level": e.warning_level,
                    "cursors": e.spreader.cursors.len(),
                })),
                _ => None,
            })
            .collect();
        serde_json::json!({
            "listeners": listeners,
            "warning_level": self.sculk.tracker.warning_level,
            "cooldown": self.sculk.tracker.cooldown_ticks,
            "darkness": self.sculk.darkness.1,
            "chunks_known": self.sculk.known.len(),
        })
    }
    /// `WardenSpawnTrackerCommand`: clear the player's tracker or set its
    /// warning level.
    pub(super) fn cmd_warden_spawn_tracker(&mut self, level: Option<i32>) {
        use horizon_client::text::Text;
        let key = match level {
            Some(level) => {
                self.sculk.tracker.set_warning_level(level);
                "commands.warden_spawn_tracker.set.success.single"
            }
            None => {
                self.sculk.tracker.reset();
                "commands.warden_spawn_tracker.clear.success.single"
            }
        };
        if self.ui.rules.bool("send_command_feedback") {
            self.system_message(Text::translate(
                key,
                &[Text::literal(super::chat::PLAYER_NAME)],
            ));
        }
    }
    /// `ServerPlayer`'s `warden_spawn_tracker` from the player data.
    fn load_warden_tracker(&mut self) {
        if self.sculk.tracker_loaded {
            return;
        }
        self.sculk.tracker_loaded = true;
        if let Some(tag) = self
            .level
            .player_extra
            .as_ref()
            .and_then(|p| p.get("warden_spawn_tracker"))
        {
            self.sculk.tracker = WardenSpawnTracker::from_nbt(tag);
        }
    }
    /// Write the tracker into the player data being saved.
    pub(super) fn save_warden_tracker(&mut self) {
        if !self.sculk.tracker_loaded {
            return;
        }
        let tag = self.sculk.tracker.to_nbt();
        let extra = self
            .level
            .player_extra
            .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
        if let horizon_world::nbt::Tag::Compound(map) = extra {
            map.insert("warden_spawn_tracker".into(), tag);
        }
    }
    /// Who is causing the block changes being processed.
    pub(super) fn sculk_actor(&self) -> Option<Who> {
        self.sculk.actor
    }

    // ---- setup and the block entity index ----

    pub(super) fn sculk_prepare(&mut self) {
        if self.sculk.tables.built {
            return;
        }
        let registry = self.pack.assets.registry;
        let n = registry.len();
        let tag = |name: &str| {
            self.res
                .items()
                .block_tags
                .get(name)
                .cloned()
                .unwrap_or_default()
        };
        let (wool, dampening, resonators, climbable) = (
            tag("minecraft:occludes_vibration_signals"),
            tag("minecraft:dampens_vibrations"),
            tag("minecraft:vibration_resonators"),
            tag("minecraft:climbable"),
        );
        let mut t = Tables {
            built: true,
            kinds: vec![0; n],
            occludes: vec![false; n],
            dampens: vec![false; n],
            resonator: vec![false; n],
            climbable: vec![false; n],
            redstone_ore: vec![false; n],
        };
        for i in 0..n {
            let facts = self.res.facts().get(BlockStateId(i as u32));
            let name: &str = &facts.name;
            t.kinds[i] = match name {
                "minecraft:sculk_sensor" => KIND_SENSOR,
                "minecraft:calibrated_sculk_sensor" => KIND_CALIBRATED,
                "minecraft:sculk_shrieker" => KIND_SHRIEKER,
                "minecraft:sculk_catalyst" => KIND_CATALYST,
                _ => 0,
            };
            t.occludes[i] = wool.contains(name);
            t.dampens[i] = dampening.contains(name);
            t.resonator[i] = resonators.contains(name);
            t.climbable[i] = climbable.contains(name);
            t.redstone_ore[i] = matches!(
                name,
                "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore"
            );
        }
        self.simulation.world.watched = t.kinds.iter().map(|k| *k != 0).collect();
        self.sculk.tables = t;
    }
    fn kind_of_state(&self, state: u32) -> u8 {
        self.sculk
            .tables
            .kinds
            .get(state as usize)
            .copied()
            .unwrap_or(0)
    }
    fn state_is(&self, table: &[bool], state: BlockStateId) -> bool {
        table_holds(table, state)
    }
    fn entity_for_kind(kind: u8) -> Option<SculkEntity> {
        Some(SculkEntity::new(match kind {
            KIND_SENSOR => SculkKind::Sensor,
            KIND_CALIBRATED => SculkKind::Calibrated,
            KIND_SHRIEKER => SculkKind::Shrieker,
            KIND_CATALYST => SculkKind::Catalyst,
            _ => return None,
        }))
    }
    /// Find the sculk blocks of chunks that just became resident, and apply
    /// the block writes recorded since the last tick.
    pub(super) fn sculk_sync_index(&mut self) {
        // Chunks that arrived or left (every half second is plenty).
        // Nothing to do while no chunk came or went.
        let revision = self.simulation.world.chunks.membership_revision();
        if self.sculk.ticks.is_multiple_of(10) && self.sculk.known_revision != Some(revision) {
            self.sculk.known_revision = Some(revision);
            let loaded: HashSet<(i32, i32)> =
                self.simulation.world.chunks.keys().copied().collect();
            let arrived: Vec<(i32, i32)> = loaded.difference(&self.sculk.known).copied().collect();
            for key in arrived {
                self.sculk_scan_chunk(key);
            }
            // The blocks of every chunk that left, in one pass: a player
            // leaving unloads thousands of chunks at once.
            let gone: HashSet<(i32, i32)> = self.sculk.known.difference(&loaded).copied().collect();
            if !gone.is_empty() {
                self.sculk
                    .index
                    .retain(|p| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
            }
            self.sculk.known = loaded;
        }
        let writes = std::mem::take(&mut self.simulation.world.watched_writes);
        for (key, old, new) in writes {
            let (old_kind, new_kind) = (self.kind_of_state(old), self.kind_of_state(new));
            if old_kind == new_kind {
                continue;
            }
            // SculkShriekerBlockEntity.preRemoveSideEffects.
            if old_kind == KIND_SHRIEKER
                && let Some(BlockEntity::Sculk(e)) = self.simulation.world.block_entities.get(&key)
                && self.shrieker_shrieking(BlockStateId(old))
            {
                let warning = e.warning_level;
                self.shrieker_respond(key, BlockStateId(old), warning);
            }
            if matches!(
                self.simulation.world.block_entities.get(&key),
                Some(BlockEntity::Sculk(_))
            ) {
                self.simulation.world.block_entities.remove(&key);
            }
            self.sculk.index.remove(&key);
            if let Some(e) = Self::entity_for_kind(new_kind) {
                self.simulation
                    .world
                    .block_entities
                    .insert(key, BlockEntity::Sculk(Box::new(e)));
                self.sculk.index.insert(key);
            }
        }
    }
    fn sculk_scan_chunk(&mut self, key: (i32, i32)) {
        let Some(chunk) = self.simulation.world.chunks.get(&key).cloned() else {
            return;
        };
        let mut found = Vec::new();
        for (s, section) in chunk.sections.iter().enumerate() {
            let y0 = self.simulation.world.bottom_y() + s as i32 * 16;
            // Without a sculk state in its palette, no block of the section
            // is one: most sections are passed over without reading a block.
            if !section.blocks.maybe_has(|v| self.kind_of_state(v) != 0) {
                continue;
            }
            if section.blocks.uniform().is_some() {
                // A uniform section of sculk blocks does not exist.
                continue;
            }
            let mut a = [0u32; 4096];
            section.unpack(&mut a);
            for (i, v) in a.iter().enumerate() {
                if self.kind_of_state(*v) != 0 {
                    let (x, z, y) = (i & 15, (i >> 4) & 15, i >> 8);
                    found.push((
                        key.0 * 16 + x as i32,
                        y0 + y as i32,
                        key.1 * 16 + z as i32,
                        *v,
                    ));
                }
            }
        }
        for (x, y, z, state) in found {
            let pos = (x, y, z);
            if !matches!(
                self.simulation.world.block_entities.get(&pos),
                Some(BlockEntity::Sculk(_))
            ) && let Some(e) = Self::entity_for_kind(self.kind_of_state(state))
            {
                self.simulation
                    .world
                    .block_entities
                    .insert(pos, BlockEntity::Sculk(Box::new(e)));
            }
            self.sculk.index.insert(pos);
        }
    }
    fn sculk_entity(&mut self, key: (i32, i32, i32)) -> Option<&mut SculkEntity> {
        match self.simulation.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Sculk(e)) => Some(e),
            _ => None,
        }
    }

    // ---- contexts ----

    fn state_dampens(&self, state: Option<BlockStateId>) -> bool {
        state.is_some_and(|s| self.state_is(&self.sculk.tables.dampens, s))
    }
    /// `GameEvent.Context.of(player, state)`.
    pub(super) fn player_context(&self, state: Option<BlockStateId>) -> Context {
        let (dead, creative) = self
            .player
            .as_ref()
            .map_or((true, true), |p| (p.health.dead_or_dying(), self.creative));
        Context {
            source: Some(Who::Player),
            owner: None,
            dampening_state: self.state_dampens(state),
            spectator: self.spectator(),
            stepping_carefully: self.keys.contains(&KeyCode::ShiftLeft),
            dampens: false,
            targetable: !dead && !creative && !self.spectator(),
            living: true,
            player: true,
        }
    }
    /// `GameEvent.Context.of(mob, state)`.
    pub(super) fn mob_context(&mut self, id: u32, state: Option<BlockStateId>) -> Context {
        let mob = self.simulation.mob_index.get(&self.simulation.mobs, id);
        self.context_of_mob(mob, id, state)
    }
    fn context_of_mob(&self, mob: Option<&Mob>, id: u32, state: Option<BlockStateId>) -> Context {
        Context {
            source: Some(Who::Mob(id)),
            owner: None,
            dampening_state: self.state_dampens(state),
            spectator: false,
            stepping_carefully: false,
            dampens: mob.is_some_and(|m| m.species() == Species::Warden),
            targetable: mob.is_some_and(|m| m.alive() && m.species() != Species::Warden),
            living: true,
            player: mob.is_some_and(|m| m.rider.is_some()),
        }
    }
    pub(super) fn who_context(&mut self, who: Option<Who>, state: Option<BlockStateId>) -> Context {
        match who {
            Some(Who::Player) => self.player_context(state),
            Some(Who::Mob(id)) => self.mob_context(id, state),
            None => Context {
                dampening_state: self.state_dampens(state),
                ..Context::default()
            },
        }
    }

    // ---- posting ----

    /// `Level.gameEvent`: tell the listeners within reach.
    pub(super) fn post_game_event(&mut self, event: &'static str, pos: [f64; 3], ctx: Context) {
        self.native_game_event(event, pos);
        super::dispense_fixture::record(|| {
            format!(
                "G {},{},{} {event}",
                pos[0].floor() as i32,
                pos[1].floor() as i32,
                pos[2].floor() as i32
            )
        });
        if self.remote.is_some() || !self.sculk.tables.built {
            return;
        }
        let at = block_of(pos);
        let radius = sculk::notification_radius(event).min(16);
        let near = self.sculk.index.within(at, radius);
        for key in near {
            self.sculk_listen(key, event, pos, &ctx);
        }
        let stamp = self.simulation.mobs.stamp();
        let slots = self.listener_slots(stamp);
        for &i in &slots {
            let i = i as usize;
            let Some(mob) = self.simulation.mobs.get(i) else {
                continue;
            };
            let (species, alive) = (mob.species(), mob.alive());
            if species == Species::Warden && alive {
                self.warden_listen(i, event, pos, &ctx);
            }
            if species == Species::Allay && alive {
                self.allay_listen(i, event, pos, &ctx);
            }
        }
        self.sculk.listeners = slots;
        self.sculk.listeners_of = Some(stamp);
    }
    /// The slots of the mobs that hear game events, in list order. An event
    /// asks for them every time it is posted, hundreds of times a tick on a
    /// busy server, and a walk over the list misses the cache once per mob, so
    /// one walk serves until the list is next written to.
    fn listener_slots(&mut self, stamp: ListStamp) -> Vec<u32> {
        let mut slots = std::mem::take(&mut self.sculk.listeners);
        // Taken: an event posted by a listener walks the list itself.
        if self.sculk.listeners_of.take() != Some(stamp) {
            slots.clear();
            slots.extend(
                self.simulation
                    .mobs
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| matches!(m.species(), Species::Warden | Species::Allay))
                    .map(|(i, _)| i as u32),
            );
        }
        slots
    }
    /// A game event at a block (`Level.gameEvent(source, event, pos)`).
    pub(super) fn post_block_game_event(
        &mut self,
        event: &'static str,
        pos: [i32; 3],
        who: Option<Who>,
        state: Option<BlockStateId>,
    ) {
        let ctx = self.who_context(who, state);
        self.post_game_event(event, center((pos[0], pos[1], pos[2])), ctx);
    }
    /// `LivingEntity.actuallyHurt`'s damage vibration, heard as the hit lands;
    /// the end-of-tick scan then knows this damage.
    pub(super) fn mob_damage_event(&mut self, id: u32) {
        if let Some(t) = self
            .simulation
            .mob_index
            .get(&self.simulation.mobs, id)
            .and_then(|m| m.last_damage.map(|(_, t)| t))
        {
            self.sculk.damaged.insert(id, t);
        }
        self.post_entity_event("minecraft:entity_damage", Who::Mob(id));
    }
    /// A game event of an entity at its position.
    pub(super) fn post_entity_event(&mut self, event: &'static str, who: Who) {
        let at = match who {
            Who::Player => self
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Who::Mob(id) => self
                .simulation
                .mob_index
                .get(&self.simulation.mobs, id)
                .map(|m| [m.position.x, m.position.y, m.position.z]),
        };
        if let Some(at) = at {
            let ctx = self.who_context(Some(who), None);
            self.post_game_event(event, at, ctx);
        }
    }
    /// `LevelEvent::GameEvent` from the block behaviour library.
    pub(super) fn sculk_level_event(
        &mut self,
        pos: BlockPos,
        event: &'static str,
        state: Option<BlockStateId>,
    ) {
        let who = self.sculk.actor;
        self.post_block_game_event(event, [pos.x, pos.y, pos.z], who, state);
    }

    fn occluded(&self, origin: [f64; 3], dest: [f64; 3]) -> bool {
        sculk::is_occluded(origin, dest, |p| {
            self.state_is(
                &self.sculk.tables.occludes,
                self.simulation.world.block(p.x, p.y, p.z),
            )
        })
    }
    /// `VibrationSystem.Listener.handleGameEvent` for a sculk block.
    fn sculk_listen(
        &mut self,
        key: (i32, i32, i32),
        event: &'static str,
        pos: [f64; 3],
        ctx: &Context,
    ) {
        let Some(entity) = self.sculk_entity(key) else {
            self.sculk.index.remove(&key);
            return;
        };
        let (kind, radius, busy) = (
            entity.kind,
            entity.listener_radius(),
            entity.vibration.current.is_some(),
        );
        if kind == SculkKind::Catalyst || busy {
            return;
        }
        let source_block = block_of(pos);
        if dist_sqr(source_block, key) > i64::from(radius * radius) {
            return;
        }
        let listens = match kind {
            SculkKind::Shrieker => sculk::shrieker_can_listen(event),
            _ => sculk::is_vibration(event),
        };
        // VibrationSystem.User.isValidVibration: a player sneaking past a
        // sensor, whose step it ignores, is AVOID_VIBRATION (sensors only;
        // a shrieker cannot trigger it).
        if listens
            && matches!(kind, SculkKind::Sensor | SculkKind::Calibrated)
            && ctx.source == Some(Who::Player)
            && !ctx.spectator
            && ctx.stepping_carefully
            && sculk::ignored_when_sneaking(event)
        {
            self.trigger_player("minecraft:avoid_vibration");
        }
        if !sculk::valid_vibration(listens, event, ctx) {
            return;
        }
        let state = self.simulation.world.block(key.0, key.1, key.2);
        let can_receive = match kind {
            SculkKind::Sensor | SculkKind::Calibrated => {
                // The sensor does not hear its own placement or removal.
                let own = source_block == [key.0, key.1, key.2]
                    && matches!(event, "minecraft:block_destroy" | "minecraft:block_place");
                let frequency = sculk::frequency(event);
                let calibrated_out = kind == SculkKind::Calibrated && {
                    let back = self.calibrated_signal(key);
                    back != 0 && frequency != back
                };
                !own && !calibrated_out
                    && frequency != 0
                    && self.res.behaviours().sculk_sensor_can_activate(state)
            }
            SculkKind::Shrieker => !self.shrieker_shrieking(state) && ctx.player,
            SculkKind::Catalyst => false,
        };
        if !can_receive {
            return;
        }
        let dest = center(key);
        if self.occluded(pos, dest) {
            return;
        }
        let distance = {
            let d = [pos[0] - dest[0], pos[1] - dest[1], pos[2] - dest[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32
        };
        let time = self.simulation.world.game_time;
        if let Some(entity) = self.sculk_entity(key) {
            entity.vibration.schedule(
                Info {
                    event,
                    distance,
                    pos,
                    source: ctx.source,
                    owner: ctx.owner,
                },
                time,
            );
        }
    }
    fn calibrated_signal(&mut self, key: (i32, i32, i32)) -> i32 {
        let pos = BlockPos::new(key.0, key.1, key.2);
        let mut value = 0;
        self.with_level(|b, level, ticks| {
            value = b.calibrated_back_signal(level, ticks, pos);
        });
        value
    }
    fn shrieker_shrieking(&self, state: BlockStateId) -> bool {
        self.state_property(state, "shrieking") == Some("true")
    }
    fn state_property(&self, state: BlockStateId, key: &str) -> Option<&str> {
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get(key))
            .map(String::as_str)
    }

    /// `Warden.VibrationUser`: accept a vibration for the warden at `index`.
    fn warden_listen(&mut self, index: usize, event: &'static str, pos: [f64; 3], ctx: &Context) {
        let now = self.simulation.world.game_time;
        let (occludes, world) = (&self.sculk.tables.occludes, &self.simulation.world);
        let avoided = warden::listen(
            &mut self.simulation.mobs[index],
            now,
            event,
            pos,
            ctx,
            |a, b| {
                sculk::is_occluded(a, b, |p| {
                    let state = world.block(p.x, p.y, p.z);
                    occludes.get(state.0 as usize).copied().unwrap_or(false)
                })
            },
        );
        if avoided {
            self.trigger_player("minecraft:avoid_vibration");
        }
    }

    /// `VibrationSystem.Listener.handleGameEvent` for the allay at `index`
    /// (`Allay.VibrationUser`).
    fn allay_listen(&mut self, index: usize, event: &'static str, pos: [f64; 3], ctx: &Context) {
        let now = self.simulation.world.game_time;
        let (occludes, world) = (&self.sculk.tables.occludes, &self.simulation.world);
        let dimension = self.dim.id();
        allay::listen(
            &mut self.simulation.mobs[index],
            now,
            event,
            pos,
            ctx,
            dimension,
            |a, b| {
                sculk::is_occluded(a, b, |p| {
                    let state = world.block(p.x, p.y, p.z);
                    occludes.get(state.0 as usize).copied().unwrap_or(false)
                })
            },
        );
    }

    // ---- ticking ----

    /// The entity phase of a tick for sculk: the player's warden spawn
    /// tracker (`ServerPlayer.tick`), and the events of entities standing on
    /// and moving over blocks.
    pub(super) fn tick_sculk_entities(&mut self) {
        self.sculk.ticks = self.sculk.ticks.wrapping_add(1);
        self.sculk_prepare();
        self.load_warden_tracker();
        self.sculk.tracker.tick();
        self.tick_darkness_blend();
        self.sculk_step_on();
        self.tick_entity_events();
    }
    /// The block entity phase for sculk: vibration tickers and catalysts, in
    /// the order the block entities were added (`Level.tickBlockEntities`).
    pub(super) fn tick_sculk_blocks(&mut self) -> Result<()> {
        self.sculk_prepare();
        self.sculk_sync_index();
        // `ServerLevel.shouldTickBlocksAt` in a block-ticking chunk, from
        // every player's tickets (`update_chunk_tickets`), once the chunk is
        // lit: asked once per chunk, as the sensors share few chunks. Nothing
        // a sculk tick does loads, unloads or lights a chunk or moves a ticket.
        let levels = self.simulation.tickets.levels();
        let chunks = &self.simulation.world.chunks;
        let mut ticking = FxHashMap::<(i32, i32), bool>::default();
        let mut ticks = |cx: i32, cz: i32| {
            *ticking.entry((cx, cz)).or_insert_with(|| {
                levels.block_ticking(cx, cz) && chunks.get(&(cx, cz)).is_some_and(|c| c.lit)
            })
        };
        // The ticking ones in the index's order, with whether the chunks
        // around them tick too. Most sensors and catalysts wait for a
        // vibration or a death: their tick would change nothing, so they
        // are passed over here (a vibration scheduled during these ticks is
        // chosen on the next tick at the earliest, as in vanilla).
        let entities = &self.simulation.world.block_entities;
        let due: Vec<((i32, i32, i32), bool)> = self
            .sculk
            .index
            .iter()
            .filter_map(|&key| {
                let (cx, cz) = (key.0 >> 4, key.2 >> 4);
                if !ticks(cx, cz)
                    || matches!(entities.get(&key), Some(BlockEntity::Sculk(e)) if e.idle())
                {
                    return None;
                }
                let adjacent = (-1..=1).all(|dx| (-1..=1).all(|dz| ticks(cx + dx, cz + dz)));
                Some((key, adjacent))
            })
            .collect();
        for (key, adjacent) in due {
            self.tick_sculk_block(key, adjacent)?;
        }
        Ok(())
    }
    fn tick_sculk_block(&mut self, key: (i32, i32, i32), adjacent: bool) -> Result<()> {
        let time = self.simulation.world.game_time;
        let Some(entity) = self.sculk_entity(key) else {
            self.sculk.index.remove(&key);
            return Ok(());
        };
        match entity.kind {
            SculkKind::Catalyst => {
                // SculkCatalystBlockEntity.serverTick.
                let mut spreader = std::mem::take(&mut entity.spreader);
                if !spreader.cursors.is_empty() {
                    let origin = BlockPos::new(key.0, key.1, key.2);
                    self.with_level(|b, level, ticks| {
                        b.sculk_update_cursors(level, ticks, &mut spreader, origin, true);
                    });
                    self.process_level_events()?;
                }
                if let Some(e) = self.sculk_entity(key) {
                    // Cursors added while spreading keep their place.
                    let added = std::mem::take(&mut e.spreader);
                    e.spreader = spreader;
                    for c in added.cursors {
                        e.spreader.add_cursor(c);
                    }
                }
            }
            SculkKind::Sensor | SculkKind::Calibrated | SculkKind::Shrieker => {
                let tick = entity
                    .vibration
                    .tick(time, adjacent, true, |d| d.floor() as i32);
                if let Some((info, travel)) = tick.started {
                    self.vibration_particle(info.pos, center(key), travel);
                }
                if let Some(info) = tick.received {
                    self.sculk_receive(key, info)?;
                }
            }
        }
        Ok(())
    }

    // ---- sensors ----

    /// `onReceiveVibration` of a sensor or a shrieker.
    fn sculk_receive(&mut self, key: (i32, i32, i32), info: Info) -> Result<()> {
        let Some(entity) = self.sculk_entity(key) else {
            return Ok(());
        };
        let (kind, radius) = (entity.kind, entity.listener_radius());
        let origin = block_of(info.pos);
        let distance = (dist_sqr(origin, key) as f64).sqrt() as f32;
        let block = match kind {
            SculkKind::Sensor => "minecraft:sculk_sensor",
            SculkKind::Calibrated => "minecraft:calibrated_sculk_sensor",
            SculkKind::Shrieker => "minecraft:sculk_shrieker",
            SculkKind::Catalyst => "minecraft:sculk_catalyst",
        };
        let mut cancel = false;
        self.native_block_receive_game(
            self.dim.id(),
            [key.0, key.1, key.2],
            block,
            info.event,
            &mut cancel,
        );
        if cancel {
            return Ok(());
        }
        match kind {
            SculkKind::Sensor | SculkKind::Calibrated => {
                let state = self.simulation.world.block(key.0, key.1, key.2);
                if self.res.behaviours().sculk_sensor_can_activate(state) {
                    let frequency = sculk::frequency(info.event);
                    if let Some(e) = self.sculk_entity(key) {
                        e.last_frequency = frequency;
                    }
                    let power = sculk::redstone_strength(distance, radius);
                    self.activate_sensor(key, info.source, power, frequency)?;
                }
            }
            SculkKind::Shrieker => {
                let by_player = info.owner.or(info.source);
                self.try_shriek(
                    key,
                    by_player == Some(Who::Player) || self.rides_player(by_player),
                )?;
            }
            SculkKind::Catalyst => {}
        }
        Ok(())
    }
    fn rides_player(&self, who: Option<Who>) -> bool {
        matches!(who, Some(Who::Mob(id)) if self.simulation.mobs.iter().any(|m| m.id == id && m.rider.is_some()))
    }
    /// `SculkSensorBlock.activate`.
    fn activate_sensor(
        &mut self,
        key: (i32, i32, i32),
        source: Option<Who>,
        power: i32,
        frequency: i32,
    ) -> Result<()> {
        let pos = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| b.sculk_sensor_activate(level, ticks, pos, power));
        self.process_level_events()?;
        // tryResonateVibration.
        if (1..=15).contains(&frequency) {
            for direction in Direction::ALL {
                let n = direction.normal();
                let relative = (key.0 + n[0], key.1 + n[1], key.2 + n[2]);
                let state = self
                    .simulation
                    .world
                    .block(relative.0, relative.1, relative.2);
                if self.state_is(&self.sculk.tables.resonator, state) {
                    let ctx = self.who_context(source, Some(state));
                    self.post_game_event(
                        sculk::RESONANCE[frequency as usize - 1],
                        center(relative),
                        ctx,
                    );
                    self.audio.play(
                        "minecraft:block.amethyst_block.resonate",
                        Some(center(relative)),
                        1.0,
                        sculk::resonance_pitch(frequency),
                    );
                }
            }
        }
        let ctx = self.who_context(source, None);
        self.post_game_event(sculk::TENDRILS_CLICKING, center(key), ctx);
        self.with_level(|b, level, ticks| b.sculk_sensor_click(level, ticks, pos));
        self.process_level_events()
    }
    /// `SculkSensorBlock.stepOn` and `SculkShriekerBlock.stepOn`: standing on
    /// them is heard (or seen) every tick.
    fn sculk_step_on(&mut self) {
        self.sculk_step_on_player();
        // `sculk_stand_on` only asks about the blocks in the index.
        if self.sculk.index.set.is_empty() {
            return;
        }
        let mut standing: Vec<(Who, [f64; 3])> = Vec::new();
        for m in &self.simulation.mobs {
            if m.alive() && m.on_ground {
                standing.push((Who::Mob(m.id), [m.position.x, m.position.y, m.position.z]));
            }
        }
        self.sculk_stand_on(standing);
    }
    /// `stepOn` for the player in place: the host, or a guest swapped in
    /// (`guest_server_tick`), as a server has no player of its own.
    pub(super) fn sculk_step_on_player(&mut self) {
        if let Some(p) = self.player.as_ref()
            && p.ground.on_ground
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.sculk_stand_on(vec![(Who::Player, at)]);
        }
    }
    fn sculk_stand_on(&mut self, standing: Vec<(Who, [f64; 3])>) {
        for (who, at) in standing {
            // getOnPosLegacy: 0.2 below the feet.
            let key = (
                at[0].floor() as i32,
                (at[1] - 0.2).floor() as i32,
                at[2].floor() as i32,
            );
            if !self.sculk.index.contains(&key) {
                continue;
            }
            let state = self.simulation.world.block(key.0, key.1, key.2);
            let is_warden = matches!(who, Who::Mob(id)
                if self.simulation.mobs.iter().any(|m| m.id == id && m.species() == Species::Warden));
            match self.kind_of_state(state.0) {
                KIND_SENSOR | KIND_CALIBRATED => {
                    if is_warden || !self.res.behaviours().sculk_sensor_can_activate(state) {
                        continue;
                    }
                    let calibrated = self.kind_of_state(state.0) == KIND_CALIBRATED;
                    if calibrated {
                        let back = self.calibrated_signal(key);
                        if back != 0 && back != 1 {
                            continue;
                        }
                    }
                    let dest = center(key);
                    let d = [at[0] - dest[0], at[1] - dest[1], at[2] - dest[2]];
                    let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
                    let time = self.simulation.world.game_time;
                    if let Some(e) = self.sculk_entity(key) {
                        e.vibration.schedule(
                            Info {
                                event: "minecraft:step",
                                distance,
                                pos: at,
                                source: Some(who),
                                owner: None,
                            },
                            time,
                        );
                    }
                }
                KIND_SHRIEKER => {
                    let by_player = who == Who::Player || self.rides_player(Some(who));
                    if by_player {
                        let _ = self.try_shriek(key, true);
                    }
                }
                _ => {}
            }
        }
    }

    // ---- shriekers ----

    fn can_respond(&self, state: BlockStateId) -> bool {
        use horizon_world::food::Difficulty;
        self.state_property(state, "can_summon") == Some("true")
            && self.simulation.world.health_rules.difficulty != Difficulty::Peaceful
            && self.ui.rules.bool("spawn_wardens")
    }
    /// `SculkShriekerBlockEntity.tryShriek`.
    fn try_shriek(&mut self, key: (i32, i32, i32), by_player: bool) -> Result<()> {
        if !by_player {
            return Ok(());
        }
        let state = self.simulation.world.block(key.0, key.1, key.2);
        if self.shrieker_shrieking(state) {
            return Ok(());
        }
        if let Some(e) = self.sculk_entity(key) {
            e.warning_level = 0;
        }
        if !self.can_respond(state) || self.try_to_warn(key) {
            self.shriek(key)?;
        }
        Ok(())
    }
    /// `WardenSpawnTracker.tryWarn` for the one player.
    fn try_to_warn(&mut self, key: (i32, i32, i32)) -> bool {
        let c = center(key);
        let warden_nearby = self.simulation.mobs.iter().any(|m| {
            m.species() == Species::Warden
                && m.alive()
                && (m.position.x - c[0]).abs() < 24.0
                && (m.position.y + f64::from(m.height()) / 2.0 - c[1]).abs() < 24.0
                && (m.position.z - c[2]).abs() < 24.0
        });
        // The only player is the one who set it off.
        let mut trackers = [self.sculk.tracker];
        let level = WardenSpawnTracker::try_warn(&mut trackers, warden_nearby);
        self.sculk.tracker = trackers[0];
        match level {
            Some(level) => {
                if let Some(e) = self.sculk_entity(key) {
                    e.warning_level = level;
                }
                true
            }
            None => false,
        }
    }
    /// `SculkShriekerBlockEntity.shriek`.
    fn shriek(&mut self, key: (i32, i32, i32)) -> Result<()> {
        let pos = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| b.sculk_shrieker_shriek(level, ticks, pos));
        self.process_level_events()?;
        let ctx = self.player_context(None);
        self.post_game_event(sculk::SHRIEK, center(key), ctx);
        Ok(())
    }
    /// The shriek ended (or the block went away while shrieking).
    pub(super) fn shrieker_respond(
        &mut self,
        key: (i32, i32, i32),
        state: BlockStateId,
        warning: i32,
    ) {
        if !(self.can_respond(state) && warning > 0) {
            return;
        }
        if !self.try_summon_warden(key, warning) {
            self.warden_reply_sound(key, warning);
        }
        self.apply_darkness(center(key), 40.0);
    }
    /// A `shrieker_respond` block event.
    pub(super) fn shrieker_block_event(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        let warning = self.sculk_entity(key).map_or(0, |e| e.warning_level);
        self.shrieker_respond(key, state, warning);
    }
    fn warden_reply_sound(&mut self, key: (i32, i32, i32), warning: i32) {
        let sound = match warning {
            1 => "minecraft:entity.warden.nearby_close",
            2 => "minecraft:entity.warden.nearby_closer",
            3 => "minecraft:entity.warden.nearby_closest",
            4 => "minecraft:entity.warden.listening_angry",
            _ => return,
        };
        let r = &mut self.simulation.world.random;
        let mut offset = |v: i32| v + r.next_int_bound(21).unwrap_or(0) - 10;
        let at = [
            f64::from(offset(key.0)),
            f64::from(offset(key.1)),
            f64::from(offset(key.2)),
        ];
        self.audio.play(sound, Some(at), 5.0, 1.0);
    }
    /// `SpawnUtil.trySpawnMob(WARDEN, TRIGGERED, level, pos, 20, 5, 6,
    /// ON_TOP_OF_COLLIDER, false)`: every candidate spot creates a warden
    /// (`EntityType.create` draws its yaw from the level random and
    /// `finalizeSpawn` plays its agitated sound), which is discarded when
    /// `checkSpawnObstruction` fails.
    fn try_summon_warden(&mut self, key: (i32, i32, i32), warning: i32) -> bool {
        if warning < 4 {
            return false;
        }
        let Some(kind) = self.res.mob_types().get("minecraft:warden").cloned() else {
            return false;
        };
        for _ in 0..20 {
            let r = &mut self.simulation.world.random;
            let dx = r.next_int_bound(11).unwrap_or(0) - 5;
            let dz = r.next_int_bound(11).unwrap_or(0) - 5;
            let Some(at) = self.spawn_position(key.0 + dx, key.1 + 6, key.2 + dz, 6) else {
                continue;
            };
            let spot = [
                f64::from(at.0) + 0.5,
                f64::from(at.1),
                f64::from(at.2) + 0.5,
            ];
            let yaw = horizon_client::camera::wrap_degrees(
                self.simulation.world.random.next_float() * 360.0,
            );
            self.simulation_server.next_entity += 1;
            let mut mob = Mob::new(
                self.simulation_server.next_entity,
                kind.clone(),
                horizon_core::geometry::Vec3::new(spot[0], spot[1], spot[2]),
                yaw,
            );
            // Warden.finalizeSpawn, then Mob.finalizeSpawn's draws from the
            // level random: the follow range bonus and left-handedness.
            let mut events = Vec::new();
            warden::finalize(&mut mob, true, &mut events);
            let _ = self.handle_mob_events(events);
            let r = &mut self.simulation.world.random;
            let bonus = r.triangle(0.0, 0.11485000000000001);
            mob.attrs.follow_range += mob.kind.follow_range * bonus;
            mob.update_max_visited_nodes();
            let _left_handed = r.next_float() < 0.05;
            if !self.warden_fits(&mob) {
                continue;
            }
            self.simulation.mobs.push(mob);
            self.native_mobs_joined(self.simulation.mobs.len() - 1, false);
            return true;
        }
        false
    }
    /// `SpawnUtil.moveToPossibleSpawnPosition` with `ON_TOP_OF_COLLIDER`.
    fn spawn_position(&self, x: i32, y: i32, z: i32, range: i32) -> Option<(i32, i32, i32)> {
        use horizon_client::mobs::data;
        let flags = |b: BlockStateId| self.res.mob_data().state(b);
        let mut search = y;
        let mut above = self.simulation.world.block(x, search, z);
        for _ in (-range..=range).rev() {
            search -= 1;
            let current = self.simulation.world.block(x, search, z);
            let empty_above = flags(above).top == 0.0;
            let full_top = flags(current).flags & data::STURDY_TOP != 0;
            if empty_above && full_top {
                return Some((x, search + 1, z));
            }
            above = current;
        }
        None
    }
    /// `Warden.checkSpawnObstruction`: no liquid in its box, no living entity
    /// in the way (`isUnobstructed`), and no block collision.
    fn warden_fits(&self, mob: &Mob) -> bool {
        let bounds = mob.bounds();
        let player_blocks = self
            .level_players()
            .iter()
            .any(|p| p.alive && !p.spectator && p.bounds.intersects(bounds));
        if player_blocks
            || self
                .simulation
                .mobs
                .iter()
                .any(|m| !m.removed && m.alive() && m.bounds().intersects(bounds))
        {
            return false;
        }
        let others: Vec<horizon_client::mobs::Seen> = Vec::new();
        let w = self.mob_world_base(&others);
        let min = bounds.min_position();
        let max = bounds.max_position();
        // Level.containsAnyLiquid: floor(min) up to ceil(max), exclusive.
        for x in min.x.floor() as i32..max.x.ceil() as i32 {
            for y in min.y.floor() as i32..max.y.ceil() as i32 {
                for z in min.z.floor() as i32..max.z.ceil() as i32 {
                    if w.fluid(x, y, z) != horizon_client::blocks::Fluid::None {
                        return false;
                    }
                }
            }
        }
        w.no_collision(bounds)
    }

    // ---- darkness ----

    /// The darkness effect's blend, where this game ticks the player in its
    /// own level (a server's client ticks it in `tick_client_effects`).
    fn tick_darkness_blend(&mut self) {
        self.sculk.darkness = darkness_blend_tick(&self.potions.effects, self.sculk.darkness);
    }
    /// The darkness effect's blend factor at the partial tick.
    fn darkness_blend(&self, partial: f32) -> f32 {
        let (previous, current) = self.sculk.darkness;
        previous + (current - previous) * partial
    }
    /// The fog's view of the darkness effect, while it is on.
    pub(super) fn darkness_fog(&self) -> Option<f32> {
        self.potions
            .effects
            .get("darkness")
            .map(|_| self.darkness_blend(self.clock.fraction() as f32))
    }
    /// `LightmapRenderStateExtractor`: (brightness taken off the gamma
    /// option, the pulsing light scale).
    pub(super) fn darkness_lighting(&self, partial: f32) -> (f32, f32) {
        let scale = self.settings.file.double("darknessEffectScale") as f32;
        let modifier = self.darkness_blend(partial) * scale;
        let pulse =
            ((self.ticks as f32 - partial) * std::f32::consts::PI * 0.025).cos() * 0.45 * modifier;
        (modifier, pulse.max(0.0) * scale)
    }

    /// `LightmapRenderStateExtractor`: the night vision effect's intensity
    /// (`GameRenderer.nightVisionScale`), or conduit power's water vision.
    pub(super) fn night_vision_intensity(&self, partial: f32) -> f32 {
        use horizon_client::fog::FogType;
        if let Some(e) = self.potions.effects.get("night_vision") {
            // MobEffectInstance.endsWithin(200); -1 is infinite.
            return if e.duration < 0 || e.duration > 200 {
                1.0
            } else {
                0.7 + ((e.duration as f32 - partial) * std::f32::consts::PI * 0.2).sin() * 0.3
            };
        }
        if self.potions.effects.get("conduit_power").is_some()
            && let Some(eye) = self.eye_position()
        {
            let in_water = horizon_client::fog::fog_type(
                &self.simulation.world,
                self.res.facts(),
                eye,
                &self.near_plane(),
            ) == FogType::Water;
            return self.fog.water_vision(in_water);
        }
        0.0
    }

    /// `Warden.applyDarknessAround` / `MobEffectUtil.addEffectToPlayersAround`.
    pub(super) fn apply_darkness(&mut self, at: [f64; 3], radius: f64) {
        self.for_each_player(|g| g.darkness_on_player(at, radius));
    }
    fn darkness_on_player(&mut self, at: [f64; 3], radius: f64) {
        if self.creative || self.spectator() {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let d = [
            p.position.x - at[0],
            p.position.y - at[1],
            p.position.z - at[2],
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] >= radius * radius {
            return;
        }
        if let Some(active) = self.potions.effects.get("darkness")
            && active.duration > warden::DARKNESS_REAPPLY - 1
        {
            return;
        }
        let mut effect =
            horizon_client::effects::EffectInstance::new("darkness", warden::DARKNESS_DURATION, 0);
        effect.visible = false;
        self.potions.effects.add(effect);
    }

    // ---- catalysts ----

    /// `SculkCatalystBlockEntity.CatalystListener.handleGameEvent` for the
    /// death of a mob: the nearest catalyst within 8 blocks takes its
    /// experience. Returns whether it was consumed.
    pub(super) fn catalyst_takes_experience(
        &mut self,
        at: [f64; 3],
        experience: impl FnOnce(&mut Self) -> u32,
    ) -> bool {
        // GameEventDispatcher: catalysts are BY_DISTANCE listeners, handled
        // nearest first (exact distance to the block centre); a tie keeps the
        // order they were visited in: chunk x, chunk z, section y, then the
        // order they registered in. The first one takes the experience.
        let source = block_of(at);
        let mut candidates: Vec<(f64, (i32, i32, i32), usize, (i32, i32, i32))> = Vec::new();
        for (order, key) in self.sculk.index.iter().copied().enumerate() {
            if dist_sqr(source, key) > 64 {
                continue;
            }
            let is_catalyst = matches!(
                self.simulation.world.block_entities.get(&key),
                Some(BlockEntity::Sculk(e)) if e.kind == SculkKind::Catalyst
            );
            if is_catalyst {
                let c = center(key);
                let d = (at[0] - c[0]).powi(2) + (at[1] - c[1]).powi(2) + (at[2] - c[2]).powi(2);
                candidates.push((d, (key.0 >> 4, key.2 >> 4, key.1 >> 4), order, key));
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let nearest = candidates.first().map(|c| (c.3, ()));
        let Some((key, _)) = nearest else {
            return false;
        };
        let xp = experience(self);
        if xp > 0 {
            // sourcePosition.relative(UP, 0.5)
            let start = BlockPos::new(source[0], (at[1] + 0.5).floor() as i32, source[2]);
            if let Some(e) = self.sculk_entity(key) {
                e.spreader.add_cursors(start, xp as i32);
            }
        }
        self.catalyst_bloom(key);
        true
    }
    fn catalyst_bloom(&mut self, key: (i32, i32, i32)) {
        let pos = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| b.sculk_catalyst_bloom(level, ticks, pos));
        self.native_sculk_bloom(self.dim.id(), [key.0, key.1, key.2]);
        let _ = self.process_level_events();
        let c = center(key);
        let options = ParticleOptions::simple(ParticleType::SculkSoul);
        for _ in 0..2 {
            // Client-side (LevelEventHandler / particles): the client random.
            let r = self.particles.random();
            let p = [
                c[0] + r.next_gaussian() * 0.2,
                c[1] + 0.65,
                c[2] + r.next_gaussian() * 0.2,
            ];
            self.particles.add(&options, p, [0.0; 3]);
        }
        let pitch = 0.6 + self.simulation.world.random.next_float() * 0.4;
        self.audio
            .play("minecraft:block.sculk_catalyst.bloom", Some(c), 2.0, pitch);
    }

    // ---- particles ----

    /// `LevelEvent` 3006: charge cursors moving over sculk, or popping.
    pub(super) fn sculk_charge_event(&mut self, pos: BlockPos, data: i32) {
        let c = center((pos.x, pos.y, pos.z));
        let count = data >> 6;
        if count > 0 {
            // Client-side (LevelEventHandler / particles): the client random.
            let r = self.particles.random();
            if r.next_float() < 0.3 + count as f32 * 0.1 {
                let volume = 0.15 + 0.02 * (count * count) as f32 * r.next_float();
                let pitch = 0.4 + 0.3 * count as f32 * r.next_float();
                self.audio
                    .play("minecraft:block.sculk.charge", Some(c), volume, pitch);
            }
            let face_data = data & 63;
            let faces: Vec<(Direction, f32, f64)> = if face_data == 0 {
                Direction::ALL
                    .into_iter()
                    .map(|d| {
                        let roll = if d == Direction::Down {
                            std::f32::consts::PI
                        } else {
                            0.0
                        };
                        let factor = if matches!(d, Direction::Up | Direction::Down) {
                            0.65
                        } else {
                            0.57
                        };
                        (d, roll, factor)
                    })
                    .collect()
            } else {
                horizon_world::level::sculk::unpack_faces(face_data)
                    .into_iter()
                    .map(|d| {
                        let roll = if d == Direction::Up {
                            std::f32::consts::PI
                        } else {
                            0.0
                        };
                        (d, roll, 0.35)
                    })
                    .collect()
            };
            for (direction, roll, factor) in faces {
                let repetitions = self
                    .particles
                    .random()
                    .next_int_bound(count + 1)
                    .unwrap_or(0);
                for _ in 0..repetitions {
                    self.spawn_particle_on_face(pos, direction, roll, factor);
                }
            }
        } else {
            self.audio
                .play("minecraft:block.sculk.charge", Some(c), 1.0, 1.0);
            let solid = self
                .res
                .facts()
                .get(self.simulation.world.block(pos.x, pos.y, pos.z))
                .full_collision;
            let (particles, spread) = if solid { (40, 0.45f32) } else { (20, 0.25f32) };
            let options = ParticleOptions::simple(ParticleType::SculkChargePop);
            for _ in 0..particles {
                // Client-side (LevelEventHandler / particles): the client random.
                let r = self.particles.random();
                let v = [
                    2.0 * r.next_float() - 1.0,
                    2.0 * r.next_float() - 1.0,
                    2.0 * r.next_float() - 1.0,
                ];
                let at = [
                    c[0] + f64::from(v[0] * spread),
                    c[1] + f64::from(v[1] * spread),
                    c[2] + f64::from(v[2] * spread),
                ];
                let speed = [
                    f64::from(v[0] * 0.07),
                    f64::from(v[1] * 0.07),
                    f64::from(v[2] * 0.07),
                ];
                self.particles.add(&options, at, speed);
            }
        }
    }
    /// `ParticleUtils.spawnParticleOnFace` for a sculk charge.
    fn spawn_particle_on_face(&mut self, pos: BlockPos, face: Direction, roll: f32, factor: f64) {
        let c = center((pos.x, pos.y, pos.z));
        let step = face.normal();
        // Client-side (LevelEventHandler / particles): the client random.
        let r = self.particles.random();
        let mut jitter = || r.next_double() - 0.5;
        let mut at = [0.0; 3];
        for i in 0..3 {
            at[i] = c[i]
                + if step[i] == 0 {
                    jitter()
                } else {
                    f64::from(step[i]) * factor
                };
        }
        let mut speed = [0.0; 3];
        for i in 0..3 {
            let s = (r.next_double() * 2.0 - 1.0) * f64::from(0.005f32);
            speed[i] = if step[i] == 0 { s } else { 0.0 };
        }
        self.particles
            .add(&ParticleOptions::sculk_charge(roll), at, speed);
    }
    /// `LevelEvent` 3007: ten shrieks rising from the shrieker, and the shriek.
    pub(super) fn shriek_event(&mut self, pos: BlockPos) {
        let top = f64::from(pos.y) + 0.5;
        for i in 0..10 {
            self.particles.add(
                &ParticleOptions::shriek(i * 5),
                [f64::from(pos.x) + 0.5, top, f64::from(pos.z) + 0.5],
                [0.0; 3],
            );
        }
        let state = self.simulation.world.block(pos.x, pos.y, pos.z);
        if self.state_property(state, "waterlogged") != Some("true") {
            let pitch = 0.6 + self.particles.random().next_float() * 0.4;
            self.audio.play(
                "minecraft:block.sculk_shrieker.shriek",
                Some([f64::from(pos.x) + 0.5, top, f64::from(pos.z) + 0.5]),
                2.0,
                pitch,
            );
        }
    }

    fn vibration_particle(&mut self, from: [f64; 3], to: [f64; 3], ticks: i32) {
        self.particles.vibration(from, to, ticks);
    }
    /// A vibration travelling to a warden (`MobEvent::Vibration`).
    pub(super) fn mob_vibration_particle(&mut self, from: [f64; 3], to: [f64; 3], ticks: i32) {
        self.vibration_particle(from, to, ticks);
    }
    /// `Warden.clientDiggingParticles`: thirty crumbs of the block underfoot.
    pub(super) fn warden_dig_particles(&mut self, at: [f64; 3]) {
        let below = (
            at[0].floor() as i32,
            (at[1] - 0.2).floor() as i32,
            at[2].floor() as i32,
        );
        let state = self.simulation.world.block(below.0, below.1, below.2);
        let Some((sprite, tint)) = self.particle_sprite(state, [below.0, below.1, below.2]) else {
            return;
        };
        for _ in 0..30 {
            // Client-side (LevelEventHandler / particles): the client random.
            let r = self.particles.random();
            let off = |v: f32| f64::from(v * 1.4 - 0.7);
            let p = [
                at[0] + off(r.next_float()),
                at[1],
                at[2] + off(r.next_float()),
            ];
            let particle =
                horizon_client::particles::Particle::terrain(r, p, [0.0; 3], sprite.clone(), tint);
            self.particles.push(particle);
        }
    }

    // ---- entity events ----

    /// `Projectile.shoot` / `onHit`: a projectile of `owner` at a position.
    pub(super) fn projectile_event(
        &mut self,
        event: &'static str,
        owner: Option<Who>,
        at: [f64; 3],
    ) {
        let mut ctx = Context::default();
        if event == "minecraft:projectile_shoot" {
            // The shooter is the source of the shot.
            ctx = self.who_context(owner, None);
        } else {
            ctx.owner = owner;
            ctx.player = owner == Some(Who::Player) || self.rides_player(owner);
        }
        self.post_game_event(event, at, ctx);
    }
    /// The player shot or threw something.
    pub(super) fn player_projectile_shot(&mut self) {
        if let Some(p) = self.player.as_ref() {
            let at = [
                p.position.x,
                p.position.y + f64::from(self.eye),
                p.position.z,
            ];
            self.projectile_event("minecraft:projectile_shoot", Some(Who::Player), at);
        }
    }

    /// Events from entities moving, getting hurt and dying, derived from what
    /// changed this tick.
    fn tick_entity_events(&mut self) {
        self.tick_player_sculk_events();
        self.tick_mob_sculk_events();
        // Dropped items are entities too: Entity.applyEffectsFromBlocks.
        let items: Vec<[f64; 3]> = self
            .simulation
            .dropped
            .iter()
            .filter(|i| i.on_ground)
            .map(|i| [i.position.x, i.position.y, i.position.z])
            .collect();
        for at in items {
            self.step_on_redstone_ore(at, false, false);
        }
    }

    /// Entity.move emits on the authoritative server too, for each ServerPlayer.
    pub(super) fn tick_player_sculk_events(&mut self) {
        // The player.
        let player = self.player.as_ref().map(|p| {
            (
                [p.position.x, p.position.y, p.position.z],
                p.ground.on_ground,
                p.health.health(),
                p.flying,
            )
        });
        if let Some((at, on_ground, health, flying)) = player {
            let crouch = self.keys.contains(&KeyCode::ShiftLeft);
            if on_ground && !self.spectator() {
                self.step_on_redstone_ore(at, crouch, true);
            }
            self.mount_events(Who::Player, self.riding, true);
            let in_water = self.in_water_at(at);
            self.entity_motion_events(
                Who::Player,
                at,
                on_ground,
                in_water,
                crouch && on_ground,
                flying,
                false,
            );
            if let Some(last) = self.sculk.last_health
                && health < last
            {
                self.post_entity_event("minecraft:entity_damage", Who::Player);
            }
            self.sculk.last_health = Some(health);
        }
    }

    /// `RedStoneOreBlock.spawnParticles`: a red dust puff for each face of the
    /// ore that is not against a solid block. Non-axis coordinates draw from
    /// the client level random, x then y then z. It is `level.addParticle`, so
    /// only the local player's own click or step draws it, not a guest's turn.
    pub(super) fn redstone_ore_particles(&mut self, pos: [i32; 3]) {
        if self.lan_acting.is_some()
            || !matches!(
                &*self.block_name(pos),
                "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore"
            )
        {
            return;
        }
        // Direction.values(): down, up, north, south, west, east.
        const FACES: [[i32; 3]; 6] = [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ];
        for step in FACES {
            let next = [pos[0] + step[0], pos[1] + step[1], pos[2] + step[2]];
            let state = self.simulation.world.block(next[0], next[1], next[2]);
            if self.res.facts().get(state).solid_render {
                continue;
            }
            let mut at = [0.0f64; 3];
            for axis in 0..3 {
                at[axis] = if step[axis] != 0 {
                    0.5 + 0.5625 * f64::from(step[axis])
                } else {
                    f64::from(self.particles.level_random().next_float())
                };
            }
            self.particles.add(
                &horizon_client::particles::ParticleOptions::dust([1.0, 0.0, 0.0], 1.0),
                [
                    f64::from(pos[0]) + at[0],
                    f64::from(pos[1]) + at[1],
                    f64::from(pos[2]) + at[2],
                ],
                [0.0; 3],
            );
        }
    }

    /// `RedStoneOreBlock.stepOn`: standing on the ore lights it, unless the
    /// entity is stepping carefully (`attack_block` only lights an unlit ore).
    fn step_on_redstone_ore(&mut self, at: [f64; 3], carefully: bool, burst: bool) {
        if carefully {
            return;
        }
        // getOnPosLegacy: 0.2 below the feet.
        let pos = [
            at[0].floor() as i32,
            (at[1] - 0.2).floor() as i32,
            at[2].floor() as i32,
        ];
        self.sculk_prepare();
        let state = self.simulation.world.block(pos[0], pos[1], pos[2]);
        if self.state_is(&self.sculk.tables.redstone_ore, state) {
            // The dust is the local player's client effect, not a mob's or an item's.
            if burst {
                self.redstone_ore_particles(pos);
            }
            let _ = self.attack_block(pos);
        }
    }

    /// `RedStoneOreBlock.useItemOn`: a right-click lights the ore whatever the
    /// hand holds. False (PASS) when the hand holds a block item, which goes on
    /// to be placed; true (SUCCESS) otherwise.
    pub(super) fn use_redstone_ore(
        &mut self,
        pos: [i32; 3],
        held: Option<&horizon_client::items::ItemStack>,
    ) -> Result<bool> {
        if !matches!(
            &*self.block_name(pos),
            "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore"
        ) {
            return Ok(false);
        }
        self.redstone_ore_particles(pos);
        self.attack_block(pos)?;
        let block_item = held.is_some_and(|s| {
            self.res
                .items()
                .get(&s.item)
                .is_some_and(|i| i.block.is_some())
        });
        Ok(!block_item)
    }

    /// Every living mob in list order, and on the workers its `Steps` when
    /// it is quiet (`QuietView::quiet_steps`). Without workers none is
    /// classified, nor before the walk has built the tables: until then
    /// `post_game_event` tells nobody, and which event comes first decides that.
    fn mob_moves(&self) -> Vec<(MobMove, Option<Steps>)> {
        let mobs = &self.simulation.mobs;
        let regions = self.simulation.regions.regions().len();
        let executor = self
            .sculk
            .tables
            .built
            .then(|| self.simulation_server.executor(regions, mobs.len()))
            .flatten();
        let Some(executor) = executor else {
            return self
                .mob_slots(|m| m.alive())
                .into_iter()
                .map(|slot| (MobMove::of(&mobs[slot]), None))
                .collect();
        };
        let view = QuietView {
            world: &self.simulation.world,
            facts: self.res.facts(),
            tables: &self.sculk.tables,
            steps: &self.sculk.steps,
            mounts: &self.sculk.mounts,
            damaged: &self.sculk.damaged,
        };
        executor
            .map(mobs, |_, m| {
                m.alive().then(|| {
                    let mob = MobMove::of(m);
                    (mob, view.quiet_steps(&mob))
                })
            })
            .into_iter()
            .flatten()
            .collect()
    }
    /// What can make a classification stale: a block written, or a chunk
    /// come or gone.
    fn block_stamp(&self) -> (u64, u64) {
        let world = &self.simulation.world;
        (world.revision, world.chunks.membership_revision())
    }
    fn tick_mob_sculk_events(&mut self) {
        let classified_at = self.block_stamp();
        let mut classified = true;
        for (mob, quiet) in self.mob_moves() {
            if classified && let Some(steps) = quiet {
                self.sculk.steps.insert(Who::Mob(mob.id), steps);
                continue;
            }
            self.mob_sculk_events(mob);
            classified = classified && self.block_stamp() == classified_at;
        }
        let alive: FxHashSet<u32> = self.simulation.mobs.iter().map(|m| m.id).collect();
        self.sculk.damaged.retain(|id, _| alive.contains(id));
        self.sculk.steps.retain(|who, _| match who {
            Who::Player => true,
            Who::Mob(id) => alive.contains(id),
        });
        self.sculk.mounts.retain(|who, _| match who {
            Who::Player => true,
            Who::Mob(id) => alive.contains(id),
        });
    }
    /// One mob's turn in the walk.
    fn mob_sculk_events(&mut self, mob: MobMove) {
        let MobMove {
            id,
            at,
            on_ground,
            in_water,
            species,
            damaged,
            vehicle,
            age,
            resting,
        } = mob;
        if on_ground {
            self.step_on_redstone_ore(at, false, false);
        }
        // A mob first seen already riding raised its event when it spawned.
        let seen = vehicle.is_some() && self.sculk.steps.contains_key(&Who::Mob(id));
        self.mount_events(Who::Mob(id), vehicle, seen);
        let emits = !matches!(species, Species::Shulker);
        if emits {
            let warden = species == Species::Warden;
            self.entity_motion_events(Who::Mob(id), at, on_ground, in_water, false, false, warden);
            // Entity.processFlappingMovement: what flaps this move flaps
            // over air only (the mob's own `MobEvent::GameEvent` is not
            // read here).
            if flaps(species, resting, on_ground, age) {
                let below = self.simulation.world.block(
                    at[0].floor() as i32,
                    (at[1] - 1.0e-5).floor() as i32,
                    at[2].floor() as i32,
                );
                if self.res.facts().is_air(below) {
                    self.post_entity_event("minecraft:flap", Who::Mob(id));
                }
            }
        }
        if let Some(t) = damaged {
            let seen = self.sculk.damaged.insert(id, t);
            if seen != Some(t) {
                self.post_entity_event("minecraft:entity_damage", Who::Mob(id));
            }
        }
    }
    /// Where the vehicle `id` stands: a mob, boat, minecart or cushion.
    fn vehicle_position(&mut self, id: u32) -> Option<[f64; 3]> {
        let p = if let Some(m) = self.simulation.mob_index.get(&self.simulation.mobs, id) {
            m.position
        } else if let Some(b) = self.simulation.boats.iter().find(|b| b.id == id) {
            b.position
        } else if let Some(c) = self.simulation.carts.iter().find(|c| c.id == id) {
            c.position
        } else {
            self.simulation
                .cushions
                .iter()
                .find(|c| c.id == id)?
                .position
        };
        Some([p.x, p.y, p.z])
    }
    /// `Entity.startRiding` and `removeVehicle`: `entity_mount` at the new
    /// vehicle and `entity_dismount` at the old one, derived from the vehicle
    /// the rider has now and the one it had last tick. `announce` is false
    /// for a rider first seen riding.
    fn mount_events(&mut self, who: Who, vehicle: Option<u32>, announce: bool) {
        let last = self.sculk.mounts.get(&who).copied();
        let now = vehicle.and_then(|id| self.vehicle_position(id));
        if last.map(|l| l.0) != vehicle {
            if let Some((_, at)) = last {
                let ctx = self.who_context(Some(who), None);
                self.post_game_event("minecraft:entity_dismount", at, ctx);
            }
            if let (true, Some(at)) = (announce, now) {
                let ctx = self.who_context(Some(who), None);
                self.post_game_event("minecraft:entity_mount", at, ctx);
            }
        }
        // A vehicle that is gone keeps the spot it was last seen at.
        let kept = last.filter(|l| Some(l.0) == vehicle).map(|l| l.1);
        match (vehicle, now.or(kept)) {
            (Some(id), Some(at)) => {
                self.sculk.mounts.insert(who, (id, at));
            }
            _ if last.is_some() => {
                self.sculk.mounts.remove(&who);
            }
            _ => {}
        }
    }
    fn in_water_at(&self, at: [f64; 3]) -> bool {
        let (x, y, z) = (
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        );
        self.res
            .facts()
            .get(self.simulation.world.block(x, y, z))
            .fluid
            == horizon_client::blocks::Fluid::Water
    }
    /// `Entity.move`'s emission: footsteps, swimming, landing and splashes.
    fn entity_motion_events(
        &mut self,
        who: Who,
        at: [f64; 3],
        on_ground: bool,
        in_water: bool,
        discrete: bool,
        flying: bool,
        warden: bool,
    ) {
        self.sculk_prepare();
        let entry = (self.sculk.steps.entry(who)).or_insert(Steps::first(at, on_ground, in_water));
        let previous = *entry;
        entry.last = at;
        entry.on_ground = on_ground;
        entry.in_water = in_water;
        let (dx, dy, dz) = (
            at[0] - previous.last[0],
            at[1] - previous.last[1],
            at[2] - previous.last[2],
        );
        if touches_down(on_ground, previous.on_ground, dy, in_water) {
            self.post_entity_event("minecraft:hit_ground", who);
        }
        // Entity.updateInWaterStateAndDoFluidPushing: entering water splashes.
        if in_water && !previous.in_water {
            self.post_entity_event("minecraft:splash", who);
        }
        // Player.getMovementEmission.
        if flying || discrete {
            return;
        }
        let block = (
            at[0].floor() as i32,
            (at[1] - 0.2).floor() as i32,
            at[2].floor() as i32,
        );
        let state = self.simulation.world.block(block.0, block.1, block.2);
        let air = self.res.facts().is_air(state);
        let climbable = self.state_is(&self.sculk.tables.climbable, state);
        let travelled = travelled(dx, dy, dz, climbable);
        let Some(entry) = self.sculk.steps.get_mut(&who) else {
            return;
        };
        entry.move_dist += travelled;
        if entry.move_dist > entry.next_step && !air {
            let steps_on_block = on_ground || climbable;
            let move_dist = entry.move_dist;
            if steps_on_block {
                entry.next_step = if warden {
                    // Warden.nextStep
                    move_dist + 0.55
                } else {
                    (move_dist as i32).wrapping_add(1) as f32
                };
                let ctx = self.who_context(Some(who), Some(state));
                self.post_game_event("minecraft:step", at, ctx);
            } else if in_water {
                entry.next_step = (move_dist as i32).wrapping_add(1) as f32;
                self.post_entity_event("minecraft:swim", who);
            }
        }
    }
    /// `LivingEntity.die` with the catalyst's reach: a death event at the
    /// mob's position.
    pub(super) fn sculk_mob_died(&mut self, m: &Mob) {
        let ctx = self.context_of_mob(Some(m), m.id, None);
        self.post_game_event(
            "minecraft:entity_die",
            [m.position.x, m.position.y, m.position.z],
            ctx,
        );
    }
}

/// `MobEffects.DARKNESS`'s `setBlendDuration(22)`: its fade in, its fade out,
/// and how long before the end the fade out starts.
const DARKNESS_BLEND: i32 = 22;

/// `MobEffectInstance.BlendState.hasEffect` for the darkness effect.
fn darkness_target(e: &EffectInstance) -> f32 {
    if e.ends_within(DARKNESS_BLEND) {
        0.0
    } else {
        1.0
    }
}

/// `MobEffectInstance.BlendState.tick` for the darkness effect: from the
/// (previous, current) factors, the next ones. It fades in and out over 22
/// ticks; with no darkness it is gone.
pub(super) fn darkness_blend_tick(effects: &PlayerEffects, (_, current): (f32, f32)) -> (f32, f32) {
    let next = match effects.get("darkness") {
        Some(e) => {
            let step = 1.0 / DARKNESS_BLEND as f32;
            current + (darkness_target(e) - current).clamp(-step, step)
        }
        None => 0.0,
    };
    (current, next)
}

/// A new darkness instance's `BlendState`: from nothing, or, when the server
/// asked for no blending, at once (`MobEffectInstance.skipBlending`'s
/// `BlendState.setImmediate`).
pub(super) fn darkness_blend_added(e: &EffectInstance, blend: bool) -> (f32, f32) {
    let factor = if blend { 0.0 } else { darkness_target(e) };
    (factor, factor)
}

/// `Bee.isFlapping` (flying, every other tick) and `Bat.isFlapping`
/// (awake, every tenth tick). Bird distance thresholds run in mob movement.
fn flaps(species: Species, resting: bool, on_ground: bool, age: u32) -> bool {
    match species {
        Species::Bee => !on_ground && age.is_multiple_of(2),
        Species::Bat => !resting && age.is_multiple_of(10),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wardens that hear an event are those in the list when it is
    /// posted, however the list changed since the last one.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn wardens_hear_events_however_the_list_changed_between_them() {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.sculk_prepare();
        let warden = game
            .res
            .mob_types()
            .get("minecraft:warden")
            .cloned()
            .unwrap();
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .cloned()
            .unwrap();
        let at = |x| horizon_core::geometry::Vec3::new(x, 100.0, 0.0);
        let post = |game: &mut Game<'_>| {
            game.post_game_event("minecraft:explode", [0.5, 100.5, 0.5], Context::default());
            let later = game.simulation.world.game_time + 1;
            let heard = |m: &mut Mob| {
                let warden = m.extra.warden.as_mut()?;
                let heard = warden.vibration.selector.chosen_candidate(later);
                warden.vibration = Default::default();
                heard
            };
            let mut ids: Vec<u32> = game
                .simulation
                .mobs
                .iter_mut()
                .filter_map(|m| heard(m).map(|_| m.id))
                .collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(post(&mut game), [0u32; 0], "nobody to hear it");
        assert_eq!(post(&mut game), [0u32; 0]);

        game.simulation
            .mobs
            .push(Mob::new(1, zombie.clone(), at(1.0), 0.0));
        game.simulation
            .mobs
            .push(Mob::new(2, warden.clone(), at(2.0), 0.0));
        assert_eq!(post(&mut game), [2], "one that joined since");
        assert_eq!(post(&mut game), [2], "and again with nothing changed");

        game.simulation
            .mobs
            .insert(0, Mob::new(3, warden.clone(), at(3.0), 0.0));
        assert_eq!(post(&mut game), [2, 3], "the slots moved");

        let level = std::mem::take(&mut game.simulation.mobs);
        assert_eq!(post(&mut game), [0u32; 0], "another level's list");
        game.simulation.mobs = level;
        assert_eq!(post(&mut game), [2, 3], "this level's again");

        game.simulation.mobs.retain(|m| m.id != 3);
        assert_eq!(post(&mut game), [2], "one that left");

        // A write that keeps every slot and makes the zombie's a listener.
        game.simulation.mobs[0] = Mob::new(1, warden, at(1.0), 0.0);
        assert_eq!(post(&mut game), [1, 2], "one replaced in its slot");

        game.simulation.mobs[0].removed = true;
        game.simulation.mobs[0].health = 0.0;
        assert_eq!(post(&mut game), [2], "the dead hear nothing");
    }

    /// One tick of a scripted mob.
    #[derive(Clone, Copy)]
    struct Pose {
        at: [f64; 3],
        ground: bool,
        water: bool,
        vehicle: Option<u32>,
        hurt: Option<i64>,
        age: u32,
    }
    fn pose(x: f64, y: f64, z: f64, ground: bool) -> Pose {
        Pose {
            at: [x, y, z],
            ground,
            water: false,
            vehicle: None,
            hurt: None,
            age: 1,
        }
    }
    struct Scripted {
        id: u32,
        kind: &'static str,
        make: fn(&mut Mob),
        poses: [Pose; 4],
    }
    fn scripted(id: u32, kind: &'static str, poses: [Pose; 4]) -> Scripted {
        Scripted {
            id,
            kind,
            make: |_| {},
            poses,
        }
    }
    /// A fresh level holding `blocks` and the script's mobs.
    fn stage(
        game: &mut Game<'_>,
        workers: usize,
        blocks: &[((i32, i32, i32), BlockStateId)],
        script: &[Scripted],
    ) {
        game.simulation_server.force_workers(workers);
        let air = game.res.facts().air;
        game.simulation.world = horizon_client::world::World::new(air, 0);
        let chunk = horizon_client::chunk::Chunk::empty(0, 0);
        game.simulation
            .world
            .chunks
            .insert((0, 0), std::sync::Arc::new(chunk));
        for &((x, y, z), state) in blocks {
            game.simulation.world.set(x, y, z, state).unwrap();
        }
        let revision = game.simulation.world.chunks.membership_revision();
        game.simulation
            .regions
            .rebuild(revision, std::iter::once((0, 0)));
        game.simulation.mobs = Default::default();
        for s in script {
            let kind = game.res.mob_types().get(s.kind).cloned().unwrap();
            let at = horizon_core::geometry::Vec3::new(0.0, 100.0, 0.0);
            let mut m = Mob::new(s.id, kind, at, 0.0);
            (s.make)(&mut m);
            game.simulation.mobs.push(m);
        }
        game.sculk.steps.clear();
        game.sculk.mounts.clear();
        game.sculk.damaged.clear();
        let regions = game.simulation.regions.regions().len();
        let has_executor = game
            .simulation_server
            .executor(regions, script.len())
            .is_some();
        assert_eq!(has_executor, workers > 0);
    }
    /// Puts every mob where the script has it this tick.
    fn pose_mobs(game: &mut Game<'_>, script: &[Scripted], tick: usize) {
        for s in script {
            let p = s.poses[tick];
            let m = (game.simulation.mobs.iter_mut())
                .find(|m| m.id == s.id)
                .unwrap();
            m.position = horizon_core::geometry::Vec3::new(p.at[0], p.at[1], p.at[2]);
            m.on_ground = p.ground;
            m.fluids.in_water = p.water;
            m.extra.mount = p.vehicle;
            m.age = p.age;
            m.last_damage = p.hurt.map(|t| (horizon_client::mobs::Damage::fall(), t));
        }
    }
    fn steps_text(game: &Game<'_>) -> Vec<String> {
        let mut steps: Vec<String> = (game.sculk.steps.iter())
            .map(|(who, s)| format!("{who:?} {s:?}"))
            .collect();
        steps.sort();
        steps
    }
    /// The events and every tick's `Steps` of the script, on `workers`.
    fn run_script(
        game: &mut Game<'_>,
        workers: usize,
        blocks: &[((i32, i32, i32), BlockStateId)],
        script: &[Scripted],
    ) -> (Vec<String>, Vec<Vec<String>>) {
        stage(game, workers, blocks, script);
        super::super::dispense_fixture::start_recording();
        let mut steps = Vec::new();
        for tick in 0..4 {
            pose_mobs(game, script, tick);
            game.tick_mob_sculk_events();
            steps.push(steps_text(game));
        }
        (super::super::dispense_fixture::stop_recording(), steps)
    }

    /// What workers call quiet stores what the walk's own code stores, and
    /// the rest raises the same events in the same order, whatever the mob
    /// is doing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn pool_and_serial_raise_the_same_events_and_keep_the_same_steps() {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.sculk_prepare();
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (stone, ladder, water, ore) = (
            state("minecraft:stone"),
            state("minecraft:ladder"),
            state("minecraft:water"),
            state("minecraft:redstone_ore"),
        );

        let mut blocks = Vec::new();
        // Rows of floor; 4, 13 and 14 hang over air.
        for z in [0, 1, 2, 5, 7, 8, 9, 10, 11, 12, 15] {
            blocks.extend((0..16).map(|x| ((x, 99, z), stone)));
        }
        blocks.extend((100..108).map(|y| ((2, y, 3), ladder)));
        blocks.extend((0..16).map(|x| ((x, 100, 6), water)));
        blocks.push(((8, 99, 8), ore));
        blocks.push(((1, 99, 15), ore));

        let walk = |z: f64, ground: bool, y: f64, xs: [f64; 4]| xs.map(|x| pose(x, y, z, ground));
        let still = |z: f64| walk(z, true, 100.0, [1.5; 4]);
        let far = [1.5, 3.5, 5.5, 7.5];
        let mut water_row = walk(6.5, false, 100.2, [1.5, 1.5, 3.5, 5.5]);
        for p in &mut water_row[1..] {
            p.water = true;
        }
        let mut riding = still(9.5);
        riding[1].vehicle = Some(2);
        riding[2].vehicle = Some(2);
        let mut first_seen_riding = still(10.5);
        first_seen_riding[0].vehicle = Some(2);
        first_seen_riding[1].vehicle = Some(2);
        let mut hurt = still(11.5);
        hurt[1].hurt = Some(50);
        hurt[2].hurt = Some(50);
        hurt[3].hurt = Some(60);
        let mut bee = still(13.5).map(|p| Pose { ground: false, ..p });
        for (tick, p) in bee.iter_mut().enumerate() {
            p.age = tick as u32;
        }
        let mut bat = still(14.5).map(|p| Pose { ground: false, ..p });
        for (p, age) in bat.iter_mut().zip([10, 11, 20, 21]) {
            p.age = age;
        }
        let mut landing = still(7.5);
        landing[0] = pose(1.5, 103.0, 7.5, false);
        let script = [
            // Walks too little to step; then one that steps.
            scripted(
                1,
                "minecraft:zombie",
                walk(1.5, true, 100.0, [1.5, 1.6, 1.7, 1.8]),
            ),
            scripted(2, "minecraft:zombie", walk(2.5, true, 100.0, far)),
            // Climbing: its height counts and the ladder takes the step.
            scripted(
                3,
                "minecraft:zombie",
                [
                    pose(2.5, 100.5, 3.5, false),
                    pose(2.5, 102.5, 3.5, false),
                    pose(2.5, 104.5, 3.5, false),
                    pose(2.5, 106.5, 3.5, false),
                ],
            ),
            // A step due, with air below and with a block below off the ground.
            scripted(4, "minecraft:zombie", walk(4.5, false, 100.0, far)),
            scripted(5, "minecraft:zombie", walk(5.5, false, 100.1, far)),
            scripted(6, "minecraft:zombie", water_row),
            scripted(7, "minecraft:zombie", landing),
            // Onto the ore on the third tick: the write comes mid-pass.
            scripted(
                8,
                "minecraft:zombie",
                walk(8.5, true, 100.0, [1.5, 1.5, 8.5, 8.5]),
            ),
            scripted(9, "minecraft:zombie", riding),
            scripted(10, "minecraft:zombie", first_seen_riding),
            scripted(11, "minecraft:zombie", hurt),
            scripted(12, "minecraft:shulker", still(12.5)),
            scripted(13, "minecraft:bee", bee),
            scripted(14, "minecraft:bat", bat),
            Scripted {
                id: 15,
                kind: "minecraft:zombie",
                make: |m| m.health = 0.0,
                poses: walk(15.5, true, 100.0, [1.5; 4]),
            },
            scripted(16, "minecraft:zombie", still(0.5)),
            // Awake, where 14 has rested since it spawned.
            Scripted {
                id: 17,
                kind: "minecraft:bat",
                make: |m| horizon_client::mobs::bat::set_resting(m, false),
                poses: bat,
            },
        ];

        let serial = run_script(&mut game, 0, &blocks, &script);
        // By hand: what the script above must raise.
        let count = |event: &str| (serial.0.iter()).filter(|l| l.ends_with(event)).count();
        assert_eq!(
            count(" minecraft:step"),
            7,
            "2, 3 and the ore: {:#?}",
            serial.0
        );
        assert_eq!(count(" minecraft:splash"), 1);
        assert_eq!(count(" minecraft:swim"), 2);
        assert_eq!(count(" minecraft:hit_ground"), 1);
        assert_eq!(count(" minecraft:entity_mount"), 1);
        assert_eq!(count(" minecraft:entity_dismount"), 2);
        assert_eq!(count(" minecraft:entity_damage"), 2);
        assert_eq!(count(" minecraft:flap"), 4, "the bee and the awake bat");
        let lit = game.simulation.world.block(8, 99, 8);
        assert_ne!(lit, ore, "the mob on the ore lit it");
        assert_eq!(
            game.simulation.world.block(1, 99, 15),
            ore,
            "the dead don't"
        );
        assert!(
            !serial.1[3].iter().any(|s| s.contains("Mob(12)")),
            "no shulker steps"
        );
        assert!(
            !serial.1[3].iter().any(|s| s.contains("Mob(15)")),
            "nor the dead"
        );

        for workers in [1, 4] {
            let pool = run_script(&mut game, workers, &blocks, &script);
            assert_eq!(pool.0, serial.0, "events on {workers} workers");
            assert_eq!(pool.1, serial.1, "steps on {workers} workers");
        }

        // A block changed mid-pass. Lighting the ore makes it climbable here,
        // so the mob hanging over it, called quiet before the write, now steps.
        let lit_ore = (0..registry.len() as u32)
            .map(BlockStateId)
            .find(|&id| {
                &*game.res.facts().get(id).name == "minecraft:redstone_ore"
                    && registry
                        .state(id)
                        .is_some_and(|s| s.properties.get("lit").is_some_and(|v| v == "true"))
            })
            .unwrap();
        game.sculk.tables.climbable[lit_ore.0 as usize] = true;
        let mut blocks: Vec<_> = (0..16).map(|x| ((x, 99, 2), stone)).collect();
        blocks.push(((4, 99, 2), ore));
        let lights = [(1.5, 100.0, true), (4.5, 100.0, true)];
        let hangs = [(2.5, 100.1, false), (4.5, 100.1, false)];
        let row = |ticks: [(f64, f64, bool); 2]| {
            let [a, b] = ticks.map(|(x, y, ground)| pose(x, y, 2.5, ground));
            [a, b, b, b]
        };
        let script = [
            scripted(1, "minecraft:zombie", row(lights)),
            scripted(2, "minecraft:zombie", row(hangs)),
            scripted(3, "minecraft:zombie", still(2.5)),
        ];
        let serial = run_script(&mut game, 0, &blocks, &script);
        let steps = (serial.0.iter()).filter(|l| l.as_str() == "G 4,100,2 minecraft:step");
        assert_eq!(
            steps.count(),
            2,
            "both, on the second tick: {:#?}",
            serial.0
        );
        for workers in [1, 4] {
            let pool = run_script(&mut game, workers, &blocks, &script);
            assert_eq!(pool.0, serial.0, "events on {workers} workers");
            assert_eq!(pool.1, serial.1, "steps on {workers} workers");
            // The classification this guards against, on that tick.
            stage(&mut game, workers, &blocks, &script);
            pose_mobs(&mut game, &script, 0);
            game.tick_mob_sculk_events();
            pose_mobs(&mut game, &script, 1);
            let moves = game.mob_moves();
            assert!(moves[0].1.is_none(), "the mob on the ore");
            assert!(moves[1].1.is_some(), "called quiet before the write");
        }
        game.sculk.tables.climbable[lit_ore.0 as usize] = false;
    }

    /// Retaining keeps the insertion order and the membership in step.
    #[test]
    fn the_index_retains_in_order() {
        let mut index = Index::default();
        for key in [(5, 0, 0), (40, 1, 0), (6, 2, 17), (-3, 3, 0), (41, 4, 0)] {
            index.insert(key);
        }
        index.retain(|p| p.0 >> 4 != 2 && p.2 >> 4 != 1);
        assert_eq!(index.order, [(5, 0, 0), (-3, 3, 0)]);
        assert!(index.contains(&(5, 0, 0)) && !index.contains(&(41, 4, 0)));
        assert_eq!(index.set.len(), 2);
        index.insert((41, 4, 0));
        assert_eq!(index.iter().last(), Some(&(41, 4, 0)));
    }

    /// The listeners near a point come out as filtering the whole index in
    /// order would give them, through inserts, removals and retains.
    #[test]
    fn listeners_within_a_radius_are_the_filtered_index_in_order() {
        let mut index = Index::default();
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % (2 * n as u64 + 1)) as i32 - n
        };
        for step in 0..3000 {
            let key = (next(70), next(40), next(70));
            match step % 7 {
                0..=3 => index.insert(key),
                4..=5 => {
                    let picked = index
                        .order
                        .get(next(1000).unsigned_abs() as usize % index.order.len().max(1))
                        .copied();
                    if let Some(k) = picked {
                        index.remove(&k);
                    }
                }
                _ if step % 500 == 6 => index.retain(|p| (p.0 ^ p.2) & 3 != 0),
                _ => {}
            }
            let at = [next(80), next(50), next(80)];
            let radius = next(16).abs();
            let expected: Vec<_> = index
                .iter()
                .copied()
                .filter(|p| dist_sqr(at, *p) <= i64::from(radius * radius))
                .collect();
            assert_eq!(
                index.within(at, radius),
                expected,
                "step {step} at {at:?} r {radius}"
            );
        }
        assert!(index.cells.values().map(Vec::len).sum::<usize>() == index.order.len());
    }

    /// A bat flaps every tenth tick while it is not resting, whether it is on
    /// the ground or not; a resting bat never flaps and nothing else does.
    #[test]
    fn a_bat_flaps_every_tenth_tick_unless_it_is_resting() {
        assert!(flaps(Species::Bat, false, false, 0));
        assert!(flaps(Species::Bat, false, true, 10));
        assert!(!flaps(Species::Bat, false, false, 11));
        assert!(!flaps(Species::Bat, true, false, 20), "resting");
        assert!(flaps(Species::Bee, false, false, 4));
        assert!(!flaps(Species::Bee, false, true, 4), "a bee on the ground");
        assert!(!flaps(Species::Bat, false, false, 5));
        assert!(!flaps(Species::Cow, false, false, 0));
    }

    #[test]
    fn centres_and_blocks_agree() {
        assert_eq!(center((1, 2, 3)), [1.5, 2.5, 3.5]);
        assert_eq!(block_of([1.5, 2.5, -0.5]), [1, 2, -1]);
        assert_eq!(dist_sqr([0, 0, 0], (3, 4, 0)), 25);
    }
}
