//! Original loader and typed std140 controls for declared uniform defaults.
//! Expected values are independent of the parser/serializer; the renderer consumes the same
//! Interface block-seeding helper used here, with provider/frame controls in its own tests.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use horizon_shaders::{GameInputs, Interface, Macros, Pack, Post, load_post, read_properties};

pub const VERTEX: &str = "#version 330 compatibility\nvoid main(){gl_Position=ftransform();}\n";

#[derive(Clone, Copy, Debug)]
pub enum Values {
    Float(&'static [f32]),
    Signed(&'static [i32]),
    Unsigned(&'static [u32]),
}

#[derive(Clone, Copy, Debug)]
pub struct Expected {
    pub name: &'static str,
    pub ty: &'static str,
    pub values: Values,
}

#[derive(Clone, Copy, Debug)]
pub struct Fixture {
    pub id: &'static str,
    pub fragment: &'static str,
    pub properties: &'static str,
    pub expected: &'static [Expected],
}

pub const FIXTURES: &[Fixture] = &[
    Fixture {
        id: "scalar",
        fragment: "#version 330\nuniform float fixtureDefault=0.625;out vec4 c;void main(){c=vec4(fixtureDefault);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "float",
            values: Values::Float(&[0.625]),
        }],
    },
    Fixture {
        id: "vector_splat",
        fragment: "#version 330\nuniform vec3 fixtureDefault=vec3(0.375);out vec4 c;void main(){c=vec4(fixtureDefault,1.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "vec3",
            values: Values::Float(&[0.375, 0.375, 0.375]),
        }],
    },
    Fixture {
        id: "vector_constructor_commas",
        fragment: "#version 330\nuniform vec3 fixtureDefault=vec3(0.125,0.375,0.625);out vec4 c;void main(){c=vec4(fixtureDefault,1.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "vec3",
            values: Values::Float(&[0.125, 0.375, 0.625]),
        }],
    },
    Fixture {
        id: "nested_vector_constructor",
        fragment: "#version 330\nuniform vec4 fixtureDefault=vec4(vec2(0.125,0.25),0.5,1.0);out vec4 c;void main(){c=fixtureDefault;}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "vec4",
            values: Values::Float(&[0.125, 0.25, 0.5, 1.0]),
        }],
    },
    Fixture {
        id: "matrix_diagonal",
        fragment: "#version 330\nuniform mat4 fixtureDefault=mat4(2.0);out vec4 c;void main(){c=fixtureDefault*vec4(0.125,0.25,0.375,0.5);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "mat4",
            values: Values::Float(&[
                2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0,
            ]),
        }],
    },
    Fixture {
        id: "matrix_constructor_commas",
        fragment: "#version 330\nuniform mat3 fixtureDefault=mat3(0.125,0.25,0.5,0.75,1.0,1.25,1.5,1.75,2.0);out vec4 c;void main(){c=vec4(fixtureDefault*vec3(1.0),1.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "mat3",
            values: Values::Float(&[0.125, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0]),
        }],
    },
    Fixture {
        id: "boolean_true",
        fragment: "#version 330\nuniform bool fixtureDefault=true;out vec4 c;void main(){c=vec4(fixtureDefault?0.75:0.25);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "bool",
            values: Values::Signed(&[1]),
        }],
    },
    Fixture {
        id: "signed_integer",
        fragment: "#version 330\nuniform int fixtureDefault=-7;out vec4 c;void main(){c=vec4(float(fixtureDefault+8));}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "int",
            values: Values::Signed(&[-7]),
        }],
    },
    Fixture {
        id: "unsigned_integer",
        fragment: "#version 330\nuniform uint fixtureDefault=17u;out vec4 c;void main(){c=vec4(float(fixtureDefault)/32.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "uint",
            values: Values::Unsigned(&[17]),
        }],
    },
    Fixture {
        id: "signed_vector",
        fragment: "#version 330\nuniform ivec3 fixtureDefault=ivec3(-3,5,8);out vec4 c;void main(){c=vec4(vec3(fixtureDefault)/16.0,1.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "ivec3",
            values: Values::Signed(&[-3, 5, 8]),
        }],
    },
    Fixture {
        id: "multiple_declarators",
        fragment: "#version 330\nuniform float fixtureA=0.125,fixtureB=0.75,fixtureC;out vec4 c;void main(){c=vec4(fixtureA,fixtureB,fixtureC,1.0);}",
        properties: "",
        expected: &[
            Expected {
                name: "fixtureA",
                ty: "float",
                values: Values::Float(&[0.125]),
            },
            Expected {
                name: "fixtureB",
                ty: "float",
                values: Values::Float(&[0.75]),
            },
            Expected {
                name: "fixtureC",
                ty: "float",
                values: Values::Float(&[0.0]),
            },
        ],
    },
    Fixture {
        id: "scalar_array",
        fragment: "#version 330\nuniform float fixtureDefault[3]=float[3](0.125,0.375,0.625);out vec4 c;void main(){c=vec4(fixtureDefault[0],fixtureDefault[1],fixtureDefault[2],1.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "float[3]",
            values: Values::Float(&[0.125, 0.375, 0.625]),
        }],
    },
    Fixture {
        id: "vector_array",
        fragment: "#version 330\nuniform vec2 fixtureDefault[2]=vec2[2](vec2(0.125,0.25),vec2(0.5,0.75));out vec4 c;void main(){c=vec4(fixtureDefault[0],fixtureDefault[1]);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "vec2[2]",
            values: Values::Float(&[0.125, 0.25, 0.5, 0.75]),
        }],
    },
    Fixture {
        id: "array_no_initializer",
        fragment: "#version 330\nuniform float fixtureDefault[2];out vec4 c;void main(){c=vec4(fixtureDefault[0],fixtureDefault[1],0.0,1.0);}",
        properties: "",
        expected: &[Expected {
            name: "fixtureDefault",
            ty: "float[2]",
            values: Values::Float(&[0.0, 0.0]),
        }],
    },
    Fixture {
        id: "no_initializers",
        fragment: "#version 330\nuniform float fixtureFloat;uniform vec3 fixtureVector;uniform mat4 fixtureMatrix;uniform bool fixtureBool;uniform int fixtureInt;out vec4 c;void main(){c=vec4(fixtureFloat+fixtureVector.x+fixtureMatrix[0][0]+float(fixtureInt)+(fixtureBool?1.0:0.0));}",
        properties: "",
        expected: &[
            Expected {
                name: "fixtureFloat",
                ty: "float",
                values: Values::Float(&[0.0]),
            },
            Expected {
                name: "fixtureVector",
                ty: "vec3",
                values: Values::Float(&[0.0, 0.0, 0.0]),
            },
            Expected {
                name: "fixtureMatrix",
                ty: "mat4",
                values: Values::Float(&[0.0; 16]),
            },
            Expected {
                name: "fixtureBool",
                ty: "bool",
                values: Values::Signed(&[0]),
            },
            Expected {
                name: "fixtureInt",
                ty: "int",
                values: Values::Signed(&[0]),
            },
        ],
    },
    Fixture {
        id: "known_and_custom_overrides",
        fragment: "#version 330\nuniform float frameTimeCounter=0.125;uniform vec3 cameraPosition=vec3(0.125);uniform float fixtureCustom=0.125;uniform float fixtureNoInit;out vec4 c;void main(){c=vec4(frameTimeCounter+cameraPosition.x+fixtureCustom+fixtureNoInit);}",
        properties: "uniform.float.fixtureCustom=frameTimeCounter*2\n",
        // First frame's explicit provider values are supplied in assert_uniform_cpu_contract.
        expected: &[
            Expected {
                name: "frameTimeCounter",
                ty: "float",
                values: Values::Float(&[0.75]),
            },
            Expected {
                name: "cameraPosition",
                ty: "vec3",
                values: Values::Float(&[0.25, 0.5, 0.75]),
            },
            Expected {
                name: "fixtureCustom",
                ty: "float",
                values: Values::Float(&[1.5]),
            },
            Expected {
                name: "fixtureNoInit",
                ty: "float",
                values: Values::Float(&[0.0]),
            },
        ],
    },
];

