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

### Actual mob snapshot check

`tools/bench_mob_snapshot.rs` also measures the real `Mob::seen` function for
7,000 constructed zombies, cows and villagers in the same 100-region layout,
with 100 synthetic player coordinates. It compares every final snapshot field
against the original serial projection outside the timing. This is still only
immutable preparation: it runs no AI, movement, damage or network clients.

Build `horizon-client-lib`, then link the standalone driver against that build's
rlib and dependency directory (the source header gives the command). Run each
mode with the prepared resource root as its second argument. The quick run used
20 warmups and 100 samples, with the same inputs and resources in all modes:

| Actual `Mob::seen` mode | Median ms | p95 ms | p99 ms | CPU | Max RSS KiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| Original serial method | 0.670 | 1.553 | 1.946 | 95.0% | 52,432 |
| Region index + serial method | 0.642 | 0.711 | 0.857 | 99.5% | 52,888 |
| Region index + 8 scoped workers | 0.834 | 1.582 | 1.929 | 204.2% | 61,924 |

Every snapshot comparison passed. This single sweep has substantial timing
variation; the small serial difference is not evidence of a speedup. It again
gives no reason to enable workers by default, and is not the outstanding live
100-player tick benchmark.

## Validation results

After clearing the disk space that interrupted the first build:

- `tools/pr_ready.py --quick`: passed, including 2,442 Rust tests, 627 Python
  tests (one skipped), rustfmt, clippy, public tree and doc-test checks. The
  runner lists 166 affected parity harnesses for CI; this is not a full parity pass.
- `test_mob_ai_parity.py --only zombie_player cow_alone villager_village`:
  all three scenarios and 703 mob ticks matched vanilla.
- `test_natural_spawn_parity.py`: 15,500 cases matched vanilla, including
  98,686 spawn attempts and 10,730 spawns.
- `test_dedicated_server_parity.py`: all 601 transcript lines matched, including
  client connection, spawn, chunks, chat and `/list` checks.
- `test_server_entities_parity.py`: all 34 entities matched.
- `test_server_player_damage_parity.py`: both client and server compared
  373,455 output lines successfully.
- `lan_e2e.py --check mobs`: host/guest zombie and skeleton combat passed using
  freshly built binaries from this commit and disposable data directories.
- `port_ledger.py check --base origin/main` ran and reported the existing ledger
  gaps in the mechanically touched functions; no parity port was claimed.

The initial unrestricted mob-AI run was interrupted after disk errors; only the
three scenarios above are claimed as passing. Full parallel gameplay validation
and a live before/after 100-player benchmark remain outstanding.
