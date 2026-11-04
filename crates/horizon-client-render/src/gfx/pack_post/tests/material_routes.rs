//! Original GPU control: program colour and subject IDs reveal each material's draw phase.
use super::*;
use crate::gfx::{
    Vertex,
    entity_skin::{SkinFrame, SkinJob},
    pack_draw::EntitySpan,
};

const SIZE: [u32; 2] = [32, 8];

fn gpu() -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("MATERIAL_ROUTE_GPU_SETUP_ABSENT: {error}");
            assert_ne!(
                std::env::var("HORIZON_REQUIRE_ROUTE_GPU").as_deref(),
                Ok("1"),
                "mandatory material routing GPU control has no adapter"
            );
            return None;
        }
    };
    eprintln!("MATERIAL_ROUTE_GPU_ADAPTER {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
        ..Default::default()
    }))
    .expect("material routing GPU device setup failed");
    Some((adapter, device, queue))
}

fn read_target(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Material route pixel receipt"),
        size: u64::from(256 * view.texture().height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        view.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        view.texture().size(),
    );
    buffer
}

fn pixels(device: &wgpu::Device, buffer: &wgpu::Buffer, label: &str, expected: [[u8; 4]; 8]) {
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap();
    let actual: [[u8; 4]; 8] = std::array::from_fn(|column| {
        let at = 4 * 256 + (column * 4 + 2) * 4;
        bytes[at..at + 4].try_into().unwrap()
    });
    eprintln!("MATERIAL_ROUTE_GPU_PIXELS {label}: {actual:?}");
    assert_eq!(actual, expected, "{label}");
}

fn quad(column: usize, layer: u32) -> [Vertex; 4] {
    let left = column as f32 / 4.0 - 1.0;
    let right = left + 0.25;
    [
        ([left, -1., 0.], [0., 1.]),
        ([left, 1., 0.], [0., 0.]),
        ([right, 1., 0.], [1., 0.]),
        ([right, -1., 0.], [1., 1.]),
    ]
    .map(|(pos, uv)| Vertex {
        pos,
        uv,
        layer,
        color: 0xffff_ffff,
        light: 0xffff_0000,
        ..bytemuck::Zeroable::zeroed()
    })
}

fn span(start: u32, stage: &str, id: i32) -> EntitySpan {
    EntitySpan {
        start,
        stage: horizon_shaders::render_stage(stage),
        id,
        ..Default::default()
    }
}

