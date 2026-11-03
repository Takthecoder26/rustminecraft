//! Backups and restore (docs/server.md, section 7). A backup is one zip in `backups/`: the world folder and
//! the server's settings and lists. On a running server the game first saves everything and stops writing
//! (`save-off`, `save-all`), and writes again once the zip is done (`save-on`), so players stay online.
//! A restore puts the backup's world in place of the current one, which is moved aside, never deleted.

use crate::config::Config;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const DIR: &str = "backups";
/// The settings and lists a backup holds beside the world.
const FILES: [&str; 6] = [
    "server.toml",
    "server.properties",
    "ops.json",
    "whitelist.json",
    "banned-players.json",
    "banned-ips.json",
];
/// Automatic backups are named `<time>-auto.zip`; only those are pruned.
const AUTO: &str = "auto";

/// `2026-10-16_18-30-05`, in UTC: sorts by time, and is valid in a file name everywhere.
fn stamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = horizon_server_files::java_properties::civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// A backup's file name: the time, and a name the owner gave (letters, digits, `-` and `_` only).
pub fn file_name(time: SystemTime, name: Option<&str>) -> Result<String, String> {
    let name = name.unwrap_or("manual");
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "backup names use letters, digits, - and _ only (got {name:?})"
        ));
    }
    Ok(format!("{}-{name}.zip", stamp(time)))
}

