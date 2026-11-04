//! 26.3 RealmsWorldsTab -> SlotOptions / Backups -> BackupInfo.
//! The configuration Worlds tab remains owned by `realms_configuration`.
use super::*;
use crate::ui;

#[derive(Clone, Debug, Default)]
pub(super) struct WorldScreens {
    slot: Option<SlotDraft>,
    backups: Vec<BackupRow>,
    backups_loaded: bool,
    info: Option<usize>,
    info_selected: Option<usize>,
    info_focused: bool,
    row_focus: Option<(usize, bool)>,
    scroll: Cell<ui::Scroll>,
    visible: Cell<(usize, usize)>,
    button_width: Cell<f32>,
    field_y: Cell<f32>,
    slider_value: f32,
    slider_dragging: bool,
    slider_editing: bool,
    popup: Option<WorldPopup>,
    popup_rect: Cell<[f32; 4]>,
    pending: Option<WorldOperation>,
    failure: Option<(String, bool)>,
}

#[derive(Clone, Debug)]
struct SlotDraft {
    options: Value,
    world_type: String,
    hardcore: bool,
    default_name: String,
    difficulty: usize,
    game_mode: usize,
    force_game_mode: bool,
    spawn_protection: i64,
}

#[derive(Clone, Debug)]
struct BackupRow {
    id: String,
    date: String,
    timestamp: i64,
    changes: Vec<(String, String)>,
    uploaded: bool,
}

#[derive(Clone, Debug)]
struct WorldPopup {
    action: RealmsAction,
    message: String,
    warning: bool,
}

#[derive(Clone, Copy, Debug)]
enum WorldOperation {
    Save,
    Restore,
    Download,
}

#[derive(Clone)]
pub(super) enum WorldEvent {
    Back,
    Difficulty,
    GameMode,
    ForceGameMode,
    SpawnProtection,
    Save,
    Changes(usize),
    Restore(usize),
    Download,
    Accept,
}

const DIFFICULTIES: [&str; 4] = [
    "options.difficulty.peaceful",
    "options.difficulty.easy",
    "options.difficulty.normal",
    "options.difficulty.hard",
];
const GAME_MODES: [&str; 3] = [
    "selectWorld.gameMode.survival",
    "selectWorld.gameMode.creative",
    "selectWorld.gameMode.adventure",
];
const LIST_TOP: f32 = 33.0;
const ROW_HEIGHT: f32 = 36.0;

impl State {
    pub(super) fn world_is_page(&self) -> bool {
        matches!(
            self.page,
            Page::SlotOptions | Page::Backups | Page::BackupInfo
        )
    }

