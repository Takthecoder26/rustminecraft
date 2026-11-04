//! 26.3 RealmsPreferredRegionSelectionScreen; selection is applied to the Settings draft.
use super::*;
use crate::ui;
use winit::keyboard::KeyCode;

const TOP: f32 = 33.0;
const ROW_HEIGHT: f32 = 16.0;
const ROW_WIDTH: f32 = 220.0;
const DONE: u8 = 10;
const CANCEL: u8 = 11;

#[derive(Clone, Debug, Default)]
pub(super) struct RegionScreen {
    rows: Vec<RegionRow>,
    selected: Option<usize>,
    focused: bool,
    scroll: Cell<ui::Scroll>,
    height: Cell<f32>,
    settings_scroll: f32,
    last_click: Option<(usize, std::time::Instant)>,
}

#[derive(Clone)]
pub(super) enum RegionEvent {
    Done,
    Cancel,
}

#[derive(Clone, Debug)]
struct RegionRow {
    preference: u8,
    region: Option<&'static str>,
    label_key: String,
    quality: i64,
}

impl RegionRow {
    fn value(&self) -> Value {
        json!({"regionSelectionPreference": self.preference, "preferredRegion": self.region})
    }
}

// RealmsConfigureWorldScreen uses LinkedHashMap: retain first insertion order,
// replace the quality of duplicates, and omit RealmsRegion.INVALID_REGION.
fn region_rows(regions: &Value) -> Vec<RegionRow> {
    let mut rows: Vec<RegionRow> = ["automatic_player", "automatic_owner"]
        .into_iter()
        .enumerate()
        .map(|(preference, name)| RegionRow {
            preference: preference as u8,
            region: None,
            label_key: format!("realms.configuration.region_preference.{name}"),
            quality: 5,
        })
        .collect();
    for data in regions
        .get("regionData")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some((name, key)) = REGIONS
            .iter()
            .find(|(name, _)| data.get("region").and_then(Value::as_str) == Some(*name))
        else {
            continue;
        };
        let quality = number(data, "serviceQuality");
        if let Some(row) = rows.iter_mut().find(|row| row.region == Some(*name)) {
            row.quality = quality;
        } else {
            rows.push(RegionRow {
                preference: 2,
                region: Some(name),
                label_key: format!("realms.configuration.region.{key}"),
                quality,
            });
        }
    }
    rows
}

fn quality_icon(quality: i64) -> &'static str {
    match quality {
        1 => "gui/sprites/icon/ping_5",
        2 => "gui/sprites/icon/ping_4",
        3 => "gui/sprites/icon/ping_3",
        4 => "gui/sprites/icon/ping_2",
        _ => "gui/sprites/icon/ping_unknown",
    }
}

impl RegionScreen {
    fn content_height(&self) -> f32 {
        self.rows.len() as f32 * ROW_HEIGHT + 4.0
    }

    fn layout(&self, height: f32) {
        let resized = self.height.replace(height) != height;
        let mut scroll = self.scroll.get();
        scroll.clamp(TOP, (height - TOP).max(TOP), self.content_height());
        self.scroll.set(scroll);
        if resized {
            self.ensure_visible();
        }
    }

    fn ensure_visible(&self) {
        let Some(index) = self.selected else { return };
        let height = self.height.get();
        if height <= TOP * 2.0 {
            return;
        }
        let mut scroll = self.scroll.get();
        let row_top = index as f32 * ROW_HEIGHT;
        if row_top < scroll.amount {
            scroll.amount = row_top;
        }
        // AbstractSelectionList.scrollToEntry keeps two pixels at both ends.
        let row_bottom = row_top + ROW_HEIGHT + 4.0;
        let view = height - TOP * 2.0;
        if row_bottom > scroll.amount + view {
            scroll.amount = row_bottom - view;
        }
        scroll.clamp(TOP, height - TOP, self.content_height());
        self.scroll.set(scroll);
    }

    fn focus_list(&mut self, reverse: bool) {
        if self.selected.is_none() && !self.rows.is_empty() {
            self.selected = Some(if reverse { self.rows.len() - 1 } else { 0 });
        }
        self.focused = true;
        self.ensure_visible();
    }
}

