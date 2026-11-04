//! Original weather controls: both precipitation kinds keep depth testing while the pack
//! independently chooses whether their visible fragments write depth.
use super::destination_tests::Fixture;
use super::world_target_tests::{clear, frame, render, texture};
use super::*;
use crate::hud::HudBatch;
use std::io::{Cursor, Write};
use wgpu::TextureFormat as T;
use wgpu::util::DeviceExt;

fn pack(property: &str, fallback: bool) -> Post {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let weather = if fallback {
        "gbuffers_textured"
    } else {
        "gbuffers_weather"
    };
    for (name, source) in [
        (
            "shaders.properties".to_owned(),
            format!("{property}\nblend.{weather}=off\n"),
        ),
        (
            format!("{weather}.vsh"),
            "#version 120\nvoid main(){gl_Position=gl_Vertex;}".into(),
        ),
        (
            format!("{weather}.fsh"),
            "#version 120\nvoid main(){gl_FragColor=vec4(0.0,0.0,0.4,1.0);}".into(),
        ),
        (
            "final.vsh".to_owned(),
            "#version 120\nvarying vec2 uv;void main(){gl_Position=ftransform();uv=gl_MultiTexCoord0.xy;}".into(),
        ),
        (
            "final.fsh".to_owned(),
            "#version 120\nvarying vec2 uv;uniform sampler2D depthtex0;uniform sampler2D depthtex1;uniform sampler2D colortex0;\nvoid main(){gl_FragColor=vec4(texture2D(depthtex0,uv).r,texture2D(depthtex1,uv).r,texture2D(colortex0,uv).b,1.0);}".into(),
        ),
    ] {
        zip.start_file(
            format!("shaders/{name}"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = horizon_shaders::Pack::from_zip_bytes(
        "original-weather-depth-control",
        zip.finish().unwrap().into_inner(),
    )
    .unwrap();
    let mut macros = horizon_shaders::Macros::standard();
    let (properties, _) = horizon_shaders::read_properties(&pack, &mut macros);
    let inputs = horizon_shaders::GameInputs {
        weather: Some(super::super::pack_weather::inputs()),
        ..Default::default()
    };
    let post = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
    assert!(post.notes.is_empty(), "{:?}", post.notes);
    assert_eq!(post.weather.as_ref().unwrap().name, weather);
    post
}

#[test]
fn weather_depth_uses_effective_properties_with_both_program_paths() {
    // This runs without an adapter: conditionals and last-definition handling reach the
    // loaded weather program, including its ordinary fallback.
    for fallback in [false, true] {
        for (properties, expected) in [
            ("", false),
            ("rain.depth=false", false),
            ("rain.depth = true", true),
            ("rain.depth=invalid", false),
            ("rain.depth=true\nrain.depth=false", false),
            (
                "#define RAIN_WRITES\n#ifdef RAIN_WRITES\nrain.depth=true\n#else\nrain.depth=false\n#endif",
                true,
            ),
        ] {
            assert_eq!(pack(properties, fallback).rain_depth, expected);
        }
    }
}

#[test]
fn weather_depth_writes_and_occlusion_reach_the_rendered_output() {
    let Some(gpu) = Fixture::new() else {
        assert_ne!(
            std::env::var("HORIZON_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "required weather GPU control has no adapter"
        );
        eprintln!("weather depth GPU control skipped: no adapter; CPU controls remain active");
        return;
    };
    if std::env::var("HORIZON_REQUIRE_GPU").as_deref() == Ok("1") {
        assert_ne!(gpu.adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    }
    for fallback in [false, true] {
        for property in ["", "rain.depth=false", "rain.depth=true"] {
            let lightmap = texture(&gpu.device, [1, 1], T::Rgba8Unorm);
            let mut post = PackPost::new(
                &gpu.device,
                &gpu.queue,
                &gpu.adapter,
                pack(property, fallback),
                T::Rgba8Unorm,
                World {
                    atlas: None,
                    material_maps: None,
                    lightmap,
                    terrain: None,
                    block_ids: &[],
                    celestial: None,
                    gui_size: [4, 2],
                    clouds: None,
                    weather: true,
                    hand: false,
                },
            );
            assert!(post.has_weather(), "{:?}", post.notes);
            assert!(post.notes.is_empty(), "{:?}", post.notes);
            for size in [[4, 2], [6, 3]] {
                for hidden in [false, true] {
                    let color = texture(&gpu.device, size, T::Rgba8Unorm);
                    let depth = texture(&gpu.device, size, T::Depth32Float);
                    // Reverse Z 0.2 becomes pack depth 0.8. Hidden precipitation is behind it.
                    clear(
                        &gpu.device,
                        &gpu.queue,
                        &color,
                        &depth,
                        [0, 0, 32, 255],
                        0.2,
                    );
                    post.set_weather(Some(weather(&gpu.device, hidden)));
                    let pixels = render(
                        &gpu.device,
                        &gpu.queue,
                        &mut post,
                        &color,
                        &depth,
                        &frame(&HudBatch::new(size)),
                        size,
                    );
                    for (i, got) in pixels.iter().enumerate() {
                        let left = i % (size[0] as usize) < size[0] as usize / 2;
                        let expected_depth = if !hidden && property == "rain.depth=true" {
                            if left { 64 } else { 128 }
                        } else {
                            204
                        };
                        let expected = [expected_depth, 204, if hidden { 32 } else { 102 }, 255];
                        assert!(
                            got.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 1),
                            "fallback={fallback}, {property:?}, {size:?}, hidden={hidden}, pixel={i}: {got:?}, expected {expected:?}"
                        );
                    }
                }
            }
        }
    }
}

fn weather(device: &wgpu::Device, hidden: bool) -> Weather {
    let mut vertices = Vec::new();
    for (left, right, z) in [(-1.0, 0.0, -0.5), (0.0, 1.0, 0.0)] {
        for (x, y) in [(left, -1.0), (right, -1.0), (right, 1.0), (left, 1.0)] {
            vertices.push(crate::mesher::Vertex {
                pos: [x, y, if hidden { 0.9 } else { z }],
                uv: [0.0; 2],
                layer: 0,
                color: u32::MAX,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0; 2],
            });
        }
    }
    let buffer = |bytes: &[u8], usage| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Original weather depth fixture"),
            contents: bytes,
            usage,
        })
    };
    Weather {
        vertices: buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX),
        indices: buffer(
            bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]),
            wgpu::BufferUsages::INDEX,
        ),
        quads: [0..1, 1..2],
        sprites: [None, None],
        generation: 1,
    }
}
