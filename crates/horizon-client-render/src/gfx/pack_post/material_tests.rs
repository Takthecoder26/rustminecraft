//! Original encoded textures exercise atlas copies, sampler selection and production post bindings.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use std::io::{Cursor, Write};
use wgpu::{TextureFormat as T, util::DeviceExt};

#[test]
fn terrain_materials_keep_exact_encoded_values_across_neighbour_edges_and_banks() {
    material_pixels(true, false);
}

#[test]
fn absent_material_maps_keep_flat_normal_and_rough_specular_defaults() {
    material_pixels(false, false);
}

#[test]
fn a_geometry_atlas_override_keeps_terrain_materials_out_of_that_pass() {
    material_pixels(true, true);
}

fn material_pixels(with_maps: bool, other_atlas: bool) {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no adapter for material pixel regression");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    })) else {
        eprintln!("no device for material pixel regression");
        return;
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    let map = |pixels: [[u8; 4]; 2], encoder: &mut wgpu::CommandEncoder| {
        // Two one-layer banks deliberately exercise the same split as terrain colour.
        let arrays: Vec<_> = pixels
            .into_iter()
            .map(|pixel| {
                device.create_texture_with_data(
                    &queue,
                    &wgpu::TextureDescriptor {
                        label: Some("Original encoded material layer"),
                        size: wgpu::Extent3d {
                            width: 16,
                            height: 16,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 5,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: T::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &super::super::box_mip_chain(&pixel.repeat(256), 16, 1),
                )
            })
            .collect();
        super::super::pack_atlas::PackAtlas::new(&device, encoder, &arrays, 1, 2, 16).unwrap()
    };
    let normal = map([[64, 192, 211, 87], [192, 64, 255, 255]], &mut encoder);
    let specular = map([[52, 231, 100, 254], [9, 16, 0, 255]], &mut encoder);
    assert_eq!(normal.size(), [32, 16]);
    assert_eq!(specular.size(), normal.size());
    queue.submit([encoder.finish()]);
    let size = [8, 2];
    let colour = texture(&device, size, T::Rgba8Unorm);
    let depth = texture(&device, size, T::Depth32Float);
    clear(&device, &queue, &colour, &depth, [0, 0, 0, 255], 0.0);
    let mut post = pack(
        &device,
        &queue,
        &adapter,
        &colour,
        with_maps.then_some([normal.view, specular.view]),
    );
    if other_atlas {
        // Entity/hand/weather geometry supplies its own colour atlas. Its maps are still defaults.
        post.terrain.as_mut().unwrap().atlas = Some(colour.clone());
    }
    let hud = HudBatch::new(size);
    let actual = render(
        &device,
        &queue,
        &mut post,
        &colour,
        &depth,
        &frame(&hud),
        size,
    );
    for (i, pixel) in actual.into_iter().enumerate() {
        let expected = if !with_maps || other_atlas {
            [128, 0, 255, 0]
        } else if i % 8 < 4 {
            [64, 231, 87, 254]
        } else {
            [192, 16, 255, 255]
        };
        assert_eq!(
            pixel, expected,
            "pixel {i}; encoded maps must select the colour texel"
        );
    }
}

fn pack(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    adapter: &wgpu::Adapter,
    lightmap: &wgpu::TextureView,
    materials: Option<[wgpu::TextureView; 2]>,
) -> PackPost {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in [
        (
            "gbuffers_terrain.vsh",
            "#version 130\nvarying vec2 uv;void main(){vec2 p=vec2(float((gl_VertexID<<1)&2),float(gl_VertexID&2));uv=p;gl_Position=vec4(p*2.0-1.0,0.5,1.0);}",
        ),
        (
            "gbuffers_terrain.fsh",
            "#version 120\nvarying vec2 uv;uniform sampler2D normals;uniform sampler2D specular;void main(){vec2 p=vec2(0.5+(uv.x-0.5)*0.02,0.5);vec4 n=texture2D(normals,p);vec4 s=texture2D(specular,p);gl_FragColor=vec4(n.r,s.g,n.a,s.a);}",
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
        "original-material-pixels",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let macros = horizon_shaders::Macros::standard();
    let properties = horizon_shaders::Properties::parse("oldLighting=false", &macros);
    let inputs = horizon_shaders::GameInputs {
        terrain: Some(Default::default()),
        ..Default::default()
    };
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let post = PackPost::new(
        device,
        queue,
        adapter,
        post,
        T::Rgba8Unorm,
        World {
            atlas: None,
            materials,
            lightmap: lightmap.clone(),
            terrain: Some(Geometry {
                buffers: &[],
                table: None,
                depth_write: false,
                atlas: None,
                ids: None,
                cull: false,
            }),
            block_ids: &[],
            celestial: None,
            gui_size: [1, 1],
            clouds: None,
            weather: false,
            hand: false,
        },
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

fn texture(device: &wgpu::Device, [width, height]: [u32; 2], format: T) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Original world-target regression input"),
            size: wgpu::Extent3d {
                width,
                height,
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

fn clear(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colour: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    pixel: [u8; 4],
    reverse_depth: f32,
) {
    let [r, g, b, a] = pixel.map(|n| f64::from(n) / 255.0);
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: colour,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
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
    queue.submit([encoder.finish()]);
}

#[allow(clippy::too_many_arguments)]
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    post: &mut PackPost,
    colour: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    frame: &Frame,
    size: [u32; 2],
) -> Vec<[u8; 4]> {
    let screen = texture(device, size, T::Rgba8Unorm);
    let stride = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    post.run(
        device,
        queue,
        &mut encoder,
        colour,
        depth,
        &screen,
        frame,
        &Default::default(),
        size,
        size,
        &[],
        &mut |rp, list, _, _| {
            if matches!(list, TerrainList::Solid) {
                rp.draw(0..3, 0..1);
            }
        },
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
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
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

fn frame(hud: &HudBatch) -> Frame<'_> {
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
        pitch: 10.0,
        fov_degrees: 85.0,
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
