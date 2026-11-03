//! Original automatic-input contract controls.
use horizon_shaders::{GameInputs, GbuffersInputs, Macros, Pack, load_post, read_properties};
use std::io::{Cursor, Write};

fn pack(request: &str) -> Pack {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in [
        ("shaders/shaders.properties", request),
        (
            "shaders/gbuffers_terrain.vsh",
            "#version 330\nvarying float fade;\nvoid main() { fade=mc_chunkFade; gl_Position=gl_Vertex; }",
        ),
        (
            "shaders/gbuffers_terrain.fsh",
            "#version 330\nvarying float fade;\nvoid main() { gl_FragColor=vec4(fade,0.0,0.0,1.0); }",
        ),
        (
            "shaders/gbuffers_entities.vsh",
            "#version 330\nconst float sentinel=mc_chunkFade; void main() { gl_Position=gl_Vertex + vec4(sentinel); }",
        ),
        (
            "shaders/gbuffers_entities.fsh",
            "#version 330\nvoid main() { gl_FragColor=vec4(1.0); }",
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    Pack::from_zip_bytes("original-fade", zip.finish().unwrap().into_inner()).unwrap()
}

#[test]
fn requested_fade_is_automatic_for_terrain_and_other_gbuffers() {
    for request in [
        "iris.features.optional=FADE_VARIABLE",
        "iris.features.required=FADE_VARIABLE",
    ] {
        let pack = pack(request);
        let mut macros = Macros::standard();
        let (properties, notes) = read_properties(&pack, &mut macros);
        assert!(notes.is_empty(), "{notes:?}");
        let inputs = GameInputs {
            terrain: Some(GbuffersInputs::default()),
            entities: Some(GbuffersInputs::default()),
            ..Default::default()
        };
        let post = load_post(&pack, "", &properties, &macros, &inputs);
        assert!(post.terrain.is_some(), "{:?}", post.notes);
        assert!(post.entities.is_some(), "{:?}", post.notes);
    }
}

#[test]
fn unrequested_fade_is_not_injected() {
    let pack = pack("");
    let mut macros = Macros::standard();
    let (properties, _) = read_properties(&pack, &mut macros);
    let inputs = GameInputs {
        terrain: Some(GbuffersInputs::default()),
        ..Default::default()
    };
    let post = load_post(&pack, "", &properties, &macros, &inputs);
    assert!(post.terrain.is_none());
    assert!(
        post.notes.iter().any(|n| n.contains("mc_chunkFade")),
        "{:?}",
        post.notes
    );
}
