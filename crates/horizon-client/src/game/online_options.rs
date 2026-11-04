//! OnlineOptionsScreen and its account-scoped asynchronous social consumer.
use super::Game;
use horizon_client::{
    auth::{self, AuthSession},
    hud::{HudAtlas, HudBatch},
    options_file::V,
    options_ui::{OnlineAction, OnlineStatus, OptionsUi, Page},
    settings::Settings,
    social_service::{
        CacheState, FriendAction, FriendTarget, FriendsList, PresenceResponse, PresenceStatus,
        SocialClient, SocialError, SocialErrorKind, UpdateType, UserAttributes,
    },
    text::Text,
    toasts::FriendToast,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) const PRIVACY_URL: &str = "https://aka.ms/MinecraftJavaXboxPrivacyAndSafety";

enum Job {
    Attributes,
    Settings(bool, bool),
    Friends,
    UpdateFriend(FriendAction),
    Presence(PresenceStatus),
}
enum Reply {
    Attributes(Result<UserAttributes, SocialError>),
    Settings(bool, bool, Result<Option<UserAttributes>, SocialError>),
    Friends(Result<FriendsList, SocialError>, CacheState, Duration),
    UpdateFriend(Result<Option<FriendsList>, SocialError>),
    Presence(
        Result<PresenceResponse, SocialError>,
        PresenceStatus,
        Duration,
    ),
}
struct Worker {
    session: Arc<AuthSession>,
    send: mpsc::Sender<(u64, Job)>,
    receive: mpsc::Receiver<(u64, Reply)>,
    cancelled: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
}
impl Worker {
    fn new(session: Arc<AuthSession>) -> Self {
        let (send, jobs) = mpsc::channel();
        let (results, receive) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let revision = Arc::new(AtomicU64::new(0));
        let current_revision = revision.clone();
        let account = session.clone();
        std::thread::spawn(move || {
            let mut client = SocialClient::new(account);
            while let Ok((id, job)) = jobs.recv() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if id != current_revision.load(Ordering::Acquire) {
                    continue;
                }
                let reply = match job {
                    Job::Attributes => Reply::Attributes(client.get_attributes()),
                    Job::Settings(list, requests) => Reply::Settings(
                        list,
                        requests,
                        client.update_friend_settings(list, requests),
                    ),
                    Job::Friends => {
                        let result = client.list_friends();
                        Reply::Friends(
                            result,
                            client.friends_cache_state(),
                            client.friends_poll_interval(),
                        )
                    }
                    Job::UpdateFriend(action) => Reply::UpdateFriend(client.update_friends(action)),
                    Job::Presence(status) => {
                        let result = client.presence(status);
                        Reply::Presence(result, status, client.presence_poll_interval())
                    }
                };
                if stop.load(Ordering::Acquire) || results.send((id, reply)).is_err() {
                    break;
                }
            }
        });
        Self {
            session,
            send,
            receive,
            cancelled,
            revision,
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) struct OnlineState {
    worker: Option<Worker>,
    revision: u64,
    busy: bool,
    loaded: bool,
    friends_enabled: bool,
    allow_requests: bool,
    rollback: Option<(bool, bool)>,
    consent: bool,
    pending_removal: Option<u128>,
    message: Option<&'static str>,
    attributes_at: Instant,
    friends_at: Instant,
    presence_at: Instant,
    friends_interval: Duration,
    presence_interval: Duration,
    last_presence: Option<PresenceStatus>,
    friends_success: bool,
    pub(super) friends: Option<FriendsList>,
    pub(super) presence: Option<PresenceResponse>,
    notifications: VecDeque<FriendToast>,
    toast: Option<FriendToast>,
}
impl Default for OnlineState {
    fn default() -> Self {
        Self {
            worker: None,
            revision: 0,
            busy: false,
            loaded: false,
            friends_enabled: false,
            allow_requests: false,
            rollback: None,
            consent: false,
            pending_removal: None,
            message: None,
            attributes_at: Instant::now(),
            friends_at: Instant::now(),
            presence_at: Instant::now() + Duration::from_secs(60),
            friends_interval: Duration::from_secs(60),
            presence_interval: Duration::from_secs(60),
            last_presence: None,
            friends_success: false,
            friends: None,
            presence: None,
            notifications: VecDeque::new(),
            toast: None,
        }
    }
}
impl OnlineState {
    fn send(&mut self, job: Job) {
        if self.busy {
            return;
        }
        if let Some(worker) = &self.worker {
            self.busy = worker.send.send((self.revision, job)).is_ok();
        }
    }
    fn status(&self, realms_available: bool) -> OnlineStatus {
        OnlineStatus {
            available: self.worker.is_some() && self.loaded,
            busy: self.busy,
            friends_enabled: self.friends_enabled,
            allow_requests: self.allow_requests,
            realms_available,
            message: self.message,
        }
    }
    /// Invalidate callbacks when the owning screen/account is cancelled. Writes already
    /// accepted by the service cannot be undone; the next read reconciles authority.
    pub(super) fn cancel_updates(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if let Some(worker) = &self.worker {
            worker.revision.store(self.revision, Ordering::Release);
        }
        self.busy = false;
        self.loaded = false;
        self.consent = false;
        self.pending_removal = None;
        self.rollback = None;
        self.attributes_at = Instant::now();
    }
    fn failure(&mut self, error: &SocialError) {
        self.message = Some(match error.kind {
            SocialErrorKind::Unauthorized => "gui.friends.error.unauthorized",
            SocialErrorKind::Forbidden => "gui.friends.error.forbidden.message",
            SocialErrorKind::Connection => "gui.friends.error.connection_issue",
            SocialErrorKind::UnknownProfile => "gui.friends.error.user_may_lack_active_profile",
            SocialErrorKind::TooManyRequests => "gui.friends.error.rateLimited.message",
            _ => "gui.friends.error.temporary_unavailable",
        });
        let retry = error
            .retry_after
            .unwrap_or(Duration::from_secs(60))
            .max(Duration::from_secs(1));
        self.attributes_at = Instant::now() + retry;
        self.friends_at = self.attributes_at;
        self.presence_at = self.attributes_at;
    }
    /// Called by startup and Game every frame; never waits for a worker.
    pub(super) fn tick(
        &mut self,
        session: Option<Arc<AuthSession>>,
        settings: &Settings,
        ui: &mut OptionsUi,
        in_world: bool,
        actual_presence: PresenceStatus,
        realms_available: bool,
    ) {
        let same = match (&self.worker, &session) {
            (Some(worker), Some(session)) => Arc::ptr_eq(&worker.session, session),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *self = Self::default(); // drops the old receiver: late account results cannot apply
            self.worker = session.map(Worker::new);
            self.message = self.worker.as_ref().map(|_| "gui.friends.loading_friends");
        }
        while let Some((revision, reply)) =
            self.worker.as_ref().and_then(|w| w.receive.try_recv().ok())
        {
            if revision != self.revision {
                continue;
            }
            self.busy = false;
            match reply {
                Reply::Attributes(Ok(attributes)) => {
                    if let (Some(list), Some(requests)) = (
                        attributes.friends_enabled(),
                        attributes.accepts_friend_requests(),
                    ) {
                        self.friends_enabled = list;
                        self.allow_requests = requests;
                        self.loaded = true;
                        self.message = None;
                    } else {
                        self.message = Some("gui.friends.error.temporary_unavailable");
                        self.attributes_at = Instant::now() + Duration::from_secs(60);
                    }
                }
                Reply::Attributes(Err(error)) => self.failure(&error),
                Reply::Settings(list, requests, result) => {
                    ui.complete_friends_consent(result.is_ok());
                    match result {
                        Ok(_) => {
                            self.friends_enabled = list;
                            self.allow_requests = requests;
                            self.message = None;
                            self.friends_at = Instant::now();
                        }
                        Err(error) => {
                            // The entry confirmation stays open until enabling succeeds.
                            // Restore its runtime gate as well, so Turn On can retry.
                            let retry_consent = ui.page() == Page::Friends
                                && list
                                && self.rollback.is_some_and(|(enabled, _)| !enabled);
                            if (!list || retry_consent)
                                && let Some((old_list, old_requests)) = self.rollback
                            {
                                self.friends_enabled = old_list;
                                self.allow_requests = old_requests;
                            }
                            if retry_consent {
                                self.consent = true;
                            }
                            self.failure(&error);
                        }
                    }
                    self.rollback = None;
                }
                Reply::UpdateFriend(result) => match result {
                    Ok(data) => {
                        ui.mark_friends_list_refreshed();
                        if let Some(data) = data {
                            self.friends = Some(data);
                        }
                        self.friends_success = false;
                        self.friends_at = Instant::now();
                        self.message = None;
                        ui.clear_friend_request();
                    }
                    Err(error) => self.failure(&error),
                },
                Reply::Friends(result, cache, interval) => {
                    self.friends_interval = interval.max(Duration::from_secs(1));
                    self.friends_at = Instant::now()
                        + self
                            .friends_interval
                            .saturating_mul(if ui.page() == Page::Friends { 1 } else { 5 });
                    match result {
                        Ok(data) if cache != CacheState::Stale => {
                            if self.friends_enabled {
                                ui.mark_friends_list_refreshed();
                                if let Some(previous) = &self.friends
                                    && self.friends_success
                                    && (!in_world || settings.file.bool("inGameNotification"))
                                {
                                    for (key, name) in friend_notifications(previous, &data) {
                                        if self.notifications.len() < 64 {
                                            let text = Text::translate(key, &[Text::literal(name)])
                                                .plain();
                                            let profile = data
                                                .friends
                                                .iter()
                                                .chain(&data.incoming_requests)
                                                .chain(&data.outgoing_requests)
                                                .find(|friend| friend.name == name);
                                            let uuid = if key
                                                == "gui.friends.toast.request_sent.message"
                                            {
                                                None
                                            } else {
                                                profile.map(|friend| friend.profile_id)
                                            };
                                            let mut toast = FriendToast::new(uuid, text);
                                            if uuid.is_some() {
                                                toast.profile_name = Some(name.to_owned());
                                            }
                                            self.notifications.push_back(toast);
                                        }
                                    }
                                }
                                self.friends = Some(data);
                                self.friends_success = true;
                                self.message = None;
                            }
                        }
                        Ok(_) => {
                            self.friends_success = false;
                        }
                        Err(error) => {
                            self.friends_success = false;
                            self.failure(&error);
                        }
                    }
                }
                Reply::Presence(result, status, interval) => {
                    self.presence_interval = interval.max(Duration::from_secs(1));
                    self.presence_at = Instant::now() + self.presence_interval;
                    match result {
                        Ok(data) => {
                            self.presence = Some(data);
                            self.last_presence = Some(status);
                        }
                        Err(error) => self.failure(&error),
                    }
                }
            }
        }
        let now = Instant::now();
        if self.worker.is_some() && !self.busy && !self.consent && self.pending_removal.is_none() {
            if !self.loaded && now >= self.attributes_at {
                self.send(Job::Attributes);
            } else if self.loaded && self.friends_enabled {
                let status = match settings.file.int("sharePresence") {
                    0 => PresenceStatus::Offline,
                    1 => PresenceStatus::Online,
                    _ => actual_presence,
                };
                if now >= self.friends_at {
                    self.send(Job::Friends);
                } else if self.friends.as_ref().is_some_and(|f| !f.friends.is_empty())
                    && now >= self.presence_at
                    && (self.last_presence != Some(status)
                        || now.saturating_duration_since(self.presence_at)
                            >= self.presence_interval.saturating_mul(4))
                {
                    self.send(Job::Presence(status));
                }
            }
        }
        if self.loaded
            && ui.page() == Page::Friends
            && !self.friends_enabled
            && !self.busy
            && !self.consent
        {
            self.consent = true;
            ui.confirm_online_friends();
        }
        ui.set_online_status(self.status(realms_available));
        ui.set_friends(
            self.worker.as_ref().map_or("", |w| w.session.name.as_str()),
            self.friends.as_ref(),
            self.presence.as_ref(),
            self.friends_success,
        );
    }
    /// Returns true only after OptionsUi has confirmed opening the privacy link.
    pub(super) fn action(
        &mut self,
        action: OnlineAction,
        settings: &mut Settings,
        ui: &mut OptionsUi,
    ) -> bool {
        if action == OnlineAction::OpenPrivacy {
            return true;
        }
        if action == OnlineAction::CancelUpdates {
            self.consent = false;
            self.pending_removal = None;
            if self.rollback.is_some() {
                self.cancel_updates();
            }
            return false;
        }
        if action == OnlineAction::RealmsNotifications {
            settings.file.set(
                "realmsNotifications",
                V::Bool(!settings.file.bool("realmsNotifications")),
            );
            return false;
        }
        if action == OnlineAction::ConfirmFriends(false) {
            if self.busy && self.rollback.is_some() {
                self.cancel_updates();
            }
            self.consent = false;
            return false;
        }
        if action == OnlineAction::ConfirmRemoval(false) {
            self.pending_removal = None;
            return false;
        }
        if ui.split_guest || !self.loaded || self.worker.is_none() || self.busy {
            return false;
        }
        let mut update = None;
        if self.friends_enabled && self.friends_success {
            let list = self.friends.as_ref();
            let contains = |entries: &[horizon_client::social_service::Friend], id| {
                entries.iter().any(|f| f.profile_id == id)
            };
            match action {
                OnlineAction::SendRequest => {
                    let name = ui.friend_request_name().to_owned();
                    if name.is_empty() {
                        return false;
                    }
                    let own = self.worker.as_ref().unwrap().session.name.as_str();
                    let validation = if name.eq_ignore_ascii_case(own) {
                        Some("gui.friends.validation.cannot_add_self")
                    } else {
                        list.and_then(|list| {
                            [
                                (&list.friends, "gui.friends.validation.already_friend"),
                                (
                                    &list.outgoing_requests,
                                    "gui.friends.validation.already_outgoing",
                                ),
                                (
                                    &list.incoming_requests,
                                    "gui.friends.validation.already_incoming",
                                ),
                            ]
                            .into_iter()
                            .find_map(|(entries, key)| {
                                entries
                                    .iter()
                                    .any(|f| f.name.eq_ignore_ascii_case(&name))
                                    .then_some(key)
                            })
                        })
                    };
                    if let Some(key) = validation {
                        self.notifications.push_back(FriendToast::new(
                            None,
                            Text::translate(key, &[Text::literal(name)]).plain(),
                        ));
                        return false;
                    }
                    update = Some(FriendAction {
                        target: FriendTarget::Name(name),
                        update_type: UpdateType::Add,
                    });
                }
                OnlineAction::AcceptRequest(id)
                    if list.is_some_and(|l| contains(&l.incoming_requests, id)) =>
                {
                    update = Some(FriendAction {
                        target: FriendTarget::ProfileId(id),
                        update_type: UpdateType::Add,
                    });
                }
                OnlineAction::DeclineRequest(id)
                    if list.is_some_and(|l| contains(&l.incoming_requests, id)) =>
                {
                    update = Some(FriendAction {
                        target: FriendTarget::ProfileId(id),
                        update_type: UpdateType::Remove,
                    });
                }
                OnlineAction::RevokeRequest(id)
                    if list.is_some_and(|l| contains(&l.outgoing_requests, id)) =>
                {
                    update = Some(FriendAction {
                        target: FriendTarget::ProfileId(id),
                        update_type: UpdateType::Remove,
                    });
                }
                OnlineAction::RemoveFriend(id) => {
                    if let Some(friend) =
                        list.and_then(|l| l.friends.iter().find(|f| f.profile_id == id))
                    {
                        self.pending_removal = Some(id);
                        ui.confirm_friend_removal(id, friend.name.clone());
                    }
                }
                OnlineAction::ConfirmRemoval(true) => {
                    if let Some(id) = self.pending_removal.take()
                        && list.is_some_and(|l| contains(&l.friends, id))
                    {
                        update = Some(FriendAction {
                            target: FriendTarget::ProfileId(id),
                            update_type: UpdateType::Remove,
                        });
                    }
                }
                _ => {}
            }
        }
        if let Some(update) = update {
            self.message = None;
            self.send(Job::UpdateFriend(update));
            ui.set_online_status(self.status(true));
            return false;
        }
        match action {
            OnlineAction::FriendsList if !self.friends_enabled => {
                self.consent = true;
                ui.confirm_online_friends();
            }
            OnlineAction::FriendsList => {
                self.rollback = Some((self.friends_enabled, self.allow_requests));
                self.friends_enabled = false;
                self.allow_requests = false;
                settings.file.set("inGameNotification", V::Bool(false));
                self.send(Job::Settings(false, false));
            }
            OnlineAction::ConfirmFriends(true) if self.consent => {
                self.consent = false;
                self.rollback = Some((self.friends_enabled, self.allow_requests));
                self.friends_enabled = true;
                self.allow_requests = true;
                self.send(Job::Settings(true, true));
            }
            OnlineAction::AcceptRequests if self.friends_enabled => {
                self.rollback = Some((self.friends_enabled, self.allow_requests));
                self.send(Job::Settings(true, !self.allow_requests));
            }
            OnlineAction::Notifications if self.friends_enabled => {
                settings.file.set(
                    "inGameNotification",
                    V::Bool(!settings.file.bool("inGameNotification")),
                );
            }
            OnlineAction::Presence if self.friends_enabled => {
                settings.file.set(
                    "sharePresence",
                    V::Int((settings.file.int("sharePresence") + 1) % 3),
                );
            }
            _ => {}
        }
        false
    }
    /// Minecraft.destroy sends offline without waiting on the UI thread.
    pub(super) fn send_offline_presence(&self) {
        if self.friends_enabled
            && self.friends.as_ref().is_some_and(|f| !f.friends.is_empty())
            && let Some(worker) = &self.worker
        {
            let session = worker.session.clone();
            std::thread::spawn(move || {
                let _ = SocialClient::new(session).presence(PresenceStatus::Offline);
            });
        }
    }

