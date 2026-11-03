//! Original loader controls for unsupported tessellation stages.
use horizon_shaders::{GameInputs, GbuffersInputs, Macros, Pack, Properties, load_post};
use std::io::{Cursor, Write};

fn load(extra: &[(&str, &str)]) -> horizon_shaders::Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let base = [
        (
            "shaders/gbuffers_terrain.vsh",
            "#version 330\nvoid main() { gl_Position=gl_Vertex; }",
        ),
        (
            "shaders/gbuffers_terrain.fsh",
            "#version 330\nvoid main() { gl_FragColor=vec4(1.0,0.0,0.0,1.0); }",
        ),
        (
            "shaders/gbuffers_basic.vsh",
            "#version 330\nvoid main() { gl_Position=gl_Vertex; }",
        ),
        (
            "shaders/gbuffers_basic.fsh",
            "#version 330\nvoid main() { gl_FragColor=vec4(0.0,1.0,0.0,1.0); }",
        ),
    ];
    for &(name, text) in base.iter().chain(extra) {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    let pack =
        Pack::from_zip_bytes("original-tessellation", zip.finish().unwrap().into_inner()).unwrap();
    let inputs = GameInputs {
        terrain: Some(GbuffersInputs::default()),
        ..Default::default()
    };
    load_post(
        &pack,
        "",
        &Properties::default(),
        &Macros::standard(),
        &inputs,
    )
}

#[test]
fn tessellation_stages_select_the_existing_fallback_instead_of_disappearing() {
    // Either stage is enough to make ordinary vertex/fragment rendering wrong.
    for stages in [
        vec![
            (
                "shaders/gbuffers_terrain.tcs",
                "#version 400\nlayout(vertices=3) out; void main() { gl_out[gl_InvocationID].gl_Position=gl_in[gl_InvocationID].gl_Position; if(gl_InvocationID==0) { gl_TessLevelOuter[0]=2.0; gl_TessLevelOuter[1]=2.0; gl_TessLevelOuter[2]=2.0; gl_TessLevelInner[0]=2.0; } }",
            ),
            (
                "shaders/gbuffers_terrain.tes",
                "#version 400\nlayout(triangles,equal_spacing,cw) in; void main() { gl_Position=gl_in[0].gl_Position*gl_TessCoord.x+gl_in[1].gl_Position*gl_TessCoord.y+gl_in[2].gl_Position*gl_TessCoord.z+vec4(0.25,0.0,0.0,0.0); }",
            ),
        ],
        vec![(
            "shaders/gbuffers_terrain.tcs",
            "#version 400\nlayout(vertices=3) out; void main() { gl_out[gl_InvocationID].gl_Position=gl_in[gl_InvocationID].gl_Position; }",
        )],
        vec![(
            "shaders/gbuffers_terrain.tes",
            "#version 400\nlayout(triangles) in; void main() { gl_Position=gl_in[0].gl_Position+vec4(0.25,0.0,0.0,0.0); }",
        )],
    ] {
        let post = load(&stages);
        assert_eq!(
            post.terrain.as_ref().map(|p| p.name.as_str()),
            Some("gbuffers_basic"),
            "{:?}",
            post.notes
        );
        assert!(
            post.notes.iter().any(|n| n.contains("gbuffers_terrain")
                && n.contains("tessellation")
                && n.contains("skipped")),
            "{:?}",
            post.notes
        );
    }
}

#[test]
fn ordinary_programs_keep_their_selected_route() {
    let ordinary = load(&[]);
    assert_eq!(
        ordinary.terrain.as_ref().map(|p| p.name.as_str()),
        Some("gbuffers_terrain")
    );
}
