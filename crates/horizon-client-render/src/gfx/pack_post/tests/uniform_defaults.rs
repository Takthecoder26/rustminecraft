//! Original CPU captures of the production seed, provider resolution, custom evaluation and
//! block-write functions used immediately before queue.write_buffer. No GPU or fake writer.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch() -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "horizon-default-provider-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn load(default: &str) -> (horizon_shaders::Interface, horizon_shaders::Custom) {
    let root = scratch();
    let result = load_at(&root, default);
    std::fs::remove_dir_all(root).unwrap();
    result
}

fn load_at(
    root: &std::path::Path,
    default: &str,
) -> (horizon_shaders::Interface, horizon_shaders::Custom) {
    let dir = root.join("shaders");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("composite.vsh"),
        "#version 330 compatibility\nvoid main(){gl_Position=ftransform();}",
    )
    .unwrap();
    std::fs::write(dir.join("composite.fsh"),format!("#version 330\nuniform float frameTimeCounter=0.125;uniform vec3 cameraPosition=vec3(0.125);uniform float fixtureCustom=0.125;uniform float fixtureDefault={default};uniform float fixtureNoInit;out vec4 c;void main(){{c=vec4(frameTimeCounter+cameraPosition.x+fixtureCustom+fixtureDefault+fixtureNoInit);}}")).unwrap();
    // Even an identically named custom property must not replace the engine camera.
    std::fs::write(
        dir.join("shaders.properties"),
        "uniform.float.fixtureCustom=frameTimeCounter*2\nuniform.float.cameraPosition=0.875\n",
    )
    .unwrap();
    let pack = horizon_shaders::Pack::open(root).unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, notes) = horizon_shaders::read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let mut post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    (
        post.programs[0].program.take().unwrap().interface,
        post.custom,
    )
}

fn value(v: &[f32]) -> pack_uniforms::Value {
    let mut data = [0.0; 16];
    data[..v.len()].copy_from_slice(v);
    pack_uniforms::Value::F(data)
}

fn read(interface: &horizon_shaders::Interface, block: &[u8], name: &str, n: usize) -> Vec<f32> {
    let offset = interface
        .uniforms
        .iter()
        .find(|u| u.name == name)
        .unwrap()
        .offset as usize;
    (0..n)
        .map(|i| {
            f32::from_ne_bytes(
                block[offset + i * 4..offset + i * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        })
        .collect()
}

#[test]
fn supplied_engine_and_custom_values_win_while_unknown_defaults_survive_each_frame() {
    let (interface, mut custom) = load("0.625");
    let names: Vec<_> = custom.uniforms().map(|(n, _)| n.to_owned()).collect();
    let fields = uniform_fields(&interface, &names);
    let mut block = interface.uniform_block();
    assert_eq!(read(&interface, &block, "fixtureDefault", 1), [0.625]);
    assert_eq!(read(&interface, &block, "fixtureCustom", 1), [0.125]);
    assert_eq!(read(&interface, &block, "cameraPosition", 3), [0.125; 3]);
    for (time, camera) in [(0.75, [0.25, 0.5, 0.75]), (0.25, [-0.5, 0.125, 0.875])] {
        let mut frame = FrameUniforms::default();
        frame.values = vec![value(&[0.0]); pack_uniforms::NAMES.len()];
        frame.values[pack_uniforms::index("frameTimeCounter").unwrap()] = value(&[time]);
        frame.values[pack_uniforms::index("cameraPosition").unwrap()] = value(&camera);
        frame.eval_custom(&mut custom);
        write_uniform_values(&mut block, &fields, &frame.values);
        assert_eq!(read(&interface, &block, "frameTimeCounter", 1), [time]);
        assert_eq!(read(&interface, &block, "cameraPosition", 3), camera);
        assert_eq!(read(&interface, &block, "fixtureCustom", 1), [time * 2.0]);
        assert_eq!(read(&interface, &block, "fixtureDefault", 1), [0.625]);
        assert_eq!(read(&interface, &block, "fixtureNoInit", 1), [0.0]);
    }
}

#[test]
fn same_shape_source_reload_gets_new_defaults_and_keeps_other_programs_independent() {
    let root = scratch();
    let (first, _) = load_at(&root, "0.625");
    let old_block = first.uniform_block();
    let (replacement, _) = load_at(&root, "0.875");
    let replacement_block = replacement.uniform_block();
    assert_eq!(first.uniform_size, replacement.uniform_size);
    assert_eq!(
        read(&replacement, &replacement_block, "fixtureDefault", 1),
        [0.875]
    );
    assert_eq!(read(&first, &old_block, "fixtureDefault", 1), [0.625]);
    let (again, _) = load_at(&root, "0.625");
    assert_eq!(
        read(&again, &again.uniform_block(), "fixtureDefault", 1),
        [0.625]
    );
    assert_eq!(
        read(&again, &again.uniform_block(), "fixtureNoInit", 1),
        [0.0]
    );
    std::fs::remove_dir_all(root).unwrap();
}
