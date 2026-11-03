//! Server region ownership at a tick barrier. Spatial connectivity is an
//! ownership rule, never a proof that gameplay in two regions commutes.
//!
//! No world data is copied here. Slots refer to the level's ordered entity
//! list, and chunk membership is frozen until all work at the barrier joins.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

pub type Chunk = (i32, i32);

/// A region's canonical chunk. Handles are valid only until the next rebuild.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub Chunk);

#[derive(Debug)]
pub struct Region {
    pub id: RegionId,
    pub chunks: Vec<Chunk>,
    /// Slots retain the input entity/player order, including ties.
    pub entities: Vec<usize>,
    pub players: Vec<usize>,
}

/// A level's ownership directory. Never shared mutably with a worker.
#[derive(Default)]
pub struct Regions {
    revision: Option<u64>,
    regions: Vec<Region>,
    owners: HashMap<Chunk, usize>,
    /// Entities in nonresident chunks still participate in the serial despawn
    /// pass and the global snapshot. They do not own or keep chunks loaded.
    dormant: Vec<usize>,
}

impl Regions {
    /// Rebuild only when chunk membership changes. The caller's revision must
    /// change on load, unload, reset, or replacement of the whole chunk map.
    /// Eight-connected resident chunks share an owner (including diagonals).
    /// A new bridge merges components; unloading a bridge splits them.
    pub fn rebuild(&mut self, revision: u64, chunks: impl Iterator<Item = Chunk>) {
        if self.revision == Some(revision) {
            return;
        }
        let mut remaining: HashSet<Chunk> = chunks.collect();
        let mut ordered: Vec<_> = remaining.iter().copied().collect();
        ordered.sort_unstable();
        self.owners.clear();
        self.regions.clear();
        self.dormant.clear();
        for start in ordered {
            if !remaining.remove(&start) {
                continue;
            }
            let mut chunks = vec![start];
            let mut head = 0;
            while head < chunks.len() {
                let (x, z) = chunks[head];
                head += 1;
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        let (Some(nx), Some(nz)) = (x.checked_add(dx), z.checked_add(dz)) else {
                            continue;
                        };
                        let next = (nx, nz);
                        if remaining.remove(&next) {
                            chunks.push(next);
                        }
                    }
                }
            }
            chunks.sort_unstable();
            let slot = self.regions.len();
            self.owners.extend(chunks.iter().map(|&c| (c, slot)));
            self.regions.push(Region {
                id: RegionId(start),
                chunks,
                entities: Vec::new(),
                players: Vec::new(),
            });
        }
        self.revision = Some(revision);
    }

    /// Publish migrations together at a barrier. Input slots are unchanged:
    /// grouping by region must never reorder the serial entity tick walk.
    /// Removed entities disappear; new entities enter once, at their new slot.
    /// Chunks are not loaded or retained by a migrating entity/player here.
    pub fn index(
        &mut self,
        entities: impl Iterator<Item = Chunk>,
        players: impl Iterator<Item = Chunk>,
    ) {
        for region in &mut self.regions {
            region.entities.clear();
            region.players.clear();
        }
        self.dormant.clear();
        for (i, chunk) in entities.enumerate() {
            match self.owners.get(&chunk) {
                Some(&owner) => self.regions[owner].entities.push(i),
                None => self.dormant.push(i),
            }
        }
        for (i, chunk) in players.enumerate() {
            if let Some(&owner) = self.owners.get(&chunk) {
                self.regions[owner].players.push(i);
            }
        }
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }
    pub fn dormant(&self) -> &[usize] {
        &self.dormant
    }
    pub fn owner(&self, chunk: Chunk) -> Option<RegionId> {
        self.owners.get(&chunk).map(|&i| self.regions[i].id)
    }

    /// A pure projection of the ordered level entities, prepared by region.
    /// Workers borrow immutable inputs; each result is owned by one worker.
    /// The join is the publication barrier. No callback can mutate the level.
    /// Dormant entities remain visible, in their original snapshot slots.
    pub fn project<T: Sync, R: Send>(
        &self,
        entities: &[T],
        workers: Workers,
        f: impl Fn(&T) -> R + Sync,
    ) -> Vec<R> {
        // A worker launch is more expensive than small snapshots. This is a
        // scheduling threshold, not a gameplay boundary or independence test.
        if entities.len() < 1024 || self.regions.len() < 2 || workers.count == 1 {
            return entities.iter().map(f).collect();
        }
        let groups: Vec<&[usize]> = self
            .regions
            .iter()
            .map(|r| r.entities.as_slice())
            .chain(std::iter::once(self.dormant.as_slice()))
            .filter(|g| !g.is_empty())
            .collect();
        let results = workers.map(&groups, |slots| {
            slots
                .iter()
                .map(|&i| (i, f(&entities[i])))
                .collect::<Vec<_>>()
        });
        let mut out: Vec<Option<R>> = (0..entities.len()).map(|_| None).collect();
        for group in results {
            for (i, value) in group {
                out[i] = Some(value);
            }
        }
        out.into_iter()
            .map(|v| v.expect("region index must cover the frozen entity list"))
            .collect()
    }
}