    /// Called by `enter` after clearing fields, replacing its old SlotOptions arm.
    pub(super) fn world_enter(&mut self) {
        self.world_release();
        self.world_screens.slider_editing = false;
        self.world_screens.popup = None;
        self.world_screens.failure = None;
        self.world_screens.pending = None;
        self.world_screens.row_focus = None;
        self.world_screens.info_selected = None;
        self.world_screens.info_focused = false;
        self.world_screens.scroll.set(ui::Scroll::default());
        match self.page {
            Page::SlotOptions => {
                self.slot = number(&self.server, "activeSlot").clamp(1, 3) as i32;
                let slot = self
                    .server
                    .get("slots")
                    .and_then(Value::as_array)
                    .and_then(|slots| {
                        slots
                            .iter()
                            .find(|slot| number(slot, "slotId") == i64::from(self.slot))
                    });
                let options = slot
                    .and_then(|s| s.get("options"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let options = options
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(options);
                let options = if options.is_object() {
                    options
                } else {
                    json!({})
                };
                let hardcore = slot
                    .and_then(|s| s.get("settings"))
                    .and_then(Value::as_array)
                    .and_then(|settings| settings.iter().find(|s| text(s, "name") == "hardcore"))
                    .and_then(|s| s.get("value"))
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.eq_ignore_ascii_case("true"));
                let default_name = translated("mco.configure.world.slot", &[self.slot.to_string()]);
                // RealmsWorldOptions.copy drops `empty`; getSlotName then maps a blank
                // name to the default, and setWorldName maps the default back to "".
                let name = text(&options, "slotName");
                let name = if name.trim().is_empty() || name == default_name {
                    String::new()
                } else {
                    name
                };
                let world_type = text(&self.server, "worldType");
                let normal = world_type == "NORMAL";
                let spawn_protection = if normal {
                    number(&options, "spawnProtection")
                } else {
                    0
                };
                let force_game_mode =
                    normal && options.get("forceGameMode").and_then(Value::as_bool) == Some(true);
                let difficulty = valid_index(&options, "difficulty", 4, 2);
                let game_mode = valid_index(&options, "gameMode", 3, 0);
                self.world_screens.slider_value = spawn_protection.clamp(0, 16) as f32 / 16.0;
                self.world_screens.slot = Some(SlotDraft {
                    options,
                    world_type,
                    hardcore,
                    default_name,
                    difficulty,
                    game_mode,
                    force_game_mode,
                    spawn_protection,
                });
                self.add_field("mco.configure.world.edit.slot.name", name, 32);
            }
            Page::Backups => {
                self.backups.clear();
                self.world_screens.backups.clear();
                self.world_screens.backups_loaded = false;
                self.world_screens.info = None;
            }
            _ => {}
        }
    }

    /// Called after `set_backups` installs the successful response. Failure is not
    /// an empty response: vanilla leaves the list blank and Download inactive.
    pub(super) fn world_backups_loaded(&mut self) {
        self.world_screens.backups = self
            .backups
            .iter()
            .enumerate()
            .map(|(i, backup)| {
                let timestamp = backup_timestamp(backup);
                let date = backup_date(timestamp);
                let mut changes = Vec::new();
                let mut uploaded = false;
                if let Some(older) = self.backups.get(i + 1)
                    && let Some(metadata) = backup.get("metadata").and_then(Value::as_object)
                {
                    // Java stores both maps in HashMap; the visible order of
                    // these otherwise identical changes is not reproduced here.
                    for (key, value) in metadata {
                        let Some(value) = metadata_string(value) else {
                            continue;
                        };
                        let previous = older
                            .get("metadata")
                            .and_then(|m| m.get(key))
                            .and_then(metadata_string);
                        if key.contains("uploaded") {
                            changes.push((key.clone(), date.clone()));
                            uploaded = true;
                        } else if previous.as_ref() != Some(&value) {
                            changes.push((key.clone(), value));
                        }
                    }
                }
                BackupRow {
                    id: text(backup, "backupId"),
                    date,
                    timestamp,
                    changes,
                    uploaded,
                }
            })
            .collect();
        self.world_screens.backups_loaded = true;
        self.world_screens.row_focus = None;
    }

    /// Controller completion hook, including a Download Done with no page change.
    pub fn world_operation_finished(&mut self) {
        self.world_screens.pending = None;
        self.world_screens.failure = None;
        self.loading = false;
    }

    /// Call before the controller's generic set_error. Backup fetch errors only
    /// log in vanilla. Save/restore errors return to configuration; download
    /// errors return to the backup screen. No failed response invents empty data.
    pub fn world_request_failed(&mut self, action: &RealmsAction, message: &str) -> bool {
        match action {
            RealmsAction::Backups { .. } => {
                log::error!("Couldn't request Realms backups: {message}");
                self.loading = false;
                true
            }
            RealmsAction::SaveSlot { .. }
            | RealmsAction::RestoreBackup { .. }
            | RealmsAction::Download { .. }
                if self.world_is_page() =>
            {
                self.world_screens.pending = None;
                self.world_screens.failure = Some((
                    message.to_owned(),
                    matches!(action, RealmsAction::Download { .. }),
                ));
                self.loading = false;
                self.focused = None;
                true
            }
            _ => false,
        }
    }

    pub(super) fn world_title(&self) -> Option<String> {
        if !self.world_is_page() {
            return None;
        }
        Some(tr(if self.world_screens.failure.is_some() {
            "mco.errorMessage.generic"
        } else {
            match self.page {
                Page::SlotOptions => "mco.configure.world.buttons.options",
                Page::BackupInfo => "mco.backup.info.title",
                _ => "mco.configure.world.backup",
            }
        }))
    }

    pub(super) fn world_controls(&self) -> Option<Vec<Control>> {
        if !self.world_is_page() {
            return None;
        }
        let control = |label: String, enabled, event| Control {
            label,
            enabled,
            event: Event::World(event),
        };
        if self.world_screens.failure.is_some() {
            return Some(vec![control(tr("gui.back"), true, WorldEvent::Back)]);
        }
        if let Some(operation) = self.world_screens.pending {
            return Some(if matches!(operation, WorldOperation::Save) {
                vec![]
            } else {
                vec![control(tr("gui.cancel"), true, WorldEvent::Back)]
            });
        }
        if self.world_screens.popup.is_some() {
            return Some(vec![
                control(tr("gui.continue"), true, WorldEvent::Accept),
                control(tr("gui.cancel"), true, WorldEvent::Back),
            ]);
        }
        if self.page == Page::SlotOptions {
            let draft = self.world_screens.slot.as_ref()?;
            let normal = draft.world_type == "NORMAL";
            let ready = !self.loading;
            let on_off = tr(if draft.force_game_mode {
                "options.on"
            } else {
                "options.off"
            });
            let protection = if draft.spawn_protection == 0 {
                tr("options.off")
            } else {
                draft.spawn_protection.to_string()
            };
            return Some(vec![
                control(
                    option_label("options.difficulty", tr(DIFFICULTIES[draft.difficulty])),
                    ready && !draft.hardcore,
                    WorldEvent::Difficulty,
                ),
                control(
                    option_label("selectWorld.gameMode", tr(GAME_MODES[draft.game_mode])),
                    ready && !draft.hardcore,
                    WorldEvent::GameMode,
                ),
                control(
                    option_label("mco.configure.world.forceGameMode", on_off),
                    ready && normal && !draft.hardcore,
                    WorldEvent::ForceGameMode,
                ),
                control(
                    option_label("mco.configure.world.spawnProtection", protection),
                    ready && normal,
                    WorldEvent::SpawnProtection,
                ),
                control(tr("gui.continue"), ready, WorldEvent::Save),
                control(tr("gui.cancel"), ready, WorldEvent::Back),
            ]);
        }
        if self.page == Page::BackupInfo {
            return Some(vec![control(tr("gui.back"), true, WorldEvent::Back)]);
        }
        let mut controls = vec![
            control(
                tr("mco.backup.button.download"),
                self.world_screens.backups_loaded && !self.world_screens.backups.is_empty(),
                WorldEvent::Download,
            ),
            control(tr("gui.back"), true, WorldEvent::Back),
        ];
        let expired = self.server.get("expired").and_then(Value::as_bool) == Some(true);
        let (first, count) = self.world_screens.visible.get();
        // Title IDs are u8. Expose only visible rows, never truncate the data list.
        for (row, backup) in self
            .world_screens
            .backups
            .iter()
            .enumerate()
            .skip(first)
            .take(count.min(120))
        {
            if !backup.changes.is_empty() {
                controls.push(control(
                    tr("mco.backup.changes.tooltip"),
                    true,
                    WorldEvent::Changes(row),
                ));
            }
            if !expired {
                controls.push(control(
                    tr("mco.backup.button.restore"),
                    true,
                    WorldEvent::Restore(row),
                ));
            }
        }
        Some(controls)
    }

    pub(super) fn world_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        if !self.world_is_page() {
            return None;
        }
        self.viewport_height.set(h);
        self.world_layout_list(h);
        let c = (w / 2.0).floor();
        let options_top = self.world_options_top(h);
        self.world_screens.field_y.set(options_top + 13.0);
        let expired = self.server.get("expired").and_then(Value::as_bool) == Some(true);
        Some(
            self.world_controls()?
                .into_iter()
                .enumerate()
                .map(|(i, control)| {
                    let (x, y, width) = if self.world_screens.failure.is_some()
                        || self.world_screens.pending.is_some()
                    {
                        (c - 75.0, h - 26.0, 150.0)
                    } else if self.world_screens.popup.is_some() {
                        let rect = self.world_screens.popup_rect.get();
                        (
                            c - 107.0 + i as f32 * 110.0,
                            rect[1] + rect[3] - 38.0,
                            104.0,
                        )
                    } else if self.page == Page::SlotOptions {
                        if i < 4 {
                            (
                                c - 154.0 + (i % 2) as f32 * 158.0,
                                options_top + 41.0 + (i / 2) as f32 * 28.0,
                                150.0,
                            )
                        } else {
                            (c - 154.0 + (i - 4) as f32 * 158.0, h - 26.0, 150.0)
                        }
                    } else if self.page == Page::BackupInfo {
                        (c - 75.0, h - 26.0, 150.0)
                    } else if i < 2 {
                        (c - 154.0 + i as f32 * 158.0, h - 26.0, 150.0)
                    } else {
                        let width = self.world_screens.button_width.get().max(8.0);
                        let (row, changes) = match control.event {
                            Event::World(WorldEvent::Changes(row)) => (row, true),
                            Event::World(WorldEvent::Restore(row)) => (row, false),
                            _ => unreachable!(),
                        };
                        let offset = if changes && !expired { 2.0 } else { 1.0 };
                        (
                            c + 148.0 - offset * (width + 8.0),
                            43.0 + row as f32 * ROW_HEIGHT
                                - self.world_screens.scroll.get().amount.floor(),
                            width,
                        )
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

    pub(super) fn world_field_rect(&self, index: usize, w: f32) -> Option<[f32; 4]> {
        (self.page == Page::SlotOptions
            && index == 0
            && self.world_screens.pending.is_none()
            && self.world_screens.failure.is_none())
        .then(|| {
            [
                (w / 2.0).floor() - 75.0,
                self.world_screens.field_y.get(),
                150.0,
                20.0,
            ]
        })
    }

    fn world_options_top(&self, h: f32) -> f32 {
        let header = if self.world_warning().is_some() {
            58.0
        } else {
            33.0
        };
        (header + 30.0_f32).min(h - 33.0 - 89.0)
    }

    fn world_warning(&self) -> Option<&'static str> {
        match self.world_screens.slot.as_ref()?.world_type.as_str() {
            "ADVENTUREMAP" => Some("mco.configure.world.edit.subscreen.adventuremap"),
            "INSPIRATION" => Some("mco.configure.world.edit.subscreen.inspiration"),
            "EXPERIENCE" => Some("mco.configure.world.edit.subscreen.experience"),
            _ => None,
        }
    }

    pub(super) fn world_draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        started: std::time::Instant,
    ) -> bool {
        if !self.world_is_page() {
            return false;
        }
        self.viewport_height.set(batch.height);
        if let Some((message, _)) = &self.world_screens.failure {
            super::super::centered(
                atlas,
                batch,
                &tr("mco.errorMessage.generic"),
                80.0,
                u32::MAX,
            );
            for (i, line) in atlas
                .font
                .split(&Text::literal(message), (batch.width - 40.0).max(1.0))
                .iter()
                .enumerate()
            {
                super::super::centered(
                    atlas,
                    batch,
                    &line.plain(),
                    100.0 + i as f32 * 9.0,
                    rgba(255, 85, 85, 255),
                );
            }
            return true;
        }
        if let Some(operation) = self.world_screens.pending {
            super::super::centered(
                atlas,
                batch,
                &tr(match operation {
                    WorldOperation::Save => "mco.configure.world.loading",
                    WorldOperation::Restore => "mco.backup.restoring",
                    WorldOperation::Download => "mco.download.preparing",
                }),
                80.0,
                u32::MAX,
            );
            return true;
        }
        if self.page == Page::SlotOptions {
            let warning = self.world_warning();
            super::super::centered(
                atlas,
                batch,
                &tr("mco.configure.world.buttons.options"),
                if warning.is_some() { 16.0 } else { 12.0 },
                u32::MAX,
            );
            if let Some(key) = warning {
                super::super::centered(atlas, batch, &tr(key), 33.0, rgba(255, 0, 0, 255));
            }
            self.world_screens
                .field_y
                .set(self.world_options_top(batch.height) + 13.0);
            if let Some(field) = self.fields.first() {
                let rect = self.world_field_rect(0, batch.width).unwrap();
                batch.text(
                    atlas,
                    &tr(field.key),
                    rect[0],
                    rect[1] - 13.0,
                    u32::MAX,
                    true,
                );
                caret.draw(
                    batch,
                    atlas,
                    (("realms", 0), &field.value, field.max),
                    rect,
                    self.focused == Some(0),
                    started,
                );
            }
            return true;
        }
        self.world_screens
            .button_width
            .set(8.0 + HudBatch::text_width(atlas, &tr("mco.backup.changes.tooltip")));
        self.world_draw_list(atlas, batch);
        if let Some(popup) = &self.world_screens.popup {
            // PopupScreen draws its parent (including the footer) under a dim layer.
            for (i, key) in ["mco.backup.button.download", "gui.back"]
                .into_iter()
                .enumerate()
            {
                let button = Button {
                    x: (batch.width / 2.0).floor() - 154.0 + i as f32 * 158.0,
                    y: batch.height - 26.0,
                    w: 150.0,
                    label: tr(key),
                    enabled: true,
                    id: 0,
                    icon: None,
                    trailing: None,
                };
                paint_button(atlas, batch, &button, false);
            }
            self.world_draw_popup(atlas, batch, popup);
        }
        true
    }

    fn world_draw_list(&self, atlas: &HudAtlas, batch: &mut HudBatch) {
        self.world_layout_list(batch.height);
        let bottom = (batch.height - 33.0).max(LIST_TOP);
        ui::list_background(batch, atlas, LIST_TOP, bottom, false);
        super::super::centered(
            atlas,
            batch,
            &tr(if self.page == Page::BackupInfo {
                "mco.backup.info.title"
            } else {
                "mco.configure.world.backup"
            }),
            12.0,
            u32::MAX,
        );
        let first_vertex = batch.vertices.len();
        let scroll = self.world_screens.scroll.get().amount.floor();
        let c = (batch.width / 2.0).floor();
        let (first, count) = self.world_screens.visible.get();
        if self.page == Page::BackupInfo {
            if let Some(backup) = self
                .world_screens
                .info
                .and_then(|i| self.world_screens.backups.get(i))
            {
                for (i, (key, value)) in backup.changes.iter().enumerate().skip(first).take(count) {
                    let y = LIST_TOP + 2.0 + i as f32 * ROW_HEIGHT - scroll;
                    if self.world_screens.info_selected == Some(i) {
                        let color = if self.world_screens.info_focused {
                            u32::MAX
                        } else {
                            rgba(128, 128, 128, 255)
                        };
                        batch.fill(atlas, c - 110.0, y, 220.0, ROW_HEIGHT, color);
                        batch.fill(
                            atlas,
                            c - 109.0,
                            y + 1.0,
                            218.0,
                            ROW_HEIGHT - 2.0,
                            rgba(0, 0, 0, 255),
                        );
                    }
                    batch.text(
                        atlas,
                        &tr(metadata_key(key)),
                        c - 108.0,
                        y + 2.0,
                        rgba(160, 160, 160, 255),
                        true,
                    );
                    batch.text(
                        atlas,
                        &metadata_value(key, value),
                        c - 108.0,
                        y + 14.0,
                        u32::MAX,
                        true,
                    );
                }
            }
        } else {
            for (i, backup) in self
                .world_screens
                .backups
                .iter()
                .enumerate()
                .skip(first)
                .take(count)
            {
                let y = LIST_TOP + 2.0 + i as f32 * ROW_HEIGHT - scroll;
                batch.text(
                    atlas,
                    &translated("mco.backup.entry", &[backup_age(backup.timestamp)]),
                    c - 148.0,
                    y + 7.0,
                    if backup.uploaded {
                        rgba(127, 255, 127, 255)
                    } else {
                        u32::MAX
                    },
                    true,
                );
                batch.text(
                    atlas,
                    &backup.date,
                    c - 148.0,
                    y + 20.0,
                    rgba(76, 76, 76, 255),
                    true,
                );
            }
            if self.world_screens.backups_loaded && self.world_screens.backups.is_empty() {
                super::super::centered(
                    atlas,
                    batch,
                    &tr("mco.backup.nobackups"),
                    LIST_TOP + ((bottom - LIST_TOP) / 2.0).floor() - 4.0,
                    u32::MAX,
                );
            }
            if self.world_screens.popup.is_some() {
                // Inline widgets are otherwise drawn by Title after this method.
                for (row, backup) in self
                    .world_screens
                    .backups
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(count)
                {
                    let width = self.world_screens.button_width.get();
                    let mut right = c + 148.0;
                    for (key, show) in [
                        (
                            "mco.backup.button.restore",
                            self.server.get("expired").and_then(Value::as_bool) != Some(true),
                        ),
                        ("mco.backup.changes.tooltip", !backup.changes.is_empty()),
                    ] {
                        if show {
                            right -= width + 8.0;
                            paint_button(
                                atlas,
                                batch,
                                &Button {
                                    x: right,
                                    y: 43.0 + row as f32 * ROW_HEIGHT - scroll,
                                    w: width,
                                    label: tr(key),
                                    enabled: true,
                                    id: 0,
                                    icon: None,
                                    trailing: None,
                                },
                                false,
                            );
                        }
                    }
                }
            }
        }
        batch.clip_since(first_vertex, 0.0, LIST_TOP, batch.width, bottom);
        if bottom - LIST_TOP > 40.0 {
            ui::scrollbar(
                batch,
                atlas,
                c + if self.page == Page::BackupInfo {
                    118.0
                } else {
                    158.0
                },
                LIST_TOP,
                bottom,
                self.world_content_height(),
                scroll,
            );
        }
        ui::separators(batch, atlas, LIST_TOP, bottom, false);
    }

    fn world_draw_popup(&self, atlas: &HudAtlas, batch: &mut HudBatch, popup: &WorldPopup) {
        let title = Text::translate(
            if popup.warning {
                "mco.warning"
            } else {
                "mco.info"
            },
            &[],
        )
        .bold();
        let title_lines = atlas.font.split(&title, 214.0);
        let message_lines = atlas.font.split(&Text::literal(&popup.message), 214.0);
        let content_height = (title_lines.len() + message_lines.len()) as f32 * 9.0 + 44.0;
        let top = ((batch.height - content_height) / 2.0).floor();
        let left = (batch.width / 2.0).floor() - 125.0;
        self.world_screens
            .popup_rect
            .set([left, top - 18.0, 250.0, content_height + 36.0]);
        batch.fill(
            atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            rgba(0, 0, 0, 180),
        );
        ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/popup/background",
            left,
            top - 18.0,
            250.0,
            content_height + 36.0,
            u32::MAX,
        );
        let mut y = top;
        for line in title_lines {
            let x = ((batch.width - atlas.font.width_text(&line)) / 2.0).floor();
            batch.text_styled(
                atlas,
                &line,
                x,
                y,
                if popup.warning {
                    rgba(255, 0, 0, 255)
                } else {
                    rgba(125, 135, 190, 255)
                },
                true,
            );
            y += 9.0;
        }
        y += 12.0;
        for line in message_lines {
            let x = ((batch.width - atlas.font.width_text(&line)) / 2.0).floor();
            batch.text_styled(atlas, &line, x, y, u32::MAX, true);
            y += 9.0;
        }
    }

    pub(super) fn world_draw_button(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        button: &Button,
        highlighted: bool,
    ) -> bool {
        if !self.world_is_page() {
            return false;
        }
        if self.page == Page::SlotOptions
            && button.id == 13
            && self.world_screens.pending.is_none()
            && self.world_screens.failure.is_none()
        {
            ui::slider_ext(
                batch,
                atlas,
                button.x,
                button.y,
                button.w,
                &Text::literal(&button.label),
                self.world_screens.slider_value,
                highlighted,
                self.world_screens.slider_editing,
                self.world_screens.slider_editing,
                button.enabled,
                self.world_screens.slider_dragging,
            );
            return true;
        }
        let first = batch.vertices.len();
        let focused = self
            .world_controls()
            .and_then(|controls| {
                controls
                    .into_iter()
                    .nth(usize::from(button.id.saturating_sub(10)))
            })
            .is_some_and(|control| match control.event {
                Event::World(WorldEvent::Changes(row)) => {
                    self.world_screens.row_focus == Some((row, false))
                }
                Event::World(WorldEvent::Restore(row)) => {
                    self.world_screens.row_focus == Some((row, true))
                }
                _ => false,
            });
        paint_button(atlas, batch, button, highlighted || focused);
        if self.page == Page::Backups
            && button.id >= 12
            && self.world_screens.popup.is_none()
            && self.world_screens.pending.is_none()
            && self.world_screens.failure.is_none()
        {
            batch.clip_since(first, 0.0, LIST_TOP, batch.width, batch.height - 33.0);
        }
        true
    }

    /// Resolve clipping here before Title's common 20px button hit rectangles.
    pub(super) fn world_hit(&self, x: f32, y: f32, size: (f32, f32)) -> Option<u8> {
        self.world_buttons(size.0, size.1)?
            .into_iter()
            .find(|b| {
                let inline = self.page == Page::Backups
                    && b.id >= 12
                    && self.world_screens.popup.is_none()
                    && self.world_screens.pending.is_none()
                    && self.world_screens.failure.is_none();
                (!inline || (LIST_TOP..size.1 - 33.0).contains(&y))
                    && (b.x..b.x + b.w).contains(&x)
                    && (b.y..b.y + 20.0).contains(&y)
            })
            .map(|b| b.id)
    }

    pub(super) fn world_press_event(
        &mut self,
        event: WorldEvent,
        x: f32,
        size: (f32, f32),
    ) -> Action {
        self.focused = None;
        self.world_screens.row_focus = None;
        self.world_screens.info_focused = false;
        self.world_screens.slider_editing = matches!(event, WorldEvent::SpawnProtection);
        if matches!(event, WorldEvent::Back) {
            return self.world_back_action().map_or(Action::None, |v| v.0);
        }
        if self.world_screens.pending.is_some() || self.world_screens.failure.is_some() {
            return Action::None;
        }
        if self.world_screens.popup.is_some() && !matches!(event, WorldEvent::Accept) {
            return Action::None;
        }
        match event {
            WorldEvent::Difficulty | WorldEvent::GameMode | WorldEvent::ForceGameMode => {
                self.world_cycle(event, false);
            }
            WorldEvent::SpawnProtection => {
                if self
                    .world_screens
                    .slot
                    .as_ref()
                    .is_some_and(|d| d.world_type == "NORMAL")
                {
                    self.world_screens.slider_dragging = true;
                    self.world_slider_at(x, size.0);
                }
            }
            WorldEvent::Save => {
                let Some(draft) = self.world_screens.slot.as_ref() else {
                    return Action::None;
                };
                let mut options = draft.options.clone();
                let name = self
                    .fields
                    .first()
                    .map(|f| f.value.clone())
                    .unwrap_or_default();
                options["slotName"] = json!(if name == draft.default_name {
                    String::new()
                } else {
                    name
                });
                options["difficulty"] = json!(draft.difficulty);
                options["gameMode"] = json!(draft.game_mode);
                if !matches!(
                    draft.world_type.as_str(),
                    "ADVENTUREMAP" | "EXPERIENCE" | "INSPIRATION"
                ) {
                    options["spawnProtection"] = json!(draft.spawn_protection);
                    options["forceGameMode"] = json!(draft.force_game_mode);
                }
                self.world_screens.pending = Some(WorldOperation::Save);
                self.focused = None;
                return self.dispatch(RealmsAction::SaveSlot {
                    id: self.id(),
                    slot: self.slot,
                    options,
                });
            }
            WorldEvent::Changes(row) => {
                if self
                    .world_screens
                    .backups
                    .get(row)
                    .is_some_and(|b| !b.changes.is_empty())
                {
                    self.world_screens.info = Some(row);
                    self.enter(Page::BackupInfo);
                }
            }
            WorldEvent::Restore(row) => {
                if self.server.get("expired").and_then(Value::as_bool) == Some(true) {
                    return Action::None;
                }
                if let Some(backup) = self.world_screens.backups.get(row) {
                    self.world_screens.popup = Some(WorldPopup {
                        action: RealmsAction::RestoreBackup {
                            id: self.id(),
                            backup_id: backup.id.clone(),
                        },
                        message: translated(
                            "mco.configure.world.restore.question.line1",
                            &[backup.date.clone(), backup_age(backup.timestamp)],
                        ),
                        warning: true,
                    });
                    self.world_screens.row_focus = None;
                }
            }
            WorldEvent::Download => {
                if self.world_screens.backups_loaded && !self.world_screens.backups.is_empty() {
                    self.world_screens.popup = Some(WorldPopup {
                        action: RealmsAction::Download {
                            id: self.id(),
                            slot: self.slot,
                        },
                        message: tr("mco.configure.world.restore.download.question.line1"),
                        warning: false,
                    });
                }
            }
            WorldEvent::Accept => {
                if let Some(popup) = self.world_screens.popup.take() {
                    self.world_screens.pending = Some(if popup.warning {
                        WorldOperation::Restore
                    } else {
                        WorldOperation::Download
                    });
                    return self.dispatch(popup.action);
                }
            }
            WorldEvent::Back => {}
        }
        Action::None
    }

    /// Called before generic back_action. The parent configure screen is reused;
    /// BackupInfo/Popup onClose reinitializes Backups and fetches a fresh list.
    pub(super) fn world_back_action(&mut self) -> Option<(Action, bool)> {
        if !self.world_is_page() {
            return None;
        }
        if matches!(self.world_screens.pending, Some(WorldOperation::Save)) {
            return Some((Action::None, false));
        }
        if self.world_screens.pending.take().is_some() {
            self.enter(Page::Configure);
            self.loading = false;
            return Some((Action::Realms(RealmsAction::CancelRequest), false));
        }
        if let Some((_, backups)) = self.world_screens.failure.take() {
            if backups {
                return Some((self.world_reload_backups(), false));
            }
            self.enter(Page::Configure);
        } else if self.world_screens.popup.take().is_some() || self.page == Page::BackupInfo {
            return Some((self.world_reload_backups(), false));
        } else {
            self.enter(Page::Configure);
        }
        self.loading = false;
        Some((Action::None, false))
    }

    fn world_reload_backups(&mut self) -> Action {
        self.dispatch(RealmsAction::Backups {
            id: self.id(),
            slot: self.slot,
        })
    }

    pub(super) fn world_press_content(&mut self, x: f32, y: f32, size: (f32, f32)) -> bool {
        if !self.world_is_page() {
            return false;
        }
        if self.world_screens.popup.is_some()
            || self.world_screens.pending.is_some()
            || self.world_screens.failure.is_some()
        {
            return true;
        }
        self.focused = None;
        self.world_screens.slider_editing = false;
        if self.page == Page::SlotOptions {
            if let Some([bx, by, bw, bh]) = self.world_field_rect(0, size.0)
                && (bx..bx + bw).contains(&x)
                && (by..by + bh).contains(&y)
            {
                self.focused = Some(0);
            }
            return true;
        }
        self.world_layout_list(size.1);
        let mut scroll = self.world_screens.scroll.get();
        let bar = (size.0 / 2.0).floor()
            + if self.page == Page::BackupInfo {
                118.0
            } else {
                158.0
            };
        if scroll.press(
            (x, y),
            bar,
            LIST_TOP,
            size.1 - 33.0,
            self.world_content_height(),
        ) {
            self.world_screens.scroll.set(scroll);
            return true;
        }
        if self.page == Page::BackupInfo {
            let left = (size.0 / 2.0).floor() - 110.0;
            if (left..left + 220.0).contains(&x) && (LIST_TOP..size.1 - 33.0).contains(&y) {
                let relative = y - LIST_TOP - 2.0 + scroll.amount.floor();
                let row = (relative / ROW_HEIGHT).floor();
                if row >= 0.0 && (row as usize) < self.world_row_count() {
                    self.world_screens.info_selected = Some(row as usize);
                    self.world_screens.info_focused = true;
                    self.world_ensure_row(row as usize, size.1);
                }
            } else {
                self.world_screens.info_focused = false;
            }
        }
        // ContainerObjectSelectionList entries have no row-click selection.
        true
    }

    pub(super) fn world_scroll(&mut self, delta: f32, h: f32) -> bool {
        if !self.world_is_page() {
            return false;
        }
        if self.page != Page::SlotOptions
            && self.world_screens.popup.is_none()
            && self.world_screens.pending.is_none()
            && self.world_screens.failure.is_none()
        {
            let mut scroll = self.world_screens.scroll.get();
            scroll.wheel(delta, 18.0, LIST_TOP, h - 33.0, self.world_content_height());
            self.world_screens.scroll.set(scroll);
            self.world_layout_list(h);
        }
        true
    }

    pub(in crate::title) fn world_drag(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if !self.world_is_page() {
            return false;
        }
        if self.world_screens.slider_dragging {
            self.world_slider_at(cursor.0, size.0);
        } else if size.1 - 66.0 > 40.0 {
            let mut scroll = self.world_screens.scroll.get();
            scroll.drag(
                cursor.1,
                LIST_TOP,
                size.1 - 33.0,
                self.world_content_height(),
            );
            self.world_screens.scroll.set(scroll);
            self.world_layout_list(size.1);
        }
        true
    }

    pub(in crate::title) fn world_release(&mut self) {
        self.world_screens.slider_dragging = false;
        let mut scroll = self.world_screens.scroll.get();
        scroll.release();
        self.world_screens.scroll.set(scroll);
    }

    fn world_slider_at(&mut self, x: f32, width: f32) {
        self.world_screens.slider_value = ui::slider_value((width / 2.0).floor() + 4.0, 150.0, x);
        if let Some(draft) = &mut self.world_screens.slot {
            draft.spawn_protection = (self.world_screens.slider_value * 16.0) as i64;
        }
    }

    fn world_cycle(&mut self, event: WorldEvent, reverse: bool) {
        if self.loading {
            return;
        }
        let Some(draft) = &mut self.world_screens.slot else {
            return;
        };
        if draft.hardcore {
            return;
        }
        match event {
            WorldEvent::Difficulty => {
                draft.difficulty = (draft.difficulty + if reverse { 3 } else { 1 }) % 4
            }
            WorldEvent::GameMode => {
                draft.game_mode = (draft.game_mode + if reverse { 2 } else { 1 }) % 3
            }
            WorldEvent::ForceGameMode if draft.world_type == "NORMAL" => {
                draft.force_game_mode = !draft.force_game_mode
            }
            _ => {}
        }
    }

    /// Title calls this before its spatial navigation. Inline backup focus is
    /// represented by a row identity, so scrolling cannot silently retarget it.
    pub(in crate::title) fn world_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        size: (f32, f32),
        focus: &mut Option<u8>,
    ) -> bool {
        use winit::keyboard::KeyCode;
        if !self.world_is_page() {
            return false;
        }
        let vertical = matches!(key, KeyCode::ArrowUp | KeyCode::ArrowDown);
        let horizontal = matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight);
        if !vertical && !horizontal {
            return false;
        }
        if self.focused.is_some() {
            return false;
        }
        if self.world_screens.popup.is_some()
            || self.world_screens.pending.is_some()
            || self.world_screens.failure.is_some()
        {
            return false;
        }
        if self.page == Page::SlotOptions {
            if *focus == Some(13) && horizontal && self.world_screens.slider_editing {
                if self
                    .world_screens
                    .slot
                    .as_ref()
                    .is_some_and(|d| d.world_type == "NORMAL")
                {
                    let step = if key == KeyCode::ArrowLeft { -1.0 } else { 1.0 } / 142.0;
                    self.world_screens.slider_value =
                        (self.world_screens.slider_value + step).clamp(0.0, 1.0);
                    if let Some(draft) = &mut self.world_screens.slot {
                        draft.spawn_protection = (self.world_screens.slider_value * 16.0) as i64;
                    }
                }
                return true;
            }
            self.world_screens.slider_editing = false;
            return false;
        }
        if self.page == Page::BackupInfo {
            if !vertical {
                return self.world_screens.info_focused;
            }
            let len = self.world_row_count();
            if len == 0 {
                return false;
            }
            let up = key == KeyCode::ArrowUp;
            let current = self
                .world_screens
                .info_selected
                .filter(|_| self.world_screens.info_focused);
            let row = current.map_or(Some(if up { len - 1 } else { 0 }), |i| {
                if up {
                    i.checked_sub(1)
                } else {
                    (i + 1 < len).then_some(i + 1)
                }
            });
            if let Some(row) = row {
                self.world_screens.info_selected = Some(row);
                self.world_screens.info_focused = true;
                *focus = None;
                self.world_ensure_row(row, size.1);
            } else {
                self.world_screens.info_focused = false;
                self.world_screens.info_selected = None;
                *focus = Some(10);
            }
            return true;
        }
        let expired = self.server.get("expired").and_then(Value::as_bool) == Some(true);
        if focus.is_some_and(|id| id >= 12) {
            self.world_screens.row_focus =
                self.world_event_for(*focus).and_then(|event| match event {
                    WorldEvent::Changes(row) => Some((row, false)),
                    WorldEvent::Restore(row) => Some((row, true)),
                    _ => None,
                });
            *focus = None;
        }
        if horizontal {
            if let Some((row, restore)) = self.world_screens.row_focus {
                let has_changes = self
                    .world_screens
                    .backups
                    .get(row)
                    .is_some_and(|b| !b.changes.is_empty());
                if key == KeyCode::ArrowLeft && restore && has_changes {
                    self.world_screens.row_focus = Some((row, false));
                } else if key == KeyCode::ArrowRight && !restore && !expired {
                    self.world_screens.row_focus = Some((row, true));
                }
                return true;
            }
            return false;
        }
        if focus.is_some_and(|id| id < 12) && key == KeyCode::ArrowDown {
            return false;
        }
        let up = key == KeyCode::ArrowUp;
        let len = self.world_screens.backups.len();
        let (start, restore) = self.world_screens.row_focus.map_or(
            (if up { len as isize - 1 } else { 0 }, false),
            |(row, restore)| (row as isize + if up { -1 } else { 1 }, restore),
        );
        let mut row = start;
        while row >= 0 && (row as usize) < len {
            let has_changes = !self.world_screens.backups[row as usize].changes.is_empty();
            if has_changes || !expired {
                self.world_screens.row_focus =
                    Some((row as usize, !expired && (restore || !has_changes)));
                self.world_ensure_row(row as usize, size.1);
                *focus = None;
                return true;
            }
            row += if up { -1 } else { 1 };
        }
        self.world_screens.row_focus = None;
        *focus = Some(
            if self.world_screens.backups_loaded && !self.world_screens.backups.is_empty() {
                10
            } else {
                11
            },
        );
        true
    }

