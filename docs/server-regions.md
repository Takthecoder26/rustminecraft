# Server simulation ownership and region execution

The server runs region-owned mob transactions on a bounded persistent pool,
then commits them in the original entity order. Independence is checked from
actual observations and shared RNG checkpoints. A region boundary never hides
a target, collision candidate, owner, projectile, or player.

## Ownership and lifecycle

`game/simulation.rs::LevelSimulation` owns the mutable world/chunks, ordered
entity families (mobs, projectiles, vehicles, items, experience and decorations),
block tick queues, chunk tickets, spawn scratch, death bookkeeping, region
indexes and outgoing level/entity events. `Game` and `levels::Parked` hold this
same owner. Dimension swaps move it as one value. Existing typed raid, sculk,
portal, streaming and save services retain their existing coordinated phases
and dimension-specific swaps. Read-only registries, collision tables, recipes,
biomes and mob types are borrowed from `Resources`/`PackResources`.

`ServerCoordination` owns the dimension manager, server-wide entity IDs, the
entity admission cutoff and one lazy persistent pool shared by all levels.
The mutable level owner is `Send`; the immutable `TickResources` contract is
`Send + Sync`. Native callbacks are excluded from worker inputs.

`server_regions::Regions` groups eight-connected resident chunks. Its identity
is the minimum chunk coordinate in the component, valid until topology changes.
A loaded bridge merges regions; unloading it splits them. The chunk-map
membership revision also detects resets and replacement. Block changes alone
do not rebuild topology. Resident connectivity defines ownership, not proof
that two simulations commute.

Every entity retains its global insertion slot. Region-local entity and player
indexes preserve the filtered order of those slots. At the tick barrier an
entity belongs to exactly one resident component, or to the dormant list when
its chunk is unloaded. Movement may cross owners during the tick; ownership is
frozen until the next barrier, when migration, removals and new entities are
indexed together. Migration never loads or retains a chunk. Existing admission,
entity-ticking ticket, despawn, portal, persistence and save rules remain in
charge. Concatenating region lists would change interleaved entity ordering;
the coordinator always walks the original global list.

## Transaction and publication barrier

1. Complete the existing spawn/raid and native pre-tick phases. Refresh topology
   and local indexes, and build the ordered `Seen` snapshot and spatial grid.
2. Reserve level-owned EntitySection insertion order in global entity order.
   Build immutable player indexes and per-mob difficulty/despawn contexts.
3. Give each job exclusive ownership of its mutable mob inputs. Workers borrow
   world/chunk/resources; they do not clone the world or its chunks. A sparse
   overlay contains only that region's updated `Seen` entries and grid cells.
4. Tick each region in its original local order, including AI, navigation,
   effects, movement, collision, damage and despawn decisions. Same-region
   pushes and single-target brain writes reach later local inputs immediately.
5. Join all jobs. Walk global entity slots and validate each prediction's
   observations, same-region prefix and level RNG checkpoint. Commit an
   unchanged transaction by moving its resulting mob/events into place.
   Otherwise run that mob serially against the current authoritative inputs
   and invalidate the rest of that owner's predicted prefix.
6. Apply immediate pushes and brain events at the existing point in that walk.
   Complete deaths, loot, event effects, projectiles and replication in their
   original coordinated phases. Clients only see completed authoritative state.

No new cross-region delayed-event queue is used. A replay has no externally
visible speculative events. The full mutable-mob tests compare every field,
event sequence and final RNG against the retained serial execution, including
breeding, overlapping owners and nectar-bearing bees that consume level RNG.
The serial reference also runs against a complete snapshot without the spatial
index, checking that indexed execution preserves the filtered results.

`SeenView` prevents untracked raw-slice observations. ID lookups record slots;
full scans record a complete-snapshot read; spatial queries record exact grid
columns, including empty columns. An earlier foreign entity entering an empty
queried column invalidates a prediction. Bounds growth, retained empty cells
and a change between indexed and full-scan queries also invalidate it. No
fixed margin or maximum entity speed is assumed. Sparse grid overlays preserve
the original candidate order and legacy full-scan fallback rules.

