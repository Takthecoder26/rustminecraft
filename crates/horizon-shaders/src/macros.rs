//! Object-like and function-like macros in GLSL stages, conditions and property values.

use std::collections::BTreeMap;

/// The Iris feature flags (`iris.features.*`) Horizon has. `BLOCK_EMISSION_ATTRIBUTE`: terrain's
/// `at_midBlock.w` is the light its block gives. `SEPARATE_HARDWARE_SAMPLERS` (ShaderDoc,
/// "Separate Hardware Shadow Samplers"): `shadowtex0`/`shadowtex1` stay plain depth samplers when
/// hardware filtering is on, `shadowtex0HW`/`shadowtex1HW` compare; Horizon picks a comparison
/// sampler by the declared type (`sampler2DShadow`), so they always are. `HIGHER_SHADOWCOLOR`:
/// `shadowcolor2` to `shadowcolor7` beside the first two.
const FEATURES: [&str; 8] = [
    "BLOCK_EMISSION_ATTRIBUTE",
    "COMPUTE_SHADERS",
    "CUSTOM_IMAGES",
    "ENTITY_TRANSLUCENT",
    "HIGHER_SHADOWCOLOR",
    "REVERSED_CULLING",
    "SEPARATE_HARDWARE_SAMPLERS",
    "SSBO",
];

