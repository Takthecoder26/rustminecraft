//! 26.3 RealmsResetWorldScreen, RealmsSelectWorldTemplateScreen,
//! RealmsSelectFileToUploadScreen and RealmsUploadScreen. IO stays with the caller.
use super::*;
use crate::ui;
use std::{cell::RefCell, collections::HashMap, time::Instant};

#[derive(Clone, Debug)]
pub(super) struct ResetScreens {
    kind: ResetKind,
    slot: i32,
    creation: Option<(String, String)>,
    template_parent: Page,
    template_warning: bool,
    template_selected: Option<usize>,
    template_failed: bool,
    templates_finished: bool,
    worlds: Vec<super::super::WorldEntry>,
    worlds_loaded: bool,
    world_selected: Option<usize>,
    scroll: Cell<ui::Scroll>,
    last_click: Option<(usize, Instant)>,
    upload: Option<UploadProgress>,
    task: bool,
    task_title: &'static str,
    version_widths: RefCell<Vec<f32>>,
    template_visible: Cell<(usize, usize)>,
    content_link: Cell<[f32; 4]>,
    icons: RefCell<HashMap<PathBuf, WorldIcon>>,
}

#[derive(Clone, Debug)]
struct WorldIcon {
    modified: std::time::SystemTime,
    pixels: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Default)]
enum ResetKind {
    #[default]
    Reset,
    Empty,
    NewRealm,
}

#[derive(Clone, Debug)]
struct UploadProgress {
    name: String,
    done: u64,
    total: u64,
    uploading: bool,
    finished: bool,
    cancelled: bool,
    error: Option<String>,
    sample: (Instant, u64),
    bytes_per_second: u64,
}

impl Default for ResetScreens {
    fn default() -> Self {
        Self {
            kind: ResetKind::Reset,
            slot: 1,
            creation: None,
            template_parent: Page::Reset,
            template_warning: false,
            template_selected: None,
            template_failed: false,
            templates_finished: false,
            worlds: Vec::new(),
            worlds_loaded: false,
            world_selected: None,
            scroll: Cell::default(),
            last_click: None,
            upload: None,
            task: false,
            task_title: "mco.reset.world.resetting.screen.title",
            version_widths: RefCell::default(),
            template_visible: Cell::new((0, 8)),
            content_link: Cell::new([0.0; 4]),
            icons: RefCell::default(),
        }
    }
}

#[derive(Clone)]
pub(super) enum ResetEvent {
    Open { slot: i32, empty: bool },
    OpenTemplates { world_type: String, warning: bool },
    Generate,
    UploadPicker,
    SelectTemplate,
    UploadSelected,
    Back,
    CancelUpload,
    Link(String),
}

const CHOICES: [(&str, &str); 6] = [
    ("mco.reset.world.generate", "new_world"),
    ("mco.upload.select.world.title", "upload"),
    ("mco.reset.world.template", "survival_spawn"),
    ("mco.reset.world.adventure", "adventure"),
    ("mco.reset.world.experience", "experience"),
    ("mco.reset.world.inspiration", "inspiration"),
];
const TEMPLATE_TYPES: [&str; 4] = ["NORMAL", "ADVENTUREMAP", "EXPERIENCE", "INSPIRATION"];
const CONTENT_CREATION: &str = "https://aka.ms/MinecraftRealmsContentCreator";

fn control(key: &str, enabled: bool, event: ResetEvent) -> Control {
    Control {
        label: tr(key),
        enabled,
        event: Event::ResetScreen(event),
    }
}

impl State {
    /// Call only after the empty-slot switch popup has been accepted. Opening this
    /// page does not initialize, switch, reset or upload anything remotely.
    pub fn reset_open(&mut self, slot: i32, empty: bool) {
        self.reset_screens.kind = if empty {
            ResetKind::Empty
        } else {
            ResetKind::Reset
        };
        self.reset_screens.slot = slot.clamp(1, 3);
        self.reset_screens.creation = None;
        self.reset_screens.task = false;
        self.reset_screens.upload = None;
        self.slot = self.reset_screens.slot;
        self.enter(Page::Reset);
        self.fields.clear();
        self.focused = None;
        self.loading = false;
    }

    /// RealmsCreateRealmScreen carries RealmCreationTask to the choice screen;
    /// Continue itself must not initialize the server.
    pub fn reset_open_new_realm(&mut self, name: String, description: String) {
        self.reset_open(number(&self.server, "activeSlot") as i32, false);
        self.reset_screens.kind = ResetKind::NewRealm;
        self.reset_screens.creation = Some((name.trim().into(), description.trim().into()));
    }

    pub fn reset_creation(&self) -> Option<(String, String)> {
        self.reset_screens.creation.clone()
    }

    pub fn reset_set_worlds(&mut self, worlds: Vec<super::super::WorldEntry>) {
        // LevelSummary.canUpload allows hardcore and other version series. The
        // existing WorldEntry lacks requiresManualConversion; do not confuse it
        // with requiresFileFixing (which vanilla permits here).
        self.reset_screens.worlds = worlds
            .into_iter()
            .filter(|w| !w.locked && !w.corrupted)
            .collect();
        self.reset_screens.worlds_loaded = true;
        self.reset_screens.world_selected = None;
    }

    pub fn reset_open_templates(&mut self, world_type: &str, warning: bool) -> Action {
        if world_type == "MINIGAME" {
            self.reset_screens.kind = ResetKind::Reset;
            self.reset_screens.creation = None;
            self.reset_screens.task = false;
        }
        self.reset_screens.template_parent = if world_type == "MINIGAME" {
            Page::Configure
        } else {
            Page::Reset
        };
        self.reset_screens.template_warning = warning && world_type == "MINIGAME";
        self.reset_screens.template_selected = None;
        self.reset_screens.template_failed = false;
        self.reset_screens.templates_finished = false;
        self.reset_screens.template_visible.set((0, 8));
        self.reset_screens.scroll.set(ui::Scroll::default());
        self.reset_screens.last_click = None;
        self.templates.clear();
        self.template_type = world_type.into();
        self.template_page = 1;
        self.enter(Page::Templates);
        self.set_loading(true);
        Action::Realms(RealmsAction::Templates {
            world_type: world_type.into(),
            page: 1,
        })
    }

