//! Entry, main list and product page layouts from Minecraft 26.3 Realms screens.
use super::*;

impl State {
    pub(super) fn draw_join(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut crate::ui::Caret,
        started: std::time::Instant,
    ) {
        let y = 63.0_f32.min(batch.height - 33.0 - 61.0);
        self.join_field_y.set(y + 13.0);
        crate::ui::list_background(batch, atlas, 33.0, batch.height - 33.0, false);
        crate::ui::separators(batch, atlas, 33.0, batch.height - 33.0, false);
        super::super::centered(atlas, batch, &self.title(), 12.0, u32::MAX);
        let rect = self.field_rect(0, batch.width);
        batch.text(
            atlas,
            &tr("mco.selectServer.joinCode"),
            rect[0],
            y,
            rgba(160, 160, 160, 255),
            true,
        );
        if let Some(field) = self.fields.first() {
            caret.draw(
                batch,
                atlas,
                (("realms", 0), &field.value, field.max),
                rect,
                self.focused == Some(0) && !self.loading,
                started,
            );
            if field.value.is_empty() {
                batch.text(
                    atlas,
                    &tr("mco.selectServer.joinCode.hint"),
                    rect[0] + 4.0,
                    rect[1] + 6.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
            }
        }
        if self.loading || self.error.is_some() {
            super::super::centered(
                atlas,
                batch,
                &tr(if self.loading {
                    "mco.selectServer.joinCode.validating"
                } else {
                    "mco.selectServer.joinCode.invalid"
                }),
                rect[1] + 56.0,
                if self.loading {
                    u32::MAX
                } else {
                    rgba(255, 0, 0, 255)
                },
            );
        }
    }

    pub(super) fn main_has_list(&self) -> bool {
        self.servers_loaded
            && (!self.servers.is_empty()
                || !self.snapshot_realms.is_empty()
                || !self.notifications.is_empty())
    }

    pub(super) fn entry_controls(&self) -> Option<Vec<Control>> {
        if let Some(controls) = self.subscreen_controls() {
            return Some(controls);
        }
        let control = |key: &str, enabled, event| Control {
            label: tr(key),
            enabled,
            event,
        };
        if self.page == Page::Redeem {
            return Some(vec![
                control(
                    "mco.selectServer.joinCode.select",
                    !self.loading && !self.field(0).is_empty(),
                    Event::Redeem,
                ),
                control("gui.back", true, Event::Back),
            ]);
        }
        if matches!(
            self.page,
            Page::AvailabilityError | Page::ConfigurationError
        ) {
            return Some(match self.availability_error_kind {
                Some(AvailabilityErrorKind::ParentalConsent) => vec![
                    control(
                        "mco.account.privacy.info.button",
                        true,
                        Event::Link("https://aka.ms/MinecraftGDPR".into()),
                    ),
                    control("gui.back", true, Event::Back),
                ],
                Some(AvailabilityErrorKind::Incompatible) => {
                    vec![control("gui.back", true, Event::Back)]
                }
                _ => vec![control("gui.ok", true, Event::Back)],
            });
        }
        if !matches!(self.page, Page::List | Page::Pdp) {
            return None;
        }
        let ready = self.page == Page::Pdp || self.servers_loaded;
        let mut controls = vec![
            control(
                "mco.selectServer.joinRealm",
                ready,
                Event::Page(Page::Redeem),
            ),
            control(
                "mco.invites.title",
                true,
                Event::Request(RealmsAction::Invites),
            ),
            control(
                "mco.news",
                true,
                Event::Link(self.news_link.clone().unwrap_or_default()),
            ),
        ];
        if self.page == Page::Pdp {
            controls.push(control(
                if self.trial_available {
                    "mco.pdp.tryForFree"
                } else {
                    "mco.selectServer.purchase"
                },
                true,
                Event::Link(
                    if self.trial_available {
                        "https://aka.ms/startjavarealmstrial"
                    } else {
                        "https://aka.ms/BuyJavaRealms"
                    }
                    .into(),
                ),
            ));
        } else {
            if self.main_has_list() {
                let server = self.selected.and_then(|i| self.servers.get(i));
                let id = server.map(|s| number(s, "id")).unwrap_or_default();
                let owned = server.is_some_and(|s| self.owned(s));
                controls.extend([
                    control(
                        "mco.selectServer.play",
                        server.is_some_and(|s| self.playable(s)),
                        Event::Request(RealmsAction::Join(id)),
                    ),
                    control(
                        "mco.selectServer.configure",
                        owned && server.is_some_and(|s| text(s, "state") != "UNINITIALIZED"),
                        Event::Request(RealmsAction::Configure(id)),
                    ),
                    control(
                        "mco.selectServer.expiredRenew",
                        owned && server.is_some_and(|s| flag(s, "expired")),
                        Event::Confirm(
                            RealmsAction::RenewSubscription(id),
                            "chat.link.confirm",
                            String::new(),
                        ),
                    ),
                    control(
                        "mco.selectServer.leave",
                        server.is_some() && !owned,
                        Event::Confirm(
                            RealmsAction::Leave(id),
                            "mco.configure.world.leave.question.line1",
                            server.map(|s| text(s, "name")).unwrap_or_default(),
                        ),
                    ),
                ]);
            }
            controls.push(control(
                if self.trial_available {
                    "mco.selectServer.freeTrial"
                } else {
                    "mco.selectServer.purchase"
                },
                ready,
                Event::Page(Page::Pdp),
            ));
        }
        controls.push(control("gui.back", true, Event::Back));
        Some(controls)
    }

    fn playable(&self, server: &Value) -> bool {
        text(server, "state") == "OPEN"
            && !flag(server, "expired")
            && (matches!(
                text(server, "compatibility").as_str(),
                "COMPATIBLE" | "NEEDS_UPGRADE"
            ) || self.owned(server))
    }

    pub(super) fn entry_buttons(&self, w: f32, h: f32) -> Option<Vec<Button>> {
        if let Some(buttons) = self.subscreen_buttons(w, h) {
            return Some(buttons);
        }
        let controls = self.entry_controls()?;
        let c = (w / 2.0).floor();
        Some(
            controls
                .into_iter()
                .enumerate()
                .map(|(i, control)| {
                    let (x, y, width, icon) = if self.page == Page::Redeem {
                        (
                            c - 100.0,
                            if i == 0 {
                                self.join_field_y.get() + 28.0
                            } else {
                                h - 26.0
                            },
                            200.0,
                            None,
                        )
                    } else if matches!(
                        self.page,
                        Page::AvailabilityError | Page::ConfigurationError
                    ) {
                        match self.availability_error_kind {
                            Some(AvailabilityErrorKind::ParentalConsent) => (
                                c - 154.0 + i as f32 * 158.0,
                                self.privacy_button_y.get(),
                                150.0,
                                None,
                            ),
                            Some(AvailabilityErrorKind::Incompatible) => {
                                (c - 100.0, h - 26.0, 200.0, None)
                            }
                            _ => (c - 100.0, h - 52.0, 200.0, None),
                        }
                    } else if i == 0 {
                        (c - 154.0, 12.0, self.join_button_width.get(), None)
                    } else if i == 1 || i == 2 {
                        (
                            c + 110.0 + (i - 1) as f32 * 24.0,
                            12.0,
                            20.0,
                            Some((if i == 1 { "icon/invite" } else { "icon/news" }, 14.0)),
                        )
                    } else if self.page == Page::Pdp {
                        (c - 154.0 + (i - 3) as f32 * 158.0, h - 26.0, 150.0, None)
                    } else if self.main_has_list() {
                        let index = i - 3;
                        (
                            c - 154.0 + (index % 3) as f32 * 104.0,
                            h - 55.0 + (index / 3) as f32 * 24.0,
                            100.0,
                            None,
                        )
                    } else {
                        (c - 102.0 + (i - 3) as f32 * 104.0, h - 31.0, 100.0, None)
                    };
                    Button {
                        x,
                        y,
                        w: width,
                        label: control.label,
                        enabled: control.enabled,
                        id: 10 + i as u8,
                        icon,
                        trailing: None,
                    }
                })
                .collect(),
        )
    }

    /// Draw after the caller records focus/narration, in place of its ordinary button paint.
    pub(in crate::title) fn draw_button(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        button: &Button,
        highlighted: bool,
    ) -> bool {
        if self.draw_subscreen_button(atlas, batch, button, highlighted) {
            return true;
        }
        if !matches!(self.page, Page::List | Page::Pdp) {
            return false;
        }
        let trial_id = if self.page == Page::List && self.main_has_list() {
            17
        } else {
            13
        };
        let family = if self.trial_available && button.id == trial_id {
            "realms_button"
        } else {
            "button"
        };
        let suffix = if !button.enabled {
            "_disabled"
        } else if highlighted {
            "_highlighted"
        } else {
            ""
        };
        let sprite = format!("gui/sprites/widget/{family}{suffix}");
        // RealmsButton metadata is 200x20, border 3 (disabled 1), with tiled inner sections.
        if family == "realms_button" {
            realms_button(atlas, batch, &sprite, button);
        } else {
            crate::ui::nine_slice(
                batch,
                atlas,
                &sprite,
                button.x,
                button.y,
                button.w,
                20.0,
                u32::MAX,
            );
        }
        if let Some((icon, size)) = button.icon {
            batch.sprite_scaled(
                atlas,
                &format!("gui/sprites/{icon}"),
                button.x + 3.0,
                button.y + 3.0,
                size,
                size,
                u32::MAX,
            );
            let count = if button.id == 11 {
                self.pending_invite_count
            } else if self.unread_news {
                usize::MAX
            } else {
                0
            };
            if count > 0 {
                let badge = if count > 5 {
                    "more".into()
                } else {
                    count.to_string()
                };
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/sprites/notification/{badge}"),
                    button.x + 15.0,
                    button.y - 3.0,
                    8.0,
                    8.0,
                    u32::MAX,
                );
            }
        } else {
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
        true
    }

    pub(in crate::title) fn draw_tooltip(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
    ) {
        if self.draw_subscreen_tooltip(atlas, batch, cursor) {
            return;
        }
        if !matches!(self.page, Page::List | Page::Pdp) {
            return;
        }
        let c = (batch.width / 2.0).floor();
        let key = if (12.0..32.0).contains(&cursor.1) && (c + 110.0..c + 130.0).contains(&cursor.0)
        {
            Some(if self.pending_invite_count == 0 {
                "mco.invites.nopending"
            } else {
                "mco.invites.pending"
            })
        } else if (12.0..32.0).contains(&cursor.1) && (c + 134.0..c + 154.0).contains(&cursor.0) {
            Some("mco.news")
        } else {
            None
        };
        if let Some(key) = key {
            crate::ui::tooltip(batch, atlas, &[Text::translate(key, &[])], cursor);
        } else if let Some(message) = self.realm_status_tooltip(cursor, (batch.width, batch.height))
        {
            crate::ui::tooltip(batch, atlas, &[message], cursor);
        }
    }

    pub(super) fn draw_entry(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        started: std::time::Instant,
    ) -> bool {
        self.viewport_height.set(batch.height);
        if self.draw_subscreen(atlas, batch) {
            return true;
        }
        if matches!(
            self.page,
            Page::AvailabilityError | Page::ConfigurationError
        ) {
            if self.availability_error_kind == Some(AvailabilityErrorKind::ParentalConsent) {
                let lines = atlas.font.split(
                    &Text::translate("mco.account.privacy.information", &[]),
                    (batch.width - 15.0).max(1.0),
                );
                let body_height = lines.len() as f32 * 9.0;
                let y = ((batch.height - body_height - 35.0) / 2.0).round();
                self.privacy_button_y.set(y + body_height + 15.0);
                for (i, line) in lines.iter().enumerate() {
                    let x =
                        (batch.width / 2.0).floor() - (atlas.font.width_text(line) / 2.0).floor();
                    batch.text_styled(atlas, line, x, y + i as f32 * 9.0, u32::MAX, true);
                }
                return true;
            }
            if self.availability_error_kind == Some(AvailabilityErrorKind::Incompatible) {
                super::super::centered(
                    atlas,
                    batch,
                    &self.availability_title,
                    12.0,
                    rgba(255, 0, 0, 255),
                );
                let detail = Text::translate(
                    "mco.client.outdated.stable.version",
                    &[Text::colored(
                        horizon_core::version::VANILLA_VERSION,
                        0xff0000,
                    )],
                );
                // MultiLineTextWidget here has no maximum width: only explicit line breaks wrap.
                let lines = atlas.font.split(&detail, f32::MAX);
                let y = 63.0_f32.min(batch.height - 33.0 - lines.len() as f32 * 9.0);
                for (i, line) in lines.iter().enumerate() {
                    let x =
                        (batch.width / 2.0).floor() - (atlas.font.width_text(line) / 2.0).floor();
                    batch.text_styled(atlas, line, x, y + i as f32 * 9.0, u32::MAX, true);
                }
                return true;
            }
            super::super::centered(atlas, batch, &self.availability_title, 80.0, u32::MAX);
            let detail = Text::literal(self.error.as_deref().unwrap_or_default());
            for (i, line) in atlas
                .font
                .split(&detail, (batch.width * 3.0 / 4.0).floor())
                .iter()
                .enumerate()
            {
                let x = (batch.width / 2.0).floor() - (atlas.font.width_text(line) / 2.0).floor();
                batch.text_styled(
                    atlas,
                    line,
                    x,
                    100.0 + i as f32 * 9.0,
                    rgba(223, 80, 80, 255),
                    true,
                );
            }
            return true;
        }
        if !matches!(self.page, Page::List | Page::Pdp) {
            return false;
        }
        let (w, h) = (batch.width, batch.height);
        self.join_button_width
            .set((HudBatch::text_width(atlas, &tr("mco.selectServer.joinRealm")) + 16.0).min(90.0));
        batch.sprite_region(
            atlas,
            "gui/title/realms",
            [(w / 2.0).floor() - 64.0, 5.0, 128.0, 34.0],
            128.0,
            64.0,
        );
        if self.page == Page::Pdp {
            self.draw_pdp(atlas, batch, started);
        } else if !self.servers_loaded {
            let y = 74.0_f32.min(h - 70.0);
            super::super::centered(
                atlas,
                batch,
                &tr("mco.selectServer.loading"),
                y + 2.0,
                u32::MAX,
            );
            let frame = (started.elapsed().as_millis() / 300 % 4) as usize;
            super::super::centered(
                atlas,
                batch,
                ["O o o", "o O o", "o o O", "o O o"][frame],
                y + 17.0,
                rgba(128, 128, 128, 255),
            );
        } else if !self.main_has_list() {
            let lines = atlas
                .font
                .split(&Text::translate("mco.selectServer.noRealms", &[]), 300.0);
            let group_height = 64.0 + 8.0 + lines.len() as f32 * 9.0 + 8.0;
            let y = 74.0_f32.min(h - 42.0 - group_height);
            batch.sprite_scaled(
                atlas,
                "gui/realms/no_realms",
                (w / 2.0).floor() - 65.0,
                y,
                130.0,
                64.0,
                u32::MAX,
            );
            let text_width = lines
                .iter()
                .map(|line| atlas.font.width_text(line))
                .fold(0.0, f32::max);
            let rect = [
                (w / 2.0).floor() - ((text_width + 8.0) / 2.0).floor(),
                y + 72.0,
                text_width + 8.0,
                lines.len() as f32 * 9.0 + 8.0,
            ];
            self.empty_text_rect.set(rect);
            if self.list_focused.get() {
                selection(atlas, batch, rect[0], rect[1], rect[2], rect[3], true);
            }
            for (i, line) in lines.iter().enumerate() {
                let x = (w / 2.0).floor() - (atlas.font.width_text(line) / 2.0).floor();
                batch.text_styled(atlas, line, x, y + 76.0 + i as f32 * 9.0, u32::MAX, true);
            }
        } else {
            self.draw_server_list(atlas, batch, started);
        }
        true
    }

    fn draw_pdp(&self, atlas: &HudAtlas, batch: &mut HudBatch, started: std::time::Instant) {
        let title = Text::translate("mco.pdp.trial.title", &[]).underlined();
        let subtitle = Text::translate("mco.pdp.trial.description", &[]);
        let features = [
            (
                "friends",
                "mco.pdp.friends.title",
                &["mco.pdp.friends.description"][..],
            ),
            (
                "minigames",
                "mco.pdp.minigames.title",
                &["mco.pdp.minigames.description"][..],
            ),
            (
                "private_server",
                "mco.pdp.private.title",
                &[
                    "mco.pdp.private.backups",
                    "mco.pdp.private.easyToManage",
                    "mco.pdp.private.worldSlots",
                    "mco.pdp.private.secure",
                ][..],
            ),
        ];
        let mut heights = [0.0; 4];
        heights[0] = wrap_height(atlas, &title)
            + 20.0
            + if self.trial_available {
                2.0 + wrap_height(atlas, &subtitle)
            } else {
                0.0
            };
        for (i, (_, key, descriptions)) in features.iter().enumerate() {
            let text_height = wrap_height(atlas, &Text::translate(key, &[]).bold())
                + 4.0
                + descriptions
                    .iter()
                    .map(|key| 2.0 + wrap_height(atlas, &Text::translate(key, &[])))
                    .sum::<f32>();
            heights[i + 1] = text_height.max(45.0) + 16.0;
        }
        self.pdp_heights.set(heights);
        let bottom = self.list_bottom(batch.height);
        self.prepare_list(atlas, batch, heights.iter().sum::<f32>() + 4.0);
        let first = batch.vertices.len();
        let c = (batch.width / 2.0).floor();
        let left = c - 145.0;
        let mut y = 46.0 - self.pixel_scroll.get().floor();
        for (i, height) in heights.iter().enumerate() {
            if self.selected == Some(i) {
                selection(
                    atlas,
                    batch,
                    left,
                    y,
                    290.0,
                    *height,
                    self.list_focused.get(),
                );
            }
            if i == 0 {
                let x = c - (atlas.font.width_text(&title).min(182.0) / 2.0).floor();
                let next = wrapped(atlas, batch, &title, x, y + 6.0) + 4.0;
                if self.trial_available {
                    let x = c - (atlas.font.width_text(&subtitle).min(182.0) / 2.0).floor();
                    // Trial sprite: six vertical frames, durations 10,2,2,2,2,2 ticks.
                    if let Some(mut r) = atlas
                        .sprites
                        .get("gui/sprites/icon/trial_available")
                        .copied()
                    {
                        let tick = (started.elapsed().as_millis() / 50 % 20) as usize;
                        let frame = if tick < 10 { 0 } else { 1 + (tick - 10) / 2 };
                        let v = (r.v1 - r.v0) / 6.0;
                        r.v0 += frame as f32 * v;
                        r.v1 = r.v0 + v;
                        batch.rect_uv(x - 12.0, next, 8.0, 8.0, r, u32::MAX);
                    }
                    wrapped(atlas, batch, &subtitle, x, next);
                }
            } else {
                let (image, key, descriptions) = &features[i - 1];
                batch.sprite_scaled(
                    atlas,
                    &format!("gui/realms/{image}"),
                    left + 6.0,
                    y + 6.0,
                    80.0,
                    45.0,
                    u32::MAX,
                );
                let x = left + 102.0;
                let mut text_y =
                    wrapped(atlas, batch, &Text::translate(key, &[]).bold(), x, y + 6.0) + 4.0;
                for key in *descriptions {
                    text_y = wrapped(atlas, batch, &Text::translate(key, &[]), x, text_y) + 2.0;
                }
            }
            y += height;
        }
        batch.clip_since(first, 0.0, 44.0, batch.width, bottom);
        self.finish_list(atlas, batch, c + 153.0);
    }

    fn prepare_list(&self, atlas: &HudAtlas, batch: &mut HudBatch, content: f32) {
        self.content_height.set(content);
        let bottom = self.list_bottom(batch.height);
        let max = (content - (bottom - 44.0)).max(0.0);
        self.pixel_scroll
            .set(self.pixel_scroll.get().clamp(0.0, max));
        // Preserve list texture anchoring in screen X and scrolled content Y.
        let start = batch.vertices.len();
        let shift = self.pixel_scroll.get().floor() % 32.0;
        crate::ui::list_background(batch, atlas, 44.0 - shift, bottom + 32.0, false);
        batch.clip_since(start, 0.0, 44.0, batch.width, bottom);
    }

    fn finish_list(&self, atlas: &HudAtlas, batch: &mut HudBatch, x: f32) {
        let bottom = self.list_bottom(batch.height);
        let height = bottom - 44.0;
        let content = self.content_height.get();
        if content > height && height > 8.0 {
            let thumb = (height * height / content)
                .floor()
                .clamp(32.0_f32.min(height - 8.0), height - 8.0);
            let y = 44.0
                + (self.pixel_scroll.get().floor() * (height - thumb) / (content - height)).floor();
            batch.fill(atlas, x, 44.0, 6.0, height, rgba(0, 0, 0, 255));
            batch.fill(atlas, x, y, 6.0, thumb, rgba(128, 128, 128, 255));
            batch.fill(atlas, x, y, 5.0, thumb - 1.0, rgba(192, 192, 192, 255));
        }
        crate::ui::separators(batch, atlas, 44.0, bottom, false);
    }

    fn visit_notification(&self) -> Option<&Value> {
        self.notifications
            .iter()
            .find(|n| text(n, "type") == "visitUrl")
    }

    fn draw_notification(&self, atlas: &HudAtlas, batch: &mut HudBatch, left: f32, y: f32) {
        let Some(notification) = self.visit_notification() else {
            return;
        };
        let message = Text::literal(notification_text(notification));
        let lines = atlas.font.split(&message, 220.0);
        batch.sprite_scaled(
            atlas,
            "gui/sprites/icon/info",
            left + 9.0,
            y + 9.0,
            20.0,
            20.0,
            u32::MAX,
        );
        for (i, line) in lines.iter().enumerate() {
            let x = left + 150.0 - (atlas.font.width_text(line) / 2.0).floor();
            batch.text_styled(atlas, line, x, y + 9.0 + i as f32 * 9.0, u32::MAX, true);
        }
        let label = notification
            .get("buttonText")
            .map(|v| notification_text(&json!({"message":v})))
            .unwrap_or_else(|| tr("mco.notification.visitUrl.buttonText.default"));
        let button = Button {
            x: left + 75.0,
            y: y + 13.0 + lines.len() as f32 * 9.0,
            w: 150.0,
            label,
            enabled: true,
            id: 0,
            icon: None,
            trailing: None,
        };
        crate::widgets::record(&button.label, [button.x, button.y, button.w, 20.0], true);
        crate::ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/widget/button",
            button.x,
            button.y,
            button.w,
            20.0,
            u32::MAX,
        );
        button.draw_label(batch, atlas, u32::MAX);
        if flag(notification, "dismissable") {
            batch.sprite_scaled(
                atlas,
                "gui/sprites/widget/cross_button",
                left + 279.0,
                y + 9.0,
                14.0,
                14.0,
                u32::MAX,
            );
        }
    }

