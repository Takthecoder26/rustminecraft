use super::*;

/// Exercise the production input body, raw vertex/sidecar addressing and bind layout.
/// The first live quad begins at arena vertex 256, after an unused allocation.
#[test]
fn terrain_provenance_reads_the_bound_arena_records() {
    use crate::mesher::{SectionMesh, TerrainVertex};
    use wgpu::util::DeviceExt;
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("provenance render requires an adapter");
    eprintln!("terrain provenance GPU: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut copy = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut copy, &[texture], 1, 1, 16).unwrap();
    queue.submit([copy.finish()]);
    let inputs = atlas.terrain_inputs(true);
    let attribute = |name: &str| &inputs.attributes.iter().find(|(n, _)| n == name).unwrap().1;
    let source = format!(
        r#"#version 450
{}
vec4 hz_Vertex,hz_Color,hz_MultiTexCoord0,hz_MultiTexCoord1,hz_MultiTexCoord2;
vec3 hz_Normal;
layout(location=0) out vec4 report;
void main() {{
{}
 vec4 entity={}; vec4 mid={};
 vec3 center=gl_VertexIndex>=512 ? mid.xyz : hz_Vertex.xyz+mid.xyz/64.0;
 report=vec4(entity.x,mid.w,center.x,center.z);
 vec2 corners[4]=vec2[4](vec2(-1,-1),vec2(-1,1),vec2(-0.5,1),vec2(-0.5,-1));
 uint index=uint(gl_VertexIndex); index=index>=768u ? index-768u+12u : (index>=512u ? index-512u+8u : index-256u);
 gl_Position=vec4(corners[index%4u]+vec2(float(index/4u)*0.5,0.0),0.0,1.0);
}}"#,
        inputs.declarations,
        inputs.body,
        attribute("mc_Entity"),
        attribute("at_midBlock")
    );
    let module = naga::front::glsl::Frontend::default()
        .parse(
            &naga::front::glsl::Options::from(naga::ShaderStage::Vertex),
            &source,
        )
        .unwrap();
    let vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Production terrain provenance inputs"),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
    });
    let fs=device.create_shader_module(wgpu::ShaderModuleDescriptor {label:None,source:wgpu::ShaderSource::Wgsl("@fragment fn main(@location(0) report:vec4<f32>)->@location(0) vec4<f32>{return report;}".into())});
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
    let origins_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[storage(0), storage(1)],
    });
    let (ids_layout, ids) =
        crate::gfx::pack_post::block_ids(&device, &[2, 101, -1, 0, 102, -1, 15, 99, -1, 7]);
    let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[],
    });
    let empty_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &empty,
        entries: &[],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[
            Some(&empty),
            Some(&origins_layout),
            Some(&empty),
            Some(&ids_layout),
        ],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &vs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TerrainVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &crate::gfx::TERRAIN_ATTRIBUTES,
            })],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fs,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let buffer = |bytes: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytes,
            usage,
        })
    };
    let mut mesh = SectionMesh {
        provenance: Some(Default::default()),
        ..Default::default()
    };
    for (state, x, width, y) in [(0, 3., 1., 4.25), (1, 4.5, 0.5, 4.5)] {
        let before = [mesh.solid.len(), 0];
        mesh.solid.extend(
            [
                [x, y, 5.],
                [x, y, 6.],
                [x + width, y, 6.],
                [x + width, y, 5.],
            ]
            .map(|pos| TerrainVertex::new(pos, [0.; 2], 0, 0, 0)),
        );
        mesh.record_provenance(before, state, [3, 4, 5]);
    }
    mesh.sort_solid();
    let records = &mesh.provenance.as_ref().unwrap()[0];
    assert_eq!(
        [records[0][0], records[1][0]],
        [1, 0],
        "fixture must reorder its quads"
    );
    let mut vertices = vec![TerrainVertex::new([0.; 3], [0.; 2], 0, 0, 0); 256];
    vertices.extend_from_slice(&mesh.solid);
    for start in [512, 768] {
        vertices.resize(start, TerrainVertex::new([0.; 3], [0.; 2], 0, 0, 0));
        vertices.extend(
            [
                [30., 40., 50.],
                [30., 40., 51.],
                [31., 40., 51.],
                [31., 40., 50.],
            ]
            .map(|pos| TerrainVertex::tiled(pos, [0.; 2], 0, 0, 0)),
        );
    }
    let vertices = buffer(
        bytemuck::cast_slice(&vertices),
        wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
    );
    // These records deliberately do not follow sprite identity (both sprites are zero).
    let mut provenance = vec![[0u32; 2]; 193];
    provenance[64..66].copy_from_slice(records);
    let provenance = buffer(
        bytemuck::cast_slice(&provenance),
        wgpu::BufferUsages::STORAGE,
    );
    let slots = buffer(
        bytemuck::cast_slice(&[0u32, 1, 2, 3]),
        wgpu::BufferUsages::STORAGE,
    );
    let origins = buffer(
        bytemuck::cast_slice(&[
            [0f32; 4],
            [100., 200., 300., 1. / 32768.],
            [100., 200., 300., 1. / 32.],
            [100., 200., 300., 1. / 32768.],
        ]),
        wgpu::BufferUsages::STORAGE,
    );
    let indices = buffer(
        bytemuck::cast_slice(&[
            0u32, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 256, 257, 258, 256, 258, 259, 512, 513, 514,
            512, 514, 515,
        ]),
        wgpu::BufferUsages::INDEX,
    );
    fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: buffer.as_entire_binding(),
        }
    }
    let origins = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &origins_layout,
        entries: &[entry(0, &slots), entry(1, &origins)],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &ids_layout,
        entries: &[entry(0, &ids), entry(1, &vertices), entry(2, &provenance)],
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 8,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
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
        pass.set_bind_group(0, &empty_group, &[]);
        pass.set_bind_group(1, &origins, &[]);
        pass.set_bind_group(2, &empty_group, &[]);
        pass.set_bind_group(3, &group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..24, 256, 0..1);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: 8,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = read.slice(..).get_mapped_range().unwrap();
    let pixels: &[[f32; 4]] = bytemuck::cast_slice(&data[..128]);
    eprintln!("terrain provenance readback: {pixels:?}");
    assert_eq!(
        pixels,
        &[
            [102., 15., 103.5, 305.5],
            [102., 15., 103.5, 305.5],
            [101., 0., 103.5, 305.5],
            [101., 0., 103.5, 305.5],
            [99., 7., 0., 0.],
            [99., 7., 0., 0.],
            [-1., 0., 0., 0.],
            [-1., 0., 0., 0.],
        ]
    );
}
