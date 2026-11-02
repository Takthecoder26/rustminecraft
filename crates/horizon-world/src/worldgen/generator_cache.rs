//! A disposable, versioned snapshot of what a `GeneratorData` load read.
//!
//! The bundle files (`noises.json`, `biomes.json`, ...: objects of resource
//! identifier to the resource's JSON text) are the load's biggest cost: each
//! is `serde_json::from_str`'d whole. The snapshot keeps them parsed, so a
//! cache hit is a postcard decode instead of a JSON parse. Everything else
//! the loader reads — `block_flags.json`, `biome_parameters/`, the `blocks/`
//! tables, `blocks/structure-blocks.json` — still comes from the directory:
//! a fingerprint over the build, the block registry and every file under it
//! must match before the snapshot is used, so the directory serves the rest
//! identically. `structure/`'s templates are excepted from the fingerprint:
//! they are read lazily, after the load, and the current file always serves.
//!
//! A world's data packs (`WorldgenOverrides`) join the bundles after they are
//! restored, so they change nothing the fingerprint must cover: every world's
//! load shares one snapshot of the resource bundle.
//!
//! The snapshot lives in the game's own cache folder ([`set_dir`]), never in
//! the resource folder: that one is shared by worktrees and copied between
//! installed versions by the launcher. Failed reads, unsupported versions,
//! corrupt payloads and write failures always use the files.
use super::generator::{Bundle, GeneratorError, resource};
use crate::{
    compression::invalid,
    registry::{self, BlockRegistry},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAGIC: &[u8; 8] = b"HZWGEN\0\0";
const VERSION: u32 = 1;
const FILE_NAME: &str = "worldgen-v1.bin";
const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 512 * 1024 * 1024;
static NEXT_TEMP: AtomicU32 = AtomicU32::new(0);
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Keep snapshots in `dir` (the client passes `<game folder>/.horizon-cache`).
/// Without it (tests, tools, servers) `GeneratorData::load` always uses the
/// files.
pub(crate) fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// Whether a cache folder was set and disk sources record for it.
pub(super) fn enabled() -> bool {
    DIR.get().is_some()
}

/// The bundle files a load parsed, by file stem (`noises`, `biomes`, ...).
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Snapshot {
    bundles: BTreeMap<String, Bundle>,
}

/// Where a load's bundles come from: the directory itself, or a snapshot of
/// what an earlier load read in it. Either way the rest of the load's inputs
/// keep coming from the directory.
pub(super) enum Bundles {
    /// `dir`'s files, with each parsed bundle remembered for [`store`] while
    /// a cache folder is set.
    Disk {
        dir: PathBuf,
        record: Option<Mutex<Snapshot>>,
    },
    /// A snapshot's bundles, with `dir` for any it doesn't hold (the
    /// fingerprint makes that only bundles the writing load never asked for).
    Snapshot { snapshot: Snapshot, dir: PathBuf },
}
impl Bundles {
    /// Bundles from `dir`; `record` (the cache is enabled) remembers each
    /// parsed bundle for [`store`].
    pub(super) fn disk(dir: &Path, record: bool) -> Self {
        Self::Disk {
            dir: dir.to_owned(),
            record: record.then(|| Mutex::new(Snapshot::default())),
        }
    }
    /// Bundles from `snapshot`, with `dir` for what it doesn't hold.
    pub(super) fn snapshot(dir: &Path, snapshot: Snapshot) -> Self {
        Self::Snapshot {
            snapshot,
            dir: dir.to_owned(),
        }
    }
    /// The worldgen directory the files come from.
    pub(super) fn dir(&self) -> &Path {
        match self {
            Self::Disk { dir, .. } | Self::Snapshot { dir, .. } => dir,
        }
    }
    fn parse(dir: &Path, name: &str) -> Result<Option<Bundle>, GeneratorError> {
        let path = dir.join(format!("{name}.json"));
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(resource(format!("{}: {e}", path.display()))),
        };
        Ok(Some(serde_json::from_str(&text)?))
    }
    /// The bundle `name`'s entries (resource id → JSON text), before the data
    /// packs' entries join them; `None` when its file isn't there.
    pub(super) fn get(&self, name: &str) -> Result<Option<Bundle>, GeneratorError> {
        match self {
            Self::Disk { dir, record } => {
                let bundle = Self::parse(dir, name)?;
                if let (Some(bundle), Some(record)) = (&bundle, record) {
                    record
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .bundles
                        .insert(name.to_owned(), bundle.clone());
                }
                Ok(bundle)
            }
            Self::Snapshot { snapshot, dir } => match snapshot.bundles.get(name) {
                Some(bundle) => Ok(Some(bundle.clone())),
                None => Self::parse(dir, name),
            },
        }
    }
    /// What a disk source recorded, for [`store`]; `None` when caching is off
    /// or this source read a snapshot.
    pub(super) fn recorded(&self) -> Option<Snapshot> {
        match self {
            Self::Disk {
                record: Some(record),
                ..
            } => Some(std::mem::take(
                &mut *record.lock().unwrap_or_else(|e| e.into_inner()),
            )),
            _ => None,
        }
    }
}

