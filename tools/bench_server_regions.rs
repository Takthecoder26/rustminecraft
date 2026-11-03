//! Standalone ownership/projection microbenchmark (not AI or a server tick).
//! Build: rustc --edition=2024 -O tools/bench_server_regions.rs -o /tmp/region-bench
//! Run each mode under /usr/bin/time -v for CPU and maximum resident memory.
#[path = "../crates/horizon-client/src/server_regions.rs"]
#[allow(dead_code)]
mod server_regions;
use server_regions::{Regions, Workers};
use std::{hint::black_box, time::Instant};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "baseline".into());
    let workers = match mode.as_str() {
        "baseline" | "serial" => 1,
        "parallel" => 8,
        _ => panic!("mode must be baseline, serial, or parallel"),
    };
    // 100 disconnected 17x17 loaded areas, 70 entities at each player.
    // The projection copies 384 bytes/entity, approximating snapshot traffic;
    // this deliberately does not pretend to model AI, navigation or combat.
    let entities: Vec<[u64; 48]> = (0..7000).map(|i| [i; 48]).collect();
    let chunks: Vec<_> = (0..100)
        .flat_map(|p| (-8..=8).flat_map(move |x| (-8..=8).map(move |z| (p * 100 + x, z))))
        .collect();
    let positions: Vec<_> = (0..7000).map(|i| ((i / 70) * 100, 0)).collect();
    let players: Vec<_> = (0..100).map(|p| (p * 100, 0)).collect();
    let mut regions = Regions::default();
    let started = Instant::now();
    regions.rebuild(1, chunks.into_iter());
    let topology_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut times = Vec::new();
    for tick in 0..350 {
        let started = Instant::now();
        let out: Vec<[u64; 48]> = if mode == "baseline" {
            entities.iter().map(|e| *black_box(e)).collect()
        } else {
            regions.index(positions.iter().copied(), players.iter().copied());
            regions.project(&entities, Workers::new(workers), |e| *black_box(e))
        };
        assert_eq!(black_box(&out), &entities);
        if tick >= 50 {
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    times.sort_by(f64::total_cmp);
    let percentile = |p: f64| times[((times.len() - 1) as f64 * p).ceil() as usize];
    println!(
        "mode={mode} samples={} entities={} regions={} topology_ms={topology_ms:.3} median_ms={:.3} p95_ms={:.3} p99_ms={:.3}",
        times.len(),
        entities.len(),
        regions.regions().len(),
        percentile(0.5),
        percentile(0.95),
        percentile(0.99)
    );
}