/// Bounded scoped workers, lending data without unsafe lifetime erasure or
/// cloning a level. All workers join even when a callback panics.
#[derive(Clone, Copy)]
pub struct Workers {
    count: usize,
}
impl Workers {
    pub fn new(count: usize) -> Self {
        Self {
            count: count.clamp(1, 8),
        }
    }
    pub fn count(self) -> usize {
        self.count
    }
    pub fn map<T: Sync, R: Send>(self, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
        let count = self.count.min(items.len());
        if count <= 1 {
            return items.iter().map(f).collect();
        }
        let next = AtomicUsize::new(0);
        let mut out: Vec<(usize, R)> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..count)
                .map(|_| {
                    scope.spawn(|| {
                        let mut out = Vec::new();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(item) = items.get(i) else {
                                break;
                            };
                            out.push((i, f(item)));
                        }
                        out
                    })
                })
                .collect();
            // Drain every join before propagating the first panic.
            let mut out = Vec::new();
            let mut panic = None;
            for worker in workers {
                match worker.join() {
                    Ok(values) => out.extend(values),
                    Err(error) => {
                        panic.get_or_insert(error);
                    }
                }
            }
            if let Some(error) = panic {
                std::panic::resume_unwind(error);
            }
            out
        });
        out.sort_unstable_by_key(|(i, _)| *i);
        out.into_iter().map(|(_, value)| value).collect()
    }
}
impl Default for Workers {
    fn default() -> Self {
        // Opt in only: the cheap projection benchmark is slower with scoped
        // workers. Do not regress production ticks to exercise concurrency.
        Self::new(
            std::env::var("HORIZON_REGION_SNAPSHOT_WORKERS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
        )
    }
}

/// Capabilities required by a phase. `Global` includes unbounded queries,
/// shared RNG, native callbacks, spawn caps and cross-entity writes whose
/// complete footprint has not been proved. Such a phase stays serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read(Chunk),
    Write(Chunk),
    Global,
}
impl Access {
    pub fn conflicts(self, other: Self) -> bool {
        match (self, other) {
            (Self::Global, _) | (_, Self::Global) => true,
            (Self::Read(_), Self::Read(_)) => false,
            (Self::Read(a) | Self::Write(a), Self::Read(b) | Self::Write(b)) => a == b,
        }
    }
}