#[test]
fn gpu_material_routes_render_both_phases_ids_and_block_shadow_owner() {
    let Some((adapter, device, queue)) = gpu() else {
        return;
    };
    use std::io::{Cursor, Write};
    const VERTEX: &str = "#version 120\nvoid main(){gl_Position=gl_Vertex;}";
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let properties = "particles.ordering=mixed\nseparateEntityDraws=true\nshadow.enabled=true\n";
    zip.start_file(
        "shaders/shaders.properties",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(properties.as_bytes()).unwrap();
    for (name, uniforms, color) in [
        (
            "gbuffers_entities",
            "uniform int entityId;",
            "vec4(float(entityId)/255.0,0.0,1.0,1.0)",
        ),
        (
            "gbuffers_block",
            "uniform int blockEntityId;",
            "vec4(float(blockEntityId)/255.0,0.0,0.0,1.0)",
        ),
        (
            "gbuffers_block_translucent",
            "uniform int blockEntityId;",
            "vec4(0.0,float(blockEntityId)/255.0,0.0,1.0)",
        ),
        ("gbuffers_particles", "", "vec4(0.0,0.0,1.0,1.0)"),
        (
            "gbuffers_particles_translucent",
            "",
            "vec4(1.0,0.0,1.0,1.0)",
        ),
        ("gbuffers_beaconbeam", "", "vec4(0.0,1.0,1.0,1.0)"),
        (
            "shadow_block",
            "const int shadowMapResolution=32;\nuniform int blockEntityId;",
            "vec4(float(blockEntityId)/255.0,0.0,0.0,1.0)",
        ),
    ] {
        for (ext, text) in [
            ("vsh", VERTEX.to_owned()),
            (
                "fsh",
                format!("#version 120\n{uniforms}\nvoid main(){{gl_FragData[0]={color};}}"),
            ),
        ] {
            zip.start_file(
                format!("shaders/{name}.{ext}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-material-route-control",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let post = horizon_shaders::load_post(
        &pack,
        "",
        &properties,
        &macros,
        &horizon_shaders::GameInputs {
            entities: Some(super::super::pack_entities::inputs()),
            ..Default::default()
        },
    );
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    let lightmap = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Material route control lightmap"),
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
    let mut post = PackPost::new(
        &device,
        &queue,
        &adapter,
        post,
        wgpu::TextureFormat::Rgba8Unorm,
        World {
            atlas: None,
            material_maps: None,
            lightmap,
            terrain: None,
            block_ids: &[],
            celestial: None,
            gui_size: SIZE,
            clouds: None,
            weather: false,
            hand: false,
        },
    );
    eprintln!("MATERIAL_ROUTE_GPU_PROGRAM_NOTES {:?}", post.notes);
    let entity_passes = &post
        .entities
        .as_ref()
        .expect("material programs built")
        .passes;
    for index in [0, 1, 3, 5, 6, 7] {
        assert!(
            entity_passes[index].is_some(),
            "missing route {index}: {:?}",
            post.notes
        );
    }
    assert!(
        post.entities.as_ref().unwrap().shadow[1].is_some(),
        "{:?}",
        post.notes
    );
    post.resize(&device, SIZE);
    let vertices: Vec<_> = [(5, 0), (0, 0), (1, 32), (2, 0), (3, 15), (6, 9), (7, 6)]
        .into_iter()
        .flat_map(|(column, layer)| quad(column, layer))
        .collect();
    let spans = [
        span(4, "BLOCK_ENTITIES", 111),
        span(8, "BLOCK_ENTITIES", 222),
        span(12, "PARTICLES", 0),
        span(20, "BLOCK_ENTITIES", 99),
        span(24, "NONE", 0),
    ];
    let skin = SkinFrame {
        vertices: 4,
        spans: vec![span(0, "ENTITIES", 37)],
        jobs: vec![SkinJob {
            dst: 0,
            count: 4,
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut rest = Vec::new();
    post.prepare_entities(
        &device,
        &queue,
        &vertices,
        &spans,
        [0.; 4],
        [0.; 4],
        &mut rest,
        Some(&skin),
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&rest),
        bytemuck::cast_slice::<_, u8>(&quad(5, 0)),
        "untagged native remainder"
    );
    queue.write_buffer(
        post.skin_target().unwrap(),
        0,
        bytemuck::cast_slice(&quad(4, 0)),
    );
    let target = post.targets.as_ref().unwrap().color[0].as_ref().unwrap()[0].clone();
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let depth = &post.targets.as_ref().unwrap().depth;
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Material route control clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
    }
    post.draw_entities(&device, &queue, &mut encoder, SIZE, false);
    let early = read_target(&device, &mut encoder, &target);
    post.draw_entities(&device, &queue, &mut encoder, SIZE, true);
    let late = read_target(&device, &mut encoder, &target);
    post.draw_shadow_entities(&device, &queue, &mut encoder, SIZE);
    let shadow = read_target(
        &device,
        &mut encoder,
        post.shadow_targets
            .as_ref()
            .unwrap()
            .color(0, false)
            .unwrap(),
    );
    queue.submit([encoder.finish()]);
    let empty = [0; 4];
    pixels(
        &device,
        &early,
        "before deferred",
        [
            [111, 0, 0, 255],
            empty,
            [0, 0, 255, 255],
            empty,
            [37, 0, 255, 255],
            empty,
            [99, 0, 0, 255],
            empty,
        ],
    );
    pixels(
        &device,
        &late,
        "after deferred",
        [
            [111, 0, 0, 255],
            [0, 222, 0, 255],
            [0, 0, 255, 255],
            [255, 0, 255, 255],
            [37, 0, 255, 255],
            empty,
            [99, 0, 0, 255],
            [0, 255, 255, 255],
        ],
    );
    // Only block subjects own this shadow program: their camera material cannot change that.
    pixels(
        &device,
        &shadow,
        "block shadow",
        [
            [111, 0, 0, 255],
            [222, 0, 0, 255],
            [255; 4],
            [255; 4],
            [255; 4],
            [255; 4],
            [99, 0, 0, 255],
            [255; 4],
        ],
    );
}
