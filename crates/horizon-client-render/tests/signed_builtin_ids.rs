//! Original signed arithmetic and overload checks through translated GLSL and pixel readback.
use std::io::{Cursor, Write};

use horizon_shaders::{Macros, Pack, Program, Target, preprocess, translate_composite};

fn vertex(selected: u32) -> String {
    format!(
        "#version 140\nout vec3 value;\n\
         float signedType(int x) {{ return 1.0; }}\n\
         float signedType(uint x) {{ return 0.0; }}\n\
         void main() {{\n\
         int corner = int(gl_VertexID) % 3;\n\
         vec2 p = vec2(corner == 1 ? 3.0 : -1.0, corner == 2 ? 3.0 : -1.0);\n\
         gl_Position = gl_InstanceID == {selected} ? vec4(p, 0.0, 1.0) : vec4(4.0, 4.0, 0.0, 1.0);\n\
         value = vec3(gl_VertexID - 1 < 0 ? 1.0 : 0.0,\n\
                      gl_InstanceID - 1 < 0 ? 1.0 : 0.0,\n\
                      0.5 * (signedType(gl_VertexID) + signedType(gl_InstanceID)));\n}}"
    )
}

fn program(selected: u32) -> Program {
    let vertex = vertex(selected);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, source) in [
        ("shaders/composite.vsh", vertex.as_str()),
        (
            "shaders/composite.fsh",
            "#version 140\nuniform float probeAlpha; in vec3 value; out vec4 color; void main() { color = vec4(value, probeAlpha); }",
        ),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = Pack::from_zip_bytes("signed-ids.zip", zip.finish().unwrap().into_inner()).unwrap();
    let macros = Macros::standard();
    translate_composite(
        &preprocess(&pack, "composite.vsh", &macros),
        &preprocess(&pack, "composite.fsh", &macros),
        Target::Buffers,
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
}

#[test]
fn signed_id_value_shaders_translate() {
    for selected in [0, 1] {
        program(selected);
    }
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn signed_id_arithmetic_and_overloads_render_their_values() {
    use wgpu::util::DeviceExt;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    eprintln!("Signed ID GPU: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    for (selected, expected) in [(0, [128u8, 255, 255, 255]), (1, [128, 0, 255, 255])] {
        let program = program(selected);
        let mut bytes = vec![0; program.interface.uniform_size as usize];
        let alpha = program
            .interface
            .uniforms
            .iter()
            .find(|u| u.name == "probeAlpha")
            .unwrap();
        bytes[alpha.offset as usize..alpha.offset as usize + 4]
            .copy_from_slice(&1.0f32.to_le_bytes());
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let module = |module| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
            })
        };
        let vs = module(program.vertex);
        let fs = module(program.fragment);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &vs,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fs,
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
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
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
        });
        let view = texture.create_view(&Default::default());
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            // All draws start at zero. Select the second instance from a zero-based batch;
            // this deliberately does not test or alter base-instance semantics.
            pass.draw(0..3, 0..selected + 1);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = read.slice(..).get_mapped_range().unwrap();
        // The center has barycentric weights (1/2, 1/4, 1/4): only vertex zero has ID-1<0.
        // Instance zero's ID-1 is negative; instance one's is zero. Both overloads take int.
        eprintln!("instance {selected}: {:?}", &mapped[..4]);
        for (actual, expected) in mapped[..4].iter().zip(expected) {
            assert!(
                actual.abs_diff(expected) <= 1,
                "instance {selected}: {:?}",
                &mapped[..4]
            );
        }
    }
}
