//! Original asset-free GPU contract probes, MIT. No player assets or pack implementation.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use wgpu::TextureFormat as T;

const SIZE: [u32; 2] = [16, 16];

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}
impl Gpu {
    fn new() -> Option<Self> {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
            Ok(adapter) => adapter,
            Err(e) => {
                assert!(
                    std::env::var_os("HORIZON_REQUIRE_GPU_TESTS").is_none(),
                    "required GPU adapter: {e}"
                );
                eprintln!("SKIPPED original uniform GPU contract: no adapter: {e}");
                return None;
            }
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
            ..Default::default()
        }))
        .expect("GPU device required: setup failure is not a contract pass");
        eprintln!("ORIGINAL_FIXTURE_ADAPTER {:?}", adapter.get_info());
        Some(Self {
            adapter,
            device,
            queue,
        })
    }
    fn texture(&self, format: T) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original translator contract input"),
                size: wgpu::Extent3d {
                    width: SIZE[0],
                    height: SIZE[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    fn load(&self, name: &str, world: World<'_>, inputs: horizon_shaders::GameInputs) -> PackPost {
        let root = original_pack(name);
        let pack = horizon_shaders::Pack::open(&root.0).expect("original fixture readable");
        let mut macros = horizon_shaders::Macros::standard();
        let (properties, feature_notes) = horizon_shaders::read_properties(&pack, &mut macros);
        eprintln!("ORIGINAL_FIXTURE_PROPERTIES {name} {feature_notes:?}");
        let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
        eprintln!(
            "ORIGINAL_FIXTURE_LOAD {name} programs={:?} notes={:?}",
            post.programs.iter().map(|p| &p.name).collect::<Vec<_>>(),
            post.notes
        );
        assert!(
            post.programs.iter().any(|p| p.name == "final"),
            "final shader must compile; notes={:?}",
            post.notes
        );
        let built = PackPost::new(
            &self.device,
            &self.queue,
            &self.adapter,
            post,
            T::Rgba8Unorm,
            world,
        );
        eprintln!("ORIGINAL_FIXTURE_PIPELINES {name} {:?}", built.notes);
        assert!(
            built.passes.iter().any(|p| p.group == Group::Final),
            "final GPU pipeline required: {:?}",
            built.notes
        );
        built
    }
    fn plain(
        &self,
        name: &str,
        lightmap: &wgpu::TextureView,
        inputs: horizon_shaders::GameInputs,
    ) -> PackPost {
        self.load(
            name,
            World {
                atlas: None,
                lightmap: lightmap.clone(),
                terrain: None,
                block_ids: &[],
                celestial: None,
                gui_size: SIZE,
                clouds: None,
                weather: false,
                hand: false,
            },
            inputs,
        )
    }
    fn clear(&self, color: &wgpu::TextureView, depth: &wgpu::TextureView, reverse_depth: f32) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Independent original fixture clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0625,
                        g: 0.0625,
                        b: 0.0625,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(reverse_depth),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        self.queue.submit([encoder.finish()]);
    }
    fn render(
        &self,
        post: &mut PackPost,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame: &Frame<'_>,
        terrain: &mut TerrainDraw<'_>,
    ) -> Vec<[u8; 4]> {
        let screen = self.texture(T::Rgba8Unorm);
        let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Original fixture readback"),
            size: u64::from(stride * SIZE[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        post.run(
            &self.device,
            &self.queue,
            &mut encoder,
            color,
            depth,
            &screen,
            frame,
            &Default::default(),
            SIZE,
            SIZE,
            &[],
            terrain,
        );
        encoder.copy_texture_to_buffer(
            screen.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            screen.texture().size(),
        );
        self.queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        read.slice(..)
            .get_mapped_range()
            .unwrap()
            .chunks_exact(stride as usize)
            .flat_map(|row| {
                row[..SIZE[0] as usize * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .copied()
            })
            .collect()
    }
}

fn all_pixels(pixels: &[[u8; 4]], expected: [u8; 4], name: &str) {
    let wrong = pixels
        .iter()
        .filter(|p| p.iter().zip(expected).any(|(a, b)| a.abs_diff(b) > 1))
        .count();
    eprintln!(
        "ORIGINAL_FIXTURE_RESULT {name} expected={expected:?} actual_first={:?} wrong={wrong}/{}",
        pixels[0],
        pixels.len()
    );
    assert_eq!(wrong, 0, "{name}: independent expected pixels {expected:?}");
}

struct OriginalPack(std::path::PathBuf);
impl Drop for OriginalPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn original_pack(name: &str) -> OriginalPack {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let (vertex, fragment, properties) = match name {
        "uniform_scalar_default" => (
            r###"#version 120
varying vec2 probeUV;
void main() {
    probeUV = gl_MultiTexCoord0.xy;
    gl_Position = ftransform();
}
"###,
            r###"#version 120
uniform float fixtureDefault = 0.625;
uniform float noInitializer;
void main() { gl_FragColor = vec4(fixtureDefault, noInitializer, 0.25, 1.0); }
"###,
            r###""###,
        ),
        "uniform_vector_default" => (
            r###"#version 120
varying vec2 probeUV;
void main() {
    probeUV = gl_MultiTexCoord0.xy;
    gl_Position = ftransform();
}
"###,
            r###"#version 120
uniform vec3 fixtureDefault = vec3(0.125, 0.375, 0.625);
void main() { gl_FragColor = vec4(fixtureDefault, 1.0); }
"###,
            r###""###,
        ),
        "uniform_recognized_override" => (
            r###"#version 120
varying vec2 probeUV;
void main() {
    probeUV = gl_MultiTexCoord0.xy;
    gl_Position = ftransform();
}
"###,
            r###"#version 120
uniform vec3 cameraPosition = vec3(0.75);
void main() { gl_FragColor = vec4(cameraPosition, 1.0); }
"###,
            r###""###,
        ),
        "uniform_scalar_reload" => (
            r###"#version 120
varying vec2 probeUV;
void main() {
    probeUV = gl_MultiTexCoord0.xy;
    gl_Position = ftransform();
}
"###,
            r###"#version 120
uniform float fixtureDefault = 0.875;
uniform float noInitializer;
void main() { gl_FragColor = vec4(fixtureDefault, noInitializer, 0.25, 1.0); }
"###,
            r###""###,
        ),
        "uniform_typed_bands" => (
            r###"#version 330 compatibility
void main(){gl_Position=ftransform();}
"###,
            r###"#version 330 compatibility
const float authoredQuarter=0.25;
uniform float scalarDefault=0.625, noInitializer;
uniform vec3 vectorDefault=vec3(0.125,0.375,0.625);
uniform mat3 matrixDefault=mat3(1.0,2.0,3.0,4.0,5.0,6.0,7.0,8.0,9.0)/16.0;
uniform bool boolDefault=true;
uniform int signedDefault=-7;
uniform uint unsignedDefault=0x80000011u;
uniform float scalarArray[3]=float[3](0.125,0.375,0.625);
uniform vec2 vectorArray[2]=vec2[2](vec2(0.125,0.25),vec2(0.5,0.75));
uniform float firstDefault=0.125, secondDefault=0.75, lastNoInitializer;
uniform vec3 cameraPosition=vec3(0.75);
uniform float customDefault=0.125, independentDefault=0.875;
void main(){
    int band=int(gl_FragCoord.x)/2;
    if(band==0) gl_FragColor=vec4(scalarDefault,noInitializer,authoredQuarter,1.0);
    else if(band==1) gl_FragColor=vec4(vectorDefault,1.0);
    else if(band==2) gl_FragColor=vec4(matrixDefault[0][1],matrixDefault[1][2],matrixDefault[2][1],1.0);
    else if(band==3) gl_FragColor=vec4(boolDefault?0.75:0.0,float(signedDefault+8)/4.0,float(unsignedDefault&255u)/255.0,unsignedDefault>=0x80000000u?1.0:0.5);
    else if(band==4) gl_FragColor=vec4(scalarArray[0],scalarArray[1],scalarArray[2],1.0);
    else if(band==5) gl_FragColor=vec4(vectorArray[0],vectorArray[1]);
    else if(band==6) gl_FragColor=vec4(firstDefault,secondDefault,lastNoInitializer,1.0);
    else gl_FragColor=vec4(cameraPosition.x,customDefault,independentDefault,1.0);
}
"###,
            r###"uniform.float.customDefault=cameraPosition.x*2
"###,
        ),
        "uniform_compute_default" => (
            "#version 330 compatibility\nvoid main(){gl_Position=ftransform();}",
            "#version 430\nuniform sampler2D colortex1;out vec4 c;void main(){c=texelFetch(colortex1,ivec2(gl_FragCoord.xy),0);}",
            "",
        ),
        _ => panic!("unknown original fixture: {name}"),
    };
    let root = std::env::temp_dir().join(format!(
        "horizon-original-uniform-gpu-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let shaders = root.join("shaders");
    std::fs::create_dir_all(&shaders).unwrap();
    std::fs::write(shaders.join("final.vsh"), vertex).unwrap();
    std::fs::write(shaders.join("final.fsh"), fragment).unwrap();
    std::fs::write(shaders.join("shaders.properties"), properties).unwrap();
    if name == "uniform_compute_default" {
        std::fs::write(shaders.join("final.csh"), "#version 430\nlayout(local_size_x=1,local_size_y=1,local_size_z=1) in;const ivec3 workGroups=ivec3(16,16,1);layout(rgba8) writeonly uniform image2D colorimg1;uniform vec4 fixtureDefault=vec4(0.125,0.375,0.625,1.0);void main(){imageStore(colorimg1,ivec2(gl_GlobalInvocationID.xy),fixtureDefault);}").unwrap();
    }
    OriginalPack(root)
}
fn uniform_case(name: &str, expected: [u8; 4]) {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.3);
    let mut post = gpu.plain(name, &color, Default::default());
    let hud = HudBatch::new(SIZE);
    let mut f = fixture_frame(&hud);
    f.camera = glam::DVec3::new(0.25, 0.5, 0.75);
    all_pixels(
        &gpu.render(&mut post, &color, &depth, &f, &mut |_, _, _, _| {}),
        expected,
        name,
    );
}

#[test]
fn custom_scalar_uniform_keeps_its_initializer_and_uninitialized_control_is_zero() {
    uniform_case("uniform_scalar_default", [159, 0, 64, 255]);
}
#[test]
fn custom_vector_uniform_keeps_its_constructor_initializer() {
    uniform_case("uniform_vector_default", [32, 96, 159, 255]);
}
#[test]
fn recognized_uniform_uses_frame_value_instead_of_initializer() {
    uniform_case("uniform_recognized_override", [64, 128, 191, 255]);
}

#[test]
fn compute_default_uniforms_reach_write_only_storage_images() {
    uniform_case("uniform_compute_default", [32, 96, 159, 255]);
}

#[test]
fn typed_defaults_reach_gpu_with_dynamic_custom_precedence() {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.3);
    let mut post = gpu.plain("uniform_typed_bands", &color, Default::default());
    let hud = HudBatch::new(SIZE);
    let mut frame = fixture_frame(&hud);
    for (camera, last) in [(0.25, [64, 128, 223, 255]), (0.5, [128, 255, 223, 255])] {
        frame.camera = glam::DVec3::new(camera, 0.5, 0.75);
        let pixels = gpu.render(&mut post, &color, &depth, &frame, &mut |_, _, _, _| {});
        let bands = [
            [159, 0, 64, 255],
            [32, 96, 159, 255],
            [32, 96, 128, 255],
            [191, 64, 17, 255],
            [32, 96, 159, 255],
            [32, 64, 128, 191],
            [32, 191, 0, 255],
            last,
        ];
        for (n, pixel) in pixels.iter().enumerate() {
            let expected = bands[(n % SIZE[0] as usize) / 2];
            assert!(
                pixel.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                "camera={camera} pixel={n}: {pixel:?} != {expected:?}"
            );
        }
        eprintln!(
            "ORIGINAL_TYPED_UNIFORM_RESULT camera={camera} pixels={} wrong=0",
            pixels.len()
        );
    }
}

#[test]
fn independent_programs_with_the_same_uniform_shape_receive_their_own_defaults_on_gpu() {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.3);
    let mut first = gpu.plain("uniform_scalar_default", &color, Default::default());
    let hud = HudBatch::new(SIZE);
    let frame = fixture_frame(&hud);
    all_pixels(
        &gpu.render(&mut first, &color, &depth, &frame, &mut |_, _, _, _| {}),
        [159, 0, 64, 255],
        "first original default",
    );
    let mut replacement = gpu.plain("uniform_scalar_reload", &color, Default::default());
    all_pixels(
        &gpu.render(
            &mut replacement,
            &color,
            &depth,
            &frame,
            &mut |_, _, _, _| {},
        ),
        [223, 0, 64, 255],
        "same-shape replacement default",
    );
    all_pixels(
        &gpu.render(&mut first, &color, &depth, &frame, &mut |_, _, _, _| {}),
        [159, 0, 64, 255],
        "independent original still retains default",
    );
}

// Frame literal adapted from MIT Horizon f700231 pack_uniforms.rs test API setup.
fn fixture_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        eye: glam::DVec3::ZERO,
        eye_rotation: [0.0; 2],
        first_person: true,
        shadow_scene_revision: (0, 0),
        shadow_cache: false,
        water_plane: None,
        weather: &[],
        weather_time: crate::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_degrees: 90.0,
        bob: glam::Mat4::IDENTITY,
        spin: glam::Mat4::IDENTITY,
        sky: crate::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: true,
        sky_blocked: false,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        outline_high_contrast: false,
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        glint_alpha: 0.75,
        glint_ticks: 0.0,
        nether_lighting: false,
        entities: &[],
        static_block_entities: &[],
        visible_static_block_entity_sections: &[],
        static_block_entity_stats: crate::block_entity_render::StaticCacheFrameStats::default(),
        entity_motion: &[],
        colored_lights: &[],
        emissive_layers: &[],
        shadow_only: &[],
        entity_spans: &[],
        skin: None,
        glowing: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        celestial: &[],
        stars: None,
        hand: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        menu_blur: None,
        chunk_fade: 0.0,
        post_effects: crate::post_chain::Request::none(),
    }
}
