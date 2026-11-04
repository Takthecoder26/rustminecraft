//! Shared Realms thumbnails and online profile names for both menu owners.
use horizon_client::{
    font::Patch,
    hud::{HudAtlas, HudBatch, MISSING_SPRITE, PACK_ICON_SIZE, PACK_ICON_SLOTS},
    realms_images::{Cache, Status},
    skins::{NameOrId, Resolve, Resolver},
    title::{
        Title,
        realms::{Page, State},
    },
};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct RealmsUi {
    images: Cache,
    aliases: Vec<String>,
    account: Option<String>,
    resolving: Vec<u128>,
    failed: HashSet<u128>,
}

impl RealmsUi {
    /// Run before drawing a title, or with `None` when another screen owns the
    /// atlas. Rebind every frame: pack screens and resource reloads reuse slots.
    pub(super) fn prepare(
        &mut self,
        title: Option<&mut Title>,
        atlas: &mut HudAtlas,
        batch: &mut HudBatch,
        profiles: &mut Resolver,
    ) {
        for location in self.aliases.drain(..) {
            atlas.sprites.remove(&location);
        }
        let Some(state) = title.and_then(Title::realms_mut) else {
            return;
        };
        if self.account != state.profile_uuid {
            self.account = state.profile_uuid.clone();
            self.images.clear();
            self.failed.clear();
        }
        self.names(state, profiles);
        self.images.poll();
        let configuration_images = if matches!(state.page, Page::Configure | Page::Slots) {
            state.configuration_template_images()
        } else {
            Vec::new()
        };
        let images = configuration_images
            .into_iter()
            .chain(state.reset_template_images());
        let mut seen = HashSet::new();
        let mut slot = 0;
        for (id, image) in images {
            if !seen.insert(id.clone()) {
                continue;
            }
            if slot >= PACK_ICON_SLOTS {
                break;
            }
            let location = format!("realms/dynamic/{id}");
            let rect = match self.images.request(&id, &image) {
                Status::Ready => {
                    let Some(rect) = atlas
                        .sprites
                        .get(&format!("horizon/pack-icon{slot}"))
                        .copied()
                    else {
                        continue;
                    };
                    let Some(rgba) = self.images.image(&location) else {
                        continue;
                    };
                    batch.patches.push(Patch {
                        x: (rect.u0 * atlas.width as f32).round() as u32,
                        y: (rect.v0 * atlas.height as f32).round() as u32,
                        width: PACK_ICON_SIZE,
                        height: PACK_ICON_SIZE,
                        rgba: rgba.to_vec(),
                    });
                    slot += 1;
                    Some(rect)
                }
                Status::Failed => atlas.sprites.get(MISSING_SPRITE).copied(),
                Status::Pending => atlas.sprites.get("gui/presets/isles").copied(),
            };
            if let Some(rect) = rect {
                atlas.sprites.insert(location.clone(), rect);
                self.aliases.push(location);
            }
        }
    }

    fn names(&mut self, state: &mut State, profiles: &mut Resolver) {
        // RealmsServerPlayerLists encodes each realm's player array as JSON in
        // a string. Resolve only bounded, valid UUIDs, never names from a URL.
        let mut wanted = Vec::new();
        for list in state
            .online_players
            .get("lists")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(players) = list
                .get("playerList")
                .and_then(serde_json::Value::as_str)
                .filter(|text| text.len() <= 64 * 1024)
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            else {
                continue;
            };
            for player in players.as_array().into_iter().flatten() {
                if let Some(id) = player
                    .get("playerId")
                    .and_then(serde_json::Value::as_str)
                    .and_then(horizon_client::auth::parse_uuid)
                    && wanted.len() < 128
                    && !wanted.contains(&id)
                {
                    wanted.push(id);
                }
            }
        }
        let local = state
            .profile_uuid
            .as_deref()
            .and_then(horizon_client::auth::parse_uuid);
        wanted.retain(|id| Some(*id) != local);
        state.online_player_names.retain(|id, _| {
            u128::from_str_radix(id, 16)
                .ok()
                .is_some_and(|id| wanted.contains(&id))
        });
        self.failed.retain(|id| wanted.contains(id));
        self.resolving
            .retain(|id| match profiles.state(&NameOrId::Id(*id)) {
                Resolve::Pending => true,
                Resolve::Done(profile) => {
                    if wanted.contains(id) {
                        if let Some(profile) =
                            profile.filter(|profile| profile.id == *id && !profile.name.is_empty())
                        {
                            state
                                .online_player_names
                                .insert(format!("{id:032x}"), profile.name);
                        } else {
                            self.failed.insert(*id);
                        }
                    }
                    false
                }
            });
        for id in wanted {
            if self.resolving.len() >= 2 {
                break;
            }
            if self.resolving.contains(&id)
                || self.failed.contains(&id)
                || state
                    .online_player_names
                    .contains_key(&format!("{id:032x}"))
            {
                continue;
            }
            match profiles.state(&NameOrId::Id(id)) {
                Resolve::Pending => self.resolving.push(id),
                Resolve::Done(Some(profile)) if profile.id == id && !profile.name.is_empty() => {
                    state
                        .online_player_names
                        .insert(format!("{id:032x}"), profile.name);
                }
                Resolve::Done(_) => {
                    self.failed.insert(id);
                }
            }
        }
    }
}
