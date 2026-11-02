//! Measure worldgen data load times using prepared client resources.
//! `worldgen_load_profile RESOURCES [CACHE_DIR]`: with CACHE_DIR, loads use and
//! refresh the snapshot there, as the client does with its `.horizon-cache`.
//! RESOURCES is the prepared version directory (it must contain `worldgen/`
//! and `facts.json`).
use horizon_world::{
    level::data::load_registry,
    worldgen::generator::GeneratorData,
};
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .expect("resource directory argument");
    let root = Path::new(&root);
    if let Some(dir) = std::env::args().nth(2) {
        GeneratorData::set_cache_dir(dir.into());
    }
    let started = Instant::now();
    let registry = Box::leak(Box::new(load_registry(root)?));
    eprintln!("registry_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    let dir = root.join("worldgen");
    for iteration in 0..3 {
        let started = Instant::now();
        let data = GeneratorData::load(&dir, registry)?;
        eprintln!(
            "worldgen_{}_ms={:.3}",
            iteration,
            started.elapsed().as_secs_f64() * 1000.0
        );
        std::hint::black_box(data);
    }
    Ok(())
}