Audited box-filtering callers use `others_intersecting`, which enforces the
exact intersection predicate at its API. Position-range searches retain their
inclusive range predicate. For these bounded queries, a sparse lookup scans
populated cells when that is cheaper than walking empty columns, preserving
global slot order without declaring the entire snapshot observed. The query
extent comes from the requested geometry and actual maximum body width, not a
region margin. Validation likewise scans changed populated cells for very large
areas, so an enormous query cannot cause an unbounded empty-cell walk. Legacy
candidate-only callers without an equivalent geometric predicate retain their
existing scan and conservative dependency rules.

Species-filtered observations have their own ordered index and read set.
Zombie village searches and villager POI ticket scans can read all villagers
without falsely depending on unrelated zombies. A foreign member's changed
state, removal, or a change into/out of the queried species invalidates that
read, including a query that originally found nobody. Same-region membership
changes are supplied by the sparse overlay in global slot order. Genuine global
villager POI dependencies therefore remain coordinated through validation and
replay; an empty villager population does not serialize every zombie region.

The player directory uses exact distance searches over all players. Local
players seed the search, but pruning uses distance bounds over the complete
tree. Original host/guest tie order, dead/creative/spectator filtering, bound
targets, sleeping owners and lead holders are preserved; nonfinite inputs use
the original ordered scan.

## Cross-region dependency audit

| Dependency | Coordination |
| --- | --- |
| Level RNG | Each job starts at the same checkpoint. A result that consumed it commits only at that exact authoritative stream state, including cached Gaussian state; otherwise serial replay consumes the original draws. A result with no level draws never overwrites the authoritative stream. No per-region seed is substituted. |
| Mob observations and collisions | The complete snapshot remains visible. Actual slot, spatial, species and full-scan reads are validated against earlier foreign writes; local overlays preserve immediate local observations. |
| Pushes and brain memories | Pending pushes retain event order. Known single-recipient writes are applied to later owned inputs; foreign recipients invalidate their owner's prefix. Riding and school-follow changes conservatively invalidate all prefixes and run the remaining conflicting work serially. |
| EntitySection insertion | A per-level allocator reserves order before jobs run. Rejected work cannot consume a process-global insertion counter or change candidate ordering. |
| Raider hurt ordering | Raider species and active raid participants stay serial because their hurt sequence remains globally coordinated. A worker stops before that entity; the coordinator runs it normally. |
| Native mods and tracing | Loaded native mods, entity tick listeners and RNG tracing retain serial execution. Arbitrary callbacks are never speculated. Content-block contacts keep their existing server-thread scope. |
| Projectiles and vehicles | The existing ordered hit, homing, passenger and steering phases operate on completed mob state. Their dependencies can span any region and are not declared independent. Projectile admission/join rules are preserved. |
| Explosions, block writes and navigation | Existing explosion, damage, block and event handlers remain coordinated. Navigation sees the same borrowed world and collision-change list during the mob phase. No explosion radius is used as an independence margin. |
| Spawning and caps | Natural/custom spawning, raids, pack finalization, global/per-player caps, IDs and their shared RNG stay in their existing serial order before entity transactions. Spawn scratch follows its level owner. |
| Death, despawn and drops | Existing ticket/despawn checks, corpse removal, death effects and loot processing surround the validated mob tick at the original points. Non-ticking despawns invalidate affected observations. |
| Replication and chunk lifecycle | All workers join before publication, streaming or unload can mutate the borrowed world. Existing event buffers and entity tracking publish committed results, including authoritative replay results. |

Sound ID interning uses persistent-thread local caches instead of process-wide
mutexes in living-entity ticking. There is no global mob-loop lock. Worker
results contain owned mob state and events; there are no unsafe world pointers.

## Execution controls

Automatic mode uses region transactions for at least 128 mobs in at least two
resident regions. Small or connected workloads retain the direct serial path.
The threshold only controls scheduling overhead; it never limits interactions.

