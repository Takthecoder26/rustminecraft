//! Explicit, blocking Realms transfers. Call only from an IO worker after confirmation.
//! Follows 26.3 FileUpload, FileDownload and RealmsUploadWorldPacker. Credentials
//! and endpoint validation remain in realms_service; archives never contain them.
use crate::realms_service::{RealmsAuth, RealmsClient, RealmsError, RealmsResult};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use horizon_world::nbt::{self, Limits, Tag};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_ARCHIVE: u64 = 5 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = MAX_ARCHIVE;
const MAX_ENTRIES: usize = 100_000;
const MAX_METADATA: u64 = 64 * 1024;
const MAX_LEVEL: u64 = 64 * 1024 * 1024;
const MAX_PACK: u64 = 250 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
static NEXT_TASK: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TransferStage {
    Preparing,
    Packing,
    Uploading,
    Downloading,
    Extracting,
    Installing,
    Complete,
}

/// Clones share cancellation and progress. A fresh control belongs to one transfer.
/// Bytes reset at each stage. Zero total means the service did not supply a length.
/// Cancellation is checked around IO chunks; transfer sockets have 10-second idle
/// timeouts. The 600-second deadline is cooperative, not a hard request bound.
/// Cancellation cannot undo an accepted upload.
#[derive(Clone, Default)]
pub struct TransferControl {
    pub cancel: Arc<AtomicBool>,
    pub bytes_done: Arc<AtomicU64>,
    pub bytes_total: Arc<AtomicU64>,
    pub stage: Arc<AtomicU8>,
}
impl TransferControl {
    fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(io::Error::other("mco.upload.cancelled"))
        } else {
            Ok(())
        }
    }
    fn begin(&self, stage: TransferStage, total: u64) {
        self.bytes_done.store(0, Ordering::Relaxed);
        self.bytes_total.store(total, Ordering::Relaxed);
        self.stage.store(stage as u8, Ordering::Release);
    }
}

pub fn upload_world(
    client: &RealmsClient,
    auth: &RealmsAuth,
    id: i64,
    slot: i32,
    folder: &Path,
) -> Result<(), String> {
    upload_world_with_control(client, auth, id, slot, folder, &TransferControl::default())
}
pub fn upload_world_with_control(
    client: &RealmsClient,
    auth: &RealmsAuth,
    id: i64,
    slot: i32,
    folder: &Path,
    control: &TransferControl,
) -> Result<(), String> {
    let realm = retry(control, 20, || client.get_own_realm(auth, id))?;
    let options = slot_update(&realm.slots, slot)?;
    upload_world_with_options_control(client, auth, id, slot, folder, &options, control)
}

pub fn upload_world_with_options_control(
    client: &RealmsClient,
    auth: &RealmsAuth,
    id: i64,
    slot: i32,
    folder: &Path,
    options: &Value,
    control: &TransferControl,
) -> Result<(), String> {
    transfer_input(id, slot)?;
    if options.get("slotId").and_then(Value::as_i64) != Some(i64::from(slot)) {
        return Err(RealmsError::InvalidInput.to_string());
    }
    control.check().map_err(io_error)?;
    // Hold the same session lock as a running world for the entire snapshot.
    let _lock = lock_world(folder).map_err(io_error)?;
    let scratch = TaskDir::new(&std::env::temp_dir()).map_err(io_error)?;
    let archive = scratch.0.join("upload.tar.gz");
    control.begin(TransferStage::Packing, 0);
    pack_world(folder, &archive, control).map_err(io_error)?;
    let info = retry(control, 20, || client.request_upload_info(auth, id, None))?;
    if !info.world_closed {
        return Err(RealmsError::WorldNotClosed.to_string());
    }
    let len = fs::metadata(&archive).map_err(io_error)?.len();
    retry(control, 5, || {
        control.begin(TransferStage::Uploading, len);
        let file = File::open(&archive).map_err(|_| RealmsError::Transport)?;
        let body = CheckedReader::new(file, MAX_ARCHIVE, control, true);
        client.upload_body_with_cancel(
            auth,
            id,
            slot,
            &info,
            "26.3",
            body,
            len,
            control.cancel.clone(),
        )
    })?;
    // Vanilla applies the selected slot's options/settings after accepting the file.
    retry(control, 20, || client.update_slot(auth, id, slot, options))?;
    control.begin(TransferStage::Complete, 0);
    Ok(())
}

