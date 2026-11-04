//! 26.3 RealmsConfigureWorldScreen and its four GridLayoutTab children.
use super::*;
use crate::ui;

#[derive(Clone, Debug, Default)]
pub(super) struct Configuration {
    draft: Option<SettingsDraft>,
    player_widths: Cell<[f32; 3]>,
    info_height: Cell<f32>,
    dragging_distance: Option<bool>,
    slider_keys: Option<bool>,
    dragging_scroll: bool,
    pub(super) saving: bool,
}

#[derive(Clone, Debug)]
struct SettingsDraft {
    id: i64,
    name: String,
    description: String,
    preference: Value,
    render: Option<u32>,
    simulation: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigurationFocus {
    Field(usize),
    Button(u8),
    Player(usize, bool),
}

const TABS: [(&str, Page); 4] = [
    ("mco.configure.worlds.title", Page::Configure),
    ("mco.configure.world.players.title", Page::Players),
    ("mco.configure.world.subscription.tab", Page::Subscription),
    ("mco.configure.world.settings.title", Page::Settings),
];

fn template_image(id: &Value, image: &Value) -> Option<(String, String)> {
    let id = id.as_i64().or_else(|| id.as_str()?.parse().ok())?;
    let image = image.as_str().filter(|image| !image.is_empty())?;
    (id != -1).then(|| (id.to_string(), image.to_owned()))
}

fn grid_y(h: f32, height: f32) -> f32 {
    24.0 + ((h - 57.0 - height) / 6.0).trunc()
}
fn tabs(w: f32) -> (f32, f32) {
    let width = w.min(400.0) - 28.0;
    // Java divides integers before rounding towards the next multiple of two.
    (
        (((w - width) / 2.0).floor() / 2.0).ceil() * 2.0,
        ((width / 4.0).floor() / 2.0).ceil() * 2.0,
    )
}

impl State {
    pub fn settings_saved(&mut self) {
        self.configuration.saving = false;
        self.configuration.draft = None;
        self.loading = false;
        if self.page == Page::Settings {
            self.fields.clear();
            self.enter_settings();
        }
    }
    pub fn set_configuration_error(&mut self, message: impl Into<String>) {
        self.enter(Page::ConfigurationError);
        self.availability_error_kind = Some(AvailabilityErrorKind::Service);
        self.availability_title = tr("mco.errorMessage.generic");
        self.set_error(message);
    }
    pub(super) fn is_configuration(&self) -> bool {
        matches!(
            self.page,
            Page::Configure | Page::Slots | Page::Players | Page::Subscription | Page::Settings
        )
    }
    fn configuration_tab(&self) -> usize {
        match self.page {
            Page::Players => 1,
            Page::Subscription => 2,
            Page::Settings => 3,
            _ => 0,
        }
    }
    fn minigame(&self) -> bool {
        text(&self.server, "worldType") == "MINIGAME"
    }
    fn active_slot(&self) -> i32 {
        number(&self.server, "activeSlot").clamp(1, 3) as i32
    }
    fn slot_data(&self, slot: i32) -> Value {
        self.server
            .get("slots")
            .and_then(Value::as_array)
            .and_then(|slots| slots.iter().find(|v| number(v, "slotId") == slot as i64))
            .cloned()
            .unwrap_or(Value::Null)
    }
    fn slot_options_for(&self, slot: i32) -> Value {
        let slot = self.slot_data(slot);
        let value = slot.get("options").cloned().unwrap_or(Value::Null);
        value
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(value)
    }
    fn slot_active(&self, slot: i32) -> bool {
        if slot == 4 {
            self.minigame()
        } else {
            !self.minigame() && self.active_slot() == slot
        }
    }
    fn slot_empty(&self, slot: i32) -> bool {
        if slot == 4 {
            number(&self.server, "minigameId") == -1
        } else {
            self.slot_options_for(slot)
                .get("empty")
                .and_then(Value::as_bool)
                == Some(true)
        }
    }
    fn configuration_template_image(&self, slot: i32) -> Option<(String, String)> {
        let (id, image) = if slot == 4 {
            (
                self.server.get("minigameId")?,
                self.server.get("minigameImage")?,
            )
        } else {
            if self.slot_empty(slot) {
                return None;
            }
            let options = self.slot_options_for(slot);
            let id = options
                .get("worldTemplateId")
                .or_else(|| options.get("templateId"))?;
            let image = options
                .get("worldTemplateImage")
                .or_else(|| options.get("templateImage"))?;
            return template_image(id, image);
        };
        template_image(id, image)
    }

    /// Base64 DTO images share RealmsTextureManager's template-id cache with the picker.
    pub fn configuration_template_images(&self) -> Vec<(String, String)> {
        (1..=4)
            .filter_map(|slot| self.configuration_template_image(slot))
            .collect()
    }