/// Whether `name` is one of [`FEATURES`], in any case.
fn is_feature(name: &str) -> bool {
    FEATURES.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// `MC_VERSION` of the version Horizon plays: 26.3 → 260300.
pub const MC_VERSION: u32 = mc_version(horizon_core::version::VERSION_ID);

/// The documented rule, `major * 10000 + minor * 100 + patch` (spec 1.5.1, OQ-1): `1.21` → 12100,
/// `1.7.10` → 10710. Parts that are not digits count as 0.
pub const fn mc_version(id: &str) -> u32 {
    let bytes = id.as_bytes();
    let (mut parts, mut part, mut i) = ([0u32; 3], 0, 0);
    while i < bytes.len() && part < 3 {
        match bytes[i] {
            b'.' => part += 1,
            b @ b'0'..=b'9' => parts[part] = parts[part] * 10 + (b - b'0') as u32,
            _ => break,
        }
        i += 1;
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

#[derive(Clone, Debug, PartialEq)]
struct Macro {
    /// `None` for an object-like macro; the parameter names of a function-like one.
    params: Option<Vec<String>>,
    body: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(BTreeMap<String, Macro>);

/// `MC_RENDER_STAGE_*` in the order the stages draw (spec 5.4); each is its index (OQ-7).
pub const RENDER_STAGES: [&str; 24] = [
    "NONE",
    "SKY",
    "SUNSET",
    "CUSTOM_SKY",
    "SUN",
    "MOON",
    "STARS",
    "VOID",
    "TERRAIN_SOLID",
    "TERRAIN_CUTOUT_MIPPED",
    "TERRAIN_CUTOUT",
    "ENTITIES",
    "BLOCK_ENTITIES",
    "DESTROY",
    "OUTLINE",
    "DEBUG",
    "HAND_SOLID",
    "TERRAIN_TRANSLUCENT",
    "TRIPWIRE",
    "PARTICLES",
    "CLOUDS",
    "RAIN_SNOW",
    "WORLD_BORDER",
    "HAND_TRANSLUCENT",
];

/// The value of `MC_RENDER_STAGE_<name>` (OQ-7: its index in `RENDER_STAGES`).
pub fn render_stage(name: &str) -> u32 {
    RENDER_STAGES
        .iter()
        .position(|s| *s == name)
        .unwrap_or_else(|| panic!("no render stage {name}")) as u32
}

impl Macros {
    /// The standard macros that need no renderer (spec 1.5.1): `MC_VERSION`, this system's
    /// `MC_OS_*`, the GL and GLSL versions Iris reports, the render stages and the host
    /// settings at their defaults. The GPU vendor and family arrive with the renderer
    /// (`define_renderer`).
    pub fn standard() -> Self {
        let mut m = Self::default();
        m.define("MC_VERSION", MC_VERSION.to_string());
        let os = match std::env::consts::OS {
            "windows" => "MC_OS_WINDOWS",
            "macos" => "MC_OS_MAC",
            "linux" => "MC_OS_LINUX",
            _ => "MC_OS_OTHER",
        };
        m.define(os, "");
        // What Iris 1.11.7 reports on 26.3, whatever the pack's `#version` (observed with a probe
        // pack): its GL context's 3.3. Packs then take the paths they take there; the
        // translation itself reads GLSL up to 4.60.
        m.define("MC_GL_VERSION", "330");
        m.define("MC_GLSL_VERSION", "330");
        for (i, stage) in RENDER_STAGES.iter().enumerate() {
            m.define(format!("MC_RENDER_STAGE_{stage}"), i.to_string());
        }
        m.define("MC_RENDER_QUALITY", "1.0");
        m.define("MC_SHADOW_QUALITY", "1.0");
        m.define("MC_HAND_DEPTH", "0.125");
        m.define("MAX_COLOR_BUFFERS", "16");
        // The normal and specular maps are on (OptiFine's shaders.txt, H: their options, on by
        // default; Iris 1.11.7 defines both too, with or without a resource pack's maps).
        m.define("MC_NORMAL_MAP", "");
        m.define("MC_SPECULAR_MAP", "");
        // Packs take Iris's extensions (compute programs, images, custom textures) where this
        // is defined.
        m.define("IS_IRIS", "");
        // Iris 1.11.7's own, as it defines them on 26.3 (observed with a probe pack): its version
        // (1.11.7 as 11107), and translucent terrain sorted back to front, as Horizon sorts it
        // (vanilla's `VertexSorting.byDistance`, `gfx/translucency.rs`).
        m.define("IRIS_VERSION", "11107");
        m.define("IRIS_HAS_TRANSLUCENCY_SORTING", "");
        m
    }

    /// The macros the renderer adds: the mipmap levels terrain is drawn with (`MC_MIPMAP_LEVEL`,
    /// Iris's: it follows the Mipmap Levels option), and `OTHER` as the GL vendor and renderer
    /// (OptiFine's shaders.txt, E and F: one of each is defined). Horizon draws through wgpu, not
    /// a GL driver, so a pack's paths for one driver (an NVIDIA extension it takes behind
    /// `MC_GL_VENDOR_NVIDIA`) are not there to take.
    pub fn define_renderer(&mut self, mipmap_levels: u32) {
        self.define("MC_GL_VENDOR_OTHER", "");
        self.define("MC_GL_RENDERER_OTHER", "");
        self.define("MC_MIPMAP_LEVEL", mipmap_levels.to_string());
    }

    /// Every feature Horizon has, as `IRIS_FEATURE_<name>`: what `shaders.properties` itself is
    /// read with. Iris 1.11.7 defines a feature it has there whether the pack lists it as
    /// optional, as required or not at all (probe pack), while its programs see only the optional
    /// ones (`define_features`).
    pub fn define_every_feature(&mut self) {
        for feature in FEATURES {
            self.define(format!("IRIS_FEATURE_{feature}"), "");
        }
    }

    /// `iris.features.optional`'s features Horizon has, as `IRIS_FEATURE_<name>`; a note for
    /// each feature `iris.features.required` names that it lacks. As in Iris 1.11.7, a name
    /// matches in any case, and the macro keeps the name as the pack wrote it (`ssbo` defines
    /// `IRIS_FEATURE_ssbo`).
    pub fn define_features(&mut self, properties: &crate::Properties) -> Vec<String> {
        let words = |key: &str| {
            properties
                .get(key)
                .map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        for feature in words("iris.features.optional") {
            if is_feature(&feature) {
                self.define(format!("IRIS_FEATURE_{feature}"), "");
            }
        }
        words("iris.features.required")
            .into_iter()
            .filter(|f| !is_feature(f))
            .map(|f| format!("the pack requires `{f}`, which is not supported yet"))
            .collect()
    }

    pub fn define(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let body = value.into();
        self.0.insert(name.into(), Macro { params: None, body });
    }

    /// The rest of a `#define` line: `NAME body`, or `NAME(a, b) body` (no space before the `(`)
    /// for a function-like macro.
    pub(crate) fn define_line(&mut self, rest: &str) -> Result<(), String> {
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        let (name, tail) = rest.split_at(len);
        if name.is_empty() {
            return Err("`#define` without a name".into());
        }
        if let Some(params) = tail.strip_prefix('(') {
            let Some((params, body)) = params.split_once(')') else {
                return Err(format!("`#define {name}(` without `)`"));
            };
            let params = params
                .split(',')
                .map(|p| p.trim().to_owned())
                .filter(|p| !p.is_empty())
                .collect();
            self.define_function(name, params, body.trim());
        } else {
            self.define(name, tail.trim());
        }
        Ok(())
    }

    pub fn define_function(&mut self, name: &str, params: Vec<String>, body: &str) {
        let body = body.to_owned();
        self.0.insert(
            name.to_owned(),
            Macro {
                params: Some(params),
                body,
            },
        );
    }

    pub fn undef(&mut self, name: &str) {
        self.0.remove(name);
    }

    pub fn is_defined(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    /// The value of an object-like macro.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .get(name)
            .filter(|m| m.params.is_none())
            .map(|m| m.body.as_str())
    }

    /// Whether another line can complete a macro invocation, including an expanded alias.
    pub(crate) fn unfinished_call(&self, text: &str, next_line: &str) -> bool {
        if !self.has_macro(text) {
            return false;
        }
        let (tokens, unfinished) = self.expand_tokens(expansion_tokens(text));
        let next = next_line.trim_start();
        unfinished
            || ((next.is_empty() || next.starts_with('('))
                && tokens
                    .iter()
                    .rfind(|t| !t.text.trim().is_empty())
                    .is_some_and(|token| {
                        !token.hidden.iter().any(|name| *name == token.text)
                            && self.0.get(&token.text).is_some_and(|m| m.params.is_some())
                    }))
    }

    /// `text` with every macro expanded, repeatedly; a macro is not expanded again inside its own
    /// expansion, so `A` defined as `A + 1` stops. Function-like macros take their arguments
    /// (expanded first, raw beside `##`, which pastes) when a `(` follows their name.
    pub fn expand(&self, text: &str) -> String {
        if !self.has_macro(text) {
            return text.to_owned();
        }
        expansion_text(&self.expand_tokens(expansion_tokens(text)).0)
    }

    fn has_macro(&self, text: &str) -> bool {
        lex(text).iter().any(|token| self.0.contains_key(*token))
    }

    fn expand_tokens<'a>(
        &'a self,
        mut tokens: Vec<ExpansionToken<'a>>,
    ) -> (Vec<ExpansionToken<'a>>, bool) {
        let mut out = Vec::new();
        let mut unfinished = false;
        let mut i = 0;
        while i < tokens.len() {
            let word = &tokens[i];
            let Some((name, m)) = self.0.get_key_value(&word.text) else {
                out.push(word.clone());
                i += 1;
                continue;
            };
            if word.hidden.contains(&name.as_str()) {
                out.push(word.clone());
                i += 1;
                continue;
            }
            let (mut body, next, mut hidden) = match &m.params {
                None => (
                    paste_tokens(expansion_tokens(&m.body)),
                    i + 1,
                    word.hidden.clone(),
                ),
                Some(params) => {
                    let opening =
                        (i + 1..tokens.len()).find(|&j| !tokens[j].text.trim().is_empty());
                    if opening.is_none_or(|j| tokens[j].text != "(") {
                        out.push(word.clone());
                        i += 1;
                        continue;
                    }
                    let Some((args, next)) = arguments(&tokens, opening.unwrap()) else {
                        unfinished = true;
                        out.push(word.clone());
                        i += 1;
                        continue;
                    };
                    // A call can end outside the replacement that produced its name. Only
                    // suppress enclosing macros still active at both ends of that call.
                    let hidden = word
                        .hidden
                        .iter()
                        .copied()
                        .filter(|name| tokens[next - 1].hidden.contains(name))
                        .collect();
                    (self.substitute(params, &args, &m.body), next, hidden)
                }
            };
            hidden.push(name);
            for token in &mut body {
                for name in &hidden {
                    if !token.hidden.contains(name) {
                        token.hidden.push(name);
                    }
                }
            }
            // C++ [cpp.rescan]: rescan together with the remaining source, without
            // revisiting tokens already emitted or converting them back into text.
            tokens.splice(i..next, body);
        }
        (out, unfinished)
    }

    /// A function-like macro's body with its parameters replaced.
    fn substitute<'a>(
        &'a self,
        params: &[String],
        args: &[Vec<ExpansionToken<'a>>],
        body: &str,
    ) -> Vec<ExpansionToken<'a>> {
        let tokens = expansion_tokens(body);
        let pasted = |at: usize, step: isize| {
            let mut j = at as isize + step;
            while (0..tokens.len() as isize).contains(&j)
                && tokens[j as usize].text.trim().is_empty()
            {
                j += step;
            }
            (0..tokens.len() as isize).contains(&j) && tokens[j as usize].text == "##"
        };
        let mut expanded = vec![None; args.len()];
        let mut out = Vec::new();
        for (at, token) in tokens.iter().enumerate() {
            match params.iter().position(|p| *p == token.text) {
                Some(k) => {
                    let raw = pasted(at, -1) || pasted(at, 1);
                    let arg = if raw {
                        args.get(k)
                    } else {
                        args.get(k)
                            .map(|arg| {
                                expanded[k].get_or_insert_with(|| self.expand_tokens(arg.clone()).0)
                            })
                            .map(|arg| &*arg)
                    };
                    let arg = arg.map_or(&[][..], |arg| trim_tokens(arg));
                    out.extend_from_slice(arg);
                    if raw && arg.is_empty() {
                        out.push(ExpansionToken::new("")); // placemarker beside ##
                    }
                }
                None => out.push(token.clone()),
            }
        }
        paste_tokens(out)
    }
}

/// Suppression belongs to each token, including tokens substituted through an argument.
/// Flattening an expansion to a string would make a disabled recursive name eligible again.
#[derive(Clone)]
struct ExpansionToken<'a> {
    text: String,
    hidden: Vec<&'a str>,
}

impl ExpansionToken<'_> {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            hidden: Vec::new(),
        }
    }
}

