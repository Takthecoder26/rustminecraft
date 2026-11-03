# Rust struct field-order / padding audit

Read-only static audit of Rust sources under `crates/`. No source was edited and no `cargo` runs were performed.

Method: parse struct definitions, estimate each field's size and alignment for primitive types and a few containers/arrays/tuples, compute the declared size versus an alignment-descending reorder, and flag structs where reordering would save at least 4 bytes per instance. GPU/`#[repr(C)]` structs were audited separately because their layout is ABI-sensitive.

Scope caveat: this is a static approximation. Rust's default `repr(Rust)` layout may already be optimized by the compiler, and custom types are approximated by `(8, 8)`; the reported savings are upper bounds. Reference counts are crude identifier-frequency proxies, not instance counts.

## Proposed-field-order key

Fields are reordered from largest alignment to smallest alignment, with equal-alignment fields ordered by descending size. Small (`bool`/`u8`/`i8`) fields are pushed to the end so they pack together.

## High-impact findings (primitive structs)

Sorted by bytes saved per instance, then crude reference count.

| Rank | Struct | Location | Declared | Proposed | Saved | Crude refs | Proposed order (first -> last) |
|---:|---|---|---:|---:|---:|---:|---|
| 1 | `SubtitleFrame` | `crates/horizon-client-render/src/hud_overlays.rs` | 104 | 96 | 8 | 6 | `position, forward, up, multiplier, background, width, height, show` |
| 2 | `Played` | `crates/horizon-client/src/lan/prediction.rs` | 104 | 96 | 8 | 38 | `off, at, velocity, offset, moved, candidate, ground, state, input` |
| 3 | `Frame` (cem anim) | `crates/horizon-client/src/mobs/cem_anim.rs` | 72 | 64 | 8 | 745 | `player, time, day_time, player_rot, frame_time, frame_counter, dimension, paused, first_person, third_person_reversed` |
| 4 | `Attrs` | `crates/horizon-client/src/mobs.rs` | 64 | 56 | 8 | 3 | `speed, attack_knockback, follow_range, knockback_resistance, spawn_reinforcements, jump_strength, attack_damage, max_health` |
| 5 | `MobSound` | `crates/horizon-client-render/src/sound_engine.rs` | 56 | 48 | 8 | 10 | `position, speed, id, attack_scale, baby, angry, silent, sniffer_digging` |
| 6 | `Fangs` | `crates/horizon-client/src/game/fangs.rs` | 56 | 48 | 8 | 39 | `pos, id, yaw, warmup, life, owner, started` |
| 7 | `DebugKeys` | `crates/horizon-client/src/debug_keys.rs` | 40 | 32 | 8 | 24 | `crash_key_time, crash_key_reported_time, crash_key_reported_count, used_debug_key_as_modifier, modifier_down` |
| 8 | `Wanted` | `crates/horizon-client-render/src/lod.rs` | 32 | 24 | 8 | 31 | `distance, x, z, class, level` |
| 9 | `Map` | `crates/horizon-client/src/game.rs` | 32 | 24 | 8 | 4598 | `container, cells, table, plain` |
| 10 | `WalkTarget` | `crates/horizon-client/src/mobs/brain.rs` | 32 | 24 | 8 | 137 | `speed, pos, close` |
| 11 | `Offspring` | `crates/horizon-client/src/mobs/llama.rs` | 32 | 24 | 8 | 43 | `jump, speed, max_health, strength, variant` |
| 12 | `GameRules` | `crates/horizon-world/src/level/mod.rs` | 28 | 20 | 8 | 109 | `random_tick_speed, max_snow_accumulation_height, fire_spread_radius_around_player, water_source_conversion, lava_source_conversion, block_drops, mob_griefing, spawn_mobs, spawn_monsters, spread_vines` |
| 13 | `NameView` | `crates/horizon-client-core/src/scoreboard.rs` | 24 | 16 | 8 | 6 | `distance_sqr, discrete, invisible_to_viewer, hud_hidden, is_camera, is_vehicle` |
| 14 | `SuggestLayout` | `crates/horizon-client-render/src/chat.rs` | 24 | 16 | 8 | 7 | `line_limit, fill, commands_only, only_past_error, anchor_to_bottom` |
| 15 | `RiddenSound` | `crates/horizon-client-render/src/sound_engine.rs` | 24 | 16 | 8 | 14 | `speed, id, silent, underwater` |
| 16 | `PlayerStatus` | `crates/horizon-client-render/src/gfx/pack_uniforms.rs` | 36 | 32 | 4 | 21 | `health, max_health, food, air, armor, survival, spectator, riding, feet_in_water, swimming, burning, hurt, invisible, sneaking, sprinting` |
| 17 | `Fact` | `crates/horizon-world/src/level/data/cache.rs` | 36 | 32 | 4 | 595 | `flags, sturdy, weak, strong, hardness, shape, fluid_kind, fluid_amount, fluid_falling, emission, dampening, push, instrument` |
| 18 | `State` (armadillo) | `crates/horizon-client/src/mobs/armadillo.rs` | 28 | 24 | 4 | n/a | `in_state_ticks, danger, sensor, next_peek, state, ball_up, danger_was_around, panic_activity, peek_received` |
| 19 | `Panda` | `crates/horizon-client/src/mobs/body_anim.rs` | 24 | 20 | 4 | 162 | `sneeze_time, sit_amount, lie_on_back_amount, roll_amount, unhappy, sneezing, eating, scared` |
| 20 | `Poplar` | `crates/horizon-world/src/worldgen/tree/foliage.rs` | 16 | 12 | 4 | 20 | `side_hole, height, flip, double` |
| 21 | `FollowPlayerRidden` | `crates/horizon-client/src/mobs/dolphin.rs` | 12 | 8 | 4 | 13 | `recalc, nautilus, following, leading` |
| 22 | `LocalDifficulty` | `crates/horizon-client/src/mobs/spawning.rs` | 12 | 8 | 4 | 28 | `effective, base, normal, hard, id` |

Notes for primitive findings:
- `Frame` in `cem_anim.rs` has a large crude reference count, but the name is generic; verify usage density before editing.
- `Map`, `State`, and `Fact` are common names, so their reference counts are not reliable proxies for instance counts.
- `GameRules` and `NameView` are small globally, so even with a good multiplier their absolute memory impact is modest unless instantiated at block/scoreboard scale.

## GPU / `#[repr(C)]` finding

| Struct | Location | Declared | Proposed | Saved | Caveat |
|---|---|---:|---:|---:|---|
| `GpuStaticEntitySection` | `crates/horizon-client-render/src/gfx.rs` | 56 | 48 | 8 | `#[repr(C)]`; field order is part of any layout/ABI/shader interop contract. Do not reorder without checking all consumers and shader expectations. |

All other audited `#[repr(C)]` structs had zero modeled reorder savings.

## Likely highest practical impact

Because every listed struct saves only 4-8 bytes per instance, the real ranking depends on instance count:
- Highest probability of large aggregate savings: `Fact` (block fact cache), `WalkTarget` (mob brain targets), `MobSound`/`RiddenSound` (active sound state), `Fangs`, `Played` prediction state, and `PlayerStatus` (per-player uniform inputs).
- Moderate probability: `Wanted` (LOD scheduling), `Panda`, armadillo `State`, `Offspring`.
- Low aggregate despite useful cleanup: singleton/global/config structs such as `DebugKeys`, `GameRules`, `SubtitleFrame`, and `SuggestLayout`.

Recommended next step is to confirm allocation counts with profiling, then apply only the non-`repr(C)` reorder edits; keep the `repr(C)` edit pending ABI review.
