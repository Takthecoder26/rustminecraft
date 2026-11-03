use super::*;

#[test]
fn provenance_follows_quads_through_reordering_and_keeps_full_state_ids() {
    let mut mesh = SectionMesh {
        provenance: Some(Default::default()),
        ..Default::default()
    };
    for (id, owner, pos, translucent) in [
        (0x1234_5678, [3, 4, 5], [5.0, 4.5, 5.0], false),
        (77, [0, 15, 15], [0.0, 15.0, 15.0], false),
        (123, [7, 2, 1], [7.0, 2.0, 1.0], true),
    ] {
        let before = [mesh.solid.len(), mesh.translucent.len()];
        let part = if translucent {
            &mut mesh.translucent
        } else {
            &mut mesh.solid
        };
        // Different source cells, the same sprite. The first model overhangs its cell.
        part.extend(
            [[0., 0., 0.], [0., 0., 1.], [1., 0., 1.], [1., 0., 0.]].map(|d| {
                TerrainVertex::new(std::array::from_fn(|k| pos[k] + d[k]), [0.; 2], 0, id, 0)
            }),
        );
        mesh.record_provenance(before, id, owner);
    }
    mesh.sort_solid();
    let provenance = mesh.provenance.as_ref().unwrap();
    for (vertices, records) in [
        (&mesh.solid, &provenance[0]),
        (&mesh.translucent, &provenance[1]),
    ] {
        for (quad, record) in vertices.as_chunks::<4>().0.iter().zip(records) {
            assert_eq!(quad[0].color, record[0]);
            let center = [0, 4, 8].map(|shift| ((record[1] >> shift) & 15) as f32 + 0.5);
            match record[0] {
                0x1234_5678 => assert_eq!(center, [3.5, 4.5, 5.5]),
                77 => assert_eq!(center, [0.5, 15.5, 15.5]),
                123 => assert_eq!(center, [7.5, 2.5, 1.5]),
                _ => unreachable!(),
            }
        }
    }
    let mut ordinary = SectionMesh::default();
    ordinary.solid.extend(mesh.solid.iter().copied());
    ordinary.record_provenance([0, 0], 1, [0, 0, 0]);
    ordinary.sort_solid();
    assert!(ordinary.provenance.is_none());
    let capacity = mesh
        .provenance
        .as_ref()
        .unwrap()
        .iter()
        .map(Vec::capacity)
        .sum::<usize>();
    mesh.clear();
    assert!(mesh.provenance.as_ref().unwrap().iter().all(Vec::is_empty));
    assert_eq!(
        mesh.provenance
            .as_ref()
            .unwrap()
            .iter()
            .map(Vec::capacity)
            .sum::<usize>(),
        capacity
    );
}

#[test]
#[ignore = "requires a scratch copy of prepared 26.3 resources (HORIZON_MESH_TEST_RESOURCES)"]
fn shader_provenance_comes_from_model_and_fluid_owners() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_MESH_TEST_RESOURCES").expect("explicit prepared resources"),
    );
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let render = assets.reload(&root).unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &render, &facts, visibility.clone()).unwrap();
    for (name, key, value, owner, fluid) in [
        ("minecraft:stone", "", "", [2, 3, 4], false),
        ("minecraft:oak_slab", "type", "top", [5, 6, 7], false),
        ("minecraft:dandelion", "", "", [9, 8, 9], false),
        ("minecraft:furnace", "lit", "false", [4, 6, 12], false),
        ("minecraft:furnace", "lit", "true", [4, 6, 12], false),
        ("minecraft:water", "level", "3", [12, 10, 12], true),
        (
            "minecraft:barrier",
            "waterlogged",
            "true",
            [1, 12, 14],
            true,
        ),
    ] {
        let state = (0..assets.registry.len())
            .map(|i| BlockStateId(i as u32))
            .find(|id| {
                assets.registry.state(*id).is_some_and(|s| {
                    s.name == name
                        && (key.is_empty() || s.properties.get(key).is_some_and(|v| v == value))
                })
            })
            .unwrap();
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(owner[0], MIN_Y + owner[1], owner[2], state.0);
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(&chunk);
        let mut mesher = Mesher {
            nether_shading: false,
            shader_pack_lighting: true,
            shader_provenance: true,
            separate_ao: false,
            assets: &assets,
            render: &render,
            tables: &tables,
            biomes: &biomes,
            visibility: visibility.clone(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: false,
            cutout_leaves: true,
            translucent_layers: &[],
            leaf_culling: LeafCullingMode::Off,
        };
        let mut mesh = mesher.mesh(&region, 0);
        mesh.sort_solid();
        let records = mesh.provenance.as_ref().unwrap();
        eprintln!(
            "provenance mesh {name} {key}={value}: state {}, solid/translucent quads {}/{}",
            state.0,
            records[0].len(),
            records[1].len()
        );
        assert!(!records[0].is_empty() || !records[1].is_empty(), "{name}");
        let expected_state = if fluid && name != "minecraft:water" {
            assets.registry.default_state("minecraft:water").unwrap().0
        } else {
            state.0
        };
        for ((vertices, records), other) in
            [(&mesh.solid, &records[0]), (&mesh.translucent, &records[1])]
                .into_iter()
                .zip([false, true])
        {
            assert_eq!(
                vertices.len() / 4,
                records.len(),
                "{name} translucent={other}"
            );
            for record in records {
                assert_eq!(record[0], expected_state, "{name}");
                let center = [0, 4, 8].map(|shift| ((record[1] >> shift) & 15) as f32 + 0.5);
                assert_eq!(center, owner.map(|n| n as f32 + 0.5), "{name}");
            }
        }
        mesher.shader_provenance = false;
        let ordinary = mesher.mesh_reusing(&region, 0, mesh, &mut MeshScratch::default());
        assert!(ordinary.provenance.is_none());
        mesher.shader_provenance = true;
        let again = mesher.mesh_reusing(&region, 0, ordinary, &mut MeshScratch::default());
        assert!(
            again
                .provenance
                .as_ref()
                .is_some_and(|p| !p[0].is_empty() || !p[1].is_empty())
        );
    }
}
