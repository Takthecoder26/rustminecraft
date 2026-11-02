use super::*;
use wgpu::util::DeviceExt;

#[test]
fn animation_updates_the_correct_bank_and_interpolates_every_encoded_mip_channel() {
    let instance = crate::gfx::test_gpu::instance();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no adapter for material animation regression");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&Default::default()))
    else {
        eprintln!("no device for material animation regression");
        return;
    };
    let arrays: Vec<_> = (0..2)
        .map(|_| {
            device.create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("Original animated material bank"),
                    size: wgpu::Extent3d {
                        width: 2,
                        height: 2,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 2,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[0; 20],
            )
        })
        .collect();
    let animation = crate::sprites::test_frames(
        1,
        vec![[128, 64, 230, 0].repeat(4), [64, 128, 254, 255].repeat(4)],
        r#"{"frametime":2,"frames":[0,1],"interpolate":true}"#,
    );
    let mut animator = MaterialAnimator::new(&[animation], 2);
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas =
        super::super::pack_atlas::PackAtlas::new(&device, &mut encoder, &arrays, 1, 2, 2).unwrap();
    queue.submit([encoder.finish()]);
    for (tick, expected) in [
        (0, [128, 64, 230, 0]),
        (1, [96, 96, 242, 127]),
        (2, [64, 128, 254, 255]),
    ] {
        let changed = animator.animate(&queue, &arrays, 1, tick).to_vec();
        assert_eq!(changed, [1]);
        let mut encoder = device.create_command_encoder(&Default::default());
        atlas.copy(&mut encoder, &arrays, changed);
        queue.submit([encoder.finish()]);
        for mip in 0..2 {
            assert_eq!(read_pixel(&device, &queue, &arrays[1], mip, 0), expected);
            assert_eq!(read_pixel(&device, &queue, &arrays[0], mip, 0), [0; 4]);
            assert_eq!(
                read_pixel(&device, &queue, atlas.view.texture(), mip, 2 >> mip),
                expected
            );
            assert_eq!(
                read_pixel(&device, &queue, atlas.view.texture(), mip, 0),
                [0; 4]
            );
        }
        assert!(animator.animate(&queue, &arrays, 1, tick).is_empty());
    }
}

fn read_pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    x: u32,
) -> [u8; 4] {
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d { x, y: 0, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    read.slice(..).get_mapped_range().unwrap()[..4]
        .try_into()
        .unwrap()
}
