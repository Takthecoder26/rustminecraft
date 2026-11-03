# Horizon documentation

To play, use [the launcher](../launcher/README.md); the project site is [playhorizon.dev](https://playhorizon.dev). These pages are for developers. What is being done now and next, the work board and how to help: [the hub](https://playhorizon.dev/hub) ([ROADMAP.md](../ROADMAP.md), [roadmap/README.md](../roadmap/README.md)).

## Getting started and code map

- [AGENTS.md](../AGENTS.md) and [Architecture](architecture.md): where each part of the code lives, how `Game` is organised, which harness checks which area.
- [Build from source](../README.md#build-from-source): your own copy of the game, resource preparation, `cargo run`.
- [Build times](build-times.md): cold and incremental build measurement per crate and the critical path.
- [How Horizon is built](how-horizon-is-built.md): the overall build and release pipeline.
- [Native client](native-client.md): options, controls, the dedicated server, `horizon-ctl` scripting, packaging.
- [Versioning](versioning.md): release numbering from 0.2 and upgrading older installs.

## Server, hosting, and play

- [Host a server](hosting.md): install, first run, `server.toml`, commands, web panel, backups, services, Docker.
- [For server owners](owners.md): every `server.properties` key beside its `server.toml` field.
- [Hosting: design](server.md) and [The next server](server-next.md): dedicated server design, targets and plan.
- [The anticheat](anticheat.md) and [The chat filter](chat-filter.md): dedicated-server checks and message filtering.
- [Survival multiplayer](smp.md): the official server's `smp` native mod.
- [Security](security.md) and [Velocity forwarding](velocity-forwarding.md): hardening and proxy support.
- [Play with Friends](play-with-friends.md): invite codes, tunnel, relay.
- [Microsoft account](microsoft-account.md): login flow.
- [Server against Pumpkin](performance/server-vs-pumpkin.md): tick time, CPU, memory, chunks, join at scale.

## Parity, porting, and vanilla reference

- [Rust rewrite status](rust-parity.md) and [Parity workflow](parity-workflow.md): what is ported and how to run harnesses.
- [CI runners](ci-runners.md): which jobs run where.
- Area notes: [block behaviours and ticking](parity/blocks.md), [section meshing](parity/meshing.md).
- [Port ledger](port-ledger.md) and [Version ports](version-ports.md): vanilla-to-Rust mapping and new-release porting.
- [Decompilation status](decompilation-status.md) and [research lab](research-lab.md): local inspection workspace.
- [Upgrading a 26.2 data pack](datapack-upgrade.md) and [Modpacks](modpacks.md): porting whole packs.

## World generation

- [World generation](world-generation.md): the pipeline from noise to features.
- [World height](world-height.md), [Noise generation](noise-generation.md), [Compiled density functions](density-functions.md), [Structure density](structure-density.md), [Terrain filling](terrain-filling.md).
- Biomes: [lookup](biome-lookup.md), [temperatures](biome-temperatures.md).
- Surfaces: [material surfaces](material-surfaces.md), [material conditions](material-conditions.md), [rule libraries](material-rule-libraries.md), [vertical anchors](vertical-anchors.md), [value providers](value-providers.md).
- [Carving](carving.md), [Heightmaps](heightmaps.md), [Feature placement](feature-placement.md), [Pregeneration](pregeneration.md).

## World data and simulation

- [World data](world-data.md): chunk storage, palettes, region files.
- [Block-state codecs](block-state-codecs.md), [Scheduled ticks](scheduled-ticks.md), [Save batch entities](save-batch-entities.md).
- [Collision geometry](collision-geometry.md): shapes, collision, movement and fluids.
- [Face visibility](face-visibility.md): which model faces the mesher emits.
- [Server levels](server-levels.md): one level per dimension.

## Rendering, resources, and UI

- [Graphics](graphics.md): settings screen, benchmark and results.
- [Resource packs](resource-packs.md): what a 26.3 pack can change and whether Horizon honours it.
- [Shader packs](shaders.md): Horizon's own WGSL shader packs.
- [Design system](design-system.md) and [Extras](extras.md): palette, panels, zoom, minimap overlays.
- [Controllers](controllers.md) and [Narrator](narrator.md): input glyphs, text-to-speech.
- Menus are vanilla's, with Horizon's graphics and settings hanging off Options and Friends buttons on title and pause menus ([Play with Friends](play-with-friends.md)).

## Mods and content

- [Mods](mods.md): content and native mods, making, installing, publishing.
- [Native mods](native-mods.md) and [Native hook points](native-hooks-neoforge.md): launcher build flow and proposed hooks.
- [Mod browser](mod-browser.md) and [Cosmetics](cosmetics.md): browsing and cosmetic items.
- [Launcher preparation](launcher-preparation.md), [launcher commands](launcher-commands.md), and [Launcher](../launcher/README.md).

## Gameplay and systems

- [Agent control](agent-control.md) and [Horizon ctl commands](horizon-ctl-commands.md): what agents drive and the socket CLI.
- [Narrator](narrator.md): vanilla text-to-speech (Ctrl+B).

## Horizon-only features and APIs

- [Horizon APIs](api/README.md): mods, shaders, agent control, servers, launcher, data formats, stability.
- Opt-in [telemetry](telemetry.md) and [benchmark report format](benchmark-report.md).

## Tools, benchmarks, and profiling

- [Benchmark](benchmark.md): Horizon versus vanilla 26.3 and performance mods.
- [Build times](build-times.md), [Profiling (JFR)](profiling.md), [CI runners](ci-runners.md).
- [Performance directory](performance/) and the `*-benchmark.md` notes: individual micro-benchmarks.

## Reference

- [Known issues](known-issues.md), [Release flow](release-flow.md), [Owners](owners.md).
