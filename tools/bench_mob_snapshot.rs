//! Benchmark actual Mob::seen projection; this does not measure a mob/server tick.
//! Build after horizon-client-lib (same features/profile), for example:
//! rustc --edition=2024 -O tools/bench_mob_snapshot.rs -L dependency=.local/parity-target/fast/deps \
//!   --extern horizon_client_lib=<path-to-libhorizon_client_lib.rlib> -o /tmp/mob-snapshot-bench
//! Run: /tmp/mob-snapshot-bench baseline|serial|parallel <prepared-resource-root>
use horizon_client_lib::{
    mobs::{Mob, MobTypes, Seen},
    server_regions::{Regions, Workers},
};
use std::{hint::black_box, path::Path, sync::Arc, time::Instant};

fn main() {
    let mode = std::env::args()
        .nth(1)
        .expect("baseline, serial, or parallel");
    let workers = match mode.as_str() {
        "baseline" | "serial" => 1,
        "parallel" => 8,
        _ => panic!("baseline, serial, or parallel"),
    };
    let root = std::env::args().nth(2).expect("prepared resource root");
    let types = MobTypes::load_for_server(Path::new(&root)).expect("mob types");
    let kinds: Vec<_> = ["minecraft:zombie", "minecraft:cow", "minecraft:villager"]
        .map(|name| Arc::new(types.types.get(name).expect(name).clone()))
        .into_iter()
        .collect();
    let mobs: Vec<_> = (0..7000)
        .map(|i| {
            let mut mob = Mob::new(
                i + 1,
                kinds[i as usize % kinds.len()].clone(),
                Default::default(),
                0.0,
            );
            mob.position.x = f64::from(i / 70) * 1600.0 + 0.5;
            mob.position.y = 64.0;
            mob.position.z = f64::from(i % 70) + 0.5;
            mob.previous = mob.position;
            mob
        })
        .collect();
    let chunks =
        (0..100).flat_map(|p| (-8..=8).flat_map(move |x| (-8..=8).map(move |z| (p * 100 + x, z))));
    let mut regions = Regions::default();
    regions.rebuild(1, chunks);
    let players: Vec<_> = (0..100).map(|p| (p * 100, 0)).collect();
    let expected: Vec<_> = mobs.iter().map(|m| m.seen(None)).collect();
    let mut times = Vec::new();
    let mut last = Vec::new();
    for iteration in 0..120 {
        let start = Instant::now();
        let out: Vec<Seen> = if mode == "baseline" {
            mobs.iter().map(|m| black_box(m).seen(None)).collect()
        } else {
            regions.index(
                mobs.iter().map(|m| {
                    (
                        m.position.x.floor() as i32 >> 4,
                        m.position.z.floor() as i32 >> 4,
                    )
                }),
                players.iter().copied(),
            );
            regions.project(&mobs, Workers::new(workers), |m| black_box(m).seen(None))
        };
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(out.len(), mobs.len());
        black_box(&out);
        if iteration >= 20 {
            times.push(elapsed);
        }
        last = out;
    }
    // Compare every observable snapshot field with the original serial method,
    // outside the timing. Seen deliberately has no implementation-pinning Eq.
    for (serial, projected) in expected.iter().zip(&last) {
        assert_eq!(format!("{serial:?}"), format!("{projected:?}"));
    }
    times.sort_by(f64::total_cmp);
    let percentile = |p: f64| times[((times.len() - 1) as f64 * p).ceil() as usize];
    println!(
        "mode={mode} samples={} mobs={} regions={} median_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
        times.len(),
        mobs.len(),
        regions.regions().len(),
        percentile(0.5),
        percentile(0.95),
        percentile(0.99)
    );
}
