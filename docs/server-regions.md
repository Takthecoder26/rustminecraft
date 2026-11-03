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
consumption, complete mutable-mob
transactions and exact indexed player selection. Broader parity, server/LAN
checks and the live comparison are recorded here after the current run finishes.

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
