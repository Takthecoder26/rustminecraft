//! Shared asynchronous Realms controller for the startup and in-game title screens.
//! No credentials enter UI state. Browser/connection effects remain with the owner.
use horizon_client::{
    auth, lang,
    realms_service::{self, RealmsAuth, RealmsClient, RealmsError, RealmsResult},
    realms_transfer::{self, TransferControl},
    title::{
        Mode, Title,
        realms::{AvailabilityErrorKind, Page, RealmsAction},
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
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
    },
    RefreshServers,
    Snapshots(Value),
    Invites(Value),
    Server(Value, Option<Page>),
    Configuration {
        server: Value,
        subscription: RealmsResult<Value>,
        tier: RealmsResult<Value>,
        regions: RealmsResult<Value>,
    },
    SettingsSaved {
        server: Value,
        render_distance: Option<u32>,
        simulation_distance: Option<u32>,
        continuation: Option<Box<RealmsAction>>,
    },
    SnapshotCreated {
        realm: Value,
        error: Option<RealmsError>,
    },
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
    CodeRefreshFailed {
        id: i64,
        error: RealmsError,
    },
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
    state_generation: u64,
    attempts: u8,
    retry_at: Option<Instant>,
    control: TransferControl,
}
struct Notices {
    receiver: Receiver<RealmsResult<(Value, Value)>>,
    identity: Arc<auth::AuthSession>,
}
/// DataFetcher keeps the last successful value in the UI; failures only change
/// the next update time. Each task owns its receiver, so slow trial/news requests
/// cannot hold up the server list.
struct FetchTask<T> {
    receiver: Option<Receiver<(RealmsResult<T>, Instant)>>,
    due: Option<Instant>,
    failures: u32,
}
impl<T: Send + 'static> Default for FetchTask<T> {
    fn default() -> Self {
        Self {
            receiver: None,
            due: None,
            failures: 0,
        }
    }
}
impl<T: Send + 'static> FetchTask<T> {
    fn poll(&mut self, period: u64, backoff: u64, stage: &'static str) -> Option<T> {
        let (result, completed) = match self.receiver.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => (Err(RealmsError::Transport), Instant::now()),
        };
        self.receiver = None;
        let cycles = if result.is_ok() {
            self.failures = 0;
            1
        } else {
            self.failures = self.failures.saturating_add(1);
            (1u64 << self.failures.min(63)).min(backoff)
        };
        self.due = Some(completed + Duration::from_secs(period * cycles));
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                log::warn!("Realms {stage}: {error}");
                None
            }
        }
    }
    fn start(&mut self, fetch: impl FnOnce() -> RealmsResult<T> + Send + 'static) {
        if self.receiver.is_some() || self.due.is_some_and(|due| Instant::now() < due) {
            return;
        }
        let (send, receiver) = mpsc::channel();
        self.receiver = Some(receiver);
        let _ = std::thread::Builder::new()
            .name("Realms data fetch".into())
            .spawn(move || {
                let result = fetch();
                let _ = send.send((result, Instant::now()));
            });
    }
}
struct AvailabilityFailure {
    kind: AvailabilityErrorKind,
    error: RealmsError,
}
struct Subscription {
    identity: Arc<auth::AuthSession>,
    generation: u64,
    availability: Option<Receiver<Result<RealmsClient, AvailabilityFailure>>>,
    client: Option<RealmsClient>,
    servers: FetchTask<(Value, Value)>,
    trial: FetchTask<bool>,
    invites: FetchTask<usize>,
    news: FetchTask<Value>,
    online_players: FetchTask<Value>,
    notifications: FetchTask<Value>,
    seen: FetchTask<Vec<String>>,
    handled_seen: HashSet<String>,
}
#[derive(Default)]
pub(super) struct RealmsRuntime {
    pub connected_realm: Option<i64>,
    pending: Option<Pending>,
    subscription: Option<Subscription>,
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
        self.subscription = None;
        self.cancel_pending();
    }
    fn cancel_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(pending) = self.pending.take() {
            pending.control.cancel.store(true, Ordering::Release);
        }
    }
    pub(super) fn start(&mut self, action: RealmsAction, title: &mut Title) {
        if matches!(action, RealmsAction::CancelRequest) {
            // Creation is not atomic: keep its result so an already-created child
            // remains accessible, but stop before the next service request.
            if let Some(pending) = self.pending.as_ref()
                && matches!(pending.action, RealmsAction::CreateSnapshotRealm { .. })
            {
                pending.control.cancel.store(true, Ordering::Release);
                if let Some(state) = title.realms_mut() {
                    state.set_loading(true);
                    state.status = Some(lang::get("gui.cancel"));
                }
                return;
            }
            let key = match self.pending.as_ref().map(|pending| &pending.action) {
                Some(RealmsAction::Upload { .. }) => "mco.upload.cancelled",
                Some(RealmsAction::Download { .. }) => "mco.download.cancelled",
                _ => "gui.cancel",
            };
            self.cancel_pending();
            if let Some(state) = title.realms_mut() {
                if state.page == Page::Upload {
                    state.reset_upload_cancelled();
                } else {
                    state.set_status(lang::get(key));
                }
            }
            return;
        }
        self.cancel_pending();
        if matches!(action, RealmsAction::Refresh) {
            self.refresh(title);
            return;
        }
        // A background snapshot started before a mutation must not replace its result.
        if let Some(subscription) = &mut self.subscription {
            subscription.servers = FetchTask::default();
            subscription.invites = FetchTask::default();
            subscription.notifications = FetchTask::default();
        }
        self.notices = None;
        prepare_title(title);
        let Some(state) = title.realms_mut() else {
            return;
        };
        let Some(identity) = auth::current().filter(|_| state.eligible) else {
            let message = lang::get("mco.error.invalid.session.message");
            if matches!(action, RealmsAction::Upload { .. }) {
                state.reset_upload_finished(Some(message));
            } else if matches!(action, RealmsAction::Templates { .. })
                && state.reset_templates_failed()
            {
                log::warn!("Realms templates: invalid session");
            } else if !state.player_request_failed(&action, message.clone())
                && !state.world_request_failed(&action, &message)
            {
                state.set_error(message);
            }
            return;
        };
        state.set_loading(true);
        let page = state.page.clone();
        let state_generation = state.request_generation;
        let control = TransferControl::default();
        let receiver = spawn(
            action.clone(),
            identity.clone(),
            title.saves.clone(),
            control.clone(),
            self.subscription
                .as_ref()
                .filter(|subscription| {
                    subscription.generation == state_generation
                        && Arc::ptr_eq(&subscription.identity, &identity)
                })
                .and_then(|subscription| subscription.client.clone()),
        );
        self.pending = Some(Pending {
            receiver,
            action,
            identity,
            page,
            generation: self.generation,
            state_generation,
            attempts: 0,
            retry_at: None,
            control,
        });
    }
    fn refresh(&mut self, title: &mut Title) {
        prepare_title(title);
        let Some(state) = title.realms_mut() else {
            return;
        };
        let Some(identity) = auth::current().filter(|_| state.eligible) else {
            self.subscription = None;
            state.set_availability_error_kind(
                AvailabilityErrorKind::Unauthorized,
                lang::get("mco.error.invalid.session.message"),
            );
            return;
        };
        if let Some(subscription) = &mut self.subscription
            && subscription.generation == state.request_generation
            && Arc::ptr_eq(&subscription.identity, &identity)
        {
            subscription.servers = FetchTask::default();
            state.set_loading(!state.servers_loaded);
            return;
        }
        state.set_loading(!state.servers_loaded);
        let session = identity.clone();
        let (send, receiver) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("Realms availability".into())
            .spawn(move || {
                let auth = credentials(&session);
                let mut client = RealmsClient::new();
                let result = availability(&mut client, &auth).map(|()| client);
                let _ = send.send(result);
            });
        self.subscription = Some(Subscription {
            identity,
            generation: state.request_generation,
            availability: Some(receiver),
            client: None,
            servers: FetchTask::default(),
            trial: FetchTask::default(),
            invites: FetchTask::default(),
            news: FetchTask::default(),
            online_players: FetchTask::default(),
            notifications: FetchTask::default(),
            seen: FetchTask::default(),
            handled_seen: HashSet::new(),
        });
    }
    fn poll_subscription(&mut self, title: &mut Title) {
        let Some(subscription) = &self.subscription else {
            return;
        };
        let valid = title.allows_multiplayer
            && auth::current()
                .is_some_and(|identity| Arc::ptr_eq(&identity, &subscription.identity))
            && matches!(&title.mode, Mode::Realms(state)
                if state.request_generation == subscription.generation
                    && state.page != Page::AvailabilityError);
        if !valid {
            self.subscription = None;
            return;
        }
        let subscription = self.subscription.as_mut().unwrap();
        let state = title.realms_mut().unwrap();
        if let Some(receiver) = &subscription.availability {
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(AvailabilityFailure {
                    kind: AvailabilityErrorKind::Service,
                    error: RealmsError::Transport,
                }),
            };
            subscription.availability = None;
            match result {
                Ok(client) => subscription.client = Some(client),
                Err(failure) => {
                    state.set_availability_error_kind(failure.kind, error_text(&failure.error));
                    self.subscription = None;
                    return;
                }
            }
        }
        let Some(client) = &subscription.client else {
            return;
        };
        // Main and PDP subscribe to trial independently of list readiness. A
        // background callback must never finish a foreground mutation's loading.
        if matches!(state.page, Page::List | Page::Pdp) {
            if let Some(trial) = subscription.trial.poll(60, 60, "trial") {
                state.set_trial_available(trial);
            }
            let client = client.clone();
            let identity = subscription.identity.clone();
            subscription
                .trial
                .start(move || client.trial_available(&credentials(&identity)));
        }
        if state.polls_server_list()
            && self.pending.is_none()
            && (!state.loading || !state.servers_loaded)
        {
            if let Some((realms, snapshots)) = subscription.servers.poll(60, 1, "server list") {
                state.snapshot_realms = snapshots
                    .get("servers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                state.set_servers(realms);
            }
            let client = client.clone();
            let identity = subscription.identity.clone();
            subscription
                .servers
                .start(move || fetch_servers(&client, &credentials(&identity)));
        }
        if matches!(state.page, Page::List | Page::Pdp) {
            if let Some(count) = subscription.invites.poll(10, 360, "pending invites") {
                self.invites = count;
                state.set_pending_invite_count(count);
            }
            let invite_client = client.clone();
            let identity = subscription.identity.clone();
            subscription.invites.start(move || {
                Ok(invite_client
                    .pending_invites(&credentials(&identity))?
                    .invites
                    .len())
            });
            if let Some(news) = subscription.news.poll(300, 1, "news") {
                state.update_news(news.get("newsLink").and_then(Value::as_str));
            }
            let client = client.clone();
            let identity = subscription.identity.clone();
            subscription
                .news
                .start(move || client.news(&credentials(&identity)));
            if let Some(notifications) = subscription.notifications.poll(300, 1, "notifications") {
                self.notifications = notifications.clone();
                state.set_notifications(notifications);
            }
            let client = subscription.client.as_ref().unwrap().clone();
            let identity = subscription.identity.clone();
            subscription
                .notifications
                .start(move || client.notifications(&credentials(&identity)));
        }
        if state.page == Page::List {
            if let Some(players) = subscription.online_players.poll(10, 1, "online players") {
                // RealmsServerPlayerLists uses `lists` containing a JSON-encoded
                // `playerList`; State owns decoding and filtering the local profile.
                state.set_online_players(players);
            }
            let client = client.clone();
            let identity = subscription.identity.clone();
            subscription
                .online_players
                .start(move || client.live_stats(&credentials(&identity)));
        }
        // A seen acknowledgement has no foreground action or loading state. The
        // subscription's identity/generation fence also owns this receiver.
        if let Some(ids) = subscription.seen.poll(10, 1, "notifications seen") {
            subscription.handled_seen.extend(ids);
            subscription.seen.due = None;
        }
        let ids: Vec<_> = state
            .visible_unseen_notification_ids()
            .into_iter()
            .filter(|id| !subscription.handled_seen.contains(id))
            .collect();
        if !ids.is_empty() {
            let client = client.clone();
            let identity = subscription.identity.clone();
            subscription.seen.start(move || {
                client.notifications_seen(&credentials(&identity), &ids)?;
                Ok(ids)
            });
        }
    }
    pub(super) fn poll(&mut self, title: &mut Title) -> Option<Completion> {
        self.poll_subscription(title);
        let pending = self.pending.as_ref()?;
        let valid = pending.generation == self.generation
            && auth::current().is_some_and(|s| Arc::ptr_eq(&s, &pending.identity))
            && title.allows_multiplayer
            && matches!(&title.mode, Mode::Realms(s) if s.loading && s.page == pending.page && s.request_generation == pending.state_generation && s.profile_uuid.as_deref() == Some(format!("{:032x}", pending.identity.uuid).as_str()));
        if !valid {
            self.cancel_pending();
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
                self.subscription
                    .as_ref()
                    .and_then(|subscription| subscription.client.clone()),
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
                    let done = pending.control.bytes_done.load(Ordering::Relaxed);
                    let total = pending.control.bytes_total.load(Ordering::Relaxed);
                    if let Some(state) = title.realms_mut() {
                        if matches!(pending.action, RealmsAction::Upload { .. }) {
                            state.reset_upload_progress(done, total, stage == 2);
                            return None;
                        }
                        let key = match stage {
                            3 => "mco.download.downloading",
                            4 | 5 => "mco.download.extracting",
                            _ => "mco.download.preparing",
                        };
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
        if result.is_ok() {
            state.world_operation_finished();
        }
        match result {
            Err(error) if matches!(completed_action, RealmsAction::Configure(_)) => {
                state.set_configuration_error(error_text(&error));
            }
            Err(error) if matches!(completed_action, RealmsAction::SaveSettings { .. }) => {
                state.set_error(error_text(&error));
            }
            Err(error) => {
                let message = if matches!(error, RealmsError::Transport)
                    && matches!(completed_action, RealmsAction::Download { .. })
                {
                    lang::get("mco.download.failed")
                } else {
                    error_text(&error)
                };
                if matches!(completed_action, RealmsAction::Upload { .. }) {
                    state.reset_upload_finished(Some(message));
                } else if matches!(completed_action, RealmsAction::Templates { .. })
                    && state.reset_templates_failed()
                {
                    log::warn!("Realms templates: {error}");
                } else if !state.player_request_failed(&completed_action, message.clone())
                    && !state.world_request_failed(&completed_action, &message)
                {
                    if matches!(error, RealmsError::TermsRequired) {
                        state.require_terms();
                    } else {
                        state.set_error(message);
                    }
                }
            }
            Ok(Update::Servers { realms, snapshots }) => {
                state.return_page(Page::List);
                state.snapshot_realms = snapshots
                    .get("servers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                state.set_servers(realms);
                if let Some(subscription) = &mut self.subscription {
                    subscription.servers.due = Some(Instant::now() + Duration::from_secs(60));
                }
            }
            Ok(Update::RefreshServers) => {
                state.return_page(Page::List);
                state.selected = None;
                self.refresh(title);
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
            Ok(Update::Configuration {
                server,
                subscription,
                tier,
                regions,
            }) => {
                state.set_server(server);
                let mut failure = None;
                match subscription {
                    Ok(value) => state.set_subscription(value),
                    Err(error) => failure = Some(error),
                }
                match tier {
                    Ok(value) => state.set_tier_configuration(value),
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
                match regions {
                    Ok(value) => state.set_regions(value),
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
                if let Some(error) = failure {
                    state.set_configuration_error(error_text(&error));
                }
            }
            Ok(Update::SettingsSaved {
                server,
                render_distance,
                simulation_distance,
                continuation,
            }) => {
                state.set_server(server);
                for (key, distance) in [
                    ("renderDistance", render_distance),
                    ("simDistance", simulation_distance),
                ] {
                    if let Some(distance) = distance
                        && let Some(range) = state
                            .tier_configuration
                            .get_mut(key)
                            .and_then(Value::as_object_mut)
                    {
                        range.insert("current".into(), json!(distance));
                    }
                }
                state.settings_saved();
                if let Some(action) = continuation {
                    self.start(*action, title);
                }
            }
            Ok(Update::SnapshotCreated { realm, error }) => {
                let id = realm.get("id").and_then(Value::as_i64);
                state
                    .servers
                    .retain(|s| s.get("id").and_then(Value::as_i64) != id);
                state.servers.push(realm.clone());
                state.snapshot_realms.retain(|s| {
                    !matches!(&completed_action, RealmsAction::CreateSnapshotRealm { parent, .. }
                        if s.get("id").and_then(Value::as_i64) == Some(*parent))
                });
                state.return_page(Page::Configure);
                state.set_server(realm);
                if let Some(error) = error {
                    state.set_error(format!(
                        "{}\n{}",
                        lang::get("mco.errorMessage.initialize.failed"),
                        error_text(&error)
                    ));
                }
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
            Ok(Update::Templates(value)) => {
                if let Some(action) = state.reset_set_templates(value) {
                    self.start(action, title);
                }
            }
            Ok(Update::Links(value)) => state.player_links_loaded(value),
            Ok(Update::CodeRefreshFailed { id, error }) => {
                state.return_page(Page::InviteLink);
                state.player_request_failed(&RealmsAction::InviteLink(id), error_text(&error));
            }
            Ok(Update::Notifications(value)) => {
                self.notifications = value.clone();
                state.set_notifications(value);
                self.notices_due = None;
            }
            Ok(Update::TermsAccepted) => state.terms_accepted(),
            Ok(Update::Done(key, page)) => {
                if matches!(completed_action, RealmsAction::Upload { .. }) {
                    state.reset_upload_finished(None);
                } else {
                    if let Some(page) = page {
                        state.return_page(page);
                    }
                    state.set_status(lang::get(key));
                }
            }
            Ok(Update::Completion(completion)) => return Some(completion),
        }
        None
    }
    /// Call from either owner's title tick, with settings.realmsNotifications.
    /// Disabled/offline/restricted accounts never start background service requests.
    pub(super) fn poll_notifications(&mut self, title: &mut Title, enabled: bool) {
        prepare_title(title);
        // The Realms screen owns independent subscriptions even when title-screen
        // notifications are disabled. Never overwrite those results with a stale
        // title request or the disabled setting's empty value.
        if matches!(title.mode, Mode::Realms(_)) {
            self.notices = None;
            return;
        }
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
fn availability(client: &mut RealmsClient, auth: &RealmsAuth) -> Result<(), AvailabilityFailure> {
    let service_failure = |error| AvailabilityFailure {
        kind: if error == RealmsError::Unauthorized {
            AvailabilityErrorKind::Unauthorized
        } else {
            AvailabilityErrorKind::Service
        },
        error,
    };
    client
        .refresh_feature_flags(auth)
        .map_err(service_failure)?;
    if client.client_compatible(auth).map_err(service_failure)? != "COMPATIBLE" {
        return Err(AvailabilityFailure {
            kind: AvailabilityErrorKind::Incompatible,
            error: RealmsError::Service {
                status: 200,
                code: Some(6001),
            },
        });
    }
    if !client.has_parental_consent(auth).map_err(service_failure)? {
        return Err(AvailabilityFailure {
            kind: AvailabilityErrorKind::ParentalConsent,
            error: RealmsError::Forbidden,
        });
    }
    Ok(())
}
pub(super) fn gate(client: &mut RealmsClient, auth: &RealmsAuth) -> RealmsResult<()> {
    availability(client, auth).map_err(|failure| failure.error)
}
fn fetch_servers(client: &RealmsClient, auth: &RealmsAuth) -> RealmsResult<(Value, Value)> {
    let realms = to_value(client.list_realms(auth)?)?;
    let snapshots = if realms_service::is_snapshot() {
        to_value(client.snapshot_eligible_realms(auth)?)?
    } else {
        Value::Null
    };
    Ok((realms, snapshots))
}
fn spawn(
    action: RealmsAction,
    identity: Arc<auth::AuthSession>,
    saves: PathBuf,
    control: TransferControl,
    client: Option<RealmsClient>,
) -> Receiver<RealmsResult<Update>> {
    let (send, receiver) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("Realms request".into())
        .spawn(move || {
            let result = execute(action, &identity, &saves, &control, client);
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
/// Validate service join targets before handing them to the connection flow.
fn valid_join_address(address: &str) -> bool {
    if address
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '?' | '#' | '@'))
    {
        return false;
    }
    let Some((host, _)) = horizon_protocol::servers::parse_address(address) else {
        return false;
    };
    let port = if address.starts_with('[') {
        address
            .split_once(']')
            .and_then(|(_, tail)| tail.strip_prefix(':'))
    } else if !host.contains(':') {
        address.split_once(':').map(|(_, port)| port)
    } else {
        None
    };
    !host.is_empty()
        && port.is_none_or(|port| port.bytes().all(|b| b.is_ascii_digit()))
        && if address.starts_with('[') || host.contains(':') {
            host.parse::<std::net::Ipv6Addr>().is_ok()
        } else {
            !host.contains(['[', ']'])
        }
}
fn execute(
    action: RealmsAction,
    identity: &auth::AuthSession,
    saves: &std::path::Path,
    control: &TransferControl,
    client: Option<RealmsClient>,
) -> RealmsResult<Update> {
    use RealmsAction::*;
    active(control)?;
    if matches!(action, SnapshotRealms | CreateSnapshotRealm { .. })
        && !realms_service::is_snapshot()
    {
        return Err(RealmsError::InvalidInput);
    }
    if let CreateSnapshotRealm {
        parent,
        name,
        description,
    } = &action
        && (*parent <= 0
            || name.trim().is_empty()
            || name.chars().count() > 32
            || description.chars().count() > 256
            || name.chars().any(char::is_control)
            || description.chars().any(char::is_control))
    {
        return Err(RealmsError::InvalidInput);
    }
    // These links are CommonLinks constants verified in realms-service/dto-request-links.javap.txt.
    if matches!(action, OpenTerms) {
        return Ok(Update::Completion(Completion::OpenUri(
            realms_service::REALMS_TERMS.into(),
        )));
    }
    let auth = credentials(identity);
    let client = match client {
        Some(client) => client,
        None => {
            let mut client = RealmsClient::new();
            gate(&mut client, &auth)?;
            client
        }
    };
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
        Ok(Update::Servers { realms, snapshots })
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
    let refresh_codes = |id| -> RealmsResult<Update> {
        active(control)?;
        Ok(match client.invite_codes(&auth, id) {
            Ok(value) => Update::Links(invite_links(value)),
            Err(error) => Update::CodeRefreshFailed { id, error },
        })
    };
    match action {
        ConfirmJoinPack(_) => Err(RealmsError::InvalidInput),
        Refresh => servers(),
        SnapshotRealms => Ok(Update::Snapshots(to_value(
            client.snapshot_eligible_realms(&auth)?,
        )?)),
        CreateSnapshotRealm {
            parent,
            name,
            description,
        } => create_snapshot(&client, &auth, parent, &name, &description, control),
        DeleteRealm(id) => {
            let realm = client.get_own_realm(&auth, id)?;
            if !realm.expired || !owned_realm(&realm, &auth) {
                return Err(RealmsError::InvalidInput);
            }
            active(control)?;
            client.delete_realm(&auth, id)?;
            Ok(Update::RefreshServers)
        }
        Join(id) => {
            let info = client.join(&auth, id)?;
            if !valid_join_address(&info.address) {
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
        Configure(id) => {
            let realm = to_value(client.get_own_realm(&auth, id)?)?;
            active(control)?;
            // Each tab has its own vanilla future. Keep the required world even
            // if another request fails, and never invent subscription/tier data.
            let (subscription, tier, regions) = std::thread::scope(|scope| {
                let subscription = scope.spawn(|| client.subscription_for(&auth, id));
                let tier = scope.spawn(|| client.tier_configuration(&auth, id));
                let regions = scope.spawn(|| client.preferred_regions(&auth));
                (
                    subscription.join().unwrap_or(Err(RealmsError::Transport)),
                    tier.join().unwrap_or(Err(RealmsError::Transport)),
                    regions.join().unwrap_or(Err(RealmsError::Transport)),
                )
            });
            active(control)?;
            Ok(Update::Configuration {
                server: realm,
                subscription,
                tier,
                regions,
            })
        }
        Slots(id) | Players(id) => server(id, None),
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
        SaveSettings {
            id,
            name,
            description,
            preference,
            render_distance,
            simulation_distance,
            continuation,
        } => {
            validate_region(&preference)?;
            let mut realm = client.get_own_realm(&auth, id)?;
            active(control)?;
            let name = name.trim().to_owned();
            let description = if description.trim().is_empty() {
                String::new()
            } else {
                description
            };
            let mut preference = preference;
            if preference
                .get("regionSelectionPreference")
                .and_then(Value::as_u64)
                != Some(2)
            {
                preference["preferredRegion"] = Value::Null;
            }
            let old_preference = realm.extra.get("regionSelectionPreference");
            let configuration_changed = realm.name.as_deref() != Some(name.as_str())
                || realm.motd.as_deref() != Some(description.as_str())
                || old_preference.and_then(|p| p.get("regionSelectionPreference"))
                    != preference.get("regionSelectionPreference")
                || old_preference
                    .and_then(|p| p.get("preferredRegion"))
                    .unwrap_or(&Value::Null)
                    != preference.get("preferredRegion").unwrap_or(&Value::Null);
            // The vanilla tab only supplies changed distances from its tier range.
            // Validate those ranges before issuing any part of the save.
            if render_distance.is_some() || simulation_distance.is_some() {
                let tier = client.tier_configuration(&auth, id)?;
                if let Some(distance) = render_distance {
                    validate_distance(&tier, "renderDistance", distance)?;
                }
                if let Some(distance) = simulation_distance {
                    validate_distance(&tier, "simDistance", distance)?;
                }
            }
            active(control)?;
            if configuration_changed {
                let mut configuration = configuration(&realm)?;
                configuration["description"]["name"] = json!(name);
                configuration["description"]["description"] = json!(description);
                configuration["regionSelectionPreference"] = preference.clone();
                client.update_configuration(&auth, id, &configuration)?;
            }
            active(control)?;
            if let Some(distance) = render_distance {
                client.update_render_distance(&auth, id, distance)?;
            }
            active(control)?;
            if let Some(distance) = simulation_distance {
                client.update_simulation_distance(&auth, id, distance)?;
            }
            active(control)?;
            // Vanilla updates its local DTO after all writes succeed; a readback
            // failure must not turn a completed save into an apparent save failure.
            realm.name = Some(name);
            realm.motd = Some(description);
            realm
                .extra
                .insert("regionSelectionPreference".into(), preference);
            Ok(Update::SettingsSaved {
                server: to_value(realm)?,
                render_distance,
                simulation_distance,
                continuation,
            })
        }
        SaveSlot { id, slot, options } => {
            let realm = client.get_own_realm(&auth, id)?;
            if realm.active_slot != slot {
                return Err(RealmsError::InvalidInput);
            }
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
        Upload {
            id,
            slot,
            folder,
            initialize,
        } => {
            prepare_target(&client, identity, id, slot, initialize.as_ref(), control)?;
            realms_transfer::upload_world_with_control(&client, &auth, id, slot, &folder, control)
                .map_err(|_| RealmsError::Transport)?;
            Ok(Update::Done("mco.upload.done", None))
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
            Ok(Update::RefreshServers)
        }
        InvitePlayer { id, name } => Ok(Update::Server(
            to_value(client.invite(&auth, id, &name)?)?,
            Some(Page::Players),
        )),
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
            let slot = reset_slot(&options)?;
            let initialize = reset_initialize(&options)?;
            let template = options
                .get("worldTemplateId")
                .and_then(Value::as_i64)
                .filter(|template| *template >= -1)
                .ok_or(RealmsError::InvalidInput)?;
            if id <= 0 {
                return Err(RealmsError::InvalidInput);
            }
            if template == -1 {
                // The editor may be cancelled. Defer all mutations until its
                // temporary world metadata has been prepared successfully.
                Ok(Update::Completion(Completion::CreateWorldForRealm {
                    id,
                    slot,
                    options,
                }))
            } else {
                prepare_target(&client, identity, id, slot, initialize.as_ref(), control)?;
                success(client.reset_world_with_template(&auth, id, template)?)?;
                active(control)?;
                server(id, Some(Page::Slots))
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
            refresh_codes(id)
        }
        UpdateInviteCode {
            id,
            code,
            enabled,
            expiration_ms,
        } => {
            client.update_invite_code(&auth, id, &code, enabled, expiration_ms)?;
            refresh_codes(id)
        }
        DeleteInviteCode { id, code } => {
            client.delete_invite_code(&auth, &code)?;
            refresh_codes(id)
        }
        RedeemInvite(code) => {
            client.redeem_invite_code(&auth, &code)?;
            Ok(Update::RefreshServers)
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
fn reset_slot(options: &Value) -> RealmsResult<i32> {
    options
        .get("slotId")
        .and_then(Value::as_i64)
        .filter(|slot| (1..=3).contains(slot))
        .map(|slot| slot as i32)
        .ok_or(RealmsError::InvalidInput)
}
pub(super) fn reset_initialize(options: &Value) -> RealmsResult<Option<(String, String)>> {
    let Some(initialize) = options.get("initialize").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let name = initialize
        .get("name")
        .and_then(Value::as_str)
        .ok_or(RealmsError::InvalidInput)?;
    let description = initialize
        .get("description")
        .and_then(Value::as_str)
        .ok_or(RealmsError::InvalidInput)?;
    validate_initialize(name, description)?;
    Ok(Some((name.to_owned(), description.to_owned())))
}
fn validate_initialize(name: &str, description: &str) -> RealmsResult<()> {
    if name.trim().is_empty()
        || name.chars().count() > 32
        || description.chars().count() > 256
        || name.chars().any(char::is_control)
        || description.chars().any(char::is_control)
    {
        Err(RealmsError::InvalidInput)
    } else {
        Ok(())
    }
}
/// RealmsResetWorldScreen.runResetTasks / RealmsUploadScreen.init. Called for
/// generated worlds only after RealmsCreateWorldFlow's metadata preparation.
pub(super) fn prepare_target(
    client: &RealmsClient,
    identity: &auth::AuthSession,
    id: i64,
    slot: i32,
    initialize: Option<&(String, String)>,
    control: &TransferControl,
) -> RealmsResult<()> {
    let check = || {
        active(control)?;
        if auth::current().is_some_and(|current| std::ptr::eq(current.as_ref(), identity)) {
            Ok(())
        } else {
            Err(RealmsError::Cancelled)
        }
    };
    if id <= 0 || !(1..=3).contains(&slot) {
        return Err(RealmsError::InvalidInput);
    }
    if let Some((name, description)) = initialize {
        validate_initialize(name, description)?;
    }
    check()?;
    let auth = credentials(identity);
    let realm = client.get_own_realm(&auth, id)?;
    check()?;
    if let Some((name, description)) = initialize {
        client.initialize(&auth, id, name, description)?;
        check()?;
    }
    if realm.active_slot != slot {
        success(client.switch_slot(&auth, id, slot)?)?;
        check()?;
    }
    Ok(())
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
fn create_snapshot(
    client: &RealmsClient,
    auth: &RealmsAuth,
    parent: i64,
    name: &str,
    description: &str,
    control: &TransferControl,
) -> RealmsResult<Update> {
    // Recheck service eligibility after confirmation, before creating anything.
    active(control)?;
    if !client
        .snapshot_eligible_realms(auth)?
        .servers
        .iter()
        .any(|s| s.id == parent)
    {
        return Err(RealmsError::InvalidInput);
    }
    active(control)?;
    let realm = client.create_snapshot_realm(auth, parent)?;
    let id = realm.id;
    let mut realm = to_value(realm)?;
    realm["name"] = json!(name.trim());
    realm["motd"] = json!(description.trim());
    // Vanilla sends only the parent id to createSnapshotRealm, then initializes
    // the child using the previously collected names. Neither step is atomic.
    let result = (|| {
        active(control)?;
        if id <= 0 || id == parent {
            return Err(RealmsError::InvalidResponse);
        }
        client.initialize(auth, id, name.trim(), description.trim())?;
        active(control)?;
        realm = to_value(client.get_own_realm(auth, id)?)?;
        Ok(())
    })();
    // Never replay creation when initialization or its readback fails. Keep the
    // child (also on cancellation) so the player can recover it from their list.
    Ok(Update::SnapshotCreated {
        realm,
        error: result.err(),
    })
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