    fn slot_name(&self, slot: i32) -> String {
        if slot == 4 {
            return tr("mco.worldSlot.minigame");
        }
        let name = text(&self.slot_options_for(slot), "slotName");
        if !name.is_empty() {
            name
        } else if self.slot_empty(slot) {
            tr("mco.configure.world.slot.empty")
        } else {
            translated("mco.configure.world.slot", &[slot.to_string()])
        }
    }
    pub(super) fn remember_settings(&mut self) {
        if self.page == Page::Settings && self.fields.len() == 2 {
            let name = self.fields[0].value.clone();
            let description = self.fields[1].value.clone();
            if let Some(draft) = &mut self.configuration.draft {
                draft.name = name;
                draft.description = description;
            }
        }
    }
    pub(super) fn enter_settings(&mut self) {
        if self
            .configuration
            .draft
            .as_ref()
            .is_none_or(|d| d.id != self.id())
        {
            self.configuration.draft = Some(SettingsDraft {
                id: self.id(),
                name: text(&self.server, "name"),
                description: text(&self.server, "motd"),
                preference: self
                    .region_normalize_preference(self.configuration_region_preference()),
                render: self.distance(false).map(|v| v.2),
                simulation: self.distance(true).map(|v| v.2),
            });
        }
        let draft = self.configuration.draft.as_ref().unwrap().clone();
        self.add_field("mco.configure.world.name", draft.name, 32);
        self.add_field("mco.configure.world.description", draft.description, 32);
    }
    pub(super) fn configuration_save(
        &mut self,
        continuation: Option<RealmsAction>,
    ) -> Option<RealmsAction> {
        self.remember_settings();
        let draft = self.configuration.draft.as_ref()?;
        let render = draft
            .render
            .filter(|v| self.distance(false).is_some_and(|d| d.2 != *v));
        let simulation = draft
            .simulation
            .filter(|v| self.distance(true).is_some_and(|d| d.2 != *v));
        let name = draft.name.trim().to_owned();
        let old_preference = self
            .server
            .get("regionSelectionPreference")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"regionSelectionPreference":0,"preferredRegion":null}));
        if name == text(&self.server, "name")
            && draft.description == text(&self.server, "motd")
            && draft.preference == old_preference
            && render.is_none()
            && simulation.is_none()
        {
            return None;
        }
        Some(RealmsAction::SaveSettings {
            id: draft.id,
            name,
            description: draft.description.clone(),
            preference: draft.preference.clone(),
            render_distance: render,
            simulation_distance: simulation,
            continuation: continuation.map(Box::new),
        })
    }
    pub fn back_action(&mut self) -> (Action, bool) {
        if let Some(result) = self
            .world_back_action()
            .or_else(|| self.reset_back_action())
        {
            return result;
        }
        if let Some(exit) = self.player_back() {
            return (Action::None, exit);
        }
        if self.configuration.saving {
            return (Action::None, false);
        }
        let save = if self.page == Page::Settings && !self.loading {
            self.configuration_save(None)
        } else {
            None
        };
        let back = self.back();
        (save.map(|a| self.dispatch(a)).unwrap_or(Action::None), back)
    }
    pub(super) fn select_configuration_tab(&mut self, page: Page) -> Action {
        if self.page == page || (self.page == Page::Slots && page == Page::Configure) {
            return Action::None;
        }
        let request = if page == Page::Subscription && self.subscription.is_null() {
            Some(RealmsAction::Subscription(self.id()))
        } else {
            None
        };
        let save = if self.page == Page::Settings {
            self.configuration_save(request.clone())
        } else {
            None
        };
        self.enter(page);
        save.or(request)
            .map(|a| self.dispatch(a))
            .unwrap_or(Action::None)
    }
    pub(super) fn configuration_region_preference(&self) -> Value {
        self.configuration
            .draft
            .as_ref()
            .filter(|d| d.id == self.id())
            .map(|d| d.preference.clone())
            .or_else(|| {
                self.server
                    .get("regionSelectionPreference")
                    .filter(|v| v.is_object())
                    .cloned()
            })
            .unwrap_or_else(|| json!({"regionSelectionPreference":0,"preferredRegion":null}))
    }

    /// TabNavigationBar consumes Ctrl+digits even when that particular tab is disabled.
    pub(in crate::title) fn configuration_tab_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        reverse: bool,
        focus: &mut Option<u8>,
    ) -> Option<(Action, bool)> {
        if !self.is_configuration() {
            return None;
        }
        let current = self.configuration_tab();
        let mut next = super::super::create_world::next_tab_index(current, key, reverse)?;
        if self.configuration.saving {
            return Some((Action::None, false));
        }
        let controls = self.configuration_controls()?;
        if key == winit::keyboard::KeyCode::Tab {
            next = current;
            for _ in 0..TABS.len() {
                next = (next + if reverse { TABS.len() - 1 } else { 1 }) % TABS.len();
                if controls[next].enabled {
                    break;
                }
            }
        }
        next = next.min(TABS.len() - 1);
        if next == current || !controls[next].enabled {
            return Some((Action::None, false));
        }
        let bar_focused = focus.is_some_and(|id| (10..14).contains(&id));
        let action = self.select_configuration_tab(TABS[next].1.clone());
        self.focused = None;
        *focus = bar_focused.then_some(10 + next as u8);
        Some((action, true))
    }

    fn configuration_focus(&self, focus: Option<u8>) -> Option<ConfigurationFocus> {
        self.focused.map(ConfigurationFocus::Field).or_else(|| {
            focus.map(|id| {
                if self.page == Page::Players && id >= 18 {
                    ConfigurationFocus::Player(
                        self.invite_visible.get().0 + usize::from(id - 18) / 2,
                        (id - 18) % 2 == 1,
                    )
                } else {
                    ConfigurationFocus::Button(id)
                }
            })
        })
    }

    fn configuration_widgets(&self, size: (f32, f32)) -> Vec<(ConfigurationFocus, [f32; 4])> {
        let buttons = self
            .configuration_buttons(size.0, size.1)
            .unwrap_or_default();
        let mut widgets = Vec::new();
        for button in buttons
            .iter()
            .filter(|b| b.enabled && (b.id == 10 + self.configuration_tab() as u8 || b.id >= 16))
        {
            if self.page != Page::Players || button.id < 18 {
                widgets.push((
                    ConfigurationFocus::Button(button.id),
                    [
                        button.x,
                        button.y,
                        button.w,
                        self.configuration_button_height(button).unwrap_or(20.0),
                    ],
                ));
            }
        }
        if self.page == Page::Settings && !self.loading {
            let index = usize::from(
                widgets
                    .first()
                    .is_some_and(|(f, _)| matches!(f, ConfigurationFocus::Button(10..=13))),
            );
            for i in (0..self.fields.len()).rev() {
                widgets.insert(
                    index,
                    (
                        ConfigurationFocus::Field(i),
                        self.configuration_field_rect(i, size.0),
                    ),
                );
            }
        }
        if self.page == Page::Players && !self.loading {
            let widths = self.configuration.player_widths.get();
            let c = (size.0 / 2.0).floor();
            let rows = self
                .server
                .get("players")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            let mut row_widgets = Vec::new();
            for (i, player) in rows.enumerate() {
                let op = player.get("operator").and_then(Value::as_bool) == Some(true);
                let rw = widths[2].max(48.0);
                let ow = widths[usize::from(op)].max(64.0);
                let y = self.players_top(size.1) + 29.0 + i as f32 * 36.0
                    - self.pixel_scroll.get().floor();
                row_widgets.push((
                    ConfigurationFocus::Player(i, false),
                    [c + 148.0 - rw - ow - 8.0, y, ow, 20.0],
                ));
                row_widgets.push((
                    ConfigurationFocus::Player(i, true),
                    [c + 148.0 - rw, y, rw, 20.0],
                ));
            }
            let index = usize::from(
                widgets
                    .first()
                    .is_some_and(|(f, _)| matches!(f, ConfigurationFocus::Button(10..=13))),
            );
            widgets.splice(index..index, row_widgets);
        }
        widgets.extend(
            buttons
                .iter()
                .filter(|b| b.enabled && (14..16).contains(&b.id))
                .map(|b| (ConfigurationFocus::Button(b.id), [b.x, b.y, b.w, 20.0])),
        );
        widgets
    }

    fn configuration_focus_widget(
        &mut self,
        target: ConfigurationFocus,
        size: (f32, f32),
        focus: &mut Option<u8>,
        tab: bool,
    ) {
        self.configuration.slider_keys = None;
        if let Some((_, [_, y, _, height])) = self
            .configuration_widgets(size)
            .into_iter()
            .find(|(f, _)| *f == target)
        {
            let area = match target {
                ConfigurationFocus::Field(_) | ConfigurationFocus::Button(16..=u8::MAX)
                    if self.page == Page::Settings =>
                {
                    Some((self.settings_top(size.1), (size.1 - 59.0).max(1.0)))
                }
                ConfigurationFocus::Player(..) => {
                    Some((self.players_top(size.1), (size.1 - 93.0).max(1.0)))
                }
                _ => None,
            };
            if let Some((top, view)) = area {
                let delta = if y < top {
                    y - top - 10.0
                } else if y + height > top + view {
                    y + height - top - view + 10.0
                } else {
                    0.0
                };
                self.pixel_scroll
                    .set((self.pixel_scroll.get() + delta).max(0.0));
                self.layout_configuration(size.1);
            }
        }
        self.focused = None;
        *focus = None;
        match target {
            ConfigurationFocus::Field(i) => self.focused = Some(i),
            ConfigurationFocus::Button(id) => {
                *focus = Some(id);
                if tab
                    && self.page == Page::Settings
                    && self.settings_distances()
                    && (16..18).contains(&id)
                {
                    self.configuration.slider_keys = Some(id == 17);
                }
            }
            ConfigurationFocus::Player(row, remove) => {
                let index = (row - self.invite_visible.get().0) * 2 + usize::from(remove);
                *focus = u8::try_from(index + 18).ok();
            }
        }
    }

    pub(in crate::title) fn configuration_tab_focus(
        &mut self,
        reverse: bool,
        size: (f32, f32),
        focus: &mut Option<u8>,
    ) -> bool {
        if !self.is_configuration() {
            return false;
        }
        if self.configuration.saving {
            return true;
        }
        let current = self.configuration_focus(*focus);
        let widgets = self.configuration_widgets(size);
        if widgets.is_empty() {
            return true;
        }
        let index = current.and_then(|f| widgets.iter().position(|(target, _)| *target == f));
        let next = if reverse {
            index.map_or(widgets.len() - 1, |i| {
                (i + widgets.len() - 1) % widgets.len()
            })
        } else {
            index.map_or(0, |i| (i + 1) % widgets.len())
        };
        self.configuration_focus_widget(widgets[next].0, size, focus, true);
        true
    }

    pub(in crate::title) fn configuration_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        size: (f32, f32),
        focus: &mut Option<u8>,
    ) -> Option<Action> {
        use winit::keyboard::KeyCode;
        if !self.is_configuration() {
            return None;
        }
        if self.configuration.saving {
            return Some(Action::None);
        }
        let current = self.configuration_focus(*focus);
        if let Some(ConfigurationFocus::Button(id @ 10..=13)) = current
            && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight)
        {
            let controls = self.configuration_controls().unwrap_or_default();
            let next = if key == KeyCode::ArrowLeft {
                (10..id)
                    .rev()
                    .find(|i| controls[usize::from(*i - 10)].enabled)
            } else {
                (id + 1..14).find(|i| controls[usize::from(*i - 10)].enabled)
            };
            return Some(next.map_or(Action::None, |id| {
                let action = self.select_configuration_tab(TABS[usize::from(id - 10)].1.clone());
                self.focused = None;
                *focus = Some(id);
                action
            }));
        }
        if let Some(ConfigurationFocus::Button(id @ 16..=17)) = current
            && self.page == Page::Settings
            && self.settings_distances()
            && self.configuration.slider_keys == Some(id == 17)
            && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight)
        {
            let simulation = id == 17;
            if let Some((min, max, current)) = self.distance(simulation)
                && let Some(draft) = &mut self.configuration.draft
            {
                let value = if simulation {
                    &mut draft.simulation
                } else {
                    &mut draft.render
                };
                *value = Some(if key == KeyCode::ArrowLeft {
                    value.unwrap_or(current).saturating_sub(1).max(min)
                } else {
                    value.unwrap_or(current).saturating_add(1).min(max)
                });
            }
            return Some(Action::None);
        }
        if self.focused.is_some() && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
            return Some(Action::None);
        }
        let widgets = self.configuration_widgets(size);
        let Some((_, [x, y, width, height])) = widgets.iter().find(|(f, _)| Some(*f) == current)
        else {
            if let Some((target, _)) = widgets.first() {
                self.configuration_focus_widget(*target, size, focus, false);
            }
            return Some(Action::None);
        };
        let (cx, cy) = (x + width / 2.0, y + height / 2.0);
        let candidates = widgets
            .iter()
            .filter_map(|(target, [x, y, width, height])| {
                let (dx, dy) = (x + width / 2.0 - cx, y + height / 2.0 - cy);
                let valid = match key {
                    KeyCode::ArrowUp => dy < -4.0,
                    KeyCode::ArrowDown => dy > 4.0,
                    KeyCode::ArrowLeft => dx < -4.0,
                    KeyCode::ArrowRight => dx > 4.0,
                    _ => false,
                };
                let distance = if matches!(key, KeyCode::ArrowUp | KeyCode::ArrowDown) {
                    dy.abs() + dx.abs() * 2.0
                } else {
                    dx.abs() + dy.abs() * 2.0
                };
                valid.then_some((*target, distance))
            })
            .collect::<Vec<_>>();
        let content = |f: ConfigurationFocus| {
            matches!(
                f,
                ConfigurationFocus::Field(_)
                    | ConfigurationFocus::Player(..)
                    | ConfigurationFocus::Button(16..=u8::MAX)
            )
        };
        let prefer_content =
            current.is_some_and(content) && candidates.iter().any(|(f, _)| content(*f));
        if let Some((target, _)) = candidates
            .into_iter()
            .filter(|(f, _)| !prefer_content || content(*f))
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            self.configuration_focus_widget(target, size, focus, false);
        }
        Some(Action::None)
    }

    /// AbstractSliderButton.keyPressed toggles adjustment without a pointer press.
    pub(in crate::title) fn configuration_activate(&mut self, focus: Option<u8>) -> bool {
        if self.page != Page::Settings
            || !self.settings_distances()
            || !focus.is_some_and(|id| (16..18).contains(&id))
        {
            return false;
        }
        if !self.loading && !self.configuration.saving {
            let simulation = focus == Some(17);
            self.configuration.slider_keys =
                (self.configuration.slider_keys != Some(simulation)).then_some(simulation);
        }
        true
    }

    pub(super) fn configuration_controls(&self) -> Option<Vec<Control>> {
        if !self.is_configuration() {
            return None;
        }
        let mut out = Vec::new();
        let expired = self.server.get("expired").and_then(Value::as_bool) == Some(true);
        let ready = !self.loading && self.id() > 0;
        for (key, page) in TABS {
            out.push(Control {
                label: tr(key),
                enabled: ready && (page != Page::Settings || !expired),
                event: Event::ConfigurationTab(page),
            });
        }
        out.push(Control {
            label: tr("mco.selectServer.play"),
            enabled: ready && !expired && text(&self.server, "state") == "OPEN",
            event: Event::Request(RealmsAction::Join(self.id())),
        });
        out.push(Control {
            label: tr("gui.back"),
            enabled: !self.configuration.saving,
            event: Event::Back,
        });
        if self.loading || self.id() <= 0 {
            return Some(out);
        }
        let mut add = |key: &str, enabled: bool, event| {
            out.push(Control {
                label: tr(key),
                enabled: ready && enabled,
                event,
            })
        };
        match self.page {
            Page::Configure | Page::Slots => {
                for slot in 1..=4 {
                    out.push(Control {
                        label: self.slot_name(slot),
                        enabled: ready
                            && !self.slot_active(slot)
                            && !(expired && self.slot_empty(slot)),
                        event: Event::ConfigurationSlot(slot),
                    });
                }
                let mut add = |key: &str, enabled: bool, event| {
                    out.push(Control {
                        label: tr(key),
                        enabled: ready && enabled,
                        event,
                    })
                };
                add(
                    "mco.configure.world.buttons.options",
                    !expired && !self.minigame(),
                    Event::Page(Page::SlotOptions),
                );
                add(
                    "mco.configure.world.backup",
                    !self.minigame(),
                    Event::Request(RealmsAction::Backups {
                        id: self.id(),
                        slot: self.active_slot(),
                    }),
                );
                add(
                    if self.minigame() {
                        "mco.configure.world.buttons.switchminigame"
                    } else if self.slot_empty(self.active_slot()) {
                        "mco.configure.world.buttons.newworld"
                    } else {
                        "mco.configure.world.buttons.resetworld"
                    },
                    !expired,
                    if self.minigame() {
                        Event::ResetScreen(reset_screens::ResetEvent::OpenTemplates {
                            world_type: "MINIGAME".into(),
                            warning: false,
                        })
                    } else {
                        Event::ResetScreen(reset_screens::ResetEvent::Open {
                            slot: self.active_slot(),
                            empty: self.slot_empty(self.active_slot()),
                        })
                    },
                );
            }
            Page::Players => {
                add(
                    "mco.configure.world.buttons.invite",
                    true,
                    Event::Page(Page::InvitePlayer),
                );
                add(
                    "mco.configure.world.buttons.invite_codes",
                    true,
                    Event::Request(RealmsAction::InviteLink(self.id())),
                );
                let (first, count) = self.invite_visible.get();
                for player in self
                    .server
                    .get("players")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .skip(first)
                    .take(count.min(118))
                {
                    let op = player.get("operator").and_then(Value::as_bool) == Some(true);
                    add(
                        if op {
                            "mco.configure.world.invites.ops.tooltip"
                        } else {
                            "mco.configure.world.invites.normal.tooltip"
                        },
                        true,
                        Event::Request(RealmsAction::SetOperator {
                            id: self.id(),
                            uuid: text(player, "uuid"),
                            operator: !op,
                        }),
                    );
                    add(
                        "mco.configure.world.invites.remove.tooltip",
                        true,
                        Event::Confirm(
                            RealmsAction::RemovePlayer {
                                id: self.id(),
                                uuid: text(player, "uuid"),
                            },
                            "mco.question",
                            translated(
                                "mco.configure.world.uninvite.player",
                                &[text(player, "name")],
                            ),
                        ),
                    );
                }
            }
            Page::Subscription => {
                add(
                    "mco.configure.world.subscription.extend",
                    true,
                    Event::Request(RealmsAction::OpenSubscription(self.id())),
                );
                add(
                    "mco.configure.world.delete.button",
                    expired,
                    Event::Confirm(
                        RealmsAction::DeleteRealm(self.id()),
                        "mco.configure.world.delete.question.line1",
                        String::new(),
                    ),
                );
            }
            Page::Settings => {
                if self.distance(false).is_some() && self.distance(true).is_some() {
                    for simulation in [false, true] {
                        add(
                            "options.chunks",
                            true,
                            Event::ConfigurationDistance(simulation),
                        );
                    }
                }
                add(
                    "mco.configure.world.buttons.region_preference",
                    true,
                    Event::Page(Page::Region),
                );
                let open = text(&self.server, "state") == "OPEN";
                add(
                    if open {
                        "mco.configure.world.buttons.close"
                    } else {
                        "mco.configure.world.buttons.open"
                    },
                    true,
                    if open {
                        Event::Confirm(
                            RealmsAction::CloseWorld(self.id()),
                            "mco.configure.world.close.question.title",
                            tr("mco.configure.world.close.question.line1"),
                        )
                    } else {
                        Event::Request(RealmsAction::OpenWorld(self.id()))
                    },
                );
            }
            _ => {}
        }
        Some(out)
    }
    fn settings_distances(&self) -> bool {
        self.distance(false).is_some() && self.distance(true).is_some()
    }
    fn settings_height(&self) -> f32 {
        if self.settings_distances() {
            290.0
        } else {
            192.0
        }
    }
    fn settings_top(&self, h: f32) -> f32 {
        grid_y(h, self.settings_height().min((h - 59.0).max(1.0)))
    }
    fn players_top(&self, h: f32) -> f32 {
        grid_y(h, h - 65.0)
    }
    fn players_bottom(&self, h: f32) -> f32 {
        self.players_top(h) + h - 93.0
    }
    fn layout_configuration(&self, h: f32) {
        self.viewport_height.set(h);
        let (content, view) = if self.page == Page::Players {
            let count = self
                .server
                .get("players")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            (21.0 + count as f32 * 36.0, (h - 93.0).max(1.0))
        } else if self.page == Page::Settings {
            (self.settings_height(), (h - 59.0).max(1.0))
        } else {
            return;
        };
        self.content_height.set(content);
        self.pixel_scroll.set(
            self.pixel_scroll
                .get()
                .clamp(0.0, (content - view).max(0.0)),
        );
        if self.page == Page::Players {
            let first = ((self.pixel_scroll.get() - 21.0).max(0.0) / 36.0).floor() as usize;
            self.invite_visible
                .set((first, (view / 36.0).ceil() as usize + 1));
        }
    }
    pub(super) fn configuration_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        if !self.is_configuration() {
            return None;
        }
        self.layout_configuration(h);
        let c = (w / 2.0).floor();
        let (tx, tw) = tabs(w);
        let mut slots_x = c - 139.0;
        Some(
            self.configuration_controls()?
                .into_iter()
                .enumerate()
                .map(|(i, control)| {
                    let (x, y, width) = match i {
                        0..=3 => (tx + i as f32 * tw, 0.0, tw),
                        4..=5 => (c - 154.0 + (i - 4) as f32 * 158.0, h - 26.0, 150.0),
                        _ => match self.page {
                            Page::Configure | Page::Slots if i < 10 => {
                                let size = if self.slot_active((i - 5) as i32) {
                                    80.0
                                } else {
                                    50.0
                                };
                                let x = slots_x;
                                slots_x += size + 16.0;
                                (x, grid_y(h, 176.0) + 80.0 - size, size)
                            }
                            Page::Configure | Page::Slots => (
                                c - 75.0,
                                grid_y(h, 176.0) + 100.0 + (i - 10) as f32 * 28.0,
                                150.0,
                            ),
                            Page::Players if i < 8 => (
                                c - 154.0 + (i - 6) as f32 * 158.0,
                                self.players_bottom(h) + 8.0,
                                150.0,
                            ),
                            Page::Players => {
                                let row = self.invite_visible.get().0 + (i - 8) / 2;
                                let player = self
                                    .server
                                    .get("players")
                                    .and_then(Value::as_array)
                                    .and_then(|p| p.get(row));
                                let op = player
                                    .and_then(|p| p.get("operator"))
                                    .and_then(Value::as_bool)
                                    == Some(true);
                                let widths = self.configuration.player_widths.get();
                                let rw = widths[2].max(48.0);
                                let ow = widths[usize::from(op)].max(64.0);
                                let remove = (i - 8) % 2 == 1;
                                (
                                    c + 148.0 - rw - if remove { 0.0 } else { ow + 8.0 },
                                    self.players_top(h) + 21.0 + row as f32 * 36.0 + 8.0
                                        - self.pixel_scroll.get().floor(),
                                    if remove { rw } else { ow },
                                )
                            }
                            Page::Subscription => (
                                c - 100.0,
                                grid_y(h, 144.0 + self.configuration.info_height.get())
                                    + if i == 6 { 76.0 } else { 110.0 },
                                200.0,
                            ),
                            Page::Settings => {
                                let top = self.settings_top(h) - self.pixel_scroll.get().floor();
                                let distances = self.settings_distances();
                                let y = if distances && i < 8 {
                                    119.0 + (i - 6) as f32 * 49.0
                                } else if i == if distances { 8 } else { 6 } {
                                    if distances { 226.0 } else { 128.0 }
                                } else {
                                    if distances { 262.0 } else { 164.0 }
                                };
                                (c - 106.0, top + y, 212.0)
                            }
                            _ => (0.0, 0.0, 0.0),
                        },
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
    pub(super) fn configuration_button_height(&self, b: &Button) -> Option<f32> {
        if !self.is_configuration() {
            return None;
        }
        Some(if b.id < 14 {
            24.0
        } else if matches!(self.page, Page::Configure | Page::Slots) && (16..20).contains(&b.id) {
            b.w
        } else {
            20.0
        })
    }
    pub(super) fn configuration_field_rect(&self, i: usize, w: f32) -> [f32; 4] {
        [
            (w / 2.0).floor() - 106.0,
            self.settings_top(self.viewport_height.get()) + 21.0 + i as f32 * 49.0
                - self.pixel_scroll.get().floor(),
            212.0,
            20.0,
        ]
    }
    pub(super) fn configuration_hit(&self, x: f32, y: f32, size: (f32, f32)) -> Option<u8> {
        self.configuration_buttons(size.0, size.1)?
            .into_iter()
            .find(|b| {
                let in_content = b.id < 16
                    || match self.page {
                        Page::Players if b.id >= 18 => {
                            (self.players_top(size.1)..self.players_bottom(size.1)).contains(&y)
                        }
                        _ => (24.0..size.1 - 35.0).contains(&y),
                    };
                in_content
                    && (b.x..b.x + b.w).contains(&x)
                    && (b.y..b.y + self.configuration_button_height(b).unwrap_or(20.0)).contains(&y)
            })
            .map(|b| b.id)
    }
    pub(super) fn press_configuration_slot(&mut self, slot: i32) -> Action {
        if slot == 4 {
            return self.reset_open_templates("MINIGAME", true);
        }
        self.slot = slot;
        if self.slot_empty(slot) {
            self.ask_action(
                Action::None,
                "mco.configure.world.slot.switch.question.line1",
                String::new(),
            );
            if let Some(confirmation) = &mut self.confirmation {
                confirmation.reset_slot = Some(slot);
            }
            return Action::None;
        }
        self.ask(
            RealmsAction::SwitchSlot {
                id: self.id(),
                slot,
            },
            "mco.configure.world.slot.switch.question.line1",
            String::new(),
        );
        Action::None
    }
    pub(super) fn configuration_distance(&mut self, simulation: bool, x: f32, width: f32) {
        let Some((min, max, _)) = self.distance(simulation) else {
            return;
        };
        let ratio = ui::slider_value((width / 2.0).floor() - 106.0, 212.0, x);
        let value = (min + (ratio * (max - min + 1) as f32).floor() as u32).min(max);
        if let Some(draft) = &mut self.configuration.draft {
            if simulation {
                draft.simulation = Some(value);
            } else {
                draft.render = Some(value);
            }
        }
        self.focused = None;
        self.configuration.slider_keys = Some(simulation);
        self.configuration.dragging_distance = Some(simulation);
    }
    pub(super) fn set_configuration_region(&mut self, preference: Value) -> bool {
        if self.configuration.draft.is_none() {
            return false;
        }
        self.configuration.draft.as_mut().unwrap().preference = preference;
        self.enter(Page::Settings);
        true
    }
    pub(super) fn scroll_configuration(&mut self, delta: f32, h: f32) {
        self.pixel_scroll
            .set((self.pixel_scroll.get() - delta * 18.0).max(0.0));
        self.layout_configuration(h);
    }
    pub(super) fn press_configuration_scrollbar(
        &mut self,
        x: f32,
        y: f32,
        size: (f32, f32),
    ) -> bool {
        let (sx, top, view) = if self.page == Page::Players {
            (
                (size.0 / 2.0).floor() + 154.0,
                self.players_top(size.1),
                (size.1 - 93.0).max(1.0),
            )
        } else if self.page == Page::Settings {
            (
                (size.0 / 2.0).floor() + 114.0,
                self.settings_top(size.1),
                (size.1 - 59.0).max(1.0),
            )
        } else {
            return false;
        };
        if (sx..sx + 6.0).contains(&x)
            && (top..top + view).contains(&y)
            && self.content_height.get() > view
        {
            self.configuration.dragging_scroll = true;
            self.drag_configuration((x, y), size);
            return true;
        }
        false
    }
    pub(in crate::title) fn drag_configuration(
        &mut self,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> bool {
        if self.configuration.dragging_scroll {
            let (top, view) = if self.page == Page::Players {
                (self.players_top(size.1), (size.1 - 93.0).max(1.0))
            } else if self.page == Page::Settings {
                (self.settings_top(size.1), (size.1 - 59.0).max(1.0))
            } else {
                return false;
            };
            let content = self.content_height.get();
            if content > view && view > 8.0 {
                let thumb = (view * view / content)
                    .floor()
                    .clamp(32.0_f32.min(view - 8.0), view - 8.0);
                self.pixel_scroll.set(
                    ((cursor.1 - top - thumb / 2.0) / (view - thumb) * (content - view))
                        .clamp(0.0, content - view),
                );
                self.layout_configuration(size.1);
            }
            return true;
        }
        if self.page != Page::Settings || self.loading {
            return false;
        }
        if let Some(simulation) = self.configuration.dragging_distance {
            self.configuration_distance(simulation, cursor.0, size.0);
            return true;
        }
        false
    }
    pub(in crate::title) fn release_configuration(&mut self) {
        self.configuration.dragging_scroll = false;
        self.configuration.dragging_distance = None;
    }
    pub(super) fn clear_configuration_drag(&mut self) {
        self.release_configuration();
        self.configuration.slider_keys = None;
    }
    pub(super) fn draw_configuration_tooltip(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
    ) -> bool {
        if !self.is_configuration() {
            return false;
        }
        let mut tooltip = None;
        if let Some(id) = self.configuration_hit(cursor.0, cursor.1, (batch.width, batch.height)) {
            tooltip = match id {
                13 if self.server.get("expired").and_then(Value::as_bool) == Some(true) => {
                    Some(tr("mco.configure.world.settings.expired"))
                }
                14 if text(&self.server, "state") == "CLOSED" => {
                    Some(tr("mco.play.button.realm.closed"))
                }
                16..=19
                    if matches!(self.page, Page::Configure | Page::Slots)
                        && !self.slot_active((id - 15) as i32) =>
                {
                    Some(tr(if id == 19 {
                        "mco.configure.world.slot.tooltip.minigame"
                    } else {
                        "mco.configure.world.slot.tooltip"
                    }))
                }
                16..=17 if self.page == Page::Settings && self.settings_distances() => {
                    Some(tr(if id == 16 {
                        "mco.configure.world.renderDistance.tooltip"
                    } else {
                        "mco.configure.world.simulationDistance.tooltip"
                    }))
                }
                _ => None,
            };
        }
        if self.page == Page::Settings {
            let rect = self.configuration_field_rect(0, batch.width);
            if (rect[0]..rect[0] + rect[2]).contains(&cursor.0)
                && (rect[1]..rect[1] + rect[3]).contains(&cursor.1)
                && self
                    .fields
                    .first()
                    .is_some_and(|f| f.value.trim().is_empty() || f.value.trim() != f.value)
            {
                tooltip = Some(tr("mco.configure.world.name.validation.whitespace"));
            }
        }
        if let Some(text) = tooltip {
            ui::tooltip(batch, atlas, &[Text::literal(text)], cursor);
        }
        true
    }
    pub(super) fn draw_configuration_button(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        b: &Button,
        highlighted: bool,
    ) -> bool {
        if !self.is_configuration() {
            return false;
        }
        let first = batch.vertices.len();
        if b.id < 14 {
            let selected = (b.id - 10) as usize == self.configuration_tab();
            let sprite = format!(
                "gui/sprites/widget/tab{}{}",
                if selected { "_selected" } else { "" },
                if highlighted { "_highlighted" } else { "" }
            );
            ui::nine_slice(batch, atlas, &sprite, b.x, b.y, b.w, 24.0, u32::MAX);
            if selected {
                ui::menu_background(batch, atlas, b.x + 2.0, 2.0, b.w - 4.0, 22.0, false);
                let width = HudBatch::text_width(atlas, &b.label).min(b.w - 4.0);
                batch.fill(
                    atlas,
                    b.x + ((b.w - width) / 2.0).floor(),
                    22.0,
                    width,
                    1.0,
                    u32::MAX,
                );
            }
            ui::menu_tab_label(
                batch,
                atlas,
                &Text::literal(&b.label),
                (b.x, 0.0, b.w, 24.0),
                selected,
                b.enabled,
                crate::hud_overlays::now_ms(),
            );
        } else if matches!(self.page, Page::Configure | Page::Slots) && (16..20).contains(&b.id) {
            let slot = (b.id - 15) as i32;
            let active = self.slot_active(slot);
            let color = if active {
                u32::MAX
            } else {
                rgba(143, 143, 143, 255)
            };
            let dynamic = self
                .configuration_template_image(slot)
                .map(|(id, _)| format!("realms/dynamic/{id}"));
            let texture = if let Some(texture) = dynamic
                .as_deref()
                .filter(|texture| atlas.sprites.contains_key(*texture))
            {
                texture
            } else if self.slot_empty(slot) || slot == 4 {
                "gui/realms/empty_frame"
            } else {
                match slot {
                    1 => "gui/title/background/panorama_0",
                    2 => "gui/title/background/panorama_2",
                    _ => "gui/title/background/panorama_3",
                }
            };
            batch.sprite_scaled(
                atlas,
                texture,
                b.x + 1.0,
                b.y + 1.0,
                b.w - 2.0,
                b.w - 2.0,
                color,
            );
            batch.sprite_scaled(
                atlas,
                "gui/sprites/widget/slot_frame",
                b.x,
                b.y,
                b.w,
                b.w,
                if highlighted && b.enabled {
                    u32::MAX
                } else if active {
                    rgba(204, 204, 204, 255)
                } else {
                    color
                },
            );
            let options = self.slot_options_for(slot);
            if options.get("hardcore").and_then(Value::as_bool) == Some(true)
                || self
                    .slot_data(slot)
                    .pointer("/settings/hardcore")
                    .and_then(Value::as_bool)
                    == Some(true)
            {
                batch.sprite_scaled(
                    atlas,
                    "gui/sprites/hud/heart/hardcore_full",
                    b.x + 3.0,
                    b.y + 4.0,
                    9.0,
                    8.0,
                    u32::MAX,
                );
            }
            ui::scrolling_text(
                batch,
                atlas,
                &Text::literal(&b.label),
                b.x + 1.0,
                b.x + b.w - 1.0,
                b.y + b.w - 14.0,
                b.y + b.w - 5.0,
                b.y + b.w - 14.0,
                u32::MAX,
            );
            if active && slot != 4 {
                let version = text(&options, "version");
                batch.text(
                    atlas,
                    &version,
                    b.x + (b.w - HudBatch::text_width(atlas, &version)) / 2.0,
                    b.y + b.w + 2.0,
                    u32::MAX,
                    true,
                );
            }
        } else if self.page == Page::Players && b.id >= 18 {
            ui::nine_slice(
                batch,
                atlas,
                if highlighted {
                    "gui/sprites/widget/button_highlighted"
                } else if !b.enabled {
                    "gui/sprites/widget/button_disabled"
                } else {
                    "gui/sprites/widget/button"
                },
                b.x,
                b.y,
                b.w,
                20.0,
                u32::MAX,
            );
            let remove = (b.id - 18) % 2 == 1;
            let row = self.invite_visible.get().0 + (b.id as usize - 18) / 2;
            let op = self
                .server
                .get("players")
                .and_then(Value::as_array)
                .and_then(|p| p.get(row))
                .and_then(|p| p.get("operator"))
                .and_then(Value::as_bool)
                == Some(true);
            let sprite = if remove {
                "remove_player"
            } else if op {
                "remove_operator"
            } else {
                "make_operator"
            };
            batch.sprite_scaled(
                atlas,
                &format!("gui/sprites/player_list/{sprite}"),
                b.x + b.w - 10.0,
                b.y + 6.0,
                8.0,
                7.0,
                u32::MAX,
            );
            ui::scrolling_text(
                batch,
                atlas,
                &Text::literal(&b.label),
                b.x + 2.0,
                b.x + b.w - 14.0,
                b.y,
                b.y + 20.0,
                b.y + 6.0,
                u32::MAX,
            );
            batch.clip_since(
                first,
                0.0,
                self.players_top(batch.height),
                batch.width,
                self.players_bottom(batch.height),
            );
        } else if self.page == Page::Settings && b.id >= 16 {
            let index = b.id - 16;
            if self.settings_distances() && index < 2 {
                let simulation = index == 1;
                let (min, max, current) = self.distance(simulation).unwrap();
                let value = self
                    .configuration
                    .draft
                    .as_ref()
                    .and_then(|d| if simulation { d.simulation } else { d.render })
                    .unwrap_or(current);
                ui::nine_slice(
                    batch,
                    atlas,
                    if highlighted && self.configuration.slider_keys != Some(simulation) {
                        "gui/sprites/widget/slider_highlighted"
                    } else {
                        "gui/sprites/widget/slider"
                    },
                    b.x,
                    b.y,
                    b.w,
                    20.0,
                    u32::MAX,
                );
                let ratio = if value <= min {
                    0.0
                } else if value >= max {
                    1.0
                } else {
                    (value - min) as f32 / (max - min + 1) as f32 + 0.5 / (max - min + 1) as f32
                };
                ui::nine_slice(
                    batch,
                    atlas,
                    if highlighted || self.configuration.slider_keys == Some(simulation) {
                        "gui/sprites/widget/slider_handle_highlighted"
                    } else {
                        "gui/sprites/widget/slider_handle"
                    },
                    b.x + ratio * (b.w - 8.0),
                    b.y,
                    8.0,
                    20.0,
                    u32::MAX,
                );
                let label = translated("options.chunks", &[value.to_string()]);
                batch.text(
                    atlas,
                    &label,
                    b.x + (b.w - HudBatch::text_width(atlas, &label)) / 2.0,
                    b.y + 6.0,
                    u32::MAX,
                    true,
                );
            } else {
                // The title's button loop already records this widget and its narration;
                // painting here only keeps one accessibility entry per button.
                let sprite = if !b.enabled {
                    "gui/sprites/widget/button_disabled"
                } else if highlighted {
                    "gui/sprites/widget/button_highlighted"
                } else {
                    "gui/sprites/widget/button"
                };
                crate::ui::nine_slice(batch, atlas, sprite, b.x, b.y, b.w, 20.0, u32::MAX);
                b.draw_label(
                    batch,
                    atlas,
                    if b.enabled {
                        u32::MAX
                    } else {
                        rgba(160, 160, 160, 255)
                    },
                );
            }
            batch.clip_since(first, 0.0, 24.0, batch.width, batch.height - 35.0);
        } else {
            return false;
        }
        true
    }
    pub(super) fn draw_configuration(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        started: std::time::Instant,
    ) -> bool {
        if !self.is_configuration() {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        self.layout_configuration(h);
        ui::tiled(
            batch,
            atlas,
            "gui/tab_header_background",
            0.0,
            0.0,
            w,
            24.0,
            16.0,
            u32::MAX,
        );
        ui::menu_background(batch, atlas, 0.0, 24.0, w, h - 24.0, false);
        let (x, tw) = tabs(w);
        for (left, width) in [(0.0, x), (x + 4.0 * tw, w - x - 4.0 * tw)] {
            ui::tiled_sized(
                batch,
                atlas,
                "gui/header_separator",
                left,
                22.0,
                width,
                2.0,
                [32.0, 2.0],
                u32::MAX,
            );
        }
        ui::tiled_sized(
            batch,
            atlas,
            "gui/footer_separator",
            0.0,
            h - 35.0,
            w,
            2.0,
            [32.0, 2.0],
            u32::MAX,
        );
        if let Some(error) = &self.error {
            super::super::centered(atlas, batch, error, 27.0, rgba(255, 85, 85, 255));
        }
        if self.loading || self.id() <= 0 {
            super::super::centered(
                atlas,
                batch,
                &tr("mco.configure.world.loading"),
                grid_y(h, 9.0),
                u32::MAX,
            );
            return true;
        }
        match self.page {
            Page::Players => self.draw_configuration_players(atlas, batch),
            Page::Settings => self.draw_configuration_settings(atlas, batch, caret, started),
            Page::Subscription => self.draw_configuration_subscription(atlas, batch),
            _ => {}
        }
        true
    }
    fn draw_configuration_players(&self, atlas: &HudAtlas, batch: &mut HudBatch) {
        let (w, h) = (batch.width, batch.height);
        self.configuration.player_widths.set(
            [
                "mco.configure.world.invites.normal.tooltip",
                "mco.configure.world.invites.ops.tooltip",
                "mco.configure.world.invites.remove.tooltip",
            ]
            .map(|k| 16.0 + HudBatch::text_width(atlas, &tr(k))),
        );
        let players = self.server.get("players").and_then(Value::as_array);
        let label = translated(
            "mco.configure.world.invited.number",
            &[players.map_or(0, Vec::len).to_string()],
        );
        let top = self.players_top(h) + 4.0 - self.pixel_scroll.get().floor();
        let first = batch.vertices.len();
        let width = HudBatch::text_width(atlas, &label);
        batch.text(
            atlas,
            &label,
            (w / 2.0).floor() - (width / 2.0).floor(),
            top + 4.0,
            u32::MAX,
            true,
        );
        batch.fill(
            atlas,
            (w / 2.0).floor() - (width / 2.0).floor(),
            top + 13.0,
            width,
            1.0,
            u32::MAX,
        );
        for (i, p) in players.into_iter().flatten().enumerate() {
            let y = top + 17.0 + i as f32 * 36.0;
            if y + 36.0 < self.players_top(h) || y > self.players_bottom(h) {
                continue;
            }
            let x = (w / 2.0).floor() - 150.0;
            super::layout::draw_owner(
                atlas,
                batch,
                &json!({"ownerUUID":text(p,"uuid"),"owner":text(p,"name")}),
                x + 2.0,
                y + 2.0,
            );
            let color = if p.get("accepted").and_then(Value::as_bool) != Some(true) {
                rgba(160, 160, 160, 255)
            } else if p.get("online").and_then(Value::as_bool) == Some(true) {
                rgba(0, 255, 0, 255)
            } else {
                u32::MAX
            };
            batch.text(atlas, &text(p, "name"), x + 42.0, y + 14.0, color, true);
        }
        batch.clip_since(first, 0.0, self.players_top(h), w, self.players_bottom(h));
        self.configuration_scrollbar(
            atlas,
            batch,
            (w / 2.0).floor() + 154.0,
            self.players_top(h),
            self.players_bottom(h) - self.players_top(h),
        );
    }
    fn draw_configuration_settings(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        started: std::time::Instant,
    ) {
        let (w, h) = (batch.width, batch.height);
        let first = batch.vertices.len();
        for (i, field) in self.fields.iter().enumerate() {
            let rect = self.configuration_field_rect(i, w);
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
                (("realms", i), &field.value, field.max),
                rect,
                self.focused == Some(i),
                started,
            );
        }
        let top = self.settings_top(h) - self.pixel_scroll.get().floor();
        let left = (w / 2.0).floor() - 106.0;
        let distances = self.settings_distances();
        if distances {
            for (key, y) in [
                ("mco.configure.world.renderDistance", 106.0),
                ("mco.configure.world.simulationDistance", 155.0),
            ] {
                batch.text(atlas, &tr(key), left, top + y, u32::MAX, true);
            }
        }
        let y = top + if distances { 204.0 } else { 106.0 };
        batch.text(
            atlas,
            &tr("mco.configure.world.region_preference"),
            left,
            y,
            u32::MAX,
            true,
        );
        let preference = self.configuration_region_preference();
        let region = text(&preference, "preferredRegion");
        let pref = number(&preference, "regionSelectionPreference");
        let key = if pref == 2 {
            REGIONS
                .iter()
                .find(|r| r.0 == region)
                .map(|r| format!("realms.configuration.region.{}", r.1))
                .unwrap_or_default()
        } else {
            format!(
                "realms.configuration.region_preference.automatic_{}",
                if pref == 1 { "owner" } else { "player" }
            )
        };
        let label = tr(&key);
        batch.text(
            atlas,
            &label,
            left + (192.0 - HudBatch::text_width(atlas, &label)) / 2.0,
            y + 13.0,
            rgba(160, 160, 160, 255),
            true,
        );
        if pref == 2 {
            batch.sprite_scaled(
                atlas,
                self.region_preference_icon(&preference),
                left + 202.0,
                y + 13.0,
                10.0,
                8.0,
                u32::MAX,
            );
        }
        batch.clip_since(first, 0.0, 24.0, w, h - 35.0);
        self.configuration_scrollbar(
            atlas,
            batch,
            left + 220.0,
            self.settings_top(h),
            (h - 59.0).min(self.settings_height()),
        );
    }
    fn draw_configuration_subscription(&self, atlas: &HudAtlas, batch: &mut HudBatch) {
        if self.subscription.is_null() {
            super::super::centered(
                atlas,
                batch,
                &tr("mco.configure.world.loading"),
                grid_y(batch.height, 9.0),
                u32::MAX,
            );
            return;
        }
        let info = if crate::realms_service::is_snapshot()
            && self
                .server
                .get("parentWorldName")
                .and_then(Value::as_str)
                .is_some()
        {
            translated(
                "mco.snapshot.subscription.info",
                &[text(&self.server, "parentWorldName")],
            )
        } else {
            tr("mco.configure.world.subscription.recurring.info")
        };
        let lines = atlas.font.split(&Text::literal(info), 200.0);
        let info_height = lines.len() as f32 * 9.0 + 8.0;
        self.configuration.info_height.set(info_height);
        let y = grid_y(batch.height, 144.0 + info_height);
        let start = self.subscription.get("startDate").unwrap_or(&Value::Null);
        let date = start
            .as_i64()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .or_else(|| {
                start
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.to_utc())
            })
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%b %-d, %Y")
                    .to_string()
            })
            .unwrap_or_default();
        let days = number(&self.subscription, "daysLeft");
        let left = if days < 0 && self.server.get("expired").and_then(Value::as_bool) == Some(true)
        {
            tr("mco.configure.world.subscription.expired")
        } else if days <= 1 {
            tr("mco.configure.world.subscription.less_than_a_day")
        } else if days >= 30 && days % 30 > 0 {
            translated(
                "mco.configure.world.subscription.remaining.months.days",
                &[(days / 30).to_string(), (days % 30).to_string()],
            )
        } else if days >= 30 {
            translated(
                "mco.configure.world.subscription.remaining.months",
                &[(days / 30).to_string()],
            )
        } else {
            translated(
                "mco.configure.world.subscription.remaining.days",
                &[days.to_string()],
            )
        };
        let label = tr(
            if text(&self.subscription, "subscriptionType") == "RECURRING"
                || text(&self.subscription, "type") == "RECURRING"
            {
                "mco.configure.world.subscription.recurring.daysleft"
            } else {
                "mco.configure.world.subscription.timeleft"
            },
        );
        for (text, dy, color) in [
            (tr("mco.configure.world.subscription.start"), 0.0, u32::MAX),
            (date, 15.0, rgba(160, 160, 160, 255)),
            (label, 38.0, u32::MAX),
            (left, 53.0, rgba(160, 160, 160, 255)),
        ] {
            super::super::centered(atlas, batch, &text, y + dy, color);
        }
        for (i, line) in lines.iter().enumerate() {
            batch.text_styled(
                atlas,
                line,
                (batch.width / 2.0).floor() - 100.0,
                y + 148.0 + i as f32 * 9.0,
                u32::MAX,
                true,
            );
        }
    }
    fn configuration_scrollbar(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        x: f32,
        top: f32,
        view: f32,
    ) {
        let content = self.content_height.get();
        if content <= view || view <= 8.0 {
            return;
        }
        let thumb = (view * view / content)
            .floor()
            .clamp(32.0_f32.min(view - 8.0), view - 8.0);
        let y = top + (self.pixel_scroll.get() * (view - thumb) / (content - view)).floor();
        batch.fill(atlas, x, top, 6.0, view, rgba(0, 0, 0, 255));
        batch.fill(atlas, x, y, 6.0, thumb, rgba(128, 128, 128, 255));
        batch.fill(atlas, x, y, 5.0, thumb - 1.0, rgba(192, 192, 192, 255));
    }
}
