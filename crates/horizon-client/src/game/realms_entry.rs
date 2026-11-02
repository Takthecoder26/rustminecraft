//! Effects shared by the lightweight title and the fully loaded game.
//! 26.3 GetServerDetailsTask waits for pack feedback before connecting;
//! RealmsCreateWorldFlow uploads metadata without starting an integrated server.
use super::{Game, realms::Completion};
use horizon_client::{
    auth, datapack, lang, packs,
    realms_service::{RealmsAuth, RealmsClient},
    realms_transfer::{self, TransferControl},
    save,
    server_packs::{self, Request, download},
    title::{Action, Mode, Title, WorldOptions, realms::Page},
};
use horizon_world::nbt::Tag;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

pub(super) enum EntryEffect {
    Join(RealmJoin),
    OpenUri(String),
}

/// Keep this value intact across StartupMenu -> PendingWorldEntry -> Game.
/// A cached pack still needs a successful Game resource reload before joining.
pub(super) struct RealmJoin {
    pub realm_id: i64,
    pub address: String,
    pub pack: Option<(u128, PathBuf)>,
    identity: Arc<auth::AuthSession>,
}
impl RealmJoin {
    fn valid(&self) -> bool {
        same_account(&self.identity)
    }
}
struct PackOffer {
    join: RealmJoin,
    request: Request,
}
struct PackDownload {
    offer: PackOffer,
    queue: download::Queue,
    progress: Receiver<download::Progress>,
    progress_closed: Option<std::time::Instant>,
}
struct CreateTarget {
    id: i64,
    slot: i32,
    identity: Arc<auth::AuthSession>,
}
struct Upload {
    identity: Arc<auth::AuthSession>,
    control: TransferControl,
    result: Receiver<Result<(), String>>,
}
#[derive(Default)]
pub(super) struct RealmEntryState {
    offer: Option<PackOffer>,
    download: Option<PackDownload>,
    create_target: Option<CreateTarget>,
    upload: Option<Upload>,
    joining: Option<RealmJoin>,
    clear_pack: bool,
}
fn same_account(identity: &Arc<auth::AuthSession>) -> bool {
    auth::current().is_some_and(|current| Arc::ptr_eq(&current, identity))
}
fn error(title: &mut Title, key: &str) {
    if let Some(state) = title.realms_mut() {
        state.set_error(lang::get(key));
    }
}
fn loading(title: &mut Title, key: &str) {
    if let Some(state) = title.realms_mut() {
        state.set_loading(true);
        state.status = Some(lang::get(key));
    }
}
impl RealmEntryState {
    /// The owner calls this before another Realms action, including CancelRequest.
    /// Queued pack downloads may finish caching, but cannot connect after cancellation.
    pub(super) fn cancel(&mut self) {
        self.offer = None;
        self.download = None;
        self.create_target = None;
        self.clear_pack |= self.joining.take().is_some();
        if let Some(upload) = self.upload.take() {
            upload.control.cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn handle_completion(
        &mut self,
        completion: Completion,
        title: &mut Title,
    ) -> Option<EntryEffect> {
        self.cancel();
        let identity = auth::current()?;
        if !title.allows_multiplayer {
            return None;
        }
        match completion {
            Completion::OpenUri(uri) => Some(EntryEffect::OpenUri(uri)),
            Completion::Join {
                realm_id,
                address,
                resource_pack_url,
                resource_pack_hash,
            } => {
                let join = RealmJoin {
                    realm_id,
                    address,
                    pack: None,
                    identity,
                };
                match (resource_pack_url, resource_pack_hash) {
                    (None, None) => Some(EntryEffect::Join(join)),
                    (Some(url), Some(hash))
                        if server_packs::valid_url(&url)
                            && server_packs::parse_sha1(&hash).is_some() =>
                    {
                        let id = pack_id(title, realm_id);
                        self.offer = Some(PackOffer {
                            join,
                            request: Request {
                                id,
                                url,
                                sha1: server_packs::parse_sha1(&hash),
                            },
                        });
                        if let Some(state) = title.realms_mut() {
                            state.require_resource_pack();
                        }
                        None
                    }
                    _ => {
                        error(title, "mco.download.resourcePack.fail");
                        None
                    }
                }
            }
            Completion::CreateWorldForRealm { id, slot, options } => {
                if id <= 0 || !(1..=3).contains(&slot) {
                    error(title, "mco.create.world.failed");
                    return None;
                }
                self.create_target = Some(CreateTarget { id, slot, identity });
                let seed = options
                    .get("seed")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                title.begin_realms_create(seed);
                None
            }
        }
    }

    pub(super) fn confirm_join_pack(
        &mut self,
        yes: bool,
        title: &mut Title,
    ) -> Option<EntryEffect> {
        let offer = self.offer.take()?;
        if let Some(state) = title.realms_mut() {
            state.return_page(Page::List);
        }
        if !yes || !offer.join.valid() || !title.allows_multiplayer {
            return None;
        }
        let queue = match std::env::current_dir()
            .and_then(|dir| download::Queue::open(dir.join("downloads")))
        {
            Ok(queue) => queue,
            Err(_) => {
                error(title, "mco.download.resourcePack.fail");
                return None;
            }
        };
        let (send, progress) = mpsc::channel();
        let user = &offer.join.identity;
        queue.download(
            vec![offer.request.clone()],
            download::headers(&user.name, user.uuid),
            send,
        );
        self.download = Some(PackDownload {
            offer,
            queue,
            progress,
            progress_closed: None,
        });
        loading(title, "multiplayer.applyingPack");
        None
    }

    /// Intercept Action::Create before either owner queues a local world entry.
    /// Returns true only when this Create belongs to a Realm.
    pub(super) fn create(&mut self, action: &Action, resources: &Path, title: &mut Title) -> bool {
        let Action::Create {
            name,
            seed,
            creative,
            options,
            ..
        } = action
        else {
            return false;
        };
        let Some(target) = self.create_target.take() else {
            return false;
        };
        title.finish_realms_create();
        let creation_packs = options.pack_dir.clone().map(TemporaryWorld);
        if !same_account(&target.identity) || !title.allows_multiplayer {
            error(title, "mco.error.invalid.session.message");
            return true;
        }
        let (name, seed, creative, options, resources) = (
            name.clone(),
            *seed,
            *creative,
            options.clone(),
            resources.to_path_buf(),
        );
        let identity = target.identity.clone();
        let control = TransferControl::default();
        let worker_control = control.clone();
        let (send, result) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("Realms create world".into())
            .spawn(move || {
                // CreateWorldScreen owns this temporary data-pack directory until
                // its callback accepts Create; ownership passes to this worker.
                let _creation_packs = creation_packs;
                let outcome = (|| {
                    let world = prepare_world(&resources, &name, seed, creative, &options)?;
                    if worker_control.cancel.load(Ordering::Acquire)
                        || !same_account(&target.identity)
                    {
                        return Err("mco.upload.cancelled".into());
                    }
                    let auth = RealmsAuth {
                        access_token: target.identity.access_token().to_owned(),
                        uuid: format!("{:032x}", target.identity.uuid),
                        name: target.identity.name.clone(),
                    };
                    let mut client = RealmsClient::new();
                    // Match RealmsRuntime's gate on this fresh client: feature
                    // flags also select the upload service's request format.
                    client
                        .refresh_feature_flags(&auth)
                        .map_err(|e| e.to_string())?;
                    if client.client_compatible(&auth).map_err(|e| e.to_string())? != "COMPATIBLE" {
                        return Err("mco.errorMessage.6001".into());
                    }
                    if !client
                        .has_parental_consent(&auth)
                        .map_err(|e| e.to_string())?
                    {
                        return Err("mco.account.privacy.information".into());
                    }
                    if worker_control.cancel.load(Ordering::Acquire)
                        || !same_account(&target.identity)
                    {
                        return Err("mco.upload.cancelled".into());
                    }
                    let slot_options = new_slot_options(target.slot, &name, creative, &options);
                    realms_transfer::upload_world_with_options_control(
                        &client,
                        &auth,
                        target.id,
                        target.slot,
                        &world.0,
                        &slot_options,
                        &worker_control,
                    )
                })();
                let _ = send.send(outcome);
            });
        if started.is_err() {
            error(title, "mco.create.world.failed");
        } else {
            self.upload = Some(Upload {
                identity,
                control,
                result,
            });
            loading(title, "mco.upload.preparing");
        }
        true
    }

    /// Call from both menu owners every frame, even while no service request runs.
    pub(super) fn poll(&mut self, title: &mut Title) -> Option<EntryEffect> {
        if title.take_realms_create_cancelled() {
            self.create_target = None;
        }
        if self
            .create_target
            .as_ref()
            .is_some_and(|target| !same_account(&target.identity))
        {
            self.create_target = None;
            title.finish_realms_create();
            error(title, "mco.error.invalid.session.message");
        }
        // Back, account changes, and privacy restrictions invalidate work already started.
        let active =
            title.allows_multiplayer && matches!(&title.mode, Mode::Realms(state) if state.loading);
        if self.offer.as_ref().is_some_and(|offer| !offer.join.valid())
            || !matches!(&title.mode, Mode::Realms(state) if state.page == Page::ResourcePack)
        {
            self.offer = None;
        }
        if self
            .download
            .as_ref()
            .is_some_and(|download| !active || !download.offer.join.valid())
            || self
                .upload
                .as_ref()
                .is_some_and(|upload| !active || !same_account(&upload.identity))
        {
            self.cancel();
            return None;
        }
        if let Some(download) = &mut self.download {
            let batches = download.queue.finished();
            if let Some(batch) = batches.into_iter().next() {
                let id = download.offer.request.id;
                let path = batch
                    .downloaded
                    .into_iter()
                    .find_map(|(got, path)| (got == id).then_some(path));
                let mut done = self.download.take()?;
                if let Some(path) = path {
                    done.offer.join.pack = Some((id, path));
                    return Some(EntryEffect::Join(done.offer.join));
                }
                error(title, "mco.download.resourcePack.fail");
            } else if download
                .progress_closed
                .is_some_and(|at| at.elapsed().as_secs() >= 1)
            {
                self.download = None;
                error(title, "mco.download.resourcePack.fail");
            } else {
                // Allow the queue to publish its result after dropping progress.
                // A panicked worker must not leave the title waiting forever.
                loop {
                    match download.progress.try_recv() {
                        Ok(_) => {}
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            download
                                .progress_closed
                                .get_or_insert_with(std::time::Instant::now);
                            break;
                        }
                    }
                }
            }
        }
        if let Some(upload) = &self.upload {
            let stage = upload.control.stage.load(Ordering::Acquire);
            let key = match stage {
                1 => "mco.upload.preparing",
                2 => "mco.upload.verifying",
                _ => "mco.upload.preparing",
            };
            if let Some(state) = title.realms_mut() {
                let total = upload.control.bytes_total.load(Ordering::Relaxed);
                let done = upload.control.bytes_done.load(Ordering::Relaxed);
                state.status = Some(if total == 0 {
                    lang::get(key)
                } else {
                    format!(
                        "{} · {}%",
                        lang::get(key),
                        done.saturating_mul(100)
                            .checked_div(total)
                            .unwrap_or(0)
                            .min(100)
                    )
                });
            }
            let result = match upload.result.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("mco.create.world.failed".into())),
            };
            if let Some(result) = result {
                self.upload = None;
                if let Some(state) = title.realms_mut() {
                    state.return_page(Page::Slots);
                    match result {
                        Ok(()) => state.set_status(lang::get("mco.upload.done")),
                        Err(_) => state.set_error(lang::get("mco.create.world.failed")),
                    }
                }
            }
        }
        None
    }
}
impl Drop for RealmEntryState {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Game<'_> {
    /// Called only after the full game's resources have loaded. This is also the
    /// PendingWorldEntry::Realm branch after StartupMenu hands over the title.
    pub(super) fn begin_realm_join(&mut self, join: RealmJoin) -> horizon_client::Result<()> {
        self.realms.connected_realm = None;
        if std::mem::take(&mut self.realms_entry.clear_pack) {
            self.server_packs_disconnect();
        }
        if !join.valid()
            || !self
                .title
                .as_ref()
                .is_some_and(|title| title.allows_multiplayer)
        {
            return Ok(());
        }
        if let Some((id, path)) = &join.pack {
            self.begin_realms_pack(*id, path.clone());
            self.realms_entry.joining = Some(join);
            if let Some(title) = &mut self.title {
                loading(title, "multiplayer.applyingPack");
            }
        } else {
            self.realms.connected_realm = None;
            self.connect_server_with_options(
                &join.address,
                &self.options.player_name(),
                None,
                false,
            )?;
            // connect_server_with_options reports some failures through the
            // title and returns Ok; only an actual remote session counts.
            self.realms.connected_realm = self.remote.as_ref().map(|_| join.realm_id);
        }
        Ok(())
    }
    /// Drive after tick_server_packs. Never infer successful application merely
    /// from an empty reload handle: failed reloads also clear that handle.
    pub(super) fn poll_realm_join(&mut self) -> horizon_client::Result<()> {
        if std::mem::take(&mut self.realms_entry.clear_pack) {
            self.server_packs_disconnect();
        }
        let Some(join) = &self.realms_entry.joining else {
            return Ok(());
        };
        if !join.valid()
            || !self.title.as_ref().is_some_and(|title| {
                title.allows_multiplayer && matches!(title.mode, Mode::Realms(_))
            })
        {
            self.realms_entry.joining = None;
            self.server_packs_disconnect();
            return Ok(());
        }
        let Some(ok) = self.take_realms_pack_result() else {
            return Ok(());
        };
        let Some(join) = self.realms_entry.joining.take() else {
            return Ok(());
        };
        if ok {
            self.realms.connected_realm = None;
            self.connect_server_with_options(
                &join.address,
                &self.options.player_name(),
                None,
                false,
            )?;
            // connect_server_with_options reports some failures through the
            // title and returns Ok; only an actual remote session counts.
            self.realms.connected_realm = self.remote.as_ref().map(|_| join.realm_id);
        } else {
            self.server_packs_disconnect();
            if let Some(title) = &mut self.title {
                error(title, "mco.download.resourcePack.fail");
            }
        }
        Ok(())
    }
}