pub fn download_world(
    client: &RealmsClient,
    auth: &RealmsAuth,
    id: i64,
    slot: i32,
    saves_root: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    download_world_with_control(
        client,
        auth,
        id,
        slot,
        saves_root,
        name,
        &TransferControl::default(),
    )
}
#[allow(clippy::too_many_arguments)]
pub fn download_world_with_control(
    client: &RealmsClient,
    auth: &RealmsAuth,
    id: i64,
    slot: i32,
    saves_root: &Path,
    name: &str,
    control: &TransferControl,
) -> Result<PathBuf, String> {
    transfer_input(id, slot)?;
    control.check().map_err(io_error)?;
    let info = retry(control, 20, || client.request_download_info(auth, id, slot))?;
    let scratch = TaskDir::new(&std::env::temp_dir()).map_err(io_error)?;
    let archive = scratch.0.join("download.tar.gz");
    control.begin(TransferStage::Downloading, 0);
    download_file(client, &info.download_link, &archive, MAX_ARCHIVE, control)?;
    fs::create_dir_all(saves_root).map_err(io_error)?;
    require_directory(saves_root).map_err(io_error)?;
    let staging = TaskDir::new(saves_root).map_err(io_error)?;
    let world = staging.0.join("world");
    fs::create_dir(&world).map_err(io_error)?;
    control.begin(TransferStage::Extracting, 0);
    extract_world(&archive, &world, control).map_err(io_error)?;
    let clean_name = sanitize_name(name);
    rewrite_level(&world.join("level.dat"), &clean_name, control).map_err(io_error)?;
    match (
        info.resource_pack_url.as_deref().filter(|s| !s.is_empty()),
        info.resource_pack_hash.as_deref().filter(|s| !s.is_empty()),
    ) {
        (Some(url), Some(hash)) => {
            if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("mco.download.failed".into());
            }
            let pack = scratch.0.join("resources.zip");
            control.begin(TransferStage::Downloading, 0);
            download_file(client, url, &pack, MAX_PACK, control)?;
            verify_pack(&pack, hash, control).map_err(io_error)?;
            // An archive-provided resource pack is replaced only inside our private staging.
            let mut input = CheckedReader::new(
                File::open(pack).map_err(io_error)?,
                MAX_PACK,
                control,
                false,
            );
            let mut output = File::create(world.join("resources.zip")).map_err(io_error)?;
            io::copy(&mut input, &mut output).map_err(io_error)?;
            output.sync_all().map_err(io_error)?;
        }
        (None, None) => {}
        _ => return Err("mco.download.failed".into()),
    }
    control.begin(TransferStage::Installing, 0);
    let installed = install_world(&world, saves_root, &clean_name, control).map_err(io_error)?;
    control.begin(TransferStage::Complete, 0);
    Ok(installed)
}

