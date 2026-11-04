//! Original GPU probes of the production terrain inputs, without a resource or shader pack.

use super::PackAtlas;
use crate::mesher::{TerrainVertex, UV_STEPS};
use wgpu::util::DeviceExt;

#[test]
fn mid_tex_coord_follows_each_quads_mapped_corners() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        return;
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    eprintln!("midpoint probe adapter: {:?}", adapter.get_info());
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 5,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut encoder, &[texture], 5, 5, 16).unwrap();
    queue.submit([encoder.finish()]);
    // Each row uses a different atlas cell. Expected atlas coordinates are independent
    // numbers, including the existing clamp of a model UV beyond the sprite's edge.
    let cases = [
        (
            "whole",
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            [1. / 6., 0.25],
            [1. / 6., 0.25],
            [[0., 0.], [1. / 3., 0.], [1. / 3., 0.5], [0., 0.5]],
        ),
        (
            "half-height",
            [[0., 0.], [1., 0.], [1., 0.5], [0., 0.5]],
            [0.5, 0.125],
            [0.5, 0.25],
            [
                [1. / 3., 0.],
                [2. / 3., 0.],
                [2. / 3., 0.25],
                [1. / 3., 0.25],
            ],
        ),
        (
            "rotated partial",
            [[0.25, 0.75], [0.25, 0.25], [0.5, 0.25], [0.5, 0.75]],
            [19. / 24., 0.25],
            [5. / 6., 0.25],
            [
                [0.75, 0.375],
                [0.75, 0.125],
                [5. / 6., 0.125],
                [5. / 6., 0.375],
            ],
        ),
        (
            "clamped",
            [[0., 0.], [1.5, 0.], [1.5, 0.5], [0., 0.5]],
            [1. / 6., 0.625],
            [1. / 6., 0.75],
            [[0., 0.5], [1. / 3., 0.5], [1. / 3., 0.75], [0., 0.75]],
        ),
        (
            "fourth corner",
            [[0., 0.], [1., 0.], [1., 0.5], [0., 1.]],
            [0.5, 0.6875],
            [0.5, 0.75],
            [
                [1. / 3., 0.5],
                [2. / 3., 0.5],
                [2. / 3., 0.75],
                [1. / 3., 1.],
            ],
        ),
    ];
    let positions = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let vertices: Vec<_> = cases
        .iter()
        .enumerate()
        .flat_map(|(layer, (_, uv, ..))| {
            (0..4).map(move |k| {
                TerrainVertex::new(positions[k], uv[k], layer as u16 | 0x8000, u32::MAX, 0)
            })
        })
        .collect();
    let mut failures = Vec::new();
    // Positive origins encode fade bits beside the power-of-two scale. Negative origins
    // use the older fractional scale encoding. Both must match the mapped vertex UVs.
    for origin in [
        f32::from_bits((1. / UV_STEPS).to_bits() | 0x12345),
        -3. - 1. / UV_STEPS,
    ] {
        for flat in [true, false] {
            for shadow in [false, true] {
                let pixels = sample(
                    &device,
                    &queue,
                    &atlas.terrain_inputs(flat),
                    &vertices,
                    [0., 0., 0., origin],
                    [flat, shadow, false],
                    [
                        "attribute vec2 mc_midTexCoord;",
                        "vec4(mc_midTexCoord, gl_MultiTexCoord0.xy)",
                    ],
                );
                for (quad, (name, _, midpoint, fallback, mapped)) in cases.iter().enumerate() {
                    let centre = if flat { midpoint } else { fallback };
                    for (corner, uv) in mapped.iter().enumerate() {
                        let actual = pixels[4 * quad + corner];
                        let expected = [centre[0], centre[1], uv[0], uv[1]];
                        if actual
                            .iter()
                            .zip(expected)
                            .any(|(a, b)| !a.is_finite() || (a - b).abs() > 1e-6)
                        {
                            failures.push(format!("{name} corner {corner}, flat={flat} shadow={shadow} origin={origin}: {actual:?} != {expected:?}"));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn chunk_fade_decodes_visibility_without_moving_or_losing_vertices() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = match pollster::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            assert!(
                std::env::var_os("HORIZON_REQUIRE_CHUNK_FADE_GPU").is_none(),
                "chunk-fade GPU control requires an adapter: {error}"
            );
            eprintln!("chunk-fade GPU control skipped: no adapter ({error})");
            return;
        }
    };
    eprintln!("chunk-fade probe adapter: {:?}", adapter.get_info());
    if std::env::var_os("HORIZON_REQUIRE_CHUNK_FADE_GPU").is_some() {
        assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    }
    let geometry_supported = adapter.limits().max_bind_groups >= 6;
    if !geometry_supported {
        eprintln!("chunk-fade geometry subcontrol skipped: fewer than six bind groups");
    }
    if std::env::var_os("HORIZON_REQUIRE_CHUNK_FADE_GPU").is_some() {
        assert!(geometry_supported, "geometry control needs six bind groups");
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut encoder, &[texture], 1, 1, 2).unwrap();
    queue.submit([encoder.finish()]);
    let positions = [
        [0.25, 0.5, 0.75],
        [1.25, 0.5, 0.75],
        [1.25, 1.5, 0.75],
        [0.25, 1.5, 0.75],
    ];
    let vertices = positions.map(|p| TerrainVertex::new(p, [0., 0.], 0, u32::MAX, 0));
    // Independent bit patterns in the existing instance format, not encoded by the helper
    // under test. The newest value is positive, so a missing pixel cannot pass as zero.
    let cases = [
        (0x3800_0000, 1.0),
        (0x387f_ffff, 1.0 / 8_388_608.0),
        (0x3860_0000, 0.25),
        (0x3840_0000, 0.5),
        (0x3d00_0000, 1.0),                          // A whole distant tile.
        ((-3.0f32 - 1.0 / UV_STEPS).to_bits(), 1.0), // Independent LOD dither.
        (0, 1.0),
    ];
    for flat in [false, true] {
        for shadow in [false, true] {
            for geometry in [false, true]
                .into_iter()
                .filter(|g| !g || flat && shadow && geometry_supported)
            {
                for declaration in ["", "attribute float mc_chunkFade;"] {
                    for (bits, fade) in cases {
                        for offset in [[0., 0., 0.], [128.5, -32., 64.]] {
                            let origin = [offset[0], offset[1], offset[2], f32::from_bits(bits)];
                            let pixels = sample(
                                &device,
                                &queue,
                                &atlas.terrain_inputs(flat),
                                &vertices,
                                origin,
                                [flat, shadow, geometry],
                                [declaration, "vec4(mc_chunkFade, gl_Vertex.xyz)"],
                            );
                            for (pixel, p) in pixels.iter().zip(positions) {
                                assert_eq!(
                                    *pixel,
                                    [fade, offset[0] + p[0], offset[1] + p[1], offset[2] + p[2]],
                                    "origin={origin:?}, flat={flat}, shadow={shadow}, declaration={declaration:?}"
                                );
                            }
                        }
                    }
                    // Geometry without a terrain fade source stays fully visible even when a
                    // shared pack vertex program asks for the input.
                    let mut inputs = atlas.terrain_inputs(flat);
                    inputs.attributes.retain(|(name, _)| name != "mc_chunkFade");
                    let pixels = sample(
                        &device,
                        &queue,
                        &inputs,
                        &vertices,
                        [0., 0., 0., f32::from_bits(0x3840_0000)],
                        [flat, shadow, geometry],
                        [declaration, "vec4(mc_chunkFade, gl_Vertex.xyz)"],
                    );
                    for (pixel, p) in pixels.iter().zip(positions) {
                        assert_eq!(*pixel, [1.0, p[0], p[1], p[2]]);
                    }
                }
            }
        }
    }
}

fn sample(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    inputs: &horizon_shaders::GbuffersInputs,
    vertices: &[TerrainVertex],
    origin: [f32; 4],
    [flat, shadow, geometry]: [bool; 3],
    [declaration, expression]: [&str; 2],
) -> Vec<[f32; 4]> {
    let pp = |code| horizon_shaders::Preprocessed {
        code,
        ..Default::default()
    };
    let vertex = pp(format!(
        "{declaration}\nvarying vec4 probe;\nvoid main() {{\n\
         probe = {expression};\n\
         gl_Position = vec4((float(gl_VertexID) + 0.5) * {} - 1.0, 0.0, 0.0, 1.0);\n}}",
        2. / vertices.len() as f32,
    ));
    // Integer attachments retain the float bits without requiring float-render-target
    // capabilities on the adapter that the test runner picks.
    let varying = if geometry { "relayed" } else { "probe" };
    let fragment = pp(format!(
        "varying vec4 {varying};\nlayout(location = 0) out uvec4 midpoint_bits;\n\
        void main() {{ midpoint_bits = floatBitsToUint({varying}); }}"
    ));
    let program = if geometry {
        let geometry = pp(
            "layout(triangles) in; layout(triangle_strip, max_vertices=3) out;\n\
             in vec4 probe[]; out vec4 relayed;\n\
             void main() { for (int i=0; i<3; i++) {\n\
                 gl_Position=gl_in[i].gl_Position; relayed=probe[i]; EmitVertex();\n\
             } EndPrimitive(); }"
                .into(),
        );
        let mut inputs = inputs.clone();
        inputs.declarations.push_str(
            "layout(set=5, binding=0) readonly buffer HzIndices { uint hz_indices[]; };\n",
        );
        let (program, triangles) = horizon_shaders::translate_geometry(
            &vertex,
            &geometry,
            &fragment,
            &horizon_shaders::GeometryInputs {
                inputs,
                pull: super::PULL.into(),
                index: "hz_indices[@]".into(),
            },
        )
        .unwrap();
        assert_eq!(triangles, 1);
        Ok(program)
    } else if shadow {
        horizon_shaders::translate_shadow(&vertex, &fragment, inputs)
    } else {
        horizon_shaders::translate_gbuffers(
            &vertex,
            &fragment,
            horizon_shaders::Target::Buffers,
            inputs,
        )
    }
    .unwrap();
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let mut entries = vec![
        vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
        vec![storage(0), storage(1)],
        vec![],
        vec![storage(0), storage(1)],
    ];
    if geometry {
        entries.extend([vec![], vec![storage(0)]]);
    }
    let layouts: Vec<_> = entries
        .iter()
        .map(|entries| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries,
            })
        })
        .collect();
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
        ..Default::default()
    });
    let module = |module| {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        })
    };
    let vs = module(program.vertex);
    let fs = module(program.fragment);
    let vertex_layouts = [
        Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TerrainVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &crate::gfx::TERRAIN_ATTRIBUTES,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![5 => Float32x4],
        }),
    ];
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &vertex_layouts[..if flat { 1 } else { 2 }],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::PointList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Uint,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let buffer = |contents: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents,
            usage,
        })
    };
    let uniform = buffer(
        &vec![0; program.interface.uniform_size.max(16) as usize],
        wgpu::BufferUsages::UNIFORM,
    );
    let slots = buffer(bytemuck::bytes_of(&0u32), wgpu::BufferUsages::STORAGE);
    let origin = buffer(
        bytemuck::bytes_of(&origin),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
    );
    let ids = buffer(bytemuck::bytes_of(&[0i32; 15]), wgpu::BufferUsages::STORAGE);
    let data = buffer(
        bytemuck::cast_slice(vertices),
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
    );
    let indices = buffer(
        bytemuck::bytes_of(&[0u32, 1, 2, 0, 2, 3]),
        wgpu::BufferUsages::STORAGE,
    );
    let mut bindings = vec![
        vec![(0, &uniform)],
        vec![(0, &slots), (1, &origin)],
        vec![],
        vec![(0, &ids), (1, &data)],
    ];
    if geometry {
        bindings.extend([vec![], vec![(0, &indices)]]);
    }
    let groups: Vec<_> = bindings
        .iter()
        .zip(&layouts)
        .map(|(buffers, layout)| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout,
                entries: &buffers
                    .iter()
                    .map(|&(binding, buffer)| wgpu::BindGroupEntry {
                        binding,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: vertices.len() as u32,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Uint,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: vertices.len() as u64 * 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = output.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        for (k, group) in groups.iter().enumerate() {
            pass.set_bind_group(k as u32, group, &[]);
        }
        pass.set_vertex_buffer(0, data.slice(..));
        if !flat {
            pass.set_vertex_buffer(1, origin.slice(..));
        }
        pass.draw(0..if geometry { 6 } else { vertices.len() as u32 }, 0..1);
    }
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        output.size(),
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = read.slice(..).get_mapped_range().unwrap();
    bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
}
