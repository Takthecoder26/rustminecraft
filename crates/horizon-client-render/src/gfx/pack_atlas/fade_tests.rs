//! Original GPU control using the production terrain input adapter.
use super::*;
use std::io::{Cursor, Write};
use wgpu::util::DeviceExt;

#[test]
fn terrain_fade_input_reads_the_section_visibility_on_gpu() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("SKIP chunk fade GPU control: no adapter");
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    eprintln!("chunk fade GPU adapter: {:?}", adapter.get_info());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("original fade control"),
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
    });
    let atlas = PackAtlas {
        view: texture.create_view(&Default::default()),
        texture: texture.clone(),
        columns: 1,
        sprite: 1,
        split: 1,
        separate_ao: false,
    };
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, text) in [
        (
            "shaders/gbuffers_terrain.vsh",
            "#version 330\nattribute float mc_chunkFade; varying float fade; void main(){fade=mc_chunkFade;gl_Position=vec4(gl_Vertex.xy,0.0,1.0);}",
        ),
        (
            "shaders/gbuffers_terrain.fsh",
            "#version 330\nvarying float fade; void main(){gl_FragColor=vec4(fade,0.0,0.0,1.0);}",
        ),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    let pack =
        horizon_shaders::Pack::from_zip_bytes("original-fade", zip.finish().unwrap().into_inner())
            .unwrap();
    let macros = horizon_shaders::Macros::standard();
    let program = horizon_shaders::translate_gbuffers(
        &horizon_shaders::preprocess(&pack, "gbuffers_terrain.vsh", &macros),
        &horizon_shaders::preprocess(&pack, "gbuffers_terrain.fsh", &macros),
        horizon_shaders::Target::Buffers,
        &atlas.terrain_inputs(false),
    )
    .unwrap();
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(program.interface.uniform_size).max(16),
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let module = |m| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(m)),
        })
    };
    let vertex = module(program.vertex);
    let fragment = module(program.fragment);
    let attributes = wgpu::vertex_attr_array![0=>Float32x3, 1=>Uint32x2, 2=>Uint32, 3=>Float32x4, 4=>Uint32x2, 5=>Float32x4];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vertex,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    // Independent encoded instance values: zero, quarter, three quarters and whole.
    // Negative W is the separate LOD transition encoding, whose visibility is whole.
    for (encoded, expected) in [
        (0x3800_0000u32 | 0x7f_ffff, 0u8),
        (0x3800_0000 | 0x60_0000, 64),
        (0x3800_0000 | 0x20_0000, 191),
        (0x3800_0000, 255),
        ((-2.0000305f32).to_bits(), 255),
    ] {
        let mut words = Vec::new();
        for [x, y] in [[-1.0f32, -1.0], [3.0, -1.0], [-1.0, 3.0]] {
            words.extend([
                x.to_bits(),
                y.to_bits(),
                0,
                0,
                0,
                0,
                1.0f32.to_bits(),
                1.0f32.to_bits(),
                1.0f32.to_bits(),
                1.0f32.to_bits(),
                0,
                0,
                0,
                0,
                0,
                encoded,
            ]);
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout::default(),
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = read.slice(..).get_mapped_range().unwrap();
        assert_eq!(mapped[0], expected, "encoded instance W {encoded:#x}");
    }
}
