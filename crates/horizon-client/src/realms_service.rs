//! Blocking Minecraft 26.3 Realms HTTP protocol; run on an IO worker.
//! Bytecode evidence: .local/menu-validation/realms-service and RealmsClient.javap.txt.
//! No account discovery, persistence, retries, UI, archive handling or implicit consent.
//! Callers must confirm mutations/terms and handle RetryAfter before invoking again.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    io::{self, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const LEGACY: &str = "https://pc.realms.minecraft.net";
const AKS: &str = "https://java.frontendlegacy.realms.minecraft-services.net";
const FRONTEND: &str = "https://java.frontend.realms.minecraft-services.net";
const MAX_RESPONSE: u64 = 8 * 1024 * 1024;
pub const BUY_REALMS: &str = "https://aka.ms/BuyJavaRealms";
pub const START_TRIAL: &str = "https://aka.ms/startjavarealmstrial";
pub const REALMS_TERMS: &str = "https://aka.ms/MinecraftRealmsTerms";

/// RealmsMainScreen.isSnapshot with vanilla's default snapshotToggle.
/// Stable releases cannot opt into snapshot Realms.
pub fn is_snapshot() -> bool {
    !horizon_core::version::STABLE
}

/// The Minecraft session access token (not a Microsoft OAuth token). Never Debug/Serialize.
#[derive(Clone)]
pub struct RealmsAuth {
    pub access_token: String,
    pub uuid: String,
    pub name: String,
}
impl RealmsAuth {
    fn cookie(&self) -> RealmsResult<String> {
        cookie_value(&self.access_token)?;
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(RealmsError::InvalidInput);
        }
        Ok(format!(
            "sid=token:{}:{};user={};version=26.3",
            self.access_token,
            uuid(&self.uuid)?,
            self.name
        ))
    }
}
pub type RealmsResult<T> = Result<T, RealmsError>;
/// Deliberately excludes response bodies, URLs, tokens and transport error strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmsError {
    Unauthorized,
    Forbidden,
    TermsRequired,
    RetryAfter { status: u16, seconds: u64 },
    Service { status: u16, code: Option<i64> },
    Redirect { status: u16 },
    Timeout,
    Cancelled,
    Transport,
    InvalidResponse,
    ResponseTooLarge,
    InvalidInput,
    UnsafeEndpoint,
    WorldNotClosed,
}
impl fmt::Display for RealmsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Realms: {self:?}")
    }
}
impl Error for RealmsError {}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RealmsServerList {
    pub servers: Vec<RealmsServer>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealmsServer {
    pub id: i64,
    pub name: Option<String>,
    pub motd: Option<String>,
    pub owner: Option<String>,
    #[serde(rename = "ownerUUID")]
    pub owner_uuid: Option<String>,
    pub state: Option<String>,
    #[serde(default)]
    pub expired: bool,
    #[serde(default)]
    pub expired_trial: bool,
    #[serde(default)]
    pub days_left: i32,
    #[serde(default)]
    pub active_slot: i32,
    pub world_type: Option<String>,
    pub compatibility: Option<String>,
    pub remote_subscription_id: Option<String>,
    /// Wire-format slot array, including options/settings; no lossy legacy conversion.
    #[serde(default)]
    pub slots: Value,
    #[serde(default)]
    pub players: Value,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinInfo {
    pub address: String,
    pub resource_pack_url: Option<String>,
    pub resource_pack_hash: Option<String>,
    pub region_data: Option<Value>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingInvites {
    pub invites: Vec<PendingInvite>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInvite {
    pub invitation_id: String,
    #[serde(rename = "worldName")]
    pub realm_name: String,
    #[serde(rename = "worldOwnerName")]
    pub realm_owner_name: String,
    #[serde(rename = "worldOwnerUuid")]
    pub realm_owner_uuid: String,
    pub date: Value,
}
// Download URLs and upload tokens may be capabilities: do not derive Debug.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldDownload {
    pub download_link: String,
    pub resource_pack_url: Option<String>,
    pub resource_pack_hash: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadInfo {
    pub world_closed: bool,
    pub token: Option<String>,
    pub upload_endpoint: String,
    /// -1 means use the endpoint's port, as in UploadInfo.selectPortOrDefault.
    pub port: Option<i32>,
}

#[derive(Clone)]
pub struct RealmsClient {
    agent: ureq::Agent,
    origin: String,
    frontend: String,
}
// Small named protocol operations. Value responses preserve evolving service DTOs;
// consumers must validate fields before use. Void endpoints accept empty 2xx bodies.
macro_rules! operations {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $out:ty, $verb:literal, $path:expr, $body:expr;)*) => {
        $(pub fn $name(&self, auth: &RealmsAuth, $($arg: $ty),*) -> RealmsResult<$out> {
            self.json(auth, $verb, &$path, $body)
        })*
    };
}
macro_rules! mutations {
    ($($name:ident($($arg:ident: $ty:ty),*), $verb:literal, $path:expr, $body:expr;)*) => {
        $(pub fn $name(&self, auth: &RealmsAuth, $($arg: $ty),*) -> RealmsResult<()> {
            self.text(auth, $verb, &$path, $body).map(|_| ())
        })*
    };
}
impl Default for RealmsClient {
    fn default() -> Self {
        Self::new()
    }
}
impl RealmsClient {
    /// Constructs an offline client. No requests until an explicit method call.
    pub fn new() -> Self {
        Self {
            agent: agent(true),
            origin: LEGACY.into(),
            frontend: FRONTEND.into(),
        }
    }
    /// Vanilla obtains this flag from the legacy origin, before choosing the AKS origin.
    pub fn refresh_feature_flags(&mut self, auth: &RealmsAuth) -> RealmsResult<Vec<String>> {
        // RealmsClient.fetchFeatureFlags treats both transport and parse failures
        // as an empty set, selecting the legacy service rather than failing availability.
        let flags: Vec<String> = (|| {
            let request = self.request(auth, "GET", &format!("{LEGACY}/feature/v1"), false)?;
            decode(&response_text(request.call())?)
        })()
        .unwrap_or_else(|error: RealmsError| {
            log::warn!("Realms feature flags: {error}");
            Vec::new()
        });
        self.origin = if flags.iter().any(|f| f == "realms_in_aks") {
            AKS
        } else {
            LEGACY
        }
        .into();
        Ok(flags)
    }
    operations! {
        list_realms() -> RealmsServerList, "GET", if is_snapshot() { "worlds/listUserWorldsOfType/any" } else { "worlds" }, None;
        get_own_realm(id: i64) -> RealmsServer, "GET", format!("worlds/{id}"), None;
        join(id: i64) -> JoinInfo, "GET", format!("worlds/v1/{id}/join/pc"), None;
        pending_invites() -> PendingInvites, "GET", "invites/pending", None;
        backups_for(id: i64) -> Value, "GET", format!("worlds/{id}/backups"), None;
        subscription_for(id: i64) -> Value, "GET", format!("subscriptions/{id}"), None;
        tier_configuration(id: i64) -> Value, "GET", format!("worlds/{id}/configuration/tier"), None;
        preferred_regions() -> Value, "GET", "regions/preferredRegions", None;
        live_stats() -> Value, "GET", "activities/liveplayerlist", None;
        notifications() -> Value, "GET", "notifications", None;
        news() -> Value, "GET", "mco/v1/news", None;
        has_parental_consent() -> bool, "GET", "mco/available", None;
        trial_available() -> bool, "GET", "trial", None;
        snapshot_eligible_realms() -> RealmsServerList, "GET", "worlds/listPrereleaseEligibleWorlds", None;
        create_snapshot_realm(parent: i64) -> RealmsServer, "POST", format!("worlds/{parent}/createPrereleaseRealm"), Some(json!(parent));
        invite(id: i64, name: &str) -> RealmsServer, "POST", format!("invites/{id}"), Some(json!({"name":name}));
        open(id: i64) -> bool, "PUT", format!("worlds/{id}/open"), None;
        close(id: i64) -> bool, "PUT", format!("worlds/{id}/close"), None;
        switch_slot(id: i64, slot: i32) -> bool, "PUT", format!("worlds/{id}/slot/{slot}"), None;
        minigame(id: i64, template: i64) -> bool, "PUT", format!("worlds/minigames/{template}/{id}"), None;
        request_download_info(id: i64, slot: i32) -> WorldDownload, "GET", format!("worlds/{id}/slot/{slot}/download"), None;
        templates(page: u32, size: u32, world_type: &str) -> Value, "GET", format!("worlds/templates/{}?page={page}&pageSize={size}", component(world_type)), None;
        op(id: i64, player: &str) -> Value, "POST", format!("ops/{id}/{}", uuid(player)?), None;
        deop(id: i64, player: &str) -> Value, "DELETE", format!("ops/{id}/{}", uuid(player)?), None;
    }
    mutations! {
        initialize(id: i64, name: &str, description: &str), "POST", format!("worlds/{id}/initialize"), Some(json!({"name":name,"description":description}));
        accept_invitation(invitation: &str), "PUT", format!("invites/accept/{}", component(invitation)), None;
        reject_invitation(invitation: &str), "PUT", format!("invites/reject/{}", component(invitation)), None;
        uninvite(id: i64, player: &str), "DELETE", format!("invites/{id}/invite/{}", uuid(player)?), None;
        leave(id: i64), "DELETE", format!("invites/{id}"), None;
        delete_realm(id: i64), "DELETE", format!("worlds/{id}"), None;
        agree_to_tos(), "POST", "mco/tos/agreed", None;
        update_render_distance(id: i64, distance: u32), "PUT", format!("worlds/{id}/configuration/renderDistance/{distance}"), None;
        update_simulation_distance(id: i64, distance: u32), "PUT", format!("worlds/{id}/configuration/simDistance/{distance}"), None;
        send_ping_results(result: &Value), "POST", "regions/ping/stat", Some(result.clone());
    }
    /// Payload: {options: RealmsSlotUpdateDto, settings: [...], regionSelectionPreference,
    /// description:{name,description}}. Region/setting schemas are preserved as JSON.
    pub fn update_configuration(
        &self,
        auth: &RealmsAuth,
        id: i64,
        configuration: &Value,
    ) -> RealmsResult<()> {
        object(configuration)?;
        self.text(
            auth,
            "POST",
            &format!("worlds/{id}/configuration"),
            Some(configuration.clone()),
        )
        .map(|_| ())
    }
    /// 26.3 slot DTO: slotId, spawnProtection, forceGameMode, difficulty, gameMode,
    /// slotName, version, compatibility, templateId, templateImage, hardcore.
    pub fn update_slot(
        &self,
        auth: &RealmsAuth,
        id: i64,
        slot: i32,
        options: &Value,
    ) -> RealmsResult<()> {
        object(options)?;
        if options.get("slotId").and_then(Value::as_i64) != Some(i64::from(slot)) {
            return Err(RealmsError::InvalidInput);
        }
        self.text(
            auth,
            "POST",
            &format!("worlds/{id}/slot/{slot}"),
            Some(options.clone()),
        )
        .map(|_| ())
    }
    pub fn restore_world(&self, auth: &RealmsAuth, id: i64, backup: &str) -> RealmsResult<()> {
        let req = self.request(
            auth,
            "PUT",
            &format!(
                "{}/worlds/{id}/backups?backupId={}",
                self.origin,
                component(backup)
            ),
            false,
        )?;
        response_text(req.timeout(Duration::from_secs(600)).send_string("")).map(|_| ())
    }
    pub fn reset_world_with_template(
        &self,
        auth: &RealmsAuth,
        id: i64,
        template: i64,
    ) -> RealmsResult<bool> {
        let body = json!({"worldTemplateId":template,"levelType":-1,"generateStructures":false,"experiments":[]});
        self.reset_world(auth, id, &body)
    }
    /// Template reset only. Seed generation requires local generation plus upload.
    pub fn reset_world(&self, auth: &RealmsAuth, id: i64, body: &Value) -> RealmsResult<bool> {
        object(body)?;
        if !body
            .get("worldTemplateId")
            .and_then(Value::as_i64)
            .is_some_and(|id| id > 0)
            || body.get("seed").is_some_and(|seed| !seed.is_null())
            || body.get("levelType").and_then(Value::as_i64) != Some(-1)
            || body.get("generateStructures").and_then(Value::as_bool) != Some(false)
            || !body
                .get("experiments")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        {
            return Err(RealmsError::InvalidInput);
        }
        let req = self.request(
            auth,
            "POST",
            &format!("{}/worlds/{id}/reset", self.origin),
            false,
        )?;
        decode(&response_text(
            req.timeout(Duration::from_secs(80))
                .send_string(&body.to_string()),
        )?)
    }
    pub fn client_compatible(&self, auth: &RealmsAuth) -> RealmsResult<String> {
        let value = self.text(auth, "GET", "mco/client/compatible", None)?;
        match value.trim() {
            "COMPATIBLE" | "OUTDATED" | "OTHER" => Ok(value.trim().into()),
            _ => Err(RealmsError::InvalidResponse),
        }
    }
    pub fn notifications_seen(&self, auth: &RealmsAuth, ids: &[String]) -> RealmsResult<()> {
        self.notification_change(auth, "seen", ids)
    }
    pub fn notifications_dismiss(&self, auth: &RealmsAuth, ids: &[String]) -> RealmsResult<()> {
        self.notification_change(auth, "dismiss", ids)
    }
    fn notification_change(
        &self,
        auth: &RealmsAuth,
        action: &str,
        ids: &[String],
    ) -> RealmsResult<()> {
        let ids: Vec<String> = ids
            .iter()
            .map(|s| dashed_uuid(s))
            .collect::<RealmsResult<_>>()?;
        self.text(
            auth,
            "POST",
            &format!("notifications/{action}"),
            Some(json!(ids)),
        )
        .map(|_| ())
    }
    pub fn invite_codes(&self, auth: &RealmsAuth, id: i64) -> RealmsResult<Value> {
        decode(&self.frontend_text(auth, "GET", &format!("?realmId={id}"), None)?)
    }
    pub fn create_invite_code(
        &self,
        auth: &RealmsAuth,
        id: i64,
        enabled: bool,
        expiration_ms: Option<i64>,
    ) -> RealmsResult<()> {
        self.write_invite_code(auth, id, None, enabled, expiration_ms)
    }
    pub fn update_invite_code(
        &self,
        auth: &RealmsAuth,
        id: i64,
        code: &str,
        enabled: bool,
        expiration_ms: Option<i64>,
    ) -> RealmsResult<()> {
        self.write_invite_code(auth, id, Some(code), enabled, expiration_ms)
    }
    fn write_invite_code(
        &self,
        auth: &RealmsAuth,
        id: i64,
        code: Option<&str>,
        enabled: bool,
        expiration: Option<i64>,
    ) -> RealmsResult<()> {
        let mut body = json!({"realmId":id,"enabled":enabled});
        if let Some(code) = code {
            body["code"] = json!(code);
        }
        if let Some(date) = expiration {
            body["expirationDate"] = json!(date);
        }
        self.frontend_text(
            auth,
            "POST",
            if code.is_some() { "/update" } else { "" },
            Some(body),
        )
        .map(|_| ())
    }
    pub fn delete_invite_code(&self, auth: &RealmsAuth, code: &str) -> RealmsResult<()> {
        self.frontend_text(
            auth,
            "DELETE",
            &format!("/delete/{}", component(code)),
            None,
        )
        .map(|_| ())
    }
    pub fn redeem_invite_code(&self, auth: &RealmsAuth, code: &str) -> RealmsResult<()> {
        self.frontend_text(
            auth,
            "POST",
            &format!("/use/{}", component(code.trim())),
            None,
        )
        .map(|_| ())
    }
    pub fn request_upload_info(
        &self,
        auth: &RealmsAuth,
        id: i64,
        previous_token: Option<&str>,
    ) -> RealmsResult<UploadInfo> {
        let body = previous_token.map_or_else(|| json!({}), |token| json!({"token":token}));
        self.json(
            auth,
            "PUT",
            &format!("worlds/{id}/backups/upload"),
            Some(body),
        )
    }
    /// Explicit upload only. Legacy HTTP endpoints are rejected, never upgraded speculatively.
    #[allow(clippy::too_many_arguments)]
    pub fn upload_body(
        &self,
        auth: &RealmsAuth,
        id: i64,
        slot: i32,
        info: &UploadInfo,
        world_version: &str,
        body: impl Read,
        len: u64,
    ) -> RealmsResult<()> {
        self.upload_body_with_cancel(
            auth,
            id,
            slot,
            info,
            world_version,
            body,
            len,
            Arc::default(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn upload_body_with_cancel(
        &self,
        auth: &RealmsAuth,
        id: i64,
        slot: i32,
        info: &UploadInfo,
        world_version: &str,
        body: impl Read,
        len: u64,
        cancel: Arc<AtomicBool>,
    ) -> RealmsResult<()> {
        let guard = TransferGuard::new(cancel);
        guard.check()?;
        if !info.world_closed {
            return Err(RealmsError::WorldNotClosed);
        }
        let token = info.token.as_deref().ok_or(RealmsError::InvalidResponse)?;
        cookie_value(token)?;
        cookie_value(world_version)?;
        let endpoint = self
            .agent
            .get(&info.upload_endpoint)
            .request_url()
            .map_err(|_| RealmsError::UnsafeEndpoint)?;
        let mut url = endpoint.as_url().clone();
        official_transfer(&url)?;
        let port = upload_port(info)?;
        url.set_port(Some(port))
            .map_err(|_| RealmsError::UnsafeEndpoint)?;
        url.set_path(&format!("/upload/{id}/{slot}"));
        url.set_query(None);
        let cookie = format!(
            "{};token={token};worldVersion={world_version}",
            auth.cookie()?
        );
        let request = transfer_agent()
            .post(url.as_str())
            .set("Cookie", &cookie)
            .set("Content-Type", "application/octet-stream")
            .set("Content-Length", &len.to_string());
        guard.check()?;
        let result = request.send(guard.reader(body.take(len)));
        guard.check()?;
        response_text_guarded(result, Some(&guard)).map(|_| ())
    }
    /// Signed download links may target a CDN. Absolutely no session headers/cookies.
    /// The caller must bound bytes, verify hashes and sanitize archives before extraction.
    pub fn download_body(&self, url: &str) -> RealmsResult<Box<dyn Read + Send + Sync>> {
        self.download_body_with_cancel(url, Arc::default())
    }
    pub fn download_body_with_cancel(
        &self,
        url: &str,
        cancel: Arc<AtomicBool>,
    ) -> RealmsResult<Box<dyn Read + Send + Sync>> {
        let guard = TransferGuard::new(cancel);
        guard.check()?;
        let req = transfer_agent().get(url);
        let parsed = req.request_url().map_err(|_| RealmsError::UnsafeEndpoint)?;
        let parsed = parsed.as_url();
        if parsed.scheme() != "https"
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(RealmsError::UnsafeEndpoint);
        }
        guard.check()?;
        let result = req.call();
        guard.check()?;
        let response = response(result)?;
        if response.status() == 277 || !(200..300).contains(&response.status()) {
            return response_text_guarded(Ok(response), Some(&guard))
                .and(Err(RealmsError::InvalidResponse));
        }
        Ok(Box::new(guard.reader(response.into_reader())))
    }
    fn request(
        &self,
        auth: &RealmsAuth,
        method: &str,
        url: &str,
        frontend: bool,
    ) -> RealmsResult<ureq::Request> {
        let mut req = self
            .agent
            .request(method, url)
            .set("Cookie", &auth.cookie()?)
            .set("Client-Ref", "26.3")
            .set("X-NetworkProtocolVersion", "777")
            .set(
                "Is-Prerelease",
                if is_snapshot() { "true" } else { "false" },
            )
            .set("Accept", "application/json")
            .set("Content-Type", "application/json; charset=utf-8");
        if frontend {
            req = req.set("x-mojangauth", &auth.access_token);
        }
        Ok(req)
    }
    fn json<T: DeserializeOwned>(
        &self,
        auth: &RealmsAuth,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> RealmsResult<T> {
        decode(&self.text(auth, method, path, body)?)
    }
    fn text(
        &self,
        auth: &RealmsAuth,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> RealmsResult<String> {
        execute(
            self.request(auth, method, &format!("{}/{path}", self.origin), false)?,
            method,
            body,
        )
    }
    fn frontend_text(
        &self,
        auth: &RealmsAuth,
        method: &str,
        suffix: &str,
        body: Option<Value>,
    ) -> RealmsResult<String> {
        execute(
            self.request(
                auth,
                method,
                &format!("{}/api/v1.0/invite/codes{suffix}", self.frontend),
                true,
            )?,
            method,
            body,
        )
    }
}
/// Returns a link only; never opens a browser or purchases a subscription.
pub fn extend_subscription_link(
    subscription: &str,
    profile: &str,
    expired_trial: bool,
) -> RealmsResult<String> {
    Ok(format!(
        "https://aka.ms/ExtendJavaRealms?subscriptionId={}&profileId={}&ref={}",
        component(subscription),
        uuid(profile)?,
        if expired_trial {
            "expiredTrial"
        } else {
            "expiredRealm"
        }
    ))
}
// Cooperative deadline: DNS, TLS and response-header parsing cannot be interrupted here.
#[derive(Clone)]
struct TransferGuard {
    cancel: Arc<AtomicBool>,
    deadline: Instant,
}
impl TransferGuard {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel,
            deadline: Instant::now() + Duration::from_secs(600),
        }
    }
    fn check(&self) -> RealmsResult<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(RealmsError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(RealmsError::Timeout)
        } else {
            Ok(())
        }
    }
    fn reader<R: Read>(&self, inner: R) -> TransferReader<R> {
        TransferReader {
            inner,
            guard: self.clone(),
        }
    }
}
struct TransferReader<R> {
    inner: R,
    guard: TransferGuard,
}
impl<R: Read> Read for TransferReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let check = || {
            self.guard.check().map_err(|error| {
                let kind = if error == RealmsError::Timeout {
                    io::ErrorKind::TimedOut
                } else {
                    io::ErrorKind::Other
                };
                io::Error::new(kind, error)
            })
        };
        check()?;
        let result = self.inner.read(buffer);
        check()?;
        result.map_err(|error| {
            if timed_out(&error) {
                io::Error::new(io::ErrorKind::TimedOut, RealmsError::Timeout)
            } else {
                io::Error::other(RealmsError::Transport)
            }
        })
    }
}
fn transfer_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .https_only(true)
        .redirects(0)
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        // ureq 2 clears socket timeouts when returning a connection to its pool.
        .max_idle_connections(0)
        .build()
}
fn upload_port(info: &UploadInfo) -> RealmsResult<u16> {
    // URL normalization removes explicit :443; Java URI preserves it. Inspect the
    // raw authority only after official_transfer validated the parsed HTTPS URL.
    let authority = info
        .upload_endpoint
        .split_once("://")
        .ok_or(RealmsError::UnsafeEndpoint)?
        .1
        .split(['/', '?', '#'])
        .next()
        .ok_or(RealmsError::UnsafeEndpoint)?;
    if authority.contains(['\\', '@']) || authority.chars().any(char::is_control) {
        return Err(RealmsError::UnsafeEndpoint);
    }
    let explicit = authority
        .rsplit_once(':')
        .map(|(_, port)| {
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err(RealmsError::UnsafeEndpoint);
            }
            port.parse::<u16>().map_err(|_| RealmsError::UnsafeEndpoint)
        })
        .transpose()?;
    let port = info
        .port
        .filter(|port| *port != -1)
        .unwrap_or(i32::from(explicit.unwrap_or(8080)));
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or(RealmsError::UnsafeEndpoint)
}
fn agent(tls: bool) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .https_only(tls)
        .redirects(0)
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .timeout_write(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
}
fn execute(req: ureq::Request, method: &str, body: Option<Value>) -> RealmsResult<String> {
    response_text(if matches!(method, "POST" | "PUT") {
        req.send_string(&body.map(|v| v.to_string()).unwrap_or_default())
    } else {
        req.call()
    })
}
fn response(result: Result<ureq::Response, ureq::Error>) -> RealmsResult<ureq::Response> {
    match result {
        Ok(r) | Err(ureq::Error::Status(_, r)) => Ok(r),
        Err(ureq::Error::Transport(error)) => Err(if timed_out(&error) {
            RealmsError::Timeout
        } else {
            RealmsError::Transport
        }),
    }
}
fn response_text(result: Result<ureq::Response, ureq::Error>) -> RealmsResult<String> {
    response_text_guarded(result, None)
}
fn response_text_guarded(
    result: Result<ureq::Response, ureq::Error>,
    guard: Option<&TransferGuard>,
) -> RealmsResult<String> {
    if let Some(guard) = guard {
        guard.check()?;
    }
    let response = response(result)?;
    let status = response.status();
    if matches!(status, 503 | 277 | 429) {
        let seconds = response
            .header("Retry-After")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|s| *s <= 120)
            .unwrap_or(5);
        return Err(RealmsError::RetryAfter { status, seconds });
    }
    if (300..400).contains(&status) {
        return Err(RealmsError::Redirect { status });
    }
    let mut body = Vec::new();
    let reader = response.into_reader();
    let reader: Box<dyn Read + Send + Sync> = match guard {
        Some(guard) => Box::new(guard.reader(reader)),
        None => reader,
    };
    let result = reader.take(MAX_RESPONSE + 1).read_to_end(&mut body);
    if let Some(guard) = guard {
        guard.check()?;
    }
    result.map_err(|e| {
        if timed_out(&e) {
            RealmsError::Timeout
        } else {
            RealmsError::Transport
        }
    })?;
    if body.len() as u64 > MAX_RESPONSE {
        return Err(RealmsError::ResponseTooLarge);
    }
    if !(200..300).contains(&status) {
        let code = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|v| v.get("errorCode").and_then(Value::as_i64));
        return Err(match (status, code) {
            (_, Some(6002)) => RealmsError::TermsRequired,
            (401, _) => RealmsError::Unauthorized,
            (403, _) => RealmsError::Forbidden,
            _ => RealmsError::Service { status, code },
        });
    }
    String::from_utf8(body).map_err(|_| RealmsError::InvalidResponse)
}
fn timed_out(mut error: &(dyn Error + 'static)) -> bool {
    loop {
        if error.downcast_ref::<std::io::Error>().is_some_and(|e| {
            matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            )
        }) {
            return true;
        }
        match error.source() {
            Some(source) => error = source,
            None => return false,
        }
    }
}
fn decode<T: DeserializeOwned>(text: &str) -> RealmsResult<T> {
    serde_json::from_str(text).map_err(|_| RealmsError::InvalidResponse)
}
fn cookie_value(value: &str) -> RealmsResult<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b';' | b',' | b'"' | b'\\'))
    {
        return Err(RealmsError::InvalidInput);
    }
    Ok(())
}
fn uuid(value: &str) -> RealmsResult<String> {
    if value.len() != 32
        && !(value.len() == 36 && [8, 13, 18, 23].iter().all(|&i| value.as_bytes()[i] == b'-'))
    {
        return Err(RealmsError::InvalidInput);
    }
    let compact = value.replace('-', "");
    if compact.len() != 32 || !compact.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(RealmsError::InvalidInput);
    }
    Ok(compact.to_ascii_lowercase())
}
fn dashed_uuid(value: &str) -> RealmsResult<String> {
    let s = uuid(value)?;
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    ))
}
fn object(value: &Value) -> RealmsResult<()> {
    if value.is_object() {
        Ok(())
    } else {
        Err(RealmsError::InvalidInput)
    }
}
fn component(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
// Use ureq's parsed URL without adding a second URL dependency.
fn official_transfer(url: &impl fmt::Display) -> RealmsResult<()> {
    let parsed = ureq::get(&url.to_string())
        .request_url()
        .map_err(|_| RealmsError::UnsafeEndpoint)?;
    let url = parsed.as_url();
    let host = url.host_str().ok_or(RealmsError::UnsafeEndpoint)?;
    let official = ["realms.minecraft.net", "realms.minecraft-services.net"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    if !official
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(RealmsError::UnsafeEndpoint);
    }
    Ok(())
}