pub const RELOAD: Fixture = Fixture {
    id: "same_name_new_default",
    fragment: "#version 330\nuniform float fixtureDefault=0.875;out vec4 c;void main(){c=vec4(fixtureDefault);}",
    properties: "",
    expected: &[Expected {
        name: "fixtureDefault",
        ty: "float",
        values: Values::Float(&[0.875]),
    }],
};

/// Invalid defaults must be rejected with a diagnostic instead of silently becoming zero.
pub const INVALID: &[(&str, &str)] = &[
    (
        "unknown_initializer_name",
        "#version 330\nuniform float fixtureDefault=undefinedDefault;out vec4 c;void main(){c=vec4(fixtureDefault);}",
    ),
    (
        "incompatible_initializer_type",
        "#version 330\nuniform float fixtureDefault=vec3(0.5);out vec4 c;void main(){c=vec4(fixtureDefault);}",
    ),
    (
        "opaque_initializer",
        "#version 330\nuniform sampler2D fixtureDefault=0;out vec4 c;void main(){c=texture(fixtureDefault,vec2(0.5));}",
    ),
];

struct ScratchPack(PathBuf);

impl ScratchPack {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-original-uniform-defaults-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for (name, contents) in files {
            let path = root.join("shaders").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        Self(root)
    }
}

