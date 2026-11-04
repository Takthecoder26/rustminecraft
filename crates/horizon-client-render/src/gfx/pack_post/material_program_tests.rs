//! Original material selection and deferred-boundary pixel controls; no external shader code.
use super::*;
use crate::gfx::{Vertex, pack_draw::EntitySpan};
use std::io::{Cursor, Write};

fn pack(order: &str, variants: bool) -> horizon_shaders::Post {
    let vertex = "#version 120\nvoid main(){gl_Position=gl_Vertex;}";
    let properties = format!(
        "particles.ordering={order}\nseparateEntityDraws=true\n\
         blend.gbuffers_block=off\nblend.gbuffers_particles=off\n\
         blend.gbuffers_block_translucent=SRC_ALPHA ONE_MINUS_SRC_ALPHA ZERO ZERO\n\
         blend.gbuffers_particles_translucent=SRC_ALPHA ONE_MINUS_SRC_ALPHA ZERO ZERO"
    );
    let mut files = vec![("shaders.properties", properties.as_str())];
    let programs = [
        (
            "gbuffers_block",
            "blockEntityId==23 && entityId==0",
            "vec4(1,0,0,1)",
        ),
        (
            "gbuffers_particles",
            "blockEntityId==0 && entityId==7",
            "vec4(0,0,1,1)",
        ),
        (
            "gbuffers_block_translucent",
            "blockEntityId==23 && entityId==0",
            "vec4(0,1,0,0.5)",
        ),
        (
            "gbuffers_particles_translucent",
            "blockEntityId==0 && entityId==7",
            "vec4(1,1,0,0.5)",
        ),
    ];
    let sources: Vec<_> = programs.iter().take(if variants {4} else {2}).flat_map(|(name, ids, color)| [
        (format!("{name}.vsh"), vertex.to_owned()),
        (format!("{name}.fsh"), format!("#version 120\nuniform int blockEntityId; uniform int entityId;\nvoid main(){{gl_FragData[0]={ids} ? {color} : vec4(1,0,1,1);}}")),
    ]).collect();
    files.extend(
        sources
            .iter()
            .map(|(name, source)| (name.as_str(), source.as_str())),
    );
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
        "original-material-pixels",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    horizon_shaders::load_post(
        &pack,
        "",
        &properties,
        &macros,
        &horizon_shaders::GameInputs {
            entities: Some(super::super::pack_entities::inputs()),
            ..Default::default()
        },
    )
}

#[test]
#[ignore = "requires a host GPU; missing adapter/device is a failure"]
fn material_programs_and_mixed_order_reach_pixels() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("host GPU adapter");
    eprintln!("MATERIAL_PROGRAM_ADAPTER {:?}", adapter.get_info());
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("host GPU device");
    let lightmap = device
        .create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    // Each source span contains both materials; HDR tiers are independent of material tags.
    let overlays = [
        0,
        super::super::entity_batch::OIT_ENTITY | (4 << 16),
        0,
        crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY | (2 << 16),
    ];
    let vertices: Vec<Vertex> = overlays
        .into_iter()
        .enumerate()
        .flat_map(|(tile, layer)| {
            let left = tile as f32 * 0.5 - 1.0;
            [
                (left, -1.0),
                (left + 0.5, -1.0),
                (left + 0.5, 1.0),
                (left, 1.0),
            ]
            .map(|(x, y)| Vertex {
                pos: [x, y, 0.0],
                layer,
                color: u32::MAX,
                light: 0xf0f0,
                normal: [0, 0, 127, 0],
                ..bytemuck::Zeroable::zeroed()
            })
        })
        .collect();
    let spans = [
        EntitySpan {
            stage: horizon_shaders::render_stage("BLOCK_ENTITIES"),
            id: 23,
            ..Default::default()
        },
        EntitySpan {
            start: 8,
            stage: horizon_shaders::render_stage("PARTICLES"),
            id: 7,
            ..Default::default()
        },
    ];
    for variants in [true, false] {
        for order in ["mixed", "before", "after"] {
            let world = World {
                atlas: None,
                material_maps: None,
                lightmap: lightmap.clone(),
                terrain: None,
                sprite_rects: &[],
                block_ids: &[],
                celestial: None,
                gui_size: [8, 4],
                clouds: None,
                weather: false,
                hand: false,
            };
            let mut post = PackPost::new(
                &device,
                &queue,
                &adapter,
                pack(order, variants),
                wgpu::TextureFormat::Rgba8Unorm,
                world,
            )
            .expect("material fixture loads");
            let mut rest = Vec::new();
            post.prepare_entities(
                &device, &queue, &vertices, &spans, [0.; 4], [0.; 4], &mut rest, None,
            );
            assert!(
                rest.is_empty(),
                "all fixture materials must reach pack programs"
            );
            post.copied |= 1;
            post.resize(&device, [8, 4]);
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Material pixel readback"),
                size: 2048,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let target = post.targets.as_ref().unwrap();
                let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[attach(
                        &target.color[0].as_ref().unwrap()[0],
                        wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    )],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &target.depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }
            // These are the production draws on either side of deferred. Snapshot each phase.
            for late in [false, true] {
                post.draw_entities(&device, &queue, &mut encoder, [8, 4], late);
                let target = &post.targets.as_ref().unwrap().color[0].as_ref().unwrap()[0];
                encoder.copy_texture_to_buffer(
                    target.texture().as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &read,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: if late { 1024 } else { 0 },
                            bytes_per_row: Some(256),
                            ..Default::default()
                        },
                    },
                    target.texture().size(),
                );
            }
            queue.submit([encoder.finish()]);
            read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = read.slice(..).get_mapped_range().unwrap();
            let early = [true, false, order != "after", order == "before"];
            for late in [false, true] {
                for y in 0..4 {
                    for x in 0..8 {
                        let tile = x / 2;
                        let expected = if !late && !early[tile] {
                            [0, 0, 0, 0]
                        } else {
                            match (variants, tile) {
                                (_, 0) | (false, 1) => [255, 0, 0, 255],
                                (true, 1) => [0, 128, 0, 0],
                                (_, 2) | (false, 3) => [0, 0, 255, 255],
                                _ => [128, 128, 0, 0],
                            }
                        };
                        let offset = usize::from(late) * 1024 + y * 256 + x * 4;
                        let pixel = &bytes[offset..offset + 4];
                        for (actual, want) in pixel.iter().zip(expected) {
                            assert!(
                                actual.abs_diff(want) <= 1,
                                "order={order} variants={variants} late={late} pixel=({x},{y}): {pixel:?}, expected={expected:?}"
                            );
                        }
                    }
                }
            }
            eprintln!(
                "MATERIAL_PROGRAM_PIXELS order={order} variants={variants}: 64 phase pixels passed"
            );
        }
    }
}