    fn toast_profiles(&self) -> Vec<serde_json::Value> {
        self.toast
            .iter()
            .chain(self.notifications.iter().take(4))
            .filter_map(|toast| {
                toast.uuid.map(|id| {
                serde_json::json!({"id": format!("{id:032x}"), "name": toast.profile_name})
            })
            })
            .collect()
    }

    pub(super) fn toast_slots(&self) -> u32 {
        self.toast.as_ref().map_or(0, |toast| {
            toast
                .slot
                .map_or(0, |slot| ((1u32 << toast.slot_count().min(31)) - 1) << slot)
        })
    }

    pub(super) fn draw_toasts(
        &mut self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        used_slots: u32,
        display_time: f64,
        hidden: bool,
    ) -> Option<bool> {
        if self.toast.is_none() {
            self.toast = self.notifications.pop_front();
        }
        let toast = self.toast.as_mut()?;
        toast.layout(atlas);
        if toast.slot.is_none() {
            let count = toast.slot_count();
            if count > Game::TOAST_SLOTS {
                self.toast = None;
                return None;
            }
            let mask = (1u32 << count) - 1;
            toast.slot =
                (0..=Game::TOAST_SLOTS - count).find(|slot| used_slots & (mask << slot) == 0);
            toast.slot?;
        }
        let (sound, finished) = toast.update(display_time);
        if finished {
            self.toast = None;
        } else if let (false, Some(slot)) = (hidden, toast.slot) {
            toast.draw(atlas, batch, 32.0 * slot as f32);
        }
        sound
    }
}

fn friend_notifications<'a>(
    old: &FriendsList,
    new: &'a FriendsList,
) -> Vec<(&'static str, &'a str)> {
    let contains = |list: &[horizon_client::social_service::Friend],
                    friend: &horizon_client::social_service::Friend| {
        list.iter().any(|f| f == friend)
    };
    let mut notices = Vec::new();
    for friend in &new.friends {
        if !contains(&old.friends, friend) {
            let accepted = contains(&old.incoming_requests, friend)
                || contains(&old.outgoing_requests, friend);
            notices.push((
                if accepted {
                    "gui.friends.toast.request_accepted.message"
                } else {
                    "gui.friends.toast.friend_added.message"
                },
                friend.name.as_str(),
            ));
        }
    }
    for (before, after, key) in [
        (
            &old.incoming_requests,
            &new.incoming_requests,
            "gui.friends.toast.request_received.message",
        ),
        (
            &old.outgoing_requests,
            &new.outgoing_requests,
            "gui.friends.toast.request_sent.message",
        ),
    ] {
        for friend in after {
            if !contains(before, friend) && !contains(&new.friends, friend) {
                notices.push((key, friend.name.as_str()));
            }
        }
    }
    notices
}