    pub(in crate::title) fn world_confirm(
        &mut self,
        focus: Option<u8>,
        size: (f32, f32),
        reverse: bool,
    ) -> Option<Action> {
        if !self.world_is_page() {
            return None;
        }
        if self.focused.is_some() {
            return Some(Action::None);
        }
        let event = self.world_event_for(focus).or_else(|| {
            self.world_screens.row_focus.map(|(row, restore)| {
                if restore {
                    WorldEvent::Restore(row)
                } else {
                    WorldEvent::Changes(row)
                }
            })
        });
        let Some(event) = event else {
            return Some(Action::None);
        };
        if matches!(event, WorldEvent::SpawnProtection) {
            self.world_screens.slider_editing = !self.world_screens.slider_editing;
            return Some(Action::None);
        }
        if matches!(
            event,
            WorldEvent::Difficulty | WorldEvent::GameMode | WorldEvent::ForceGameMode
        ) {
            self.world_cycle(event, reverse);
            return Some(Action::None);
        }
        Some(self.world_press_event(event, 0.0, size))
    }

    pub(in crate::title) fn world_reverse_cycle(
        &mut self,
        x: f32,
        y: f32,
        size: (f32, f32),
    ) -> bool {
        if self.page != Page::SlotOptions {
            return false;
        }
        if let Some(event) = self.world_event_for(self.world_hit(x, y, size))
            && matches!(
                event,
                WorldEvent::Difficulty | WorldEvent::GameMode | WorldEvent::ForceGameMode
            )
        {
            self.world_cycle(event, true);
            return true;
        }
        false
    }