    fn draw_server_list(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        started: std::time::Instant,
    ) {
        let notification_height = self.visit_notification().map_or(0.0, |n| {
            38.0 + atlas
                .font
                .split(&Text::literal(notification_text(n)), 220.0)
                .len() as f32
                * 9.0
        });
        self.notification_height.set(notification_height);
        self.prepare_list(
            atlas,
            batch,
            notification_height + self.servers.len() as f32 * 36.0 + 4.0,
        );
        let first = batch.vertices.len();
        let left = (batch.width / 2.0).floor() - 150.0;
        self.draw_notification(atlas, batch, left, 46.0 - self.pixel_scroll.get().floor());
        for (i, server) in self.servers.iter().enumerate() {
            let y = 46.0 + notification_height + i as f32 * 36.0 - self.pixel_scroll.get().floor();
            if self.selected == Some(i) {
                selection(atlas, batch, left, y, 300.0, 36.0, self.list_focused.get());
            }
            let (x, top, right) = (left + 2.0, y + 2.0, left + 298.0);
            if text(server, "state") == "UNINITIALIZED" {
                batch.sprite_scaled(
                    atlas,
                    "gui/sprites/icon/new_realm",
                    x - 5.0,
                    top + 6.0,
                    40.0,
                    20.0,
                    u32::MAX,
                );
                batch.text(
                    atlas,
                    &tr("mco.selectServer.uninitialized"),
                    x + 38.0,
                    top + 12.0,
                    rgba(127, 255, 127, 255),
                    true,
                );
                continue;
            }
            draw_owner(atlas, batch, server, x, top);
            let owned = self.owned(server);
            let expired = flag(server, "expired");
            let status = if expired {
                Some("expired")
            } else if text(server, "state") == "CLOSED" {
                Some("closed")
            } else if owned && number(server, "daysLeft") < 7 {
                Some("expires_soon")
            } else if text(server, "state") == "OPEN" {
                Some("open")
            } else {
                None
            };
            if let Some(status) = status {
                let name = format!("gui/sprites/realm_status/{status}");
                if let Some(mut r) = atlas.sprites.get(&name).copied() {
                    // Only expires_soon is animated: 10x56, two 28px frames at 10 ticks.
                    // open/closed/expired are complete 10x28 traffic-light sprites.
                    if status == "expires_soon" && r.h > 28 {
                        let frame = (started.elapsed().as_millis() / 500 % 2) as f32;
                        let dv = (r.v1 - r.v0) / 2.0;
                        r.v0 += frame * dv;
                        r.v1 = r.v0 + dv;
                    }
                    batch.rect_uv(right - 17.0, top + 2.0, 10.0, 28.0, r, u32::MAX);
                }
            }
            let version = text(server, "activeVersion");
            let minigame = text(server, "worldType") == "MINIGAME";
            let version_x = right - 20.0 - HudBatch::text_width(atlas, &version);
            clamped(
                atlas,
                batch,
                &text(server, "name"),
                x + 38.0,
                top + 1.0,
                version_x,
                u32::MAX,
            );
            if !minigame {
                batch.text(
                    atlas,
                    &version,
                    version_x,
                    top + 1.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
            }
            if minigame {
                let detail = translated(
                    "mco.selectServer.minigameName",
                    &[text(server, "minigameName")],
                );
                batch.text(
                    atlas,
                    &detail,
                    x + 38.0,
                    top + 12.0,
                    rgba(255, 255, 85, 255),
                    true,
                );
            } else {
                let mode = server
                    .get("gameMode")
                    .and_then(Value::as_i64)
                    .and_then(|mode| {
                        [
                            "gameMode.survival",
                            "gameMode.creative",
                            "gameMode.adventure",
                            "gameMode.spectator",
                        ]
                        .get(mode as usize)
                    })
                    .map(|key| {
                        tr(if flag(server, "isHardcore") {
                            "gameMode.hardcore"
                        } else {
                            key
                        })
                    })
                    .unwrap_or_default();
                let mode_x = right - 20.0 - HudBatch::text_width(atlas, &mode);
                batch.text(
                    atlas,
                    &mode,
                    mode_x,
                    top + 12.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
                clamped(
                    atlas,
                    batch,
                    &text(server, "motd"),
                    x + 38.0,
                    top + 12.0,
                    mode_x,
                    rgba(128, 128, 128, 255),
                );
            }
            if !owned {
                batch.text(
                    atlas,
                    &text(server, "owner"),
                    x + 38.0,
                    top + 23.0,
                    rgba(128, 128, 128, 255),
                    true,
                );
            } else if expired {
                batch.text(
                    atlas,
                    &tr(if flag(server, "expiredTrial") {
                        "mco.selectServer.expiredTrial"
                    } else {
                        "mco.selectServer.expiredList"
                    }),
                    x + 38.0,
                    top + 23.0,
                    rgba(223, 80, 80, 255),
                    true,
                );
            }
        }
        batch.clip_since(
            first,
            0.0,
            44.0,
            batch.width,
            self.list_bottom(batch.height),
        );
        self.finish_list(atlas, batch, left + 308.0);
    }

    pub(super) fn press_entry_row(&mut self, x: f32, y: f32, size: (f32, f32)) -> (Action, bool) {
        if self.page == Page::List && !self.main_has_list() {
            return (Action::None, false);
        }
        let half_width = if self.page == Page::Pdp { 145.0 } else { 150.0 };
        let c = (size.0 / 2.0).floor();
        let bottom = self.list_bottom(size.1);
        if !(44.0..bottom).contains(&y) {
            return (Action::None, false);
        }
        let bar_x = c + half_width + 8.0;
        if (bar_x..bar_x + 6.0).contains(&x) {
            let max = (self.content_height.get() - (bottom - 44.0)).max(0.0);
            self.dragging_scrollbar = max > 0.0;
            self.drag_scrollbar(y, size.1);
            return (Action::None, false);
        }
        if !(c - half_width..c + half_width).contains(&x) {
            return (Action::None, false);
        }
        self.list_focused.set(true);
        let local_y = y - 46.0 + self.pixel_scroll.get().floor();
        if local_y < 0.0 {
            return (Action::None, false);
        }
        if self.page == Page::Pdp {
            let mut bottom = 0.0;
            self.selected = self.pdp_heights.get().iter().position(|height| {
                bottom += height;
                local_y < bottom
            });
        } else {
            if local_y < self.notification_height.get() {
                if let Some(notification) = self.visit_notification().cloned() {
                    if flag(&notification, "dismissable")
                        && x >= c + 129.0
                        && (9.0..23.0).contains(&local_y)
                    {
                        return (
                            self.dispatch(RealmsAction::DismissNotification(text(
                                &notification,
                                "notificationUuid",
                            ))),
                            false,
                        );
                    }
                    let button_y = self.notification_height.get() - 25.0;
                    if (c - 75.0..c + 75.0).contains(&x)
                        && (button_y..button_y + 20.0).contains(&local_y)
                    {
                        let url = text(&notification, "url");
                        if valid_link(&url) {
                            self.ask_action(Action::OpenUri(url.clone()), "chat.link.confirm", url);
                        }
                    }
                }
                return (Action::None, false);
            }
            let i = ((local_y - self.notification_height.get()) / 36.0) as usize;
            if let Some(server) = self.servers.get(i).cloned() {
                let now = std::time::Instant::now();
                let double = self.last_row_click.is_some_and(|(previous, time)| {
                    previous == i && now.duration_since(time).as_millis() < 250
                });
                self.last_row_click = Some((i, now));
                self.selected = Some(i);
                if text(&server, "state") == "UNINITIALIZED" {
                    self.server = server;
                    self.enter(Page::Initialize);
                } else if double && self.playable(&server) {
                    return (
                        self.dispatch(RealmsAction::Join(number(&server, "id"))),
                        false,
                    );
                }
            }
        }
        (Action::None, false)
    }

    pub(in crate::title) fn drag_scrollbar(&mut self, y: f32, height: f32) -> bool {
        if !self.dragging_scrollbar {
            return false;
        }
        let top = if self.page == Page::Invites {
            33.0
        } else {
            44.0
        };
        let view = self.list_bottom(height) - top;
        let content = self.content_height.get();
        if content > view && view > 8.0 {
            let thumb = (view * view / content)
                .floor()
                .clamp(32.0_f32.min(view - 8.0), view - 8.0);
            self.pixel_scroll.set(
                ((y - top - thumb / 2.0) / (view - thumb) * (content - view))
                    .clamp(0.0, content - view),
            );
        }
        true
    }

    pub(in crate::title) fn release_scrollbar(&mut self) {
        self.dragging_scrollbar = false;
    }

    pub(super) fn arrow_entry(&mut self, up: bool, size: (f32, f32)) {
        self.list_focused.set(true);
        let heights = if self.page == Page::Pdp {
            self.pdp_heights.get().to_vec()
        } else {
            vec![36.0; self.servers.len()]
        };
        if heights.is_empty() {
            return;
        }
        let index = self.selected.map_or(0, |i| {
            if up {
                i.saturating_sub(1)
            } else {
                (i + 1).min(heights.len() - 1)
            }
        });
        self.selected = Some(index);
        let top = heights[..index].iter().sum::<f32>()
            + if self.page == Page::List {
                self.notification_height.get()
            } else {
                0.0
            };
        let bottom = top + heights[index];
        let visible = self.list_bottom(size.1) - 44.0;
        if top < self.pixel_scroll.get() {
            self.pixel_scroll.set(top);
        } else if bottom > self.pixel_scroll.get() + visible {
            self.pixel_scroll.set((bottom - visible).max(0.0));
        }
    }
}

fn realms_button(atlas: &HudAtlas, batch: &mut HudBatch, sprite: &str, button: &Button) {
    let Some(r) = atlas.sprites.get(sprite).copied() else {
        return;
    };
    let border = if button.enabled { 3.0 } else { 1.0 };
    // The height is unchanged. The horizontal inner section tiles from its left edge.
    // This is the 200x20 RealmsButton nine-slice, including the 1px disabled border.
    let sections = [
        (0.0, border, 0.0),
        (border, button.w - 2.0 * border, border),
        (button.w - border, border, 200.0 - border),
    ];
    for (dx, width, source) in sections {
        let mut part = r;
        part.u0 = r.u0 + (r.u1 - r.u0) * source / 200.0;
        part.u1 = r.u0 + (r.u1 - r.u0) * (source + width) / 200.0;
        batch.rect_uv(button.x + dx, button.y, width, 20.0, part, u32::MAX);
    }
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}
fn wrap_height(atlas: &HudAtlas, text: &Text) -> f32 {
    atlas.font.split(text, 182.0).len() as f32 * 9.0
}
fn wrapped(atlas: &HudAtlas, batch: &mut HudBatch, text: &Text, x: f32, mut y: f32) -> f32 {
    for line in atlas.font.split(text, 182.0) {
        batch.text_styled(atlas, &line, x, y, u32::MAX, true);
        y += 9.0;
    }
    y
}
#[allow(clippy::too_many_arguments)]
fn selection(
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    focused: bool,
) {
    batch.fill(
        atlas,
        x,
        y,
        w,
        h,
        if focused {
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
fn clamped(
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    text: &str,
    x: f32,
    y: f32,
    right: f32,
    color: u32,
) {
    let width = (right - x).max(0.0);
    let value = if HudBatch::text_width(atlas, text) > width {
        format!(
            "{}...",
            &text[..atlas
                .font
                .fit(text, (width - HudBatch::text_width(atlas, "... ")).max(0.0))]
        )
    } else {
        text.to_owned()
    };
    batch.text(atlas, &value, x, y, color, true);
}
pub(super) fn draw_owner(atlas: &HudAtlas, batch: &mut HudBatch, server: &Value, x: f32, y: f32) {
    let uuid = normalized(&text(server, "ownerUUID"));
    let id = u128::from_str_radix(&uuid, 16).unwrap_or_default();
    batch
        .inline_profiles
        .push(json!({"id": text(server, "ownerUUID"), "name": text(server, "owner")}));
    let skin = atlas
        .object_skins
        .get(&format!("id:{id:032x}"))
        .or_else(|| {
            atlas.object_skins.get(&format!(
                "name:{}",
                text(server, "owner").to_ascii_lowercase()
            ))
        })
        .filter(|skin| atlas.sprites.contains_key(*skin))
        .map(String::as_str)
        .unwrap_or_else(|| crate::tab_list::default_skin(id));
    if let Some(r) = atlas.sprites.get(skin).copied() {
        for u in [8.0, 40.0] {
            let mut face = r;
            face.u0 = r.u0 + (r.u1 - r.u0) * u / 64.0;
            face.u1 = r.u0 + (r.u1 - r.u0) * (u + 8.0) / 64.0;
            face.v0 = r.v0 + (r.v1 - r.v0) * 8.0 / 64.0;
            face.v1 = r.v0 + (r.v1 - r.v0) * 16.0 / 64.0;
            batch.rect_uv(x, y, 32.0, 32.0, face, u32::MAX);
        }
    }
}
