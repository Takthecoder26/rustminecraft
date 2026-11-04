//! 26.3 RealmsPlayersTab entry points, RealmsInviteScreen, RealmsInviteCodesScreen,
//! RealmsEditInviteCodeScreen and InviteCodeExpiration. Requests stay in game/realms.
use super::*;
use crate::ui;
use winit::keyboard::KeyCode;

#[derive(Clone, Debug, Default)]
pub(super) struct PlayerScreens {
    fetched: bool,
    selected: Option<usize>,
    scroll: Cell<f32>,
    subtitle_height: Cell<f32>,
    dragging_scroll: bool,
    scroll_drag_y: f32,
    dragging_expiration: bool,
    slider_keys: bool,
    back_locked: bool,
    draft: Option<CodeDraft>,
    error_return: Option<Page>,
}

#[derive(Clone, Debug)]
struct CodeDraft {
    code: String,
    original_enabled: bool,
    enabled: bool,
    original_expiration: Option<i64>,
    expiration: usize,
    expiration_changed: bool,
}

#[derive(Clone)]
pub(super) enum PlayerEvent {
    Invite,
    Edit,
    ToggleActive,
    Expiration,
    Save,
}

const EXPIRATIONS: [(&str, Option<i64>); 7] = [
    (
        "mco.configure.world.invite_codes.expiration.30_minutes",
        Some(1_800_000),
    ),
    (
        "mco.configure.world.invite_codes.expiration.1_hour",
        Some(3_600_000),
    ),
    (
        "mco.configure.world.invite_codes.expiration.6_hours",
        Some(21_600_000),
    ),
    (
        "mco.configure.world.invite_codes.expiration.12_hours",
        Some(43_200_000),
    ),
    (
        "mco.configure.world.invite_codes.expiration.1_day",
        Some(86_400_000),
    ),
    (
        "mco.configure.world.invite_codes.expiration.7_days",
        Some(604_800_000),
    ),
    ("mco.configure.world.invite_codes.expiration.never", None),
];

fn closest_expiration(expiration: Option<i64>, now: i64) -> usize {
    let Some(expiration) = expiration else {
        return 6;
    };
    let remaining = i128::from(expiration) - i128::from(now);
    EXPIRATIONS
        .iter()
        .enumerate()
        .filter_map(|(i, (_, duration))| {
            duration.map(|duration| (i, (i128::from(duration) - remaining).abs()))
        })
        .min_by_key(|(_, difference)| *difference)
        .map_or(6, |(i, _)| i)
}

impl CodeDraft {
    fn new(code: &Value, now: i64) -> Self {
        let original_expiration = code.get("expirationDate").and_then(|date| {
            date.as_i64()
                .or_else(|| date.as_str().and_then(|date| date.parse().ok()))
        });
        let enabled = code
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Self {
            code: text(code, "code"),
            original_enabled: enabled,
            enabled,
            original_expiration,
            expiration: closest_expiration(original_expiration, now),
            expiration_changed: false,
        }
    }
    fn select_expiration(&mut self, index: usize) {
        let index = index.min(EXPIRATIONS.len() - 1);
        if index != self.expiration {
            self.expiration = index;
            // Moving away and back still renews the expiration relative to save time.
            self.expiration_changed = true;
        }
    }
    fn resolved_expiration(&self, now: i64) -> Option<i64> {
        if self.expiration_changed {
            EXPIRATIONS[self.expiration]
                .1
                .map(|duration| now.saturating_add(duration))
        } else {
            self.original_expiration
        }
    }
}

fn content_top(height: f32, footer: f32, content: f32) -> f32 {
    63.0_f32.min(height - footer - content)
}

impl State {
    /// Call at the end of enter(), after clearing the old fields. Unlike the Java
    /// profile-name rule, EditBox's default input capacity here is 32 characters.
    pub(super) fn player_enter(&mut self) {
        self.player_release();
        self.player_screens.slider_keys = false;
        self.list_focused.set(false);
        if self.page == Page::InvitePlayer {
            self.add_field("mco.configure.world.invite.profile.name", String::new(), 32);
            self.focused = Some(0);
        }
    }

