//! A few blocks drawn in a screen as a small scene seen from an angle: what
//! Create's Ponder shows its machines in (docs/native-mods.md, "Scenes in
//! screens"). The blocks' models and their mod block entities' drawing are
//! projected into the GUI with their own depth, as the smithing screen draws
//! its armour stand.
use super::Game;
use super::native_block_entity_draw::{BlockEntityDraw, place_turning};
use glam::{Mat4, Vec3};
use horizon_client::{
    block_entity_render::diffuse_lights,
    hud::HudBatch,
    mesher::{CULL_BACK, SHADED_CULL_W, Vertex},
    mod_api::{NativeSceneBlock, NativeSceneView},
};
use horizon_world::{level::data::state_flags, registry::BlockStateId};
use std::collections::HashMap;

/// A quad in the scene's blocks, before it is seen: its corners, UVs in the
/// atlas, ABGR colour, its outward normal, and whether it is left out when
/// seen from behind.
struct SceneQuad {
    corners: [Vec3; 4],
    uv: [[f32; 2]; 4],
    color: u32,
    normal: Vec3,
    cull: bool,
}

impl SceneQuad {
    /// Whether `m` shows this quad: a culling quad only from its front.
    fn seen(&self, m: Mat4) -> bool {
        !self.cull || m.transform_vector3(self.normal).z > 0.0
    }
}

/// The scene's quad of `q`, a quad a block entity drew, read as the entity
/// pipelines read its `normal.w` (`entity.wgsl`). Only 127 is left to shade
/// here: `native_draw_block` and `native_draw_model` draw it, and the other
/// `native_draw_*` bake their colour (0 or `CULL_BACK`). Only `CULL_BACK` and
/// `SHADED_CULL_W` lose their back faces.
fn drawn_quad(q: &[Vertex; 4]) -> SceneQuad {
    let corners = q.each_ref().map(|v| Vec3::from_array(v.pos));
    let normal = (corners[1] - corners[0])
        .cross(corners[2] - corners[0])
        .normalize_or_zero();
    // A quad drawn as a triangle has its area in the other half.
    let normal = if normal == Vec3::ZERO {
        (corners[2] - corners[1])
            .cross(corners[3] - corners[1])
            .normalize_or_zero()
    } else {
        normal
    };
    let w = q[0].normal[3];
    SceneQuad {
        corners,
        uv: q.each_ref().map(|v| v.uv),
        color: if w == 127 {
            shaded(q[0].color, face_shade(normal))
        } else {
            q[0].color
        },
        normal,
        cull: w == CULL_BACK[3] || w == SHADED_CULL_W,
    }
}