- `HORIZON_SIMULATION_MODE=legacy`: retained direct serial mob loop.
- `HORIZON_SIMULATION_MODE=serial`: region transactions on the server thread.
- `HORIZON_SIMULATION_MODE=parallel`: force region transactions, including small
  test scenarios.
- `HORIZON_SIMULATION_WORKERS=1..8`: bounded pool size; defaults to the project's
  worker-count policy, capped at eight. Pool construction failure falls back
  to serial execution.
- `HORIZON_REGION_METRICS=1`: region, predicted, committed and replayed counts.

The older snapshot projection knob (`HORIZON_REGION_SNAPSHOT_WORKERS`) stays at
one by default: snapshot copying alone did not benefit from scoped threads.
The persistent simulation pool performs real mob ticks.

## Validation and benchmark

The focused tests cover topology/bridge merge and split, stable ordering,
negative/extreme chunk coordinates, migration/removal/unload/dormancy, exact
sparse-grid equivalence, negative-space reads, distant ID/full-scan reads,
foreign bounds growth, sparse populated-cell queries, shared RNG/Gaussian
consumption, complete mutable-mob transactions and exact indexed player
selection. At implementation revision `4c5925eb2`, the full mob-AI vanilla oracle passes all 272 scenarios and all
127,179 mob ticks. Forced-parallel dedicated-server, server-entity,
player-damage and natural-spawning harnesses pass. Live server-thread/LAN
checks pass for mobs, portals, projectiles/vehicles and blasts; the mob check
was repeated against the final runtime.

The repository check at that implementation revision passes formatting, clippy (including the server
feature configuration), 5,853 workspace Rust tests, 627 Python tests and 24
content-root tests. Launcher formatting/clippy and 463 launcher tests pass;
its translation-completeness test fails on missing Hebrew strings. The exact
unchanged test also fails against base `9697aaf14`, independently reproducing
that existing failure.

Before integration with newer upstream changes, 225 distinct affected parity
entries passed without a recorded behavior mismatch. The remaining broad sweep
was stopped when the user requested delivery after the extended run; it is not
reported as a completed sweep. Save-fixture setup and an item-order oracle run
required retries; the fixture startup and both item-order cases then succeeded.

Current main `e41334ac0` is integrated at `98aaa6e9e`. The merged server passes
`cargo check -p horizon-client --lib`, formatting, seven region topology and
concurrency tests, and 15 focused mob transaction, player-index and dependency
tests. These include a new controlling-rider input regression and the complete
serial/parallel mutable-state, ordered-event and RNG comparison. The full
workspace/parity sweep and performance measurements were not repeated after
that integration; the measured engine revisions are explicit below.

`tools/bench_region_ticks.py` starts 100 real `horizon-bots` clients, teleports
them to a 512-block grid in a fixed-seed flat world, and summons 70 persistent
AI-enabled zombies per player in interleaved global order (7,000 total). Natural
spawning and random block ticks are off; players have resistance, and mobs are
invulnerable to keep the population stable. Each mode uses a fresh world and
scratch user directories. The script rejects changed player/mob counts and
collects whole-tick median/p95/p99 from the server performance report, individual
mob-phase timings, process-tree CPU and RSS, and prediction/replay counts. Setup runs while frozen, and both engines warm
up for the same fixed number of simulation ticks before timing starts.

```sh
cargo build --profile fast -p horizon-client -p horizon-client-lib -p horizon-bots \
  --bin horizon-client --bin horizon-ctl --bin horizon-bots
python3 tools/bench_region_ticks.py --bin target/fast
```

The retained serial and parallel modes use identical commands, seed, view and
simulation distance, fixed-tick warmup and measurement windows in the same binary.
`--before-bin` instead uses a separately built pre-change engine for the baseline;
the final comparison uses the PR's original base revision rather than attributing
improvements common to both new execution modes to parallel scheduling.
This is a quick workload comparison, not a claim about every mob mix: shared
RNG, native mods, active raids and unbounded observations can require replay.