    pub(in crate::title) fn world_tab(
        &mut self,
        reverse: bool,
        size: (f32, f32),
        focus: &mut Option<u8>,
    ) -> bool {
        if !self.world_is_page() {
            return false;
        }
        self.world_buttons(size.0, size.1);
        // (kind, index, restore): field, button, backup child, info list.
        let mut entries = Vec::<(u8, usize, bool)>::new();
        let modal = self.world_screens.popup.is_some()
            || self.world_screens.pending.is_some()
            || self.world_screens.failure.is_some();
        if !modal && self.page == Page::SlotOptions {
            entries.push((0, 0, false));
        }
        if !modal && self.page == Page::Backups {
            let expired = self.server.get("expired").and_then(Value::as_bool) == Some(true);
            for (row, backup) in self.world_screens.backups.iter().enumerate() {
                if !backup.changes.is_empty() {
                    entries.push((2, row, false));
                }
                if !expired {
                    entries.push((2, row, true));
                }
            }
        } else if !modal && self.page == Page::BackupInfo && self.world_row_count() > 0 {
            entries.push((3, 0, false));
        }
        for (i, control) in self
            .world_controls()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            if control.enabled
                && !matches!(
                    control.event,
                    Event::World(WorldEvent::Changes(_) | WorldEvent::Restore(_))
                )
            {
                entries.push((1, i + 10, false));
            }
        }
        let mouse_row = self.world_event_for(*focus).and_then(|event| match event {
            WorldEvent::Changes(row) => Some((2, row, false)),
            WorldEvent::Restore(row) => Some((2, row, true)),
            _ => None,
        });
        let current = self
            .focused
            .map(|i| (0, i, false))
            .or(mouse_row)
            .or_else(|| {
                self.world_screens
                    .row_focus
                    .map(|(row, restore)| (2, row, restore))
            })
            .or_else(|| self.world_screens.info_focused.then_some((3, 0, false)))
            .or_else(|| focus.map(|id| (1, usize::from(id), false)));
        if entries.is_empty() {
            return true;
        }
        let current =
            current.and_then(|current| entries.iter().position(|entry| *entry == current));
        let next = if reverse {
            current.map_or(entries.len() - 1, |i| {
                (i + entries.len() - 1) % entries.len()
            })
        } else {
            current.map_or(0, |i| (i + 1) % entries.len())
        };
        self.focused = None;
        self.world_screens.row_focus = None;
        self.world_screens.info_focused = false;
        self.world_screens.slider_editing = false;
        *focus = None;
        match entries[next] {
            (0, i, _) => self.focused = Some(i),
            (1, id, _) => {
                *focus = Some(id as u8);
                self.world_screens.slider_editing = self.page == Page::SlotOptions && id == 13;
            }
            (2, row, restore) => {
                self.world_screens.row_focus = Some((row, restore));
                self.world_ensure_row(row, size.1);
            }
            (3, _, _) => {
                let row = self.world_screens.info_selected.unwrap_or(if reverse {
                    self.world_row_count() - 1
                } else {
                    0
                });
                self.world_screens.info_selected = Some(row);
                self.world_screens.info_focused = true;
                self.world_ensure_row(row, size.1);
            }
            _ => unreachable!(),
        }
        true
    }

    fn world_event_for(&self, focus: Option<u8>) -> Option<WorldEvent> {
        let index = usize::from(focus?.checked_sub(10)?);
        match self
            .world_controls()?
            .into_iter()
            .nth(index)
            .filter(|c| c.enabled)?
            .event
        {
            Event::World(event) => Some(event),
            _ => None,
        }
    }

    fn world_row_count(&self) -> usize {
        if self.page == Page::BackupInfo {
            self.world_screens
                .info
                .and_then(|i| self.world_screens.backups.get(i))
                .map_or(0, |b| b.changes.len())
        } else {
            self.world_screens.backups.len()
        }
    }
    fn world_content_height(&self) -> f32 {
        self.world_row_count() as f32 * ROW_HEIGHT + 4.0
    }
    fn world_layout_list(&self, h: f32) {
        let mut scroll = self.world_screens.scroll.get();
        scroll.clamp(
            LIST_TOP,
            (h - 33.0).max(LIST_TOP),
            self.world_content_height(),
        );
        self.world_screens.scroll.set(scroll);
        let first = ((scroll.amount - 2.0).max(0.0) / ROW_HEIGHT).floor() as usize;
        let count = ((h - 66.0).max(0.0) / ROW_HEIGHT).ceil() as usize + 1;
        self.world_screens.visible.set((
            first,
            count.min(self.world_row_count().saturating_sub(first)),
        ));
    }
    fn world_ensure_row(&self, row: usize, h: f32) {
        let mut scroll = self.world_screens.scroll.get();
        let top = row as f32 * ROW_HEIGHT;
        if top < scroll.amount {
            scroll.amount = top;
        }
        let bottom = top + ROW_HEIGHT + 4.0;
        if bottom > scroll.amount + h - 66.0 {
            scroll.amount = bottom - (h - 66.0);
        }
        self.world_screens.scroll.set(scroll);
        self.world_layout_list(h);
    }
}