/// Adds a folder to the zip, under `prefix`, skipping the game's session lock.
fn add_dir(zip: &mut ZipWriter<BufWriter<File>>, dir: &Path, prefix: &str) -> Result<(), String> {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true);
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let inner = format!("{prefix}/{name}");
        if path.is_dir() {
            add_dir(zip, &path, &inner)?;
        } else if name != "session.lock" {
            zip.start_file(inner.as_str(), options)
                .map_err(|e| e.to_string())?;
            let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            std::io::copy(&mut file, zip).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Zips the world and the settings into `backups/<file_name>`. The zip is written under a temporary name
/// and renamed when complete, so a backup that looks finished is one.
pub fn create(dir: &Path, config: &Config, file_name: &str) -> Result<PathBuf, String> {
    let backups = dir.join(DIR);
    std::fs::create_dir_all(&backups).map_err(|e| format!("{}: {e}", backups.display()))?;
    let path = backups.join(file_name);
    let part = backups.join(format!("{file_name}.part"));
    let result = (|| {
        let file = File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
        let mut zip = ZipWriter::new(BufWriter::new(file));
        let world = dir.join(&config.world.name);
        if world.is_dir() {
            add_dir(&mut zip, &world, &config.world.name)?;
        }
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for name in FILES {
            if let Ok(bytes) = std::fs::read(dir.join(name)) {
                zip.start_file(name, options).map_err(|e| e.to_string())?;
                zip.write_all(&bytes).map_err(|e| e.to_string())?;
            }
        }
        zip.finish()
            .map_err(|e| e.to_string())?
            .into_inner()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&part, &path).map_err(|e| e.to_string())?;
            Ok(path)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

pub struct Backup {
    pub name: String,
    pub bytes: u64,
}

/// A restore queued by the API for the supervisor to apply after the game stops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restore {
    pub name: String,
    /// `None` replaces the whole world; a resource location replaces only that dimension.
    pub dimension: Option<String>,
}

impl Restore {
    pub fn apply(&self, dir: &Path, config: &Config) -> Result<Option<PathBuf>, String> {
        match &self.dimension {
            Some(dimension) => restore_dimension(dir, config, &self.name, dimension),
            None => restore(dir, config, &self.name),
        }
    }

    pub fn what(&self) -> String {
        self.dimension
            .as_ref()
            .map_or_else(|| self.name.clone(), |d| format!("{} ({d})", self.name))
    }
}

/// The finished backups, newest first.
pub fn list(dir: &Path) -> Vec<Backup> {
    let mut out: Vec<Backup> = std::fs::read_dir(dir.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.ends_with(".zip").then(|| Backup {
                bytes: e.metadata().map_or(0, |m| m.len()),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Deletes the oldest automatic backups beyond `keep`. Backups made by hand are never deleted.
pub fn prune(dir: &Path, keep: usize) -> Vec<String> {
    let auto_suffix = format!("-{AUTO}.zip");
    list(dir)
        .into_iter()
        .filter(|b| b.name.ends_with(&auto_suffix))
        .skip(keep)
        .filter(|b| std::fs::remove_file(dir.join(DIR).join(&b.name)).is_ok())
        .map(|b| b.name)
        .collect()
}

pub fn auto_name(time: SystemTime) -> String {
    file_name(time, Some(AUTO)).expect("the automatic name is valid")
}

/// The backup a name points at: a file in `backups/`, by its full name or the start of it.
pub fn find(dir: &Path, name: &str) -> Result<String, String> {
    let all = list(dir);
    if let Some(b) = all
        .iter()
        .find(|b| b.name == name || b.name == format!("{name}.zip"))
    {
        return Ok(b.name.clone());
    }
    let matches: Vec<&Backup> = all.iter().filter(|b| b.name.starts_with(name)).collect();
    match matches.as_slice() {
        [one] => return Ok(one.name.clone()),
        [] => {}
        _ => {
            return Err(format!(
                "{name:?} matches {} backups: be more precise",
                matches.len()
            ));
        }
    }
    // The name given to `backup NAME` (or `auto`, `manual`): the newest backup with it.
    let label = format!("-{name}.zip");
    all.iter()
        .find(|b| b.name.ends_with(&label))
        .map(|b| b.name.clone())
        .ok_or_else(|| {
            format!("no backup {name:?} in {DIR}/ (`horizon-server backups` lists them)")
        })
}

/// Puts the backup's world in place of the current one, which goes to `backups/replaced-<time>/`. The
/// server must be stopped. Returns where the old world went.
pub fn restore(dir: &Path, config: &Config, name: &str) -> Result<Option<PathBuf>, String> {
    let zip_path = dir.join(DIR).join(name);
    let scratch = dir.join(".horizon/restore");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let file = File::open(&zip_path).map_err(|e| format!("{}: {e}", zip_path.display()))?;
    ZipArchive::new(file)
        .and_then(|mut z| z.extract(&scratch))
        .map_err(|e| format!("{name}: {e}"))?;
    let world = &config.world.name;
    let restored = scratch.join(world);
    if !restored.join("level.dat").is_file() {
        let _ = std::fs::remove_dir_all(&scratch);
        return Err(format!(
            "{name} holds no world named {world:?} (no {world}/level.dat)"
        ));
    }
    let current = dir.join(world);
    let moved = if current.exists() {
        let aside = dir
            .join(DIR)
            .join(format!("replaced-{}", stamp(SystemTime::now())));
        std::fs::create_dir_all(&aside).map_err(|e| e.to_string())?;
        let target = aside.join(world);
        std::fs::rename(&current, &target)
            .map_err(|e| format!("moving the current world aside: {e}"))?;
        Some(target)
    } else {
        None
    };
    std::fs::rename(&restored, &current)
        .map_err(|e| format!("putting the backup in place: {e}"))?;
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(moved)
}

#[derive(Debug)]
struct Dimension {
    path: PathBuf,
}

/// The dimension's path below the world folder. Resource locations are checked before they become paths.
fn dimension(dimension: &str) -> Result<Dimension, String> {
    let Some((namespace, path)) = dimension.split_once(':') else {
        return Err(format!(
            "dimension {dimension:?} needs a namespace, like minecraft:the_nether"
        ));
    };
    if namespace.is_empty()
        || !namespace
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-.".contains(&c))
        || path.is_empty()
        || !path
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-./".contains(&c))
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("invalid dimension resource location {dimension:?}"));
    }
    Ok(Dimension {
        path: PathBuf::from("dimensions").join(namespace).join(path),
    })
}

/// Checks the public dimension argument without opening a backup.
pub fn validate_dimension(dimension: &str) -> Result<(), String> {
    self::dimension(dimension).map(|_| ())
}

fn dimension_entry(path: &Path, prefix: &Path) -> bool {
    path.strip_prefix(prefix)
        .is_ok_and(|relative| !relative.as_os_str().is_empty())
}

fn open(dir: &Path, name: &str) -> Result<ZipArchive<File>, String> {
    let path = dir.join(DIR).join(name);
    let file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    ZipArchive::new(file).map_err(|e| format!("{name}: {e}"))
}

/// Refuses a missing dimension before a running server is stopped for the restore.
pub fn check_dimension(
    dir: &Path,
    config: &Config,
    name: &str,
    dimension_id: &str,
) -> Result<(), String> {
    let dimension = dimension(dimension_id)?;
    let prefix = Path::new(&config.world.name).join(&dimension.path);
    let mut zip = open(dir, name)?;
    let mut found = false;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|e| format!("{name}: {e}"))?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if dimension_entry(&path, &prefix) {
            found = true;
        }
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "{name} holds no dimension {dimension_id:?} for world {:?}",
            config.world.name
        ))
    }
}

