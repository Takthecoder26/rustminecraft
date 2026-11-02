use super::*;

#[test]
fn material_format_metadata_defines_only_the_advertised_labpbr_macros() {
    let root = std::env::temp_dir().join(format!("horizon-material-format-{}", std::process::id()));
    let folder = root.join("assets/minecraft/optifine");
    std::fs::create_dir_all(&folder).unwrap();
    for (format, labpbr, version) in [
        ("lab-pbr/1.3", true, true),
        ("lab-pbr", true, false),
        ("lab-pbr/1_3", true, false),
        ("old-pbr/1.3", false, false),
    ] {
        std::fs::write(
            folder.join("texture.properties"),
            format!("format={format}\n"),
        )
        .unwrap();
        let mut macros = Macros::standard();
        define_material_format(&mut macros, &root);
        assert_eq!(
            macros.get("MC_TEXTURE_FORMAT_LAB_PBR").is_some(),
            labpbr,
            "{format}"
        );
        assert_eq!(
            macros.get("MC_TEXTURE_FORMAT_LAB_PBR_1_3").is_some(),
            version,
            "{format}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn material_maps_reconcile_a_stale_startup_selection_once_and_reload_actual_pixels() {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            horizon_client::sprites::set_labpbr_maps(self.0);
        }
    }
    let _restore = Restore(horizon_client::sprites::labpbr_maps());
    let root = std::env::temp_dir().join(format!("horizon-material-reload-{}", std::process::id()));
    let folder = root.join("assets/probe/textures/block");
    std::fs::create_dir_all(&folder).unwrap();
    for (name, colour) in [
        ("stone", [90, 100, 110, 255]),
        ("stone_n", [64, 192, 211, 87]),
    ] {
        let mut png = png::Encoder::new(
            std::fs::File::create(folder.join(format!("{name}.png"))).unwrap(),
            16,
            16,
        );
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&colour.repeat(256))
            .unwrap();
    }
    let load = || {
        let mut sprites = horizon_client::sprites::Sprites::default();
        sprites.try_insert(&root, "probe:block/stone").unwrap();
        sprites.finish();
        sprites.normals
    };
    let mut settings = horizon_client::settings::Settings::default();
    // The worker took a shaders-off snapshot before the startup menu selected a pack.
    horizon_client::sprites::set_labpbr_maps(false);
    assert!(load().is_empty());
    let mut requested = false;
    settings.horizon.shader_pack = "original-standard.zip".into();
    assert!(refresh_material_maps(&settings, &mut requested));
    assert_eq!(&load()[..4], &[64, 192, 211, 87]);
    for _ in 0..3 {
        assert!(
            !refresh_material_maps(&settings, &mut requested),
            "later frames must not queue a reload"
        );
    }
    for pick in ["original-native.hshader", "other-standard.zip"] {
        settings.horizon.shader_pack = pick.into();
        assert!(!refresh_material_maps(&settings, &mut requested));
        assert_eq!(&load()[..4], &[64, 192, 211, 87]);
    }
    settings.horizon.shader_pack.clear();
    assert!(refresh_material_maps(&settings, &mut requested));
    assert!(load().is_empty());
    assert!(!refresh_material_maps(&settings, &mut requested));
    // The converse stale snapshot must also release unused maps exactly once.
    horizon_client::sprites::set_labpbr_maps(true);
    requested = true;
    assert!(refresh_material_maps(&settings, &mut requested));
    assert!(load().is_empty());
    assert!(!refresh_material_maps(&settings, &mut requested));
    // Opposing split views retain separate requests; unchanged selections must not
    // alternate the process-global decode flag or ask for another resource reload.
    let mut host_request = true;
    let mut guest_request = false;
    let mut host = horizon_client::settings::Settings::default();
    host.horizon.shader_pack = "host-standard.zip".into();
    let guest = horizon_client::settings::Settings::default();
    let decode_request = horizon_client::sprites::labpbr_maps();
    for _ in 0..3 {
        assert!(!refresh_material_maps(&host, &mut host_request));
        assert!(!refresh_material_maps(&guest, &mut guest_request));
        assert_eq!(horizon_client::sprites::labpbr_maps(), decode_request);
    }
    std::fs::remove_dir_all(root).unwrap();
}
