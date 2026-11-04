//! Original CPU material-run and posed-metadata controls.
use super::*;

#[test]
fn material_runs_preserve_subjects_and_ignore_hdr_and_instantaneous_alpha() {
    for (stage, layers, opaque, translucent) in [
        (
            "BLOCK_ENTITIES",
            [
                0,
                super::super::entity_batch::OIT_ENTITY | (3 << 16),
                super::super::TRANSLUCENT_ITEM_OVERLAY,
                0,
            ],
            BLOCK,
            BLOCK_TRANSLUCENT,
        ),
        (
            "PARTICLES",
            [
                3 << 16,
                super::super::TRANSLUCENT_PARTICLE_OVERLAY | (4 << 16),
                super::super::TRANSLUCENT_PARTICLE_OVERLAY,
                0,
            ],
            PARTICLE,
            PARTICLE_TRANSLUCENT,
        ),
    ] {
        let vertices: Vec<Vertex> = layers
            .into_iter()
            .enumerate()
            .flat_map(|(i, layer)| {
                [Vertex {
                    layer,
                    color: if i.is_multiple_of(2) { 0 } else { u32::MAX },
                    pos: [i as f32, 0., 0.],
                    ..bytemuck::Zeroable::zeroed()
                }; 4]
            })
            .collect();
        let span = EntitySpan {
            id: 55,
            stage: render_stage(stage),
            hidden: true,
            emissive: true,
            ..Default::default()
        };
        let actual: Vec<_> = material_runs(span, &vertices)
            .map(|(s, v)| {
                assert_eq!(
                    (s.id, s.stage, s.hidden, s.emissive),
                    (55, span.stage, true, true)
                );
                (program_of(&s).unwrap(), v[0].pos[0] as u32, v.len())
            })
            .collect();
        assert_eq!(
            actual,
            [(opaque, 0, 4), (translucent, 1, 8), (opaque, 3, 4)],
            "{stage}"
        );
    }
}

#[test]
fn posed_block_materials_keep_the_block_shadow_program_and_lit_caster_rule() {
    let span = EntitySpan {
        stage: render_stage("BLOCK_ENTITIES"),
        id: 87,
        emissive: true,
        ..Default::default()
    };
    let skin = super::super::entity_skin::SkinFrame {
        vertices: 12,
        spans: vec![span],
        jobs: vec![
            super::super::entity_skin::SkinJob {
                dst: 0,
                count: 4,
                overlay: 0,
                ..Default::default()
            },
            super::super::entity_skin::SkinJob {
                dst: 4,
                count: 8,
                overlay: super::super::entity_batch::OIT_ENTITY,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let runs = skin_runs(&skin);
    assert_eq!(
        runs.iter()
            .map(|(s, r, _)| (s.id, program_of(s).unwrap(), r.clone()))
            .collect::<Vec<_>>(),
        [(87, BLOCK, 0..4), (87, BLOCK_TRANSLUCENT, 4..12)]
    );
    let casters = Casters {
        blocks: true,
        only_lit: true,
        ..Default::default()
    };
    for (span, _, _) in runs {
        let program = program_of(&span).unwrap();
        assert_eq!(shadow_program(program), Some(BLOCK));
        assert!(casters.cast(&span, program));
        assert!(!casters.cast(
            &EntitySpan {
                emissive: false,
                ..span
            },
            program
        ));
    }
}
