//! Minecraft 26.3 / authlib 10.0.77 player service contract.
//!
//! Synchronous: own this client on an IO worker, never the render thread. A client
//! belongs to one session; replacing the account requires a new client. No requests
//! occur in constructors. UI consent, optimistic updates/rollback, notifications,
//! and presence scheduling belong to the caller. Missing attributes are unknown,
//! not permission grants. Failed requests preserve caches but return errors.
use crate::auth::{self, AuthSession};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fmt,
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_BODY: u64 = 2 * 1024 * 1024;
const DISCOVERY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FRIEND_COOLDOWN: Duration = Duration::from_secs(10);
const DEFAULT_POLL: Duration = Duration::from_secs(60);
const MAX_RETRY: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToggleValue {
    Disabled,
    Enabled,
}
impl ToggleValue {
    pub fn is_enabled(self) -> bool {
        self == Self::Enabled
    }
}
impl From<bool> for ToggleValue {
    fn from(value: bool) -> Self {
        if value { Self::Enabled } else { Self::Disabled }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ChatToggleValue {
    Disabled,
    FriendsOnly,
    Enabled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Privilege {
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Privileges {
    pub online_chat: Option<Privilege>,
    pub multiplayer_server: Option<Privilege>,
    pub multiplayer_realms: Option<Privilege>,
    pub telemetry: Option<Privilege>,
    pub optional_telemetry: Option<Privilege>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendsPreferences {
    pub friends: ToggleValue,
    pub accept_invites: ToggleValue,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfanityFilterPreferences {
    pub profanity_filter_on: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPreferences {
    pub text_communication: ChatToggleValue,
}

/// ISO instant, validated on receipt; retained without precision loss.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct IsoInstant(String);
impl IsoInstant {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de> Deserialize<'de> for IsoInstant {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        chrono::DateTime::parse_from_rfc3339(&text)
            .map_err(|_| serde::de::Error::custom("invalid service instant"))?;
        Ok(Self(text))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BannedScope {
    #[serde(with = "uuid")]
    pub ban_id: u128,
    pub expires: Option<IsoInstant>,
    pub reason: String,
    pub reason_message: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BanStatus {
    #[serde(default, deserialize_with = "null_default")]
    pub banned_scopes: HashMap<String, BannedScope>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserAttributes {
    pub privileges: Option<Privileges>,
    pub profanity_filter_preferences: Option<ProfanityFilterPreferences>,
    pub friends_preferences: Option<FriendsPreferences>,
    pub chat_preferences: Option<ChatPreferences>,
    pub ban_status: Option<BanStatus>,
}
impl UserAttributes {
    /// None means absent/unknown, rather than a locally invented account setting.
    pub fn friends_enabled(&self) -> Option<bool> {
        self.friends_preferences
            .as_ref()
            .map(|p| p.friends.is_enabled())
    }
    pub fn accepts_friend_requests(&self) -> Option<bool> {
        self.friends_preferences
            .as_ref()
            .map(|p| p.accept_invites.is_enabled())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttributesUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    profanity_filter_preferences: Option<ProfanityFilterPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    friends_preferences: Option<FriendsPreferences>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Friend {
    #[serde(with = "uuid")]
    pub profile_id: u128,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendsList {
    #[serde(default, deserialize_with = "null_default")]
    pub friends: Vec<Friend>,
    #[serde(default, deserialize_with = "null_default")]
    pub incoming_requests: Vec<Friend>,
    #[serde(default, deserialize_with = "null_default")]
    pub outgoing_requests: Vec<Friend>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FriendTarget {
    Name(String),
    ProfileId(u128),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UpdateType {
    Add,
    Remove,
}
/// ADD sends or accepts; REMOVE removes, declines, or revokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendAction {
    pub target: FriendTarget,
    pub update_type: UpdateType,
}
impl FriendAction {
    fn body(&self) -> serde_json::Value {
        match &self.target {
            FriendTarget::Name(name) => {
                serde_json::json!({"name": name, "updateType": self.update_type})
            }
            FriendTarget::ProfileId(id) => {
                serde_json::json!({"profileId": format!("{id:032x}"), "updateType": self.update_type})
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PresenceStatus {
    Online,
    PlayingOffline,
    PlayingRealms,
    PlayingServer,
    PlayingHostedServer,
    Offline,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceEntry {
    #[serde(with = "uuid")]
    pub profile_id: u128,
    #[serde(with = "uuid")]
    pub pmid: u128,
    pub status: PresenceStatus,
    pub last_updated: IsoInstant,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceResponse {
    #[serde(default, deserialize_with = "null_default")]
    pub presence: Vec<PresenceEntry>,
}
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}
mod uuid {
    use super::*;
    pub fn serialize<S: Serializer>(id: &u128, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{id:032x}"))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
        let value = String::deserialize(d)?;
        let valid = value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
            || value.len() == 36
                && value.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                });
        if !valid {
            return Err(serde::de::Error::custom("invalid service UUID"));
        }
        auth::parse_uuid(&value).ok_or_else(|| serde::de::Error::custom("invalid service UUID"))
    }
}

/// No URL, response body, player input, or credentials are stored in errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocialErrorKind {
    UnknownProfile,
    Unauthorized,
    Forbidden,
    TooManyRequests,
    ServiceUnavailable,
    Connection,
    InvalidResponse,
    ResponseTooLarge,
    MissingEndpoint,
    InvalidEndpoint,
    Http,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocialError {
    pub kind: SocialErrorKind,
    pub status: Option<u16>,
    /// Caller should reschedule, not sleep on the render thread.
    pub retry_after: Option<Duration>,
}
impl SocialError {
    fn new(kind: SocialErrorKind) -> Self {
        Self {
            kind,
            status: None,
            retry_after: None,
        }
    }
    fn http(status: u16, retry_after: Option<Duration>) -> Self {
        let kind = match status {
            400 => SocialErrorKind::UnknownProfile,
            401 => SocialErrorKind::Unauthorized,
            403 => SocialErrorKind::Forbidden,
            429 => SocialErrorKind::TooManyRequests,
            500..=599 => SocialErrorKind::ServiceUnavailable,
            _ => SocialErrorKind::Http,
        };
        Self {
            kind,
            status: Some(status),
            retry_after,
        }
    }
}
impl fmt::Display for SocialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "social service: {:?}", self.kind)
    }
}
impl std::error::Error for SocialError {}

#[derive(Deserialize)]
struct DiscoveryDocument {
    environment: String,
    discovery: Discovery,
}
#[derive(Deserialize)]
struct Discovery {
    player: EndpointMap,
}
#[derive(Deserialize)]
struct EndpointMap {
    endpoints: HashMap<String, Endpoint>,
}
#[derive(Deserialize)]
struct Endpoint {
    uri: String,
}
struct WireResponse<T> {
    body: Option<T>,
    etag: Option<String>,
    retry_after: Option<Duration>,
}
struct Backoff {
    until: Instant,
    failures: u32,
    error: SocialError,
}

/// State of the last read; stale snapshots must not trigger friend notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheState {
    Empty,
    Fresh,
    Cached,
    Stale,
}

/// Serialized ownership (`&mut self`) coalesces calls without holding UI locks.
/// Retry is caller-driven with bounded exponential backoff, never an implicit
/// replay of a write. Cache storage is per account and bounded by MAX_BODY.
pub struct SocialClient {
    session: Arc<AuthSession>,
    agent: ureq::Agent,
    discovery_url: String,
    allow_loopback: bool,
    discovery: Option<(DiscoveryDocument, Instant)>,
    backoff: HashMap<&'static str, Backoff>,
    attributes: Option<UserAttributes>,
    friends: Option<FriendsList>,
    friends_etag: Option<String>,
    friends_next: Option<Instant>,
    presence: Option<PresenceResponse>,
    presence_etag: Option<String>,
    friends_poll: Duration,
    presence_poll: Duration,
    friends_state: CacheState,
    presence_state: CacheState,
}
impl SocialClient {
    pub fn new(session: Arc<AuthSession>) -> Self {
        Self::with_discovery_url(session, auth::discovery_url())
    }
    /// Explicit endpoint injection for isolated mock servers. HTTP is allowed
    /// only for literal loopback hosts, and only with a loopback discovery URL.
    pub fn with_discovery_url(session: Arc<AuthSession>, discovery_url: String) -> Self {
        let allow_loopback = is_loopback_http(&discovery_url);
        Self {
            session,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(10))
                .timeout_connect(Duration::from_secs(5))
                .timeout_read(Duration::from_secs(5))
                .timeout_write(Duration::from_secs(5))
                .redirects(0)
                .build(),
            discovery_url,
            allow_loopback,
            discovery: None,
            backoff: HashMap::new(),
            attributes: None,
            friends: None,
            friends_etag: None,
            friends_next: None,
            presence: None,
            presence_etag: None,
            friends_poll: DEFAULT_POLL,
            presence_poll: DEFAULT_POLL,
            friends_state: CacheState::Empty,
            presence_state: CacheState::Empty,
        }
    }
    pub fn cached_attributes(&self) -> Option<&UserAttributes> {
        self.attributes.as_ref()
    }
    pub fn cached_friends(&self) -> Option<&FriendsList> {
        self.friends.as_ref()
    }
    pub fn cached_presence(&self) -> Option<&PresenceResponse> {
        self.presence.as_ref()
    }
    pub fn friends_cache_state(&self) -> CacheState {
        self.friends_state
    }
    pub fn presence_cache_state(&self) -> CacheState {
        self.presence_state
    }
    /// Overlay uses this; background polling uses five times this interval.
    pub fn friends_poll_interval(&self) -> Duration {
        self.friends_poll
    }
    /// Changed presence after this interval, heartbeat after five intervals.
    pub fn presence_poll_interval(&self) -> Duration {
        self.presence_poll
    }

    /// Fetch before populating account-controlled options; never substitutes
    /// offline/default preferences for an unsuccessful authenticated response.
    pub fn get_attributes(&mut self) -> Result<UserAttributes, SocialError> {
        let response: WireResponse<UserAttributes> =
            self.call("getAttributes", "GET", None, None)?;
        let value = response
            .body
            .ok_or_else(|| SocialError::new(SocialErrorKind::InvalidResponse))?;
        self.attributes = Some(value.clone());
        Ok(value)
    }
    pub fn update_friend_settings(
        &mut self,
        enabled: bool,
        allow_requests: bool,
    ) -> Result<Option<UserAttributes>, SocialError> {
        self.update_attributes(AttributesUpdate {
            friends_preferences: Some(FriendsPreferences {
                friends: enabled.into(),
                accept_invites: allow_requests.into(),
            }),
            profanity_filter_preferences: None,
        })
    }
    pub fn update_profanity_filter(
        &mut self,
        enabled: bool,
    ) -> Result<Option<UserAttributes>, SocialError> {
        self.update_attributes(AttributesUpdate {
            friends_preferences: None,
            profanity_filter_preferences: Some(ProfanityFilterPreferences {
                profanity_filter_on: enabled,
            }),
        })
    }
    fn update_attributes(
        &mut self,
        update: AttributesUpdate,
    ) -> Result<Option<UserAttributes>, SocialError> {
        let body = serde_json::to_string(&update)
            .map_err(|_| SocialError::new(SocialErrorKind::InvalidResponse))?;
        let response: WireResponse<UserAttributes> =
            self.call("updateAttributes", "POST", Some(&body), None)?;
        // Empty acknowledgement is a successful write, not evidence of server
        // preferences. Invalidate and let the caller refresh getAttributes.
        self.attributes = response.body.clone();
        Ok(response.body)
    }
    pub fn list_friends(&mut self) -> Result<FriendsList, SocialError> {
        self.friends_state = if self.friends.is_some() {
            CacheState::Stale
        } else {
            CacheState::Empty
        };
        self.check_backoff("getFriends")?;
        if self.friends_next.is_some_and(|t| Instant::now() < t)
            && let Some(value) = &self.friends
        {
            self.friends_state = CacheState::Cached;
            return Ok(value.clone());
        }
        let etag = self.friends_etag.clone();
        let response: WireResponse<FriendsList> =
            self.call("getFriends", "GET", None, etag.as_deref())?;
        self.friends_state = CacheState::Fresh;
        if let Some(interval) = response.retry_after {
            self.friends_poll = interval;
        }
        if let Some(body) = response.body {
            self.friends = Some(body);
            self.friends_etag = response.etag;
        }
        self.friends_next = Some(Instant::now() + FRIEND_COOLDOWN);
        let value = self.friends.get_or_insert_with(FriendsList::default);
        Ok(value.clone())
    }
    pub fn update_friends(
        &mut self,
        action: FriendAction,
    ) -> Result<Option<FriendsList>, SocialError> {
        let body = action.body().to_string();
        let response: WireResponse<FriendsList> =
            self.call("updateFriends", "PUT", Some(&body), None)?;
        if let Some(value) = &response.body {
            self.friends_state = CacheState::Fresh;
            self.friends = Some(value.clone());
            self.friends_etag = None;
            self.friends_next = Some(Instant::now() + FRIEND_COOLDOWN);
            self.backoff.remove("getFriends");
        } else {
            self.friends_next = None;
            self.friends_etag = None;
            self.friends_state = if self.friends.is_some() {
                CacheState::Stale
            } else {
                CacheState::Empty
            };
        }
        Ok(response.body)
    }
    /// The caller maps sharing NONE=>Offline, LIMITED=>Online, ALL=>actual state,
    /// and suppresses periodic posts if friends are disabled or the list is empty.
    pub fn presence(&mut self, status: PresenceStatus) -> Result<PresenceResponse, SocialError> {
        let body = serde_json::json!({"status": status}).to_string();
        let etag = self.presence_etag.clone();
        self.presence_state = if self.presence.is_some() {
            CacheState::Stale
        } else {
            CacheState::Empty
        };
        let response: WireResponse<PresenceResponse> =
            self.call("updatePresence", "POST", Some(&body), etag.as_deref())?;
        self.presence_state = CacheState::Fresh;
        if let Some(interval) = response.retry_after {
            self.presence_poll = interval;
        }
        self.presence_etag = response.etag;
        if let Some(body) = response.body {
            self.presence = Some(body);
        }
        Ok(self
            .presence
            .get_or_insert_with(PresenceResponse::default)
            .clone())
    }

    fn check_backoff(&self, key: &'static str) -> Result<(), SocialError> {
        if let Some(state) = self.backoff.get(key)
            && let Some(remaining) = state.until.checked_duration_since(Instant::now())
        {
            let mut error = state.error.clone();
            error.retry_after = Some(remaining);
            return Err(error);
        }
        Ok(())
    }
    fn remember<T>(
        &mut self,
        key: &'static str,
        result: Result<T, SocialError>,
    ) -> Result<T, SocialError> {
        match result {
            Ok(value) => {
                self.backoff.remove(key);
                Ok(value)
            }
            Err(mut error) => {
                let failures = self
                    .backoff
                    .get(key)
                    .map_or(0, |s| s.failures.saturating_add(1))
                    .min(6);
                let base = if key == "discovery" { 60 } else { 1 };
                let delay = error
                    .retry_after
                    .unwrap_or(Duration::from_secs(base * (1 << failures)))
                    .min(MAX_RETRY);
                error.retry_after = Some(delay);
                self.backoff.insert(
                    key,
                    Backoff {
                        until: Instant::now() + delay,
                        failures,
                        error: error.clone(),
                    },
                );
                Err(error)
            }
        }
    }
    fn endpoint(&mut self, key: &'static str) -> Result<String, SocialError> {
        if self
            .discovery
            .as_ref()
            .is_none_or(|(_, time)| time.elapsed() >= DISCOVERY_TTL)
        {
            self.check_backoff("discovery")?;
            let result = self
                .request::<DiscoveryDocument>(&self.discovery_url, "GET", None, None, false)
                .and_then(|r| {
                    r.body
                        .ok_or_else(|| SocialError::new(SocialErrorKind::InvalidResponse))
                })
                .and_then(|d| {
                    if d.environment == "offline" {
                        Err(SocialError::new(SocialErrorKind::ServiceUnavailable))
                    } else {
                        Ok(d)
                    }
                });
            let document = self.remember("discovery", result)?;
            self.discovery = Some((document, Instant::now()));
        }
        self.discovery
            .as_ref()
            .and_then(|(d, _)| d.discovery.player.endpoints.get(key))
            .map(|endpoint| endpoint.uri.clone())
            .ok_or_else(|| SocialError::new(SocialErrorKind::MissingEndpoint))
    }
    fn call<T: DeserializeOwned>(
        &mut self,
        key: &'static str,
        method: &str,
        body: Option<&str>,
        etag: Option<&str>,
    ) -> Result<WireResponse<T>, SocialError> {
        self.check_backoff(key)?;
        let url = self.endpoint(key)?;
        let result = self.request(&url, method, body, etag, true);
        self.remember(key, result)
    }
    fn request<T: DeserializeOwned>(
        &self,
        url: &str,
        method: &str,
        body: Option<&str>,
        etag: Option<&str>,
        authenticated: bool,
    ) -> Result<WireResponse<T>, SocialError> {
        if !valid_url(url, self.allow_loopback) {
            return Err(SocialError::new(SocialErrorKind::InvalidEndpoint));
        }
        let mut request = self.agent.request(method, url);
        if authenticated {
            request = request.set(
                "Authorization",
                &format!("Bearer {}", self.session.access_token()),
            );
        }
        if let Some(etag) = etag {
            request = request.set("If-None-Match", etag);
        }
        let result = match body {
            Some(body) => request
                .set("Content-Type", "application/json; charset=utf-8")
                .send_string(body),
            None => request.call(),
        };
        let response = match result {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                return Err(SocialError::http(status, retry_after(&response)));
            }
            Err(ureq::Error::Transport(_)) => {
                return Err(SocialError::new(SocialErrorKind::Connection));
            }
        };
        let retry_after = retry_after(&response);
        if response.status() == 304 {
            // A conditional cache hit without a prior validator is malformed.
            if etag.is_none() {
                return Err(SocialError::new(SocialErrorKind::InvalidResponse));
            }
            return Ok(WireResponse {
                body: None,
                etag: etag.map(str::to_owned),
                retry_after,
            });
        }
        if !(200..300).contains(&response.status()) {
            return Err(SocialError::http(response.status(), retry_after));
        }
        let etag = response
            .header("ETag")
            .filter(|s| s.len() <= 1024)
            .map(str::to_owned);
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_BODY + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SocialError::new(SocialErrorKind::Connection))?;
        if bytes.len() as u64 > MAX_BODY {
            return Err(SocialError::new(SocialErrorKind::ResponseTooLarge));
        }
        let body = if bytes.is_empty() {
            None
        } else {
            serde_json::from_slice::<Option<T>>(&bytes)
                .map_err(|_| SocialError::new(SocialErrorKind::InvalidResponse))?
        };
        Ok(WireResponse {
            body,
            etag,
            retry_after,
        })
    }
}
fn retry_after(response: &ureq::Response) -> Option<Duration> {
    response
        .header("Retry-After")
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds).min(MAX_RETRY))
}
fn is_loopback_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority
        .strip_prefix("127.0.0.1")
        .or_else(|| authority.strip_prefix("[::1]"));
    host_port.is_some_and(|suffix| {
        suffix.is_empty()
            || suffix
                .strip_prefix(':')
                .is_some_and(|port| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
    })
}
fn valid_url(url: &str, allow_loopback: bool) -> bool {
    if url
        .bytes()
        .any(|b| b.is_ascii_control() || b == b' ' || b == b'\\')
        || url.contains('#')
    {
        return false;
    }
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    url.starts_with("https://") || allow_loopback && is_loopback_http(url)
}
