//! Original material-program contract fixtures. No third-party shader code or assets.
use horizon_shaders::{GameInputs, GbuffersInputs, Macros, Pack, Post, Properties, load_post};
use std::io::{Cursor, Write};

fn load(files: &[(&str, &str)], properties: &str) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for &(name, source) in files {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = Pack::from_zip_bytes(
        "original-material-contract",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let macros = Macros::standard();
    load_post(
        &pack,
        "",
        &Properties::parse(properties, &macros),
        &macros,
        &GameInputs {
            entities: Some(GbuffersInputs::default()),
            ..Default::default()
        },
    )
}

const VERTEX: &str = "#version 120\nvoid main() { gl_Position = gl_Vertex; }";
const FRAGMENT: &str = "#version 120\nvoid main() { gl_FragData[0] = vec4(1.0); }";

#[test]
fn mixed_keeps_opaque_particles_before_deferred() {
    let files = [
        ("gbuffers_particles.vsh", VERTEX),
        ("gbuffers_particles.fsh", FRAGMENT),
    ];
    let post = load(
        &files,
        "particles.ordering=mixed\nparticles.before.deferred=false",
    );
    assert!(post.particles.is_some(), "{:?}", post.notes);
    assert!(
        !post.particles_late,
        "mixed must leave opaque particles visible to deferred"
    );
    assert!(!load(&files, "particles.ordering=before").particles_late);
    assert!(load(&files, "particles.ordering=after").particles_late);
}

#[test]
fn translucent_material_files_contribute_their_resource_requirements() {
    let post = load(
        &[
            ("gbuffers_block_translucent.vsh", VERTEX),
            (
                "gbuffers_block_translucent.fsh",
                "#version 120\nconst vec4 colortex3ClearColor=vec4(0.25,0.5,0.75,1.0);\nvoid main(){gl_FragData[0]=vec4(1.0);}",
            ),
            ("gbuffers_particles_translucent.vsh", VERTEX),
            (
                "gbuffers_particles_translucent.fsh",
                "#version 120\nuniform sampler2D shadowtex0;\nconst bool colortex4Clear=false;\nvoid main(){gl_FragData[0]=texture2D(shadowtex0,vec2(0.5));}",
            ),
        ],
        "particles.ordering=mixed",
    );
    assert_eq!(
        post.config.buffers[3].clear_color,
        Some([0.25, 0.5, 0.75, 1.0]),
        "{:?}",
        post.notes
    );
    assert!(
        !post.config.buffers[4].clear,
        "translucent particle settings must be read"
    );
    assert!(
        post.shadow_pass,
        "a translucent-only material can request shadow resources"
    );
}

#[test]
fn material_variants_keep_authored_targets_blends_and_fallbacks() {
    let files = [
        ("gbuffers_block.vsh", VERTEX),
        ("gbuffers_block.fsh", FRAGMENT),
        ("gbuffers_particles.vsh", VERTEX),
        ("gbuffers_particles.fsh", FRAGMENT),
        ("gbuffers_block_translucent.vsh", VERTEX),
        (
            "gbuffers_block_translucent.fsh",
            "#version 120\n/* RENDERTARGETS: 3 */\nvoid main(){gl_FragData[0]=vec4(0.5);}",
        ),
        ("gbuffers_particles_translucent.vsh", VERTEX),
        ("gbuffers_particles_translucent.fsh", FRAGMENT),
    ];
    let post = load(
        &files,
        "blend.gbuffers_block=off\nblend.gbuffers_particles=off\nblend.gbuffers_block_translucent=SRC_ALPHA ONE_MINUS_SRC_ALPHA ZERO ZERO\nblend.gbuffers_block_translucent.colortex3=off",
    );
    let block = post
        .block_translucent
        .as_ref()
        .expect("block material variant");
    assert_eq!(block.name, "gbuffers_block_translucent");
    assert_eq!(block.buffers, [3]);
    use horizon_shaders::Factor::{OneMinusSrcAlpha, SrcAlpha, Zero};
    assert_eq!(block.blend, Some([SrcAlpha, OneMinusSrcAlpha, Zero, Zero]));
    assert_eq!(block.buffer_blends, [(3, None)]);
    assert_eq!(
        post.particles_translucent.as_ref().unwrap().name,
        "gbuffers_particles_translucent"
    );
    for (files, props) in [
        (&files[..4], ""),
        (
            &files[..],
            "program.gbuffers_block_translucent.enabled=false\nprogram.gbuffers_particles_translucent.enabled=false",
        ),
    ] {
        let fallback = load(files, props);
        assert_eq!(fallback.block_translucent.unwrap().name, "gbuffers_block");
        assert_eq!(
            fallback.particles_translucent.unwrap().name,
            "gbuffers_particles"
        );
    }
    let broken = files.map(|(name, source)| {
        (
            name,
            if name.ends_with("_translucent.fsh") {
                "#version 120\nvoid main( {"
            } else {
                source
            },
        )
    });
    let fallback = load(&broken, "");
    assert_eq!(fallback.block_translucent.unwrap().name, "gbuffers_block");
    assert_eq!(
        fallback.particles_translucent.unwrap().name,
        "gbuffers_particles"
    );
    assert!(
        !fallback.notes.is_empty(),
        "bad material programs must be diagnosed"
    );
}

#[test]
fn both_particle_materials_follow_explicit_order_before_the_legacy_hint() {
    for (properties, expected) in [
        (
            "particles.ordering=mixed\nparticles.before.deferred=true",
            (false, true),
        ),
        (
            "particles.ordering=before\nparticles.before.deferred=false",
            (false, false),
        ),
        (
            "particles.ordering=after\nparticles.before.deferred=true",
            (true, true),
        ),
        ("particles.before.deferred=true", (false, false)),
        ("particles.before.deferred=false", (true, true)),
        ("", (true, true)),
    ] {
        let post = load(&[("fixture.txt", "")], properties);
        assert_eq!(
            (post.particles_late, post.particles_translucent_late),
            expected,
            "{properties}"
        );
    }
}

#[test]
fn translucent_only_material_storage_is_sized_before_gpu_pipeline_creation() {
    let post = load(
        &[
            ("gbuffers_block_translucent.vsh", VERTEX),
            (
                "gbuffers_block_translucent.fsh",
                "#version 430\nlayout(std430,binding=7) readonly buffer MaterialData { vec4 value; };\nvoid main(){gl_FragData[0]=value;}",
            ),
        ],
        "bufferObject.7=1",
    );
    assert_eq!(
        post.buffer_minimums().get(&7),
        Some(&16),
        "{:?}",
        post.notes
    );
}
