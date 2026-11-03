//! Original asset-free shadow image provider regression.
use super::*;
use crate::{gfx::Frame, hud::HudBatch};
use std::io::{Cursor, Write};
use wgpu::TextureFormat as T;

fn pack(image: bool) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut add = |name: &str, text: &str| {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    };
    let vertex = "#version 120\nvoid main(){gl_Position=ftransform();}\n";
    add("shaders.properties", "shadow.enabled=true\n");
    if image {
        add(
            "composite.csh",
            "#version 430\nlayout(local_size_x=1,local_size_y=1) in;\nconst ivec3 workGroups=ivec3(16,16,1);\nlayout(rgba8) writeonly uniform image2D shadowcolorimg0;\nvoid main(){imageStore(shadowcolorimg0,ivec2(gl_GlobalInvocationID.xy),vec4(0.25,0.5,0.75,1));}\n",
        );
    } else {
        add("shadowcomp.vsh", vertex);
        add(
            "shadowcomp.fsh",
            "#version 120\n/* RENDERTARGETS:0 */\nvoid main(){gl_FragData[0]=vec4(0.25,0.5,0.75,1);}\n",
        );
    }
    add("final.vsh", vertex);
    add(
        "final.fsh",
        "#version 120\nuniform sampler2D shadowcolor0;\nconst int shadowMapResolution=16;\nvoid main(){gl_FragColor=texture2D(shadowcolor0,gl_FragCoord.xy/16.0);}\n",
    );
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-shadow-image",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn shadow_image_provider_stores_reach_sampler_pixels() {
    let gpu = Fixture::new().expect("GPU required for explicit shadow contract run");
    let mut wrong = Vec::new();
    for image in [false, true] {
        let mut post = gpu.build(pack(image));
        for frame in 0..2 {
            let actual = gpu.render(&mut post, [16, 16]);
            let count = actual
                .iter()
                .filter(|p| {
                    p.iter()
                        .zip([64u8, 128, 191, 255])
                        .any(|(a, b)| a.abs_diff(b) > 1)
                })
                .count();
            eprintln!(
                "shadow image={image} frame={frame}: wrong={count}/256 first={:?}",
                actual[0]
            );
            if count != 0 {
                wrong.push((image, frame, count));
            }
        }
    }
    assert!(wrong.is_empty(), "provider pixel failures: {wrong:?}");
}

fn original(files: &[(&str, &str)]) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in files {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-shadow-lifecycle",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &Default::default());
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    post
}

