use super::*;
fn png(path: &Path, width: u32, height: u32, pixels: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = png::Encoder::new(std::fs::File::create(path).unwrap(), width, height);
    writer.set_color(png::ColorType::Rgba);
    writer.set_depth(png::BitDepth::Eight);
    writer
        .write_header()
        .unwrap()
        .write_image_data(pixels)
        .unwrap();
}
#[test]
fn material_frames_follow_the_timeline_and_static_companions_survive() {
    let root = std::env::temp_dir().join(format!("horizon-pbr-frames-{}", std::process::id()));
    let base = root.join("stone.png");
    let mut frames = [32, 128, 255, 255].repeat(16 * 16);
    frames.extend([224, 128, 255, 64].repeat(16 * 16));
    png(&root.join("stone_n.png"), 16, 32, &frames);
    png(
        &root.join("stone_s.png"),
        16,
        16,
        &[17, 232, 79, 254].repeat(16 * 16),
    );
    let inherited = AnimationMetadata::from_json(r#"{"frametime":2,"frames":[1,0]}"#).unwrap();
    let normal = material_map(&base, "_n", Some(&inherited), 2).unwrap();
    assert_eq!(normal.frames.len(), 2);
    let clip = normal.clip.unwrap();
    assert_eq!((clip.sample(0).frame, clip.sample(2).frame), (1, 0));
    assert_eq!(&normal.frames[1][..4], &[224, 128, 255, 64]);
    let specular = material_map(&base, "_s", Some(&inherited), 2).unwrap();
    assert!(specular.clip.is_none());
    assert_eq!(&specular.frames[0][..4], &[17, 232, 79, 254]);
    std::fs::write(
        root.join("stone_n.png.mcmeta"),
        r#"{"animation":{"frametime":3,"interpolate":true,"frames":[0,1]}}"#,
    )
    .unwrap();
    let own = material_map(&base, "_n", Some(&inherited), 2).unwrap();
    let animation = SpriteAnimation {
        layer: 0,
        frames: own.frames,
        clip: own.clip.unwrap(),
    };
    assert_eq!(
        animation.blend(1),
        FrameBlend {
            frame: 0,
            next: 1,
            millis: 333
        }
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn connected_texture_files_load_their_own_material_companions() {
    let root = std::env::temp_dir().join(format!("hz-ctm-material-{}", std::process::id()));
    png(
        &root.join("assets/minecraft/optifine/ctm/glass/0_n.png"),
        16,
        16,
        &[17, 128, 254, 99].repeat(256),
    );
    let maps = read_material_maps(&root, "assets/minecraft/optifine/ctm/glass/0.png", None, 1);
    assert_eq!(
        &maps[0].as_ref().unwrap().frames[0][..4],
        &[17, 128, 254, 99]
    );
    assert!(maps[1].is_none());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn high_resolution_materials_share_colour_geometry_without_changing_encoding() {
    let mut sprites = Sprites {
        native: vec![
            (16, Arc::new(vec![[90, 100, 110, 255].repeat(16 * 16)]), 0),
            (16, Arc::new(vec![[90, 100, 110, 255].repeat(16 * 16)]), 0),
        ],
        pbr: vec![
            [
                Some((128, [64, 192, 211, 87].repeat(128 * 128))),
                Some((128, [52, 231, 100, 254].repeat(128 * 128))),
            ],
            [None, None],
        ],
        ..Default::default()
    };
    sprites.finish();
    assert_eq!(sprites.size, 128);
    assert_eq!(sprites.pbr_size, sprites.size);
    assert_eq!(&sprites.normals[..4], &[64, 192, 211, 87]);
    assert_eq!(&sprites.specular[..4], &[52, 231, 100, 254]);
    assert_eq!(&sprites.normals[128 * 128 * 4..][..4], &FLAT_NORMAL);
    assert_eq!(sprites.normals.len(), sprites.pixels.len());
    assert_eq!(sprites.specular.len(), sprites.pixels.len());
}

#[test]
fn the_shared_budget_includes_colour_and_both_material_maps() {
    let mut sprites = Sprites {
        native: vec![(16, Arc::new(vec![vec![0; 16 * 16 * 4]]), 0); 1300],
        pbr: vec![[None, None]; 1300],
        ..Default::default()
    };
    sprites.pbr[0][0] = Some((512, vec![128; 512 * 512 * 4]));
    // The server layout chooses the same geometry without allocating three large arrays.
    sprites.finish_without_pixels();
    assert_eq!(sprites.size, 128);
    let bytes = 1300_u64 * u64::from(sprites.size).pow(2) * 4 * 3;
    assert!(bytes <= DEFAULT_TEXTURE_BUDGET);
    assert!(bytes * 4 > DEFAULT_TEXTURE_BUDGET);
    assert!(sprites.pixels.is_empty() && sprites.normals.is_empty() && sprites.specular.is_empty());
}

#[test]
fn inherited_rectangular_frames_use_physical_capacity_and_the_maps_pixel_scale() {
    let root = std::env::temp_dir().join(format!("horizon-material-grid-{}", std::process::id()));
    let inherited =
        AnimationMetadata::from_json(r#"{"width":16,"height":8,"frametime":2,"frames":[2,0,2]}"#)
            .unwrap();
    let colours = [
        [11, 22, 33, 44],
        [55, 66, 77, 88],
        [99, 110, 121, 132],
        [143, 154, 165, 176],
    ];
    for (width, height) in [(16, 32), (32, 64)] {
        let name = format!("grid_{width}");
        let pixels: Vec<_> = colours
            .iter()
            .flat_map(|pixel| pixel.repeat((width * height / 4) as usize))
            .collect();
        png(&root.join(format!("{name}_n.png")), width, height, &pixels);
        let map =
            material_map(&root.join(format!("{name}.png")), "_n", Some(&inherited), 4).unwrap();
        assert_eq!(map.frames.len(), 4);
        assert_eq!(map.side, width);
        for (frame, colour) in map.frames.iter().zip(colours) {
            assert!(
                frame
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| *pixel == colour)
            );
        }
        let clip = map.clip.unwrap();
        assert_eq!(
            [
                clip.sample(0).frame,
                clip.sample(2).frame,
                clip.sample(4).frame
            ],
            [2, 0, 2]
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn square_companions_stay_static_until_their_own_metadata_declares_a_grid() {
    let root = std::env::temp_dir().join(format!("horizon-material-square-{}", std::process::id()));
    let inherited =
        AnimationMetadata::from_json(r#"{"width":16,"height":16,"frames":[0,1,2,3]}"#).unwrap();
    let colours = [
        [11, 22, 33, 44],
        [55, 66, 77, 88],
        [99, 110, 121, 132],
        [143, 154, 165, 176],
    ];
    let mut pixels = Vec::new();
    for y in 0..32 {
        for x in 0..32 {
            pixels.extend(colours[(y / 16) * 2 + x / 16]);
        }
    }
    png(&root.join("grid_n.png"), 32, 32, &pixels);
    let base = root.join("grid.png");
    let map = material_map(&base, "_n", Some(&inherited), 4).unwrap();
    assert!(map.clip.is_none());
    assert_eq!(map.frames.len(), 1);
    assert_eq!(map.frames[0], pixels);
    std::fs::write(
        root.join("grid_n.png.mcmeta"),
        r#"{"animation":{"width":16,"height":16,"frametime":2,"frames":[3,1]}}"#,
    )
    .unwrap();
    let map = material_map(&base, "_n", Some(&inherited), 4).unwrap();
    assert_eq!(map.frames.len(), 4);
    assert_eq!(map.side, 16);
    for (frame, colour) in map.frames.iter().zip(colours) {
        assert!(
            frame
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == colour)
        );
    }
    let clip = map.clip.unwrap();
    assert_eq!([clip.sample(0).frame, clip.sample(2).frame], [3, 1]);
    std::fs::remove_dir_all(root).unwrap();
}
