//! RealmsPendingInvitesScreen and the trusted ConfirmLinkScreen layout.
use super::*;

impl State {
    pub(in crate::title) fn button_height(&self, button: &Button) -> f32 {
        if let Some(height) = self
            .entry_button_height(button)
            .or_else(|| self.reset_button_height(button))
        {
            return height;
        }
        if let Some(height) = self.configuration_button_height(button) {
            return height;
        }
        if self.page == Page::Invites && button.id >= 11 {
            21.0
        } else {
            20.0
        }
    }
    pub(super) fn link_confirmation(&self) -> Option<&str> {
        if self.page != Page::Confirm {
            return None;
        }
        match &self.confirmation.as_ref()?.action {
            Action::OpenUri(url) => Some(url),
            _ => None,
        }
    }

    pub(super) fn subscreen_controls(&self) -> Option<Vec<Control>> {
        let control = |key: &str, event| Control {
            label: tr(key),
            enabled: true,
            event,
        };
        if self.link_confirmation().is_some() {
            return Some(vec![
                control("chat.link.open", Event::AcceptConfirmation),
                control("chat.copy", Event::CopyLink),
                control("gui.cancel", Event::Back),
            ]);
        }
        if self.page != Page::Invites {
            return None;
        }
        let mut controls = vec![control("gui.done", Event::Back)];
        let (first, count) = self.invite_visible.get();
        // IDs are u8 in the title widget API. Only visible rows need controls.
        for invite in self.invites.iter().skip(first).take(count.min(120)) {
            let id = text(invite, "invitationId");
            controls.push(control(
                "mco.invites.button.accept",
                Event::Request(RealmsAction::AcceptInvite(id.clone())),
            ));
            controls.push(control(
                "mco.invites.button.reject",
                Event::Request(RealmsAction::RejectInvite(id)),
            ));
        }
        Some(controls)
    }