impl State {
    /// RealmsSettingsTab uses the same service-quality icon as the selector.
    pub(super) fn region_preference_icon(&self, preference: &Value) -> &'static str {
        let region = preference.get("preferredRegion").and_then(Value::as_str);
        let quality = region_rows(&self.regions)
            .iter()
            .find(|row| row.region.is_some() && row.region == region)
            .map_or(5, |row| row.quality);
        quality_icon(quality)
    }

    /// RealmsSettingsTab.updateData supplies the first offered manual region when
    /// the server has MANUAL with no region. This never changes server data.
    pub(super) fn region_normalize_preference(&self, preference: Value) -> Value {
        let mode = match number(&preference, "regionSelectionPreference") {
            1 => 1,
            2 => 2,
            _ => 0,
        };
        let region = if mode == 2 {
            preference
                .get("preferredRegion")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| {
                    region_rows(&self.regions)
                        .get(2)
                        .map_or(Value::Null, |row| json!(row.region))
                })
        } else {
            Value::Null
        };
        json!({"regionSelectionPreference": mode, "preferredRegion": region})
    }

    /// Call before enter() clears the Settings scroll and fields, after remembering
    /// its text edits. The preference must come from the pending Settings draft.
    pub(super) fn region_enter(&mut self, preference: Value) {
        let preference = self.region_normalize_preference(preference);
        let rows = region_rows(&self.regions);
        let selected = rows.iter().position(|row| row.value() == preference);
        self.region_screen = RegionScreen {
            rows,
            selected,
            settings_scroll: self.pixel_scroll.get(),
            ..RegionScreen::default()
        };
    }

    pub(super) fn region_controls(&self) -> Option<Vec<Control>> {
        (self.page == Page::Region).then(|| {
            vec![
                Control {
                    label: tr("gui.done"),
                    enabled: self.region_screen.selected.is_some() && !self.configuration.saving,
                    event: Event::Region(RegionEvent::Done),
                },
                Control {
                    label: tr("gui.cancel"),
                    enabled: !self.configuration.saving,
                    event: Event::Region(RegionEvent::Cancel),
                },
            ]
        })
    }

    pub(super) fn region_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        let controls = self.region_controls()?;
        self.region_screen.layout(h);
        Some(
            controls
                .into_iter()
                .enumerate()
                .map(|(i, control)| Button {
                    x: (w / 2.0).floor() - 154.0 + i as f32 * 158.0,
                    y: h - 26.0,
                    w: 150.0,
                    label: control.label,
                    enabled: control.enabled,
                    id: DONE + i as u8,
                    icon: None,
                    trailing: None,
                })
                .collect(),
        )
    }

    /// Done applies only the local draft; Cancel and Escape discard this screen's
    /// selection. A missing draft must never fall through to a service mutation.
    pub(super) fn region_event(&mut self, event: RegionEvent) -> Action {
        if self.page != Page::Region || self.configuration.saving {
            return Action::None;
        }
        let saved_scroll = self.region_screen.settings_scroll;
        match event {
            RegionEvent::Done => {
                let Some(row) = self
                    .region_screen
                    .selected
                    .and_then(|i| self.region_screen.rows.get(i))
                else {
                    return Action::None;
                };
                let preference = row.value();
                if !self.set_configuration_region(preference) {
                    self.enter(Page::Settings);
                }
            }
            RegionEvent::Cancel => self.enter(Page::Settings),
        }
        self.pixel_scroll.set(saved_scroll);
        self.region_screen.scroll.get_mut().release();
        self.region_screen.focused = false;
        self.region_screen.last_click = None;
        Action::None
    }

    /// Handle a press outside the footer. The result requests a click sound for
    /// double-click acceptance; it is NOT the title State::press exit flag.
    pub(in crate::title) fn region_press(&mut self, x: f32, y: f32, size: (f32, f32)) -> bool {
        if self.page != Page::Region || self.configuration.saving {
            return false;
        }
        self.region_screen.layout(size.1);
        let screen = &mut self.region_screen;
        let content = screen.content_height();
        let left = (size.0 / 2.0).floor() - ROW_WIDTH / 2.0;
        if screen
            .scroll
            .get_mut()
            .press((x, y), left + ROW_WIDTH + 8.0, TOP, size.1 - TOP, content)
        {
            return false;
        }
        if !(TOP..size.1 - TOP).contains(&y) || !(left..left + ROW_WIDTH).contains(&x) {
            screen.last_click = None;
            return false;
        }
        let relative = y - TOP - 2.0 + screen.scroll.get().amount.floor();
        if relative < 0.0 {
            return false;
        }
        let index = (relative / ROW_HEIGHT) as usize;
        if index >= screen.rows.len() {
            return false;
        }
        let now = std::time::Instant::now();
        let double_click = screen.last_click.is_some_and(|(previous, time)| {
            previous == index && now.duration_since(time).as_millis() < 250
        });
        screen.last_click = Some((index, now));
        screen.selected = Some(index);
        screen.focused = true;
        let row_y = TOP + 2.0 + index as f32 * ROW_HEIGHT - screen.scroll.get().amount.floor();
        if row_y + 2.0 < TOP || row_y + ROW_HEIGHT - 2.0 > size.1 - TOP {
            screen.ensure_visible();
        }
        if double_click {
            self.region_event(RegionEvent::Done);
        }
        double_click
    }

    pub(super) fn region_scroll(&mut self, delta: f32, h: f32) -> bool {
        if self.page != Page::Region {
            return false;
        }
        self.region_screen.layout(h);
        if !self.configuration.saving {
            let content = self.region_screen.content_height();
            self.region_screen.scroll.get_mut().wheel(
                delta,
                ROW_HEIGHT / 2.0,
                TOP,
                (h - TOP).max(TOP),
                content,
            );
        }
        true
    }

    pub(in crate::title) fn region_drag(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if self.page != Page::Region || !self.region_screen.scroll.get().held() {
            return false;
        }
        let content = self.region_screen.content_height();
        if size.1 > TOP * 2.0 + 40.0 && !self.configuration.saving {
            self.region_screen
                .scroll
                .get_mut()
                .drag(cursor.1, TOP, size.1 - TOP, content);
        }
        true
    }

    pub(in crate::title) fn region_release(&mut self) {
        self.region_screen.scroll.get_mut().release();
    }

    /// The list is a single Tab stop, followed by Done and Cancel.
    pub(in crate::title) fn region_tab(&mut self, reverse: bool, focus: &mut Option<u8>) -> bool {
        if self.page != Page::Region {
            return false;
        }
        if self.configuration.saving {
            return true;
        }
        let screen = &mut self.region_screen;
        let current = if focus.is_some() {
            *focus
        } else if screen.focused {
            Some(0)
        } else {
            None
        };
        let entries = if screen.selected.is_some() {
            &[0, DONE, CANCEL][..]
        } else {
            &[0, CANCEL][..]
        };
        let index = current.and_then(|id| entries.iter().position(|entry| *entry == id));
        let next = if reverse {
            index.map_or(entries.len() - 1, |i| {
                (i + entries.len() - 1) % entries.len()
            })
        } else {
            index.map_or(0, |i| (i + 1) % entries.len())
        };
        screen.focused = entries[next] == 0;
        *focus = (entries[next] != 0).then_some(entries[next]);
        if screen.focused {
            screen.focus_list(reverse);
        }
        true
    }

    /// Consume arrows only on this screen, keeping the visible focus in the list
    /// or footer. At a list edge ObjectSelectionList clears its selection.
    pub(in crate::title) fn region_arrow(
        &mut self,
        key: KeyCode,
        size: (f32, f32),
        focus: &mut Option<u8>,
    ) -> bool {
        if self.page != Page::Region
            || !matches!(
                key,
                KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight
            )
        {
            return false;
        }
        if self.configuration.saving {
            return true;
        }
        self.region_screen.layout(size.1);
        let screen = &mut self.region_screen;
        if let Some(button) = *focus {
            screen.focused = false;
            match key {
                KeyCode::ArrowUp => {
                    *focus = None;
                    screen.focus_list(true);
                }
                KeyCode::ArrowLeft if button == CANCEL && screen.selected.is_some() => {
                    *focus = Some(DONE)
                }
                KeyCode::ArrowRight if button == DONE => *focus = Some(CANCEL),
                _ => {}
            }
            return true;
        }
        if !screen.focused {
            screen.focus_list(key == KeyCode::ArrowUp);
            return true;
        }
        let next = match key {
            KeyCode::ArrowUp => screen.selected.and_then(|i| i.checked_sub(1)),
            KeyCode::ArrowDown => screen
                .selected
                .map_or(Some(0), |i| (i + 1 < screen.rows.len()).then_some(i + 1)),
            _ => None,
        };
        screen.selected = next;
        if next.is_some() {
            screen.ensure_visible();
        } else {
            screen.focused = false;
            if key == KeyCode::ArrowDown {
                *focus = Some(CANCEL);
            }
        }
        true
    }

    /// Invoke for Enter/Space when no footer button has focus.
    pub(in crate::title) fn region_activate(&mut self) -> bool {
        if self.page != Page::Region
            || !self.region_screen.focused
            || self.region_screen.selected.is_none()
            || self.configuration.saving
        {
            return false;
        }
        self.region_event(RegionEvent::Done);
        true
    }

    pub(super) fn region_draw(&self, atlas: &HudAtlas, batch: &mut HudBatch) -> bool {
        if self.page != Page::Region {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        let screen = &self.region_screen;
        screen.layout(h);
        let bottom = (h - TOP).max(TOP);
        // AbstractSelectionList uses the list's right/bottom plus integer scroll
        // as the background UV origin, so the texture moves with the entries.
        let first = batch.vertices.len();
        let scroll = screen.scroll.get().amount.floor();
        let phase_x = w.rem_euclid(32.0);
        let phase_y = (bottom + scroll).rem_euclid(32.0);
        ui::tiled(
            batch,
            atlas,
            "gui/menu_list_background",
            -phase_x,
            TOP - phase_y,
            w + phase_x,
            bottom - TOP + phase_y,
            32.0,
            u32::MAX,
        );
        batch.clip_since(first, 0.0, TOP, w, bottom);
        super::super::centered(
            atlas,
            batch,
            &tr("mco.configure.world.region_preference.title"),
            12.0,
            u32::MAX,
        );
        let first = batch.vertices.len();
        let left = (w / 2.0).floor() - ROW_WIDTH / 2.0;
        for (i, row) in screen.rows.iter().enumerate() {
            let y = TOP + 2.0 + i as f32 * ROW_HEIGHT - scroll;
            if y + ROW_HEIGHT < TOP || y > bottom {
                continue;
            }
            if screen.selected == Some(i) {
                let outline = if screen.focused {
                    u32::MAX
                } else {
                    rgba(128, 128, 128, 255)
                };
                batch.fill(atlas, left, y, ROW_WIDTH, ROW_HEIGHT, outline);
                batch.fill(
                    atlas,
                    left + 1.0,
                    y + 1.0,
                    ROW_WIDTH - 2.0,
                    ROW_HEIGHT - 2.0,
                    rgba(0, 0, 0, 255),
                );
            }
            batch.text(
                atlas,
                &tr(&row.label_key),
                left + 7.0,
                y + 4.0,
                u32::MAX,
                true,
            );
            if row.region.is_some() {
                batch.sprite_scaled(
                    atlas,
                    quality_icon(row.quality),
                    left + ROW_WIDTH - 20.0,
                    y + 4.0,
                    10.0,
                    8.0,
                    u32::MAX,
                );
            }
        }
        batch.clip_since(first, 0.0, TOP, w, bottom);
        ui::separators(batch, atlas, TOP, bottom, false);
        if bottom - TOP >= 40.0 {
            ui::scrollbar(
                batch,
                atlas,
                left + ROW_WIDTH + 8.0,
                TOP,
                bottom,
                screen.content_height(),
                scroll,
            );
        }
        true
    }
}