### Measured comparison (2026-11-03)

Same 100 connected standing players, 512-block spacing, 7,000 invulnerable
AI-enabled zombies, seed 12345, flat terrain, view/simulation distance 2,
200 simulation warmup ticks and a 20-second measurement. Both engines use the
`fast` optimized profile and Rust 1.99.0 on a Ryzen 5 5600X (6 cores/12 threads).
The new pool has eight workers. Validation scheduling/compilation was paused
for measurement after active harnesses finished; other desktop applications
remained running. This is one quick sweep on a shared machine.

The original engine was built from base `9697aaf14`; the new engine was built
from `4c5925eb2`. The direct serial control uses the new engine with region
execution disabled. The complete inputs, percentile summaries and selected
phase timings are in [the measurement JSON](performance/server-regions-100-players.json).

| Execution | Tick median ms | Tick p95 ms | Tick p99 ms | Mob median ms | Mob p95 ms | Mob p99 ms | CPU % | Mean RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Original engine | 133.74 | 189.87 | 418.22 | 99.2 | 142.2 | 365.8 | 98.50 | 442.38 |
| New engine, direct serial control | 182.63 | 550.65 | 679.61 | 136.1 | 404.9 | 609.9 | 81.99 | 454.81 |
| New engine, parallel regions | 91.56 | 101.05 | 110.53 | 60.2 | 66.3 | 72.9 | 236.50 | 521.65 |

CPU 100% means one logical core. Peak RSS was 443.96/456.22/547.60 MiB,
respectively. The reports contain 139/79/219 ticks; timed mob spans cover
138/78/218 ticks, with the initial frozen/report boundary omitted. Every
measurement kept all 100 players and 7,000 mobs. Parallel execution committed
all 7,000 predictions across 100 regions per tick, with no replay in this
workload. The direct serial control had substantial tail variation; this
single sweep does not establish a small-workload or serial-mode speedup.

Against the original engine, parallel execution reduced median mob time by
39%, whole-tick median by 32%, p95 by 47% and p99 by 74%, using more CPU and
about 79 MiB more mean RSS. This dense collision workload still runs below
20 TPS (6.93 before, 10.90 after). The results establish a live improvement,
not a guarantee that every 100-player mob mix fits a 50 ms tick budget.

```sh
python3 tools/bench_region_ticks.py --bin .local/parity-target/fast \
  --before-bin /path/to/base-build/target/fast --modes legacy,control,parallel \
  --warmup 10 --settle 20 --seconds 20
```

### After the coordinator work (2026-11-04)

Two runs of the same workload at `ce803b1ea`, parallel mode only, with the
`fast` profile plus frame pointers and line tables, on the same shared machine
(other applications running, the bots on the same CPU). Their full reports are
under `coordinator_work` in [the measurement JSON](performance/server-regions-100-players.json).

| Run | Tick average ms | Median | p95 | p99 | Max | Ticks over 50 ms | Mob median ms | CPU % | Mean RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 47.44 | 47.09 | 51.06 | 53.72 | 54.82 | 26 of 401 | 28.4 | 394.65 | 544.63 |
| 2 | 49.06 | 48.05 | 56.25 | 66.28 | 75.04 | 88 of 401 | 28.9 | 399.49 | 558.37 |

Both runs hold 20 TPS with all 7,000 predictions committed and no replay, and
the average tick fits the 50 ms budget. Single ticks still go over it, so this
workload has no margin on this machine, and runs differ by a millisecond or
two. The remaining time is mostly the workers' 16 ms round, the 7 ms commit walk
and 8 ms of `tick_lan`.

The time came from the coordinator's walks over every mob, each a cache miss
per mob: game events and melee hits find their listeners and mob directly,
states and entity sends are built on the workers, a mob's rarely used fields
sit behind a box so a clone copies less, and the scans a tick repeats run on the
workers (`Game::mob_slots`) or not at all. The replaced mobs are freed by one
worker at the end of the tick, so those scans do not queue behind the frees.
