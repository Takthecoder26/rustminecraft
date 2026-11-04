//! FriendsOverlayScreen, FriendsTab/PendingTab and their entry containers (26.3).
//! Service requests and the preserved parent screen belong to gameplay.
use super::*;
use crate::social_service::{Friend, PresenceStatus};

const WIDTH: f32 = 220.0;
const LIST_WIDTH: f32 = 204.0;
const TOP: f32 = 40.0;
const ADD_HEIGHT: f32 = 44.0;
const GRAY: u32 = 0xffa0a0a0;

fn presence_key(status: PresenceStatus) -> &'static str {
    match status {
        PresenceStatus::Online => "gui.friends.presence.status.online",
        PresenceStatus::Offline => "gui.friends.presence.status.offline",
        PresenceStatus::PlayingOffline => "gui.friends.presence.status.playing_offline",
        PresenceStatus::PlayingRealms => "gui.friends.presence.status.playing_realms",
        PresenceStatus::PlayingServer => "gui.friends.presence.status.playing_server",
        PresenceStatus::PlayingHostedServer => "gui.friends.presence.status.playing_hosted_server",
    }
}

fn presence_order(status: PresenceStatus) -> u8 {
    match status {
        PresenceStatus::PlayingHostedServer => 0,
        PresenceStatus::PlayingServer => 1,
        PresenceStatus::PlayingRealms => 2,
        PresenceStatus::PlayingOffline => 3,
        PresenceStatus::Online => 4,
        PresenceStatus::Offline => 5,
    }
}

fn sprite(batch: &mut HudBatch, atlas: &HudAtlas, name: &str, rect: [f32; 4]) {
    ui::blit_sprite(
        batch,
        atlas,
        &format!("gui/sprites/friends/{name}"),
        rect[0],
        rect[1],
        rect[2],
        rect[3],
        u32::MAX,
    );
}

impl OptionsUi {
    /// The caller draws the preserved parent with no hover and applies screen blur first.
    pub fn friends_overlay_visible(&self) -> bool {
        self.page() == Page::Friends
            && !self.online_consent
            && !self.online_privacy
            && self.friend_removal.is_none()
    }

    pub(super) fn friends_dialog_layout(&self) -> Vec<Widget> {
        let removal = self.friend_removal.is_some();
        let mut buttons = Vec::new();
        for (i, [x, y, width, height]) in self.friends_dialog_buttons.iter().copied().enumerate() {
            let action = if removal {
                OnlineAction::ConfirmRemoval(i == 0)
            } else {
                OnlineAction::ConfirmFriends(i == 0)
            };
            let key = match (removal, i) {
                (true, 0) => "gui.remove",
                (true, _) => "gui.cancel",
                (false, 0) => "options.friendsList.confirm.turnOn",
                (false, _) => "options.friendsList.confirm.turnOff",
            };
            let mut wd = button(x, y, width, tr(key), Act::Online(action));
            wd.h = height;
            if self.online_status.busy {
                wd = inactive(wd);
            }
            buttons.push(wd);
        }
        if !removal {
            for &[x, y, width, height] in &self.friends_link_rects {
                let mut wd = button(
                    x,
                    y,
                    width,
                    tr("options.friendsList.confirm.message.link"),
                    Act::Online(OnlineAction::XboxPrivacy),
                );
                wd.h = height;
                buttons.push(wd);
            }
        }
        buttons
    }

