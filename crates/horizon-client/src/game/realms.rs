//! Shared asynchronous Realms controller for the startup and in-game title screens.
//! No credentials enter UI state. Browser/connection effects remain with the owner.
use horizon_client::{
    auth, lang,
    realms_service::{self, RealmsAuth, RealmsClient, RealmsError, RealmsResult},
    realms_transfer::{self, TransferControl},
    title::{
        Mode, Title,
        realms::{Page, RealmsAction},
    },
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

pub(super) enum Completion {
    /// The owner must confirm and enqueue a supplied resource pack before connecting.
    Join {
        realm_id: i64,
        address: String,
        resource_pack_url: Option<String>,
        resource_pack_hash: Option<String>,
    },
    /// Pass through the existing link confirmation flow, including terms links.
    OpenUri(String),
    /// 26.3 generates new worlds locally and uploads them; never send a legacy seed reset.
    CreateWorldForRealm { id: i64, slot: i32, options: Value },
}

enum Update {
    Servers {
        realms: Value,
        snapshots: Value,
        trial: bool,
    },
    Snapshots(Value),
    Invites(Value),
    Server(Value, Option<Page>),
    Tier {
        server: Value,
        tier: Value,
        regions: Value,
    },
    Distance(Value),
    Backups(Value),
    Subscription(Value),
    Templates(Value),
    Links(Value),
    DeletedCode(String),
    Notifications(Value),
    TermsAccepted,
    Done(&'static str, Option<Page>),
    Completion(Completion),
}
struct Pending {
    receiver: Receiver<RealmsResult<Update>>,
    action: RealmsAction,
    identity: Arc<auth::AuthSession>,
    page: Page,
    generation: u64,
    attempts: u8,
    retry_at: Option<Instant>,
    control: TransferControl,
}
struct Notices {
    receiver: Receiver<RealmsResult<(Value, Value)>>,
    identity: Arc<auth::AuthSession>,
}
#[derive(Default)]
pub(super) struct RealmsRuntime {
    pub connected_realm: Option<i64>,
    pending: Option<Pending>,
    generation: u64,
    notices: Option<Notices>,
    notices_due: Option<Instant>,
    invites: usize,
    notifications: Value,
}

pub(super) fn prepare_title(title: &mut Title) {
    let session = auth::current();
    title.realms_eligible = session
        .as_ref()
        .is_some_and(|s| !s.access_token().is_empty())
        && title.allows_multiplayer;
    title.realms_profile_uuid = session.map(|s| format!("{:032x}", s.uuid));
    let eligible = title.realms_eligible;
    let uuid = title.realms_profile_uuid.clone();
    if let Some(state) = title.realms_mut() {
        state.eligible = eligible;
        state.profile_uuid = uuid;
    }
}
impl RealmsRuntime {
    pub(super) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(pending) = self.pending.take() {
            pending.control.cancel.store(true, Ordering::Release);
        }
    }
    pub(super) fn start(&mut self, action: RealmsAction, title: &mut Title) {
        if matches!(action, RealmsAction::CancelRequest) {
            let key = match self.pending.as_ref().map(|pending| &pending.action) {
                Some(RealmsAction::Upload { .. }) => "mco.upload.cancelled",
                Some(RealmsAction::Download { .. }) => "mco.download.cancelled",
                _ => "gui.cancel",
            };
            self.cancel();
            if let Some(state) = title.realms_mut() {
                state.set_status(lang::get(key));
            }
            return;
        }
        self.cancel();
        // A background snapshot started before a mutation must not replace its result.
        self.notices = None;
        prepare_title(title);
        let Some(state) = title.realms_mut() else {
            return;
        };
        let Some(identity) = auth::current().filter(|_| state.eligible) else {
            state.set_error(lang::get("mco.error.invalid.session.message"));
            return;
        };
        state.set_loading(true);
        let page = state.page.clone();
        let control = TransferControl::default();
        let receiver = spawn(
            action.clone(),
            identity.clone(),
            title.saves.clone(),
            control.clone(),
        );
        self.pending = Some(Pending {
            receiver,
            action,
            identity,
            page,
            generation: self.generation,
            attempts: 0,
            retry_at: None,
            control,
        });
    }
    pub(super) fn poll(&mut self, title: &mut Title) -> Option<Completion> {
        let pending = self.pending.as_ref()?;
        let valid = pending.generation == self.generation
            && auth::current().is_some_and(|s| Arc::ptr_eq(&s, &pending.identity))
            && title.allows_multiplayer
            && matches!(&title.mode, Mode::Realms(s) if s.loading && s.page == pending.page && s.profile_uuid.as_deref() == Some(format!("{:032x}", pending.identity.uuid).as_str()));
        if !valid {
            self.cancel();
            return None;
        }
        if let Some(at) = pending.retry_at {
            if Instant::now() < at {
                return None;
            }
            let pending = self.pending.as_mut()?;
            pending.retry_at = None;
            pending.receiver = spawn(
                pending.action.clone(),
                auth::current()?,
                title.saves.clone(),
                pending.control.clone(),
            );
        }
        let result = match self.pending.as_ref()?.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                let pending = self.pending.as_ref()?;
                if matches!(
                    pending.action,
                    RealmsAction::Upload { .. } | RealmsAction::Download { .. }
                ) {
                    let stage = pending.control.stage.load(Ordering::Acquire);
                    let key = match stage {
                        1 => "mco.upload.preparing",
                        2 => "mco.upload.verifying",
                        3 => "mco.download.downloading",
                        4 | 5 => "mco.download.extracting",
                        _ => "mco.download.preparing",
                    };
                    let done = pending.control.bytes_done.load(Ordering::Relaxed);
                    let total = pending.control.bytes_total.load(Ordering::Relaxed);
                    if let Some(state) = title.realms_mut() {
                        let percent = if total > 0 {
                            format!(
                                " · {}%",
                                done.saturating_mul(100)
                                    .checked_div(total)
                                    .unwrap_or(0)
                                    .min(100)
                            )
                        } else {
                            String::new()
                        };
                        state.status = Some(format!("{}{percent}", lang::get(key)));
                    }
                }
                return None;
            }
            Err(TryRecvError::Disconnected) => Err(RealmsError::Transport),
        };
        if let Err(RealmsError::RetryAfter { seconds, .. }) = &result {
            let pending = self.pending.as_mut()?;
            if retryable(&pending.action) && pending.attempts < 3 {
                pending.attempts += 1;
                pending.retry_at =
                    Some(Instant::now() + Duration::from_secs((*seconds).clamp(1, 120)));
                if let Some(state) = title.realms_mut() {
                    state.status = Some(lang::get("mco.errorMessage.serviceBusy"));
                }
                return None;
            }
        }
        let completed_action = self.pending.take()?.action;
        let state = title.realms_mut()?;
        state.set_loading(false);
        match result {
            Err(RealmsError::TermsRequired) => state.require_terms(),
            Err(error) => {
                let message = if matches!(error, RealmsError::Transport) {
                    match completed_action {
                        RealmsAction::Upload { .. } => horizon_client::text::format_translation(
                            &lang::get("mco.upload.failed"),
                            &[horizon_client::text::Text::literal(lang::get(
                                "mco.errorMessage.connectionFailure",
                            ))],
                        )
                        .plain(),
                        RealmsAction::Download { .. } => lang::get("mco.download.failed"),
                        _ => error_text(&error),
                    }
                } else {
                    error_text(&error)
                };
                state.set_error(message);
            }
            Ok(Update::Servers {
                realms,
                snapshots,
                trial,
            }) => {
                state.return_page(Page::List);
                state.set_servers(realms);
                state.snapshot_realms = snapshots
                    .get("servers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                state.trial_available = trial;
            }
            Ok(Update::Snapshots(value)) => {
                state.snapshot_realms = value
                    .get("servers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
            }
            Ok(Update::Invites(value)) => {
                self.invites = value
                    .get("invites")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                self.notices_due = None;
                state.return_page(Page::Invites);
                state.set_invites(value);
            }
            Ok(Update::Server(value, page)) => {
                if let Some(page) = page {
                    state.return_page(page);
                }
                state.set_server(value);
            }
            Ok(Update::Tier {
                server,
                tier,
                regions,
            }) => {
                state.set_server(server);
                state.set_tier_configuration(tier);
                state.set_regions(regions);
            }
            Ok(Update::Distance(tier)) => state.set_tier_configuration(tier),
            Ok(Update::Backups(value)) => state.set_backups(value),
            Ok(Update::Subscription(value)) => state.set_subscription(value),
            Ok(Update::Templates(value)) => state.set_templates(value),
            Ok(Update::Links(value)) => state.set_invite_links(value),
            Ok(Update::DeletedCode(code)) => {
                let mut codes = state
                    .invite_links
                    .get("inviteCodes")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                codes
                    .retain(|item| item.get("code").and_then(Value::as_str) != Some(code.as_str()));
                state.set_invite_links(invite_links(json!(codes)));
            }
            Ok(Update::Notifications(value)) => {
                self.notifications = value.clone();
                state.set_notifications(value);
                self.notices_due = None;
            }
            Ok(Update::TermsAccepted) => state.terms_accepted(),
            Ok(Update::Done(key, page)) => {
                if let Some(page) = page {
                    state.return_page(page);
                }
                state.set_status(lang::get(key));
            }
            Ok(Update::Completion(completion)) => return Some(completion),
        }
        None
    }
    /// Call from either owner's title tick, with settings.realmsNotifications.
    /// Disabled/offline/restricted accounts never start background service requests.
    pub(super) fn poll_notifications(&mut self, title: &mut Title, enabled: bool) {
        prepare_title(title);
        if !enabled || !title.realms_eligible {
            self.notices = None;
            self.notices_due = None;
            self.invites = 0;
            self.notifications = Value::Null;
            title.realms_notification_count = 0;
            if let Some(state) = title.realms_mut() {
                state.set_notifications(Value::Null);
            }
            return;
        }
        if let Some(pending) = &self.notices {
            let valid = auth::current().is_some_and(|s| Arc::ptr_eq(&s, &pending.identity));
            let result = pending.receiver.try_recv();
            if !valid || !matches!(result, Err(TryRecvError::Empty)) {
                self.notices = None;
                if valid {
                    match result {
                        Ok(Ok((invites, notifications))) => {
                            self.invites = invites
                                .get("invites")
                                .and_then(Value::as_array)
                                .map_or(0, Vec::len);
                            self.notifications = notifications;
                        }
                        Ok(Err(RealmsError::RetryAfter { seconds, .. })) => {
                            self.notices_due =
                                Some(Instant::now() + Duration::from_secs(seconds.clamp(60, 120)));
                        }
                        _ => {}
                    }
                }
            }
        }
        title.realms_notification_count = self.notification_count();
        if let Some(state) = title.realms_mut() {
            state.set_notifications(self.notifications.clone());
        }
        if self.notices.is_some()
            || self.pending.is_some()
            || self.notices_due.is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        let Some(identity) = auth::current() else {
            return;
        };
        let session = identity.clone();
        let (send, receiver) = mpsc::channel();
        self.notices_due = Some(Instant::now() + Duration::from_secs(60));
        if std::thread::Builder::new()
            .name("Realms notifications".into())
            .spawn(move || {
                let auth = credentials(&session);
                let mut client = RealmsClient::new();
                let result = gate(&mut client, &auth).and_then(|()| {
                    Ok((
                        to_value(client.pending_invites(&auth)?)?,
                        client.notifications(&auth)?,
                    ))
                });
                let _ = send.send(result);
            })
            .is_ok()
        {
            self.notices = Some(Notices { receiver, identity });
        }
    }
    pub(super) fn notification_count(&self) -> usize {
        self.invites
            + self
                .notifications
                .as_array()
                .or_else(|| {
                    self.notifications
                        .get("notifications")
                        .and_then(Value::as_array)
                })
                .map_or(0, |items| {
                    items
                        .iter()
                        .filter(|item| item.get("seen").and_then(Value::as_bool) != Some(true))
                        .count()
                })
    }
}
impl Drop for RealmsRuntime {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn credentials(session: &auth::AuthSession) -> RealmsAuth {
    RealmsAuth {
        access_token: session.access_token().to_owned(),
        uuid: format!("{:032x}", session.uuid),
        name: session.name.clone(),
    }
}
pub(super) fn gate(client: &mut RealmsClient, auth: &RealmsAuth) -> RealmsResult<()> {
    client.refresh_feature_flags(auth)?;
    if client.client_compatible(auth)? != "COMPATIBLE" {
        return Err(RealmsError::Service {
            status: 200,
            code: Some(6001),
        });
    }
    if !client.has_parental_consent(auth)? {
        return Err(RealmsError::Forbidden);
    }
    Ok(())
}
fn spawn(
    action: RealmsAction,
    identity: Arc<auth::AuthSession>,
    saves: PathBuf,
    control: TransferControl,
) -> Receiver<RealmsResult<Update>> {
    let (send, receiver) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("Realms request".into())
        .spawn(move || {
            let result = execute(action, &identity, &saves, &control);
            let _ = send.send(result);
        });
    receiver
}
fn active(control: &TransferControl) -> RealmsResult<()> {
    if control.cancel.load(Ordering::Acquire) {
        Err(RealmsError::Transport)
    } else {
        Ok(())
    }
}
fn to_value(value: impl serde::Serialize) -> RealmsResult<Value> {
    serde_json::to_value(value).map_err(|_| RealmsError::InvalidResponse)
}
fn success(value: bool) -> RealmsResult<()> {
    if value {
        Ok(())
    } else {
        Err(RealmsError::Service {
            status: 200,
            code: None,
        })
    }
}
fn execute(
    action: RealmsAction,
    identity: &auth::AuthSession,
    saves: &std::path::Path,
    control: &TransferControl,
) -> RealmsResult<Update> {
    use RealmsAction::*;
    active(control)?;
    if matches!(action, SnapshotRealms | CreateSnapshotRealm(_)) && !realms_service::is_snapshot() {
        return Err(RealmsError::InvalidInput);
    }
    // These links are CommonLinks constants verified in realms-service/dto-request-links.javap.txt.
    if matches!(action, OpenTerms) {
        return Ok(Update::Completion(Completion::OpenUri(
            realms_service::REALMS_TERMS.into(),
        )));
    }
    let auth = credentials(identity);
    let mut client = RealmsClient::new();
    gate(&mut client, &auth)?;
    active(control)?;
    let servers = || -> RealmsResult<Update> {
        let realms = to_value(client.list_realms(&auth)?)?;
        active(control)?;
        let snapshots = if realms_service::is_snapshot() {
            to_value(client.snapshot_eligible_realms(&auth)?)?
        } else {
            Value::Null
        };
        active(control)?;
        // Trial discovery must not hide an otherwise usable Realm list on failure.
        let trial = client.trial_available(&auth).unwrap_or(false);
        Ok(Update::Servers {
            realms,
            snapshots,
            trial,
        })
    };
    let server = |id, page| -> RealmsResult<Update> {
        Ok(Update::Server(
            to_value(client.get_own_realm(&auth, id)?)?,
            page,
        ))
    };
    let invites = || -> RealmsResult<Update> {
        Ok(Update::Invites(to_value(client.pending_invites(&auth)?)?))
    };
    match action {
        ConfirmJoinPack(_) => Err(RealmsError::InvalidInput),
        Refresh => servers(),
        SnapshotRealms => Ok(Update::Snapshots(to_value(
            client.snapshot_eligible_realms(&auth)?,
        )?)),
        CreateSnapshotRealm(parent) => {
            // Recheck service eligibility after the confirmation, before creating anything.
            if !client
                .snapshot_eligible_realms(&auth)?
                .servers
                .iter()
                .any(|s| s.id == parent)
            {
                return Err(RealmsError::InvalidInput);
            }
            active(control)?;
            let realm = client.create_snapshot_realm(&auth, parent)?;
            Ok(Update::Server(to_value(realm)?, Some(Page::Configure)))
        }
        DeleteRealm(id) => {
            let realm = client.get_own_realm(&auth, id)?;
            if !realm.expired || !owned_realm(&realm, &auth) {
                return Err(RealmsError::InvalidInput);
            }
            active(control)?;
            client.delete_realm(&auth, id)?;
            servers()
        }
        Join(id) => {
            let info = client.join(&auth, id)?;
            if info.address.is_empty() || info.address.chars().any(char::is_whitespace) {
                return Err(RealmsError::InvalidResponse);
            }
            let url = info.resource_pack_url.filter(|s| !s.is_empty());
            let hash = info.resource_pack_hash.filter(|s| !s.is_empty());
            if url.is_some() != hash.is_some()
                || hash
                    .as_ref()
                    .is_some_and(|h| h.len() != 40 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(RealmsError::InvalidResponse);
            }
            Ok(Update::Completion(Completion::Join {
                realm_id: id,
                address: info.address,
                resource_pack_url: url,
                resource_pack_hash: hash,
            }))
        }
        Invites => invites(),
        AcceptInvite(id) => {
            client.accept_invitation(&auth, &id)?;
            invites()
        }
        RejectInvite(id) => {
            client.reject_invitation(&auth, &id)?;
            invites()
        }
        Configure(id) | Slots(id) | Players(id) => server(id, None),
        Initialize {
            id,
            name,
            description,
        } => {
            client.initialize(&auth, id, &name, &description)?;
            server(id, Some(Page::Configure))
        }
        TierConfiguration(id) => {
            let realm = to_value(client.get_own_realm(&auth, id)?)?;
            active(control)?;
            let tier = client.tier_configuration(&auth, id)?;
            active(control)?;
            let regions = client.preferred_regions(&auth)?;
            Ok(Update::Tier {
                server: realm,
                tier,
                regions,
            })
        }
        SetRenderDistance { id, distance } => {
            validate_distance(
                &client.tier_configuration(&auth, id)?,
                "renderDistance",
                distance,
            )?;
            active(control)?;
            client.update_render_distance(&auth, id, distance)?;
            Ok(Update::Distance(client.tier_configuration(&auth, id)?))
        }
        SetSimulationDistance { id, distance } => {
            validate_distance(
                &client.tier_configuration(&auth, id)?,
                "simDistance",
                distance,
            )?;
            active(control)?;
            client.update_simulation_distance(&auth, id, distance)?;
            Ok(Update::Distance(client.tier_configuration(&auth, id)?))
        }
        SetRegion { id, preference } => {
            validate_region(&preference)?;
            let realm = client.get_own_realm(&auth, id)?;
            let mut configuration = configuration(&realm)?;
            let mut region = configuration
                .get("regionSelectionPreference")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            for key in ["regionSelectionPreference", "preferredRegion"] {
                region[key] = preference.get(key).cloned().unwrap_or(Value::Null);
            }
            configuration["regionSelectionPreference"] = region;
            active(control)?;
            client.update_configuration(&auth, id, &configuration)?;
            server(id, None)
        }
        CancelRequest => unreachable!("cancellation is handled before spawning a worker"),
        SaveConfig {
            id,
            name,
            description,
        } => {
            let realm = client.get_own_realm(&auth, id)?;
            active(control)?;
            if realm.state.as_deref() == Some("UNINITIALIZED") {
                client.initialize(&auth, id, &name, &description)?;
            } else {
                let mut configuration = configuration(&realm)?;
                configuration["description"]["name"] = json!(name);
                configuration["description"]["description"] = json!(description);
                client.update_configuration(&auth, id, &configuration)?;
            }
            server(id, Some(Page::Configure))
        }
        SaveSlot { id, slot, options } => {
            let realm = client.get_own_realm(&auth, id)?;
            let (mut merged, _settings) = slot_options(&realm.slots, slot)?;
            let changes = options.as_object().ok_or(RealmsError::InvalidInput)?;
            let object = merged.as_object_mut().ok_or(RealmsError::InvalidResponse)?;
            // Only editable fields replace the fresh server metadata; retain unknown DTO fields.
            for key in [
                "spawnProtection",
                "forceGameMode",
                "difficulty",
                "gameMode",
                "slotName",
            ] {
                if let Some(value) = changes.get(key) {
                    object.insert(key.into(), value.clone());
                }
            }
            active(control)?;
            client.update_slot(&auth, id, slot, &merged)?;
            server(id, Some(Page::Slots))
        }
        SwitchSlot { id, slot } => {
            success(client.switch_slot(&auth, id, slot)?)?;
            server(id, Some(Page::Slots))
        }
        Backups { id, .. } => Ok(Update::Backups(client.backups_for(&auth, id)?)),
        RestoreBackup { id, backup_id } => {
            client.restore_world(&auth, id, &backup_id)?;
            server(id, Some(Page::Slots))
        }
        Upload { id, slot, folder } => {
            realms_transfer::upload_world_with_control(&client, &auth, id, slot, &folder, control)
                .map_err(|_| RealmsError::Transport)?;
            Ok(Update::Done("mco.upload.done", Some(Page::Slots)))
        }
        Download { id, slot } => {
            let realm = client.get_own_realm(&auth, id)?;
            active(control)?;
            let name = realm
                .name
                .unwrap_or_else(|| lang::get("selectWorld.newWorld"));
            realms_transfer::download_world_with_control(
                &client, &auth, id, slot, saves, &name, control,
            )
            .map_err(|_| RealmsError::Transport)?;
            Ok(Update::Done("mco.download.done", None))
        }
        Subscription(id) => Ok(Update::Subscription(client.subscription_for(&auth, id)?)),
        OpenSubscription(id) | RenewSubscription(id) => {
            let renewal = matches!(action, RenewSubscription(_));
            let realm = client.get_own_realm(&auth, id)?;
            if !owned_realm(&realm, &auth) {
                return Err(RealmsError::InvalidInput);
            }
            let mut uri = match realm.remote_subscription_id.as_deref() {
                Some(subscription) => realms_service::extend_subscription_link(
                    subscription,
                    &auth.uuid,
                    realm.expired_trial,
                )?,
                None => "https://aka.ms/ExtendJavaRealms".into(),
            };
            // Subscription tab always uses NONE; main-list renewal uses its expiry referral.
            if !renewal && let Some((base, _)) = uri.rsplit_once("&ref=") {
                uri = base.to_owned();
            }
            Ok(Update::Completion(Completion::OpenUri(uri)))
        }
        Purchase => Ok(Update::Completion(Completion::OpenUri(
            if client.trial_available(&auth)? {
                realms_service::START_TRIAL
            } else {
                realms_service::BUY_REALMS
            }
            .into(),
        ))),
        OpenTerms => unreachable!(),
        AcceptTerms => {
            client.agree_to_tos(&auth)?;
            Ok(Update::TermsAccepted)
        }
        OpenWorld(id) => {
            success(client.open(&auth, id)?)?;
            server(id, Some(Page::Configure))
        }
        CloseWorld(id) => {
            success(client.close(&auth, id)?)?;
            server(id, Some(Page::Configure))
        }
        Leave(id) => {
            client.leave(&auth, id)?;
            servers()
        }
        InvitePlayer { id, name } => {
            client.invite(&auth, id, &name)?;
            server(id, Some(Page::Players))
        }
        RemovePlayer { id, uuid } => {
            client.uninvite(&auth, id, &uuid)?;
            server(id, Some(Page::Players))
        }
        SetOperator { id, uuid, operator } => {
            if operator {
                client.op(&auth, id, &uuid)?;
            } else {
                client.deop(&auth, id, &uuid)?;
            }
            server(id, Some(Page::Players))
        }
        Reset { id, options } => {
            if let Some(template) = options
                .get("worldTemplateId")
                .and_then(Value::as_i64)
                .filter(|id| *id >= 0)
            {
                success(client.reset_world_with_template(&auth, id, template)?)?;
                server(id, Some(Page::Slots))
            } else {
                let realm = client.get_own_realm(&auth, id)?;
                Ok(Update::Completion(Completion::CreateWorldForRealm {
                    id,
                    slot: realm.active_slot,
                    options,
                }))
            }
        }
        Templates { world_type, page } => Ok(Update::Templates(client.templates(
            &auth,
            u32::try_from(page).map_err(|_| RealmsError::InvalidInput)?,
            10,
            &world_type,
        )?)),
        SwitchMinigame { id, template_id } => {
            success(client.minigame(
                &auth,
                id,
                template_id.parse().map_err(|_| RealmsError::InvalidInput)?,
            )?)?;
            server(id, Some(Page::Configure))
        }
        InviteLink(id) => Ok(Update::Links(invite_links(client.invite_codes(&auth, id)?))),
        CreateInviteCode { id } => {
            client.create_invite_code(&auth, id, true, None)?;
            Ok(Update::Links(invite_links(client.invite_codes(&auth, id)?)))
        }
        UpdateInviteCode {
            id,
            code,
            enabled,
            expiration_ms,
        } => {
            client.update_invite_code(&auth, id, &code, enabled, expiration_ms)?;
            Ok(Update::Links(invite_links(client.invite_codes(&auth, id)?)))
        }
        DeleteInviteCode { code } => {
            client.delete_invite_code(&auth, &code)?;
            Ok(Update::DeletedCode(code))
        }
        RedeemInvite(code) => {
            client.redeem_invite_code(&auth, &code)?;
            servers()
        }
        MarkNotificationsSeen(ids) => {
            client.notifications_seen(&auth, &ids)?;
            Ok(Update::Notifications(client.notifications(&auth)?))
        }
        DismissNotification(id) => {
            client.notifications_dismiss(&auth, &[id])?;
            Ok(Update::Notifications(client.notifications(&auth)?))
        }
    }
}
fn owned_realm(realm: &realms_service::RealmsServer, auth: &RealmsAuth) -> bool {
    realm.owner_uuid.as_deref().is_some_and(|owner| {
        !owner.is_empty()
            && owner
                .replace('-', "")
                .eq_ignore_ascii_case(&auth.uuid.replace('-', ""))
    })
}
fn validate_distance(tier: &Value, key: &str, distance: u32) -> RealmsResult<()> {
    let range = tier.get(key).ok_or(RealmsError::InvalidResponse)?;
    let min = range
        .get("min")
        .and_then(Value::as_u64)
        .ok_or(RealmsError::InvalidResponse)?;
    let max = range
        .get("max")
        .and_then(Value::as_u64)
        .ok_or(RealmsError::InvalidResponse)?;
    if min > max {
        return Err(RealmsError::InvalidResponse);
    }
    if (min..=max).contains(&u64::from(distance)) {
        Ok(())
    } else {
        Err(RealmsError::InvalidInput)
    }
}
fn validate_region(preference: &Value) -> RealmsResult<()> {
    match preference
        .get("regionSelectionPreference")
        .and_then(Value::as_u64)
    {
        Some(0 | 1)
            if preference
                .get("preferredRegion")
                .is_none_or(|v| v.is_null() || v.is_string()) =>
        {
            Ok(())
        }
        Some(2)
            if preference
                .get("preferredRegion")
                .and_then(Value::as_str)
                .is_some_and(|region| !region.is_empty()) =>
        {
            Ok(())
        }
        _ => Err(RealmsError::InvalidInput),
    }
}
fn configuration(realm: &realms_service::RealmsServer) -> RealmsResult<Value> {
    let mut value = realm
        .extra
        .get("configuration")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let (options, settings) = slot_options(&realm.slots, realm.active_slot)?;
    value["options"] = options;
    value["settings"] = settings;
    if let Some(region) = realm.extra.get("regionSelectionPreference") {
        value["regionSelectionPreference"] = region.clone();
    }
    if !value.get("description").is_some_and(Value::is_object) {
        value["description"] = json!({});
    }
    value["description"]["name"] = json!(realm.name);
    value["description"]["description"] = json!(realm.motd);
    Ok(value)
}
fn slot_options(slots: &Value, slot: i32) -> RealmsResult<(Value, Value)> {
    let entry = slots
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|v| v.get("slotId").and_then(Value::as_i64) == Some(i64::from(slot)))
        })
        .ok_or(RealmsError::InvalidResponse)?;
    let mut options = match entry.get("options") {
        Some(Value::String(s)) => {
            serde_json::from_str(s).map_err(|_| RealmsError::InvalidResponse)?
        }
        Some(v) if v.is_object() => v.clone(),
        _ => return Err(RealmsError::InvalidResponse),
    };
    if !options.is_object() {
        return Err(RealmsError::InvalidResponse);
    }
    let settings = entry
        .get("settings")
        .filter(|v| v.is_array())
        .cloned()
        .ok_or(RealmsError::InvalidResponse)?;
    options["slotId"] = json!(slot);
    options["hardcore"] = json!(
        settings
            .as_array()
            .and_then(|items| items
                .iter()
                .find(|v| v.get("name").and_then(Value::as_str) == Some("hardcore")))
            .and_then(|setting| setting.get("value").and_then(Value::as_str))
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    );
    // RealmsSlotUpdateDto is not RealmsWorldOptions: only these fields go on the wire.
    // Keep the settings array alongside it for RealmsConfigurationDto, never inside options.
    options
        .as_object_mut()
        .ok_or(RealmsError::InvalidResponse)?
        .retain(|key, _| {
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
    Ok((options, settings))
}
fn invite_links(value: Value) -> Value {
    // InviteCodeList.getInviteCodesArray accepts both the bare array and `result`.
    let codes = value
        .as_array()
        .or_else(|| value.get("result").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();
    let token = codes
        .iter()
        .find(|code| {
            code.get("enabled").and_then(Value::as_bool) == Some(true)
                && code
                    .get("expirationDate")
                    .and_then(Value::as_i64)
                    .is_none_or(|date| date > auth::now_millis())
        })
        .and_then(|code| code.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    json!({"inviteCodes": codes, "token": token})
}
fn retryable(action: &RealmsAction) -> bool {
    matches!(
        action,
        RealmsAction::Refresh
            | RealmsAction::SnapshotRealms
            | RealmsAction::Join(_)
            | RealmsAction::Invites
            | RealmsAction::Configure(_)
            | RealmsAction::Slots(_)
            | RealmsAction::Players(_)
            | RealmsAction::Backups { .. }
            | RealmsAction::Subscription(_)
            | RealmsAction::TierConfiguration(_)
            | RealmsAction::Templates { .. }
            | RealmsAction::InviteLink(_)
    )
}
fn error_text(error: &RealmsError) -> String {
    let key = match error {
        RealmsError::Unauthorized => "mco.error.invalid.session.message".to_owned(),
        RealmsError::Forbidden => "mco.account.privacy.information".to_owned(),
        RealmsError::RetryAfter { .. } => "mco.errorMessage.serviceBusy".to_owned(),
        RealmsError::WorldNotClosed => "mco.upload.close.failure".to_owned(),
        RealmsError::Service {
            code: Some(code @ 6001..=6009),
            ..
        } => format!("mco.errorMessage.{code}"),
        _ => "mco.errorMessage.connectionFailure".to_owned(),
    };
    lang::get(&key)
}
