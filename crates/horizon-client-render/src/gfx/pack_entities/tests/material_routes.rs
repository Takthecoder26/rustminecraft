//! CPU controls call the production draw planner without constructing a GPU instance.
use super::*;
use crate::gfx::entity_skin::{SkinFrame, SkinJob};

#[derive(Default)]
struct Cpu {
    vertices: Vec<Vertex>,
    uniforms: Vec<u8>,
    draws: Vec<Draw>,
    rest: Vec<Vertex>,
}

impl Cpu {
    fn prepare(
        &mut self,
        vertices: &[Vertex],
        spans: &[EntitySpan],
        programs: [bool; PROGRAMS],
        casters: Casters,
        skin: Option<&SkinFrame>,
    ) -> u32 {
        DrawPlan {
            scratch: &mut self.vertices,
            uniforms: &mut self.uniforms,
            draws: &mut self.draws,
            stride: 256,
        }
        .prepare(
            vertices,
            spans,
            programs,
            ([0.; 4], [1; 2]),
            casters,
            &mut self.rest,
            skin,
        )
    }

    fn word(&self, slot: usize, field: usize) -> i32 {
        let at = slot * 256 + field * 4;
        i32::from_le_bytes(self.uniforms[at..at + 4].try_into().unwrap())
    }
}

fn span(start: u32, stage: &str, id: i32) -> EntitySpan {
    EntitySpan {
        start,
        stage: render_stage(stage),
        id,
        ..Default::default()
    }
}

fn quad(layer: u32, x: f32) -> [Vertex; 4] {
    std::array::from_fn(|i| Vertex {
        pos: [x, (i % 2) as f32, 0.0],
        uv: [0.25, if i == 0 || i == 3 { 1.0 } else { 0.0 }],
        layer,
        color: 0xffff_ffff,
        light: 0xffff_0000,
        ..bytemuck::Zeroable::zeroed()
    })
}

fn casters() -> Casters {
    Casters {
        entities: true,
        blocks: true,
        ..Default::default()
    }
}

#[test]
fn block_materials_portals_and_beams_keep_distinct_draws_and_subjects() {
    let vertices: Vec<_> = [
        (0, 0.0),
        (super::super::entity_batch::OIT_ENTITY | 3 << 16, 1.0),
        (super::super::TRANSLUCENT_ITEM_OVERLAY, 2.0),
        (0, 3.0),
        (9, 4.0),
        (6, 5.0),
    ]
    .into_iter()
    .flat_map(|(layer, x)| quad(layer, x))
    .collect();
    let mut cpu = Cpu::default();
    cpu.prepare(
        &vertices,
        &[span(0, "BLOCK_ENTITIES", 111)],
        [true; PROGRAMS],
        casters(),
        None,
    );
    assert!(cpu.rest.is_empty());
    assert_eq!(
        cpu.draws
            .iter()
            .map(|d| (d.base, d.quads, d.program, d.portal, d.shadow))
            .collect::<Vec<_>>(),
        [
            (0, 1, BLOCK, false, true),
            (4, 2, BLOCK_TRANSLUCENT, false, true),
            (12, 1, BLOCK, false, true),
            (16, 1, BLOCK, true, true),
            (20, 1, BEAM, false, false),
        ]
    );
    for (slot, draw) in cpu.draws.iter().enumerate() {
        assert_eq!(draw.slot as usize, slot);
        assert_eq!(cpu.word(slot, 0), render_stage("BLOCK_ENTITIES") as i32);
        if draw.program != BEAM {
            assert_eq!((cpu.word(slot, 1), cpu.word(slot, 2)), (0, 111));
            assert_eq!(shadow_owner(draw.program), Some(BLOCK));
        }
        assert!(
            cpu.uniforms[slot * 256 + 32..(slot + 1) * 256]
                .iter()
                .all(|&b| b == 0)
        );
    }
    assert_eq!(
        cpu.vertices
            .iter()
            .step_by(4)
            .map(|v| v.pos[0])
            .collect::<Vec<_>>(),
        [0., 1., 2., 3., 4., 5.]
    );
    assert_eq!(
        cpu.vertices[16..20]
            .iter()
            .map(|v| v.uv)
            .collect::<Vec<_>>(),
        [[0., 0.], [0., 1.], [1., 1.], [1., 0.]]
    );
    assert!(
        cpu.vertices[16..]
            .iter()
            .all(|v| v.layer == 0 && v.light == 0xf0f0)
    );
}