fn fingerprint(dir: &Path, registry: &BlockRegistry) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(b"horizon worldgen data 1");
    // The build that resolved the data: the version constant alone misses
    // loader changes that keep the snapshot layout.
    let exe = std::env::current_exe()?;
    let built = fs::metadata(&exe)?;
    hash_str(&mut hasher, &exe.to_string_lossy());
    hasher.update(built.len().to_le_bytes());
    let modified = built
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| invalid(e.to_string()))?;
    hasher.update(modified.as_nanos().to_le_bytes());
    hasher.update(sources_digest(dir)?);
    // The registry argument may be modified independently of the bundle.
    hasher.update(registry::cache_key(registry)?);
    Ok(hasher.finalize().into())
}

/// The files under the worldgen dir the load may read, as their lengths and
/// modification times, and the hash of their contents.
type Sources = (Vec<(PathBuf, u64, Option<SystemTime>)>, [u8; 32]);
/// Source hashes already computed by this process: a second dimension's data
/// loading from the same directory does not hash it again. A file whose
/// length or modification time changed is read again.
static SOURCES: Mutex<Vec<Sources>> = Mutex::new(Vec::new());

fn sources_digest(dir: &Path) -> io::Result<[u8; 32]> {
    let mut paths = Vec::new();
    collect(dir, dir, &mut paths)?;
    paths.sort();
    let stamps = paths
        .iter()
        .map(|path| {
            let meta = fs::metadata(path)?;
            Ok((path.clone(), meta.len(), meta.modified().ok()))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let known = SOURCES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, digest)) = known.iter().find(|(s, _)| *s == stamps) {
        return Ok(*digest);
    }
    drop(known);
    let mut hasher = Sha256::new();
    for path in &paths {
        hash_file(&mut hasher, dir, path)?;
    }
    let digest: [u8; 32] = hasher.finalize().into();
    SOURCES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((stamps, digest));
    Ok(digest)
}