/// UUID.nameUUIDFromBytes(GetServerDetailsTask.generatePackId's UTF-8 recipe).
fn pack_id(title: &mut Title, realm_id: i64) -> u128 {
    let server = title
        .realms_mut()
        .and_then(|state| {
            if state.server.get("id").and_then(Value::as_i64) == Some(realm_id) {
                Some(state.server.clone())
            } else {
                state
                    .servers
                    .iter()
                    .find(|s| s.get("id").and_then(Value::as_i64) == Some(realm_id))
                    .cloned()
            }
        })
        .unwrap_or(Value::Null);
    let recipe = if let Some(name) = server.get("minigameName").and_then(Value::as_str) {
        format!("minigame:{name}")
    } else {
        format!(
            "realms:{}:{}",
            server
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            server
                .get("activeSlot")
                .and_then(Value::as_i64)
                .unwrap_or(1)
        )
    };
    let mut bytes = horizon_protocol::crypto::md5(recipe.as_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    u128::from_be_bytes(bytes)
}

/// Owns only a directory exclusively created by this request; never a user save.
struct TemporaryWorld(PathBuf);
impl Drop for TemporaryWorld {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
static NEXT_WORLD: AtomicU64 = AtomicU64::new(0);
fn prepare_world(
    resources: &Path,
    name: &str,
    seed: Option<i64>,
    creative: bool,
    options: &WorldOptions,
) -> Result<TemporaryWorld, String> {
    let fail = |_| "mco.create.world.failed".to_owned();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let world = std::env::temp_dir().join(format!(
        "minecraft_realms_world_upload-{}-{nanos}-{}",
        std::process::id(),
        NEXT_WORLD.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&world).map_err(fail)?;
    let world = TemporaryWorld(world);
    let seed = seed.unwrap_or_else(|| {
        let t = nanos as u64;
        (t ^ (t >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64
    });
    let mut disabled: Vec<String> = datapack::feature_packs(resources)
        .into_iter()
        .map(|pack| pack.id)
        .filter(|id| !options.data_packs.contains(id))
        .collect();
    if let Some(dir) = &options.pack_dir {
        disabled.extend(
            packs::folder(dir, horizon_core::version::DATA_PACK_FORMAT)
                .into_iter()
                .map(|pack| pack.id)
                .filter(|id| !options.data_packs.contains(id)),
        );
        datapack::install(dir, &world.0, &options.data_packs)?;
    }
    let enabled_features = options.enabled_features(resources);
    let game_rules = options
        .rules
        .copy_with_features(&enabled_features)
        .by_name();
    let mut level = save::Level {
        name: name.to_owned(),
        seed,
        creative: creative && !options.hardcore,
        hardcore: options.hardcore,
        difficulty: horizon_client::title::DIFFICULTIES[options.difficulty().min(3)].into(),
        allow_commands: Some(options.commands(creative)),
        data_packs: Some((options.data_packs.clone(), disabled)),
        enabled_features,
        game_rules,
        ..save::Level::default()
    };
    save::write_world_gen_settings_with(
        &world.0,
        seed,
        options.world_gen(),
        options.flat_settings(),
        options.single_biome(),
    )
    .map_err(|_| "mco.create.world.failed".to_owned())?;
    level
        .write_without_player(&world.0)
        .map_err(|_| "mco.create.world.failed".to_owned())?;
    // Level's ordinary save path describes a live server. Vanilla's fresh
    // PrimaryLevelData has not initialized spawn and createTag(null) has no player.
    let file = world.0.join("level.dat");
    let mut tag = save::read_dat(&file).ok_or_else(|| "mco.create.world.failed".to_owned())?;
    let Some(Tag::Compound(data)) = tag.get_mut("Data") else {
        return Err("mco.create.world.failed".into());
    };
    data.insert("initialized".into(), Tag::Byte(0));
    data.remove(&"singleplayer_uuid".into());
    data.remove(&"Player".into());
    save::write_dat(&file, tag).map_err(|_| "mco.create.world.failed".to_owned())?;
    // write_dat preserves its previous value; never upload that initialized
    // intermediate metadata as a recovery copy of the uninitialized world.
    std::fs::remove_file(world.0.join("level.dat_old")).map_err(fail)?;
    Ok(world)
}
fn new_slot_options(slot: i32, name: &str, creative: bool, options: &WorldOptions) -> Value {
    // RealmsWorldOptions.createFromSettings + RealmsSlotUpdateDto.fromSlot.
    json!({"slotId":slot,"spawnProtection":0,"forceGameMode":false,
        "difficulty":options.difficulty(),"gameMode":i32::from(creative && !options.hardcore),
        "slotName":name,"version":horizon_core::version::VANILLA_VERSION,
        "compatibility":"UNVERIFIABLE","templateId":-1,"templateImage":null,"hardcore":options.hardcore})
}
