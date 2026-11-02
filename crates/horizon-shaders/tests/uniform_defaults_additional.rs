// SPDX-License-Identifier: MIT
//! Original supplemental uniform-default controls, authored before executing them.
//! No third-party shader source or converter implementation was used.
//! Values/relative offsets below are independent
//! GLSL/std140 expectations, read through the real loader and Interface seed helper.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use horizon_shaders::{GameInputs, Interface, Macros, Pack, Post, load_post, read_properties};

const VERTEX: &str = "#version 330\nvoid main(){gl_Position=vec4(0.0,0.0,0.0,1.0);}";

struct ScratchPack(PathBuf);

impl ScratchPack {
    fn new(files: &[(&str, &str)]) -> Self {
        static SERIAL: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-uniform-review-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        for (name, text) in files {
            let path = root.join("shaders").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Self(root)
    }
}

impl Drop for ScratchPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn load(files: &[(&str, &str)]) -> Post {
    let scratch = ScratchPack::new(files);
    let pack = Pack::open(&scratch.0).unwrap();
    let mut macros = Macros::standard();
    let (properties, notes) = read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    load_post(&pack, "", &properties, &macros, &GameInputs::default())
}

fn fragment(source: &str) -> Post {
    load(&[("composite.vsh", VERTEX), ("composite.fsh", source)])
}

fn seeded(source: &str) -> Interface {
    let mut post = fragment(source);
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.programs.len(), 1, "program disappeared");
    post.programs[0].program.take().unwrap().interface
}

fn assert_words(interface: &Interface, name: &str, ty: &str, expected: &[u32]) {
    let uniform = interface.uniforms.iter().find(|u| u.name == name).unwrap();
    assert_eq!(uniform.ty, ty);
    assert!(
        uniform.initializer.is_some(),
        "{name}: authored default lost"
    );
    let block = interface.uniform_block();
    let start = uniform.offset as usize;
    let actual: Vec<u32> = block[start..start + expected.len() * 4]
        .chunks_exact(4)
        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(actual, expected, "{name}: bytes/padding differ");
}

#[test]
fn unsigned_vector_defaults_keep_every_bit_above_float_precision() {
    let interface = seeded(
        "#version 330\nuniform uvec4 fixtureDefault=uvec4(0xffffffffu,0x80000000u,16777217u,0x12345678u);out vec4 c;void main(){c=vec4(fixtureDefault&uvec4(255u))/255.0;}",
    );
    assert_words(
        &interface,
        "fixtureDefault",
        "uvec4",
        &[0xffff_ffff, 0x8000_0000, 0x0100_0001, 0x1234_5678],
    );
}

#[test]
fn asymmetric_matrix_arrays_keep_column_and_array_padding_zero() {
    let interface = seeded(
        "#version 330\nuniform mat2x3 fixtureDefault[2]=mat2x3[2](mat2x3(1.0,2.0,3.0,4.0,5.0,6.0),mat2x3(7.0,8.0,9.0,10.0,11.0,12.0));out vec4 c;void main(){c=vec4(fixtureDefault[0][0]+fixtureDefault[1][1],1.0);}",
    );
    // Two matrices; two vec3 columns each; column stride 16, matrix/array stride 32.
    // The fourth word of every column is padding, including the final column.
    assert_words(
        &interface,
        "fixtureDefault",
        "mat2x3[2]",
        &[
            0x3f80_0000,
            0x4000_0000,
            0x4040_0000,
            0,
            0x4080_0000,
            0x40a0_0000,
            0x40c0_0000,
            0,
            0x40e0_0000,
            0x4100_0000,
            0x4110_0000,
            0,
            0x4120_0000,
            0x4130_0000,
            0x4140_0000,
            0,
        ],
    );
}

#[test]
fn generated_mirror_names_do_not_shadow_authored_constants_or_declarators() {
    let interface = seeded(
        "#version 330\nconst float hz_uniform_default_0=0.25;const float hz_uniform_default_1=0.125;uniform float first=hz_uniform_default_1,fixtureDefault=hz_uniform_default_0*2.0,last;out vec4 c;void main(){c=vec4(first,fixtureDefault,last,1.0);}",
    );
    assert_words(&interface, "first", "float", &[0x3e00_0000]);
    assert_words(&interface, "fixtureDefault", "float", &[0x3f00_0000]);
    let absent = interface
        .uniforms
        .iter()
        .find(|u| u.name == "last")
        .unwrap();
    assert!(absent.initializer.is_none());
}

