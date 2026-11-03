//! Original weather-depth GPU regressions from public format documentation; no pack assets.
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
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
            ..Default::default()
        }))
        .expect("GPU device required: setup failure is not a contract pass");
        eprintln!("DEPTH_FIXTURE_ADAPTER {:?}", adapter.get_info());
        Some(Self {
            adapter,
            device,
            queue,
        })
    }
    fn texture(&self, format: T) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original depth fixture input"),
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
    fn load(
        &self,
        files: Vec<(String, String)>,
        world: World<'_>,
        inputs: horizon_shaders::GameInputs,
    ) -> PackPost {
        use std::io::{Cursor, Write};
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, contents) in files {
            zip.start_file(
                format!("shaders/{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(contents.as_bytes()).unwrap();
        }
        let pack = horizon_shaders::Pack::from_zip_bytes(
            "original-depth-fixture",
            zip.finish().unwrap().into_inner(),
        )
        .unwrap();
        let mut macros = horizon_shaders::Macros::standard();
        let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
        let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
        assert!(
            post.programs.iter().any(|p| p.name == "final"),
            "{:?}",
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
        assert!(
            built.passes.iter().any(|p| p.group == Group::Final),
            "{:?}",
            built.notes
        );
        built
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

fn entity_routes(specialized: bool, ordering: &str, deferred: bool) {
    let Some(gpu) = Gpu::new() else {
        eprintln!("STAGE_ROUTES_NO_ADAPTER");
        return;
    };
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let mut files=vec![
        ("shaders.properties".into(), format!("separateEntityDraws=true\nparticles.ordering={ordering}\n")),
        ("final.vsh".into(), "#version 120\nvarying vec2 uv;void main(){uv=gl_MultiTexCoord0.xy;gl_Position=ftransform();}".into()),
        ("final.fsh".into(), "#version 120\nuniform sampler2D colortex0;varying vec2 uv;void main(){gl_FragColor=texture2D(colortex0,uv);}".into())
    ];
    if deferred {
        files.push(("deferred.vsh".into(), "#version 120\nvarying vec2 uv;void main(){uv=gl_MultiTexCoord0.xy;gl_Position=ftransform();}".into()));
        files.push(("deferred.fsh".into(), "#version 120\n/* DRAWBUFFERS:0 */\nuniform sampler2D colortex0;varying vec2 uv;void main(){gl_FragData[0]=vec4(texture2D(colortex0,uv).rgb*0.5,1.0);}".into()));
    }
    let names = [
        ("gbuffers_block", [1, 0, 0]),
        ("gbuffers_block_translucent", [0, 1, 0]),
        ("gbuffers_particles", [0, 0, 1]),
        ("gbuffers_particles_translucent", [1, 1, 0]),
    ];
    for (i, (name, rgb)) in names.iter().enumerate() {
        if !specialized && i % 2 == 1 {
            continue;
        }
        files.push((
            format!("{name}.vsh"),
            "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}".into(),
        ));
        files.push((format!("{name}.fsh"), format!("#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){{gl_FragData[0]=vec4({}.0,{}.0,{}.0,1.0);}}",rgb[0],rgb[1],rgb[2])));
    }
    let mut post = gpu.load(
        files,
        World {
            atlas: None,
            lightmap: color.clone(),
            terrain: None,
            block_ids: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
        horizon_shaders::GameInputs {
            entities: Some(super::super::pack_entities::inputs()),
            ..Default::default()
        },
    );
    let mut vertices = Vec::new();
    let mut spans = Vec::new();
    for i in 0..4 {
        if i % 2 == 0 {
            spans.push(super::super::pack_draw::EntitySpan {
                start: vertices.len() as u32,
                stage: horizon_shaders::render_stage(if i < 2 {
                    "BLOCK_ENTITIES"
                } else {
                    "PARTICLES"
                }),
                translucent: false,
                id: 17,
                ..Default::default()
            });
        }
        let left = -1.0 + i as f32 * 0.5;
        let right = left + 0.5;
        vertices.extend(
            [
                [left, -1., 0.],
                [right, -1., 0.],
                [right, 1., 0.],
                [left, 1., 0.],
            ]
            .map(|pos| super::super::Vertex {
                pos,
                uv: [0.; 2],
                layer: match i {
                    1 => super::super::entity_batch::OIT_ENTITY,
                    3 => super::super::TRANSLUCENT_PARTICLE_OVERLAY,
                    _ => 0,
                },
                color: u32::MAX,
                light: 0,
                normal: [0, 0, 127, 0],
                glint_uv: [0.; 2],
            }),
        );
    }
    let mut rest = Vec::new();
    post.prepare_entities(
        &gpu.device,
        &gpu.queue,
        &vertices,
        &spans,
        [0.; 4],
        &mut rest,
    );
    assert!(
        rest.is_empty(),
        "all routes should have a program or fallback"
    );
    let hud = HudBatch::new(SIZE);
    let pixels = gpu.render(
        &mut post,
        &color,
        &depth,
        &fixture_frame(&hud),
        &mut |_, _, _, _| {},
    );
    // Deferred halves the colors drawn before it. Later geometry retains full RGB.
    let opaque = if deferred { 128 } else { 255 };
    let particle = if deferred && ordering != "after" {
        128
    } else {
        255
    };
    let translucent_particle = if deferred && ordering == "before" {
        128
    } else {
        255
    };
    let expected = [
        [opaque, 0, 0, 255],
        if specialized {
            [0, 255, 0, 255]
        } else {
            [255, 0, 0, 255]
        },
        [0, 0, particle, 255],
        if specialized && ordering == "mixed" {
            [255, 255, 0, 255]
        } else {
            [0, 0, translucent_particle, 255]
        },
    ];
    let wrong = pixels
        .iter()
        .enumerate()
        .filter(|(i, p)| {
            p.iter()
                .zip(expected[(i % 16) / 4])
                .any(|(actual, want)| actual.abs_diff(want) > 1)
        })
        .count();
    eprintln!(
        "STAGE_ROUTES specialized={specialized} wrong={wrong}/{} samples={:?}",
        pixels.len(),
        [pixels[2], pixels[6], pixels[10], pixels[14]]
    );
    assert_eq!(wrong, 0, "independent constant RGB per geometry category");
}
#[test]
fn block_particle_translucent_routes_are_distinct() {
    entity_routes(true, "mixed", false);
}
#[test]
fn block_particle_routes_keep_base_fallbacks() {
    entity_routes(false, "mixed", false);
}

#[test]
fn mixed_particles_straddle_deferred() {
    entity_routes(true, "mixed", true);
}
#[test]
fn before_particles_use_base_program_before_deferred() {
    entity_routes(true, "before", true);
}
#[test]
fn after_particles_use_base_program_after_deferred() {
    entity_routes(true, "after", true);
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