    /// Append successful pages without clearing selection or scroll. The runtime
    /// starts the returned request without re-entering the template page. Fence
    /// callbacks by the originating request/page/type before calling this method.
    pub fn reset_set_templates(&mut self, value: Value) -> Option<RealmsAction> {
        if self.page != Page::Templates {
            return None;
        }
        let page = value
            .get("page")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(self.template_page);
        let rows = array(value, "templates");
        self.loading = false;
        self.error = None;
        self.reset_screens.template_failed = false;
        if rows.is_empty() {
            self.reset_screens.templates_finished = true;
            return None;
        }
        self.templates.extend(rows);
        self.template_page = page.saturating_add(1);
        Some(RealmsAction::Templates {
            world_type: self.template_type.clone(),
            page: self.template_page,
        })
    }

    /// Vanilla stops paging after a failure and leaves successfully loaded rows.
    pub fn reset_templates_failed(&mut self) -> bool {
        if self.page != Page::Templates {
            return false;
        }
        self.loading = false;
        self.error = None;
        self.reset_screens.template_failed = true;
        self.reset_screens.templates_finished = true;
        true
    }

    /// RealmsTextureManager's image field is base64 PNG, not a URL. The renderer
    /// asynchronously installs it as `realms/dynamic/{id}`; null images use the
    /// shipped isles preset below. No synthetic preview substitutes for an image.
    pub fn reset_template_images(&self) -> Vec<(String, String)> {
        if self.page != Page::Templates {
            return Vec::new();
        }
        let (first, count) = self.reset_screens.template_visible.get();
        self.templates
            .iter()
            .skip(first)
            .take(count)
            .filter_map(|t| Some((text(t, "id"), t.get("image")?.as_str()?.to_owned())))
            .collect()
    }

    /// Used after the local CreateWorldScreen hands its temporary world back.
    pub fn reset_begin_upload(&mut self, name: String) {
        self.enter(Page::Upload);
        self.fields.clear();
        self.focused = None;
        self.reset_screens.upload = Some(UploadProgress {
            name,
            done: 0,
            total: 0,
            uploading: false,
            finished: false,
            cancelled: false,
            error: None,
            sample: (Instant::now(), 0),
            bytes_per_second: 0,
        });
        self.set_loading(true);
    }

    pub fn reset_upload_progress(&mut self, done: u64, total: u64, uploading: bool) {
        let Some(upload) = &mut self.reset_screens.upload else {
            return;
        };
        if upload.finished {
            return;
        }
        if uploading != upload.uploading || done < upload.done {
            upload.sample = (Instant::now(), done);
            upload.bytes_per_second = 0;
        }
        let elapsed = upload.sample.0.elapsed();
        if elapsed.as_secs_f64() >= 1.0 {
            upload.bytes_per_second =
                (done.saturating_sub(upload.sample.1) as f64 / elapsed.as_secs_f64()) as u64;
            upload.sample = (Instant::now(), done);
        }
        upload.done = done;
        upload.total = total;
        upload.uploading = uploading;
    }

    /// Stay on RealmsUploadScreen after completion. Done/Back refetches the
    /// configuration screen instead of silently replacing the status page.
    pub fn reset_upload_finished(&mut self, error: Option<String>) {
        if let Some(upload) = &mut self.reset_screens.upload {
            upload.finished = true;
            upload.error = error;
            self.loading = false;
            self.error = None;
            self.status = None;
        }
    }

    pub fn reset_upload_cancelled(&mut self) {
        if let Some(upload) = &mut self.reset_screens.upload {
            upload.cancelled = true;
            upload.finished = true;
            self.loading = false;
            self.error = None;
            self.status = None;
        }
    }