#[test]
fn explicit_zero_has_seed_metadata_and_absent_initializer_does_not() {
    let interface = seeded(
        "#version 330\nuniform float fixtureZero=0.0,fixtureAbsent;out vec4 c;void main(){c=vec4(fixtureZero,fixtureAbsent,0.0,1.0);}",
    );
    assert_words(&interface, "fixtureZero", "float", &[0]);
    let absent = interface
        .uniforms
        .iter()
        .find(|u| u.name == "fixtureAbsent")
        .unwrap();
    assert!(absent.initializer.is_none());
    let start = absent.offset as usize;
    assert_eq!(&interface.uniform_block()[start..start + 4], &[0; 4]);
}

#[test]
fn invalid_constant_in_an_include_keeps_the_authored_file_and_line() {
    let post = load(&[
        ("composite.vsh", VERTEX),
        (
            "composite.fsh",
            "#version 330\n#include \"lib/initializer.glsl\"\nout vec4 c;void main(){c=vec4(fixtureDefault);}",
        ),
        (
            "lib/initializer.glsl",
            "// diagnostic fixture\nconst float q=0.25;\nuniform float fixtureDefault=missingDefaultTerm+q;\n",
        ),
    ]);
    assert!(post.programs.is_empty(), "invalid initializer accepted");
    assert!(
        post.notes
            .iter()
            .any(|n| n.contains("lib/initializer.glsl:3:")),
        "authored include origin lost: {:?}",
        post.notes
    );
}

#[test]
fn initializer_validation_keeps_source_order_and_rejects_runtime_reads() {
    // Forward lookup is invalid; a runtime-uniform read is outside the approved
    // constant-default scope and must be diagnosed rather than replaced by zero.
    for source in [
        "#version 330\nuniform float fixtureDefault=authoredLater;const float authoredLater=0.25;out vec4 c;void main(){c=vec4(fixtureDefault);}",
        "#version 330\nuniform float priorDefault=0.25;uniform float fixtureDefault=priorDefault;out vec4 c;void main(){c=vec4(fixtureDefault+priorDefault);}",
    ] {
        let post = fragment(source);
        assert!(post.programs.is_empty(), "unsupported initializer accepted");
        assert!(!post.notes.is_empty(), "rejection had no diagnostic");
    }
}