fn extract_dimension(
    dir: &Path,
    config: &Config,
    name: &str,
    dimension: &Dimension,
    scratch: &Path,
) -> Result<(), String> {
    let prefix = Path::new(&config.world.name).join(&dimension.path);
    let mut zip = open(dir, name)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| format!("{name}: {e}"))?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if !dimension_entry(&path, &prefix) {
            continue;
        }
        let target = scratch.join(path);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut output = File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        std::io::copy(&mut entry, &mut output).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(())
}

fn replaced(dir: &Path) -> Result<PathBuf, String> {
    let base = format!("replaced-{}", stamp(SystemTime::now()));
    for suffix in 0..1000 {
        let name = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        let path = dir.join(DIR).join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err("could not choose a folder for the replaced dimension".into())
}

/// Restores one dimension while leaving every other dimension as it is. The overwritten files are moved
/// under `backups/replaced-<time>/`, just like a whole-world restore.
pub fn restore_dimension(
    dir: &Path,
    config: &Config,
    name: &str,
    dimension_id: &str,
) -> Result<Option<PathBuf>, String> {
    check_dimension(dir, config, name, dimension_id)?;
    let dimension = dimension(dimension_id)?;
    let scratch = dir.join(".horizon/restore");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| e.to_string())?;
    let result = (|| {
        extract_dimension(dir, config, name, &dimension, &scratch)?;
        let world = Path::new(&config.world.name);
        let restored = scratch.join(world).join(&dimension.path);
        let current = dir.join(world).join(&dimension.path);
        let moved = if current.exists() {
            let aside = replaced(dir)?;
            let target = aside.join(world).join(&dimension.path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&current, &target)
                .map_err(|e| format!("moving the current dimension aside: {e}"))?;
            Some(target)
        } else {
            None
        };
        if let Some(parent) = current.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&restored, &current)
            .map_err(|e| format!("putting the backup's dimension in place: {e}"))?;
        Ok(moved)
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// `every` from server.toml: `"6h"`, `"30m"`, `"1d"`, or `"0"` for no automatic backups.
pub fn interval(every: &str) -> Result<Option<std::time::Duration>, String> {
    let every = every.trim();
    if every == "0" || every.is_empty() {
        return Ok(None);
    }
    let (number, unit) = every.split_at(every.len() - 1);
    let n: u64 = number.trim().parse().map_err(|_| {
        format!("backups.every is {every:?}: use a number and m, h or d, like \"6h\"")
    })?;
    let secs = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => {
            return Err(format!(
                "backups.every is {every:?}: use a number and m, h or d, like \"6h\""
            ));
        }
    };
    Ok((n > 0).then(|| std::time::Duration::from_secs(n * secs)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_by_time() {
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_792_000_000);
        assert_eq!(
            file_name(t, Some("before-update")).unwrap(),
            "2026-10-14_17-46-40-before-update.zip"
        );
        assert!(file_name(t, Some("../x")).is_err());
        assert_eq!(auto_name(t), "2026-10-14_17-46-40-auto.zip");
    }

    #[test]
    fn intervals() {
        assert_eq!(
            interval("6h").unwrap(),
            Some(std::time::Duration::from_secs(21_600))
        );
        assert_eq!(
            interval("30m").unwrap(),
            Some(std::time::Duration::from_secs(1800))
        );
        assert_eq!(interval("0").unwrap(), None);
        assert!(interval("6 hours").is_err());
    }

    #[test]
    fn back_up_and_restore() {
        let dir =
            std::env::temp_dir().join(format!("horizon-server-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::default();
        let world = dir.join("world");
        std::fs::create_dir_all(world.join("region")).unwrap();
        std::fs::write(world.join("level.dat"), b"before").unwrap();
        std::fs::write(world.join("region/r.0.0.mca"), vec![7u8; 10_000]).unwrap();
        std::fs::write(world.join("session.lock"), b"x").unwrap();
        std::fs::write(dir.join("server.toml"), crate::config::TEMPLATE).unwrap();
        let t = SystemTime::now();
        let name = file_name(t, None).unwrap();
        create(&dir, &config, &name).unwrap();
        assert_eq!(list(&dir).len(), 1);
        // The world changes, then the backup comes back.
        std::fs::write(world.join("level.dat"), b"after").unwrap();
        let moved = restore(&dir, &config, &find(&dir, &name[..10]).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read(world.join("level.dat")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(world.join("region/r.0.0.mca")).unwrap().len(),
            10_000
        );
        assert!(!world.join("session.lock").exists());
        assert_eq!(std::fs::read(moved.join("level.dat")).unwrap(), b"after");
        // Only automatic backups are pruned.
        for i in 0..3 {
            let later = t + std::time::Duration::from_secs(10 + i);
            create(&dir, &config, &auto_name(later)).unwrap();
        }
        assert_eq!(prune(&dir, 1).len(), 2);
        let left: Vec<String> = list(&dir).into_iter().map(|b| b.name).collect();
        assert_eq!(left.len(), 2);
        assert!(left.iter().any(|n| n.ends_with("-manual.zip")));
        // A backup is found by its name, or the newest with a label.
        assert_eq!(find(&dir, "manual").unwrap(), name);
        assert_eq!(find(&dir, &left[0]).unwrap(), left[0]);
        assert_eq!(
            find(&dir, left[0].trim_end_matches(".zip")).unwrap(),
            left[0]
        );
        assert_eq!(find(&dir, "auto").unwrap(), left[0]);
        assert!(find(&dir, "nothing").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn restores_one_dimension_without_changing_the_others() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-server-dimension-restore-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::default();
        let world = dir.join("world");
        for (path, contents) in [
            ("level.dat", "level-before"),
            (
                "dimensions/minecraft/overworld/region/r.0.0.mca",
                "overworld-before",
            ),
            (
                "dimensions/minecraft/the_nether/region/r.0.0.mca",
                "nether-before",
            ),
            (
                "dimensions/minecraft/the_end/region/r.0.0.mca",
                "end-before",
            ),
            ("dimensions/acme/moon/region/r.0.0.mca", "moon-before"),
        ] {
            let path = world.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        let name = file_name(SystemTime::now(), Some("dimensions")).unwrap();
        create(&dir, &config, &name).unwrap();

        let write = |path: &str, contents: &str| {
            std::fs::write(world.join(path), contents).unwrap();
        };
        write("level.dat", "level-after");
        write(
            "dimensions/minecraft/overworld/region/r.0.0.mca",
            "overworld-after",
        );
        write(
            "dimensions/minecraft/the_nether/region/r.0.0.mca",
            "nether-after",
        );
        write("dimensions/minecraft/the_end/region/r.0.0.mca", "end-after");
        write("dimensions/acme/moon/region/r.0.0.mca", "moon-after");

        let moved = restore_dimension(&dir, &config, &name, "minecraft:the_nether")
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/minecraft/the_nether/region/r.0.0.mca"))
                .unwrap(),
            "nether-before"
        );
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/minecraft/overworld/region/r.0.0.mca"))
                .unwrap(),
            "overworld-after"
        );
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/minecraft/the_end/region/r.0.0.mca"))
                .unwrap(),
            "end-after"
        );
        assert_eq!(
            std::fs::read_to_string(moved.join("region/r.0.0.mca")).unwrap(),
            "nether-after"
        );

        write(
            "dimensions/minecraft/the_nether/region/r.0.0.mca",
            "nether-kept",
        );
        let moved = restore_dimension(&dir, &config, &name, "minecraft:overworld")
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/minecraft/overworld/region/r.0.0.mca"))
                .unwrap(),
            "overworld-before"
        );
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/minecraft/the_nether/region/r.0.0.mca"))
                .unwrap(),
            "nether-kept"
        );
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/minecraft/the_end/region/r.0.0.mca"))
                .unwrap(),
            "end-after"
        );
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/acme/moon/region/r.0.0.mca")).unwrap(),
            "moon-after"
        );
        assert_eq!(
            std::fs::read_to_string(moved.join("region/r.0.0.mca")).unwrap(),
            "overworld-after"
        );
        assert_eq!(
            std::fs::read_to_string(world.join("level.dat")).unwrap(),
            "level-after"
        );

        restore_dimension(&dir, &config, &name, "acme:moon").unwrap();
        assert_eq!(
            std::fs::read_to_string(world.join("dimensions/acme/moon/region/r.0.0.mca")).unwrap(),
            "moon-before"
        );
        assert!(check_dimension(&dir, &config, &name, "acme:missing").is_err());
        for invalid in ["the_nether", "../end", "minecraft:../end", "Minecraft:end"] {
            assert!(validate_dimension(invalid).is_err(), "{invalid}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