    fn reset_title_key(&self) -> &'static str {
        match self.reset_screens.kind {
            ResetKind::Reset => "mco.reset.world.title",
            ResetKind::Empty => "mco.configure.world.switch.slot",
            ResetKind::NewRealm => "mco.selectServer.create",
        }
    }

    fn reset_subtitle_key(&self) -> &'static str {
        match self.reset_screens.kind {
            ResetKind::Reset => "mco.reset.world.warning",
            ResetKind::Empty => "mco.configure.world.switch.slot.subtitle",
            ResetKind::NewRealm => "mco.selectServer.create.subtitle",
        }
    }

    fn reset_template_title(&self) -> &'static str {
        match self.template_type.as_str() {
            "MINIGAME" => "mco.template.title.minigame",
            "ADVENTUREMAP" => "mco.reset.world.adventure",
            "EXPERIENCE" => "mco.reset.world.experience",
            "INSPIRATION" => "mco.reset.world.inspiration",
            _ => "mco.reset.world.template",
        }
    }

    fn reset_is_picker(&self) -> bool {
        self.page == Page::Upload && self.reset_screens.upload.is_none()
    }

    fn reset_rows(&self) -> Vec<usize> {
        if self.page == Page::Templates {
            return (0..self.templates.len()).collect();
        }
        if !self.reset_is_picker() {
            return Vec::new();
        }
        let search = self
            .fields
            .first()
            .map(|f| f.value.to_lowercase())
            .unwrap_or_default();
        if !self.reset_screens.worlds_loaded {
            self.local_worlds
                .iter()
                .enumerate()
                .filter_map(|(i, (name, path))| {
                    (name.to_lowercase().contains(&search)
                        || path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&search))
                    .then_some(i)
                })
                .collect()
        } else {
            self.reset_screens
                .worlds
                .iter()
                .enumerate()
                .filter_map(|(i, world)| {
                    (world.name.to_lowercase().contains(&search)
                        || world.folder.to_lowercase().contains(&search))
                    .then_some(i)
                })
                .collect()
        }
    }

    fn reset_world(&self, index: usize) -> Option<(String, PathBuf)> {
        if !self.reset_screens.worlds_loaded {
            return self.local_worlds.get(index).cloned();
        }
        let world = self.reset_screens.worlds.get(index)?;
        let (_, path) = self
            .local_worlds
            .iter()
            .find(|(_, p)| p.file_name().is_some_and(|n| n == world.folder.as_str()))?;
        Some((world.name.clone(), path.clone()))
    }

    pub(super) fn reset_controls(&self) -> Option<Vec<Control>> {
        match self.page {
            Page::Reset if self.reset_screens.task => {
                Some(vec![control("gui.cancel", true, ResetEvent::Back)])
            }
            Page::Reset => {
                let enabled = self.editable();
                let mut controls = vec![
                    control(CHOICES[0].0, enabled, ResetEvent::Generate),
                    control(CHOICES[1].0, enabled, ResetEvent::UploadPicker),
                ];
                controls.extend(TEMPLATE_TYPES.iter().enumerate().map(|(i, kind)| {
                    control(
                        CHOICES[i + 2].0,
                        enabled,
                        ResetEvent::OpenTemplates {
                            world_type: (*kind).into(),
                            warning: false,
                        },
                    )
                }));
                controls.push(control("gui.back", true, ResetEvent::Back));
                Some(controls)
            }
            Page::Templates => {
                let selected = self
                    .reset_screens
                    .template_selected
                    .and_then(|i| self.templates.get(i));
                let mut controls = vec![
                    control(
                        "mco.template.button.select",
                        selected.is_some() && self.editable(),
                        ResetEvent::SelectTemplate,
                    ),
                    control("gui.cancel", true, ResetEvent::Back),
                ];
                if let Some(template) = selected {
                    for (field, key) in [
                        ("trailer", "mco.template.button.trailer"),
                        ("link", "mco.template.button.publisher"),
                    ] {
                        let url = text(template, field);
                        if valid_link(&url) {
                            controls.push(control(key, true, ResetEvent::Link(url)));
                        }
                    }
                }
                Some(controls)
            }
            Page::Upload => {
                if let Some(upload) = &self.reset_screens.upload {
                    Some(vec![control(
                        if !upload.finished {
                            "gui.cancel"
                        } else if upload.error.is_none() && !upload.cancelled {
                            "gui.done"
                        } else {
                            "gui.back"
                        },
                        upload.finished
                            || !(upload.uploading
                                && upload.total > 0
                                && upload.done >= upload.total),
                        if upload.finished {
                            ResetEvent::Back
                        } else {
                            ResetEvent::CancelUpload
                        },
                    )])
                } else {
                    let selected = self
                        .reset_screens
                        .world_selected
                        .filter(|i| self.reset_rows().contains(i));
                    Some(vec![
                        control(
                            "mco.upload.button.name",
                            selected.and_then(|i| self.reset_world(i)).is_some() && self.editable(),
                            ResetEvent::UploadSelected,
                        ),
                        control("gui.back", true, ResetEvent::Back),
                    ])
                }
            }
            _ => None,
        }
    }

    pub(super) fn reset_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        let controls = self.reset_controls()?;
        let center = (w / 2.0).floor();
        if self.page == Page::Templates {
            self.reset_scroll_value((w, h));
        }
        let top = 63.0_f32.min(h - 33.0 - 154.0);
        Some(
            controls
                .into_iter()
                .enumerate()
                .map(|(i, control)| {
                    let (x, y, width) = match self.page {
                        Page::Reset if !self.reset_screens.task && i < 6 => (
                            center - 122.0 + (i % 3) as f32 * 92.0,
                            top + (i / 3) as f32 * 85.0,
                            60.0,
                        ),
                        Page::Templates => {
                            // Invisible publisher/trailer buttons retain their grid cells.
                            let cell = if i == 0 {
                                1
                            } else if i == 1 {
                                2
                            } else if let Event::ResetScreen(ResetEvent::Link(url)) = &control.event
                            {
                                let trailer = self
                                    .reset_screens
                                    .template_selected
                                    .and_then(|selected| self.templates.get(selected))
                                    .is_some_and(|template| text(template, "trailer") == *url);
                                if trailer { 0 } else { 3 }
                            } else {
                                3
                            };
                            (center - 212.0 + cell as f32 * 108.0, h - 26.0, 100.0)
                        }
                        Page::Upload if self.reset_screens.upload.is_none() => {
                            (center - 154.0 + i as f32 * 158.0, h - 26.0, 150.0)
                        }
                        _ => (center - 75.0, h - 26.0, 150.0),
                    };
                    Button {
                        x,
                        y,
                        w: width,
                        label: control.label,
                        enabled: control.enabled,
                        id: 10 + i as u8,
                        icon: None,
                        trailing: None,
                    }
                })
                .collect(),
        )
    }

    pub(super) fn reset_button_height(&self, button: &Button) -> Option<f32> {
        self.reset_controls()?;
        Some(
            if self.page == Page::Reset && !self.reset_screens.task && button.id < 16 {
                69.0
            } else {
                20.0
            },
        )
    }

    pub(super) fn reset_hit(&self, x: f32, y: f32, size: (f32, f32)) -> Option<u8> {
        self.reset_buttons(size.0, size.1)?
            .into_iter()
            .find(|b| {
                (b.x..b.x + b.w).contains(&x)
                    && (b.y..b.y + self.reset_button_height(b).unwrap_or(20.0)).contains(&y)
            })
            .map(|b| b.id)
    }

    pub(super) fn reset_field_rect(&self, index: usize, w: f32) -> Option<[f32; 4]> {
        (self.reset_is_picker() && index == 0).then_some([
            (w / 2.0).floor() - 100.0,
            21.0,
            200.0,
            20.0,
        ])
    }

    fn reset_options(&self, template: i64) -> Value {
        let mut options = json!({"slotId":self.reset_screens.slot,"worldTemplateId":template});
        if let Some((name, description)) = self.reset_creation() {
            options["initialize"] = json!({"name":name,"description":description});
        }
        options
    }

    pub(super) fn reset_event(&mut self, event: ResetEvent) -> Action {
        match event {
            ResetEvent::Open { slot, empty } => {
                self.reset_open(slot, empty);
                Action::None
            }
            ResetEvent::OpenTemplates {
                world_type,
                warning,
            } => self.reset_open_templates(&world_type, warning),
            ResetEvent::Generate => self.dispatch(RealmsAction::Reset {
                id: self.id(),
                options: self.reset_options(-1),
            }),
            ResetEvent::UploadPicker => {
                self.reset_screens.upload = None;
                self.reset_screens.world_selected = None;
                self.reset_screens.scroll.set(ui::Scroll::default());
                self.reset_screens.last_click = None;
                self.enter(Page::Upload);
                self.add_field("selectWorld.search", String::new(), 1024);
                self.focused = Some(0);
                self.loading = false;
                Action::None
            }
            ResetEvent::SelectTemplate => {
                let Some(template) = self
                    .reset_screens
                    .template_selected
                    .and_then(|i| self.templates.get(i))
                else {
                    return Action::None;
                };
                let id = text(template, "id");
                let Ok(template_id) = id.parse::<i64>() else {
                    return Action::None;
                };
                let action = if self.template_type == "MINIGAME" {
                    if text(template, "type") != "MINIGAME" {
                        return Action::None;
                    }
                    RealmsAction::SwitchMinigame {
                        id: self.id(),
                        template_id: id,
                    }
                } else {
                    RealmsAction::Reset {
                        id: self.id(),
                        options: self.reset_options(template_id),
                    }
                };
                self.enter(Page::Reset);
                self.fields.clear();
                self.focused = None;
                self.reset_screens.task = true;
                self.reset_screens.task_title = if self.template_type == "MINIGAME" {
                    "mco.minigame.world.starting.screen.title"
                } else if matches!(self.reset_screens.kind, ResetKind::Reset) {
                    "mco.reset.world.resetting.screen.title"
                } else {
                    "mco.create.world.reset.title"
                };
                self.dispatch(action)
            }
            ResetEvent::UploadSelected => {
                let Some(index) = self
                    .reset_screens
                    .world_selected
                    .filter(|i| self.reset_rows().contains(i))
                else {
                    return Action::None;
                };
                let Some((name, folder)) = self.reset_world(index) else {
                    return Action::None;
                };
                self.reset_begin_upload(name);
                self.dispatch(RealmsAction::Upload {
                    id: self.id(),
                    slot: self.reset_screens.slot,
                    folder,
                    initialize: self.reset_creation(),
                })
            }
            ResetEvent::Back => self
                .reset_back_action()
                .map_or(Action::None, |(action, _)| action),
            ResetEvent::CancelUpload => {
                let started = self
                    .reset_screens
                    .upload
                    .as_ref()
                    .is_some_and(|u| u.uploading);
                if started {
                    self.reset_upload_cancelled();
                } else {
                    self.reset_show_choices();
                }
                Action::Realms(RealmsAction::CancelRequest)
            }
            ResetEvent::Link(url) => {
                if valid_link(&url) {
                    self.ask_action(Action::OpenUri(url.clone()), "chat.link.confirm", url);
                }
                Action::None
            }
        }
    }

    fn reset_show_choices(&mut self) {
        self.enter(Page::Reset);
        self.fields.clear();
        self.focused = None;
        self.reset_screens.upload = None;
        self.reset_screens.task = false;
        self.loading = false;
    }

    pub(super) fn reset_back_action(&mut self) -> Option<(Action, bool)> {
        let action = match self.page {
            Page::Templates => {
                let page = self.reset_screens.template_parent.clone();
                self.reset_show_choices();
                if page != Page::Reset {
                    self.enter(page);
                }
                Action::Realms(RealmsAction::CancelRequest)
            }
            Page::Upload => {
                if let Some(upload) = &self.reset_screens.upload {
                    if !upload.finished {
                        self.reset_event(ResetEvent::CancelUpload)
                    } else {
                        self.dispatch(RealmsAction::Configure(self.id()))
                    }
                } else {
                    self.reset_show_choices();
                    Action::None
                }
            }
            Page::Reset => {
                let task = self.reset_screens.task;
                self.reset_screens.task = false;
                let creation = self.reset_creation();
                self.enter(if matches!(self.reset_screens.kind, ResetKind::NewRealm) {
                    Page::Initialize
                } else {
                    Page::Configure
                });
                if self.page == Page::Initialize
                    && let Some((name, description)) = creation
                {
                    if let Some(f) = self.fields.get_mut(0) {
                        f.value = name;
                    }
                    if let Some(f) = self.fields.get_mut(1) {
                        f.value = description;
                    }
                }
                self.loading = false;
                if task {
                    Action::Realms(RealmsAction::CancelRequest)
                } else {
                    Action::None
                }
            }
            _ => return None,
        };
        Some((action, false))
    }

    fn reset_list_geometry(&self, size: (f32, f32)) -> Option<(f32, f32, f32, f32, f32, f32)> {
        let (top, width, row_height) = if self.page == Page::Templates {
            (
                if self.reset_screens.template_warning {
                    59.0
                } else {
                    33.0
                },
                300.0,
                46.0,
            )
        } else if self.reset_is_picker() {
            (49.0, 270.0, 36.0)
        } else {
            return None;
        };
        let bottom = (size.1 - 33.0).max(top + 1.0);
        let left = (size.0 / 2.0).floor() - width / 2.0;
        let content = self.reset_rows().len() as f32 * row_height + 4.0;
        Some((top, bottom, left, width, row_height, content))
    }

    fn reset_scroll_value(&self, size: (f32, f32)) -> ui::Scroll {
        let mut scroll = self.reset_screens.scroll.get();
        if let Some((top, bottom, _, _, row_height, content)) = self.reset_list_geometry(size) {
            scroll.clamp(top, bottom, content);
            if self.page == Page::Templates {
                let first = ((scroll.amount - 4.0).max(0.0) / row_height).floor() as usize;
                let count = ((bottom - top) / row_height).ceil() as usize + 2;
                self.reset_screens.template_visible.set((first, count));
            }
        }
        self.reset_screens.scroll.set(scroll);
        scroll
    }

    pub(super) fn reset_scroll(&mut self, delta: f32, size: (f32, f32)) -> bool {
        let Some((top, bottom, _, _, row_height, content)) = self.reset_list_geometry(size) else {
            return false;
        };
        let mut scroll = self.reset_screens.scroll.get();
        scroll.wheel(delta, row_height / 2.0, top, bottom, content);
        self.reset_screens.scroll.set(scroll);
        true
    }

    pub(in crate::title) fn reset_drag(&mut self, _x: f32, y: f32, size: (f32, f32)) -> bool {
        let Some((top, bottom, _, _, _, content)) = self.reset_list_geometry(size) else {
            return false;
        };
        let mut scroll = self.reset_screens.scroll.get();
        if bottom - top > 40.0 {
            scroll.drag(y, top, bottom, content);
        }
        self.reset_screens.scroll.set(scroll);
        true
    }

    pub(in crate::title) fn reset_release(&mut self) {
        let mut scroll = self.reset_screens.scroll.get();
        scroll.release();
        self.reset_screens.scroll.set(scroll);
    }

    /// ObjectSelectionList participates in tab navigation before the footer.
    pub(in crate::title) fn reset_tab(
        &mut self,
        reverse: bool,
        size: (f32, f32),
        focus: &mut Option<u8>,
    ) -> bool {
        if self.reset_list_geometry(size).is_none() {
            return false;
        }
        // 0 is the search field, 1 is the list, 10+ are normal button IDs.
        let mut entries = Vec::new();
        if self.reset_is_picker() {
            entries.push(0);
        }
        if !self.reset_rows().is_empty() {
            entries.push(1);
        }
        let mut buttons = self.reset_buttons(size.0, size.1).unwrap_or_default();
        buttons.sort_by(|a, b| a.x.total_cmp(&b.x));
        entries.extend(buttons.iter().filter(|b| b.enabled).map(|b| b.id));
        if entries.is_empty() {
            return true;
        }
        let current = if self.focused.is_some() {
            Some(0)
        } else if self.list_focused.get() {
            Some(1)
        } else {
            *focus
        };
        let at = current.and_then(|value| entries.iter().position(|entry| *entry == value));
        let next = if reverse {
            at.map_or(entries.len() - 1, |i| {
                (i + entries.len() - 1) % entries.len()
            })
        } else {
            at.map_or(0, |i| (i + 1) % entries.len())
        };
        self.focused = None;
        self.list_focused.set(false);
        *focus = None;
        match entries[next] {
            0 => self.focused = Some(0),
            1 => {
                let selected = if self.page == Page::Templates {
                    self.reset_screens.template_selected
                } else {
                    self.reset_screens.world_selected
                };
                if !selected.is_some_and(|i| self.reset_rows().contains(&i)) {
                    self.reset_arrow(false, size);
                }
                self.list_focused.set(true);
            }
            id => *focus = Some(id),
        }
        true
    }

    pub(in crate::title) fn reset_activate(&mut self) -> Option<(Action, bool)> {
        if !self.list_focused.get() || !self.editable() {
            return None;
        }
        let event =
            if self.page == Page::Templates && self.reset_screens.template_selected.is_some() {
                ResetEvent::SelectTemplate
            } else if self.reset_is_picker() && self.reset_screens.world_selected.is_some() {
                ResetEvent::UploadSelected
            } else {
                return None;
            };
        Some((self.reset_event(event), true))
    }

    pub(super) fn reset_arrow(&mut self, up: bool, size: (f32, f32)) -> bool {
        let Some((top, bottom, _, _, height, content)) = self.reset_list_geometry(size) else {
            return false;
        };
        let rows = self.reset_rows();
        if rows.is_empty() {
            return true;
        }
        self.focused = None;
        self.list_focused.set(true);
        let selected = if self.page == Page::Templates {
            self.reset_screens.template_selected
        } else {
            self.reset_screens.world_selected
        };
        let row = selected.and_then(|i| rows.iter().position(|r| *r == i));
        let row = row.map_or(0, |r| {
            if up {
                r.saturating_sub(1)
            } else {
                (r + 1).min(rows.len() - 1)
            }
        });
        if self.page == Page::Templates {
            self.reset_screens.template_selected = Some(rows[row]);
        } else {
            self.reset_screens.world_selected = Some(rows[row]);
        }
        let mut scroll = self.reset_screens.scroll.get();
        let y = 2.0 + row as f32 * height;
        if y < scroll.amount {
            scroll.amount = y;
        } else if y + height > scroll.amount + bottom - top {
            scroll.amount = y + height - (bottom - top);
        }
        scroll.clamp(top, bottom, content);
        self.reset_screens.scroll.set(scroll);
        true
    }

    pub(super) fn reset_press_list(&mut self, x: f32, y: f32, size: (f32, f32)) -> Option<Action> {
        let (top, bottom, left, width, height, content) = self.reset_list_geometry(size)?;
        if let Some([fx, fy, fw, fh]) = self.reset_field_rect(0, size.0)
            && (fx..fx + fw).contains(&x)
            && (fy..fy + fh).contains(&y)
        {
            self.focused = Some(0);
            self.list_focused.set(false);
            return Some(Action::None);
        }
        self.focused = None;
        if !(top..bottom).contains(&y) {
            return Some(Action::None);
        }
        let mut scroll = self.reset_scroll_value(size);
        if scroll.press((x, y), left + width + 8.0, top, bottom, content) {
            self.reset_screens.scroll.set(scroll);
            return Some(Action::None);
        }
        let [lx, ly, lw, lh] = self.reset_screens.content_link.get();
        if self.page == Page::Templates && (lx..lx + lw).contains(&x) && (ly..ly + lh).contains(&y)
        {
            return Some(self.reset_event(ResetEvent::Link(CONTENT_CREATION.into())));
        }
        if !(left..left + width).contains(&x) {
            return Some(Action::None);
        }
        let row_y = y - top - 2.0 + scroll.amount.floor();
        if row_y < 0.0 {
            return Some(Action::None);
        }
        let Some(index) = self.reset_rows().get((row_y / height) as usize).copied() else {
            return Some(Action::None);
        };
        let double = self
            .reset_screens
            .last_click
            .is_some_and(|(i, at)| i == index && at.elapsed().as_millis() < 250);
        self.reset_screens.last_click = Some((index, Instant::now()));
        self.list_focused.set(true);
        if self.page == Page::Templates {
            self.reset_screens.template_selected = Some(index);
            if let Some(url) = self.reset_template_link_at(index, x, y, size) {
                return Some(self.reset_event(ResetEvent::Link(url)));
            }
            if double {
                return Some(self.reset_event(ResetEvent::SelectTemplate));
            }
        } else {
            self.reset_screens.world_selected = Some(index);
            if double {
                return Some(self.reset_event(ResetEvent::UploadSelected));
            }
        }
        Some(Action::None)
    }

    fn reset_template_link_at(
        &self,
        index: usize,
        x: f32,
        y: f32,
        size: (f32, f32),
    ) -> Option<String> {
        let (top, bottom, left, width, row_height, _) = self.reset_list_geometry(size)?;
        if !(top..bottom).contains(&y) {
            return None;
        }
        let template = self.templates.get(index)?;
        let version_width = *self.reset_screens.version_widths.borrow().get(index)?;
        let row_y =
            top + 4.0 + index as f32 * row_height - self.reset_scroll_value(size).amount.floor();
        let right = left + width - 2.0;
        for (key, offset) in [("link", 25.0), ("trailer", 45.0)] {
            let lx = right - version_width - offset;
            let url = text(template, key);
            if valid_link(&url)
                && (lx..lx + 15.0).contains(&x)
                && (row_y..row_y + 15.0).contains(&y)
            {
                return Some(url);
            }
        }
        None
    }

    pub(super) fn reset_draw_button(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        button: &Button,
        highlighted: bool,
    ) -> bool {
        if self.page != Page::Reset || self.reset_screens.task || !(10..16).contains(&button.id) {
            return false;
        }
        let index = (button.id - 10) as usize;
        let tint = if highlighted {
            rgba(142, 142, 142, 255)
        } else {
            u32::MAX
        };
        batch.sprite_scaled(
            atlas,
            &format!("gui/realms/{}", CHOICES[index].1),
            button.x + 2.0,
            button.y + 2.0,
            56.0,
            56.0,
            tint,
        );
        batch.sprite_scaled(
            atlas,
            "gui/sprites/widget/slot_frame",
            button.x,
            button.y,
            60.0,
            60.0,
            tint,
        );
        let x = button.x + 28.0 - (atlas.font.width(&button.label) / 2.0).floor();
        batch.text(
            atlas,
            &button.label,
            x,
            button.y - 14.0,
            if highlighted {
                rgba(160, 160, 160, 255)
            } else {
                u32::MAX
            },
            true,
        );
        true
    }

    pub(super) fn reset_draw_tooltip(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
    ) -> bool {
        if self.page != Page::Templates {
            return false;
        }
        for (i, template) in self.templates.iter().enumerate() {
            if let Some(url) =
                self.reset_template_link_at(i, cursor.0, cursor.1, (batch.width, batch.height))
            {
                let trailer = url == text(template, "trailer");
                let key = if trailer {
                    "mco.template.trailer.tooltip"
                } else {
                    "mco.template.info.tooltip"
                };
                ui::tooltip(batch, atlas, &[Text::literal(tr(key))], cursor);
                break;
            }
        }
        let [x, y, w, h] = self.reset_screens.content_link.get();
        if (x..x + w).contains(&cursor.0) && (y..y + h).contains(&cursor.1) {
            ui::tooltip(batch, atlas, &[Text::literal(CONTENT_CREATION)], cursor);
        }
        true
    }

    pub(super) fn reset_draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        started: Instant,
    ) -> bool {
        match self.page {
            Page::Reset if self.reset_screens.task => {
                super::super::centered(
                    atlas,
                    batch,
                    &tr(self.reset_screens.task_title),
                    50.0,
                    u32::MAX,
                );
                if let Some(error) = &self.error {
                    super::super::centered(atlas, batch, error, 110.0, rgba(255, 0, 0, 255));
                }
                true
            }
            Page::Reset => {
                super::super::centered(atlas, batch, &tr(self.reset_title_key()), 5.0, u32::MAX);
                super::super::centered(
                    atlas,
                    batch,
                    &tr(self.reset_subtitle_key()),
                    20.0,
                    if matches!(self.reset_screens.kind, ResetKind::Reset) {
                        rgba(255, 0, 0, 255)
                    } else {
                        rgba(160, 160, 160, 255)
                    },
                );
                true
            }
            Page::Templates => {
                self.reset_draw_templates(atlas, batch);
                true
            }
            Page::Upload => {
                if let Some(upload) = &self.reset_screens.upload {
                    self.reset_draw_upload(atlas, batch, upload, started);
                } else {
                    self.reset_draw_worlds(atlas, batch, caret, started);
                }
                true
            }
            _ => false,
        }
    }

    fn reset_draw_templates(&self, atlas: &HudAtlas, batch: &mut HudBatch) {
        let size = (batch.width, batch.height);
        let Some((top, bottom, left, width, row_height, content)) = self.reset_list_geometry(size)
        else {
            return;
        };
        ui::list_background(batch, atlas, top, bottom, false);
        super::super::centered(
            atlas,
            batch,
            &tr(self.reset_template_title()),
            12.0,
            u32::MAX,
        );
        if self.reset_screens.template_warning {
            for (i, key) in [
                "mco.minigame.world.info.line1",
                "mco.minigame.world.info.line2",
            ]
            .iter()
            .enumerate()
            {
                super::super::centered(
                    atlas,
                    batch,
                    &tr(key),
                    25.0 + i as f32 * 13.0,
                    rgba(186, 186, 186, 255),
                );
            }
        }
        let scroll = self.reset_scroll_value(size).amount.floor();
        *self.reset_screens.version_widths.borrow_mut() = self
            .templates
            .iter()
            .map(|t| atlas.font.width(&text(t, "version")))
            .collect();
        let first = batch.vertices.len();
        for (i, template) in self.templates.iter().enumerate() {
            let y = top + 4.0 + i as f32 * row_height - scroll;
            if y + row_height <= top || y >= bottom {
                continue;
            }
            self.reset_draw_selection(
                atlas,
                batch,
                [left, y - 2.0, width, row_height],
                self.reset_screens.template_selected == Some(i),
            );
            let x = left + 2.0;
            let image = if template.get("image").and_then(Value::as_str).is_some() {
                format!("realms/dynamic/{}", text(template, "id"))
            } else {
                "gui/presets/isles".into()
            };
            // The asynchronous atlas owner supplies a real missing-image sprite on
            // decode failure. Until then keep the frame, without fabricating art.
            if atlas.sprites.contains_key(&image) {
                batch.sprite_scaled(atlas, &image, x + 1.0, y + 2.0, 38.0, 38.0, u32::MAX);
            }
            batch.sprite_scaled(
                atlas,
                "gui/sprites/widget/slot_frame",
                x,
                y + 1.0,
                40.0,
                40.0,
                u32::MAX,
            );
            let right = left + width - 2.0;
            let version = text(template, "version");
            let version_width = atlas.font.width(&version);
            batch.text(
                atlas,
                &text(template, "name"),
                x + 65.0,
                y + 5.0,
                u32::MAX,
                true,
            );
            batch.text(
                atlas,
                &version,
                right - version_width - 5.0,
                y + 5.0,
                rgba(160, 160, 160, 255),
                true,
            );
            batch.text(
                atlas,
                &text(template, "author"),
                x + 65.0,
                y + 19.0,
                rgba(160, 160, 160, 255),
                true,
            );
            batch.text(
                atlas,
                &text(template, "recommendedPlayers"),
                x + 65.0,
                y + 33.0,
                rgba(128, 128, 128, 255),
                true,
            );
            for (key, sprite, offset) in [("link", "link", 25.0), ("trailer", "video_link", 45.0)] {
                if valid_link(&text(template, key)) {
                    batch.sprite_scaled(
                        atlas,
                        &format!("gui/sprites/icon/{sprite}"),
                        right - version_width - offset,
                        y,
                        15.0,
                        15.0,
                        u32::MAX,
                    );
                }
            }
            crate::widgets::record_as(
                &text(template, "name"),
                [left, y - 2.0, width, row_height],
                true,
                crate::widgets::Kind::Row,
            );
        }
        batch.clip_since(first, 0.0, top, batch.width, bottom);
        self.reset_screens.content_link.set([0.0; 4]);
        if self.templates.is_empty() && self.reset_screens.templates_finished {
            if self.reset_screens.template_failed {
                super::super::centered(
                    atlas,
                    batch,
                    &tr("mco.template.select.failure"),
                    92.0,
                    u32::MAX,
                );
            } else {
                let link = tr("mco.template.select.none.linkTitle");
                let message = translated("mco.template.select.none", std::slice::from_ref(&link));
                for (i, line) in message.lines().enumerate() {
                    let y = 92.0 + i as f32 * 13.0;
                    let x = (batch.width / 2.0).floor() - (atlas.font.width(line) / 2.0).floor();
                    if let Some(at) = line.find(&link) {
                        batch.text(atlas, &line[..at], x, y, u32::MAX, true);
                        let lx = x + atlas.font.width(&line[..at]);
                        batch.text(atlas, &link, lx, y, rgba(51, 102, 187, 255), true);
                        batch.text(
                            atlas,
                            &line[at + link.len()..],
                            lx + atlas.font.width(&link),
                            y,
                            u32::MAX,
                            true,
                        );
                        self.reset_screens.content_link.set([
                            lx,
                            y - 3.0,
                            atlas.font.width(&link),
                            11.0,
                        ]);
                    } else {
                        batch.text(atlas, line, x, y, u32::MAX, true);
                    }
                }
            }
        }
        ui::separators(batch, atlas, top, bottom, false);
        ui::scrollbar(
            batch,
            atlas,
            left + width + 8.0,
            top,
            bottom,
            content,
            scroll,
        );
    }

    fn reset_draw_selection(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        [x, y, w, h]: [f32; 4],
        selected: bool,
    ) {
        if selected {
            batch.fill(
                atlas,
                x,
                y,
                w,
                h,
                if self.list_focused.get() {
                    u32::MAX
                } else {
                    rgba(128, 128, 128, 255)
                },
            );
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                w - 2.0,
                h - 2.0,
                rgba(0, 0, 0, 255),
            );
        }
    }

    fn reset_draw_worlds(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        started: Instant,
    ) {
        let size = (batch.width, batch.height);
        let Some((top, bottom, left, width, row_height, content)) = self.reset_list_geometry(size)
        else {
            return;
        };
        ui::list_background(batch, atlas, top, bottom, false);
        super::super::centered(
            atlas,
            batch,
            &tr("mco.upload.select.world.title"),
            8.0,
            u32::MAX,
        );
        if let Some(field) = self.fields.first() {
            caret.draw(
                batch,
                atlas,
                (("realms", 0), &field.value, field.max),
                self.reset_field_rect(0, batch.width).unwrap(),
                self.focused == Some(0),
                started,
            );
        }
        let scroll = self.reset_scroll_value(size).amount.floor();
        let first = batch.vertices.len();
        for (row, i) in self.reset_rows().into_iter().enumerate() {
            let y = top + 4.0 + row as f32 * row_height - scroll;
            if y + row_height <= top || y >= bottom {
                continue;
            }
            self.reset_draw_selection(
                atlas,
                batch,
                [left, y - 2.0, width, row_height],
                self.reset_screens.world_selected == Some(i),
            );
            let x = left + 2.0;
            let Some((name, folder)) = self.reset_world(i) else {
                continue;
            };
            let tx = x + 35.0;
            let max_width = width - 39.0;
            let fitted = &name[..atlas.font.fit(&name, max_width)];
            batch.text(atlas, fitted, tx, y + 1.0, u32::MAX, true);
            if let Some(world) = self.reset_screens.worlds.get(i) {
                let date: chrono::DateTime<chrono::Local> = world.modified.into();
                let line = format!("{} ({})", world.folder, date.format("%d/%m/%Y, %H:%M"));
                batch.text(
                    atlas,
                    &line[..atlas.font.fit(&line, max_width)],
                    tx,
                    y + 12.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
                let details = super::super::world_info(world);
                let start = batch.vertices.len();
                batch.text_styled(
                    atlas,
                    &details,
                    tx,
                    y + 21.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
                batch.clip_since(start, tx, top, left + width - 2.0, bottom);
            } else {
                batch.text(
                    atlas,
                    &folder.file_name().unwrap_or_default().to_string_lossy(),
                    tx,
                    y + 12.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
            }
            self.reset_draw_world_icon(
                atlas,
                batch,
                &folder,
                row % crate::hud::WORLD_ICON_SLOTS,
                (x, y),
            );
            crate::widgets::record_as(
                &name,
                [left, y - 2.0, width, row_height],
                true,
                crate::widgets::Kind::Row,
            );
        }
        if if self.reset_screens.worlds_loaded {
            self.reset_screens.worlds.is_empty()
        } else {
            self.local_worlds.is_empty()
        } {
            super::super::centered(
                atlas,
                batch,
                &tr("mco.upload.select.world.none"),
                top + 16.0,
                u32::MAX,
            );
        }
        batch.clip_since(first, 0.0, top, batch.width, bottom);
        ui::separators(batch, atlas, top, bottom, false);
        ui::scrollbar(
            batch,
            atlas,
            left + width + 8.0,
            top,
            bottom,
            content,
            scroll,
        );
    }

    fn reset_draw_world_icon(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        folder: &std::path::Path,
        slot: usize,
        (x, y): (f32, f32),
    ) {
        let path = folder.join("icon.png");
        let mut icons = self.reset_screens.icons.borrow_mut();
        let pixels = if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            let icon = icons.entry(path.clone()).or_insert_with(|| WorldIcon {
                modified: std::time::UNIX_EPOCH,
                pixels: None,
            });
            if icon.modified != modified {
                icon.modified = modified;
                icon.pixels = crate::hud::decode(&path)
                    .ok()
                    .filter(|(w, h, _)| *w == 64 && *h == 64)
                    .map(|(_, _, pixels)| pixels);
            }
            icon.pixels.as_ref()
        } else {
            None
        };
        let sprite = format!("horizon/world-icon{slot}");
        if let Some(pixels) = pixels
            && let Some(rect) = atlas.sprites.get(&sprite)
        {
            batch.patches.push(crate::font::Patch {
                x: (rect.u0 * atlas.width as f32).round() as u32,
                y: (rect.v0 * atlas.height as f32).round() as u32,
                width: 64,
                height: 64,
                rgba: pixels.clone(),
            });
            batch.sprite_scaled(atlas, &sprite, x, y, 32.0, 32.0, u32::MAX);
        } else {
            batch.sprite_scaled(atlas, "misc/unknown_server", x, y, 32.0, 32.0, u32::MAX);
        }
    }

    fn reset_draw_upload(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        upload: &UploadProgress,
        started: Instant,
    ) {
        let status = if upload.cancelled {
            tr("mco.upload.cancelled")
        } else if let Some(error) = &upload.error {
            translated("mco.upload.failed", std::slice::from_ref(error))
        } else if upload.finished {
            tr("mco.upload.done")
        } else if upload.uploading && upload.total > 0 && upload.done >= upload.total {
            tr("mco.upload.verifying")
        } else if upload.uploading {
            translated("mco.upload.uploading", std::slice::from_ref(&upload.name))
        } else {
            tr("mco.upload.preparing")
        };
        super::super::centered(atlas, batch, &status, 50.0, u32::MAX);
        if !upload.finished {
            let dots =
                ["", ".", ". .", ". . ."][(started.elapsed().as_millis() / 500 % 4) as usize];
            batch.text(
                atlas,
                dots,
                (batch.width / 2.0).floor() + (atlas.font.width(&status) / 2.0).floor() + 5.0,
                50.0,
                u32::MAX,
                true,
            );
        }
        if upload.uploading && upload.total > 0 && !upload.cancelled {
            let percentage = (upload.done as f64 / upload.total as f64).clamp(0.0, 1.0);
            let progress = format!("{:.1}", percentage * 100.0);
            let left = ((batch.width - 200.0) / 2.0).floor();
            let width = (200.0 * percentage).round() as f32;
            batch.fill(atlas, left - 1.0, 79.0, width + 2.0, 17.0, u32::MAX);
            batch.fill(atlas, left, 80.0, width, 15.0, rgba(128, 128, 128, 255));
            super::super::centered(
                atlas,
                batch,
                &translated("mco.upload.percent", std::slice::from_ref(&progress)),
                84.0,
                u32::MAX,
            );
            if upload.bytes_per_second > 0 {
                let bytes = upload.bytes_per_second;
                let speed = if bytes < 1024 {
                    format!("{bytes} B")
                } else {
                    let exp = ((bytes as f64).ln() / 1024.0_f64.ln())
                        .floor()
                        .clamp(1.0, 6.0) as usize;
                    format!(
                        "{:.1} {}B",
                        bytes as f64 / 1024.0_f64.powi(exp as i32),
                        ["", "K", "M", "G", "T", "P", "E"][exp]
                    )
                };
                batch.text(
                    atlas,
                    &format!("({speed}/s)"),
                    (batch.width / 2.0).floor()
                        + (atlas.font.width(&progress) / 2.0).floor()
                        + 15.0,
                    84.0,
                    u32::MAX,
                    true,
                );
            }
        }
    }
}