    /// Call before dispatch changes the page. Returning from Edit preserves the
    /// parent's selection and scroll, but a fresh Players -> Codes visit fetches.
    pub(super) fn player_before_dispatch(&mut self, action: &RealmsAction) {
        match action {
            RealmsAction::InviteLink(_) => {
                if !matches!(self.page, Page::InviteLink | Page::EditInviteCode) {
                    self.player_screens = PlayerScreens::default();
                    self.invite_links = Value::Null;
                }
                self.player_screens.back_locked = false;
            }
            RealmsAction::CreateInviteCode { .. }
            | RealmsAction::DeleteInviteCode { .. }
            | RealmsAction::UpdateInviteCode { .. } => {
                self.player_screens.back_locked = true;
                self.player_release();
            }
            _ => {}
        }
    }

    /// The runtime supplies the freshly fetched list after every code mutation.
    /// It must not discard a successful mutation's response or merely remove a
    /// local row: RealmsInviteCodesScreen.refreshInviteCodes fetches again.
    pub fn player_links_loaded(&mut self, value: Value) {
        self.invite_links = value;
        self.loading = false;
        self.player_screens.back_locked = false;
        self.player_screens.fetched = true;
        self.player_screens.selected = None;
        self.player_screens.scroll.set(0.0);
        self.player_screens.draft = None;
        if self.page == Page::EditInviteCode {
            self.enter(Page::InviteLink);
        }
    }

    /// Returns true when this family owns the error. Runtime generation/page
    /// fencing must run first, so an abandoned form cannot reopen after a reply.
    pub fn player_request_failed(&mut self, action: &RealmsAction, message: String) -> bool {
        if matches!(action, RealmsAction::InvitePlayer { .. }) {
            self.loading = false;
            self.error = Some(tr("mco.configure.world.players.error"));
            self.status = None;
            return true;
        }
        if !matches!(
            action,
            RealmsAction::InviteLink(_)
                | RealmsAction::CreateInviteCode { .. }
                | RealmsAction::DeleteInviteCode { .. }
                | RealmsAction::UpdateInviteCode { .. }
        ) {
            return false;
        }
        self.player_screens.back_locked = false;
        let parent = if self.page == Page::EditInviteCode {
            Page::EditInviteCode
        } else if self.player_screens.fetched {
            Page::InviteLink
        } else {
            Page::Players
        };
        self.player_screens.error_return = Some(parent);
        self.enter(Page::PlayerError);
        self.loading = false;
        self.error = Some(message);
        true
    }

    pub(super) fn player_back(&mut self) -> Option<bool> {
        let parent = match self.page {
            Page::InvitePlayer => Page::Players,
            Page::InviteLink | Page::EditInviteCode if self.player_screens.back_locked => {
                return Some(false);
            }
            Page::InviteLink => Page::Players,
            Page::EditInviteCode => Page::InviteLink,
            Page::PlayerError => self
                .player_screens
                .error_return
                .take()
                .unwrap_or(Page::Players),
            _ => return None,
        };
        self.enter(parent);
        self.loading = false;
        Some(false)
    }