/// Every file under `dir`, recursively, but `structure/`'s templates.
fn collect(root: &Path, dir: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            // `structure/<namespace>/<path>.nbt` templates are read lazily,
            // after the load: the snapshot holds none of them and the file on
            // disk at that time serves, so they are not inputs to cover.
            if !(dir == root && entry.file_name() == "structure") {
                collect(root, &entry.path(), paths)?;
            }
        } else if entry.file_type()?.is_file() {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn hash_str(hasher: &mut Sha256, text: &str) {
    hasher.update((text.len() as u64).to_le_bytes());
    hasher.update(text.as_bytes());
}

fn hash_file(hasher: &mut Sha256, root: &Path, path: &Path) -> io::Result<()> {
    let relative = path
        .strip_prefix(root)
        .map_err(|e| invalid(e.to_string()))?;
    hash_str(hasher, &relative.to_string_lossy());
    let bytes = fs::read(path)?;
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(())
}

/// The snapshot for `dir`'s files and this build's resolution, when the
/// cache folder's file still describes them.
pub(super) fn load(dir: &Path, registry: &BlockRegistry) -> Option<Snapshot> {
    load_from(DIR.get()?, dir, registry)
}

pub(super) fn load_from(cache: &Path, dir: &Path, registry: &BlockRegistry) -> Option<Snapshot> {
    let bytes = fs::read(cache.join(FILE_NAME)).ok()?;
    let fingerprint = fingerprint(dir, registry).ok()?;
    // Validate bounds before decoding untrusted cache contents.
    if bytes.len() < 8 + 4 + 32 + 32 || bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    if &bytes[..8] != MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().ok()?) != VERSION
        || bytes[12..44] != fingerprint
    {
        return None;
    }
    let body = &bytes[76..];
    if Sha256::digest(body).as_slice() != &bytes[44..76] {
        return None;
    }
    let expanded_len = u32::from_le_bytes(body.get(..4)?.try_into().ok()?) as usize;
    if expanded_len > MAX_DECODED_BYTES {
        return None;
    }
    let expanded = lz4_flex::decompress_size_prepended(body).ok()?;
    postcard::from_bytes(&expanded).ok()
}

/// `snapshot` into the cache folder, for the next load; never fatal.
pub(super) fn store(dir: &Path, registry: &BlockRegistry, snapshot: &Snapshot) {
    if let Some(cache) = DIR.get() {
        let _ = write(cache, dir, registry, snapshot);
    }
}