fn trim_tokens<'t, 'a>(mut tokens: &'t [ExpansionToken<'a>]) -> &'t [ExpansionToken<'a>] {
    while tokens.first().is_some_and(|t| t.text.trim().is_empty()) {
        tokens = &tokens[1..];
    }
    while tokens.last().is_some_and(|t| t.text.trim().is_empty()) {
        tokens = &tokens[..tokens.len() - 1];
    }
    tokens
}

fn paste_tokens<'a>(mut tokens: Vec<ExpansionToken<'a>>) -> Vec<ExpansionToken<'a>> {
    while let Some(i) = tokens.iter().position(|t| t.text == "##") {
        let left =
            (0..i).rfind(|&j| !tokens[j].text.trim().is_empty() || tokens[j].text.is_empty());
        let right = (i + 1..tokens.len())
            .find(|&j| !tokens[j].text.trim().is_empty() || tokens[j].text.is_empty());
        let (Some(left), Some(right)) = (left, right) else {
            break;
        };
        let mut joined = tokens[left].clone();
        if joined.text.is_empty() {
            joined = tokens[right].clone();
        } else if !tokens[right].text.is_empty() {
            joined.text.push_str(&tokens[right].text);
            joined
                .hidden
                .retain(|name| tokens[right].hidden.contains(name));
        }
        tokens.splice(left..=right, [joined]);
    }
    tokens.retain(|t| !t.text.is_empty());
    tokens
}

