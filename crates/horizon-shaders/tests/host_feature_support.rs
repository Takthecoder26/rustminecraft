//! Original host availability and optional shader-path selection controls.
use horizon_shaders::{Macros, Properties};

fn request(name: &str, macros: &Macros) -> Properties {
    Properties::parse(
        &format!("iris.features.optional={name}\niris.features.required={name}"),
        macros,
    )
}

#[test]
fn host_availability_controls_required_and_optional_shader_paths() {
    for supported in [false, true] {
        let mut macros = Macros::standard();
        macros.define_supported_feature("PER_BUFFER_BLENDING", supported);
        let notes = macros.define_features(&request("PER_BUFFER_BLENDING", &macros));
        assert_eq!(notes.is_empty(), supported, "{notes:?}");
        let selected = Properties::parse(
            "#ifdef IRIS_FEATURE_PER_BUFFER_BLENDING\npath=independent\n#else\npath=fallback\n#endif",
            &macros,
        );
        assert_eq!(
            selected.get("path"),
            Some(if supported { "independent" } else { "fallback" })
        );
    }
}

#[test]
fn supported_features_still_require_an_optional_request_for_the_macro() {
    let mut macros = Macros::standard();
    macros.define_supported_feature("PER_BUFFER_BLENDING", true);
    assert!(macros.define_features(&Properties::default()).is_empty());
    assert!(!macros.is_defined("IRIS_FEATURE_PER_BUFFER_BLENDING"));
    let required = Properties::parse("iris.features.required=PER_BUFFER_BLENDING", &macros);
    assert!(macros.define_features(&required).is_empty());
    assert!(!macros.is_defined("IRIS_FEATURE_PER_BUFFER_BLENDING"));
}

#[test]
fn disabled_host_features_override_defaults_and_drop_stale_optional_macros() {
    let mut macros = Macros::standard();
    let properties = request("COMPUTE_SHADERS", &macros);
    assert!(macros.define_features(&properties).is_empty());
    assert!(macros.is_defined("IRIS_FEATURE_COMPUTE_SHADERS"));
    macros.define_supported_feature("COMPUTE_SHADERS", false);
    let notes = macros.define_features(&properties);
    assert_eq!(
        notes,
        ["the pack requires `COMPUTE_SHADERS`, which is not supported yet"]
    );
    assert!(!macros.is_defined("IRIS_FEATURE_COMPUTE_SHADERS"));
}