impl Drop for ScratchPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn load_fixture(fixture: Fixture) -> Post {
    let scratch = ScratchPack::new(&[
        ("composite.vsh", VERTEX),
        ("composite.fsh", fixture.fragment),
        ("shaders.properties", fixture.properties),
    ]);
    let pack = Pack::open(&scratch.0).unwrap();
    let mut macros = Macros::standard();
    let (properties, notes) = read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{}: {notes:?}", fixture.id);
    load_post(&pack, "", &properties, &macros, &GameInputs::default())
}

#[test]
fn original_initialized_uniform_declarations_compile() {
    for fixture in FIXTURES.iter().chain(std::iter::once(&RELOAD)) {
        let post = load_fixture(*fixture);
        assert!(post.notes.is_empty(), "{}: {:?}", fixture.id, post.notes);
        assert_eq!(
            post.programs.len(),
            1,
            "{}: program disappeared",
            fixture.id
        );
        assert!(
            post.programs[0].program.is_some(),
            "{}: no draw program",
            fixture.id
        );
    }
}

#[test]
fn initialized_declarators_export_only_the_authored_names_and_types() {
    for fixture in FIXTURES.iter().chain(std::iter::once(&RELOAD)) {
        let post = load_fixture(*fixture);
        let program = post
            .programs
            .first()
            .and_then(|p| p.program.as_ref())
            .unwrap_or_else(|| panic!("{}: {:?}", fixture.id, post.notes));
        let mut actual: Vec<_> = program
            .interface
            .uniforms
            .iter()
            .filter(|u| !u.name.starts_with("hz_"))
            .map(|u| (u.name.as_str(), u.ty.as_str()))
            .collect();
        let mut expected: Vec<_> = fixture.expected.iter().map(|e| (e.name, e.ty)).collect();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            actual, expected,
            "{}: initializer tokens became declarators",
            fixture.id
        );
    }
}