    pub(super) fn subscreen_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        if self.page == Page::Invites {
            self.layout_invites(h);
        }
        let controls = self.subscreen_controls()?;
        let c = (w / 2.0).floor();
        let link = self.link_confirmation().is_some();
        let (first, _) = self.invite_visible.get();
        Some(
            controls
                .into_iter()
                .enumerate()
                .map(|(i, control)| {
                    let (x, y, width) = if link {
                        (
                            c - 154.0 + i as f32 * 104.0,
                            self.link_button_y.get(),
                            100.0,
                        )
                    } else if i == 0 {
                        (c - 100.0, h - 26.0, 200.0)
                    } else {
                        let row = first + (i - 1) / 2;
                        let reject = (i - 1) % 2 == 1;
                        (
                            c - 140.0 + 2.0 + if reject { 247.0 } else { 218.0 },
                            43.0 + row as f32 * 36.0 - self.pixel_scroll.get().floor(),
                            21.0,
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

    pub(super) fn draw_subscreen_button(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        button: &Button,
        highlighted: bool,
    ) -> bool {
        if self.world_draw_button(atlas, batch, button, highlighted)
            || self.player_draw_button(atlas, batch, button, highlighted)
            || self.reset_draw_button(atlas, batch, button, highlighted)
            || self.draw_configuration_button(atlas, batch, button, highlighted)
        {
            return true;
        }
        if self.page != Page::Invites || button.id == 10 {
            return false;
        }
        let first = batch.vertices.len();
        let accept = (button.id - 11).is_multiple_of(2);
        let sprite = format!(
            "gui/sprites/pending_invite/{}{}",
            if accept { "accept" } else { "reject" },
            if highlighted { "_highlighted" } else { "" }
        );
        // TextAndIcon(false) extracts its default button skin, then its icon at +1,+1.
        crate::ui::nine_slice(
            batch,
            atlas,
            if highlighted {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            },
            button.x,
            button.y,
            21.0,
            21.0,
            u32::MAX,
        );
        batch.sprite_scaled(
            atlas,
            &sprite,
            button.x + 1.0,
            button.y + 1.0,
            18.0,
            18.0,
            u32::MAX,
        );
        batch.clip_since(first, 0.0, 33.0, batch.width, batch.height - 33.0);
        true
    }

    pub(super) fn draw_subscreen_tooltip(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
    ) -> bool {
        if self.reset_draw_tooltip(atlas, batch, cursor)
            || self.draw_configuration_tooltip(atlas, batch, cursor)
        {
            return true;
        }
        if self.page != Page::Invites {
            return false;
        }
        if (33.0..batch.height - 33.0).contains(&cursor.1) {
            for button in self
                .subscreen_buttons(batch.width, batch.height)
                .unwrap_or_default()
                .iter()
                .skip(1)
            {
                if (button.x..button.x + 21.0).contains(&cursor.0)
                    && (button.y..button.y + 21.0).contains(&cursor.1)
                {
                    crate::ui::tooltip(batch, atlas, &[Text::literal(&button.label)], cursor);
                    break;
                }
            }
        }
        true
    }

    pub(super) fn draw_subscreen(&self, atlas: &HudAtlas, batch: &mut HudBatch) -> bool {
        if let Some(url) = self.link_confirmation() {
            let lines = atlas
                .font
                .split(&Text::literal(url), (batch.width - 50.0).max(1.0));
            let rows = lines.len().min(15);
            // Title 9, vertical gaps 8+8, message rows, button cell paddingTop16 + height20.
            let total = 61.0 + rows as f32 * 9.0;
            let y = ((batch.height - total) / 2.0).round();
            super::super::centered(atlas, batch, &tr("chat.link.confirmTrusted"), y, u32::MAX);
            for (i, line) in lines.iter().take(15).enumerate() {
                let x = (batch.width / 2.0).floor() - (atlas.font.width_text(line) / 2.0).floor();
                batch.text_styled(atlas, line, x, y + 17.0 + i as f32 * 9.0, u32::MAX, true);
            }
            self.link_button_y.set(y + 41.0 + rows as f32 * 9.0);
            return true;
        }
        if self.page != Page::Invites {
            return false;
        }
        self.layout_invites(batch.height);
        let bottom = batch.height - 33.0;
        crate::ui::list_background(batch, atlas, 33.0, bottom, false);
        super::super::centered(atlas, batch, &tr("mco.invites.title"), 12.0, u32::MAX);
        let first_vertex = batch.vertices.len();
        let left = (batch.width / 2.0).floor() - 140.0;
        for (i, invite) in self.invites.iter().enumerate() {
            let y = 35.0 + i as f32 * 36.0 - self.pixel_scroll.get().floor();
            if y + 36.0 <= 33.0 || y >= bottom {
                continue;
            }
            let owner = json!({"ownerUUID": text(invite, "worldOwnerUuid"), "owner": text(invite, "worldOwnerName")});
            super::layout::draw_owner(atlas, batch, &owner, left + 2.0, y + 2.0);
            let name = text(invite, "worldName");
            let owner_name = text(invite, "worldOwnerName");
            let age = invite_age(invite);
            for (value, dy, color) in [
                (name, 1.0, u32::MAX),
                (owner_name, 12.0, rgba(160, 160, 160, 255)),
                (age, 24.0, rgba(160, 160, 160, 255)),
            ] {
                let shown = if HudBatch::text_width(atlas, &value) > 174.0 {
                    format!(
                        "{}…",
                        &value[..atlas
                            .font
                            .fit(&value, 174.0 - HudBatch::text_width(atlas, "…"))]
                    )
                } else {
                    value
                };
                batch.text(atlas, &shown, left + 40.0, y + 2.0 + dy, color, true);
            }
        }
        batch.clip_since(first_vertex, 0.0, 33.0, batch.width, bottom);
        if !self.loading && self.invites.is_empty() {
            super::super::centered(
                atlas,
                batch,
                &tr("mco.invites.nopending"),
                batch.height / 2.0 - 20.0,
                u32::MAX,
            );
        }
        self.draw_invite_scrollbar(atlas, batch);
        crate::ui::separators(batch, atlas, 33.0, bottom, false);
        true
    }

    fn layout_invites(&self, h: f32) {
        self.viewport_height.set(h);
        let content = self.invites.len() as f32 * 36.0 + 4.0;
        self.content_height.set(content);
        let max = (content - (h - 66.0)).max(0.0);
        self.pixel_scroll
            .set(self.pixel_scroll.get().clamp(0.0, max));
        let first = ((self.pixel_scroll.get() - 2.0).max(0.0) / 36.0).floor() as usize;
        let count = ((h - 66.0).max(0.0) / 36.0).ceil() as usize + 1;
        self.invite_visible
            .set((first, count.min(self.invites.len().saturating_sub(first))));
    }

    fn draw_invite_scrollbar(&self, atlas: &HudAtlas, batch: &mut HudBatch) {
        let view = batch.height - 66.0;
        let content = self.content_height.get();
        if content <= view || view <= 8.0 {
            return;
        }
        let thumb = (view * view / content)
            .floor()
            .clamp(32.0_f32.min(view - 8.0), view - 8.0);
        let x = (batch.width / 2.0).floor() + 148.0;
        let y =
            33.0 + (self.pixel_scroll.get().floor() * (view - thumb) / (content - view)).floor();
        batch.fill(atlas, x, 33.0, 6.0, view, rgba(0, 0, 0, 255));
        batch.fill(atlas, x, y, 6.0, thumb, rgba(128, 128, 128, 255));
        batch.fill(atlas, x, y, 5.0, thumb - 1.0, rgba(192, 192, 192, 255));
    }

    pub(super) fn scroll_invites(&mut self, delta: f32, h: f32) {
        self.pixel_scroll
            .set((self.pixel_scroll.get() - delta * 18.0).max(0.0));
        self.layout_invites(h);
    }

    pub(super) fn press_invite_list(&mut self, x: f32, y: f32, size: (f32, f32)) {
        if !(33.0..size.1 - 33.0).contains(&y) {
            return;
        }
        let c = (size.0 / 2.0).floor();
        if (c + 148.0..c + 154.0).contains(&x) {
            self.dragging_scrollbar = self.content_height.get() > size.1 - 66.0;
            self.drag_scrollbar(y, size.1);
        } else if (c - 140.0..c + 140.0).contains(&x) {
            let row = ((y - 35.0 + self.pixel_scroll.get()).max(0.0) / 36.0) as usize;
            self.selected = (row < self.invites.len()).then_some(row);
        }
    }

    pub(super) fn arrow_invites(&mut self, up: bool, h: f32) {
        if self.invites.is_empty() {
            return;
        }
        let row = self.selected.map_or(0, |i| {
            if up {
                i.saturating_sub(1)
            } else {
                (i + 1).min(self.invites.len() - 1)
            }
        });
        self.selected = Some(row);
        let top = row as f32 * 36.0;
        let view = h - 66.0;
        if top < self.pixel_scroll.get() {
            self.pixel_scroll.set(top);
        } else if top + 36.0 > self.pixel_scroll.get() + view {
            self.pixel_scroll.set((top + 36.0 - view).max(0.0));
        }
        self.layout_invites(h);
    }

    pub(super) fn realm_status_tooltip(
        &self,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> Option<Text> {
        if self.page != Page::List || !self.main_has_list() {
            return None;
        }
        let c = (size.0 / 2.0).floor();
        if (size.1 - 55.0..size.1 - 35.0).contains(&cursor.1)
            && (c - 154.0..c - 54.0).contains(&cursor.0)
        {
            let selected = self.selected.and_then(|i| self.servers.get(i))?;
            if text(selected, "state") == "CLOSED" {
                return Some(Text::translate("mco.play.button.realm.closed", &[]));
            }
        }
        if !(44.0..size.1 - 66.0).contains(&cursor.1)
            || !(c + 131.0..=c + 141.0).contains(&cursor.0)
        {
            return None;
        }
        for (i, server) in self.servers.iter().enumerate() {
            let y = 50.0
                + self.notification_height.get()
                + (self.snapshot_realms.len() + i) as f32 * 36.0
                - self.pixel_scroll.get().floor();
            if !(y..=y + 28.0).contains(&cursor.1) || text(server, "state") == "UNINITIALIZED" {
                continue;
            }
            let key = if server.get("expired").and_then(Value::as_bool) == Some(true) {
                "mco.selectServer.expired"
            } else if text(server, "state") == "CLOSED" {
                "mco.selectServer.closed"
            } else if self.owned(server) && number(server, "daysLeft") < 7 {
                let days = number(server, "daysLeft");
                if days <= 0 {
                    "mco.selectServer.expires.soon"
                } else if days == 1 {
                    "mco.selectServer.expires.day"
                } else {
                    return Some(Text::translate(
                        "mco.selectServer.expires.days",
                        &[Text::literal(days.to_string())],
                    ));
                }
            } else if text(server, "state") == "OPEN" {
                "mco.selectServer.open"
            } else {
                return None;
            };
            return Some(Text::translate(key, &[]));
        }
        None
    }
}

fn invite_age(invite: &Value) -> String {
    let date = invite.get("date").unwrap_or(&Value::Null);
    let timestamp = date.as_i64().or_else(|| {
        date.as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis())
    });
    let Some(timestamp) = timestamp else {
        return String::new();
    };
    let millis = chrono::Utc::now()
        .timestamp_millis()
        .saturating_sub(timestamp);
    if millis < 0 {
        return tr("mco.time.now");
    }
    let seconds = millis / 1000;
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
