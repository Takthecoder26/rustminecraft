//! Original hardware negotiation and logical-target blend control.
use super::*;
use std::io::{Cursor, Write};

#[test]
fn independent_blending_pixels_and_feature_negotiation_match_the_adapter() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("SKIP independent blend GPU control: no adapter");
        return;
    };
    let supported = adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::INDEPENDENT_BLEND);
    eprintln!(
        "independent blend adapter: {:?}, supported={supported}",
        adapter.get_info()
    );
    if !supported {
        eprintln!("SKIP positive independent blend pixels: adapter lacks capability");
        return;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, text) in [
        (
            "shaders/shaders.properties",
            "iris.features.required=PER_BUFFER_BLENDING\niris.features.optional=PER_BUFFER_BLENDING\nblend.gbuffers_terrain=off\nblend.gbuffers_terrain.colortex5=ONE ONE ONE ONE\nblend.gbuffers_terrain.colortex2=off",
        ),
        (
            "shaders/gbuffers_terrain.vsh",
            "#version 330\nvoid main(){gl_Position=gl_Vertex;}",
        ),
        (
            "shaders/gbuffers_terrain.fsh",
            "#version 330\n/* DRAWBUFFERS:52 */\nvoid main(){gl_FragData[0]=vec4(0.25,0.0,0.0,1.0);gl_FragData[1]=vec4(0.0,0.5,0.0,1.0);}",
        ),
    ] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(text.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-blend",
        archive.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    macros.define_renderer(0);
    crate::gfx::define_pack_adapter_features(&adapter, &mut macros);
    let (properties, feature_notes) = horizon_shaders::read_properties(&pack, &mut macros);
    let inputs = horizon_shaders::GameInputs {
        terrain: Some(horizon_shaders::GbuffersInputs {
            body: "vec2 p=vec2(float((gl_VertexIndex<<1)&2),float(gl_VertexIndex&2)); hz_Vertex=vec4(p*2.0-1.0,0.0,1.0);".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    let program = post
        .terrain
        .take()
        .expect("original geometry program translates");
    let buffers = pack_images::Buffers::new(
        &device,
        Default::default(),
        Default::default(),
        wgpu::ShaderStages::FRAGMENT,
    );
    let stand_ins = std::cell::RefCell::new(Vec::new());
    let resources = Resources {
        textures: &[],
        images: &[],
        left_out: &[],
        buffers: &buffers,
        indices: None,
        gpu: Gpu::of(&adapter, device.features()),
        storable: 0,
        released: std::cell::Cell::new(("", 0)),
        stand_ins: &stand_ins,
    };
    let draws = DrawUniforms::new(&device);
    let mut notes = Vec::new();
    let pass = build_pass(
        &device,
        program,
        &[wgpu::TextureFormat::Rgba8Unorm; BUFFERS],
        None,
        wgpu::TextureFormat::Rgba8Unorm,
        &mut notes,
        None,
        &[],
        &draws.layout,
        &resources,
    )
    .unwrap_or_else(|| panic!("independent target blends build: {notes:?}"));
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pass.layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: pass.ubo.as_entire_binding(),
        }],
    });
    let colors: Vec<_> = (0..2)
        .map(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        })
        .collect();
    let views: Vec<_> = colors
        .iter()
        .map(|t| t.create_view(&Default::default()))
        .collect();
    let readbacks: Vec<_> = (0..2)
        .map(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 4,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        })
        .collect();
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let attachments: Vec<_> = views
            .iter()
            .map(|view| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.125,
                            g: 0.125,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &attachments,
            ..Default::default()
        });
        render.set_pipeline(pass.render());
        render.set_bind_group(0, &group, &[]);
        render.draw(0..3, 0..1);
        render.draw(0..3, 0..1);
    }
    for (color, read) in colors.iter().zip(&readbacks) {
        encoder.copy_texture_to_buffer(
            color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: read,
                layout: wgpu::TexelCopyBufferLayout::default(),
            },
            color.size(),
        );
    }
    queue.submit([encoder.finish()]);
    for read in &readbacks {
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    }
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels: Vec<Vec<u8>> = readbacks
        .iter()
        .map(|read| read.slice(..).get_mapped_range().unwrap().to_vec())
        .collect();
    eprintln!("logical targets5/2, additive/off after two draws: {pixels:?}");
    assert_eq!(pixels, [vec![160, 32, 0, 255], vec![0, 128, 0, 255]]);
    assert!(
        feature_notes.is_empty(),
        "capable adapter must accept required blending: {feature_notes:?}"
    );
    assert!(macros.is_defined("IRIS_FEATURE_PER_BUFFER_BLENDING"));
}