    fn player_codes(&self) -> &[Value] {
        self.invite_links
            .get("inviteCodes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    fn player_selected_code(&self) -> Option<&Value> {
        self.player_screens
            .selected
            .and_then(|i| self.player_codes().get(i))
    }
    fn player_remove_confirmation(&self) -> bool {
        self.page == Page::Confirm
            && self.confirmation.as_ref().is_some_and(|c| {
                matches!(c.action, Action::Realms(RealmsAction::RemovePlayer { .. }))
            })
    }
    fn player_is_page(&self) -> bool {
        matches!(
            self.page,
            Page::InvitePlayer | Page::InviteLink | Page::EditInviteCode | Page::PlayerError
        ) || self.player_remove_confirmation()
    }

    pub(super) fn player_controls(&self) -> Option<Vec<Control>> {
        let mut controls = Vec::new();
        let mut add = |label: String, enabled, event| {
            controls.push(Control {
                label,
                enabled,
                event,
            })
        };
        let ready = !self.loading;
        match self.page {
            Page::InvitePlayer => {
                add(
                    tr("mco.configure.world.buttons.invite"),
                    ready,
                    Event::Player(PlayerEvent::Invite),
                );
                add(tr("gui.back"), true, Event::Back);
            }
            Page::InviteLink => {
                let selected = self.player_selected_code();
                let code = selected.map(|c| text(c, "code")).unwrap_or_default();
                add(
                    tr("mco.configure.world.invite_codes.copy"),
                    ready && selected.is_some(),
                    Event::Copy(code.clone()),
                );
                add(
                    tr("mco.configure.world.invite_codes.delete"),
                    ready && selected.is_some(),
                    Event::Request(RealmsAction::DeleteInviteCode {
                        id: self.id(),
                        code,
                    }),
                );
                add(
                    tr("mco.configure.world.invite_codes.edit"),
                    ready && selected.is_some(),
                    Event::Player(PlayerEvent::Edit),
                );
                add(
                    tr("mco.configure.world.invite_codes.create"),
                    ready && self.player_screens.fetched && self.player_codes().len() < 5,
                    Event::Request(RealmsAction::CreateInviteCode { id: self.id() }),
                );
                add(
                    tr("gui.back"),
                    !self.player_screens.back_locked,
                    Event::Back,
                );
            }
            Page::EditInviteCode => {
                let draft = self.player_screens.draft.as_ref()?;
                add(
                    translated(
                        "options.generic_value",
                        &[
                            tr("mco.configure.world.invite_codes.edit.active"),
                            tr(if draft.enabled {
                                "options.on"
                            } else {
                                "options.off"
                            }),
                        ],
                    ),
                    ready,
                    Event::Player(PlayerEvent::ToggleActive),
                );
                add(
                    translated(
                        "options.generic_value",
                        &[
                            tr("mco.configure.world.invite_codes.edit.expiration_date"),
                            tr(EXPIRATIONS[draft.expiration].0),
                        ],
                    ),
                    ready && draft.enabled,
                    Event::Player(PlayerEvent::Expiration),
                );
                // Vanilla deliberately leaves Copy usable while saving/deleting.
                add(
                    tr("mco.configure.world.invite_codes.copy"),
                    true,
                    Event::Copy(draft.code.clone()),
                );
                add(
                    tr("mco.configure.world.invite_codes.delete"),
                    ready,
                    Event::Request(RealmsAction::DeleteInviteCode {
                        id: self.id(),
                        code: draft.code.clone(),
                    }),
                );
                add(tr("gui.done"), ready, Event::Player(PlayerEvent::Save));
                add(
                    tr("gui.back"),
                    !self.player_screens.back_locked,
                    Event::Back,
                );
            }
            Page::PlayerError => add(tr("gui.ok"), true, Event::Back),
            _ if self.player_remove_confirmation() => {
                add(tr("gui.yes"), ready, Event::AcceptConfirmation);
                add(tr("gui.no"), ready, Event::Back);
            }
            _ => return None,
        }
        Some(controls)
    }

    pub(super) fn player_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        let controls = self.player_controls()?;
        self.viewport_height.set(h);
        let c = (w / 2.0).floor();
        Some(
            controls
                .into_iter()
                .enumerate()
                .map(|(i, control)| {
                    let (x, y, width) = match self.page {
                        Page::InvitePlayer => (
                            c - 100.0,
                            if i == 0 {
                                content_top(h, 33.0, 61.0) + 41.0
                            } else {
                                h - 26.0
                            },
                            200.0,
                        ),
                        Page::InviteLink if i < 3 => (c - 154.0 + i as f32 * 105.0, h - 56.0, 97.0),
                        Page::InviteLink => (c - 154.0 + (i - 3) as f32 * 158.0, h - 28.0, 150.0),
                        Page::EditInviteCode if i == 0 => {
                            (c + 54.0, content_top(h, 64.0, 58.0) + 1.0, 100.0)
                        }
                        Page::EditInviteCode if i == 1 => {
                            (c - 154.0, content_top(h, 64.0, 58.0) + 38.0, 308.0)
                        }
                        Page::EditInviteCode => (
                            c - 154.0 + ((i - 2) % 2) as f32 * 158.0,
                            h - 56.0 + ((i - 2) / 2) as f32 * 28.0,
                            150.0,
                        ),
                        Page::PlayerError => (c - 100.0, h - 52.0, 200.0),
                        _ => (c - 105.0 + i as f32 * 110.0, 157.0, 100.0),
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

    pub(super) fn player_field_rect(&self, index: usize, w: f32) -> Option<[f32; 4]> {
        (self.page == Page::InvitePlayer && index == 0).then(|| {
            [
                (w / 2.0).floor() - 100.0,
                content_top(self.viewport_height.get(), 33.0, 61.0) + 13.0,
                200.0,
                20.0,
            ]
        })
    }

    pub(super) fn player_press_event(&mut self, event: PlayerEvent, x: f32, w: f32) -> Action {
        if self.loading {
            return Action::None;
        }
        match event {
            PlayerEvent::Invite => {
                let raw = self
                    .fields
                    .first()
                    .map(|f| f.value.as_str())
                    .unwrap_or_default();
                let invalid = if raw.trim().is_empty() {
                    Some("mco.configure.world.players.error")
                } else if self
                    .server
                    .get("players")
                    .and_then(Value::as_array)
                    .is_some_and(|players| {
                        players
                            .iter()
                            .any(|p| text(p, "name").eq_ignore_ascii_case(raw))
                    })
                {
                    Some("mco.configure.world.players.invite.duplicate")
                } else {
                    None
                };
                if let Some(key) = invalid {
                    self.error = Some(tr(key));
                    self.status = None;
                    return Action::None;
                }
                self.dispatch(RealmsAction::InvitePlayer {
                    id: self.id(),
                    // String.trim(), unlike Unicode-aware Rust str::trim().
                    name: raw.trim_matches(|c: char| c <= '\u{20}').to_owned(),
                })
            }
            PlayerEvent::Edit => {
                if let Some(code) = self.player_selected_code() {
                    self.player_screens.draft =
                        Some(CodeDraft::new(code, chrono::Utc::now().timestamp_millis()));
                    self.enter(Page::EditInviteCode);
                }
                Action::None
            }
            PlayerEvent::ToggleActive => {
                if let Some(draft) = &mut self.player_screens.draft {
                    draft.enabled = !draft.enabled;
                }
                self.player_screens.slider_keys = false;
                Action::None
            }
            PlayerEvent::Expiration => {
                self.player_set_expiration(x, w);
                self.player_screens.dragging_expiration = true;
                self.player_screens.slider_keys = true;
                Action::None
            }
            PlayerEvent::Save => {
                let Some(draft) = &self.player_screens.draft else {
                    return Action::None;
                };
                if draft.enabled == draft.original_enabled && !draft.expiration_changed {
                    self.player_back();
                    return Action::None;
                }
                self.dispatch(RealmsAction::UpdateInviteCode {
                    id: self.id(),
                    code: draft.code.clone(),
                    enabled: draft.enabled,
                    expiration_ms: draft.resolved_expiration(chrono::Utc::now().timestamp_millis()),
                })
            }
        }
    }

    fn player_set_expiration(&mut self, x: f32, width: f32) {
        if self.loading {
            return;
        }
        if let Some(draft) = &mut self.player_screens.draft
            && draft.enabled
        {
            draft.select_expiration(
                (ui::slider_value((width / 2.0).floor() - 154.0, 308.0, x) * 6.0).round() as usize,
            );
        }
    }
    fn player_list_top(&self) -> f32 {
        33.0 + if self.player_screens.fetched && self.player_codes().is_empty() {
            self.player_screens.subtitle_height.get()
        } else {
            0.0
        }
    }
    fn player_list_layout(&self, h: f32) -> (f32, f32, f32) {
        self.viewport_height.set(h);
        let top = self.player_list_top();
        let view = (h - 64.0 - top).max(0.0);
        let content = self.player_codes().len() as f32 * 28.0 + 4.0;
        self.player_screens.scroll.set(
            self.player_screens
                .scroll
                .get()
                .clamp(0.0, (content - view).max(0.0)),
        );
        (top, view, content)
    }
    fn player_scroll_thumb(&self, h: f32) -> Option<(f32, f32, f32)> {
        let (top, view, content) = self.player_list_layout(h);
        if view <= 8.0 || content <= view {
            return None;
        }
        let thumb = (view * view / content)
            .floor()
            .clamp(32.0_f32.min(view - 8.0), view - 8.0);
        let y =
            top + (self.player_screens.scroll.get() * (view - thumb) / (content - view)).floor();
        Some((y, thumb, view))
    }

    /// Called before the parent's generic rows/fields handler. Buttons have
    /// already been dispatched; this owns blank space, input and list scrolling.
    pub(super) fn player_press_content(&mut self, x: f32, y: f32, size: (f32, f32)) -> bool {
        if !self.player_is_page() {
            return false;
        }
        self.focused = None;
        self.list_focused.set(false);
        self.player_screens.slider_keys = false;
        if self.loading {
            return true;
        }
        if let Some([bx, by, bw, bh]) = self.player_field_rect(0, size.0) {
            if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                self.focused = Some(0);
            }
        } else if self.page == Page::InviteLink {
            let (top, view, _) = self.player_list_layout(size.1);
            if !(top..top + view).contains(&y) {
                return true;
            }
            let c = (size.0 / 2.0).floor();
            if (c + 162.0..c + 168.0).contains(&x) && self.player_scroll_thumb(size.1).is_some() {
                self.player_screens.dragging_scroll = true;
                self.player_screens.scroll_drag_y = y;
            } else if (c - 154.0..c + 154.0).contains(&x) {
                let row_y = y - top - 2.0 + self.player_screens.scroll.get().floor();
                let index = (row_y / 28.0).floor() as usize;
                if row_y >= 0.0 && index < self.player_codes().len() {
                    self.player_screens.selected = Some(index);
                    self.list_focused.set(true);
                }
            }
        }
        true
    }

    pub(in crate::title) fn player_drag(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if self.loading {
            return false;
        }
        if self.page == Page::EditInviteCode && self.player_screens.dragging_expiration {
            self.player_set_expiration(cursor.0, size.0);
            return true;
        }
        if self.page == Page::InviteLink && self.player_screens.dragging_scroll {
            if let Some((_, thumb, view)) = self.player_scroll_thumb(size.1) {
                let (top, _, content) = self.player_list_layout(size.1);
                let maximum = content - view;
                let scroll = if cursor.1 < top {
                    0.0
                } else if cursor.1 > top + view {
                    maximum
                } else {
                    self.player_screens.scroll.get()
                        + (cursor.1 - self.player_screens.scroll_drag_y)
                            * (maximum / (view - thumb)).max(1.0)
                };
                self.player_screens.scroll.set(scroll.clamp(0.0, maximum));
                self.player_screens.scroll_drag_y = cursor.1;
            }
            return true;
        }
        false
    }
    pub(in crate::title) fn player_release(&mut self) {
        self.player_screens.dragging_scroll = false;
        self.player_screens.dragging_expiration = false;
    }
    pub(super) fn player_scroll(&mut self, delta: f32, size: (f32, f32)) -> bool {
        if !self.player_is_page() {
            return false;
        }
        if self.page == Page::InviteLink && !self.loading {
            self.player_screens
                .scroll
                .set((self.player_screens.scroll.get() - delta * 14.0).max(0.0));
            self.player_list_layout(size.1);
        }
        true
    }
    pub(super) fn player_arrow(&mut self, up: bool, size: (f32, f32)) -> bool {
        if !self.player_is_page() {
            return false;
        }
        if self.page == Page::InviteLink && !self.loading && !self.player_codes().is_empty() {
            let index = self.player_screens.selected.map_or(
                if up { self.player_codes().len() - 1 } else { 0 },
                |i| {
                    if up {
                        i.saturating_sub(1)
                    } else {
                        (i + 1).min(self.player_codes().len() - 1)
                    }
                },
            );
            self.player_screens.selected = Some(index);
            self.list_focused.set(true);
            let (_, view, _) = self.player_list_layout(size.1);
            let top = 2.0 + index as f32 * 28.0;
            let scroll = self.player_screens.scroll.get();
            self.player_screens.scroll.set(if top < scroll + 2.0 {
                (top - 2.0).max(0.0)
            } else if top + 30.0 > scroll + view {
                (top + 30.0 - view).max(0.0)
            } else {
                scroll
            });
            self.player_list_layout(size.1);
        }
        true
    }

    pub(super) fn player_tab(
        &mut self,
        reverse: bool,
        buttons: &[u8],
        focus: &mut Option<u8>,
    ) -> bool {
        if !self.player_is_page() {
            return false;
        }
        // -2 is the name field, -1 the object list; neither is a title button.
        let mut entries: Vec<i16> = Vec::new();
        if self.page == Page::InvitePlayer && !self.loading {
            entries.push(-2);
        }
        if self.page == Page::InviteLink && !self.loading && !self.player_codes().is_empty() {
            entries.push(-1);
        }
        entries.extend(buttons.iter().map(|id| i16::from(*id)));
        if entries.is_empty() {
            return true;
        }
        let current = if self.focused.is_some() {
            Some(-2)
        } else if self.list_focused.get() {
            Some(-1)
        } else {
            focus.map(i16::from)
        };
        let index = current.and_then(|value| entries.iter().position(|entry| *entry == value));
        let next = if reverse {
            index.map_or(entries.len() - 1, |i| {
                (i + entries.len() - 1) % entries.len()
            })
        } else {
            index.map_or(0, |i| (i + 1) % entries.len())
        };
        let target = entries[next];
        self.focused = (target == -2).then_some(0);
        self.list_focused.set(target == -1);
        *focus = u8::try_from(target).ok();
        self.player_screens.slider_keys = self.page == Page::EditInviteCode && *focus == Some(11);
        if target == -1 && self.player_screens.selected.is_none() {
            self.player_arrow(false, (0.0, self.viewport_height.get()));
        }
        true
    }

    /// Title must call this before spatial button navigation (and report the
    /// new spatial focus via player_button_focus afterwards).
    pub(in crate::title) fn player_key(
        &mut self,
        key: KeyCode,
        focus: &mut Option<u8>,
        size: (f32, f32),
    ) -> bool {
        if !self.player_is_page() {
            return false;
        }
        if self.page == Page::InviteLink
            && focus.is_none()
            && matches!(key, KeyCode::ArrowUp | KeyCode::ArrowDown)
        {
            return self.player_arrow(key == KeyCode::ArrowUp, size);
        }
        if self.page == Page::EditInviteCode
            && *focus == Some(11)
            && self.player_screens.slider_keys
            && !self.loading
            && let Some(draft) = &mut self.player_screens.draft
            && draft.enabled
            && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight)
        {
            let index = if key == KeyCode::ArrowLeft {
                draft.expiration.saturating_sub(1)
            } else {
                draft.expiration + 1
            };
            draft.select_expiration(index);
            return true;
        }
        false
    }
    pub(in crate::title) fn player_button_focus(&mut self, focus: Option<u8>) {
        if !self.player_is_page() {
            return;
        }
        self.player_screens.slider_keys = false;
        if focus.is_some() {
            self.list_focused.set(false);
            self.focused = None;
        }
    }
    /// Enter/Space toggles slider adjustment, never synthesizes a click at x=0.
    pub(in crate::title) fn player_activate(&mut self, focus: Option<u8>) -> bool {
        if self.page != Page::EditInviteCode || focus != Some(11) {
            return false;
        }
        if !self.loading
            && self
                .player_screens
                .draft
                .as_ref()
                .is_some_and(|d| d.enabled)
        {
            self.player_screens.slider_keys = !self.player_screens.slider_keys;
        }
        true
    }

    pub(super) fn player_draw_button(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        button: &Button,
        highlighted: bool,
    ) -> bool {
        if self.page != Page::EditInviteCode || button.id != 11 {
            return false;
        }
        let Some(draft) = &self.player_screens.draft else {
            return false;
        };
        ui::nine_slice(
            batch,
            atlas,
            if highlighted && button.enabled && !self.player_screens.slider_keys {
                "gui/sprites/widget/slider_highlighted"
            } else {
                "gui/sprites/widget/slider"
            },
            button.x,
            button.y,
            button.w,
            20.0,
            u32::MAX,
        );
        ui::nine_slice(
            batch,
            atlas,
            if button.enabled && (highlighted || self.player_screens.slider_keys) {
                "gui/sprites/widget/slider_handle_highlighted"
            } else {
                "gui/sprites/widget/slider_handle"
            },
            button.x + (draft.expiration as f32 / 6.0 * (button.w - 8.0)).floor(),
            button.y,
            8.0,
            20.0,
            u32::MAX,
        );
        ui::scrolling_text(
            batch,
            atlas,
            &Text::literal(&button.label),
            button.x + 2.0,
            button.x + button.w - 2.0,
            button.y,
            button.y + 20.0,
            button.y + 6.0,
            if button.enabled {
                u32::MAX
            } else {
                rgba(160, 160, 160, 255)
            },
        );
        true
    }

    pub(super) fn player_draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        started: std::time::Instant,
    ) -> bool {
        if !self.player_is_page() {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        self.viewport_height.set(h);
        if self.player_remove_confirmation() {
            if let Some(confirmation) = &self.confirmation {
                super::super::centered(atlas, batch, &tr(confirmation.key), 79.0, u32::MAX);
                super::super::centered(atlas, batch, &confirmation.detail, 105.0, u32::MAX);
            }
            return true;
        }
        if self.page == Page::PlayerError {
            super::super::centered(
                atlas,
                batch,
                &tr("mco.errorMessage.generic"),
                80.0,
                u32::MAX,
            );
            for (i, line) in atlas
                .font
                .split(
                    &Text::literal(self.error.as_deref().unwrap_or_default()),
                    (w * 3.0 / 4.0).floor(),
                )
                .iter()
                .enumerate()
            {
                super::super::centered(
                    atlas,
                    batch,
                    &line.plain(),
                    100.0 + i as f32 * 9.0,
                    rgba(223, 80, 80, 255),
                );
            }
            return true;
        }
        let title = match self.page {
            Page::InvitePlayer => tr("mco.configure.world.buttons.invite"),
            Page::EditInviteCode => tr("mco.configure.world.invite_codes.edit.title"),
            _ if self.player_screens.fetched => translated(
                "mco.configure.world.invite_codes.title",
                &[self.player_codes().len().to_string(), "5".into()],
            ),
            _ => tr("mco.configure.world.buttons.invite_codes"),
        };
        if self.page == Page::InvitePlayer {
            super::super::centered(atlas, batch, &title, 12.0, u32::MAX);
            if let Some(field) = self.fields.first() {
                let rect = self.player_field_rect(0, w).unwrap();
                batch.text(
                    atlas,
                    &tr(field.key),
                    rect[0],
                    rect[1] - 13.0,
                    rgba(160, 160, 160, 255),
                    true,
                );
                caret.draw(
                    batch,
                    atlas,
                    (("realms", 0), &field.value, field.max),
                    rect,
                    self.focused == Some(0) && !self.loading,
                    started,
                );
            }
            let message = if self.loading {
                Some(tr("mco.configure.world.players.inviting"))
            } else {
                self.error.clone()
            };
            if let Some(message) = message {
                super::super::centered(
                    atlas,
                    batch,
                    &message,
                    content_top(h, 33.0, 61.0) + 69.0,
                    if self.loading {
                        rgba(160, 160, 160, 255)
                    } else {
                        rgba(255, 0, 0, 255)
                    },
                );
            }
        } else if self.page == Page::EditInviteCode {
            ui::list_background(batch, atlas, 33.0, h - 64.0, false);
            super::super::centered(atlas, batch, &title, 12.0, u32::MAX);
            if let Some(draft) = &self.player_screens.draft {
                let x = (w / 2.0).floor() - 154.0;
                let y = content_top(h, 64.0, 58.0);
                let expiration = draft
                    .original_expiration
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .map(|date| {
                        date.with_timezone(&chrono::Local)
                            .format("%x, %-I:%M %p")
                            .to_string()
                    })
                    .unwrap_or_else(|| tr(EXPIRATIONS[6].0));
                let expires = translated(
                    "mco.configure.world.invite_codes.edit.expires",
                    &[expiration],
                );
                for (label, dy, color) in [
                    (&draft.code, 0.0, u32::MAX),
                    (&expires, 13.0, rgba(160, 160, 160, 255)),
                ] {
                    let shown = if HudBatch::text_width(atlas, label) > 200.0 {
                        format!(
                            "{}…",
                            &label[..atlas
                                .font
                                .fit(label, 200.0 - HudBatch::text_width(atlas, "…"))]
                        )
                    } else {
                        label.clone()
                    };
                    batch.text(atlas, &shown, x, y + dy, color, true);
                }
            }
            ui::separators(batch, atlas, 33.0, h - 64.0, false);
        } else {
            let subtitle = if self.player_screens.fetched && self.player_codes().is_empty() {
                atlas.font.split(
                    &Text::literal(translated(
                        "mco.configure.world.invite_codes.subtitle",
                        &["5".into()],
                    )),
                    308.0,
                )
            } else {
                Vec::new()
            };
            self.player_screens
                .subtitle_height
                .set(subtitle.len() as f32 * 9.0);
            let (top, view, _) = self.player_list_layout(h);
            ui::list_background(batch, atlas, top, top + view, false);
            // HeaderAndFooterLayout centers the title+8px gap+subtitle as a unit.
            let title_y = if subtitle.is_empty() { 12.0 } else { 8.0 };
            super::super::centered(atlas, batch, &title, title_y, u32::MAX);
            for (i, line) in subtitle.iter().enumerate() {
                super::super::centered(
                    atlas,
                    batch,
                    &line.plain(),
                    title_y + 17.0 + i as f32 * 9.0,
                    rgba(160, 160, 160, 255),
                );
            }
            if !self.player_screens.fetched {
                let y = content_top(h, 64.0, 28.0);
                super::super::centered(
                    atlas,
                    batch,
                    &tr("mco.configure.world.invite_codes.loading"),
                    y + 2.0,
                    u32::MAX,
                );
                let dots = ["O o o", "o O o", "o o O", "o O o"]
                    [(started.elapsed().as_millis() / 300 % 4) as usize];
                super::super::centered(atlas, batch, dots, y + 17.0, rgba(128, 128, 128, 255));
            } else {
                let x = (w / 2.0).floor() - 154.0;
                let first = batch.vertices.len();
                for (i, code) in self.player_codes().iter().enumerate() {
                    let y = top + 2.0 + i as f32 * 28.0 - self.player_screens.scroll.get().floor();
                    if y + 28.0 <= top || y >= top + view {
                        continue;
                    }
                    if self.player_screens.selected == Some(i) {
                        batch.fill(
                            atlas,
                            x,
                            y,
                            308.0,
                            28.0,
                            if self.list_focused.get() {
                                u32::MAX
                            } else {
                                rgba(128, 128, 128, 255)
                            },
                        );
                        batch.fill(atlas, x + 1.0, y + 1.0, 306.0, 26.0, rgba(0, 0, 0, 255));
                    }
                    batch.text(
                        atlas,
                        &text(code, "code"),
                        x + 10.0,
                        y + 10.0,
                        u32::MAX,
                        true,
                    );
                }
                batch.clip_since(first, 0.0, top, w, top + view);
                if let Some((y, thumb, _)) = self.player_scroll_thumb(h) {
                    let x = (w / 2.0).floor() + 162.0;
                    ui::nine_slice(
                        batch,
                        atlas,
                        "gui/sprites/widget/scroller_background",
                        x,
                        top,
                        6.0,
                        view,
                        u32::MAX,
                    );
                    ui::nine_slice(
                        batch,
                        atlas,
                        "gui/sprites/widget/scroller",
                        x,
                        y,
                        6.0,
                        thumb,
                        u32::MAX,
                    );
                }
            }
            ui::separators(batch, atlas, top, top + view, false);
        }
        true
    }
}
