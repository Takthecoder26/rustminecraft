#[test]
fn shared_sprite_states_keep_distinct_authored_ids_and_light() {
    let ids = horizon_shaders::BlockIds::parse(&horizon_shaders::Properties::parse(
        "block.101=probe:lamp:lit=false\nblock.102=probe:lamp:lit=true\nblock.103=probe:other\nblock.104=minecraft:water:level=3\n",
        &horizon_shaders::Macros::standard(),
    ));
    for (name, key, value, emission, expected) in [
        ("probe:lamp", "lit", "false", 0, [101, -1, 0]),
        ("probe:lamp", "lit", "true", 15, [102, -1, 15]),
        ("probe:other", "lit", "true", 7, [103, -1, 7]),
        ("minecraft:water", "level", "3", 0, [104, 1, 0]),
        ("probe:missing", "lit", "true", 0, [-1, -1, 0]),
    ] {
        let state = horizon_world::chunk::BlockState {
            name: name.into(),
            properties: std::collections::BTreeMap::from([(key.into(), value.into())]),
        };
        assert_eq!(super::state_attributes(&state, &ids, emission), expected);
    }
}

#[test]
fn a_sprite_takes_the_light_of_its_block() {
    let votes = std::collections::BTreeMap::from([
        ((0, 14), (true, 1)),
        ((0, 0), (false, 5)),
        ((1, 13), (false, 4)),
        ((1, 0), (false, 4)),
        ((2, 15), (false, 4)),
        ((2, 0), (false, 5)),
        ((3, 7), (false, 2)),
    ]);
    assert_eq!(super::layer_lights(&votes, 5), [14, 0, 0, 7, 0]);
}

#[test]
fn a_sprite_takes_the_light_of_its_block() {
    let votes = std::collections::BTreeMap::from([
        ((0, 14), (true, 1)),
        ((0, 0), (false, 5)),
        ((1, 13), (false, 4)),
        ((1, 0), (false, 4)),
        ((2, 15), (false, 4)),
        ((2, 0), (false, 5)),
        ((3, 7), (false, 2)),
    ]);
    assert_eq!(super::layer_lights(&votes, 5), [14, 0, 0, 7, 0]);
}
