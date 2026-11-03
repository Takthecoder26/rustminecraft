//! Original macro-rescanning controls.
use horizon_shaders::Macros;

#[test]
fn object_alias_invokes_function_macro_with_following_arguments() {
    let mut macros = Macros::default();
    macros.define_function("read", vec!["value".into()], "((value)+1)");
    macros.define("alias", "read");
    assert_eq!(macros.expand("alias(2)"), "((2)+1)");
}

#[test]
fn direct_function_macro_control() {
    let mut macros = Macros::default();
    macros.define_function("read", vec!["value".into()], "((value)+1)");
    assert_eq!(macros.expand("read(2)"), "((2)+1)");
}

#[test]
fn object_alias_without_call_is_not_invoked() {
    let mut macros = Macros::default();
    macros.define_function("read", vec!["value".into()], "((value)+1)");
    macros.define("alias", "read");
    assert_eq!(macros.expand("alias + 2"), "read + 2");
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn increment() -> Macros {
    let mut macros = Macros::default();
    macros.define_function("increment", vec!["value".into()], "((value)+1)");
    macros
}

#[test]
fn alias_chains_rescan_at_the_original_argument_boundary() {
    let mut macros = increment();
    macros.define("alias", "intermediate");
    macros.define("intermediate", "increment");
    assert_eq!(compact(&macros.expand("alias(2)")), "((2)+1)");
    assert_eq!(compact(&macros.expand("alias(alias(2))")), "((((2)+1))+1)");
    assert_eq!(
        compact(&macros.expand("alias(1) alias(2)")),
        "((1)+1)((2)+1)"
    );
}

#[test]
fn function_replacements_and_arguments_can_produce_a_callable_name() {
    let mut macros = increment();
    macros.define_function("callable", vec![], "increment");
    macros.define_function("identity", vec!["value".into()], "value");
    assert_eq!(compact(&macros.expand("callable()(2)")), "((2)+1)");
    assert_eq!(compact(&macros.expand("identity(increment)(2)")), "((2)+1)");
}

#[test]
fn token_pasting_can_produce_a_callable_name() {
    let mut macros = increment();
    macros.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
    assert_eq!(compact(&macros.expand("CAT(incre,ment)(2)")), "((2)+1)");
    assert_eq!(compact(&macros.expand("CAT(,increment)(2)")), "((2)+1)");
    assert_eq!(compact(&macros.expand("CAT(increment,)(2)")), "((2)+1)");
}

#[test]
fn compound_object_replacements_rescan_their_final_name() {
    let mut macros = increment();
    macros.define("alias", "1 + increment");
    assert_eq!(compact(&macros.expand("alias(2)")), "1+((2)+1)");
}

#[test]
fn self_and_mutual_recursion_remain_suppressed_per_token() {
    let mut macros = Macros::default();
    macros.define("SELF", "SELF+1");
    assert_eq!(compact(&macros.expand("SELF SELF")), "SELF+1SELF+1");
    macros.define("A", "B");
    macros.define("B", "A");
    assert_eq!(macros.expand("A"), "A");
    macros.define_function("F", vec!["value".into()], "G(value)");
    macros.define_function("G", vec!["value".into()], "F(value)");
    assert_eq!(macros.expand("F(2)"), "F(2)");
}

#[test]
fn recursively_named_alias_is_not_reexpanded_after_rescanning() {
    let mut macros = Macros::default();
    macros.define("A", "F");
    macros.define_function("F", vec!["value".into()], "A");
    assert_eq!(macros.expand("A(1)"), "F");
}

#[test]
fn multiline_alias_call_preprocesses_and_translates_to_the_expected_constant() {
    use horizon_shaders::{Pack, Target, preprocess, translate_composite};
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, source) in [
        (
            "shaders/final.vsh",
            "#version 440 compatibility\nvoid main() { gl_Position = gl_Vertex; }\n",
        ),
        (
            "shaders/final.fsh",
            "#version 440 compatibility\n#define increment(value) ((value)+1)\n#define alias increment\nconst int result = alias(\n2\n);\nvoid main() { gl_FragColor = vec4(float(result)); }\n",
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(source.as_bytes()).unwrap();
    }
    let pack = Pack::from_zip_bytes("original.zip", zip.finish().unwrap().into_inner()).unwrap();
    let vertex = preprocess(&pack, "final.vsh", &Macros::default());
    let fragment = preprocess(&pack, "final.fsh", &Macros::default());
    assert!(
        fragment.diagnostics.is_empty(),
        "{:?}",
        fragment.diagnostics
    );
    let program =
        translate_composite(&vertex, &fragment, Target::Window).unwrap_or_else(|e| panic!("{e:?}"));
    let result = program
        .fragment
        .constants
        .iter()
        .find_map(|(_, c)| (c.name.as_deref() == Some("result")).then_some(c))
        .unwrap();
    assert!(matches!(
        program.fragment.global_expressions[result.init],
        naga::Expression::Literal(naga::Literal::I32(3))
    ));
    for module in [&program.vertex, &program.fragment] {
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(module)
        .unwrap();
        naga::back::spv::write_vec(module, &info, &naga::back::spv::Options::default(), None)
            .unwrap();
    }
}

#[test]
fn a_suppressed_argument_name_stays_suppressed_when_reinserted() {
    let mut macros = Macros::default();
    macros.define_function("identity", vec!["value".into()], "value");
    macros.define("alias", "identity");
    assert_eq!(macros.expand("identity(alias)(2)"), "identity(2)");
    assert_eq!(macros.expand("identity(identity)(2)"), "identity(2)");
}

#[test]
fn pasting_joins_adjacent_edge_tokens_and_rescans_chained_results() {
    let mut macros = increment();
    macros.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
    assert_eq!(
        macros.expand("CAT(first second, third fourth)"),
        "first secondthird fourth"
    );
    macros.define("N", "7");
    macros.define("NZ", "13");
    macros.define_function("MIX", vec!["value".into()], "value ## Z + value");
    assert_eq!(macros.expand("MIX(N)"), "13 + 7");
    macros.define_function(
        "JOIN",
        vec!["a".into(), "b".into(), "c".into()],
        "a ## b ## c",
    );
    assert_eq!(compact(&macros.expand("JOIN(in,cre,ment)(2)")), "((2)+1)");
}