#[test]
fn shadow_images_in_compute_turn_on_targets_and_read_stage_constants() {
    let post = original(&[(
        "composite.csh",
        "#version 430\nlayout(local_size_x=1) in;\n/* const int shadowcolor1Format=R32UI; */\nconst bool shadowcolor1Clear=false;\nconst int shadowMapResolution=16;\nlayout(r32ui) writeonly uniform uimage2D shadowcolorimg1;\nvoid main(){imageStore(shadowcolorimg1,ivec2(0),uvec4(7));}\n",
    )]);
    assert!(post.shadow_pass);
    assert_eq!(post.config.shadow.resolution, 16);
    assert_eq!(
        post.config.shadow.colors[1].format,
        Format::parse("R32UI").unwrap()
    );
    assert!(!post.config.shadow.colors[1].clear);
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn shadow_image_lifecycle_feedback_and_image_only_demand() {
    let gpu = Fixture::new().expect("GPU required");
    let vertex = "#version 120\nvoid main(){gl_Position=ftransform();}\n";
    for n in 0..2 {
        for loads in [false, true] {
            let declarations = if loads {
                String::new()
            } else {
                format!("uniform sampler2D shadowcolor{n};")
            };
            let before = if loads {
                format!("vec4 before=imageLoad(shadowcolorimg{n},p);")
            } else {
                String::new()
            };
            let value = if loads {
                "before".into()
            } else {
                format!("texelFetch(shadowcolor{n},p,0)")
            };
            let compute = format!(
                "#version 430\nlayout(local_size_x=1,local_size_y=1) in;\nconst ivec3 workGroups=ivec3(16,16,1);\nlayout(rgba8) {} uniform image2D shadowcolorimg{n};\n{declarations}\nvoid main(){{ivec2 p=ivec2(gl_GlobalInvocationID.xy);{before}imageStore(shadowcolorimg{n},p,{value}+vec4(0.25,0,0,0));}}\n",
                if loads { "" } else { "writeonly" }
            );
            let initial = format!(
                "#version 120\n/* RENDERTARGETS:{n} */\nvoid main(){{gl_FragData[0]=vec4(0.25,0.5,0.75,1);}}\n"
            );
            // Only image bindings (no final sampler and no shadow.enabled property) must demand
            // a shadow target too. The compute sampler form separately exercises feedback copies.
            let final_source = format!(
                "#version 430\nconst int shadowMapResolution=16;\nlayout(rgba8) readonly uniform image2D shadowcolorimg{n};\nvoid main(){{gl_FragColor=imageLoad(shadowcolorimg{n},ivec2(gl_FragCoord.xy));}}\n"
            );
            let mut post = gpu.build(original(&[
                ("shadowcomp.vsh", vertex),
                ("shadowcomp.fsh", &initial),
                ("composite.csh", &compute),
                ("final.vsh", vertex),
                ("final.fsh", &final_source),
            ]));
            assert!(
                post.notes.iter().all(|n| n.starts_with("shadow map ")),
                "{:?}",
                post.notes
            );
            let shadow = post.shadow_targets.as_ref().unwrap();
            assert!(shadow.color[n].is_some());
            assert!(
                shadow.color[1 - n].is_none(),
                "unused shadow target allocated"
            );
            assert!(shadow.scratch[n].is_some());
            for size in [[16, 16], [8, 8], [16, 16]] {
                let pixels = gpu.render(&mut post, size);
                assert!(
                    pixels.iter().all(|p| p
                        .iter()
                        .zip([128u8, 128, 191, 255])
                        .all(|(a, b)| a.abs_diff(b) <= 1)),
                    "n={n} loads={loads}: {:?}",
                    pixels[0]
                );
            }
        }
    }
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn shadow_image_device_binding_limit_skips_only_unsupported_program() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU required");
    let mut limits = adapter.limits();
    limits.max_storage_textures_per_shader_stage = 0;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let gpu = Fixture {
        adapter,
        device,
        queue,
    };
    let mut post = gpu.build(pack(true));
    assert!(
        post.notes
            .iter()
            .any(|n| n.starts_with("composite: skipped:")),
        "{:?}",
        post.notes
    );
    assert_eq!(post.shadow_targets.as_ref().unwrap().storage, 0);
    assert!(
        gpu.render(&mut post, [16, 16])
            .iter()
            .all(|p| *p == [255; 4])
    );
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn shadow_image_integer_format_declared_only_in_compute_reaches_pixels() {
    let gpu = Fixture::new().expect("GPU required");
    let mut post=gpu.build(original(&[
        ("composite.csh", "#version 430\nlayout(local_size_x=1,local_size_y=1) in;\nconst ivec3 workGroups=ivec3(16,16,1);\n/* const int shadowcolor1Format=R32UI; */\nconst int shadowMapResolution=16;\nlayout(r32ui) writeonly uniform uimage2D shadowcolorimg1;\nvoid main(){imageStore(shadowcolorimg1,ivec2(gl_GlobalInvocationID.xy),uvec4(11));}\n"),
        ("final.vsh", "#version 120\nvoid main(){gl_Position=ftransform();}\n"),
        ("final.fsh", "#version 430\nuniform usampler2D shadowcolor1;\nvoid main(){gl_FragColor=vec4(float(texelFetch(shadowcolor1,ivec2(gl_FragCoord.xy),0).r)/32.0,0,0,1);}\n"),
    ]));
    assert!(
        post.notes.iter().all(|n| n.starts_with("shadow map ")),
        "{:?}",
        post.notes
    );
    assert_eq!(
        post.shadow_targets.as_ref().unwrap().color[1]
            .as_ref()
            .unwrap()[0]
            .texture()
            .format(),
        T::R32Uint
    );
    assert!(gpu.render(&mut post, [16, 16]).iter().all(|p| {
        p.iter()
            .zip([88u8, 0, 0, 255])
            .all(|(a, b)| a.abs_diff(b) <= 1)
    }));
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn shadow_images_clear_before_begin_and_preserve_uncleared_history() {
    let gpu = Fixture::new().expect("GPU required");
    for clear in [true, false] {
        let compute = format!(
            "#version 430\nlayout(local_size_x=1,local_size_y=1) in;\nconst ivec3 workGroups=ivec3(16,16,1);\nconst int shadowMapResolution=16;\nconst bool shadowcolor0Clear={clear};\nconst vec4 shadowcolor0ClearColor=vec4(0.125,0.25,0.5,1);\nlayout(rgba8) uniform image2D shadowcolorimg0;\nvoid main(){{ivec2 p=ivec2(gl_GlobalInvocationID.xy);vec4 old=imageLoad(shadowcolorimg0,p);imageStore(shadowcolorimg0,p,vec4(old.r+0.25,0.25,0.5,1));}}\n"
        );
        let mut post=gpu.build(original(&[
            ("begin.csh",&compute),
            ("final.vsh","#version 120\nvoid main(){gl_Position=ftransform();}\n"),
            ("final.fsh","#version 430\nlayout(rgba8) readonly uniform image2D shadowcolorimg0;\nvoid main(){gl_FragColor=imageLoad(shadowcolorimg0,ivec2(gl_FragCoord.xy));}\n"),
        ]));
        assert!(
            post.notes.iter().all(|n| n.starts_with("shadow map ")),
            "{:?}",
            post.notes
        );
        for frame in 0..3 {
            let red = if clear { 96 } else { [64, 128, 191][frame] };
            let pixels = gpu.render_with_cache(&mut post, [16, 16], true);
            assert!(
                pixels.iter().all(|p| p
                    .iter()
                    .zip([red, 64, 128, 255])
                    .all(|(a, b)| a.abs_diff(b) <= 1)),
                "clear={clear} frame={frame}: {:?}",
                pixels[0]
            );
        }
    }
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn shadow_image_early_color_clear_keeps_depth_until_shadow_draw() {
    let gpu = Fixture::new().expect("GPU required");
    let config = horizon_shaders::Shadow {
        resolution: 1,
        ..Default::default()
    };
    let mut targets = super::super::pack_shadow::ShadowTargets::new(
        &gpu.device,
        &config,
        [T::Rgba8Unorm; 2],
        1,
        1,
        0,
        false,
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &targets.depth.1,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.25),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
    targets.begin_frame();
    targets.clear_colors(&mut encoder);
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &targets.depth.0,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: Default::default(),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    encoder.copy_texture_to_buffer(
        targets.color(0, false).unwrap().texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 4,
                ..Default::default()
            },
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    read.slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let values = read.slice(..).get_mapped_range().unwrap();
    assert_eq!(f32::from_le_bytes(values[..4].try_into().unwrap()), 0.25);
    assert_eq!(&values[4..], &[255; 4]);
    let (depth, colors) = targets.clear_ops(std::iter::once(0));
    assert!(matches!(depth.load, wgpu::LoadOp::Clear(1.0)));
    assert!(matches!(colors[0].load, wgpu::LoadOp::Load));
}

struct Fixture {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Fixture {
    fn new() -> Option<Self> {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("Post destination test skipped: no GPU adapter");
            return None;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            required_features: adapter.features()
                & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            ..Default::default()
        }))
        .expect("GPU device for destination contract");
        eprintln!("Post destination adapter: {:?}", adapter.get_info());
        Some(Self {
            adapter,
            device,
            queue,
        })
    }

    fn texture(&self, size: [u32; 2], format: T) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Original post destination fixture"),
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
        eprintln!("Provider notes: {:?}", post.notes);
        post
    }

    fn render(&self, post: &mut PackPost, size: [u32; 2]) -> Vec<[u8; 4]> {
        self.render_with_cache(post, size, false)
    }

    fn render_with_cache(&self, post: &mut PackPost, size: [u32; 2], cache: bool) -> Vec<[u8; 4]> {
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
        let mut frame = fixture_frame(&hud);
        frame.shadow_cache = cache;
        post.run(
            &self.device,
            &self.queue,
            &mut encoder,
            &world,
            &depth,
            &screen,
            &frame,
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
