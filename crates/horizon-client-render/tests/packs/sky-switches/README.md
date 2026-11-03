# Original sky stage markers

Asset-free regression pack, MIT. The vertex shader paints six independent screen stripes using `renderStage` and `gl_VertexID`; no texture or Minecraft resource is read. The fragment shader clips each oversized triangle to its eight-pixel stripe. The test explicitly disables blending to isolate whether each geometry draw happens.

The expected switch masks come from the public shader-format documentation: `sky` controls SKY/SUNSET/VOID, `stars` controls STARS, and `sun` and `moon` independently control their celestial draws. This fixture does not establish horizon geometry or named-pack visual parity.

Run `cargo test -p horizon-client-render --lib sky_feature_switches_control_independent_gpu_stage_markers -- --ignored --nocapture` with isolated scratch homes and a GPU. Missing GPU setup fails this explicit run rather than silently passing.