#[test]
fn dedicated_suppliers_do_not_hide_conflicting_linked_initializers() {
    for (name, ty, first, second) in [
        ("renderStage", "int", "1", "2"),
        ("alphaTestRef", "float", "0.125", "0.75"),
    ] {
        for (fragment_value, valid) in [(first, true), (second, false)] {
            let vertex = format!(
                "#version 330\nuniform {ty} {name}={first};out float v;void main(){{v=float({name});gl_Position=vec4(0.0,0.0,0.0,1.0);}}"
            );
            let fragment = format!(
                "#version 330\nuniform {ty} {name}={fragment_value};in float v;out vec4 c;void main(){{c=vec4(float({name})+v);}}"
            );
            let scratch = ScratchPack::new(&[
                ("gbuffers_terrain.vsh", &vertex),
                ("gbuffers_terrain.fsh", &fragment),
            ]);
            let pack = Pack::open(&scratch.0).unwrap();
            let macros = Macros::standard();
            let vertex = horizon_shaders::preprocess(&pack, "gbuffers_terrain.vsh", &macros);
            let fragment = horizon_shaders::preprocess(&pack, "gbuffers_terrain.fsh", &macros);
            let result = horizon_shaders::translate_gbuffers(
                &vertex,
                &fragment,
                horizon_shaders::Target::Buffers,
                &horizon_shaders::GbuffersInputs::default(),
            );
            if valid {
                assert!(result.is_ok(), "{name}: matching defaults rejected");
            } else {
                let errors = result.err().unwrap_or_else(|| {
                    panic!("{name}: supplier hid conflicting linked initializers")
                });
                assert!(
                    errors
                        .iter()
                        .any(|e| e.message.contains("different initializers")),
                    "{name}: wrong diagnostic: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn multidimensional_defaults_keep_the_authored_shape_and_both_array_strides() {
    let interface = seeded(
        "#version 430\nuniform float fixtureDefault[2][3]=float[2][3](float[3](1.0,2.0,3.0),float[3](4.0,5.0,6.0));out vec4 c;void main(){c=vec4(fixtureDefault[1][2]);}",
    );
    // Outer count 2/stride 48; inner count 3/stride 16; each scalar's padding is zero.
    assert_words(
        &interface,
        "fixtureDefault",
        "float[2][3]",
        &[
            0x3f80_0000,
            0,
            0,
            0,
            0x4000_0000,
            0,
            0,
            0,
            0x4040_0000,
            0,
            0,
            0,
            0x4080_0000,
            0,
            0,
            0,
            0x40a0_0000,
            0,
            0,
            0,
            0x40c0_0000,
            0,
            0,
            0,
        ],
    );
    let absent = seeded(
        "#version 430\nuniform float fixtureDefault[2][3];out vec4 c;void main(){c=vec4(fixtureDefault[1][2]);}",
    );
    let u = absent
        .uniforms
        .iter()
        .find(|u| u.name == "fixtureDefault")
        .unwrap();
    assert!(u.initializer.is_none());
    assert_eq!(
        &absent.uniform_block()[u.offset as usize..u.offset as usize + 96],
        &[0; 96]
    );
}

#[test]
fn array_type_prefixes_keep_inner_dimensions_after_each_declarator() {
    let flat = seeded(
        "#version 430\nuniform float[3] fixtureDefault=float[3](1.0,2.0,3.0);out vec4 c;void main(){c=vec4(fixtureDefault[2]);}",
    );
    assert_words(
        &flat,
        "fixtureDefault",
        "float[3]",
        &[
            0x3f80_0000,
            0,
            0,
            0,
            0x4000_0000,
            0,
            0,
            0,
            0x4040_0000,
            0,
            0,
            0,
        ],
    );
    // GLSL 4.60, 4.1.9: float[3] a[2] is the same type as float a[2][3].
    let nested = seeded(
        "#version 430\nuniform float[3] fixtureDefault[2]=float[2][3](float[3](1.0,2.0,3.0),float[3](4.0,5.0,6.0));out vec4 c;void main(){c=vec4(fixtureDefault[1][2]);}",
    );
    assert_words(
        &nested,
        "fixtureDefault",
        "float[2][3]",
        &[
            0x3f80_0000,
            0,
            0,
            0,
            0x4000_0000,
            0,
            0,
            0,
            0x4040_0000,
            0,
            0,
            0,
            0x4080_0000,
            0,
            0,
            0,
            0x40a0_0000,
            0,
            0,
            0,
            0x40c0_0000,
            0,
            0,
            0,
        ],
    );
}

#[test]
fn malformed_array_suffix_is_diagnosed_instead_of_being_normalized_away() {
    let post = fragment(
        "#version 430\nuniform float fixtureDefault[2]unexpected[3]=float[2][3](float[3](1.0,2.0,3.0),float[3](4.0,5.0,6.0));out vec4 c;void main(){c=vec4(fixtureDefault[1][2]);}",
    );
    assert!(post.programs.is_empty(), "malformed suffix accepted");
    assert!(!post.notes.is_empty());
}

#[test]
fn linked_float_zeros_compare_equal_without_discarding_integer_bits_or_seed_signs() {
    for (ty, first, second, valid) in [
        ("float", "+0.0", "-0.0", true),
        ("vec3", "vec3(0.0)", "vec3(-0.0,0.0,-0.0)", true),
        ("uint", "0u", "0x80000000u", false),
    ] {
        let read = if ty == "vec3" {
            "sharedDefault.x"
        } else {
            "sharedDefault"
        };
        let vertex = format!(
            "#version 330\nuniform {ty} sharedDefault={first};out float v;void main(){{v=float({read});gl_Position=vec4(0,0,0,1);}}"
        );
        let fragment = format!(
            "#version 330\nuniform {ty} sharedDefault={second};in float v;out vec4 c;void main(){{c=vec4(float({read})+v);}}"
        );
        let post = load(&[("composite.vsh", &vertex), ("composite.fsh", &fragment)]);
        if valid {
            assert!(post.notes.is_empty(), "{ty}: {:?}", post.notes);
            assert_eq!(post.programs.len(), 1);
        } else {
            assert!(post.programs.is_empty());
            assert!(
                post.notes
                    .iter()
                    .any(|n| n.contains("different initializers"))
            );
        }
    }
    let negative = seeded(
        "#version 330\nuniform float fixtureDefault=-0.0;out vec4 c;void main(){c=vec4(fixtureDefault);}",
    );
    assert_words(&negative, "fixtureDefault", "float", &[0x8000_0000]);
}