fn transfer_input(id: i64, slot: i32) -> Result<(), String> {
    if id <= 0 || !(1..=3).contains(&slot) {
        Err(RealmsError::InvalidInput.to_string())
    } else {
        Ok(())
    }
}
pub(crate) fn slot_update(
    slots: &serde_json::Value,
    slot: i32,
) -> Result<serde_json::Value, String> {
    let entry = slots
        .as_array()
        .and_then(|v| {
            v.iter()
                .find(|v| v.get("slotId").and_then(|v| v.as_i64()) == Some(i64::from(slot)))
        })
        .ok_or_else(|| RealmsError::InvalidResponse.to_string())?;
    let mut options = match entry.get("options") {
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str(s).map_err(|_| RealmsError::InvalidResponse.to_string())?
        }
        Some(v) => v.clone(),
        None => entry.clone(),
    };
    let map = options
        .as_object_mut()
        .ok_or_else(|| RealmsError::InvalidResponse.to_string())?;
    map.insert("slotId".into(), slot.into());
    map.insert("version".into(), "26.3".into());
    let hardcore = entry
        .get("settings")
        .and_then(|v| v.as_array())
        .and_then(|v| {
            v.iter()
                .find(|setting| setting.get("name").and_then(|v| v.as_str()) == Some("hardcore"))
        })
        .and_then(|setting| setting.get("value").and_then(|v| v.as_str()))
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    map.insert("hardcore".into(), hardcore.into());
    // Exact RealmsSlotUpdateDto fields; settings are used only to derive hardcore.
    map.retain(|key, _| {
        matches!(
            key.as_str(),
            "slotId"
                | "spawnProtection"
                | "forceGameMode"
                | "difficulty"
                | "gameMode"
                | "slotName"
                | "version"
                | "compatibility"
                | "templateId"
                | "templateImage"
                | "hardcore"
        )
    });
    Ok(options)
}
fn retry<T>(
    control: &TransferControl,
    attempts: usize,
    mut action: impl FnMut() -> RealmsResult<T>,
) -> Result<T, String> {
    for attempt in 0..attempts {
        control.check().map_err(io_error)?;
        match action() {
            Ok(value) => {
                control.check().map_err(io_error)?;
                return Ok(value);
            }
            Err(RealmsError::RetryAfter { seconds, .. }) if attempt + 1 < attempts => {
                for _ in 0..seconds.min(120) * 10 {
                    control.check().map_err(io_error)?;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            Err(e) => {
                control.check().map_err(io_error)?;
                return Err(e.to_string());
            }
        }
    }
    Err("mco.download.failed".into())
}
fn io_error(error: io::Error) -> String {
    if error.to_string() == "mco.upload.cancelled" {
        "mco.upload.cancelled".into()
    } else {
        "mco.download.failed".into()
    }
}
fn invalid() -> io::Error {
    io::Error::other("invalid or oversized Realms archive")
}

struct TaskDir(PathBuf);
impl TaskDir {
    fn new(root: &Path) -> io::Result<Self> {
        require_directory(root)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_nanos();
        for _ in 0..128 {
            let path = root.join(format!(
                ".realms-transfer-{}-{stamp}-{}.new",
                std::process::id(),
                NEXT_TASK.fetch_add(1, Ordering::Relaxed)
            ));
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(invalid())
    }
}
impl Drop for TaskDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn metadata(path: &Path) -> io::Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    Ok(meta)
}
fn require_directory(path: &Path) -> io::Result<()> {
    // Reject symlinks/junctions in every existing ancestor, including the root selected by UI.
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if !metadata(ancestor)?.is_dir() {
            return Err(invalid());
        }
    }
    Ok(())
}
fn lock_world(folder: &Path) -> io::Result<File> {
    require_directory(folder)?;
    if !metadata(&folder.join("level.dat"))?.is_file() {
        return Err(invalid());
    }
    let path = folder.join("session.lock");
    match metadata(&path) {
        Ok(m) if !m.is_file() => return Err(invalid()),
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    lock.try_lock()
        .map_err(|_| io::Error::other("world already open"))?;
    Ok(lock)
}

/// Limits include bytes discarded by the tar iterator and extended headers, not
/// just extracted file sizes. One extra byte distinguishes EOF from overflow.
struct CheckedReader<'a, R> {
    inner: R,
    remaining: u64,
    control: &'a TransferControl,
    progress: bool,
}
impl<'a, R> CheckedReader<'a, R> {
    fn new(inner: R, limit: u64, control: &'a TransferControl, progress: bool) -> Self {
        Self {
            inner,
            remaining: limit,
            control,
            progress,
        }
    }
}
impl<R: Read> Read for CheckedReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.control.check()?;
        if buffer.is_empty() {
            return Ok(0);
        }
        let max = buffer
            .len()
            .min(self.remaining.saturating_add(1).min(64 * 1024) as usize);
        let result = self.inner.read(&mut buffer[..max]);
        self.control.check()?;
        let n = result?;
        if n as u64 > self.remaining {
            return Err(invalid());
        }
        self.remaining -= n as u64;
        if self.progress {
            self.control
                .bytes_done
                .fetch_add(n as u64, Ordering::Relaxed);
        }
        Ok(n)
    }
}
struct LimitedWriter<W> {
    inner: W,
    remaining: u64,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.remaining {
            return Err(invalid());
        }
        let n = self.inner.write(buf)?;
        self.remaining -= n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
fn pack_world(folder: &Path, output: &Path, control: &TransferControl) -> io::Result<()> {
    require_directory(folder)?;
    let zipped = GzEncoder::new(
        LimitedWriter {
            inner: new_file(output)?,
            remaining: MAX_ARCHIVE,
        },
        Compression::default(),
    );
    let mut tar = tar::Builder::new(LimitedWriter {
        inner: zipped,
        remaining: MAX_ARCHIVE,
    });
    tar.follow_symlinks(false);
    let mut pending = vec![(folder.to_path_buf(), String::from("world"), 0)];
    let mut count = 0;
    while let Some((path, name, depth)) = pending.pop() {
        control.check()?;
        count += 1;
        if count > MAX_ENTRIES || depth > MAX_DEPTH {
            return Err(invalid());
        }
        let meta = metadata(&path)?;
        if meta.is_dir() {
            tar.append_dir(&name, &path)?;
            for entry in fs::read_dir(&path)? {
                let entry = entry?;
                let filename = entry.file_name().into_string().map_err(|_| invalid())?;
                if filename == "session.lock" {
                    continue;
                }
                safe_component(&filename)?;
                pending.push((entry.path(), format!("{name}/{filename}"), depth + 1));
                if pending.len() + count > MAX_ENTRIES {
                    return Err(invalid());
                }
            }
        } else if meta.is_file() && meta.len() <= MAX_FILE {
            let mut header = tar::Header::new_gnu();
            header.set_size(meta.len());
            header.set_mode(0o600);
            header.set_mtime(0);
            header.set_cksum();
            let file = File::open(&path)?;
            let mut input = CheckedReader::new(file.take(meta.len()), MAX_FILE, control, true);
            tar.append_data(&mut header, &name, &mut input)?;
        } else {
            return Err(invalid());
        }
    }
    let output = tar.into_inner()?.inner.finish()?.inner;
    output.sync_all()?;
    Ok(())
}
fn archive_reader<'a>(
    path: &Path,
    control: &'a TransferControl,
) -> io::Result<CheckedReader<'a, GzDecoder<File>>> {
    if metadata(path)?.len() > MAX_ARCHIVE {
        return Err(invalid());
    }
    Ok(CheckedReader::new(
        GzDecoder::new(File::open(path)?),
        MAX_ARCHIVE,
        control,
        false,
    ))
}
fn extract_world(archive: &Path, output: &Path, control: &TransferControl) -> io::Result<()> {
    // Raw preflight bounds extension allocations before tar resolves PAX/GNU names.
    let mut preflight = tar::Archive::new(archive_reader(archive, control)?);
    for (count, entry) in preflight.entries()?.raw(true).enumerate() {
        let entry = entry?;
        let kind = entry.header().entry_type();
        if count >= MAX_ENTRIES || entry.size() > MAX_FILE {
            return Err(invalid());
        }
        if kind.is_pax_local_extensions() || kind.is_gnu_longname() {
            if entry.size() > MAX_METADATA {
                return Err(invalid());
            }
        } else if !kind.is_file() && !kind.is_dir() {
            return Err(invalid());
        }
    }
    let mut archive = tar::Archive::new(archive_reader(archive, control)?);
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for entry in archive.entries()? {
        control.check()?;
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(invalid());
        }
        let name = String::from_utf8(entry.path_bytes().into_owned()).map_err(|_| invalid())?;
        let path = world_path(&name)?;
        if path.as_os_str().is_empty() {
            if !kind.is_dir() {
                return Err(invalid());
            }
            continue;
        }
        // Case folding also prevents aliases when a download is moved to Windows.
        if !seen.insert(path.to_string_lossy().to_lowercase()) {
            return Err(invalid());
        }
        total = total.checked_add(entry.size()).ok_or_else(invalid)?;
        if entry.size() > MAX_FILE || total > MAX_ARCHIVE || seen.len() > MAX_ENTRIES {
            return Err(invalid());
        }
        let destination = output.join(&path);
        if kind.is_dir() {
            if entry.size() != 0 {
                return Err(invalid());
            }
            fs::create_dir_all(&destination)?;
        } else {
            fs::create_dir_all(destination.parent().ok_or_else(invalid)?)?;
            let size = entry.size();
            let mut file = new_file(&destination)?;
            let copied = io::copy(
                &mut CheckedReader::new(&mut entry, MAX_FILE, control, true),
                &mut file,
            )?;
            if copied != size {
                return Err(invalid());
            }
        }
    }
    // Consume trailing gzip data so corrupt CRC/truncation cannot publish a save.
    io::copy(&mut archive.into_inner(), &mut io::sink())?;
    if !metadata(&output.join("level.dat"))?.is_file() {
        return Err(invalid());
    }
    let lock = output.join("session.lock");
    if lock.exists() {
        fs::remove_file(lock)?;
    }
    Ok(())
}
fn safe_component(s: &str) -> io::Result<()> {
    let base = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && base.as_bytes()[3].is_ascii_digit());
    if s.is_empty()
        || s == "."
        || s == ".."
        || s.len() > 255
        || s.ends_with(['.', ' '])
        || reserved
        || s.chars()
            .any(|c| c.is_control() || "\\/:<>\"|?*".contains(c))
    {
        return Err(invalid());
    }
    Ok(())
}
fn world_path(name: &str) -> io::Result<PathBuf> {
    let name = name.trim_end_matches('/');
    if name.len() > 4096 {
        return Err(invalid());
    }
    let mut parts = name.split('/');
    if parts.next() != Some("world") {
        return Err(invalid());
    }
    let mut path = PathBuf::new();
    for (depth, part) in parts.enumerate() {
        if depth >= MAX_DEPTH {
            return Err(invalid());
        }
        safe_component(part)?;
        path.push(part);
    }
    Ok(path)
}
fn sanitize_name(name: &str) -> String {
    let mut s: String = name
        .trim()
        .chars()
        .take(80)
        .map(|c| {
            if c.is_control() || "\\/:<>\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    while s.len() > 160 {
        s.pop();
    }
    s = s.trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() || s == "." || s == ".." {
        s = "Realm".into();
    }
    if safe_component(&s).is_err() {
        s = format!("_{s}");
    }
    s
}
fn rewrite_level(path: &Path, name: &str, control: &TransferControl) -> io::Result<()> {
    let mut bytes = Vec::new();
    CheckedReader::new(GzDecoder::new(File::open(path)?), MAX_LEVEL, control, false)
        .read_to_end(&mut bytes)?;
    let (mut root, consumed) = nbt::read_named(&bytes, Limits::default()).map_err(|_| invalid())?;
    if consumed != bytes.len() {
        return Err(invalid());
    }
    let Some(Tag::Compound(data)) = root.tag.get_mut("Data") else {
        return Err(invalid());
    };
    data.remove(&"Player".into());
    data.insert("LevelName".into(), Tag::String(name.into()));
    let bytes = nbt::write_named(&root, Limits::default()).map_err(|_| invalid())?;
    let mut zipped = GzEncoder::new(File::create(path)?, Compression::default());
    zipped.write_all(&bytes)?;
    zipped.finish()?.sync_all()?;
    control.check()
}
fn download_file(
    client: &RealmsClient,
    url: &str,
    path: &Path,
    limit: u64,
    control: &TransferControl,
) -> Result<(), String> {
    // Service URL validation and no-redirect policy apply equally to world and pack.
    let body = retry(control, 20, || {
        client.download_body_with_cancel(url, control.cancel.clone())
    })?;
    let mut reader = CheckedReader::new(body, limit, control, true);
    let mut output = new_file(path).map_err(io_error)?;
    io::copy(&mut reader, &mut output).map_err(io_error)?;
    output.sync_all().map_err(io_error)
}
fn verify_pack(path: &Path, expected: &str, control: &TransferControl) -> io::Result<()> {
    let mut reader = CheckedReader::new(File::open(path)?, MAX_PACK, control, false);
    let mut digest = Sha1::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    if !format!("{:x}", digest.finalize()).eq_ignore_ascii_case(expected) {
        return Err(invalid());
    }
    Ok(())
}

fn install_world(
    world: &Path,
    root: &Path,
    name: &str,
    control: &TransferControl,
) -> io::Result<PathBuf> {
    require_directory(root)?;
    for suffix in 0..10_000 {
        control.check()?;
        let name = if suffix == 0 {
            name.to_string()
        } else {
            format!("{name}-{suffix}")
        };
        let destination = root.join(name);
        match fs::create_dir(&destination) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        // Exclusive directory reservation never replaces an existing world, even
        // when another transfer picks the same name. Roll back partial installation.
        let guard = TaskDir(destination.clone());
        for entry in fs::read_dir(world)? {
            control.check()?;
            let entry = entry?;
            let target = destination.join(entry.file_name());
            if fs::symlink_metadata(&target).is_ok() {
                return Err(invalid());
            }
            fs::rename(entry.path(), target)?;
        }
        control.check()?;
        std::mem::forget(guard);
        return Ok(destination);
    }
    Err(invalid())
}
