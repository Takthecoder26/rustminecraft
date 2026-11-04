//! What a mod's machine does to the world as a player's hand would, with no
//! player: break a block with a tool and keep what drops (a drill, a saw, a
//! harvester), show its crack while it works, and use an item as a player
//! would (a deployer) (docs/native-mods.md, "Machines").
use super::{Game, GuestUse, Hit, InteractTarget, Screen, spill::MachineBreak};
use horizon_client::{
    command::args::GameMode,
    first_person::Hand,
    items::ItemStack,
    mod_api::{NativeFakeUse, NativeUseTarget},
};
use horizon_core::{geometry::Vec3, position::BlockPos};
use winit::keyboard::KeyCode;

/// `Player.getStandingEyeHeight`: a fake player's eyes above its feet.
const FAKE_EYE: f32 = 1.62;

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Breaks the block at `pos` as a machine does, with no player: vanilla's
    /// `Level.destroyBlock(pos, true)` with the loot rolled for `tool`
    /// (`Block.dropResources(state, level, pos, blockEntity, null, tool)`):
    /// the break particles and sound, the block's drops with what its block
    /// entity copies into them (a shulker box's contents), experience for an
    /// ore unless `tool` has silk touch, then the block's fluid in its place.
    /// `tool` picks the drops as a player's held item does (silk touch,
    /// fortune, shears); `None` is an empty hand. The mods' say on drops
    /// (`native_loot_drops`) applies. With `collect`, the drops are returned
    /// and nothing is popped (a harvester storing them); without, they pop as
    /// item entities and the list is empty. What pops off because of the break
    /// (a torch on the block) isn't the machine's: it pops as with no tool and
    /// is never in the list. Nothing drops where `doTileDrops` is off. None,
    /// with nothing broken, for air, an unloaded block and off a server.
    /// API @API:machine_breaking@.
    pub(super) fn native_machine_destroy_block(
        &mut self,
        pos: [i32; 3],
        tool: Option<&ItemStack>,
        collect: bool,
    ) -> Option<Vec<ItemStack>> {
        if !self.native_authoritative() || !self.world.loaded(pos[0], pos[2]) {
            return None;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.behaviours().is_air(state) {
            return None;
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        let machine = MachineBreak {
            pos: at,
            tool: tool.cloned(),
            kept: Vec::new(),
        };
        let mut drops = self.with_level(|b, level, ticks| {
            level.machine_break = Some(machine);
            b.destroy_block(level, ticks, at, true);
            level.machine_break.take().map_or_else(Vec::new, |m| m.kept)
        });
        let _ = self.process_level_events();
        self.native_block_drops(pos, state, None, tool, None, &mut drops);
        if collect {
            return Some(drops);
        }
        for stack in drops {
            self.pop_resource(pos, stack);
        }
        Some(Vec::new())
    }

    /// `ServerLevel.destroyBlockProgress`: shows breaker `breaker`'s crack on
    /// the block at `pos` at stage `progress` (0 to 9) to the players within 64
    /// blocks, as a zombie breaking a door shows its; any other stage takes the
    /// crack away. A machine passes an id of its own that no entity has (a
    /// negative one), the same each time, so a new stage replaces its old one;
    /// a crack not renewed for 400 ticks fades. Off a server nothing happens.
    /// API @API:machine_breaking@.
    pub(super) fn native_set_break_progress(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        if !self.native_authoritative() {
            return;
        }
        self.server_destroy_block_progress(breaker, pos, progress);
    }

    /// Uses `stack` as a player standing with its eyes at `eye`, looking
    /// `yaw` and `pitch` degrees (as `Entity.getYRot` and `getXRot`), would:
    /// NeoForge's `FakePlayer` doing what a deployer does. On a block
    /// (`NativeUseTarget::Block`) it is `ServerPlayerGameMode.useItemOn`:
    /// the block reacts first (a lever flips, a cauldron fills), else the
    /// item is used on it (a block placed, bone meal spread, a bucket
    /// filled); in the air (`Item`) `useItem` (a bucket picks up the fluid it
    /// looks at); on an entity (`Entity`) `Player.interactOn` (a sheep
    /// sheared, a cow milked). With `sneaking` it is a secondary use, as a
    /// player holding sneak's. The fake player is in survival, holds `stack`
    /// in its main hand and nothing else, and has no connection: no screen
    /// it opens stays open, and its stats and advancements count for no
    /// one. The answer is what it holds and carries afterwards. None, with
    /// nothing used, off a server and while a player's action is being run
    /// (from inside a player's hook: run it from a block entity's tick).
    /// API @API:fake_player@.
    pub(super) fn native_use_as_player(
        &mut self,
        stack: ItemStack,
        eye: [f64; 3],
        yaw: f32,
        pitch: f32,
        target: NativeUseTarget,
        sneaking: bool,
    ) -> Option<NativeFakeUse> {
        if !self.native_authoritative()
            || self.remote.is_some()
            || self.player.is_some()
            || self.lan_acting.is_some()
            || !eye.iter().all(|v| v.is_finite())
            || !(yaw.is_finite() && pitch.is_finite())
        {
            return None;
        }
        let feet = Vec3::new(eye[0], eye[1] - f64::from(FAKE_EYE), eye[2]);
        let body = horizon_client::motion::PlayerMotion::new(self.motion, feet).ok()?;
        let slots = self.inventory.len().max(36);
        let mut inventory = vec![None; slots];
        inventory[0] = (stack.count > 0).then_some(stack);
        let keys = if sneaking {
            [KeyCode::ShiftLeft].into_iter().collect()
        } else {
            Default::default()
        };
        let (forced_hit, guest_use) = match target {
            NativeUseTarget::Block { pos, face, point } => (
                Some(Hit {
                    pos,
                    face: face.normal(),
                    point,
                }),
                GuestUse::OnBlock(Hand::Main),
            ),
            NativeUseTarget::Item => (None, GuestUse::Item(Hand::Main)),
            NativeUseTarget::Entity { id, location } => (
                None,
                GuestUse::Entity(Hand::Main, InteractTarget { id, location }),
            ),
        };
        // The server's own player state, empty while nobody acts, kept to put
        // back: the fake player leaves nothing of itself behind.
        let player = self.player.replace(body);
        let eye_height = std::mem::replace(&mut self.eye, FAKE_EYE);
        let look = (
            std::mem::replace(&mut self.yaw, yaw),
            std::mem::replace(&mut self.pitch, pitch),
        );
        let inventory = std::mem::replace(&mut self.inventory, inventory);
        let selected = std::mem::replace(&mut self.selected, 0);
        let armor = std::mem::take(&mut self.armor);
        let offhand = self.offhand.take();
        let carried = self.carried.take();
        let keys = std::mem::replace(&mut self.keys, keys);
        let screen = std::mem::replace(&mut self.screen, Screen::Playing);
        let container = self.container.take();
        let sign_edit = self.sign_edit.take();
        let book = self.book.take();
        let command_block = self.command_block_screen.take();
        let mount_menu = std::mem::take(&mut self.mount_menu);
        let craft_size = std::mem::replace(&mut self.craft_size, 2);
        let craft_grid = std::mem::replace(&mut self.craft_grid, vec![None; 9]);
        let mode = std::mem::replace(&mut self.mode, GameMode::Survival);
        let creative = std::mem::replace(&mut self.creative, false);
        let mining = self.mining.take();
        let using = self.using.take();
        let use_delay = std::mem::replace(&mut self.use_delay, 0);
        let cooldowns = std::mem::take(&mut self.cooldowns);
        let riding = self.riding.take();
        let sleeping = self.sleeping.take();
        let reach = (self.block_reach, self.entity_reach);
        let hit = std::mem::replace(&mut self.forced_hit, forced_hit);
        let tick_hit = self.tick_hit.take();
        let used_as = self.guest_use.replace(guest_use);
        let failed = std::mem::replace(&mut self.use_failed, false);
        self.block_reach = self.interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
        self.entity_reach = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);

        let consumed = matches!(self.use_item(), Ok(Some(_))) && !self.use_failed;

        let mut left = std::mem::replace(&mut self.inventory, inventory);
        let held = left
            .get_mut(0)
            .and_then(Option::take)
            .filter(|s| s.count > 0);
        let other = left
            .into_iter()
            .chain([self.offhand.take(), self.carried.take()])
            .chain(std::mem::take(&mut self.armor))
            .flatten()
            .filter(|s| s.count > 0)
            .collect();
        self.player = player;
        self.eye = eye_height;
        (self.yaw, self.pitch) = look;
        self.selected = selected;
        self.armor = armor;
        self.offhand = offhand;
        self.carried = carried;
        self.keys = keys;
        self.screen = screen;
        self.container = container;
        self.sign_edit = sign_edit;
        self.book = book;
        self.command_block_screen = command_block;
        self.mount_menu = mount_menu;
        self.craft_size = craft_size;
        self.craft_grid = craft_grid;
        self.mode = mode;
        self.creative = creative;
        self.mining = mining;
        self.using = using;
        self.use_delay = use_delay;
        self.cooldowns = cooldowns;
        self.riding = riding;
        self.sleeping = sleeping;
        (self.block_reach, self.entity_reach) = reach;
        self.forced_hit = hit;
        self.tick_hit = tick_hit;
        self.guest_use = used_as;
        self.use_failed = failed;
        Some(NativeFakeUse {
            consumed,
            held,
            other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::chunk::Chunk;
    use std::sync::Arc;

    const TARGET: [i32; 3] = [8, 64, 8];
    const ABOVE: [i32; 3] = [8, 65, 8];

    /// A dedicated server's game on the prepared resources, so machines work.
    fn server_game() -> Game<'static> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let res: &'static _ = Box::leak(Box::new(loaded.resources));
        let mut game = Game::new(
            res,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        game.options.dedicated = Some(Vec::new());
        game
    }

    /// A clear chunk holding `blocks`, and no items or orbs lying about.
    fn set_up(game: &mut Game<'_>, blocks: &[([i32; 3], &str)]) {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.dropped.clear();
        game.orbs.clear();
        for &(at, name) in blocks {
            let state = game.pack.assets.registry.default_state(name).unwrap();
            game.world.set(at[0], at[1], at[2], state).unwrap();
        }
    }

    fn items(stacks: impl IntoIterator<Item = ItemStack>) -> Vec<String> {
        stacks.into_iter().map(|s| s.item.to_string()).collect()
    }

    fn popped(game: &Game<'_>) -> Vec<String> {
        items(game.dropped.iter().map(|e| e.stack.clone()))
    }

    fn experience(game: &Game<'_>) -> u32 {
        game.orbs.iter().map(|o| o.value * o.count).sum()
    }

    fn pick_with(enchantment: &str) -> ItemStack {
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pick.enchant(enchantment, 1);
        pick
    }

    /// Dirt under a torch or short grass: the machine keeps the dirt, and the
    /// neighbour pops off on its own, as if no tool had been near it. With
    /// shears, short grass drops itself; with none, wheat seeds at most.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn neighbours_that_pop_off_are_not_the_machines() {
        let mut game = server_game();
        let shears = ItemStack::new("minecraft:shears", 1);

        set_up(
            &mut game,
            &[(TARGET, "minecraft:dirt"), (ABOVE, "minecraft:torch")],
        );
        let kept = game
            .native_machine_destroy_block(TARGET, Some(&shears), true)
            .unwrap();
        assert_eq!(items(kept), ["minecraft:dirt"]);
        assert_eq!(popped(&game), ["minecraft:torch"]);
        let air = game.res.facts().air;
        assert_eq!(game.world.block(ABOVE[0], ABOVE[1], ABOVE[2]), air);

        set_up(
            &mut game,
            &[(TARGET, "minecraft:dirt"), (ABOVE, "minecraft:short_grass")],
        );
        let kept = game
            .native_machine_destroy_block(TARGET, Some(&shears), true)
            .unwrap();
        assert_eq!(items(kept), ["minecraft:dirt"]);
        assert_eq!(game.world.block(ABOVE[0], ABOVE[1], ABOVE[2]), air);
        let grass = popped(&game);
        assert!(grass.len() <= 1, "{grass:?}");
        assert!(
            grass.iter().all(|i| i == "minecraft:wheat_seeds"),
            "{grass:?}"
        );

        // The tool still counts on the block it was aimed at.
        set_up(&mut game, &[(TARGET, "minecraft:short_grass")]);
        let kept = game
            .native_machine_destroy_block(TARGET, Some(&shears), true)
            .unwrap();
        assert_eq!(items(kept), ["minecraft:short_grass"]);
        assert!(popped(&game).is_empty());
    }

    /// `DropExperienceBlock` for a diamond ore is `UniformInt.of(3, 7)` and
    /// `SpawnerBlock` is 15 + nextInt(15) + nextInt(15). Silk touch on the ore
    /// gives the ore itself and no experience.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn silk_touch_gives_a_machine_no_ore_experience() {
        let mut game = server_game();
        let ore = "minecraft:diamond_ore";

        set_up(&mut game, &[(TARGET, ore)]);
        let silk = pick_with("minecraft:silk_touch");
        let kept = game
            .native_machine_destroy_block(TARGET, Some(&silk), true)
            .unwrap();
        assert_eq!(items(kept), [ore]);
        assert_eq!(experience(&game), 0);

        let fortune = pick_with("minecraft:fortune");
        for tool in [Some(&fortune), None] {
            set_up(&mut game, &[(TARGET, ore)]);
            let kept = game
                .native_machine_destroy_block(TARGET, tool, true)
                .unwrap();
            let kept = items(kept);
            assert!(!kept.is_empty() && kept.iter().all(|i| i == "minecraft:diamond"));
            let xp = experience(&game);
            assert!(
                (3..=7).contains(&xp),
                "{xp} with fortune: {}",
                tool.is_some()
            );
        }

        set_up(&mut game, &[(TARGET, "minecraft:spawner")]);
        game.native_machine_destroy_block(TARGET, Some(&silk), true)
            .unwrap();
        assert!((15..=43).contains(&experience(&game)));
    }
}
