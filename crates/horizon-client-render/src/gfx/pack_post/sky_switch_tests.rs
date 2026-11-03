//! Original asset-free GPU contract probes, MIT. No player assets or pack implementation.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use wgpu::TextureFormat as T;

const SIZE: [u32; 2] = [48, 8];

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}
impl Gpu {
    fn new() -> Self {
        let instance = crate::gfx::test_gpu::instance();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("GPU adapter required: setup failure is not a contract pass");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
            ..Default::default()
        }))
        .expect("GPU device required: setup failure is not a contract pass");
        eprintln!("ORIGINAL_FIXTURE_ADAPTER {:?}", adapter.get_info());
        Self {
            adapter,
            device,
            queue,
        }
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
    fn load(&self, properties: &str, lightmap: &wgpu::TextureView) -> PackPost {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/packs/sky-switches");
        let pack = horizon_shaders::Pack::open(&path).unwrap();
        let macros = horizon_shaders::Macros::standard();
        let properties = horizon_shaders::Properties::parse(properties, &macros);
        let inputs = horizon_shaders::GameInputs {
            sky: Some(super::super::pack_sky::inputs()),
            celestial: Some(super::super::pack_sky::celestial_inputs()),
            ..Default::default()
        };
        let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
        assert!(post.notes.is_empty(), "{:?}", post.notes);
        assert!(post.sky.is_some() && post.sky_textured.is_some());
        let built = PackPost::new(
            &self.device,
            &self.queue,
            &self.adapter,
            post,
            T::Rgba8Unorm,
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
        );
        assert!(built.notes.is_empty(), "{:?}", built.notes);
        assert!(
            built
                .sky
                .as_ref()
                .is_some_and(|s| s.basic.is_some() && s.textured.is_some())
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
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
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

#[test]
#[ignore = "asset-free GPU sky switch regression; requires a GPU"]
fn sky_feature_switches_control_independent_gpu_stage_markers() {
    let gpu = Gpu::new();
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.0);
    let hud = HudBatch::new(SIZE);
    let celestial = [crate::mesher::Vertex {
        pos: [0.0; 3],
        uv: [0.0; 2],
        layer: 0,
        color: u32::MAX,
        light: 0,
        normal: [0; 4],
        glint_uv: [0.0; 2],
    }; 8];
    let mut frame = fixture_frame(&hud);
    frame.title = false;
    frame.skybox = true;
    frame.sky.sunrise_color = [1.0; 4];
    frame.dark_disc = true;
    frame.stars = Some((glam::Mat4::IDENTITY, 1.0));
    frame.celestial = &celestial;
    let mut failures = Vec::new();
    for (properties, enabled) in [
        ("", [true; 6]),
        ("sky=false", [false, false, true, true, true, false]),
        ("stars=false", [true, true, true, true, false, true]),
        ("sun=false", [true, true, false, true, true, true]),
        ("moon=false", [true, true, true, false, true, true]),
        ("sky=false\nstars=false\nsun=false\nmoon=false", [false; 6]),
        ("sky=true\nstars=true\nsun=true\nmoon=true", [true; 6]),
    ] {
        let mut post = gpu.load(
            &format!("blend.gbuffers_skybasic=off\nblend.gbuffers_skytextured=off\n{properties}"),
            &color,
        );
        for empty in [false, true, false] {
            let (stars, celestial, dark, sunrise) = (
                frame.stars,
                frame.celestial,
                frame.dark_disc,
                frame.sky.sunrise_color,
            );
            if empty {
                frame.stars = None;
                frame.celestial = &[];
                frame.dark_disc = false;
                frame.sky.sunrise_color[3] = 0.0;
            }
            let pixels = gpu.render(&mut post, &color, &depth, &frame, &mut |_, _, _, _| {});
            let expected = if empty {
                [enabled[0], false, false, false, false, false]
            } else {
                enabled
            };
            let mut wrong = 0;
            for (i, pixel) in pixels.iter().enumerate() {
                let on = expected[(i % SIZE[0] as usize) / 8];
                if pixel[..3] != if on { [255; 3] } else { [0; 3] } {
                    wrong += 1;
                }
            }
            eprintln!(
                "SKY_SWITCH_GPU properties={properties:?} empty={empty} wrong={wrong}/{}",
                pixels.len()
            );
            if wrong != 0 {
                failures.push((properties, empty, wrong));
            }
            (
                frame.stars,
                frame.celestial,
                frame.dark_disc,
                frame.sky.sunrise_color,
            ) = (stars, celestial, dark, sunrise);
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