#[test]
fn particle_materials_use_the_layer_tag_not_hdr_or_vertex_alpha() {
    let mut vertices: Vec<_> = [0, 15 | 4 << 16, 0, 15]
        .into_iter()
        .enumerate()
        .flat_map(|(i, layer)| quad(layer, i as f32))
        .collect();
    vertices[..4].iter_mut().for_each(|v| v.color = 0x7fff_ffff);
    let mut cpu = Cpu::default();
    cpu.prepare(
        &vertices,
        &[span(0, "PARTICLES", 0)],
        [true; PROGRAMS],
        casters(),
        None,
    );
    assert_eq!(
        cpu.draws
            .iter()
            .map(|d| (d.base, d.program, d.shadow))
            .collect::<Vec<_>>(),
        [
            (0, PARTICLE, false),
            (4, PARTICLE_TRANSLUCENT, false),
            (8, PARTICLE, false),
            (12, PARTICLE_TRANSLUCENT, false),
        ]
    );
    let mut available = [true; PROGRAMS];
    available[PARTICLE_TRANSLUCENT] = false;
    cpu.prepare(
        &vertices,
        &[span(0, "PARTICLES", 0)],
        available,
        casters(),
        None,
    );
    assert_eq!(
        cpu.draws.iter().map(|d| d.program).collect::<Vec<_>>(),
        [PARTICLE, PARTICLE]
    );
    assert_eq!(
        cpu.rest
            .iter()
            .step_by(4)
            .map(|v| v.pos[0])
            .collect::<Vec<_>>(),
        [1., 3.]
    );
}

#[test]
fn partial_block_coverage_keeps_native_remainder_and_hidden_shadow_ownership() {
    let vertices: Vec<_> = [0, 32, 33, 0]
        .into_iter()
        .enumerate()
        .flat_map(|(i, layer)| quad(layer, i as f32))
        .collect();
    let mut available = [false; PROGRAMS];
    available[BLOCK_TRANSLUCENT] = true;
    let subject = EntitySpan {
        emissive: true,
        ..span(0, "BLOCK_ENTITIES", 73)
    };
    let only_lit = Casters {
        blocks: true,
        only_lit: true,
        ..Default::default()
    };
    let mut cpu = Cpu::default();
    cpu.prepare(&vertices, &[subject], available, only_lit, None);
    assert_eq!(
        cpu.rest
            .iter()
            .step_by(4)
            .map(|v| v.pos[0])
            .collect::<Vec<_>>(),
        [0., 3.]
    );
    assert_eq!(
        cpu.draws
            .iter()
            .map(|d| (d.program, d.pack, d.shadow))
            .collect::<Vec<_>>(),
        [
            (BLOCK, false, true),
            (BLOCK_TRANSLUCENT, true, true),
            (BLOCK, false, true),
        ]
    );
    cpu.prepare(
        &vertices,
        &[EntitySpan {
            hidden: true,
            ..subject
        }],
        available,
        only_lit,
        None,
    );
    assert!(cpu.rest.is_empty());
    assert!(cpu.draws.iter().all(|d| !d.pack && d.shadow));
    cpu.prepare(
        &vertices,
        &[EntitySpan {
            emissive: false,
            hidden: true,
            ..subject
        }],
        available,
        only_lit,
        None,
    );
    assert!(cpu.draws.is_empty() && cpu.vertices.is_empty() && cpu.uniforms.is_empty());
}

#[test]
fn posed_prefix_cpu_offsets_and_uniform_ids_survive_material_splits_and_reuse() {
    let skin = SkinFrame {
        vertices: 12,
        spans: vec![
            span(0, "ENTITIES", 40),
            span(4, "BLOCK_ENTITIES", 99),
            EntitySpan {
                hidden: true,
                ..span(8, "ENTITIES", 41)
            },
        ],
        jobs: [1, super::super::entity_batch::OIT_ENTITY | 5, 0]
            .into_iter()
            .enumerate()
            .map(|(i, overlay)| SkinJob {
                dst: i as u32 * 4,
                count: 4,
                overlay,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let vertices: Vec<_> = [quad(2, 0.), quad(0, 1.), quad(32, 2.)]
        .into_iter()
        .flatten()
        .collect();
    let mut cpu = Cpu::default();
    assert_eq!(
        cpu.prepare(
            &vertices,
            &[span(4, "BLOCK_ENTITIES", 61)],
            [true; PROGRAMS],
            casters(),
            Some(&skin)
        ),
        12
    );
    assert_eq!(
        cpu.draws
            .iter()
            .map(|d| (d.base, d.program, d.pack, d.shadow))
            .collect::<Vec<_>>(),
        [
            (0, ENTITY, true, true),
            (4, BLOCK_TRANSLUCENT, true, true),
            (8, ENTITY, false, true),
            (12, BLOCK, true, true),
            (16, BLOCK_TRANSLUCENT, true, true),
        ]
    );
    assert_eq!(
        (0..5)
            .map(|slot| (cpu.word(slot, 1), cpu.word(slot, 2)))
            .collect::<Vec<_>>(),
        [(40, 0), (0, 99), (41, 0), (0, 61), (0, 61)]
    );
    let color: &[f32] = bytemuck::cast_slice(&cpu.uniforms[16..32]);
    assert_eq!(&color[..3], &[1., 0., 0.]);
    assert!((color[3] - 77. / 255.).abs() < 1e-6);
    assert_eq!(cpu.vertices.len(), 8);
    assert_eq!(cpu.rest.len(), 4);
    cpu.prepare(&[], &[], [true; PROGRAMS], casters(), None);
    assert!(
        cpu.draws.is_empty()
            && cpu.vertices.is_empty()
            && cpu.rest.is_empty()
            && cpu.uniforms.is_empty()
    );
}
