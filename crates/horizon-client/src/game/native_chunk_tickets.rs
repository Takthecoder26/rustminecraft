//! Chunks a native mod keeps loaded (after NeoForge's `TicketController`): a train
//! running where nobody is, a long contraption, a machine that must keep
//! working (docs/native-mods.md, "Chunk tickets"). A held chunk is as a
//! `/forceload`ed one, its level stays open, but `/forceload` neither lists
//! nor releases it and it isn't saved.
use super::Game;

/// `/forceload`'s limit of 30,000,000 blocks each way
/// (`argument.pos.outofworld`), in chunks.
const WORLD_EDGE_CHUNKS: i32 = 30_000_000 >> 4;

fn chunk_in_world(chunk: [i32; 2]) -> bool {
    chunk
        .iter()
        .all(|c| (-WORLD_EDGE_CHUNKS..WORLD_EDGE_CHUNKS).contains(c))
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Holds chunk `chunk` (`[x, z]`, block coordinates `>> 4`) of the level
    /// in place loaded and ticking for `owner`, or, with `hold` false, lets
    /// go of it. `owner` names what holds it, under the mod's namespace
    /// (`create:train/4f0c…`, `create:contraption/12,64,-3`): the chunk stays
    /// while any owner holds it, as with NeoForge's
    /// `TicketController.forceChunk`. A held chunk is loaded with the ring around it, its
    /// blocks and entities tick as a `/forceload`ed one's do, and its level
    /// stays open with nobody there. Tickets are kept until the owner lets go
    /// or the server stops, not saved: keep what you hold (a level
    /// attachment) and hold it again in `native_level_load`, which comes only
    /// when that level opens. True when the set of owners changed; false for
    /// an owner without a namespace, for a chunk past 30,000,000 blocks and
    /// off a server. API @API:chunk_tickets@.
    pub(super) fn native_force_chunk(&mut self, owner: &str, chunk: [i32; 2], hold: bool) -> bool {
        let namespaced = owner
            .split_once(':')
            .is_some_and(|(ns, path)| !ns.is_empty() && !path.is_empty());
        if !namespaced || !chunk_in_world(chunk) || !self.native_authoritative() {
            return false;
        }
        let key = (chunk[0], chunk[1]);
        let level = self.mod_chunk_tickets.entry(self.dim).or_default();
        let changed = if hold {
            level.entry(key).or_default().insert(owner.to_owned())
        } else {
            let removed = level.get_mut(&key).is_some_and(|o| o.remove(owner));
            if level.get(&key).is_some_and(|o| o.is_empty()) {
                level.remove(&key);
            }
            removed
        };
        if !changed {
            return false;
        }
        if level.contains_key(&key) {
            if self.streaming.mod_forced.insert(key) {
                self.streaming.rank_forced(key.0, key.1, key.0, key.1);
            }
        } else {
            self.streaming.mod_forced.remove(&key);
        }
        true
    }

    /// The chunks of the level in place `owner` holds (`native_force_chunk`),
    /// sorted. API @API:chunk_tickets@.
    pub(super) fn native_forced_chunks(&self, owner: &str) -> Vec<[i32; 2]> {
        let mut held: Vec<[i32; 2]> = self
            .mod_chunk_tickets
            .get(&self.dim)
            .into_iter()
            .flatten()
            .filter(|(_, owners)| owners.contains(owner))
            .map(|(&(x, z), _)| [x, z])
            .collect();
        held.sort_unstable();
        held
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Audio, Game, Options, Progress, Settings, integrated::server_thread_options, load_resources,
    };
    use super::chunk_in_world;

    /// Block 29,999,999 is the last `/forceload` takes and -30,000,000 the
    /// first, in chunks 1,874,999 and -1,875,000.
    #[test]
    fn chunks_stop_where_forceload_stops() {
        for inside in [[1_874_999, 0], [0, 1_874_999], [-1_875_000, -1_875_000]] {
            assert!(chunk_in_world(inside), "{inside:?}");
        }
        for outside in [
            [1_875_000, 0],
            [0, 1_875_000],
            [-1_875_001, 0],
            [0, -1_875_001],
            [i32::MAX, 0],
            [0, i32::MIN],
        ] {
            assert!(!chunk_in_world(outside), "{outside:?}");
        }
    }

    /// `native_level_load` on an integrated server's first level runs inside
    /// `integrated_open`, before `serving` is set: the options that Game has
    /// by then are what let a mod hold its chunks again and read the level's
    /// attachments. A window's Game, with the options it started from, can't.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_server_threads_game_holds_chunks_before_serving_is_set() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES")
                .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
        );
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
            &mut |_, _| unreachable!("no menu handoff"),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        let level = horizon_client::mod_api::Holder::Level;

        // The window.
        assert!(!game.native_force_chunk("mymod:a", [3, -4], true));
        assert!(game.streaming.mod_forced.is_empty());
        assert!(game.attachment_map_quiet(level).is_err());

        // The server thread, `integrated_open` not yet past `serving`.
        game.options = server_thread_options(&game.options);
        assert!(game.serving.is_none());
        assert!(game.native_force_chunk("mymod:a", [3, -4], true));
        assert!(game.streaming.mod_forced.contains(&(3, -4)));
        assert_eq!(game.native_forced_chunks("mymod:a"), [[3, -4]]);
        assert!(game.attachment_map_quiet(level).is_ok());

        // Past the world's edge, where the ring around it would overflow.
        assert!(!game.native_force_chunk("mymod:a", [i32::MAX, 0], true));
        assert!(!game.native_force_chunk("mymod:a", [1_875_000, 0], true));
        assert_eq!(game.native_forced_chunks("mymod:a"), [[3, -4]]);
        assert_eq!(game.streaming.mod_forced.len(), 1);
    }
}