fn valid_index(options: &Value, key: &str, len: usize, fallback: usize) -> usize {
    options
        .get(key)
        .and_then(Value::as_u64)
        .filter(|v| *v < len as u64)
        .map_or(fallback, |v| v as usize)
}
fn option_label(key: &str, value: String) -> String {
    translated("options.generic_value", &[tr(key), value])
}
fn metadata_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Bool(_) | Value::Number(_) => Some(value.to_string()),
        _ => None,
    }
}
fn backup_timestamp(backup: &Value) -> i64 {
    backup
        .get("lastModifiedDate")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or_default()
}
// The caller currently has no system-locale date formatter. Keep local time;
// Java's localized SHORT date/time remains an explicit parity limitation.
fn backup_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp_millis(timestamp)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m/%d/%y, %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
fn backup_age(timestamp: i64) -> String {
    let elapsed = chrono::Utc::now()
        .timestamp_millis()
        .saturating_sub(timestamp);
    if elapsed < 0 {
        return tr("mco.util.time.now");
    }
    let seconds = elapsed / 1000;
    let (key, value) = if seconds < 60 {
        ("mco.time.secondsAgo", seconds)
    } else if seconds < 3600 {
        ("mco.time.minutesAgo", seconds / 60)
    } else if seconds < 86400 {
        ("mco.time.hoursAgo", seconds / 3600)
    } else {
        ("mco.time.daysAgo", seconds / 86400)
    };
    translated(key, &[value.to_string()])
}
fn metadata_key(key: &str) -> &'static str {
    match key {
        "template_name" => "mco.backup.entry.templateName",
        "game_difficulty" => "mco.backup.entry.gameDifficulty",
        "name" => "mco.backup.entry.name",
        "game_server_version" => "mco.backup.entry.gameServerVersion",
        "uploaded" => "mco.backup.entry.uploaded",
        "enabled_packs" => "mco.backup.entry.enabledPack",
        "description" => "mco.backup.entry.description",
        "game_mode" => "mco.backup.entry.gameMode",
        "seed" => "mco.backup.entry.seed",
        "world_type" => "mco.backup.entry.worldType",
        _ => "mco.backup.entry.undefined",
    }
}
fn metadata_value(key: &str, value: &str) -> String {
    let lower = key.to_lowercase();
    let choices: Option<&[&str]> = if lower.contains("game") && lower.contains("mode") {
        Some(&GAME_MODES)
    } else if lower.contains("game") && lower.contains("difficulty") {
        Some(&DIFFICULTIES)
    } else {
        None
    };
    if let Some(choices) = choices {
        return value
            .parse::<usize>()
            .ok()
            .and_then(|i| choices.get(i))
            .map_or_else(|| tr("mco.backup.unknown"), |key| tr(key));
    }
    if key == "world_type" {
        return tr(match value.to_uppercase().as_str() {
            "NORMAL" => "mco.backup.entry.worldType.normal",
            "MINIGAME" => "mco.backup.entry.worldType.minigame",
            "ADVENTUREMAP" => "mco.backup.entry.worldType.adventureMap",
            "EXPERIENCE" => "mco.backup.entry.worldType.experience",
            "INSPIRATION" => "mco.backup.entry.worldType.inspiration",
            _ => "mco.backup.entry.worldType.unknown",
        });
    }
    value.to_owned()
}
fn paint_button(atlas: &HudAtlas, batch: &mut HudBatch, button: &Button, highlighted: bool) {
    ui::nine_slice(
        batch,
        atlas,
        if !button.enabled {
            "gui/sprites/widget/button_disabled"
        } else if highlighted {
            "gui/sprites/widget/button_highlighted"
        } else {
            "gui/sprites/widget/button"
        },
        button.x,
        button.y,
        button.w,
        20.0,
        u32::MAX,
    );
    button.draw_label(
        batch,
        atlas,
        if button.enabled {
            u32::MAX
        } else {
            rgba(160, 160, 160, 255)
        },
    );
}
