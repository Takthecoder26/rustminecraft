//! Realms template and minigame thumbnails (`RealmsTextureManager.worldTemplate`).
//! Service images are Base64 PNGs, not URLs. Decode and resize off the UI thread;
//! callers bind the cached RGBA to the atlas's reusable pack-icon slots each frame.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, mpsc},
};

/// Matches the reusable `hud::PACK_ICON_SIZE` slots.
pub const IMAGE_SIZE: u32 = 128;
const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENCODED_BYTES: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;
const MAX_PIXELS: u64 = 4096 * 4096;
const MAX_ENTRIES: usize = 128;
const MAX_CACHED_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_WORKERS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Ready,
    Failed,
}

struct Entry {
    source: Arc<str>,
    generation: u64,
    status: Status,
    rgba: Option<Vec<u8>>,
}

struct Completed {
    location: String,
    generation: u64,
    rgba: Option<Vec<u8>>,
}

/// Bounded thumbnail cache. A full worker pool returns `Pending` without queuing
/// another job: request visible images again next frame. Errors are cached until
/// that id's content changes. No network requests or file I/O happen here.
pub struct Cache {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    source_bytes: usize,
    generation: u64,
    in_flight: usize,
    send: mpsc::Sender<Completed>,
    done: mpsc::Receiver<Completed>,
    ready: HashSet<String>,
}

impl Default for Cache {
    fn default() -> Self {
        let (send, done) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            source_bytes: 0,
            generation: 0,
            in_flight: 0,
            send,
            done,
            ready: HashSet::new(),
        }
    }
}

impl Cache {
    /// Start decoding, or reuse the same id and content. Missing (`null`) images
    /// are handled by the caller with `gui/presets/isles`; an empty string is an
    /// invalid image and becomes `Failed`, as does an oversized service payload.
    pub fn request(&mut self, id: &str, image: &str) -> Status {
        self.poll();
        if id.len() > 256 {
            return Status::Failed;
        }
        let location = format!("realms/dynamic/{id}");
        if image.len() > MAX_ENCODED_BYTES {
            self.remove(&location);
            return Status::Failed;
        }
        if let Some(entry) = self.entries.get(&location)
            && entry.source.as_ref() == image
        {
            let status = entry.status;
            self.touch(&location);
            return status;
        }
        // Changed content immediately stops exposing the previous pixels, even
        // when both workers are still finishing old requests.
        self.remove(&location);
        if self.in_flight >= MAX_WORKERS {
            return Status::Pending;
        }
        while self.entries.len() >= MAX_ENTRIES
            || self.source_bytes + image.len() > MAX_CACHED_SOURCE_BYTES
        {
            let Some(oldest) = self.order.front().cloned() else {
                break;
            };
            self.remove(&oldest);
        }
        self.generation += 1;
        let generation = self.generation;
        let source: Arc<str> = image.into();
        self.source_bytes += source.len();
        self.entries.insert(
            location.clone(),
            Entry {
                source: source.clone(),
                generation,
                status: Status::Pending,
                rgba: None,
            },
        );
        self.touch(&location);
        let (send, job_location) = (self.send.clone(), location.clone());
        let spawned = std::thread::Builder::new()
            .name("Realms image".into())
            .spawn(move || {
                let rgba = decode(&source);
                let _ = send.send(Completed {
                    location: job_location,
                    generation,
                    rgba,
                });
            });
        if spawned.is_err() {
            self.entries.get_mut(&location).unwrap().status = Status::Failed;
            self.ready.insert(location);
            return Status::Failed;
        }
        self.in_flight += 1;
        Status::Pending
    }

    /// Accept finished jobs without waiting. Replaced/evicted content and jobs
    /// from before `clear` cannot put stale pixels back into the cache.
    pub fn poll(&mut self) {
        while let Ok(completed) = self.done.try_recv() {
            self.in_flight -= 1;
            if let Some(entry) = self.entries.get_mut(&completed.location)
                && entry.generation == completed.generation
            {
                entry.status = if completed.rgba.is_some() {
                    Status::Ready
                } else {
                    Status::Failed
                };
                entry.rgba = completed.rgba;
                self.ready.insert(completed.location);
            }
        }
    }

    /// Cached 128x128 RGBA pixels, for binding every frame. Call `poll` or
    /// `request` first. The location is `realms/dynamic/{id}`.
    pub fn image(&self, location: &str) -> Option<&[u8]> {
        self.entries.get(location)?.rgba.as_deref()
    }

    /// Queue cached success/failure for another upload after an atlas reload.
    /// `false` means request the service image again (not cached or still pending).
    pub fn again(&mut self, location: &str) -> bool {
        if self
            .entries
            .get(location)
            .is_none_or(|entry| entry.status == Status::Pending)
        {
            return false;
        }
        self.touch(location);
        self.ready.insert(location.to_owned());
        true
    }

    /// Completed locations with 128x128 RGBA, or `None` for the missing texture.
    /// Taking these does not consume the cache; `image` and `again` still work.
    pub fn take_ready(&mut self) -> Vec<(String, Option<Vec<u8>>)> {
        self.poll();
        self.ready
            .drain()
            .filter_map(|location| {
                let image = self.entries.get(&location)?.rgba.clone();
                Some((location, image))
            })
            .collect()
    }

    /// Forget an account/session's cached images without waiting for workers.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.ready.clear();
        self.source_bytes = 0;
    }

    fn touch(&mut self, location: &str) {
        self.order.retain(|key| key != location);
        self.order.push_back(location.to_owned());
    }

    fn remove(&mut self, location: &str) {
        if let Some(entry) = self.entries.remove(location) {
            self.source_bytes -= entry.source.len();
        }
        self.order.retain(|key| key != location);
        self.ready.remove(location);
    }
}

fn decode(source: &str) -> Option<Vec<u8>> {
    if source.len() > MAX_ENCODED_BYTES {
        return None;
    }
    let data = crate::skins::base64_decode(source)?;
    if data.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    // NativeImage and hud::decode_bytes tolerate incorrect PNG checksums.
    decoder.ignore_checksums(true);
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let header = decoder.read_header_info().ok()?;
    if header.width == 0
        || header.height == 0
        || u64::from(header.width) * u64::from(header.height) > MAX_PIXELS
    {
        return None;
    }
    let mut reader = decoder.read_info().ok()?;
    let size = reader.output_buffer_size()?;
    if size as u64 > MAX_PIXELS * 4 {
        return None;
    }
    let mut pixels = vec![0; size];
    let info = reader.next_frame(&mut pixels).ok()?;
    let channels = info.color_type.samples();
    let side = IMAGE_SIZE as usize;
    let mut rgba = vec![0; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let sx = x * info.width as usize / side;
            let sy = y * info.height as usize / side;
            let p = &pixels[(sy * info.width as usize + sx) * channels..];
            let color = match channels {
                4 => [p[0], p[1], p[2], p[3]],
                3 => [p[0], p[1], p[2], 255],
                2 => [p[0], p[0], p[0], p[1]],
                _ => [p[0], p[0], p[0], 255],
            };
            rgba[(y * side + x) * 4..][..4].copy_from_slice(&color);
        }
    }
    Some(rgba)
}
