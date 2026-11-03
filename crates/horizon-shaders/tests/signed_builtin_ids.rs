//! GLSL's GL ID names are signed even though Naga's backend Index inputs are unsigned.
use std::io::{Cursor, Write};

use horizon_shaders::{Macros, Pack, Program, Target, preprocess, translate_composite};

fn program(source: &str) -> Result<Program, Vec<horizon_shaders::Error>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, text) in [
        ("shaders/composite.vsh", source),
        (
            "shaders/composite.fsh",
            "#version 130\nvoid main() { gl_FragColor = vec4(1.0); }",
        ),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    let pack = Pack::from_zip_bytes("ids.zip", zip.finish().unwrap().into_inner()).unwrap();
    let macros = Macros::standard();
    translate_composite(
        &preprocess(&pack, "composite.vsh", &macros),
        &preprocess(&pack, "composite.fsh", &macros),
        Target::Buffers,
    )
}

fn signed_id(name: &str) {
    let first_version = if name == "gl_InstanceID" {
        "140"
    } else {
        "130"
    };
    for version in [first_version, "330 compatibility", "450 core"] {
        for expression in [
            name.to_owned(),
            format!("{name} % 3"),
            format!("{name} - 1"),
        ] {
            let source = format!(
                "#version {version}\nint takeSigned(int x) {{ return x; }}\n\
                 void main() {{ int i = {expression}; gl_Position = vec4(float(takeSigned(i)), 0.0, 0.0, 1.0); }}"
            );
            program(&source).unwrap_or_else(|e| panic!("{version}: {expression}: {e:?}"));
        }
    }
}

#[test]
fn vertex_id_is_a_signed_glsl_integer() {
    signed_id("gl_VertexID");
}

#[test]
fn instance_id_is_a_signed_glsl_integer() {
    signed_id("gl_InstanceID");
}

#[test]
fn explicit_cast_controls_and_raw_backend_inputs_keep_working() {
    for source in [
        "#version 140\nvoid main() { int v = int(gl_VertexID), i = int(gl_InstanceID); gl_Position = vec4(float(v + i), 0.0, 0.0, 1.0); }",
        "#version 450\nvoid main() { uint v = gl_VertexIndex, i = gl_InstanceIndex; gl_Position = vec4(float(v + i), 0.0, 0.0, 1.0); }",
    ] {
        let p = program(source).unwrap_or_else(|e| panic!("{e:?}"));
        // These are the backend's wgpu inputs, not the GL source expression's type.
        let mut indices = 0;
        for argument in &p.vertex.entry_points[0].function.arguments {
            if matches!(
                argument.binding,
                Some(naga::Binding::BuiltIn(
                    naga::BuiltIn::VertexIndex | naga::BuiltIn::InstanceIndex
                ))
            ) {
                assert!(matches!(
                    p.vertex.types[argument.ty].inner,
                    naga::TypeInner::Scalar(naga::Scalar {
                        kind: naga::ScalarKind::Uint,
                        width: 4
                    })
                ));
                indices += 1;
            }
        }
        assert_eq!(indices, 2);
    }
}

#[test]
fn unknown_id_names_stay_errors() {
    let errors = program("#version 130\nvoid main() { gl_Position = vec4(float(gl_UnknownID)); }")
        .err()
        .unwrap();
    assert!(errors.iter().any(|e| e.message.contains("gl_UnknownID")));
}
