//! Pictures baked per item at one window scale, for the Wardrobe and its Shop. Each is asked for
//! once per scale, including one that turned out to have nothing to show.
use std::collections::HashMap;

enum Slot<T> {
    /// On its way.
    Waiting,
    /// Nothing to show.
    Nothing,
    Ready(T),
}

pub(super) struct Bakes<T> {
    scale: f32,
    slots: HashMap<String, Slot<T>>,
    /// Pictures of another scale, drawn before and not drawn again.
    replaced: Vec<T>,
}

impl<T> Default for Bakes<T> {
    fn default() -> Self {
        Self {
            scale: 0.,
            slots: HashMap::new(),
            replaced: Vec::new(),
        }
    }
}

impl<T> Bakes<T> {
    /// Whether `id` is yet to be baked at `scale`. Says yes once: the caller then owes `finish`.
    pub(super) fn start(&mut self, id: &str, scale: f32) -> bool {
        if self.scale != scale {
            self.scale = scale;
            self.replaced
                .extend(self.slots.drain().filter_map(|(_, slot)| match slot {
                    Slot::Ready(picture) => Some(picture),
                    _ => None,
                }));
        }
        if self.slots.contains_key(id) {
            return false;
        }
        self.slots.insert(id.to_owned(), Slot::Waiting);
        true
    }

    /// A bake started at `scale` came back. One the window has moved on from was never drawn, so
    /// it just goes.
    pub(super) fn finish(&mut self, id: &str, scale: f32, picture: Option<T>) {
        if scale != self.scale {
            return;
        }
        if let Some(slot) = self.slots.get_mut(id) {
            if matches!(slot, Slot::Waiting) {
                *slot = picture.map_or(Slot::Nothing, Slot::Ready);
            }
        }
    }

    /// `id`'s picture, if it has one yet.
    pub(super) fn get(&self, id: &str) -> Option<&T> {
        match self.slots.get(id) {
            Some(Slot::Ready(picture)) => Some(picture),
            _ => None,
        }
    }

    /// `id`'s picture at `scale`, baked here the first time.
    pub(super) fn get_or_bake(
        &mut self,
        id: &str,
        scale: f32,
        bake: impl FnOnce() -> T,
    ) -> Option<&T> {
        if self.start(id, scale) {
            self.finish(id, scale, Some(bake()));
        }
        self.get(id)
    }

    /// The pictures replaced since the last call.
    pub(super) fn drain_replaced(&mut self) -> std::vec::Drain<'_, T> {
        self.replaced.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_with_nothing_to_show_is_not_baked_again() {
        let mut bakes = Bakes::<u32>::default();
        assert!(bakes.start("visor", 1.));
        bakes.finish("visor", 1., None);
        assert!(!bakes.start("visor", 1.));
        assert_eq!(bakes.get("visor"), None);
    }

    #[test]
    fn redraws_while_a_picture_is_on_its_way_do_not_bake_it_twice() {
        let mut bakes = Bakes::<u32>::default();
        assert!(bakes.start("visor", 1.));
        assert!(!bakes.start("visor", 1.));
        assert_eq!(bakes.get("visor"), None);
        bakes.finish("visor", 1., Some(7));
        assert_eq!(bakes.get("visor"), Some(&7));
    }

    #[test]
    fn a_new_scale_bakes_again_and_hands_over_the_old_picture_once() {
        let mut bakes = Bakes::<u32>::default();
        bakes.start("visor", 1.);
        bakes.finish("visor", 1., Some(7));
        assert!(bakes.start("visor", 2.));
        assert_eq!(bakes.get("visor"), None);
        assert_eq!(bakes.drain_replaced().collect::<Vec<_>>(), [7]);
        assert_eq!(bakes.drain_replaced().count(), 0);
    }

    #[test]
    fn a_picture_for_a_scale_the_window_left_is_never_shown() {
        let mut bakes = Bakes::<u32>::default();
        bakes.start("visor", 1.);
        bakes.start("visor", 2.);
        bakes.finish("visor", 1., Some(7));
        assert_eq!(bakes.get("visor"), None);
        bakes.finish("visor", 2., Some(8));
        assert_eq!(bakes.get("visor"), Some(&8));
        // It never reached the atlas, so there is nothing to free.
        assert_eq!(bakes.drain_replaced().count(), 0);
    }

    #[test]
    fn a_mark_is_kept_for_its_scale_and_baked_again_on_another() {
        let mut bakes = Bakes::<u32>::default();
        let mut made = 0;
        let mut mark = |scale: f32| {
            *bakes
                .get_or_bake("tag", scale, || {
                    made += 1;
                    made
                })
                .unwrap()
        };
        assert_eq!(mark(1.), 1);
        assert_eq!(mark(1.), 1);
        assert_eq!(mark(2.), 2);
        assert_eq!(mark(2.), 2);
    }
}
