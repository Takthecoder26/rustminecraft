//! Optional block-state render-layer overrides from `block.properties` (spec 4.2).
use crate::{BlockIds, Properties};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockRenderLayer {
    Solid,
    Cutout,
    Translucent,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockLayers {
    rules: BlockIds,
}

impl BlockLayers {
    pub fn parse(properties: &Properties) -> Self {
        let entries = properties
            .entries
            .iter()
            .filter_map(|entry| {
                let id = match entry.key.as_str() {
                    "layer.solid" => 0,
                    "layer.cutout" | "layer.cutout_mipped" => 1,
                    "layer.translucent" => 2,
                    _ => return None,
                };
                Some(crate::Entry {
                    key: format!("block.{id}"),
                    ..entry.clone()
                })
            })
            .collect();
        Self {
            rules: BlockIds::parse(&Properties {
                entries,
                notes: Vec::new(),
            }),
        }
    }

    pub fn resolve_tags(&self, tags: &HashMap<String, HashSet<String>>) -> Self {
        Self {
            rules: self.rules.resolve_tags(tags),
        }
    }

    /// An unmatched state keeps its model/sprite default. Explicit entries beat tags;
    /// matching state filters then file order break ties, as for block IDs.
    pub fn layer<'a>(
        &self,
        name: &str,
        properties: impl Fn(&str) -> Option<&'a str>,
    ) -> Option<BlockRenderLayer> {
        match self.rules.id(name, properties)? {
            0 => Some(BlockRenderLayer::Solid),
            1 => Some(BlockRenderLayer::Cutout),
            2 => Some(BlockRenderLayer::Translucent),
            _ => None,
        }
    }

    pub fn uses_tags(&self) -> bool {
        self.rules.uses_tags()
    }
    pub fn notes(&self) -> &[String] {
        &self.rules.notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    #[test]
    fn layers_match_states_tags_alias_and_explicit_priority() {
        let rules = BlockLayers::parse(&Properties::parse(
            "layer.translucent=%test:wood\nlayer.cutout=oak_log:axis=x\nlayer.cutout_mipped=oak_log:axis=z\nlayer.solid=glass\nblock.42=stone\nlayer.unknown=dirt",
            &Macros::standard(),
        )).resolve_tags(&HashMap::from([("test:wood".into(), HashSet::from(["minecraft:oak_log".into()]))]));
        assert_eq!(
            rules.layer("minecraft:oak_log", |_| Some("x")),
            Some(BlockRenderLayer::Cutout)
        );
        assert_eq!(
            rules.layer("minecraft:oak_log", |_| Some("z")),
            Some(BlockRenderLayer::Cutout)
        );
        assert_eq!(
            rules.layer("minecraft:oak_log", |_| Some("y")),
            Some(BlockRenderLayer::Translucent)
        );
        assert_eq!(
            rules.layer("minecraft:glass", |_| None),
            Some(BlockRenderLayer::Solid)
        );
        assert_eq!(rules.layer("minecraft:stone", |_| None), None);
        assert_eq!(rules.layer("minecraft:dirt", |_| None), None);
        assert!(rules.uses_tags());
        assert!(rules.notes().is_empty());
    }
}