#[test]
fn matching_uniform_initializers_are_shared_across_the_two_stages() {
    let vertex = "#version 330\nuniform float sharedDefault=0.625;out float v;void main(){v=sharedDefault;gl_Position=vec4(0.0,0.0,0.0,1.0);}";
    let fragment = "#version 330\nuniform float sharedDefault=0.625;in float v;out vec4 c;void main(){c=vec4(sharedDefault+v);}";
    let scratch = ScratchPack::new(&[("composite.vsh", vertex), ("composite.fsh", fragment)]);
    let pack = Pack::open(&scratch.0).unwrap();
    let mut macros = Macros::standard();
    let (properties, _) = read_properties(&pack, &mut macros);
    let post = load_post(&pack, "", &properties, &macros, &GameInputs::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let program = post.programs[0].program.as_ref().unwrap();
    assert_eq!(
        program
            .interface
            .uniforms
            .iter()
            .filter(|u| u.name == "sharedDefault")
            .count(),
        1
    );
}

#[test]
fn invalid_uniform_initializers_are_not_silently_ignored() {
    for (id, fragment) in INVALID {
        let fixture = Fixture {
            id,
            fragment,
            properties: "",
            expected: &[],
        };
        let post = load_fixture(fixture);
        assert!(
            post.programs.is_empty(),
            "{id}: invalid initializer was accepted"
        );
        assert!(!post.notes.is_empty(), "{id}: rejection had no diagnostic");
    }
}

#[test]
fn reflected_initializers_seed_the_real_block_with_typed_std140_values() {
    for fixture in FIXTURES
        .iter()
        .filter(|f| f.id != "known_and_custom_overrides")
        .chain(std::iter::once(&RELOAD))
    {
        let mut post = load_fixture(*fixture);
        assert!(post.notes.is_empty(), "{}: {:?}", fixture.id, post.notes);
        let program = post.programs[0].program.take().unwrap();
        let block = program.interface.uniform_block();
        assert_capture(
            fixture.id,
            &Capture {
                interface: program.interface,
                block,
            },
            fixture.expected,
        );
    }
}

#[test]
fn constant_arithmetic_braces_and_unsigned_bits_are_preserved() {
    let fixture = Fixture {
        id: "arithmetic_braces_unsigned",
        fragment: "#version 450\nconst float q=0.25;uniform vec3 arithmetic=vec3(q*2.0,1.0-q,q/2.0);uniform int precedence=1+2*3;uniform uint bits=0x80000011u;uniform vec2 braces[2]={{0.125,0.25},{0.5,0.75}};out vec4 c;void main(){c=vec4(arithmetic,1.0)+vec4(braces[0],braces[1])+vec4(float(precedence)+float(bits));}",
        properties: "",
        expected: &[
            Expected {
                name: "arithmetic",
                ty: "vec3",
                values: Values::Float(&[0.5, 0.75, 0.125]),
            },
            Expected {
                name: "precedence",
                ty: "int",
                values: Values::Signed(&[7]),
            },
            Expected {
                name: "bits",
                ty: "uint",
                values: Values::Unsigned(&[0x80000011]),
            },
            Expected {
                name: "braces",
                ty: "vec2[2]",
                values: Values::Float(&[0.125, 0.25, 0.5, 0.75]),
            },
        ],
    };
    let mut post = load_fixture(fixture);
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let program = post.programs[0].program.take().unwrap();
    let block = program.interface.uniform_block();
    assert_capture(
        fixture.id,
        &Capture {
            interface: program.interface,
            block,
        },
        fixture.expected,
    );
}

#[test]
fn either_linked_stage_can_supply_the_default_and_conflicts_are_rejected() {
    for (vertex_init, fragment_init, valid) in [
        ("=0.625", "", true),
        ("", "=0.625", true),
        ("=0.625", "=0.625", true),
        ("=0.625", "=0.875", false),
    ] {
        let vertex = format!(
            "#version 330\nuniform float sharedDefault{vertex_init};out float v;void main(){{v=sharedDefault;gl_Position=vec4(0,0,0,1);}}"
        );
        let fragment = format!(
            "#version 330\nuniform float sharedDefault{fragment_init};in float v;out vec4 c;void main(){{c=vec4(sharedDefault+v);}}"
        );
        let scratch = ScratchPack::new(&[("composite.vsh", &vertex), ("composite.fsh", &fragment)]);
        let pack = Pack::open(&scratch.0).unwrap();
        let macros = Macros::standard();
        let mut post = load_post(
            &pack,
            "",
            &Default::default(),
            &macros,
            &GameInputs::default(),
        );
        if valid {
            assert!(post.notes.is_empty(), "{:?}", post.notes);
            let program = post.programs[0].program.take().unwrap();
            let block = program.interface.uniform_block();
            assert_capture(
                "linked",
                &Capture {
                    interface: program.interface,
                    block,
                },
                &[Expected {
                    name: "sharedDefault",
                    ty: "float",
                    values: Values::Float(&[0.625]),
                }],
            );
        } else {
            assert!(post.programs.is_empty());
            assert!(
                post.notes
                    .iter()
                    .any(|n| n.contains("different initializers")),
                "{:?}",
                post.notes
            );
        }
    }
}

#[test]
fn compute_uniform_defaults_use_the_same_typed_seed() {
    let scratch = ScratchPack::new(&[(
        "composite.csh",
        "#version 430\nlayout(local_size_x=1) in;uniform vec3 fixtureDefault=vec3(0.125,0.375,0.625);void main(){}",
    )]);
    let pack = Pack::open(&scratch.0).unwrap();
    let source = horizon_shaders::preprocess(&pack, "composite.csh", &Macros::standard());
    let program = horizon_shaders::translate_compute(&source).unwrap_or_else(|e| panic!("{e:?}"));
    let block = program.interface.uniform_block();
    assert_capture(
        "compute",
        &Capture {
            interface: program.interface,
            block,
        },
        &[Expected {
            name: "fixtureDefault",
            ty: "vec3",
            values: Values::Float(&[0.125, 0.375, 0.625]),
        }],
    );
}

/// Snapshot copied from actual production CPU UBO preparation, before GPU submission.
pub struct Capture {
    pub interface: Interface,
    pub block: Vec<u8>,
}

/// Independent std140 check. Locations come from real reflection; values never come
/// from the parser/writer under test. Matrices and arrays have std140 stride.
pub fn assert_capture(id: &str, capture: &Capture, expected: &[Expected]) {
    assert!(
        capture.block.len() >= capture.interface.uniform_size as usize,
        "{id}: short block"
    );
    for field in expected {
        let uniform = capture
            .interface
            .uniforms
            .iter()
            .find(|u| u.name == field.name)
            .unwrap_or_else(|| panic!("{id}: missing {}", field.name));
        assert_eq!(uniform.ty, field.ty, "{id}: {} type", field.name);
        let count = match field.values {
            Values::Float(v) => v.len(),
            Values::Signed(v) => v.len(),
            Values::Unsigned(v) => v.len(),
        };
        for component in 0..count {
            let (base, array) = field
                .ty
                .split_once('[')
                .map_or((field.ty, false), |(base, _)| (base, true));
            let (elements, stride) = match base {
                "mat4" => (16, 64),
                "mat3" => (9, 48),
                "mat2" => (4, 32),
                "vec4" | "ivec4" | "uvec4" => (4, 16),
                "vec3" | "ivec3" | "uvec3" => (3, 16),
                "vec2" | "ivec2" | "uvec2" => (2, 16),
                _ => (1, 16),
            };
            let within = if array {
                component % elements
            } else {
                component
            };
            let matrix_rows = match base {
                "mat4" => 4,
                "mat3" => 3,
                "mat2" => 2,
                _ => 0,
            };
            let relative = if let Some(column) = within.checked_div(matrix_rows) {
                column * 16 + within % matrix_rows * 4
            } else {
                within * 4
            } + if array {
                component / elements * stride
            } else {
                0
            };
            let offset = uniform.offset as usize + relative;
            let bytes: [u8; 4] = capture
                .block
                .get(offset..offset + 4)
                .unwrap_or_else(|| {
                    panic!("{id}: {} component {component} outside block", field.name)
                })
                .try_into()
                .unwrap();
            let expected_bits = match field.values {
                Values::Float(v) => v[component].to_bits(),
                Values::Signed(v) => v[component] as u32,
                Values::Unsigned(v) => v[component],
            };
            assert_eq!(
                u32::from_ne_bytes(bytes),
                expected_bits,
                "{id}: {} component {component}",
                field.name
            );
        }
    }
}