    pub(super) fn draw_friends_dialog(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        let (w, h) = (batch.width, batch.height);
        self.size = (w, h);
        let removal = self.friend_removal.is_some();
        if removal {
            self.draw_friends(batch, atlas, (-1.0, -1.0));
        }
        crate::widgets::begin_screen(|| {
            crate::widgets::ScreenInfo::new(
                if removal {
                    "options/friends/remove"
                } else {
                    "options/online/consent"
                },
                tr(if removal {
                    "gui.friends.confirm_title"
                } else {
                    "options.friendsList.confirm.title"
                })
                .plain(),
            )
        });
        batch.fill(atlas, 0.0, 0.0, w, h, 0x80000000);
        let title = if removal {
            tr("gui.friends.confirm_title").bold()
        } else {
            tr("options.friendsList.confirm.title").underlined()
        };
        let message = if removal {
            tr("gui.friends.confirm_unfriend")
        } else {
            Text::translate(
                "options.friendsList.confirm.message",
                &[tr("options.friendsList.confirm.message.link")
                    .with_color(0x5555ff)
                    .underlined()],
            )
        };
        let lines: Vec<_> = atlas
            .font
            .split(&message, if removal { 214.0 } else { (w - 180.0).max(1.0) })
            .into_iter()
            .take(15)
            .collect();
        let message_width = lines
            .iter()
            .map(|line| atlas.font.width_text(line))
            .fold(0.0_f32, f32::max);
        let layout_width = if removal {
            214.0
        } else {
            304.0_f32.max(message_width + 20.0)
        };
        let layout_height = lines.len() as f32 * 9.0 + if removal { 53.0 } else { 71.0 };
        let left = ((w - layout_width) / 2.0).floor();
        let top = ((h - layout_height) / 2.0).floor();
        ui::blit_sprite(
            batch,
            atlas,
            if removal {
                "gui/sprites/popup/background"
            } else {
                "gui/sprites/friends/background_dark"
            },
            left - 18.0,
            top - 18.0,
            layout_width + if removal { 36.0 } else { 37.0 },
            layout_height + if removal { 36.0 } else { 37.0 },
            u32::MAX,
        );
        batch.text_styled(
            atlas,
            &title,
            ((w - atlas.font.width_text(&title)) / 2.0).floor(),
            top,
            u32::MAX,
            true,
        );
        self.friends_link_rects.clear();
        let message_x = ((w - message_width) / 2.0).floor();
        for (i, line) in lines.iter().enumerate() {
            let y = top + if removal { 21.0 } else { 22.0 } + i as f32 * 9.0;
            let mut x = if removal {
                ((w - atlas.font.width_text(line)) / 2.0).floor()
            } else {
                message_x
            };
            batch.text_styled(atlas, line, x, y, u32::MAX, true);
            for span in &line.spans {
                let width = atlas
                    .font
                    .width_text(&Text::styled(&span.text, span.style.clone()));
                if !removal && span.style.underlined == Some(true) {
                    self.friends_link_rects.push([x, y, width, 9.0]);
                }
                x += width;
            }
        }
        let width = if removal { 104.0 } else { 150.0 };
        let gap = if removal { 6.0 } else { 4.0 };
        let x = ((w - width * 2.0 - gap) / 2.0).floor();
        let y = top + layout_height - 20.0;
        self.friends_dialog_buttons = [[x, y, width, 20.0], [x + width + gap, y, width, 20.0]];
        for (i, wd) in self.friends_dialog_layout().iter().enumerate() {
            let WKind::Button { label, active, .. } = &wd.kind else {
                continue;
            };
            let hover = ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                || self.focus.as_ref().is_some_and(|f| matches_focus(f, i, wd));
            if i < 2 {
                ui::button(batch, atlas, wd.x, wd.y, wd.w, wd.h, label, *active, hover);
            } else {
                crate::widgets::record(&label.plain(), [wd.x, wd.y, wd.w, wd.h], true);
            }
        }
    }

