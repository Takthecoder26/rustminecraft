//! Original weather-depth GPU regressions from public format documentation; no pack assets.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use wgpu::{TextureFormat as T, util::DeviceExt};

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

fn all_pixels(pixels: &[[u8; 4]], expected: [u8; 4], name: &str) {
    let wrong = pixels
        .iter()
        .filter(|p| p.iter().zip(expected).any(|(a, b)| a.abs_diff(b) > 1))
        .count();
    eprintln!(
        "DEPTH_FIXTURE_RESULT {name} expected={expected:?} actual_first={:?} wrong={wrong}/{}",
        pixels[0],
        pixels.len()
    );
    assert_eq!(wrong, 0, "{name}: independent expected pixels {expected:?}");
}

fn weather_depth(property: &str, writes: bool) {
    weather_depth_kind(property, writes, false);
}

fn weather_depth_kind(property: &str, writes: bool, snow: bool) {
    let Some(gpu) = Gpu::new() else {
        eprintln!("WEATHER_DEPTH_NO_ADAPTER");
        return;
    };
    let color = gpu.texture(T::Rgba8Unorm);
    let depth = gpu.texture(T::Depth32Float);
    gpu.clear(&color, &depth, 0.3); // reverse .3 is GL window depth .7.
    let vertex = "#version 120\nvoid main(){gl_Position=vec4(gl_Vertex.xy,-0.8,1.0);}";
    let fragment = "#version 120\n/* DRAWBUFFERS:0 */\nvoid main(){gl_FragData[0]=vec4(1.0);}";
    let mut post = gpu.load(vec![
        ("gbuffers_weather.vsh".into(), vertex.into()),
        ("gbuffers_weather.fsh".into(), fragment.into()),
        // An opaque program ensures snapshots are taken even on main before #7831.
        ("gbuffers_terrain.vsh".into(), vertex.into()),
        ("gbuffers_terrain.fsh".into(), fragment.into()),
        ("shaders.properties".into(), property.into()),
        ("final.vsh".into(), "#version 120\nvarying vec2 uv;void main(){uv=gl_MultiTexCoord0.xy;gl_Position=ftransform();}".into()),
        ("final.fsh".into(), "#version 120\nuniform sampler2D depthtex0;uniform sampler2D depthtex1;uniform sampler2D depthtex2;varying vec2 uv;void main(){gl_FragColor=vec4(texture2D(depthtex0,uv).r,texture2D(depthtex1,uv).r,texture2D(depthtex2,uv).r,1.0);}".into()),
    ], World {
        atlas: None, lightmap: color.clone(),
        terrain: Some(Geometry { buffers: &super::super::pack_entities::buffers(), table: None, depth_write: true, atlas: None, ids: None, cull: false }),
        block_ids: &[], celestial: None, gui_size: SIZE, clouds: None, weather: true, hand: false,
    }, horizon_shaders::GameInputs {
        terrain: Some(super::super::pack_weather::inputs()),
        weather: Some(super::super::pack_weather::inputs()),
        ..Default::default()
    });
    assert!(
        post.weather.is_some() && post.terrain.is_some(),
        "{:?}",
        post.notes
    );
    let vertices = [[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.], [-1., 1., 0.]].map(|pos| {
        super::super::Vertex {
            pos,
            uv: [0.; 2],
            layer: 0,
            color: u32::MAX,
            light: 0,
            normal: [0, 0, 127, 0],
            glint_uv: [0.; 2],
        }
    });
    let vertices = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Original rain quad"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
    let indices = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Original rain indices"),
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        });
    post.set_weather(Some(Weather {
        vertices,
        indices,
        quads: if snow { [0..0, 0..1] } else { [0..1, 1..1] },
        sprites: [None, None],
        generation: 1,
    }));
    let hud = HudBatch::new(SIZE);
    let pixels = gpu.render(
        &mut post,
        &color,
        &depth,
        &fixture_frame(&hud),
        &mut |_, _, _, _| {},
    );
    all_pixels(
        &pixels,
        [if writes { 26 } else { 179 }, 179, 179, 255],
        property,
    );
}

#[test]
fn rain_depth_defaults_to_no_writes() {
    weather_depth("", false);
}
#[test]
fn rain_depth_false_preserves_snapshots() {
    weather_depth("rain.depth=false", false);
}
#[test]
fn rain_depth_invalid_preserves_default() {
    weather_depth("rain.depth=invalid", false);
}
#[test]
fn rain_depth_true_changes_only_depth0() {
    weather_depth("rain.depth=true", true);
}

#[test]
fn snow_depth_defaults_to_no_writes() {
    weather_depth_kind("", false, true);
}
#[test]
fn snow_depth_true_changes_only_depth0() {
    weather_depth_kind("rain.depth=true", true, true);
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
