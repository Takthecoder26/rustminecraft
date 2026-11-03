use super::*;

/// Run explicitly on renderer changes: `cargo test -p horizon-client-render --lib provenance -- --include-ignored --test-threads=1`.
#[test]
#[ignore = "requires a Vulkan/Metal/DX12 adapter"]
fn emulated_geometry_reads_provenance_of_each_pulled_source_vertex() {
    super::provenance_tests::render_probe(|inputs| {
        let pp = |code: &str| {
            use std::io::{Cursor, Write};
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            zip.start_file(
                "shaders/probe.glsl",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(code.as_bytes()).unwrap();
            let pack = horizon_shaders::Pack::from_zip_bytes(
                "original-geometry-probe",
                zip.finish().unwrap().into_inner(),
            )
            .unwrap();
            {
                let stage = horizon_shaders::preprocess(
                    &pack,
                    "probe.glsl",
                    &horizon_shaders::Macros::standard(),
                );
                assert!(stage.diagnostics.is_empty(), "{:?}", stage.diagnostics);
                stage
            }
        };
        let vertex = pp(r#"#version 450 compatibility
attribute vec4 mc_Entity;
attribute vec4 at_midBlock;
varying vec4 source_report;
void main() {
    uint raw=uint(gl_VertexID);
    vec3 center=raw>=512u ? at_midBlock.xyz : gl_Vertex.xyz+at_midBlock.xyz/64.0;
    source_report=vec4(mc_Entity.x,at_midBlock.w,center.x,center.z);
    vec2 corners[4]=vec2[4](vec2(-1,-1),vec2(-1,1),vec2(-0.5,1),vec2(-0.5,-1));
    uint index=raw>=768u ? raw-768u+12u : (raw>=512u ? raw-512u+8u : raw-256u);
    gl_Position=vec4(corners[index%4u]+vec2(float(index/4u)*0.5,0.0),0.0,1.0);
}"#);
        let geometry = pp(r#"#version 450 compatibility
layout(triangles) in;
layout(triangle_strip,max_vertices=3) out;
in vec4 source_report[];
out vec4 report;
void main() {
    for(int i=0;i<3;i++) { report=source_report[i];gl_Position=gl_in[i].gl_Position;EmitVertex(); }
    EndPrimitive();
}"#);
        let fragment = pp(
            "#version 450 compatibility\nin vec4 report;layout(location=0) out vec4 color;void main(){color=report;}",
        );
        let mut inputs = horizon_shaders::GeometryInputs {
            inputs: inputs.clone(),
            pull: PULL.into(),
            index: "hz_indices[@]".into(),
        };
        inputs.inputs.declarations.push_str(
            "layout(set=5,binding=0) readonly buffer HzIndices { uint hz_indices[]; };\n",
        );
        let (program, triangles) =
            horizon_shaders::translate_geometry(&vertex, &geometry, &fragment, &inputs).unwrap();
        assert_eq!(triangles, 1);
        (program.vertex, true, program.interface.uniform_size)
    });
}