    /// TabNavigationBar.keyPressed, before ordinary edit-box input. `control`
    /// is the platform edit shortcut modifier (Control, or Command on macOS).
    /// Some means consumed, including selecting the already-current tab.
    pub fn friends_tab_shortcut(
        &mut self,
        key: winit::keyboard::KeyCode,
        control: bool,
        shift: bool,
    ) -> Option<Vec<Event>> {
        use winit::keyboard::KeyCode as K;
        if !self.friends_overlay_visible() || !control {
            return None;
        }
        let current = i32::from(self.friends_pending_tab);
        let index = match key {
            K::Digit1 | K::Numpad1 => 0,
            K::Digit2
            | K::Numpad2
            | K::Digit3
            | K::Numpad3
            | K::Digit4
            | K::Numpad4
            | K::Digit5
            | K::Numpad5
            | K::Digit6
            | K::Numpad6
            | K::Digit7
            | K::Numpad7
            | K::Digit8
            | K::Numpad8
            | K::Digit9
            | K::Numpad9
            | K::Digit0
            | K::Numpad0 => 1,
            K::Tab => (current + if shift { -1_i32 } else { 1 }).rem_euclid(2),
            _ => return None,
        };
        let pending = index == 1;
        let changed = pending != self.friends_pending_tab;
        if changed {
            let tabs_focused = matches!(self.focus, Some(FocusId::Act(Act::FriendsTab(_))));
            self.select_friends_tab(pending);
            if tabs_focused {
                self.focus = Some(FocusId::Act(Act::FriendsTab(pending)));
                self.focus_from_keyboard = true;
            }
        }
        Some(if changed {
            vec![Event::ButtonClick]
        } else {
            Vec::new()
        })
    }

    pub(super) fn select_friends_tab(&mut self, pending: bool) {
        if pending == self.friends_pending_tab {
            return;
        }
        self.friends_pending_tab = pending;
        self.scroll = 0.0;
        self.focus = None;
        self.settings_search.focused = false;
    }

    fn friend_presence_status(&self, id: u128) -> PresenceStatus {
        self.friend_presence
            .presence
            .iter()
            .find(|p| p.profile_id == id)
            .map_or(PresenceStatus::Offline, |p| p.status)
    }

    pub(super) fn refresh_friend_order(&mut self) {
        let mut order: Vec<_> = (0..self.friend_list.friends.len()).collect();
        order.sort_by_key(|&i| {
            presence_order(self.friend_presence_status(self.friend_list.friends[i].profile_id))
        });
        self.friend_order = order;
    }

    fn ordered_friends(&self) -> Vec<&Friend> {
        self.friend_order
            .iter()
            .filter_map(|&i| self.friend_list.friends.get(i))
            .collect()
    }