pub(super) fn write(
    cache: &Path,
    dir: &Path,
    registry: &BlockRegistry,
    snapshot: &Snapshot,
) -> io::Result<()> {
    let fingerprint = fingerprint(dir, registry)?;
    let serialized = postcard::to_allocvec(snapshot).map_err(|e| invalid(e.to_string()))?;
    let body = lz4_flex::compress_prepend_size(&serialized);
    if body.len() + 76 > MAX_FILE_BYTES {
        return Err(invalid("worldgen snapshot too large"));
    }
    let mut bytes = Vec::with_capacity(76 + body.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&fingerprint);
    bytes.extend_from_slice(&Sha256::digest(&body));
    bytes.extend_from_slice(&body);
    fs::create_dir_all(cache)?;
    let path = cache.join(FILE_NAME);
    let tmp = cache.join(format!(
        "{FILE_NAME}.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    static NEXT_FIXTURE: AtomicU32 = AtomicU32::new(0);

    struct FixtureDir(PathBuf);
    impl FixtureDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "horizon-worldgen-cache-test-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            Self(dir)
        }
    }
    impl Drop for FixtureDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A worldgen dir holding two bundle files, a `blocks/` table and a
    /// `structure/` template.
    fn worldgen() -> FixtureDir {
        let root = FixtureDir::new();
        let dir = root.0.join("worldgen");
        fs::create_dir_all(dir.join("blocks")).unwrap();
        fs::create_dir_all(dir.join("structure/minecraft/test")).unwrap();
        fs::write(dir.join("block_flags.json"), "[[false,false]]").unwrap();
        fs::write(
            dir.join("biomes.json"),
            r#"{"minecraft:plains":"{\"temperature\":0.8}","minecraft:desert":"{\"temperature\":2.0}"}"#,
        )
        .unwrap();
        fs::write(
            dir.join("noises.json"),
            r#"{"minecraft:surface":"[1,2,3]"}"#,
        )
        .unwrap();
        fs::write(dir.join("blocks/props.nbt"), b"nbt-bytes").unwrap();
        fs::write(dir.join("structure/minecraft/test/a.nbt"), b"template").unwrap();
        root
    }

    fn registry_fixture() -> &'static BlockRegistry {
        let facts = r#"{"states":[[null,false,false,null,"minecraft:air"],[null,false,false,null,"minecraft:stone"]],"defaults":{"minecraft:air":0,"minecraft:stone":1}}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        Box::leak(Box::new(registry))
    }

    #[test]
    fn a_snapshot_serves_the_bundles_and_the_dir_the_rest() {
        let root = worldgen();
        let dir = root.0.join("worldgen");
        let disk = Bundles::disk(&dir, true);
        let biomes = disk.get("biomes").unwrap().unwrap();
        assert_eq!(biomes.len(), 2);
        assert!(disk.get("missing").unwrap().is_none());
        let recorded = disk.recorded().expect("a disk source records");
        assert!(recorded.bundles.contains_key("biomes"));
        assert!(
            !recorded.bundles.contains_key("missing"),
            "absent bundles are not recorded: the dir answers on restore"
        );

        let snapshot = Bundles::snapshot(&dir, recorded);
        assert_eq!(snapshot.get("biomes").unwrap().unwrap(), biomes);
        // A bundle the writing load never read falls back to the directory.
        assert_eq!(snapshot.get("noises").unwrap().unwrap().len(), 1);
        assert!(snapshot.recorded().is_none());
    }

    #[test]
    fn a_written_snapshot_loads_and_source_changes_invalidate() {
        let root = worldgen();
        let dir = root.0.join("worldgen");
        let registry = registry_fixture();
        let cache = root.0.join("cache");
        let mut snapshot = Snapshot::default();
        snapshot.bundles.insert(
            "biomes".into(),
            [("minecraft:plains".to_string(), "{}".to_string())]
                .into_iter()
                .collect(),
        );
        write(&cache, &dir, registry, &snapshot).unwrap();
        assert!(cache.join(FILE_NAME).is_file());
        assert!(
            !dir.join(FILE_NAME).exists(),
            "the resource folder stays untouched"
        );
        let restored = load_from(&cache, &dir, registry).expect("cache hit");
        assert_eq!(restored.bundles, snapshot.bundles);

        // A changed file anywhere under the dir misses, even a blocks/ table.
        fs::OpenOptions::new()
            .append(true)
            .open(dir.join("blocks/props.nbt"))
            .unwrap()
            .write_all(b"changed")
            .unwrap();
        assert!(load_from(&cache, &dir, registry).is_none());
        write(&cache, &dir, registry, &snapshot).unwrap();
        assert!(load_from(&cache, &dir, registry).is_some());

        // A template change does not invalidate: it is no input of the load.
        fs::write(dir.join("structure/minecraft/test/a.nbt"), b"other").unwrap();
        assert!(
            load_from(&cache, &dir, registry).is_some(),
            "lazily-read templates stay out of the fingerprint"
        );

        let cache_file = cache.join(FILE_NAME);
        let mut bytes = fs::read(&cache_file).unwrap();
        bytes[80] ^= 0x80;
        fs::write(&cache_file, bytes).unwrap();
        assert!(
            load_from(&cache, &dir, registry).is_none(),
            "checksum failure must invalidate"
        );
    }

    #[test]
    fn sources_are_hashed_once_until_a_file_changes() {
        let root = worldgen();
        let dir = root.0.join("worldgen");
        let entries = || {
            SOURCES
                .lock()
                .unwrap()
                .iter()
                .filter(|(stamps, _)| stamps[0].0.starts_with(&dir))
                .count()
        };
        let first = sources_digest(&dir).unwrap();
        assert_eq!(sources_digest(&dir).unwrap(), first);
        assert_eq!(entries(), 1);
        let mut bytes = fs::read(dir.join("noises.json")).unwrap();
        bytes[0] ^= 1;
        fs::write(dir.join("noises.json"), bytes).unwrap();
        assert_ne!(sources_digest(&dir).unwrap(), first);
        assert_eq!(entries(), 2);
    }
}
