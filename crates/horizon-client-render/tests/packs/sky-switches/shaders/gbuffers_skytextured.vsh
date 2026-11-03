#version 130
uniform int renderStage;
flat out int marker;
void main() {
    int stripe = 0;
    if (renderStage == MC_RENDER_STAGE_SUNSET) stripe = 1;
    if (renderStage == MC_RENDER_STAGE_SUN) stripe = 2;
    if (renderStage == MC_RENDER_STAGE_MOON) stripe = 3;
    if (renderStage == MC_RENDER_STAGE_STARS) stripe = 4;
    if (renderStage == MC_RENDER_STAGE_VOID) stripe = 5;
    marker = stripe;
    int corner = int(gl_VertexID) % 3;
    vec2 p = vec2(corner == 1 ? 3.0 : -1.0, corner == 2 ? 3.0 : -1.0);
    gl_Position = vec4((p.x + 1.0) / 6.0 + float(stripe) / 3.0 - 1.0, p.y, 0.0, 1.0);
}