/// The states of `blocks` by position, for telling which faces are hidden,
/// without those outside the registry's `state_count` states: the model lookup
/// skips them too, and their facts would be out of bounds.
fn occupied_states(
    blocks: &[NativeSceneBlock<'_>],
    state_count: usize,
) -> HashMap<[i32; 3], BlockStateId> {
    blocks
        .iter()
        .filter(|b| (b.state.0 as usize) < state_count)
        .map(|b| (b.pos, b.state))
        .collect()
}

/// The GUI transform of `view` in the box `rect` (x0, y0, x1, y1): the view's
/// centre at the box's middle, a block `scale` pixels across. Unturned, the
/// scene is seen from the north looking south, as a player at yaw 0 sees it,
/// up the screen's up; `yaw` turns it counterclockwise seen from above, and
/// `pitch` tilts its top towards the viewer. The GUI's y runs down and its z
/// towards the viewer, so the point reflection keeps the picture unmirrored.
fn scene_matrix(rect: [f32; 4], view: NativeSceneView) -> Mat4 {
    let middle = Vec3::new((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0, 0.0);
    Mat4::from_translation(middle)
        * Mat4::from_scale(Vec3::splat(-view.scale))
        * Mat4::from_rotation_x(-view.pitch.to_radians())
        * Mat4::from_rotation_y(view.yaw.to_radians())
        * Mat4::from_translation(-Vec3::from_array(view.center))
}

/// The step a model face's direction points: the model's `Direction` lists
/// its sides in the order the game's does.
fn step(direction: horizon_assets::model::Direction) -> [i32; 3] {
    horizon_core::geometry::Direction::ALL[direction as usize].normal()
}

/// How bright a face is with `normal` (world space, unit), as blocks are
/// shaded: 1 for the top, 0.5 underneath, 0.8 north and south, 0.6 east and
/// west, in between for a slanted face.
fn face_shade(normal: Vec3) -> f32 {
    let up = if normal.y > 0.0 { 1.0 } else { 0.5 };
    (normal.x * normal.x * 0.6 + normal.y * normal.y * up + normal.z * normal.z * 0.8).min(1.0)
}

/// `color` (ABGR) with its red, green and blue times `shade`.
fn shaded(color: u32, shade: f32) -> u32 {
    let ch = |s: u32| (((color >> s & 255) as f32 * shade) as u32).min(255) << s;
    (color & 0xFF00_0000) | ch(0) | ch(8) | ch(16)
}

#[allow(dead_code)]
impl Game<'_> {
    /// Draws `blocks`, a scene of block states at positions of its own (not
    /// the world's), into the box `rect` (x0, y0, x1, y1, GUI pixels) of a
    /// screen's `batch`, seen as `view` says: Create's Ponder scenes. Each
    /// block is drawn with its model, full bright and shaded by face, a face
    /// against a solid neighbour in the scene left out; a block with an
    /// `entity` (a mod's) is drawn by `native_block_entity_submit` as in the
    /// world too, its `spin` parts turning with the game time. What lies
    /// outside the box is cut off, and the scene has a depth of its own, so
    /// it never covers the screen's other drawing or is covered by it but in
    /// the order drawn. Draw it from a screen's drawing hook
    /// (`native_screen_draw`). A state the registry doesn't know is left out.
    /// A client's call; nothing is drawn on a server.
    /// API @API:scenes_in_screens@.
    pub(super) fn native_draw_scene(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        blocks: &[NativeSceneBlock<'_>],
        view: NativeSceneView,
    ) {
        if !self.native_window() || !view.scale.is_finite() || view.scale <= 0.0 {
            return;
        }
        let occupied = occupied_states(blocks, self.res.behaviours().table.state_count());
        let solid = |pos: [i32; 3]| {
            occupied.get(&pos).is_some_and(|&state| {
                self.res.behaviours().facts(state).flags & state_flags::SOLID_RENDER != 0
            })
        };
        let grass = self.pack.item_renderer.grass_color(0.5, 1.0) as u32;
        let tables = &self.pack.context.tables;
        let mut quads = Vec::new();
        for block in blocks {
            let at = Vec3::new(
                block.pos[0] as f32,
                block.pos[1] as f32,
                block.pos[2] as f32,
            );
            let Ok(models) = self
                .pack
                .render
                .model_catalog
                .collect_seeded(block.state, 42)
            else {
                continue;
            };
            for face in models.iter().flat_map(|(_, model)| &model.faces) {
                if let Some(cull) = face.quad.cull {
                    let [dx, dy, dz] = step(cull);
                    if solid([block.pos[0] + dx, block.pos[1] + dy, block.pos[2] + dz]) {
                        continue;
                    }
                }
                let Some(sprite) = self
                    .pack
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.gui.atlas.sprites.get(name))
                else {
                    continue;
                };
                let [nx, ny, nz] = step(face.quad.facing);
                let normal = Vec3::new(nx as f32, ny as f32, nz as f32);
                let mut color = shaded(0xFFFF_FFFF, face_shade(normal));
                if let Some(t) = tables.default_tint(block.state, face.quad.tint_index, grass) {
                    color = horizon_client::hud_layout::multiply(
                        color,
                        horizon_client::mobs::render::abgr(t),
                    );
                }
                quads.push(SceneQuad {
                    corners: face.quad.positions.map(|p| at + Vec3::from_array(p)),
                    uv: face.quad.uv.map(|uv| {
                        [
                            sprite.u0 + uv[0] * (sprite.u1 - sprite.u0),
                            sprite.v0 + uv[1] * (sprite.v1 - sprite.v0),
                        ]
                    }),
                    color,
                    normal,
                    // As terrain is drawn: a block's own faces lose their backs.
                    cull: true,
                });
            }
            if let Some(entity) = block.entity {
                let name = &*self.res.facts().get(block.state).name;
                let mut draw = BlockEntityDraw::posed(
                    self.clock.fraction() as f32,
                    self.world.game_time,
                    240 | 240 << 8,
                    block.pos,
                    Mat4::IDENTITY,
                    diffuse_lights(false),
                    Vec::new(),
                );
                self.native_block_entity_submit(block.pos, name, block.state, entity, &mut draw);
                let (drawn, spins) = draw.into_turning();
                let mut placed: Vec<Vertex> = Vec::with_capacity(drawn.len());
                place_turning(
                    &drawn,
                    &spins,
                    self.world.game_time,
                    self.clock.fraction() as f32,
                    at,
                    &mut placed,
                );
                quads.extend(placed.as_chunks::<4>().0.iter().map(drawn_quad));
            }
        }
        let m = scene_matrix(rect, view);
        let mut seen: Vec<(f32, [[f32; 3]; 4], [[f32; 2]; 4], u32)> = quads
            .into_iter()
            .filter(|q| q.seen(m))
            .map(|q| {
                let p = q.corners.map(|c| m.transform_point3(c));
                let depth = p.iter().map(|v| v.z).sum::<f32>() / 4.0;
                (depth, p.map(|v| v.to_array()), q.uv, q.color)
            })
            .collect();
        // Far to near, for the faces that let others show through.
        seen.sort_by(|a, b| a.0.total_cmp(&b.0));
        let start = batch.vertices.len();
        for (_, corners, uv, color) in seen {
            batch.quad_3d(corners, uv, color);
        }
        batch.depth_since(start, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(yaw: f32, pitch: f32) -> NativeSceneView {
        NativeSceneView::new([0.5, 0.5, 0.5], yaw, pitch, 20.0)
    }

    /// A quad of `corners` with one colour and `normal`.
    fn drawn(corners: [[f32; 3]; 4], color: u32, normal: [i8; 4]) -> [Vertex; 4] {
        corners.map(|pos| Vertex {
            pos,
            uv: [0.0, 0.0],
            layer: 0,
            color,
            light: 0,
            normal,
            glint_uv: [0.0, 0.0],
        })
    }

    /// Wound to face east, as `native_draw_quad` asks.
    const EAST: [[f32; 3]; 4] = [
        [1.0, 1.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
    ];
    /// Wound to face south: its back is what an unturned scene sees.
    const SOUTH: [[f32; 3]; 4] = [
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
    ];

    /// An east face comes out at 0.6 of white, 153, drawn either way: already
    /// baked to it by `native_draw_quad`, or left to shade by
    /// `native_draw_block`. Shaded twice it would be about 0.36.
    #[test]
    fn a_face_is_shaded_once() {
        for baked in [[0; 4], CULL_BACK] {
            let q = drawn_quad(&drawn(EAST, 0xFF99_9999, baked));
            assert_eq!(q.color, 0xFF99_9999, "baked with {baked:?}");
        }
        let q = drawn_quad(&drawn(EAST, 0xFFFF_FFFF, [127, 0, 0, 127]));
        assert_eq!(q.color, 0xFF99_9999);
    }

    /// As in the world: a flat quad or a block drawn by an entity shows from
    /// behind, and only the culling kinds lose their back faces.
    #[test]
    fn only_culling_quads_are_hidden_from_behind() {
        let m = scene_matrix([0.0, 0.0, 100.0, 100.0], view(0.0, 0.0));
        let from_behind = |normal: [i8; 4]| drawn_quad(&drawn(SOUTH, !0, normal)).seen(m);
        assert!(from_behind([0; 4]), "a baked flat quad");
        assert!(from_behind([0, 0, 127, 127]), "a shaded quad");
        assert!(!from_behind(CULL_BACK), "entitySolid");
        assert!(!from_behind([0, 0, 127, SHADED_CULL_W]), "entityCutoutCull");

        let turned = scene_matrix([0.0, 0.0, 100.0, 100.0], view(180.0, 0.0));
        assert!(
            drawn_quad(&drawn(SOUTH, !0, CULL_BACK)).seen(turned),
            "a culling quad still shows from its front"
        );
    }

    #[test]
    fn a_state_the_registry_does_not_know_occupies_nothing() {
        let block = |pos, id| NativeSceneBlock::new(pos, BlockStateId(id), None);
        let blocks = [
            block([0, 0, 0], 9),
            block([1, 0, 0], 10),
            block([2, 0, 0], u32::MAX),
        ];
        let occupied = occupied_states(&blocks, 10);
        assert_eq!(occupied.len(), 1);
        assert_eq!(occupied.get(&[0, 0, 0]), Some(&BlockStateId(9)));
    }

    /// Unturned, the scene is seen from the north as a player at yaw 0 sees
    /// it: the centre in the box's middle, up up the screen, east to the
    /// viewer's left, south away and its north face towards the viewer.
    #[test]
    fn an_unturned_scene_is_seen_from_the_north() {
        let m = scene_matrix([0.0, 0.0, 100.0, 60.0], view(0.0, 0.0));
        let at = |p: [f32; 3]| m.transform_point3(Vec3::from_array(p));
        assert_eq!(at([0.5, 0.5, 0.5]), Vec3::new(50.0, 30.0, 0.0));
        assert!(at([0.5, 1.5, 0.5]).y < 30.0, "up is up the screen");
        assert!(at([1.5, 0.5, 0.5]).x < 50.0, "east is on the left");
        assert!(at([0.5, 0.5, 1.5]).z < 0.0, "south is away");
        assert!(
            m.transform_vector3(Vec3::NEG_Z).z > 0.0,
            "the north face is seen"
        );
        assert!(
            m.transform_vector3(Vec3::Y).z.abs() < 1e-5,
            "the top is edge on"
        );
    }

    /// A tilt shows the top and hides the bottom; a quarter turn brings the
    /// east face round to the viewer (counterclockwise seen from above: east
    /// goes to the north, the side the viewer is on).
    #[test]
    fn pitch_shows_the_top_and_yaw_turns_the_scene() {
        let tilted = scene_matrix([0.0, 0.0, 100.0, 100.0], view(0.0, 30.0));
        assert!(tilted.transform_vector3(Vec3::Y).z > 0.0);
        assert!(tilted.transform_vector3(Vec3::NEG_Y).z < 0.0);
        let turned = scene_matrix([0.0, 0.0, 100.0, 100.0], view(90.0, 0.0));
        assert!(turned.transform_vector3(Vec3::X).z > 0.99);
    }

    #[test]
    fn faces_are_shaded_as_blocks_are() {
        assert_eq!(face_shade(Vec3::Y), 1.0);
        assert_eq!(face_shade(Vec3::NEG_Y), 0.5);
        assert!((face_shade(Vec3::Z) - 0.8).abs() < 1e-6);
        assert!((face_shade(Vec3::X) - 0.6).abs() < 1e-6);
        assert_eq!(shaded(0xFF80_4020, 0.5), 0xFF40_2010);
    }
}