fn expansion_tokens<'a>(text: &str) -> Vec<ExpansionToken<'a>> {
    expansion_lex(text)
        .into_iter()
        .map(ExpansionToken::new)
        .collect()
}

/// Keep the translator's single-character lexer unchanged; preprocessing also needs compound
/// punctuators so serializing separate tokens cannot accidentally create `++`, `<<`, etc.
fn expansion_lex(text: &str) -> Vec<&str> {
    const COMPOUND: &[&str] = &[
        "<<=", ">>=", "##", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "^^", "+=",
        "-=", "*=", "/=", "%=", "&=", "^=", "|=",
    ];
    let mut parts = lex(text).into_iter();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(part) = parts.next() {
        let mut len = part.len();
        if let Some(op) = COMPOUND.iter().find(|op| text[at..].starts_with(**op)) {
            while len < op.len() {
                len += parts.next().unwrap().len();
            }
        }
        out.push(&text[at..at + len]);
        at += len;
    }
    out
}

fn expansion_text(tokens: &[ExpansionToken<'_>]) -> String {
    let mut out = String::new();
    let mut previous = "";
    for token in tokens {
        let text = token.text.as_str();
        if !previous.trim().is_empty() && !text.trim().is_empty() {
            let joined = format!("{previous}{text}");
            let split = expansion_lex(&joined);
            if split.as_slice() != [previous, text]
                || joined.starts_with("//")
                || joined.starts_with("/*")
            {
                out.push(' ');
            }
        }
        out.push_str(text);
        previous = text;
    }
    out
}

/// The arguments of a call whose `(` is the next non-blank token from `i`, split at top-level
/// commas, and the index after its `)`. `None` when no `(` follows or it never closes.
fn arguments<'a>(
    tokens: &[ExpansionToken<'a>],
    i: usize,
) -> Option<(Vec<Vec<ExpansionToken<'a>>>, usize)> {
    let (mut depth, mut args, mut from) = (0, Vec::new(), i + 1);
    for (j, t) in tokens.iter().enumerate().skip(i + 1) {
        match t.text.as_str() {
            "(" => depth += 1,
            ")" if depth == 0 => {
                args.push(tokens[from..j].to_vec());
                return Some((args, j + 1));
            }
            ")" => depth -= 1,
            "," if depth == 0 => {
                args.push(tokens[from..j].to_vec());
                from = j + 1;
            }
            _ => {}
        }
    }
    None
}

/// Preprocessing tokens: identifiers, numbers (with their suffixes and exponents), runs of
/// white space, `##`, and single characters. Concatenated they give back `text`.
pub(crate) fn lex(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let start = i;
        i += 1;
        if is_ident_start(c) {
            while i < bytes.len() && is_ident(bytes[i] as char) {
                i += 1;
            }
        } else if c.is_ascii_digit() || (c == '.' && bytes.get(i).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() {
                let d = bytes[i] as char;
                let exp_sign = (d == '+' || d == '-') && matches!(bytes[i - 1], b'e' | b'E');
                if !(is_ident(d) || d == '.' || exp_sign) {
                    break;
                }
                i += 1;
            }
        } else if c.is_whitespace() {
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
        } else if c == '#' && bytes.get(i) == Some(&b'#') {
            i += 1;
        } else {
            while !text.is_char_boundary(i) {
                i += 1;
            }
        }
        out.push(&text[start..i]);
    }
    out
}

pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub(crate) fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `fsh` translates with `m`: a program that declares what it reads only under
    /// a macro's condition translates when the condition holds.
    fn translates(fsh: &str, m: &Macros) -> Result<(), String> {
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let pack = crate::Pack::from_zip_bytes(
            "P.zip",
            crate::pack::tests::zip(&[
                ("shaders/composite.vsh", vsh),
                ("shaders/composite.fsh", fsh),
            ]),
        )
        .unwrap();
        crate::translate_composite(
            &crate::preprocess(&pack, "composite.vsh", m),
            &crate::preprocess(&pack, "composite.fsh", m),
            crate::Target::Buffers,
        )
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
    }

    /// Packs see what Iris 1.11.7 defines on 26.3: GL and GLSL 3.3, its version and its sorted
    /// translucent terrain, so they take the paths they take there.
    #[test]
    fn programs_see_the_versions_iris_reports() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #if MC_GL_VERSION == 330 && MC_GLSL_VERSION == 330 && IRIS_VERSION >= 11107 \\\n\
                   && defined IRIS_HAS_TRANSLUCENCY_SORTING\nconst float v = 1.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        translates(fsh, &Macros::standard()).unwrap();
    }

    /// With the renderer's macros a pack sees one GL vendor and renderer, `OTHER`, and the
    /// mipmap levels terrain is drawn with.
    #[test]
    fn programs_see_the_renderers_vendor_and_mipmap_levels() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #if defined MC_GL_VENDOR_OTHER && defined MC_GL_RENDERER_OTHER && MC_MIPMAP_LEVEL == 2\n\
                   const float v = 1.0;\n#endif\n\
                   #if defined MC_GL_VENDOR_NVIDIA || defined MC_GL_RENDERER_GEFORCE\nconst float v = 2.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        let mut m = Macros::standard();
        m.define_renderer(2);
        translates(fsh, &m).unwrap();
    }

    /// Packs take their normal- and specular-map branches, as they do with the maps on: a
    /// declaration made only there is seen.
    #[test]
    fn programs_take_the_normal_and_specular_map_branches() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #ifdef MC_NORMAL_MAP\nconst float bump = 1.0;\n#endif\n\
                   #ifdef MC_SPECULAR_MAP\nconst float shine = 2.0;\n#endif\n\
                   void main() { c = vec4(bump, shine, 0.0, 1.0); }\n";
        translates(fsh, &Macros::standard()).unwrap();
    }

    /// `BLOCK_EMISSION_ATTRIBUTE` (terrain's `at_midBlock.w` is its block's light) is one
    /// Horizon has: defined when optional, no note when required.
    #[test]
    fn block_emission_attribute_is_a_feature() {
        let properties = crate::Properties::parse(
            "iris.features.optional = BLOCK_EMISSION_ATTRIBUTE\n\
             iris.features.required = BLOCK_EMISSION_ATTRIBUTE\n",
            &Macros::standard(),
        );
        let mut m = Macros::standard();
        assert!(m.define_features(&properties).is_empty());
        assert!(m.is_defined("IRIS_FEATURE_BLOCK_EMISSION_ATTRIBUTE"));
    }

    #[test]
    fn iris_features_are_defined_or_noted() {
        let properties = crate::Properties::parse(
            "iris.features.optional = COMPUTE_SHADERS SEPARATE_HARDWARE_SAMPLERS NOT_A_FEATURE\n\
             iris.features.required = ENTITY_TRANSLUCENT SEPARATE_HARDWARE_SAMPLERS NOT_A_FEATURE\n",
            &Macros::standard(),
        );
        let mut m = Macros::standard();
        let notes = m.define_features(&properties);
        assert!(m.is_defined("IRIS_FEATURE_COMPUTE_SHADERS"));
        assert!(m.is_defined("IRIS_FEATURE_SEPARATE_HARDWARE_SAMPLERS"));
        assert!(!m.is_defined("IRIS_FEATURE_NOT_A_FEATURE"));
        assert_eq!(
            notes,
            ["the pack requires `NOT_A_FEATURE`, which is not supported yet"]
        );
        // HIGHER_SHADOWCOLOR: Iris 1.11.7 defines it for `iris.features.optional` (probe p7951c).
        let mut m = Macros::default();
        let notes = m.define_features(&crate::Properties::parse(
            "iris.features.optional = HIGHER_SHADOWCOLOR\n\
             iris.features.required = HIGHER_SHADOWCOLOR\n",
            &Macros::default(),
        ));
        assert!(m.is_defined("IRIS_FEATURE_HIGHER_SHADOWCOLOR") && notes.is_empty());
    }

    /// Iris 1.11.7, with probe packs of our own (#8996): a feature name matches in any case, and
    /// an optional one defines the macro with the name as written, not in upper case.
    #[test]
    fn feature_names_match_in_any_case() {
        let mut m = Macros::standard();
        let notes = m.define_features(&crate::Properties::parse(
            "iris.features.optional = ssbo Custom_Images\n\
             iris.features.required = compute_shaders ssbo custom_images Not_A_Feature\n",
            &Macros::standard(),
        ));
        assert!(m.is_defined("IRIS_FEATURE_ssbo"));
        assert!(m.is_defined("IRIS_FEATURE_Custom_Images"));
        for absent in [
            "IRIS_FEATURE_SSBO",
            "IRIS_FEATURE_CUSTOM_IMAGES",
            "IRIS_FEATURE_compute_shaders",
            "IRIS_FEATURE_COMPUTE_SHADERS",
        ] {
            assert!(!m.is_defined(absent), "{absent}");
        }
        assert_eq!(
            notes,
            ["the pack requires `Not_A_Feature`, which is not supported yet"]
        );
    }

    #[test]
    fn versions_follow_the_documented_rule() {
        assert_eq!(mc_version("1.21"), 12100);
        assert_eq!(mc_version("1.21.0"), 12100);
        assert_eq!(mc_version("1.7.10"), 10710);
        assert_eq!(mc_version("26.3"), 260300);
        assert_eq!(MC_VERSION, 260300);
        assert_eq!(Macros::standard().get("MC_VERSION"), Some("260300"));
    }

    #[test]
    fn expansion_replaces_whole_words_and_stops_on_recursion() {
        let mut m = Macros::default();
        m.define("RES", "SIZE");
        m.define("SIZE", "512");
        m.define("LOOP", "LOOP + 1");
        assert_eq!(m.expand("RES RES 0.5"), "512 512 0.5");
        assert_eq!(
            m.expand("RESOLUTION colortex4 LOOP"),
            "RESOLUTION colortex4 LOOP + 1"
        );
        m.define("e5", "x");
        assert_eq!(m.expand("1e5"), "1e5");
    }

    #[test]
    fn function_like_macros_take_arguments_and_paste() {
        let mut m = Macros::default();
        m.define("N", "4");
        m.define_function("SQ", vec!["x".into()], "((x) * (x))");
        m.define_function("CAT", vec!["a".into(), "b".into()], "a ## b");
        m.define_function("PI", vec![], "3.14");
        assert_eq!(m.expand("SQ(N + 1)"), "((4 + 1) * (4 + 1))");
        assert_eq!(m.expand("SQ(min(N, 2))"), "((min(4, 2)) * (min(4, 2)))");
        assert_eq!(m.expand("CAT(N, 2) CAT(col, ortex4)"), "N2 colortex4");
        assert_eq!(m.expand("PI() SQ SQ(1"), "3.14 SQ SQ(1");
        assert_eq!(m.get("SQ"), None);
    }

    fn definitions(lines: &[&str]) -> Macros {
        let mut macros = Macros::default();
        for line in lines {
            macros.define_line(line).unwrap();
        }
        macros
    }

    /// GLSL 4.60 section 3.3 adopts C++ [cpp.rescan] and [cpp.subst]. These fixtures
    /// are original examples of replacement lists borrowing the caller's tokens.
    #[test]
    fn aliases_and_returned_macro_names_invoke_following_arguments() {
        let m = definitions(&[
            "APPLY(x) ((x) + 2)",
            "ALIAS APPLY",
            "CHAIN ALIAS",
            "RETURN() CHAIN",
            "CALL(f, x) f(x)",
            "PASS(x) x",
            "CAT(a, b) a ## b",
            "PASTED AP ## PLY",
            "N 5",
        ]);
        for call in [
            "ALIAS(N)",
            "CHAIN (N)",
            "RETURN()(N)",
            "CALL(ALIAS, N)",
            "PASS(ALIAS)(N)",
            "CAT(AP, PLY)(N)",
            "CAT(, APPLY)(N)",
            "PASTED(N)",
        ] {
            assert_eq!(m.expand(call), "((5) + 2)", "{call}");
        }
        assert_eq!(m.expand("ALIAS + RETURN()"), "APPLY + APPLY");
        translates(
            "#version 330\nout vec4 c;\nvoid main() { c = vec4(float(ALIAS(N))); }\n",
            &m,
        )
        .unwrap();
    }

    #[test]
    fn disabled_names_survive_argument_prescan_and_substitution() {
        let m = definitions(&[
            "PASS(x) x",
            "LOOP LOOP + 1",
            "LEFT RIGHT",
            "RIGHT LEFT",
            "STEP(x) STEP(x + 1)",
            "AGAIN() AGAIN",
        ]);
        assert_eq!(m.expand("PASS(LOOP)"), "LOOP + 1");
        assert_eq!(m.expand("PASS(LEFT) LEFT"), "LEFT LEFT");
        assert_eq!(m.expand("PASS(STEP(3))"), "STEP(3 + 1)");
        assert_eq!(m.expand("PASS(AGAIN())()"), "AGAIN()");
    }

    #[test]
    fn rescanning_expires_enclosing_aliases_and_does_not_revisit_emitted_names() {
        let m = definitions(&["ALIAS APPLY", "APPLY(x) ALIAS", "OPEN (", "PASS(x) x"]);
        // The invocation ends in the caller, after ALIAS's replacement has ended.
        assert_eq!(m.expand("ALIAS(3)"), "APPLY");
        // OPEN has not expanded when APPLY is checked for an opening parenthesis.
        assert_eq!(m.expand("APPLY OPEN 3)"), "APPLY ( 3)");
        assert_eq!(m.expand("PASS(APPLY) (3)"), "APPLY");
        let nested = definitions(&["START APPLY(", "APPLY(x) (x)", "FINISH(x) START x)"]);
        assert_eq!(nested.expand("FINISH(4)"), "(4)");
    }

    #[test]
    fn expansion_preserves_token_boundaries_except_at_pastes() {
        let m = definitions(&[
            "VALUE 1",
            "NAME() item",
            "PLUS +",
            "SLASH /",
            "CAT(a,b) a ## b",
        ]);
        assert_eq!(
            m.expand("VALUE.5 NAME()0 PLUS+ SLASH*"),
            "1 .5 item 0 + + / *"
        );
        assert_eq!(
            m.expand("VALUE + i++ == j && k<<=2"),
            "1 + i++ == j && k<<=2"
        );
        assert_eq!(
            m.expand("CAT(item, 0) CAT(1, .5) CAT(+, +)"),
            "item0 1.5 ++"
        );
        assert_eq!(m.expand("CAT(,) CAT(, value) CAT(value,)"), " value value");
    }

    #[test]
    fn unfinished_alias_calls_follow_expansion_and_suppression() {
        let m = definitions(&[
            "APPLY(x) (x)",
            "ALIAS APPLY",
            "RETURN() ALIAS",
            "AGAIN(x) AGAIN(",
        ]);
        for (text, next) in [
            ("ALIAS(", "1)"),
            ("RETURN()(1,", "2)"),
            ("ALIAS", "(2)"),
            ("APPLY  ", ""),
        ] {
            assert!(m.unfinished_call(text, next), "{text}");
        }
        for (text, next) in [
            ("ALIAS(2)", "(3)"),
            ("ALIAS + 2", "(3)"),
            ("AGAIN(2)", "3)"),
            ("ALIAS", "[3]"),
        ] {
            assert!(!m.unfinished_call(text, next), "{text}");
        }
    }
}