/// Consecutive jobs that can share a barrier. A later job never passes a
/// conflicting earlier job. Spatially separate jobs with Global access are
/// intentionally singleton batches, even if their owners differ.
pub fn batches(access: &[Vec<Access>]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..access.len() {
        let conflict = access[start..i].iter().any(|job| {
            job.iter()
                .any(|&a| access[i].iter().any(|&b| a.conflicts(b)))
        });
        if conflict {
            out.push(start..i);
            start = i;
        }
    }
    if start < access.len() {
        out.push(start..access.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rebuild(r: &mut Regions, rev: u64, chunks: &[Chunk]) {
        r.rebuild(rev, chunks.iter().copied());
    }

    #[test]
    fn loading_and_unloading_a_bridge_merges_and_splits() {
        let mut r = Regions::default();
        rebuild(&mut r, 1, &[(0, 0), (2, 0), (100, 0)]);
        assert_eq!(r.regions().len(), 3);
        rebuild(&mut r, 2, &[(100, 0), (2, 0), (1, 0), (0, 0)]);
        assert_eq!(r.owner((0, 0)), r.owner((2, 0)));
        assert_ne!(r.owner((0, 0)), r.owner((100, 0)));
        rebuild(&mut r, 3, &[(0, 0), (2, 0)]);
        assert_ne!(r.owner((0, 0)), r.owner((2, 0)));
        assert_eq!(r.owner((100, 0)), None);
    }

    #[test]
    fn diagonal_and_negative_chunks_connect_without_integer_wraparound() {
        let mut r = Regions::default();
        rebuild(
            &mut r,
            1,
            &[(-2, -2), (-1, -1), (i32::MIN, 0), (i32::MAX, 0)],
        );
        assert_eq!(r.owner((-2, -2)), r.owner((-1, -1)));
        assert_ne!(r.owner((i32::MIN, 0)), r.owner((i32::MAX, 0)));
        assert_eq!(r.regions().len(), 3);
    }

    #[test]
    fn migration_removal_and_unload_keep_local_and_global_order() {
        let mut r = Regions::default();
        rebuild(&mut r, 1, &[(0, 0), (10, 0)]);
        r.index(
            [(10, 0), (0, 0), (10, 0), (99, 0)].into_iter(),
            [(10, 0), (0, 0)].into_iter(),
        );
        assert_eq!(r.regions()[0].entities, [1]);
        assert_eq!(r.regions()[1].entities, [0, 2]);
        assert_eq!(r.regions()[1].players, [0]);
        assert_eq!(r.dormant(), [3]);
        r.index([(0, 0), (0, 0), (10, 0)].into_iter(), std::iter::empty());
        assert_eq!(r.regions()[0].entities, [0, 1]);
        assert_eq!(r.regions()[1].entities, [2]);
        assert!(r.dormant().is_empty());
        assert!(r.regions()[1].players.is_empty());
        rebuild(&mut r, 2, &[]);
        r.index([(0, 0)].into_iter(), std::iter::empty());
        assert_eq!(r.dormant(), [0]);
    }

    #[test]
    fn parallel_projection_including_dormant_entities_is_serial_equivalent() {
        let mut r = Regions::default();
        rebuild(&mut r, 1, &[(0, 0), (10, 0), (20, 0)]);
        let entities: Vec<_> = (0..4097).collect();
        r.index(
            entities.iter().map(|i| ((i % 4) * 10, 0)),
            std::iter::empty(),
        );
        let expected: Vec<_> = entities.iter().map(|i| i * 13).collect();
        for workers in [1, 2, 8] {
            assert_eq!(
                r.project(&entities, Workers::new(workers), |i| i * 13),
                expected
            );
        }
    }

    #[test]
    fn bounded_workers_overlap_and_publish_only_after_all_jobs_complete() {
        use std::sync::{Barrier, atomic::AtomicUsize};
        let barrier = Barrier::new(2);
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let values = Workers::new(2).map(&[0, 1], |i| {
            let n = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(n, Ordering::SeqCst);
            barrier.wait();
            active.fetch_sub(1, Ordering::SeqCst);
            finished.fetch_add(1, Ordering::SeqCst);
            i + 1
        });
        assert_eq!(values, [1, 2]);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(finished.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn conflicts_cannot_overtake_and_global_rng_serializes_distant_owners() {
        use Access::*;
        let jobs = vec![
            vec![Read((0, 0))],
            vec![Read((0, 0))],
            vec![Write((0, 0))],
            vec![Write((99, 0))],
            vec![Global],
            vec![Global],
        ];
        assert_eq!(batches(&jobs), [0..2, 2..4, 4..5, 5..6]);
        assert!(batches(&[]).is_empty());
    }

    #[test]
    fn panic_joins_other_workers_before_returning() {
        let finished = AtomicUsize::new(0);
        let barrier = std::sync::Barrier::new(2);
        let result = std::panic::catch_unwind(|| {
            Workers::new(2).map(&[0, 1], |i| {
                barrier.wait();
                if *i == 0 {
                    panic!("failed region");
                }
                finished.fetch_add(1, Ordering::SeqCst);
            })
        });
        assert!(result.is_err());
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }
}