impl Game<'_> {
    pub(super) fn tick_online_options(&mut self) {
        if self.options.dedicated.is_some() || self.serving.is_some() {
            return;
        }
        let actual = if self.title.is_some() {
            PresenceStatus::Online
        } else if self.integrated.is_some() {
            if self.is_published() {
                PresenceStatus::PlayingHostedServer
            } else {
                PresenceStatus::PlayingOffline
            }
        } else if self.realms.connected_realm.is_some() {
            PresenceStatus::PlayingRealms
        } else if self.remote.is_some() {
            PresenceStatus::PlayingServer
        } else {
            PresenceStatus::PlayingOffline
        };
        self.online.tick(
            auth::current(),
            &self.settings,
            &mut self.ui.options,
            self.title.is_none(),
            actual,
            true,
        );
    }
    pub(super) fn online_option_action(&mut self, action: OnlineAction) {
        if self
            .online
            .action(action, &mut self.settings, &mut self.ui.options)
        {
            super::packs::open_uri(PRIVACY_URL);
        }
        self.settings.save(&Self::options_path());
    }
    pub(super) fn draw_online_toasts(&mut self, batch: &mut HudBatch, hidden: bool) {
        let profiles = self.online.toast_profiles();
        if !profiles.is_empty() {
            self.resolve_inline_profiles(&profiles);
            self.flush_skin_slots();
        }
        let used = self.toast_slots_used() | self.tutorial_toast_slots();
        if let Some(show) = self.online.draw_toasts(
            &self.gui.atlas,
            batch,
            used,
            f64::from(self.settings.notification_display_time),
            hidden,
        ) {
            self.audio.play(
                if show {
                    "minecraft:ui.toast.in"
                } else {
                    "minecraft:ui.toast.out"
                },
                None,
                1.0,
                1.0,
            );
        }
    }
}
