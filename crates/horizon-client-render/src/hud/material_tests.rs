use super::*;

#[test]
fn namespaced_material_companions_do_not_join_directory_colour_sprites() {
    let root =
        std::env::temp_dir().join(format!("horizon-material-discovery-{}", std::process::id()));
    let atlas = root.join("assets/minecraft/atlases");
    let images = root.join("assets/probe/textures/block");
    fs::create_dir_all(&atlas).unwrap();
    fs::create_dir_all(&images).unwrap();
    fs::write(
        atlas.join("blocks.json"),
        r#"{"sources":[{"type":"directory","source":"block","prefix":"block/"}]}"#,
    )
    .unwrap();
    for name in ["stone", "stone_n", "stone_s", "direction_n"] {
        let mut png = png::Encoder::new(
            fs::File::create(images.join(format!("{name}.png"))).unwrap(),
            2,
            2,
        );
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&[123; 16])
            .unwrap();
    }
    let (images, _) = atlas_sprites(&root, &[]);
    let names: Vec<_> = images.iter().map(|image| image.0.as_str()).collect();
    assert_eq!(names, ["probe:block/direction_n", "probe:block/stone"]);
    fs::remove_dir_all(root).unwrap();
}
