//! Original pack-load and GPU storage readback control; no game resources.
use super::*;
use std::io::{Cursor, Write};

const SEED: &[u8] = &[0, 0, 0, 0, 255, 128, 127, 0, 1, 2, 3, 4, 254, 253, 252, 251];

fn loaded() -> horizon_shaders::Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, contents) in [
        (
            "shaders/shaders.properties",
            b"bufferObject.0=32 data/seed.bin\nbufferObject.3=4 true 1 1\n".as_slice(),
        ),
        ("shaders/data/seed.bin", SEED),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-binary-buffer-control",
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
#[ignore = "requires a GPU adapter"]
fn binary_initialization_reaches_gpu_and_survives_resize_until_reload() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU adapter");
    eprintln!("binary buffer adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let make = || {
        let post = loaded();
        Buffers::new(
            &device,
            post.buffer_objects,
            post.buffer_initializers,
            Default::default(),
            wgpu::ShaderStages::COMPUTE,
        )
    };
    let mut buffers = make();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Original storage seed control"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@group(0) @binding(0) var<storage, read_write> seed: array<u32>;
@group(1) @binding(0) var<storage, read_write> result: array<u32>;
@compute @workgroup_size(1) fn main() {
    for (var i=0u; i<8u; i++) { result[i]=seed[i]; }
    seed[0]+=1u;
}
"#
            .into(),
        ),
    });
    let result_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&buffers.layout), Some(&result_layout)],
        ..Default::default()
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 32,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &result_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    let run = |buffers: &mut Buffers, size| {
        buffers.resize(&device, size);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, buffers.group.as_ref().unwrap(), &[]);
            pass.set_bind_group(1, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &read, 0, 32);
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let actual: Vec<_> = read
            .slice(..)
            .get_mapped_range()
            .unwrap()
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        read.unmap();
        actual
    };
    let original: Vec<_> = SEED
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .chain([0; 4])
        .collect();
    assert_eq!(run(&mut buffers, [2, 2]), original);
    let mut mutated = original.clone();
    mutated[0] += 1;
    assert_eq!(
        run(&mut buffers, [4, 3]),
        mutated,
        "fixed data must survive relative-buffer resize"
    );
    let mut reloaded = make();
    assert_eq!(
        run(&mut reloaded, [4, 3]),
        original,
        "reload restores the binary seed"
    );
}
