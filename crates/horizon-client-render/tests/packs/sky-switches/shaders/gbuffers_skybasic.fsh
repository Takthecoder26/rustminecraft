#version 130
flat in int marker;
void main() { if (int(gl_FragCoord.x) / 8 != marker) discard; gl_FragData[0] = vec4(1.0); }
