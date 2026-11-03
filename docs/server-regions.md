# Server simulation ownership and region execution

This is an incomplete foundation for #8645. It does **not** implement parallel
mob ticks or resolve the reported roughly 150 ms mob phase with 100 players.
The gameplay phase stays serial. Snapshot projection has an opt-in bounded
worker path; its synthetic benchmark regresses, so serial is the default.

## Owners and the tick barrier

`game/simulation.rs::LevelSimulation` owns the level's ordered mobs, network-id
index, death bookkeeping, natural-spawn scratch, region directory and worker
configuration. `Game` and `levels::Parked` each hold the same owner; a dimension
swap moves the entire structure. Death bookkeeping also follows an entity's
cross-dimension transfer. Immutable game tables remain in `Resources`, and
`MobWorld` lends those tables and a read-only world to the serial entity phase.
World/chunk mutation, other entity types and server-wide coordination still live
in `Game`; extracting those owners is outstanding.

`server_regions::Regions` groups eight-connected **resident** chunks. Its ID is
the minimum chunk coordinate in the component and is valid until a topology
rebuild. A loaded bridge merges components; an unloaded bridge splits them.
The chunk-map membership revision invalidates the topology, including a reset
or replacement of the map. Ordinary block writes do not rebuild topology.
Connectivity defines ownership only. It does not prove simulation independence.

At the start of the mob entity phase, after native pre-tick callbacks:

1. Refresh topology and index each mob and player into its resident owner.
2. Preserve input slots in every region's local index. Dormant entities in
   unloaded chunks keep separate slots; they do not load or retain chunks.
3. Prepare immutable `Seen` snapshots, using the original global slot order.
4. Join every worker before exposing snapshots to the entity tick loop.
5. Execute that loop in its original global order. Movement, pushes and brain
   writes update the shared snapshot immediately, as before. Migration is
   reindexed at the next barrier; the directory is not queried during movement.

No entity is ticked by iterating regions. Concatenating region lists would
reorder interleaved joins. Unloaded entities retain the existing serial despawn
rules. New spawns, deaths and portal transfers become index changes at the next
barrier, without altering their current save/network lifecycle. Streaming and
unloading run outside the immutable projection; workers cannot retain a world
reference after joining. No world or chunk contents are cloned for this work.

## Dependency audit and current execution

This table records current code constraints, not a claim of complete vanilla
independence analysis. No fixed halo, maximum speed assumption, or new deferred
cross-region event queue is used as a correctness argument.

| Dependency | Evidence / risk | Current coordination |
| --- | --- | --- |
| Level RNG | `mobs::ai::tick`, brain duration draws, bee ticks, school ticks and block contact consume the same `LegacyRandom`. Draw counts depend on state. | One serial scratch RNG, restored to the level in original order. No per-region seed or cloned RNG streams. |
| Mob observations | `others_near`, `seen(id)` and direct full-snapshot scans include remote targets, owners, caravans and raid members. | One globally ordered snapshot, updated after every mob. Local region indexes do not filter these queries. |
| Immediate mob events | `apply_brain_events` changes partners, gaze, retreats, riders, school membership and breeding cooldowns before later mobs tick. `PendingPushes` changes later movement. | Apply in the existing loop, immediately and in order. |
| Native mods | Pre/post tick callbacks, canceled ticks and content effect callbacks may have arbitrary dependencies. | Existing server-thread callbacks; no worker invocation. Goal objects now require `Send + Sync` for immutable snapshot borrowing. Custom native goals must satisfy those bounds. |
| Projectiles and vehicles | Homing target IDs, passenger chains, hit selection and entity join order can span owners; #8639 separately changes projectile join ordering. | Existing ordered phases and rider synchronization. No new projectile/vehicle schedule. |
| Explosions and damage | An explosion can damage entities and change blocks beyond any selected chunk boundary. Damage can trigger immediate deaths and drops. | Existing serial event, damage, explosion and loot paths. |
| Block changes and navigation | `path_writes` is drained once; collision changes are derived from the shared world; an earlier tick may change a later mob's observed state. | Existing invalidation and navigation path. Projection workers do not navigate or write blocks. |
| Natural/custom spawning | Global and per-player caps, spawn order, IDs, pack finalization and shared RNG affect later spawns. | Existing pre-entity spawning and raids; the scratch cache now follows its level owner. |
| Despawning and drops | Entity-ticking tickets, nearest players, loot RNG, corpse removal and death effects are ordered. | Existing despawn/death/drop handling; no new delayed effects. |
| Replication | Clients must not receive partially prepared state. | Projection barrier joins before any gameplay or publication. Existing LAN event/replication phases remain authoritative. |

`Access::{Read,Write,Global}` and `batches` provide a conservative capability
planner: conflicting jobs cannot pass each other, and global dependencies force
singleton batches. It is tested but is **not yet a full-mob scheduling API**.
Proving bounded footprints, or implementing validated transactions for unbounded
queries and level RNG, remains necessary before scheduling whole regions.

## Worker path and validation

`HORIZON_REGION_SNAPSHOT_WORKERS=2` through `8` opts into scoped snapshot workers;
invalid values fall back to one, and counts clamp to 1–8. Each worker owns its
results and borrows immutable entities. There are no world clones or locks in
this projection. All threads join, including on panic. These are scoped worker
threads created per projection, not a persistent server worker pool. A persistent
pool and the full spawning/AI/navigation/movement/damage/drop/replication pipeline
on independently mutable region owners remain outstanding.

The unit tests exercise connectivity, bridge merge/split, diagonal/negative and
extreme chunk coordinates, ordered migration, removal, unload, dormant entities,
serial/parallel projection equivalence, bounded overlap, join publication,
panic joins, and read/write/global conflict ordering.

## Quick benchmark (2026-11-03)

Reproduce without game resources:

```sh
rustc --edition=2024 -O tools/bench_server_regions.rs -o /tmp/region-bench
/tmp/region-bench baseline
/tmp/region-bench serial
/tmp/region-bench parallel
```

Same inputs: 100 disconnected 17×17 loaded areas, 70 synthetic entities per
player (7,000 total), 384-byte output per entity. 50 warmup iterations followed
by 300 measured samples, with output equality checked every iteration. Region
modes include rebuilding local indexes; topology is built once before timing.
CPU and RSS below come from `wait4` for each separate process (including setup).
This measures directory/projection overhead, **not server or mob tick time**.

| Mode | Median ms | p95 ms | p99 ms | CPU (100% = one core) | Max RSS KiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Original serial copy | 0.103 | 0.126 | 0.212 | 96.1% | 9,512 |
| Region index + serial copy | 0.156 | 0.166 | 0.201 | 99.5% | 9,300 |
| Region index + 8 scoped workers | 0.476 | 0.627 | 0.894 | 207.4% | 19,408 |

Initial topology construction took 7.8–8.4 ms. The projection benchmark gives no
reason to enable workers by default. An identical live 100-player baseline and
changed server benchmark, with whole-tick median/p95/p99, CPU and memory, is still
required. These numbers must not be reported as a speedup of the mob pipeline.
