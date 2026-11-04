use super::*;

fn load(names: &[&str], properties: &str) -> Post {
    const VERTEX: &str = "#version 120\nvoid main(){gl_Position=gl_Vertex;}";
    const FRAGMENT: &str = "#version 120\nvoid main(){gl_FragData[0]=vec4(1.0);}";
    let files: Vec<_> = names
        .iter()
        .flat_map(|name| {
            [
                (format!("shaders/{name}.vsh"), VERTEX),
                (format!("shaders/{name}.fsh"), FRAGMENT),
            ]
        })
        .collect();
    let files: Vec<_> = files
        .iter()
        .map(|(name, text)| (name.as_str(), *text))
        .collect();
    let pack = Pack::from_zip_bytes("material-routes", crate::pack::tests::zip(&files)).unwrap();
    let macros = Macros::standard();
    let properties = Properties::parse(properties, &macros);
    let post = load_post(
        &pack,
        "",
        &properties,
        &macros,
        &GameInputs {
            entities: Some(GbuffersInputs {
                body: "hz_Vertex=vec4(0.0,0.0,0.0,1.0);hz_Color=vec4(1.0);".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

#[test]
fn translucent_programs_select_their_own_sources_then_existing_fallbacks() {
    for (names, expected_block, expected_particle) in [
        (
            vec![
                "gbuffers_block",
                "gbuffers_block_translucent",
                "gbuffers_particles",
                "gbuffers_particles_translucent",
            ],
            "gbuffers_block_translucent",
            "gbuffers_particles_translucent",
        ),
        (
            vec!["gbuffers_block", "gbuffers_particles"],
            "gbuffers_block",
            "gbuffers_particles",
        ),
        (
            vec!["gbuffers_terrain", "gbuffers_textured_lit"],
            "gbuffers_terrain",
            "gbuffers_textured_lit",
        ),
        (vec!["gbuffers_basic"], "gbuffers_basic", "gbuffers_basic"),
    ] {
        let post = load(&names, "separateEntityDraws=true\nparticles.ordering=mixed");
        let block = post.block_translucent.unwrap();
        let particle = post.particles_translucent.unwrap();
        assert_eq!(
            (block.name.as_str(), block.group, block.blend),
            (expected_block, Group::Translucent, Some(TRANSLUCENT_BLEND))
        );
        assert_eq!(
            (particle.name.as_str(), particle.group, particle.blend),
            (
                expected_particle,
                Group::Translucent,
                Some(TRANSLUCENT_BLEND)
            )
        );
    }
    let post = load(
        &[
            "gbuffers_block_translucent",
            "gbuffers_particles_translucent",
        ],
        "",
    );
    assert!(post.block.is_none() && post.particles.is_none());
    assert!(post.block_translucent.is_some() && post.particles_translucent.is_some());
}

#[test]
fn particle_ordering_controls_each_material_and_its_sampler_phase() {
    for (properties, deferred, expected) in [
        ("particles.ordering=before", true, [false, false]),
        ("particles.ordering=after", false, [true, true]),
        ("particles.ordering=mixed", true, [false, true]),
        (
            "particles.ordering=mixed\nparticles.before.deferred=true",
            true,
            [false, true],
        ),
        ("particles.before.deferred=true", true, [false, false]),
        ("particles.before.deferred=false", false, [true, true]),
        ("", true, [true, true]),
        ("", false, [false, true]),
    ] {
        let mut names = vec!["gbuffers_particles", "gbuffers_particles_translucent"];
        if deferred {
            names.push("deferred");
        }
        let post = load(&names, properties);
        assert_eq!(
            [post.particles_late, post.particles_translucent_late],
            expected,
            "{properties}, deferred={deferred}"
        );
        for (program, late) in [post.particles, post.particles_translucent]
            .into_iter()
            .zip(expected)
        {
            assert_eq!(
                program.unwrap().group,
                if late {
                    Group::Translucent
                } else {
                    Group::Gbuffers
                }
            );
        }
    }
}

#[test]
fn material_routes_keep_program_specific_blends_and_entity_phase() {
    let names = [
        "gbuffers_block",
        "gbuffers_block_translucent",
        "gbuffers_entities_translucent",
        "gbuffers_particles",
        "gbuffers_particles_translucent",
    ];
    for separate in [false, true] {
        let post = load(
            &names,
            &format!(
                "separateEntityDraws={separate}\nparticles.ordering=mixed\nblend.gbuffers_block=off\nblend.gbuffers_particles=off\nblend.gbuffers_block_translucent=ONE ZERO ONE ZERO\nblend.gbuffers_particles_translucent=off"
            ),
        );
        assert_eq!(post.block.unwrap().blend, None);
        assert_eq!(post.particles.unwrap().blend, None);
        let block = post.block_translucent.unwrap();
        assert_eq!(
            block.blend,
            Some([Factor::One, Factor::Zero, Factor::One, Factor::Zero])
        );
        assert_eq!(
            block.group,
            if separate {
                Group::Translucent
            } else {
                Group::Gbuffers
            }
        );
        assert_eq!(post.entities_translucent.unwrap().group, block.group);
        assert_eq!(post.particles_translucent.unwrap().blend, None);
    }
}