    pub(super) fn friend_list_error(&self) -> Option<&'static str> {
        if !self.online_status.available {
            Some("gui.friends.error.unauthorized")
        } else {
            self.online_status
                .message
                .filter(|key| key.starts_with("gui.friends.error."))
        }
    }

    fn friends_loading(&self) -> bool {
        self.friend_list_error().is_none() && !self.friends_ready
    }

    pub(super) fn friends_layout(&mut self, w: f32, h: f32) -> (Vec<Widget>, f32, f32, f32) {
        let left = (w / 2.0).floor() - WIDTH / 2.0;
        let top = TOP
            + if self.friends_pending_tab {
                0.0
            } else {
                ADD_HEIGHT
            };
        let bottom = h - TOP;
        let active = self.online_status.available
            && self.online_status.friends_enabled
            && self.friends_ready
            && self.friend_list_error().is_none()
            && !self.online_status.busy
            && !self.split_guest;
        let mut out = vec![
            button(
                left,
                TOP - 27.0,
                110.0,
                tr("gui.friends.tab_friends"),
                Act::FriendsTab(false),
            ),
            button(
                left + 110.0,
                TOP - 27.0,
                110.0,
                Text::translate(
                    "gui.friends.requests_count",
                    &[Text::literal(
                        self.friend_list.incoming_requests.len().to_string(),
                    )],
                ),
                Act::FriendsTab(true),
            ),
        ];
        if !self.friends_pending_tab {
            self.settings_search.x = left + 8.0;
            self.settings_search.y = TOP + 3.0;
            self.settings_search.width = 181.0;
            self.settings_search.height = 20.0;
            self.settings_search.max_length = 32;
            let send = button(
                left + 192.0,
                TOP + 3.0,
                20.0,
                tr("gui.friends.send_request"),
                Act::Online(OnlineAction::SendRequest),
            );
            out.push(if active && !self.friend_request_name().is_empty() {
                send
            } else {
                inactive(send)
            });
            let [x, y, width, height] = self.friends_profile_rect;
            let mut copy = button(
                x,
                y,
                width,
                Text::literal(&self.friend_profile),
                Act::CopyFriendProfile,
            );
            copy.h = height;
            out.push(copy);
        }
        self.row_bands.clear();
        let mut y = top - self.scroll;
        if self.friends_ready && self.friend_list_error().is_none() {
            if self.friends_pending_tab {
                for (incoming, list) in [
                    (true, &self.friend_list.incoming_requests),
                    (false, &self.friend_list.outgoing_requests),
                ] {
                    if list.is_empty() {
                        continue;
                    }
                    y += 9.0;
                    for friend in list {
                        let row = self.row_bands.len();
                        self.row_bands.push([y, y + 28.0]);
                        let mut add = |x, key, action| {
                            let mut wd = button(x, y + 4.0, 20.0, tr(key), Act::Online(action));
                            if !active {
                                wd = inactive(wd);
                            }
                            wd.list = true;
                            wd.row = row;
                            out.push(wd);
                        };
                        if incoming {
                            add(
                                left + 168.0,
                                "gui.friends.accept",
                                OnlineAction::AcceptRequest(friend.profile_id),
                            );
                            add(
                                left + 192.0,
                                "gui.friends.decline",
                                OnlineAction::DeclineRequest(friend.profile_id),
                            );
                        } else {
                            add(
                                left + 192.0,
                                "gui.friends.cancel_request",
                                OnlineAction::RevokeRequest(friend.profile_id),
                            );
                        }
                        y += 28.0;
                    }
                }
            } else {
                // ReserveStrategy.BOTH reserves eight pixels on both sides of the 204px list.
                let ids: Vec<_> = self
                    .ordered_friends()
                    .iter()
                    .map(|p| p.profile_id)
                    .collect();
                for id in ids {
                    let mut wd = button(
                        left + 192.0,
                        y + 4.0,
                        20.0,
                        tr("gui.friends.unfriend"),
                        Act::Online(OnlineAction::RemoveFriend(id)),
                    );
                    if !active {
                        wd = inactive(wd);
                    }
                    wd.list = true;
                    wd.row = self.row_bands.len();
                    self.row_bands.push([y, y + 28.0]);
                    out.push(wd);
                    y += 28.0;
                }
                for &[x, link_y, width, height] in &self.friends_link_rects {
                    let mut link = button(
                        x,
                        link_y,
                        width,
                        tr("gui.friends.empty_state.link"),
                        Act::Online(OnlineAction::XboxPrivacy),
                    );
                    link.h = height;
                    link.list = true;
                    link.row = self.row_bands.len();
                    out.push(link);
                }
                y += self.friends_text_height;
            }
        }
        let content = (y + self.scroll - top).max(0.0);
        let previous = self.scroll;
        self.clamp_scroll(top, bottom, content);
        for widget in &mut out {
            if widget.list {
                widget.y += previous - self.scroll;
            }
        }
        for band in &mut self.row_bands {
            band.iter_mut().for_each(|y| *y += previous - self.scroll);
        }
        (out, top, bottom, content)
    }

    fn draw_friend_entry(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        player: &Friend,
        x: f32,
        y: f32,
        status: bool,
    ) {
        if self.friends_high_contrast {
            batch.fill(atlas, x - 4.0, y, LIST_WIDTH + 8.0, 28.0, 0xff000000);
        }
        batch.inline_profiles.push(serde_json::json!({
            "id": format!("{:032x}", player.profile_id),
            "name": player.name,
        }));
        let skin = atlas
            .object_skins
            .get(&format!("id:{:032x}", player.profile_id))
            .map_or_else(
                || crate::tab_list::default_skin(player.profile_id),
                String::as_str,
            );
        let mut draws = Vec::new();
        crate::tab_list::face(
            &mut draws,
            skin,
            x as i32,
            (y + 2.0) as i32,
            24,
            true,
            false,
            u32::MAX,
        );
        crate::hud_layout::paint(batch, atlas, &draws);
        batch.text_styled(
            atlas,
            &Text::literal(&player.name),
            x + 28.0,
            y + if status { 5.0 } else { 10.0 },
            u32::MAX,
            true,
        );
        if status {
            let presence = self.friend_presence_status(player.profile_id);
            let text =
                tr(presence_key(presence)).with_color(if presence == PresenceStatus::Offline {
                    0xa0a0a0
                } else {
                    0x00ff00
                });
            let mut draws = Vec::new();
            crate::screen_draw::scrolling_text(
                &mut draws,
                &crate::screen_draw::Measure(&atlas.font),
                &text,
                x as i32 + 28,
                x as i32 + 28,
                x as i32 + 182,
                y as i32 + 15,
                y as i32 + 24,
                crate::hud_overlays::now_ms(),
            );
            crate::screen_draw::paint(batch, atlas, &draws);
        }
    }

    fn friends_account_text(&self) -> Text {
        let link = tr("gui.friends.empty_state.link")
            .with_color(text::GRAY)
            .underlined();
        Text::translate(
            if self.friend_list.friends.is_empty() {
                "gui.friends.empty_state"
            } else {
                "gui.friends.manage_account_footer"
            },
            &[link],
        )
        .with_color(text::GRAY)
    }

    pub(super) fn draw_friends(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
    ) {
        let (w, h) = (batch.width, batch.height);
        let left = (w / 2.0).floor() - WIDTH / 2.0;
        let account = self.friends_account_text();
        let account_lines = atlas.font.split(&account, LIST_WIDTH);
        self.friends_text_height = account_lines.len() as f32 * 9.0;
        let label = tr("gui.friends.my_profile_name");
        self.friends_profile_rect = [
            left + 8.0 + atlas.font.width_text(&label),
            TOP + 29.0,
            atlas.font.width_text(&Text::literal(&self.friend_profile)),
            9.0,
        ];
        let (_, top, bottom, content) = self.friends_layout(w, h);
        self.size = (w, h);
        sprite(
            batch,
            atlas,
            "background",
            [left - 8.0, TOP - 8.0, WIDTH + 16.0, h - 80.0 + 17.0],
        );
        if !self.friends_pending_tab {
            if !self.friends_editable() {
                self.settings_search.focused = false;
                if self.focus == Some(FocusId::Search) {
                    self.focus = None;
                }
            }
            self.settings_search.message = tr("gui.friends.enter_nickname").plain();
            self.settings_search.text_color = if self.friends_editable() {
                0xffe0e0e0
            } else {
                0xff707070
            };
            self.settings_search.draw(batch, atlas, None);
            if self.settings_search.value.is_empty() && !self.settings_search.focused {
                batch.text_styled(
                    atlas,
                    &tr("gui.friends.enter_nickname"),
                    left + 12.0,
                    TOP + 9.0,
                    0xff707070,
                    true,
                );
            }
            let label = tr("gui.friends.my_profile_name");
            batch.text_styled(atlas, &label, left + 8.0, TOP + 29.0, GRAY, true);
            sprite(
                batch,
                atlas,
                "list_separator_top",
                [left, TOP + 42.0, WIDTH, 2.0],
            );
        }
        self.friends_link_rects.clear();
        let list_start = batch.vertices.len();
        let center_text = |batch: &mut HudBatch, text: &Text, y: f32, color| {
            batch.text_styled(
                atlas,
                text,
                left + 8.0 + ((LIST_WIDTH - atlas.font.width_text(text)) / 2.0).floor(),
                y,
                color,
                true,
            );
        };
        if let Some(error) = self.friend_list_error() {
            let lines = atlas.font.split(&tr(error), LIST_WIDTH);
            let y = top + ((bottom - top - lines.len() as f32 * 9.0) / 2.0).floor();
            for (i, line) in lines.iter().enumerate() {
                center_text(batch, line, y + i as f32 * 9.0, GRAY);
            }
        } else if self.friends_loading() {
            let y = top + ((bottom - top - 28.0) / 2.0).floor();
            center_text(
                batch,
                &tr(if self.friends_pending_tab {
                    "gui.friends.loading_requests"
                } else {
                    "gui.friends.loading_friends"
                }),
                y + 2.0,
                u32::MAX,
            );
            let frames = ["O o o", "o O o", "o o O", "o O o"];
            center_text(
                batch,
                &Text::literal(
                    frames[(crate::hud_overlays::now_ms() / 300).rem_euclid(4) as usize],
                ),
                y + 17.0,
                0xff808080,
            );
        } else if self.friends_pending_tab {
            if self.friend_list.incoming_requests.is_empty()
                && self.friend_list.outgoing_requests.is_empty()
            {
                let lines = atlas
                    .font
                    .split(&tr("gui.friends.pending.empty"), LIST_WIDTH);
                let y = top + ((bottom - top - lines.len() as f32 * 9.0) / 2.0).floor();
                for (i, line) in lines.iter().enumerate() {
                    center_text(batch, line, y + i as f32 * 9.0, GRAY);
                }
            } else {
                let mut y = top - self.scroll;
                for (key, list) in [
                    (
                        "gui.friends.pending.received",
                        &self.friend_list.incoming_requests,
                    ),
                    (
                        "gui.friends.pending.sent",
                        &self.friend_list.outgoing_requests,
                    ),
                ] {
                    if list.is_empty() {
                        continue;
                    }
                    center_text(batch, &tr(key).bold().underlined(), y, u32::MAX);
                    y += 9.0;
                    for player in list {
                        self.draw_friend_entry(batch, atlas, player, left + 8.0, y, false);
                        y += 28.0;
                    }
                }
            }
        } else {
            let empty = self.friend_list.friends.is_empty();
            let mut y = top - self.scroll;
            if empty {
                y = top + ((bottom - top - 56.0 - self.friends_text_height) / 2.0).floor();
                sprite(
                    batch,
                    atlas,
                    "illustrations_00",
                    [left + 46.0, y, 128.0, 48.0],
                );
                y += 56.0;
            } else {
                for player in self.ordered_friends() {
                    self.draw_friend_entry(batch, atlas, player, left + 8.0, y, true);
                    y += 28.0;
                }
            }
            for (i, line) in account_lines.iter().enumerate() {
                let line_y = y + i as f32 * 9.0;
                center_text(batch, line, line_y, GRAY);
                let mut x = left + 8.0 + ((LIST_WIDTH - atlas.font.width_text(line)) / 2.0).floor();
                for span in &line.spans {
                    let span_text = Text::styled(&span.text, span.style.clone());
                    let width = atlas.font.width_text(&span_text);
                    if span.style.underlined == Some(true) {
                        self.friends_link_rects.push([x, line_y, width, 9.0]);
                    }
                    x += width;
                }
            }
        }
        let (widgets, _, _, _) = self.friends_layout(w, h);
        batch.clip_since(list_start, left + 8.0, top, left + 212.0, bottom);
        let mut tooltip = None;
        for (i, wd) in widgets.iter().enumerate() {
            if wd.list && (wd.y + wd.h <= top || wd.y >= bottom) {
                continue;
            }
            let WKind::Button { label, active, act } = &wd.kind else {
                continue;
            };
            let hover = ui::hovered(cursor, wd.x, wd.y, wd.w, wd.h)
                && (!wd.list || (cursor.1 >= top && cursor.1 < bottom));
            let focused = self.focus.as_ref().is_some_and(|f| matches_focus(f, i, wd));
            crate::widgets::record(&label.plain(), [wd.x, wd.y, wd.w, wd.h], *active);
            crate::widgets::target([wd.x, wd.y, wd.w, wd.h]);
            crate::widgets::narratable(|| {
                crate::widgets::Narratable::button(label.plain(), [wd.x, wd.y, wd.w, wd.h], *active)
                    .key(crate::widgets::key_of(&("friends", i)))
                    .focused(focused)
            });
            let start = batch.vertices.len();
            match act {
                Act::FriendsTab(pending) => {
                    let selected = *pending == self.friends_pending_tab;
                    sprite(
                        batch,
                        atlas,
                        if hover || focused {
                            "button_highlighted"
                        } else if selected {
                            "button"
                        } else {
                            "button_disabled"
                        },
                        [wd.x, wd.y, wd.w, wd.h],
                    );
                    let tw = atlas.font.width_text(label).min(106.0);
                    batch.text_styled(
                        atlas,
                        label,
                        wd.x + ((wd.w - atlas.font.width_text(label)) / 2.0).floor(),
                        wd.y + if selected { 6.0 } else { 7.0 },
                        u32::MAX,
                        true,
                    );
                    if selected {
                        batch.fill(
                            atlas,
                            wd.x + ((wd.w - tw) / 2.0).floor(),
                            wd.y + 18.0,
                            tw,
                            1.0,
                            u32::MAX,
                        );
                    }
                }
                Act::CopyFriendProfile => {
                    batch.text_styled(
                        atlas,
                        &if hover || focused {
                            label.clone().underlined()
                        } else {
                            label.clone()
                        },
                        wd.x,
                        wd.y,
                        u32::MAX,
                        true,
                    );
                    if hover || focused {
                        tooltip = Some(tr("gui.friends.copy_to_clipboard"));
                    }
                }
                Act::Online(OnlineAction::XboxPrivacy) => {}
                Act::Online(action) => {
                    ui::nine_slice(
                        batch,
                        atlas,
                        if !active {
                            "gui/sprites/widget/button_disabled"
                        } else if hover || focused {
                            "gui/sprites/widget/button_highlighted"
                        } else {
                            "gui/sprites/widget/button"
                        },
                        wd.x,
                        wd.y,
                        20.0,
                        20.0,
                        u32::MAX,
                    );
                    let loading = self.friends_pending_action == Some(*action)
                        || (*action == OnlineAction::SendRequest && self.friends_loading());
                    if loading {
                        sprite(batch, atlas, "loading", [wd.x + 7.0, wd.y + 9.0, 5.0, 2.0]);
                        if hover || focused {
                            tooltip = Some(tr("gui.friends.button.loading"));
                        }
                        if wd.list {
                            batch.clip_since(start, left + 8.0, top, left + 212.0, bottom);
                        }
                        continue;
                    }
                    let (name, iw, ih): (&str, f32, f32) = match action {
                        OnlineAction::SendRequest => ("send_request", 15.0, 15.0),
                        OnlineAction::RemoveFriend(_) => ("remove", 13.0, 11.0),
                        OnlineAction::AcceptRequest(_) => (
                            if hover || focused {
                                "accept_highlighted"
                            } else {
                                "accept"
                            },
                            18.0,
                            18.0,
                        ),
                        OnlineAction::DeclineRequest(_) => (
                            if hover || focused {
                                "reject_highlighted"
                            } else {
                                "reject"
                            },
                            18.0,
                            18.0,
                        ),
                        OnlineAction::RevokeRequest(_) => ("cancel", 12.0, 12.0),
                        _ => continue,
                    };
                    sprite(
                        batch,
                        atlas,
                        name,
                        [
                            wd.x + ((20.0 - iw) / 2.0).floor(),
                            wd.y + ((20.0 - ih) / 2.0).floor(),
                            iw,
                            ih,
                        ],
                    );
                    if hover || focused {
                        tooltip = Some(
                            if *action == OnlineAction::SendRequest
                                && self.friend_request_name().is_empty()
                            {
                                tr("gui.friends.empty_nickname")
                            } else {
                                label.clone()
                            },
                        );
                    }
                }
                _ => {}
            }
            if wd.list {
                batch.clip_since(start, left + 8.0, top, left + 212.0, bottom);
            }
        }
        ui::scrollbar(
            batch,
            atlas,
            left + 214.0,
            top,
            bottom,
            content,
            self.scroll,
        );
        if let Some(tip) = tooltip {
            ui::tooltip(batch, atlas, &atlas.font.split(&tip, 170.0), cursor);
        }
    }
}
