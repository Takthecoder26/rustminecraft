//! Original, asset-free indirect-dispatch control through the production pack pipeline.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use std::io::{Cursor, Write};
use wgpu::TextureFormat as T;

fn pack(indirect: bool) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let props = format!(
        "bufferObject.0=16\nbufferObject.1=16\n{}",
        if indirect {
            "indirect.composite=0 4\n"
        } else {
            ""
        }
    );
    for (name, source) in [
        ("shaders.properties", props.as_str()),
        (
            "prepare.csh",
            "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=0) buffer Commands { uvec4 args; };\nlayout(std430,binding=1) buffer Results { uint count; };\nvoid main() { args=uvec4(99u,2u,1u,1u); count=0u; }\n",
        ),
        (
            "composite.csh",
            "#version 430\nlayout(local_size_x=1) in;\nconst ivec3 workGroups=ivec3(1);\nlayout(std430,binding=1) buffer Results { uint count; };\nvoid main() { atomicAdd(count,1u); }\n",
        ),
        (
            "final.vsh",
            "#version 120\nvoid main() { gl_Position=ftransform(); }\n",
        ),
        (
            "final.fsh",
            "#version 430 compatibility\nlayout(std430,binding=1) buffer Results { uint count; };\nvoid main() { gl_FragColor=vec4(float(count==1u),float(count==2u),float(count==0u),1.0); }\n",
        ),
    ] {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-indirect-control",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, notes) = horizon_shaders::read_properties(&pack, &mut macros);
    assert!(notes.is_empty(), "{notes:?}");
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

#[test]
#[ignore = "requires a GPU adapter"]
fn indirect_dispatch_uses_ssbo_counts_and_byte_offset() {
    let gpu = Fixture::new();
    for indirect in [false, true] {
        let mut post = gpu.build(pack(indirect));
        for size in [[2, 1], [4, 2]] {
            let actual = gpu.render(&mut post, size);
            let expected = if indirect {
                [0, 255, 0, 255]
            } else {
                [255, 0, 0, 255]
            };
            eprintln!("indirect={indirect} size={size:?}: {actual:?}");
            assert!(
                actual.iter().all(|p| p == &expected),
                "indirect={indirect}: {actual:?}"
            );
        }
    }
}

struct Fixture {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Fixture {
    fn new() -> Self {
        let instance = crate::gfx::test_gpu::instance();
        let adapter =
            pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device for destination contract");
        eprintln!("Indirect dispatch adapter: {:?}", adapter.get_info());
        Self {
            adapter,
            device,
            queue,
        }
    }

    fn texture(&self, size: [u32; 2], format: T) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original indirect dispatch fixture"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
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

    fn build(&self, post: Post) -> PackPost {
        let lightmap = self.texture([1, 1], T::Rgba8Unorm);
        let post = PackPost::new(
            &self.device,
            &self.queue,
            &self.adapter,
            post,
            T::Rgba8Unorm,
            World {
                atlas: None,
                lightmap,
                terrain: None,
                block_ids: &[],
                celestial: None,
                gui_size: [16, 16],
                clouds: None,
                weather: false,
                hand: false,
            },
        );
        assert!(
            post.notes.iter().all(|n| n.starts_with("shadow map ")),
            "{:?}",
            post.notes
        );
        post
    }

    fn render(&self, post: &mut PackPost, size: [u32; 2]) -> Vec<[u8; 4]> {
        let world = self.texture(size, T::Rgba8Unorm);
        let depth = self.texture(size, T::Depth32Float);
        let screen = self.texture(size, T::Rgba8Unorm);
        let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(stride * size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let hud = HudBatch::new(size);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        post.run(
            &self.device,
            &self.queue,
            &mut encoder,
            &world,
            &depth,
            &screen,
            &fixture_frame(&hud),
            &Default::default(),
            size,
            size,
            &[],
            &mut |_, _, _, _| {},
        );
        encoder.copy_texture_to_buffer(
            screen.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    bytes_per_row: Some(stride),
                    ..Default::default()
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
                row[..size[0] as usize * 4]
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
