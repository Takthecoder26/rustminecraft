//! Section visibility readback through the shared terrain-input probe.

use super::{PackAtlas, TerrainVertex, UV_STEPS, sample};

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
                                &atlas,
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
                        &atlas,
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
