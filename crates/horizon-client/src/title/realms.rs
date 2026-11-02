//! Realms screens and credential-free UI state. Network work belongs to the caller.
//! Layout reference: 26.3 RealmsMainScreen.createFooter (100px, three columns,
//! 4px spacing, 44px header) and the configure/backup/terms screens.
use super::{Action, Button, Mode, Title};
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    lang,
    text::Text,
};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum RealmsAction {
    Refresh,
    CancelRequest,
    ConfirmJoinPack(bool),
    Initialize {
        id: i64,
        name: String,
        description: String,
    },
    TierConfiguration(i64),
    SetRenderDistance {
        id: i64,
        distance: u32,
    },
    SetSimulationDistance {
        id: i64,
        distance: u32,
    },
    SetRegion {
        id: i64,
        preference: Value,
    },
    Join(i64),
    Invites,
    AcceptInvite(String),
    RejectInvite(String),
    Configure(i64),
    SaveConfig {
        id: i64,
        name: String,
        description: String,
    },
    Slots(i64),
    SwitchSlot {
        id: i64,
        slot: i32,
    },
    SaveSlot {
        id: i64,
        slot: i32,
        options: Value,
    },
    Backups {
        id: i64,
        slot: i32,
    },
    RestoreBackup {
        id: i64,
        backup_id: String,
    },
    Upload {
        id: i64,
        slot: i32,
        folder: PathBuf,
    },
    Download {
        id: i64,
        slot: i32,
    },
    Subscription(i64),
    OpenSubscription(i64),
    Purchase,
    OpenTerms,
    AcceptTerms,
    OpenWorld(i64),
    CloseWorld(i64),
    Leave(i64),
    Players(i64),
    InvitePlayer {
        id: i64,
        name: String,
    },
    RemovePlayer {
        id: i64,
        uuid: String,
    },
    SetOperator {
        id: i64,
        uuid: String,
        operator: bool,
    },
    Reset {
        id: i64,
        options: Value,
    },
    Templates {
        world_type: String,
        page: i32,
    },
    SwitchMinigame {
        id: i64,
        template_id: String,
    },
    InviteLink(i64),
    RedeemInvite(String),
    MarkNotificationsSeen(Vec<String>),
    DismissNotification(String),
    CreateInviteCode {
        id: i64,
    },
    DeleteInviteCode {
        code: String,
    },
    UpdateInviteCode {
        id: i64,
        code: String,
        enabled: bool,
        expiration_ms: Option<i64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    List,
    Invites,
    Configure,
    Settings,
    Slots,
    SlotOptions,
    Backups,
    Upload,
    Subscription,
    Players,
    InvitePlayer,
    Reset,
    Templates,
    InviteLink,
    Redeem,
    Terms,
    Confirm,
    Notifications,
    Initialize,
    Distances,
    Region,
    ResourcePack,
}

#[derive(Clone, Debug)]
struct Field {
    key: &'static str,
    value: String,
    max: usize,
}
#[derive(Clone, Debug)]
struct Confirmation {
    action: Action,
    key: &'static str,
    detail: String,
    previous: Page,
}
#[derive(Clone, Debug)]
pub struct State {
    pub eligible: bool,
    pub profile_uuid: Option<String>,
    pub page: Page,
    pub loading: bool,
    pub error: Option<String>,
    pub status: Option<String>,
    pub servers: Vec<Value>,
    pub invites: Vec<Value>,
    pub notifications: Vec<Value>,
    pub server: Value,
    pub backups: Vec<Value>,
    pub subscription: Value,
    pub templates: Vec<Value>,
    pub invite_links: Value,
    pub selected: Option<usize>,
    pub slot: i32,
    pub template_type: String,
    pub template_page: i32,
    pub local_worlds: Vec<(String, PathBuf)>,
    pub tier_configuration: Value,
    pub regions: Value,
    offset: usize,
    fields: Vec<Field>,
    focused: Option<usize>,
    confirmation: Option<Confirmation>,
    slot_options: Value,
    terms_return: Page,
}

#[derive(Clone)]
enum Event {
    Back,
    Page(Page),
    Request(RealmsAction),
    Confirm(RealmsAction, &'static str, String),
    SaveConfig,
    SaveSlot,
    InvitePlayer,
    Redeem,
    Reset,
    Toggle(&'static str),
    Cycle(&'static str, i64),
    AcceptConfirmation,
    Copy(String),
    Link(String),
    TemplateType,
    Initialize,
    Distance(bool, i64),
}
struct Control {
    label: String,
    enabled: bool,
    event: Event,
}
// RealmsRegion nameId / translation-key suffix, verified against 26.3 bytecode.
const REGIONS: &[(&str, &str)] = &[
    ("AustraliaEast", "australia_east"),
    ("AustraliaSoutheast", "australia_southeast"),
    ("BrazilSouth", "brazil_south"),
    ("CanadaCentral", "canada_central"),
    ("CentralIndia", "central_india"),
    ("CentralUs", "central_us"),
    ("EastAsia", "east_asia"),
    ("EastUs", "east_us"),
    ("EastUs2", "east_us_2"),
    ("FranceCentral", "france_central"),
    ("JapanEast", "japan_east"),
    ("JapanWest", "japan_west"),
    ("KoreaCentral", "korea_central"),
    ("MexicoCentral", "mexico_central"),
    ("NorthCentralUs", "north_central_us"),
    ("NorthEurope", "north_europe"),
    ("SouthCentralUs", "south_central_us"),
    ("SoutheastAsia", "southeast_asia"),
    ("SouthAfricaNorth", "south_africa_north"),
    ("SwedenCentral", "sweden_central"),
    ("UAENorth", "uae_north"),
    ("UKSouth", "uk_south"),
    ("WestCentralUs", "west_central_us"),
    ("WestEurope", "west_europe"),
    ("WestUs", "west_us"),
    ("WestUs2", "west_us_2"),
    ("WestUs3", "west_us_3"),
];
fn valid_link(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url.chars().any(char::is_control)
}
fn tr(key: &str) -> String {
    lang::get(key)
}
fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn number(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_default()
}
fn array(value: Value, key: &str) -> Vec<Value> {
    value
        .as_array()
        .or_else(|| value.get(key).and_then(Value::as_array))
        .cloned()
        .unwrap_or_default()
}
fn translated(key: &str, args: &[String]) -> String {
    crate::text::format_translation(
        &tr(key),
        &args.iter().map(Text::literal).collect::<Vec<_>>(),
    )
    .plain()
}
fn normalized(uuid: &str) -> String {
    uuid.replace('-', "").to_ascii_lowercase()
}
fn notification_text(value: &Value) -> String {
    let message = value.get("message").unwrap_or(&Value::Null);
    let key = message
        .get("translationKey")
        .and_then(Value::as_str)
        .unwrap_or("mco.notification.visitUrl.message.default");
    let args = message
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|v| v.as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    translated(key, &args)
}

impl State {
    pub fn new(
        eligible: bool,
        profile_uuid: Option<String>,
        local_worlds: Vec<(String, PathBuf)>,
    ) -> Self {
        Self {
            eligible,
            profile_uuid,
            page: Page::List,
            loading: false,
            error: None,
            status: None,
            servers: vec![],
            invites: vec![],
            notifications: vec![],
            server: Value::Null,
            backups: vec![],
            subscription: Value::Null,
            templates: vec![],
            invite_links: Value::Null,
            selected: None,
            slot: 1,
            template_type: "WORLD_TEMPLATE".into(),
            template_page: 1,
            local_worlds,
            tier_configuration: Value::Null,
            regions: Value::Null,
            offset: 0,
            fields: vec![],
            focused: None,
            confirmation: None,
            slot_options: json!({}),
            terms_return: Page::List,
        }
    }
    pub fn set_tier_configuration(&mut self, value: Value) {
        self.tier_configuration = value;
        self.loading = false;
    }
    pub fn set_regions(&mut self, value: Value) {
        self.regions = value;
    }
    /// RealmsSettingsTab.currentOrDefault and IntRange use server-provided bounds.
    fn distance(&self, simulation: bool) -> Option<(u32, u32, u32)> {
        let range = self.tier_configuration.get(if simulation {
            "simDistance"
        } else {
            "renderDistance"
        })?;
        let min = u32::try_from(range.get("min")?.as_u64()?).ok()?;
        let max = u32::try_from(range.get("max")?.as_u64()?).ok()?;
        let current = range
            .get("current")
            .and_then(Value::as_u64)
            .or_else(|| range.get("defaultValue").and_then(Value::as_u64))?;
        (min <= max).then(|| (min, max, current.clamp(min as u64, max as u64) as u32))
    }
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
        if loading {
            self.error = None;
            self.status = None;
        }
    }
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.loading = false;
        self.error = Some(message.into());
    }
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.loading = false;
        self.status = Some(message.into());
    }
    pub fn set_servers(&mut self, value: Value) {
        let id = self
            .selected
            .and_then(|i| self.servers.get(i))
            .map(|s| number(s, "id"));
        self.servers = array(value, "servers");
        self.loading = false;
        self.selected = id.and_then(|id| self.servers.iter().position(|s| number(s, "id") == id));
        self.offset = 0;
    }
    pub fn set_invites(&mut self, value: Value) {
        self.invites = array(value, "invites");
        self.loading = false;
        self.selected = None;
    }
    pub fn set_server(&mut self, value: Value) {
        self.server = value;
        self.loading = false;
        if self.page == Page::Configure {
            self.slot = number(&self.server, "activeSlot").clamp(1, 3) as i32;
        }
        if self.page == Page::Configure && text(&self.server, "state") == "UNINITIALIZED" {
            self.enter(Page::Initialize);
        } else if matches!(self.page, Page::Settings) {
            self.enter(Page::Settings);
        }
    }
    pub fn set_backups(&mut self, value: Value) {
        self.backups = array(value, "backups");
        self.loading = false;
        self.selected = None;
    }
    pub fn set_subscription(&mut self, value: Value) {
        self.subscription = value;
        self.loading = false;
    }
    pub fn set_templates(&mut self, value: Value) {
        self.templates = array(value, "templates");
        self.loading = false;
        self.selected = None;
        self.offset = 0;
    }
    pub fn set_invite_links(&mut self, value: Value) {
        self.invite_links = value;
        self.loading = false;
    }
    pub fn set_notifications(&mut self, value: Value) {
        self.notifications = array(value, "notifications");
    }
    pub fn return_page(&mut self, page: Page) {
        self.enter(page);
        self.loading = false;
    }
    pub fn require_resource_pack(&mut self) {
        self.loading = false;
        self.enter(Page::ResourcePack);
    }
    pub fn require_terms(&mut self) {
        self.loading = false;
        self.error = None;
        self.terms_return = self.page.clone();
        self.enter(Page::Terms);
    }
    pub fn terms_accepted(&mut self) {
        let page = self.terms_return.clone();
        self.enter(page);
        self.loading = false;
    }
    pub fn id(&self) -> i64 {
        number(&self.server, "id")
    }
    fn owned(&self, server: &Value) -> bool {
        self.profile_uuid.as_ref().is_some_and(|uuid| {
            !uuid.is_empty() && normalized(uuid) == normalized(&text(server, "ownerUUID"))
        })
    }
    fn editable(&self) -> bool {
        self.owned(&self.server)
            && self.server.get("expired").and_then(Value::as_bool) != Some(true)
    }
    fn field(&self, index: usize) -> String {
        self.fields
            .get(index)
            .map(|f| f.value.trim().to_owned())
            .unwrap_or_default()
    }
    fn add_field(&mut self, key: &'static str, value: String, max: usize) {
        self.fields.push(Field { key, value, max });
    }
    fn enter(&mut self, page: Page) {
        self.page = page;
        self.selected = None;
        self.offset = 0;
        self.focused = None;
        self.fields.clear();
        self.error = None;
        self.status = None;
        match self.page {
            Page::Settings | Page::Initialize => {
                self.add_field("mco.configure.world.name", text(&self.server, "name"), 32);
                self.add_field(
                    "mco.configure.world.description",
                    text(&self.server, "motd"),
                    256,
                );
            }
            Page::SlotOptions => {
                let options = self
                    .server
                    .get("slots")
                    .and_then(Value::as_array)
                    .and_then(|slots| {
                        slots
                            .iter()
                            .find(|s| number(s, "slotId") == self.slot as i64)
                    })
                    .and_then(|s| s.get("options"))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                self.slot_options = if let Some(s) = options.as_str() {
                    serde_json::from_str(s).unwrap_or_else(|_| json!({}))
                } else {
                    options
                };
                if !self.slot_options.is_object() {
                    self.slot_options = json!({});
                }
                self.add_field(
                    "mco.configure.world.edit.slot.name",
                    text(&self.slot_options, "slotName"),
                    32,
                );
            }
            Page::InvitePlayer => {
                self.add_field("mco.configure.world.invite.profile.name", String::new(), 16)
            }
            Page::Redeem => self.add_field("mco.selectServer.joinCode.hint", String::new(), 128),
            Page::Reset => self.add_field("mco.reset.world.seed", String::new(), 64),
            _ => {}
        }
        if !self.fields.is_empty() {
            self.focused = Some(0);
        }
    }
    fn ask(&mut self, action: RealmsAction, key: &'static str, detail: String) {
        self.ask_action(Action::Realms(action), key, detail);
    }
    fn ask_action(&mut self, action: Action, key: &'static str, detail: String) {
        self.confirmation = Some(Confirmation {
            action,
            key,
            detail,
            previous: self.page.clone(),
        });
        self.enter(Page::Confirm);
    }
    /// Returns true when Back should leave Realms. Loading requests must finish first.
    pub fn back(&mut self) -> bool {
        if self.loading {
            return false;
        }
        let page = match self.page {
            Page::List => return true,
            Page::Confirm => self
                .confirmation
                .take()
                .map(|c| c.previous)
                .unwrap_or(Page::List),
            Page::Terms => self.terms_return.clone(),
            Page::Configure
            | Page::Invites
            | Page::Redeem
            | Page::Notifications
            | Page::Initialize => Page::List,
            Page::Region => Page::Distances,
            Page::SlotOptions | Page::Backups | Page::Upload | Page::Reset => Page::Slots,
            Page::Templates if self.template_type == "MINIGAME" => Page::Configure,
            Page::Templates => Page::Reset,
            Page::InvitePlayer => Page::Players,
            _ => Page::Configure,
        };
        self.enter(page);
        false
    }
    pub fn title(&self) -> String {
        tr(match self.page {
            Page::Notifications => "mco.news",
            Page::Initialize => "mco.create.world",
            Page::Distances => "mco.configure.world.settings.title",
            Page::Region => "mco.configure.world.region_preference.title",
            Page::ResourcePack => "mco.warning",
            Page::List => "menu.online",
            Page::Invites => "mco.invites.title",
            Page::Configure => "mco.configure.world.title",
            Page::Settings => "mco.configure.world.settings.title",
            Page::Slots => "mco.configure.worlds.title",
            Page::SlotOptions => "mco.configure.world.buttons.options",
            Page::Backups => "mco.configure.world.backup",
            Page::Upload => "mco.upload.select.world.title",
            Page::Subscription => "mco.configure.world.subscription.title",
            Page::Players => "mco.configure.world.players.title",
            Page::InvitePlayer => "mco.configure.world.buttons.invite",
            Page::Reset => "mco.reset.world.title",
            Page::Templates => "mco.template.title",
            Page::InviteLink => "mco.configure.world.buttons.invite_codes",
            Page::Redeem => "mco.selectServer.joinCode.title",
            Page::Terms => "mco.terms.title",
            Page::Confirm => "mco.warning",
        })
    }
    fn rows(&self) -> Vec<(String, String)> {
        match self.page {
            Page::Region => REGIONS
                .iter()
                .map(|(_, key)| {
                    (
                        tr(&format!("realms.configuration.region.{key}")),
                        String::new(),
                    )
                })
                .collect(),
            Page::List => self
                .servers
                .iter()
                .map(|s| {
                    let status = if s.get("expired").and_then(Value::as_bool) == Some(true) {
                        "mco.selectServer.expired"
                    } else if text(s, "state") == "OPEN" {
                        "mco.selectServer.open"
                    } else {
                        "mco.selectServer.closed"
                    };
                    (
                        text(s, "name"),
                        format!(
                            "{} · {} · {}",
                            text(s, "owner"),
                            text(s, "motd"),
                            tr(status)
                        ),
                    )
                })
                .collect(),
            Page::InviteLink => self
                .invite_links
                .get("inviteCodes")
                .and_then(Value::as_array)
                .map(|codes| {
                    codes
                        .iter()
                        .map(|c| {
                            (
                                text(c, "code"),
                                format!(
                                    "{} · {}",
                                    tr(
                                        if c.get("enabled").and_then(Value::as_bool) == Some(true) {
                                            "options.on"
                                        } else {
                                            "options.off"
                                        }
                                    ),
                                    c.get("expirationDate")
                                        .filter(|v| !v.is_null())
                                        .map(Value::to_string)
                                        .unwrap_or_else(|| tr(
                                            "mco.configure.world.invite_codes.expiration.never"
                                        ))
                                ),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Page::Notifications => self
                .notifications
                .iter()
                .map(|n| (notification_text(n), text(n, "url")))
                .collect(),
            Page::Invites => self
                .invites
                .iter()
                .map(|s| (text(s, "worldName"), text(s, "worldOwnerName")))
                .collect(),
            Page::Backups => self
                .backups
                .iter()
                .map(|b| {
                    (
                        translated(
                            "mco.backup.entry",
                            &[b.get("lastModifiedDate")
                                .map(Value::to_string)
                                .unwrap_or_default()],
                        ),
                        text(b, "backupId"),
                    )
                })
                .collect(),
            Page::Upload => self
                .local_worlds
                .iter()
                .map(|(name, folder)| {
                    (
                        name.clone(),
                        folder
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                    )
                })
                .collect(),
            Page::Players => self
                .server
                .get("players")
                .and_then(Value::as_array)
                .map(|players| {
                    players
                        .iter()
                        .map(|p| {
                            (
                                text(p, "name"),
                                tr(
                                    if p.get("operator").and_then(Value::as_bool) == Some(true) {
                                        "mco.configure.world.invites.ops.tooltip"
                                    } else {
                                        "mco.configure.world.invites.normal.tooltip"
                                    },
                                ),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Page::Templates => self
                .templates
                .iter()
                .map(|t| (text(t, "name"), text(t, "author")))
                .collect(),
            Page::Slots => (1..=3)
                .map(|slot| {
                    (
                        translated("mco.configure.world.slot", &[slot.to_string()]),
                        if slot == number(&self.server, "activeSlot") {
                            tr("mco.configure.current.minigame")
                        } else {
                            String::new()
                        },
                    )
                })
                .collect(),
            _ => vec![],
        }
    }
    fn controls(&self) -> Vec<Control> {
        if self.loading {
            return vec![Control {
                label: tr("gui.cancel"),
                enabled: true,
                event: Event::Request(RealmsAction::CancelRequest),
            }];
        }
        let mut controls = vec![];
        macro_rules! add {
            ($key:expr, $enabled:expr, $event:expr) => {
                controls.push(Control {
                    label: tr($key),
                    enabled: $enabled && !self.loading,
                    event: $event,
                })
            };
        }
        if !self.eligible {
            add!("gui.back", true, Event::Back);
            return controls;
        }
        let id = self.id();
        let slot = self.slot;
        let editable = self.editable();
        match self.page {
            Page::ResourcePack => {
                add!(
                    "gui.yes",
                    true,
                    Event::Request(RealmsAction::ConfirmJoinPack(true))
                );
                add!(
                    "gui.no",
                    true,
                    Event::Request(RealmsAction::ConfirmJoinPack(false))
                );
                return controls;
            }
            Page::Initialize => add!(
                "mco.create.world",
                editable && !self.field(0).is_empty(),
                Event::Initialize
            ),
            Page::Distances => {
                for (simulation, key) in [
                    (false, "mco.configure.world.renderDistance"),
                    (true, "mco.configure.world.simulationDistance"),
                ] {
                    let bounds = self.distance(simulation);
                    for delta in [-1, 1] {
                        let available = bounds.is_some_and(|(min, max, current)| {
                            if delta < 0 {
                                current > min
                            } else {
                                current < max
                            }
                        });
                        controls.push(Control {
                            label: format!(
                                "{} {} {}",
                                tr(key),
                                bounds.map(|(_, _, v)| v.to_string()).unwrap_or_default(),
                                if delta < 0 { "−" } else { "+" }
                            ),
                            enabled: editable && available,
                            event: Event::Distance(simulation, delta),
                        });
                    }
                }
                add!(
                    "mco.configure.world.buttons.region_preference",
                    editable,
                    Event::Page(Page::Region)
                );
                add!(
                    "selectServer.refresh",
                    editable,
                    Event::Request(RealmsAction::TierConfiguration(id))
                );
            }
            Page::Region => {
                for (preference, key) in [
                    (0, "realms.configuration.region_preference.automatic_player"),
                    (1, "realms.configuration.region_preference.automatic_owner"),
                ] {
                    add!(
                        key,
                        editable,
                        Event::Request(RealmsAction::SetRegion {
                            id,
                            preference: json!({"regionSelectionPreference":preference,"preferredRegion":null})
                        })
                    );
                }
                let region = self.selected.and_then(|i| REGIONS.get(i));
                add!(
                    "mco.configure.world.buttons.save",
                    editable && region.is_some(),
                    Event::Request(RealmsAction::SetRegion {
                        id,
                        preference: json!({"regionSelectionPreference":2,"preferredRegion":region.map(|r|r.0)})
                    })
                );
            }
            Page::List => {
                let server = self.selected.and_then(|i| self.servers.get(i));
                let sid = server.map(|s| number(s, "id")).unwrap_or_default();
                let owner = server.is_some_and(|s| self.owned(s));
                let playable = server.is_some_and(|s| {
                    text(s, "state") == "OPEN"
                        && s.get("expired").and_then(Value::as_bool) != Some(true)
                });
                add!(
                    "mco.selectServer.play",
                    playable,
                    Event::Request(RealmsAction::Join(sid))
                );
                add!(
                    "mco.selectServer.configure",
                    owner,
                    Event::Request(RealmsAction::Configure(sid))
                );
                add!(
                    "mco.selectServer.leave",
                    server.is_some() && !owner,
                    Event::Confirm(
                        RealmsAction::Leave(sid),
                        "mco.configure.world.leave.question.line1",
                        server.map(|s| text(s, "name")).unwrap_or_default()
                    )
                );
                add!(
                    "mco.invites.title",
                    true,
                    Event::Request(RealmsAction::Invites)
                );
                add!("mco.news", true, Event::Page(Page::Notifications));
                add!("mco.selectServer.joinCode", true, Event::Page(Page::Redeem));
                add!(
                    "mco.selectServer.purchase",
                    true,
                    Event::Confirm(RealmsAction::Purchase, "chat.link.confirm", String::new())
                );
                add!(
                    "selectServer.refresh",
                    true,
                    Event::Request(RealmsAction::Refresh)
                );
            }
            Page::Notifications => {
                let notification = self.selected.and_then(|i| self.notifications.get(i));
                let uuid = notification
                    .map(|n| text(n, "notificationUuid"))
                    .unwrap_or_default();
                add!(
                    "mco.notification.dismiss",
                    notification.is_some_and(
                        |n| n.get("dismissable").and_then(Value::as_bool) == Some(true)
                    ),
                    Event::Request(RealmsAction::DismissNotification(uuid))
                );
                let url = notification.map(|n| text(n, "url")).unwrap_or_default();
                add!(
                    "mco.notification.visitUrl.buttonText.default",
                    valid_link(&url),
                    Event::Link(url)
                );
                let unseen = self
                    .notifications
                    .iter()
                    .filter(|n| n.get("seen").and_then(Value::as_bool) != Some(true))
                    .map(|n| text(n, "notificationUuid"))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>();
                add!(
                    "gui.done",
                    !unseen.is_empty(),
                    Event::Request(RealmsAction::MarkNotificationsSeen(unseen))
                );
            }
            Page::Invites => {
                let invite = self.selected.and_then(|i| self.invites.get(i));
                let iid = invite.map(|i| text(i, "invitationId")).unwrap_or_default();
                add!(
                    "mco.invites.button.accept",
                    invite.is_some(),
                    Event::Request(RealmsAction::AcceptInvite(iid.clone()))
                );
                add!(
                    "mco.invites.button.reject",
                    invite.is_some(),
                    Event::Confirm(
                        RealmsAction::RejectInvite(iid),
                        "mco.invites.button.reject",
                        invite.map(|i| text(i, "worldName")).unwrap_or_default()
                    )
                );
                add!(
                    "selectServer.refresh",
                    true,
                    Event::Request(RealmsAction::Invites)
                );
            }
            Page::Configure => {
                add!(
                    "mco.configure.world.buttons.settings",
                    editable,
                    Event::Page(Page::Settings)
                );
                add!(
                    "mco.configure.worlds.title",
                    self.owned(&self.server),
                    Event::Page(Page::Slots)
                );
                add!(
                    "mco.configure.world.buttons.players",
                    editable,
                    Event::Request(RealmsAction::Players(id))
                );
                add!(
                    "mco.configure.world.buttons.subscription",
                    self.owned(&self.server),
                    Event::Request(RealmsAction::Subscription(id))
                );
                if text(&self.server, "state") == "OPEN" {
                    add!(
                        "mco.configure.world.buttons.close",
                        editable,
                        Event::Confirm(
                            RealmsAction::CloseWorld(id),
                            "mco.configure.world.close.question.line1",
                            String::new()
                        )
                    );
                } else {
                    add!(
                        "mco.configure.world.buttons.open",
                        editable,
                        Event::Request(RealmsAction::OpenWorld(id))
                    );
                }
                add!(
                    "mco.configure.world.buttons.invite_codes",
                    editable,
                    Event::Request(RealmsAction::InviteLink(id))
                );
                add!(
                    "mco.configure.world.buttons.switchminigame",
                    editable,
                    Event::Request(RealmsAction::Templates {
                        world_type: "MINIGAME".into(),
                        page: 1
                    })
                );
            }
            Page::Settings => {
                add!(
                    "mco.configure.world.buttons.save",
                    editable && !self.field(0).is_empty(),
                    Event::SaveConfig
                );
                add!(
                    "mco.configure.world.renderDistance",
                    editable,
                    Event::Request(RealmsAction::TierConfiguration(id))
                );
            }
            Page::Slots => {
                let chosen = self.selected.map(|i| i as i32 + 1).unwrap_or(slot);
                add!(
                    "mco.configure.world.slot.tooltip",
                    editable && chosen != number(&self.server, "activeSlot") as i32,
                    Event::Confirm(
                        RealmsAction::SwitchSlot { id, slot: chosen },
                        "mco.configure.world.slot.switch.question.line1",
                        chosen.to_string()
                    )
                );
                add!(
                    "mco.configure.world.buttons.options",
                    editable,
                    Event::Page(Page::SlotOptions)
                );
                add!(
                    "mco.configure.world.backup",
                    self.owned(&self.server),
                    Event::Request(RealmsAction::Backups { id, slot: chosen })
                );
                add!(
                    "mco.backup.button.upload",
                    editable,
                    Event::Page(Page::Upload)
                );
                add!(
                    "mco.backup.button.download",
                    self.owned(&self.server),
                    Event::Confirm(
                        RealmsAction::Download { id, slot: chosen },
                        "mco.configure.world.restore.download.question.line1",
                        chosen.to_string()
                    )
                );
                // The reset service acts on the active slot, so never silently reset a different world.
                add!(
                    "mco.backup.button.reset",
                    editable && chosen == number(&self.server, "activeSlot") as i32,
                    Event::Page(Page::Reset)
                );
            }
            Page::SlotOptions => {
                {
                    let (key, label, default) =
                        ("forceGameMode", "mco.configure.world.forceGameMode", false);
                    let value = self
                        .slot_options
                        .get(key)
                        .and_then(Value::as_bool)
                        .unwrap_or(default);
                    add!(label, editable, Event::Toggle(key));
                    let c = controls.last_mut().unwrap();
                    c.label = format!(
                        "{}: {}",
                        c.label,
                        tr(if value { "options.on" } else { "options.off" })
                    );
                }
                // Numeric options use the same bounded values as RealmsWorldOptions.
                for (key, label, limit) in [
                    ("difficulty", "options.difficulty", 4),
                    ("gameMode", "selectWorld.gameMode", 3),
                    ("spawnProtection", "mco.configure.world.spawnProtection", 17),
                ] {
                    controls.push(Control {
                        label: format!("{}: {}", tr(label), number(&self.slot_options, key)),
                        enabled: editable && !self.loading,
                        event: Event::Cycle(key, limit),
                    });
                }
                controls.push(Control {
                    label: tr("mco.configure.world.buttons.save"),
                    enabled: editable && !self.loading,
                    event: Event::SaveSlot,
                });
            }
            Page::Backups => {
                let backup = self.selected.and_then(|i| self.backups.get(i));
                add!(
                    "mco.backup.button.restore",
                    editable && backup.is_some(),
                    Event::Confirm(
                        RealmsAction::RestoreBackup {
                            id,
                            backup_id: backup.map(|b| text(b, "backupId")).unwrap_or_default()
                        },
                        "mco.configure.world.restore.question.line2",
                        backup.map(|b| text(b, "backupId")).unwrap_or_default()
                    )
                );
                add!(
                    "mco.backup.button.download",
                    self.owned(&self.server),
                    Event::Confirm(
                        RealmsAction::Download { id, slot },
                        "mco.configure.world.restore.download.question.line1",
                        String::new()
                    )
                );
                add!(
                    "selectServer.refresh",
                    true,
                    Event::Request(RealmsAction::Backups { id, slot })
                );
            }
            Page::Upload => {
                let world = self.selected.and_then(|i| self.local_worlds.get(i));
                add!(
                    "mco.upload.button.name",
                    editable && world.is_some(),
                    Event::Confirm(
                        RealmsAction::Upload {
                            id,
                            slot,
                            folder: world.map(|(_, p)| p.clone()).unwrap_or_default()
                        },
                        "mco.reset.world.warning",
                        world.map(|(n, _)| n.clone()).unwrap_or_default()
                    )
                );
            }
            Page::Subscription => add!(
                "mco.configure.world.subscription.extend",
                self.owned(&self.server),
                Event::Confirm(
                    RealmsAction::OpenSubscription(id),
                    "chat.link.confirm",
                    String::new()
                )
            ),
            Page::Players => {
                let player = self
                    .server
                    .get("players")
                    .and_then(Value::as_array)
                    .and_then(|p| self.selected.and_then(|i| p.get(i)));
                let uuid = player.map(|p| text(p, "uuid")).unwrap_or_default();
                let op = player
                    .and_then(|p| p.get("operator"))
                    .and_then(Value::as_bool)
                    == Some(true);
                add!(
                    "mco.configure.world.buttons.invite",
                    editable,
                    Event::Page(Page::InvitePlayer)
                );
                add!(
                    "mco.configure.world.invites.remove.tooltip",
                    editable && player.is_some(),
                    Event::Confirm(
                        RealmsAction::RemovePlayer {
                            id,
                            uuid: uuid.clone()
                        },
                        "mco.configure.world.uninvite.question",
                        player.map(|p| text(p, "name")).unwrap_or_default()
                    )
                );
                add!(
                    if op {
                        "mco.configure.world.invites.normal.tooltip"
                    } else {
                        "mco.configure.world.invites.ops.tooltip"
                    },
                    editable && player.is_some(),
                    Event::Request(RealmsAction::SetOperator {
                        id,
                        uuid,
                        operator: !op
                    })
                );
            }
            Page::InvitePlayer => add!(
                "mco.configure.world.buttons.invite",
                editable && !self.field(0).is_empty(),
                Event::InvitePlayer
            ),
            Page::Redeem => add!(
                "mco.selectServer.joinCode.select",
                !self.field(0).is_empty(),
                Event::Redeem
            ),
            Page::Reset => {
                add!("mco.reset.world.generate", editable, Event::Reset);
                add!(
                    "mco.reset.world.template",
                    editable,
                    Event::Request(RealmsAction::Templates {
                        world_type: "WORLD_TEMPLATE".into(),
                        page: 1
                    })
                );
            }
            Page::Templates => {
                let template = self.selected.and_then(|i| self.templates.get(i));
                let template_id = template
                    .map(|t| {
                        t.get("id")
                            .map(|id| {
                                id.as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| id.to_string())
                            })
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                let action = if self.template_type == "MINIGAME" {
                    RealmsAction::SwitchMinigame { id, template_id }
                } else {
                    RealmsAction::Reset {
                        id,
                        options: json!({"seed":null,"worldTemplateId":template_id.parse::<i64>().unwrap_or(-1),"levelType":0,"generateStructures":true}),
                    }
                };
                add!(
                    "mco.template.button.select",
                    editable && template.is_some(),
                    Event::Confirm(
                        action,
                        "mco.reset.world.warning",
                        template.map(|t| text(t, "name")).unwrap_or_default()
                    )
                );
                add!(
                    "gui.back",
                    self.template_page > 1,
                    Event::Request(RealmsAction::Templates {
                        world_type: self.template_type.clone(),
                        page: (self.template_page - 1).max(1)
                    })
                );
                add!(
                    "spectatorMenu.next_page",
                    !self.templates.is_empty(),
                    Event::Request(RealmsAction::Templates {
                        world_type: self.template_type.clone(),
                        page: self.template_page + 1
                    })
                );
                add!("mco.template.title", true, Event::TemplateType);
                let trailer = template.map(|t| text(t, "trailer")).unwrap_or_default();
                let publisher = template.map(|t| text(t, "link")).unwrap_or_default();
                add!(
                    "mco.template.button.trailer",
                    valid_link(&trailer),
                    Event::Link(trailer)
                );
                add!(
                    "mco.template.button.publisher",
                    valid_link(&publisher),
                    Event::Link(publisher)
                );
            }
            Page::InviteLink => {
                let code = self
                    .invite_links
                    .get("inviteCodes")
                    .and_then(Value::as_array)
                    .and_then(|codes| self.selected.and_then(|i| codes.get(i)));
                let token = code.map(|c| text(c, "code")).unwrap_or_default();
                let enabled = code
                    .and_then(|c| c.get("enabled"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let expiration_ms = code
                    .and_then(|c| c.get("expirationDate"))
                    .and_then(Value::as_i64);
                add!(
                    "mco.configure.world.invite_codes.copy",
                    !token.is_empty(),
                    Event::Copy(token.clone())
                );
                add!(
                    "mco.configure.world.invite_codes.create",
                    editable,
                    Event::Request(RealmsAction::CreateInviteCode { id })
                );
                add!(
                    "mco.configure.world.invite_codes.delete",
                    editable && code.is_some(),
                    Event::Confirm(
                        RealmsAction::DeleteInviteCode {
                            code: token.clone()
                        },
                        "mco.configure.world.invite_codes.delete",
                        token.clone()
                    )
                );
                add!(
                    if enabled { "options.off" } else { "options.on" },
                    editable && code.is_some(),
                    Event::Request(RealmsAction::UpdateInviteCode {
                        id,
                        code: token.clone(),
                        enabled: !enabled,
                        expiration_ms
                    })
                );
                add!(
                    "mco.configure.world.invite_codes.expiration.1_day",
                    editable && code.is_some(),
                    Event::Request(RealmsAction::UpdateInviteCode {
                        id,
                        code: token.clone(),
                        enabled,
                        expiration_ms: Some(chrono::Utc::now().timestamp_millis() + 86_400_000)
                    })
                );
                add!(
                    "mco.configure.world.invite_codes.expiration.never",
                    editable && code.is_some(),
                    Event::Request(RealmsAction::UpdateInviteCode {
                        id,
                        code: token,
                        enabled,
                        expiration_ms: None
                    })
                );
                add!(
                    "selectServer.refresh",
                    editable,
                    Event::Request(RealmsAction::InviteLink(id))
                );
            }
            Page::Terms => {
                add!(
                    "mco.terms.sentence.2",
                    true,
                    Event::Request(RealmsAction::OpenTerms)
                );
                add!(
                    "mco.terms.buttons.agree",
                    true,
                    Event::Request(RealmsAction::AcceptTerms)
                );
                add!("mco.terms.buttons.disagree", true, Event::Back);
                return controls;
            }
            Page::Confirm => {
                add!(
                    "gui.yes",
                    self.confirmation.is_some(),
                    Event::AcceptConfirmation
                );
                add!("gui.no", true, Event::Back);
                return controls;
            }
        }
        controls.push(Control {
            label: tr("gui.back"),
            enabled: !self.loading,
            event: Event::Back,
        });
        controls
    }

    fn dispatch(&mut self, action: RealmsAction) -> Action {
        if action == RealmsAction::CancelRequest {
            self.loading = false;
            self.error = None;
            self.status = None;
            return Action::Realms(action);
        }
        match &action {
            RealmsAction::TierConfiguration(_) => self.enter(Page::Distances),
            RealmsAction::ConfirmJoinPack(false) => {
                self.enter(Page::List);
                return Action::Realms(action);
            }
            RealmsAction::Configure(id) => {
                self.server = self
                    .servers
                    .iter()
                    .find(|s| number(s, "id") == *id)
                    .cloned()
                    .unwrap_or(Value::Null);
                self.enter(Page::Configure);
            }
            RealmsAction::Invites => self.enter(Page::Invites),
            RealmsAction::Players(_) => self.enter(Page::Players),
            RealmsAction::Backups { slot, .. } => {
                self.slot = *slot;
                self.enter(Page::Backups);
            }
            RealmsAction::Subscription(_) => self.enter(Page::Subscription),
            RealmsAction::InviteLink(_) => self.enter(Page::InviteLink),
            RealmsAction::Templates { world_type, page } => {
                self.template_type = world_type.clone();
                self.template_page = *page;
                self.enter(Page::Templates);
            }
            _ => {}
        }
        if !matches!(
            action,
            RealmsAction::OpenTerms | RealmsAction::OpenSubscription(_) | RealmsAction::Purchase
        ) {
            self.set_loading(true);
        }
        Action::Realms(action)
    }
    pub(super) fn press(
        &mut self,
        hit: Option<u8>,
        x: f32,
        y: f32,
        size: (f32, f32),
    ) -> (Action, bool) {
        if let Some(index) = hit.and_then(|id| id.checked_sub(10)).map(usize::from) {
            let Some(control) = self.controls().into_iter().nth(index).filter(|c| c.enabled) else {
                return (Action::None, false);
            };
            let action = match control.event {
                Event::Back => return (Action::None, self.back()),
                Event::Page(page) => {
                    if self.page == Page::Slots {
                        self.slot = self.selected.map(|i| i as i32 + 1).unwrap_or(self.slot);
                    }
                    self.enter(page);
                    Action::None
                }
                Event::Request(action) => self.dispatch(action),
                Event::Confirm(action, key, detail) => {
                    self.ask(action, key, detail);
                    Action::None
                }
                Event::Initialize => self.dispatch(RealmsAction::Initialize {
                    id: self.id(),
                    name: self.field(0),
                    description: self.field(1),
                }),
                Event::Distance(simulation, delta) => {
                    let Some((min, max, current)) = self.distance(simulation) else {
                        return (Action::None, false);
                    };
                    let distance = (current as i64 + delta).clamp(min as i64, max as i64) as u32;
                    self.dispatch(if simulation {
                        RealmsAction::SetSimulationDistance {
                            id: self.id(),
                            distance,
                        }
                    } else {
                        RealmsAction::SetRenderDistance {
                            id: self.id(),
                            distance,
                        }
                    })
                }
                Event::SaveConfig => self.dispatch(RealmsAction::SaveConfig {
                    id: self.id(),
                    name: self.field(0),
                    description: self.field(1),
                }),
                Event::SaveSlot => {
                    self.slot_options["slotName"] = json!(self.field(0));
                    self.dispatch(RealmsAction::SaveSlot {
                        id: self.id(),
                        slot: self.slot,
                        options: self.slot_options.clone(),
                    })
                }
                Event::InvitePlayer => self.dispatch(RealmsAction::InvitePlayer {
                    id: self.id(),
                    name: self.field(0),
                }),
                Event::Redeem => {
                    self.ask(
                        RealmsAction::RedeemInvite(self.field(0)),
                        "mco.selectServer.joinRealm",
                        String::new(),
                    );
                    Action::None
                }
                Event::Reset => {
                    self.ask(RealmsAction::Reset{id:self.id(),options:json!({"seed":self.field(0),"worldTemplateId":-1,"levelType":0,"generateStructures":true})},"mco.configure.world.reset.question.line1",String::new());
                    Action::None
                }
                Event::Toggle(key) => {
                    let default = matches!(key, "pvp" | "spawnMonsters");
                    let old = self
                        .slot_options
                        .get(key)
                        .and_then(Value::as_bool)
                        .unwrap_or(default);
                    self.slot_options[key] = json!(!old);
                    Action::None
                }
                Event::Cycle(key, max) => {
                    self.slot_options[key] = json!((number(&self.slot_options, key) + 1) % max);
                    Action::None
                }
                Event::AcceptConfirmation => {
                    if let Some(c) = self.confirmation.take() {
                        self.enter(c.previous);
                        match c.action {
                            Action::Realms(action) => self.dispatch(action),
                            action => action,
                        }
                    } else {
                        Action::None
                    }
                }
                Event::Copy(value) => Action::CopyText(value),
                Event::Link(url) => {
                    if valid_link(&url) {
                        self.ask_action(Action::OpenUri(url.clone()), "chat.link.confirm", url);
                    }
                    Action::None
                }
                Event::TemplateType => {
                    let types = [
                        "WORLD_TEMPLATE",
                        "ADVENTUREMAP",
                        "EXPERIENCE",
                        "INSPIRATION",
                        "MINIGAME",
                    ];
                    let next = (types
                        .iter()
                        .position(|t| *t == self.template_type)
                        .unwrap_or(0)
                        + 1)
                        % types.len();
                    self.dispatch(RealmsAction::Templates {
                        world_type: types[next].into(),
                        page: 1,
                    })
                }
            };
            return (action, false);
        }
        if self.loading {
            return (Action::None, false);
        }
        self.focused = None;
        for i in 0..self.fields.len() {
            let [bx, by, bw, bh] = self.field_rect(i, size.0);
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                self.focused = Some(i);
                return (Action::None, false);
            }
        }
        let count = self.rows().len();
        let bottom = self.list_bottom(size.1);
        if (size.0 / 2.0 - 154.0..size.0 / 2.0 + 154.0).contains(&x) && (44.0..bottom).contains(&y)
        {
            let index = ((y - 44.0) / 36.0) as usize + self.offset;
            if index < count {
                self.selected = Some(index);
            }
        }
        (Action::None, false)
    }
    pub(super) fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        let controls = self.controls();
        let rows = controls.len().div_ceil(3);
        let top = h - 11.0 - rows as f32 * 24.0;
        controls
            .into_iter()
            .enumerate()
            .map(|(i, c)| Button {
                x: if self.loading {
                    (w / 2.0).floor() - 50.0
                } else {
                    (w / 2.0).floor() - 154.0 + (i % 3) as f32 * 104.0
                },
                y: top + (i / 3) as f32 * 24.0,
                w: 100.0,
                label: c.label,
                enabled: c.enabled,
                id: 10 + i as u8,
                icon: None,
                trailing: None,
            })
            .collect()
    }
    fn list_bottom(&self, h: f32) -> f32 {
        h - 19.0 - self.controls().len().div_ceil(3) as f32 * 24.0
    }
    fn field_rect(&self, index: usize, w: f32) -> [f32; 4] {
        [
            (w / 2.0).floor() - 150.0,
            62.0 + index as f32 * 42.0,
            300.0,
            20.0,
        ]
    }
    pub(super) fn focused_rect(&self, size: (f32, f32)) -> Option<[f32; 4]> {
        self.focused.map(|i| self.field_rect(i, size.0))
    }
    pub(super) fn focused_text(
        &mut self,
    ) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        let i = self.focused?;
        let f = self.fields.get_mut(i)?;
        Some((("realms", i), &mut f.value, f.max))
    }
    pub(super) fn blur(&mut self) {
        self.focused = None;
    }
    pub(super) fn scroll(&mut self, delta: f32, size: (f32, f32)) {
        let visible = ((self.list_bottom(size.1) - 44.0) / 36.0).floor().max(1.0) as usize;
        let max = self.rows().len().saturating_sub(visible);
        self.offset = (self.offset as i64 - (delta * 3.0) as i64).clamp(0, max as i64) as usize;
    }
    pub(super) fn arrow(&mut self, up: bool, size: (f32, f32)) {
        let len = self.rows().len();
        if len == 0 {
            return;
        }
        let selected = self.selected.map_or(0, |i| {
            if up {
                i.saturating_sub(1)
            } else {
                (i + 1).min(len - 1)
            }
        });
        self.selected = Some(selected);
        let visible = ((self.list_bottom(size.1) - 44.0) / 36.0).floor().max(1.0) as usize;
        if selected < self.offset {
            self.offset = selected
        } else if selected >= self.offset + visible {
            self.offset = selected + 1 - visible
        }
    }
    pub(super) fn tab(&mut self, reverse: bool, buttons: &[u8], focus: &mut Option<u8>) {
        let mut entries: Vec<(bool, usize)> = (0..self.fields.len()).map(|i| (true, i)).collect();
        entries.extend(buttons.iter().map(|id| (false, *id as usize)));
        if entries.is_empty() {
            return;
        }
        let current = self
            .focused
            .map(|i| (true, i))
            .or_else(|| focus.map(|id| (false, id as usize)));
        let index = current.and_then(|c| entries.iter().position(|e| *e == c));
        let next = if reverse {
            index.map_or(entries.len() - 1, |i| {
                (i + entries.len() - 1) % entries.len()
            })
        } else {
            index.map_or(0, |i| (i + 1) % entries.len())
        };
        self.focused = None;
        *focus = None;
        let (field, id) = entries[next];
        if field {
            self.focused = Some(id)
        } else {
            *focus = Some(id as u8)
        }
    }
    pub(super) fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut crate::ui::Caret,
        started: std::time::Instant,
    ) {
        let (w, h) = (batch.width, batch.height);
        super::centered(atlas, batch, &self.title(), 15.0, u32::MAX);
        let mut message = if !self.eligible {
            tr("horizon.realms.sign_in_required")
        } else if self.loading {
            tr("mco.configure.world.loading")
        } else {
            String::new()
        };
        if !self.loading && self.eligible {
            message = match self.page {
                Page::List if self.servers.is_empty() => tr("mco.selectServer.noRealms"),
                Page::Invites if self.invites.is_empty() => tr("mco.invites.nopending"),
                Page::Backups if self.backups.is_empty() => tr("mco.backup.nobackups"),
                Page::Upload if self.local_worlds.is_empty() => tr("mco.upload.select.world.none"),
                Page::Terms => tr("mco.terms.sentence.1"),
                Page::ResourcePack => tr("mco.configure.world.resourcepack.question"),
                Page::Confirm => self
                    .confirmation
                    .as_ref()
                    .map(|c| format!("{}\n{}", tr(c.key), c.detail))
                    .unwrap_or_default(),
                Page::Subscription => format!(
                    "{}: {}\n{}: {}",
                    tr("mco.configure.world.subscription.start"),
                    self.subscription
                        .get("startDate")
                        .map(Value::to_string)
                        .unwrap_or_default(),
                    tr("mco.configure.world.subscription.timeleft"),
                    self.subscription
                        .get("daysLeft")
                        .map(Value::to_string)
                        .unwrap_or_default()
                ),
                Page::Configure => format!(
                    "{}\n{}",
                    text(&self.server, "name"),
                    text(&self.server, "motd")
                ),
                _ => String::new(),
            };
        }
        if let Some(error) = &self.error {
            message = error.clone()
        } else if let Some(status) = &self.status {
            message = status.clone()
        }
        if !message.is_empty() {
            let lines = atlas
                .font
                .split(&Text::literal(message), (w - 40.0).clamp(100.0, 400.0));
            for (i, line) in lines.iter().enumerate() {
                super::centered(
                    atlas,
                    batch,
                    &line.plain(),
                    47.0 + i as f32 * 10.0,
                    if self.error.is_some() {
                        rgba(255, 85, 85, 255)
                    } else {
                        u32::MAX
                    },
                );
            }
            if self.loading || self.error.is_some() || self.status.is_some() || !self.eligible {
                return;
            }
        }
        for (i, field) in self.fields.iter().enumerate() {
            let rect = self.field_rect(i, w);
            batch.text(
                atlas,
                &tr(field.key),
                rect[0],
                rect[1] - 12.0,
                u32::MAX,
                true,
            );
            caret.draw(
                batch,
                atlas,
                (("realms", i), &field.value, field.max),
                rect,
                self.focused == Some(i),
                started,
            );
        }
        let bottom = self.list_bottom(h);
        for (i, (name, detail)) in self.rows().into_iter().enumerate().skip(self.offset) {
            let y = 44.0 + (i - self.offset) as f32 * 36.0;
            if y + 36.0 > bottom {
                break;
            }
            let x = (w / 2.0).floor() - 154.0;
            if self.selected == Some(i) {
                batch.fill(atlas, x, y, 308.0, 34.0, rgba(100, 100, 100, 180));
            }
            batch.text(
                atlas,
                &name[..atlas.font.fit(&name, 300.0)],
                x + 4.0,
                y + 4.0,
                u32::MAX,
                true,
            );
            batch.text(
                atlas,
                &detail[..atlas.font.fit(&detail, 300.0)],
                x + 4.0,
                y + 17.0,
                rgba(170, 170, 170, 255),
                true,
            );
        }
    }
}

impl Title {
    /// Called by the vanilla title button and by alternate startup layouts.
    pub fn open_realms(&mut self) -> Action {
        let worlds = self
            .recent_worlds()
            .into_iter()
            .filter(|w| !w.hardcore && !w.locked && !w.corrupted && w.incompatible.is_none())
            .map(|w| (w.name, self.saves.join(w.folder)))
            .collect();
        let eligible = self.realms_eligible && self.allows_multiplayer;
        let mut state = State::new(eligible, self.realms_profile_uuid.clone(), worlds);
        let action = if eligible {
            state.dispatch(RealmsAction::Refresh)
        } else {
            Action::None
        };
        self.mode = Mode::Realms(state);
        self.focused_button = None;
        action
    }
    pub fn realms_mut(&mut self) -> Option<&mut State> {
        match &mut self.mode {
            Mode::Realms(state) => Some(state),
            _ => None,
        }
    }
}
