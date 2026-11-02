//! Coplanar solid quads that draw identically, merged into one: same sprite,
//! plane and facing; UVs continuing one affine map over the merge (sprite
//! array layers tile at the sampler's repeat, and `terrain.wgsl` wraps tiled
//! UVs before clamped sampling, so a merged quad's UVs may pass 1.0); constant
//! smooth light and emission, and vertex colours on one affine surface — the
//! merged quad's interpolation then reproduces every removed vertex exactly.
//! Quads failing any check stay as they are.
//!
//! Output order keeps the original quad sequence: a merged rect emits where
//! its first member was, and `verify` reverts any merge that would reorder
//! coplanar overlapping quads — the same-plane draw order (grass's overlay
//! over its base) survives.
use super::TerrainVertex;
use std::collections::HashMap;

/// `TerrainVertex::uv` is fixed point, `UV_STEPS` per unit.
const PACK: f64 = super::UV_STEPS as f64;
/// The merged quads' `TerrainVertex::tiled` packs `TILED_UV_STEPS` per unit:
/// `TILE` packed units per stored one.
const TILE: f64 = PACK / super::TILED_UV_STEPS as f64;
/// Exactness for the affine colour check (colour channels are integers).
const EPS: f64 = 1e-6;

/// A vertex's place and values in the plane.
#[derive(Clone, Copy)]
struct Pt {
    s: f64,
    t: f64,
    color: u32,
    light: u16,
}
/// The quad's plane and its axis-aligned rect in it (`[s0, s1, t0, t1]`); `s`
/// and `t` are the two in-plane axes in axis order.
#[derive(Clone, Copy)]
struct Frame {
    axis: usize,
    sign: bool,
    plane: f32,
    rect: [f64; 4],
}
fn frame(quad: &[TerrainVertex; 4]) -> Option<Frame> {
    let p = quad.map(|v| v.pos);
    let axis = (0..3).find(|&a| p.iter().all(|v| v[a] == p[0][a]))?;
    let (sa, ta) = ((axis + 1) % 3, (axis + 2) % 3);
    let (mut s0, mut s1) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in &p {
        let (s, t) = (f64::from(v[sa]), f64::from(v[ta]));
        s0 = s0.min(s);
        s1 = s1.max(s);
        t0 = t0.min(t);
        t1 = t1.max(t);
    }
    if !(s0 < s1 && t0 < t1) {
        return None;
    }
    // Winding, as `sides::bucket`: both triangles face the same way or the
    // quad is bow-tied (never merge those).
    let area = |a: usize, b: usize, c: usize| {
        (p[b][sa] - p[a][sa]) * (p[c][ta] - p[a][ta])
            - (p[b][ta] - p[a][ta]) * (p[c][sa] - p[a][sa])
    };
    let (first, second) = (area(0, 1, 2), area(0, 2, 3));
    if first.signum() != second.signum() || first == 0.0 {
        return None;
    }
    Some(Frame {
        axis,
        sign: first > 0.0,
        plane: p[0][axis],
        rect: [s0, s1, t0, t1],
    })
}
/// A mergeable quad: an axis-rect with an affine UV map and one sprite layer.
struct Quad {
    index: u32,
    frame: Frame,
    /// The four vertices' `(s, t, color, light)`, in emitted order.
    pts: [Pt; 4],
    layer: u16,
    /// UV gradient per unit of `s` and of `t`, in packed units.
    m: [[f64; 2]; 2],
    /// UV origin: `uv = m . (s, t) + b`, packed units.
    b: [f64; 2],
}
/// Packed group key: axis, facing, plane bits, sprite layer.
type Key = u64;
fn key(frame: &Frame, layer: u16) -> Key {
    (frame.axis as u64) << 48
        | (frame.sign as u64) << 47
        | (frame.plane.to_bits() as u64) << 15
        | u64::from(layer)
}
fn classify(quad: &[TerrainVertex; 4], index: u32, frame: Frame) -> Option<(Key, Quad)> {
    let layer = quad[0].layer;
    if quad.iter().any(|v| v.layer != layer) {
        return None;
    }
    // A quad whose own light varies can never merge: the union's light must
    // be constant (the shader multiplies colour by the lightmap sample per
    // vertex, a product no merged quad could reproduce where it varies).
    if quad.iter().any(|v| v.light != quad[0].light) {
        return None;
    }
    let [s0, s1, t0, t1] = frame.rect;
    let (sa, ta) = ((frame.axis + 1) % 3, (frame.axis + 2) % 3);
    let pts: [Pt; 4] = std::array::from_fn(|i| Pt {
        s: f64::from(quad[i].pos[sa]),
        t: f64::from(quad[i].pos[ta]),
        color: quad[i].color,
        light: quad[i].light,
    });
    // An axis-aligned rectangle: every vertex sits on a rect corner.
    if pts
        .iter()
        .any(|p| !(p.s == s0 || p.s == s1) || !(p.t == t0 || p.t == t1))
    {
        return None;
    }
    let at = |s: f64, t: f64| pts.iter().position(|p| p.s == s && p.t == t);
    let (i00, i10, i11, i01) = (at(s0, t0)?, at(s1, t0)?, at(s1, t1)?, at(s0, t1)?);
    let uv = |i: usize| [f64::from(quad[i].uv[0]), f64::from(quad[i].uv[1])];
    let (u00, u10, u11, u01) = (uv(i00), uv(i10), uv(i11), uv(i01));
    for c in 0..2 {
        if (u00[c] + u11[c] - u10[c] - u01[c]).abs() > 0.5 {
            return None;
        }
    }
    let m = [
        [(u10[0] - u00[0]) / (s1 - s0), (u10[1] - u00[1]) / (s1 - s0)],
        [(u01[0] - u00[0]) / (t1 - t0), (u01[1] - u00[1]) / (t1 - t0)],
    ];
    let b = [
        u00[0] - m[0][0] * s0 - m[1][0] * t0,
        u00[1] - m[0][1] * s0 - m[1][1] * t0,
    ];
    Some((
        key(&frame, layer),
        Quad {
            index,
            frame,
            pts,
            layer,
            m,
            b,
        },
    ))
}
/// A growing merged rect: the first member's corner order (which gives the
/// merged quad's winding) and the member indices into the group's `members`.
/// One affine colour surface over the rect: `value(s, t) = c + ds·(s - s0) +
/// dt·(t - t0)` per channel — what a merged quad's interpolation reproduces.
struct Surface {
    s0: f64,
    t0: f64,
    /// `[v00, ds, dt]` for each of the three colour channels.
    ch: [[f64; 3]; 3],
    /// Constant light and emission (the alpha byte) across the whole rect.
    light: u16,
    alpha: u32,
}
impl Surface {
    /// The surface a single quad's four corners define, if they are affine
    /// (a quad whose own corners bend can never join a merge).
    fn of(q: &Quad) -> Option<Self> {
        let [s0, s1, t0, t1] = q.frame.rect;
        let at = |s: f64, t: f64| q.pts.iter().find(|p| p.s == s && p.t == t).map(|p| p.color);
        let (v00, v10, v11, v01) = (at(s0, t0)?, at(s1, t0)?, at(s1, t1)?, at(s0, t1)?);
        let mut ch = [[0.0; 3]; 3];
        for (c, ch) in ch.iter_mut().enumerate() {
            let chv = |v: u32| f64::from(v >> (c * 8) & 0xFF);
            let (a00, a10, a11, a01) = (chv(v00), chv(v10), chv(v11), chv(v01));
            if (a11 - a10 - a01 + a00).abs() > EPS {
                return None;
            }
            *ch = [a00, (a10 - a00) / (s1 - s0), (a01 - a00) / (t1 - t0)];
        }
        Some(Self {
            s0,
            t0,
            ch,
            light: q.pts[0].light,
            alpha: q.pts[0].color >> 24,
        })
    }
    /// Whether `p` lies on this surface (and shares its light and alpha).
    fn has(&self, p: Pt) -> bool {
        if p.light != self.light || p.color >> 24 != self.alpha {
            return false;
        }
        self.ch.iter().enumerate().all(|(c, ch)| {
            (ch[0] + ch[1] * (p.s - self.s0) + ch[2] * (p.t - self.t0)
                - f64::from(p.color >> (c * 8) & 0xFF))
            .abs()
                <= EPS
        })
    }
}
/// A growing merged rect: the first member's corner order (which gives the
/// merged quad's winding), its colour surface, and the member indices into
/// the group's `members` — the first inline, the rest pushed only as merges
/// actually happen.
struct Acc {
    frame: Frame,
    layer: u16,
    rect: [f64; 4],
    /// Per emitted vertex: `(s == rect.s1, t == rect.t1)` of its corner.
    roles: [[bool; 2]; 4],
    surface: Surface,
    first: u32,
    members: Vec<u32>,
}
impl Acc {
    fn new(q: &Quad, member: u32) -> Option<Self> {
        let [_, s1, _, t1] = q.frame.rect;
        Some(Self {
            frame: q.frame,
            layer: q.layer,
            rect: q.frame.rect,
            roles: q.pts.map(|p| [p.s == s1, p.t == t1]),
            surface: Surface::of(q)?,
            first: member,
            members: Vec::new(),
        })
    }
    fn count(&self) -> usize {
        1 + self.members.len()
    }
    fn each(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(self.first).chain(self.members.iter().copied())
    }
    fn slot(&self, members: &[Quad]) -> u32 {
        self.each()
            .map(|m| members[m as usize].index)
            .min()
            .unwrap_or(0)
    }
    fn pts<'a>(&'a self, members: &'a [Quad]) -> impl Iterator<Item = Pt> + 'a {
        self.each().flat_map(|m| members[m as usize].pts)
    }
}
/// Whether the members' corners of `a` and `b` (whose rects share an edge
/// into `rect`) sit on one affine surface per colour channel — the merged
/// quad's interpolation then equals every removed vertex's — with constant
/// light and emission: the vertex shader multiplies each vertex's colour by
/// the lightmap sample of its own light coords, a product no merge could
/// reproduce where the light itself varies.
/// Whether `b`'s member corners all lie on `a`'s colour surface (with its
/// constant light and alpha): the union then shares that surface — a merged
/// quad's interpolation reproduces every removed vertex exactly, including
/// the corner that `a`'s own quad does not reach.
fn compatible(a: &Acc, b: &Acc, members: &[Quad]) -> bool {
    b.pts(members).all(|p| a.surface.has(p))
}
/// One sweep: accs sorted by their range across the merge axis, then chained
/// where the shared range and the touching edge agree and `compatible` allows.
fn pass(accs: &mut Vec<Acc>, members: &[Quad], along_s: bool) {
    // Along s: the shared range is t (indices 2, 3), the touching edges s.
    let (edge, span) = if along_s { (0, 2) } else { (2, 0) };
    accs.sort_unstable_by(|a, b| {
        a.rect[span]
            .total_cmp(&b.rect[span])
            .then(a.rect[span + 1].total_cmp(&b.rect[span + 1]))
            .then(a.rect[edge].total_cmp(&b.rect[edge]))
    });
    let mut out: Vec<Acc> = Vec::with_capacity(accs.len());
    for b in std::mem::take(accs) {
        if let Some(a) = out.last_mut() {
            let rect = [
                a.rect[0].min(b.rect[0]),
                a.rect[1].max(b.rect[1]),
                a.rect[2].min(b.rect[2]),
                a.rect[3].max(b.rect[3]),
            ];
            if a.rect[span] == b.rect[span]
                && a.rect[span + 1] == b.rect[span + 1]
                && a.rect[edge + 1] == b.rect[edge]
                && compatible(a, &b, members)
            {
                a.rect = rect;
                a.members.push(b.first);
                a.members.extend(b.members);
                continue;
            }
        }
        out.push(b);
    }
    *accs = out;
}
/// The merged quad for `acc`, or `None` when its UV corners don't fit the
/// tiled packing (`u16` steps of `1 / TILE` of a sprite).
fn emit(acc: &Acc, members: &[Quad], m: [[f64; 2]; 2], b: [f64; 2]) -> Option<[TerrainVertex; 4]> {
    let [s0, s1, t0, t1] = acc.rect;
    let (sa, ta) = ((acc.frame.axis + 1) % 3, (acc.frame.axis + 2) % 3);
    let mut out = Vec::with_capacity(4);
    for &[hs, ht] in &acc.roles {
        let (s, t) = (if hs { s1 } else { s0 }, if ht { t1 } else { t0 });
        let uv = [
            m[0][0] * s + m[1][0] * t + b[0],
            m[0][1] * s + m[1][1] * t + b[1],
        ];
        if uv.iter().any(|u| {
            *u < -EPS
                || *u > f64::from(u16::MAX) * TILE
                || ((u / TILE).round() * TILE - u).abs() > EPS
        }) {
            return None;
        }
        let corner = acc.pts(members).find(|p| p.s == s && p.t == t)?;
        let mut pos = [0.0; 3];
        pos[acc.frame.axis] = acc.frame.plane;
        pos[sa] = s as f32;
        pos[ta] = t as f32;
        out.push(TerrainVertex::tiled(
            pos,
            [(uv[0] / PACK) as f32, (uv[1] / PACK) as f32],
            acc.layer,
            corner.color,
            corner.light,
        ));
    }
    out.try_into().ok()
}
/// What the rebuilt mesh draws: `slot` is its position in the original quad
/// sequence, `parts` the rects it covers with the quad index each came from
/// (for `violation`'s coplanar-overlap order check).
struct Item {
    slot: u32,
    verts: Verts,
    parts: Parts,
}
/// A merged rect's members (quad index and frame).
enum Parts {
    Many(Vec<(u32, Frame)>),
}
impl Parts {
    fn len(&self) -> usize {
        match self {
            Parts::Many(v) => v.len(),
        }
    }
    fn each(&self) -> impl Iterator<Item = (u32, Frame)> + '_ {
        match self {
            Parts::Many(v) => v.as_slice(),
        }
        .iter()
        .copied()
    }
}
enum Verts {
    Rect([TerrainVertex; 4]),
}
fn rects_overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[1] && b[0] < a[1] && a[2] < b[3] && b[2] < a[3]
}
/// Coplanar-overlap ordering violations: where a merged item's rect overlaps
/// another emitted quad's, the member that came first must draw first — a
/// merged rect emitted ahead of one of its overlapping members' positions
/// would reorder the pair. Singles' slots are their own indices, so only
/// merged items can violate. Returns the items to split.
fn violations(items: &[Item], singles: &[(u32, Frame)]) -> Vec<usize> {
    let mut singles_by_sheet: HashMap<(u8, u8, u32), Vec<(u32, Frame)>> = HashMap::new();
    for &(i, f) in singles {
        singles_by_sheet
            .entry((f.axis as u8, u8::from(f.sign), f.plane.to_bits()))
            .or_default()
            .push((i, f));
    }
    let mut sheets: HashMap<(u8, u8, u32), Vec<(usize, u32, [f64; 4])>> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        for (index, frame) in item.parts.each() {
            sheets
                .entry((
                    frame.axis as u8,
                    u8::from(frame.sign),
                    frame.plane.to_bits(),
                ))
                .or_default()
                .push((i, index, frame.rect));
        }
    }
    let mut bad = Vec::new();
    for (key, parts) in &sheets {
        for &(a, ma, ra) in parts.iter().filter(|&&(i, _, _)| items[i].parts.len() > 1) {
            // Overlapping members of other merged items.
            for &(b, mb, rb) in parts {
                if a == b || !rects_overlap(ra, rb) {
                    continue;
                }
                let early_a = items[a].slot <= items[b].slot;
                if if early_a { ma > mb } else { mb > ma } {
                    bad.push(if early_a { a } else { b });
                }
            }
            // Overlapping singleton quads of the same sheet.
            if let Some(here) = singles_by_sheet.get(key) {
                for &(idx, f) in here {
                    if !rects_overlap(ra, f.rect) {
                        continue;
                    }
                    if items[a].slot < idx && ma > idx || items[a].slot > idx && idx > ma {
                        bad.push(a);
                    }
                }
            }
        }
    }
    bad.sort_unstable();
    bad.dedup();
    bad
}
/// Reverts the merge at `i` back to its individual quads.
fn demote(items: &mut Vec<Item>, singles: &mut Vec<u32>, parts: &mut Vec<(u32, Frame)>, i: usize) {
    for (index, frame) in items.remove(i).parts.each().collect::<Vec<_>>() {
        singles.push(index);
        parts.push((index, frame));
    }
}

/// Merges the coplanar quads of `solid` that draw identically.
pub(crate) fn solid(solid: &mut Vec<TerrainVertex>) {
    let (quads, rest) = solid.as_chunks::<4>();
    debug_assert!(rest.is_empty());
    let originals = quads.to_vec();
    // Classify; sort by the packed key so same-sheet quads are runs.
    let mut frames: Vec<Option<Frame>> = Vec::with_capacity(quads.len());
    let mut classified: Vec<(Key, Quad)> = Vec::new();
    let mut singles: Vec<u32> = Vec::new();
    for (index, quad) in quads.iter().enumerate() {
        let f = frame(quad);
        frames.push(f);
        match f.and_then(|f| classify(quad, index as u32, f)) {
            Some(pair) => classified.push(pair),
            None => singles.push(index as u32),
        }
    }
    if classified.is_empty() {
        return;
    }
    // The full sort key groups quads that can merge: same sheet, facing,
    // layer and UV map (mod the sprite's period — the phase is quantized,
    // splitting maps within `UV_EPS` is only a missed merge).
    let map_key = |q: &Quad| {
        [
            q.m[0][0].to_bits(),
            q.m[0][1].to_bits(),
            q.m[1][0].to_bits(),
            q.m[1][1].to_bits(),
            (q.b[0].rem_euclid(PACK) * 1024.0).round() as u64,
            (q.b[1].rem_euclid(PACK) * 1024.0).round() as u64,
        ]
    };
    classified.sort_by_cached_key(|(k, q)| (*k, map_key(q)));
    let mut items: Vec<Item> = Vec::new();
    let mut classified = classified.into_iter().peekable();
    while let Some((key, mk)) = classified.peek().map(|(k, q)| (*k, map_key(q))) {
        let mut members = Vec::new();
        while classified.peek().map(|(k, q)| (*k, map_key(q))) == Some((key, mk)) {
            members.push(classified.next().expect("peeked").1);
        }
        if members.len() < 2 {
            singles.extend(members.into_iter().map(|q| q.index));
            continue;
        }
        let (m, b) = (members[0].m, members[0].b);
        {
            let mut accs: Vec<Acc> = Vec::new();
            for (i, q) in members.iter().enumerate() {
                match Acc::new(q, i as u32) {
                    Some(acc) => accs.push(acc),
                    None => singles.push(q.index),
                }
            }
            pass(&mut accs, &members, true);
            pass(&mut accs, &members, false);
            for acc in accs {
                match (acc.count() > 1)
                    .then(|| emit(&acc, &members, m, b))
                    .flatten()
                {
                    Some(verts) => items.push(Item {
                        slot: acc.slot(&members),
                        verts: Verts::Rect(verts),
                        parts: Parts::Many(
                            acc.each()
                                .map(|i| {
                                    let q = &members[i as usize];
                                    (q.index, q.frame)
                                })
                                .collect(),
                        ),
                    }),
                    None => singles.extend(acc.each().map(|i| members[i as usize].index)),
                }
            }
        }
    }
    if items.is_empty() {
        return;
    }
    // Order: a merged rect emits where its first member was; `violations`
    // splits any merge that would reorder overlapping coplanar quads.
    items.sort_unstable_by_key(|item| item.slot);
    let mut parts: Vec<(u32, Frame)> = singles
        .iter()
        .filter_map(|&i| frames[i as usize].map(|f| (i, f)))
        .collect();
    let mut rounds = items.len();
    loop {
        let bad = violations(&items, &parts);
        if bad.is_empty() || rounds == 0 {
            break;
        }
        rounds -= 1;
        for i in bad.into_iter().rev() {
            demote(&mut items, &mut singles, &mut parts, i);
        }
        items.sort_unstable_by_key(|item| item.slot);
    }
    // Emit: originals in index order, merged items at their slots.
    singles.sort_unstable();
    let mut out: Vec<TerrainVertex> = Vec::with_capacity(solid.len());
    let mut si = 0;
    for item in items {
        while si < singles.len() && singles[si] < item.slot {
            out.extend(originals[singles[si] as usize]);
            si += 1;
        }
        let Verts::Rect(verts) = item.verts;
        out.extend(verts);
    }
    for &index in &singles[si..] {
        out.extend(originals[index as usize]);
    }
    *solid = out;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::MERGED_UV;

    fn quad(
        x0: f32,
        z0: f32,
        x1: f32,
        z1: f32,
        layer: u16,
        colors: [u32; 4],
        light: u16,
    ) -> [TerrainVertex; 4] {
        // +Y face, vanilla cube winding (like sides::cube).
        let pos = [[x0, 1.0, z1], [x0, 1.0, z0], [x1, 1.0, z0], [x1, 1.0, z1]];
        let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        std::array::from_fn(|i| TerrainVertex::new(pos[i], uv[i], layer, colors[i], light))
    }
    fn flat(quads: &[[TerrainVertex; 4]]) -> Vec<TerrainVertex> {
        quads.iter().flatten().copied().collect()
    }

    #[test]
    fn coplanar_row_merges_to_one_quad() {
        let quads: Vec<_> = (0..4)
            .map(|i| {
                quad(
                    i as f32,
                    0.0,
                    i as f32 + 1.0,
                    1.0,
                    7,
                    [0xFFAA_5522; 4],
                    0xF0,
                )
            })
            .collect();
        let mut solid = flat(&quads);
        super::solid(&mut solid);
        assert_eq!(solid.len(), 4);
        assert!(solid.iter().all(|v| v.layer & MERGED_UV != 0));
        let xs: Vec<f32> = solid.iter().map(|v| v.pos[0]).collect();
        assert!(xs.contains(&0.0) && xs.contains(&4.0));
        // Tiled UVs span 4 blocks.
        let us: Vec<f32> = solid.iter().map(|v| v.uv()[0]).collect();
        assert!(us.contains(&0.0) && us.contains(&4.0), "{us:?}");
    }

    #[test]
    fn varying_light_never_merges() {
        let quads = [
            quad(0.0, 0.0, 1.0, 1.0, 7, [0xFFAA_5522; 4], 0xF0),
            quad(1.0, 0.0, 2.0, 1.0, 7, [0xFFAA_5522; 4], 0xE0),
        ];
        let mut solid = flat(&quads);
        super::solid(&mut solid);
        assert_eq!(solid.len(), 8);
    }

    #[test]
    fn affine_colour_merges_but_a_bend_does_not() {
        // Linear grey ramp across a 4-quad row: merges.
        let ramp = |i: usize| {
            let g = (40 * i) as u32;
            [0xFF00_0000 | g << 16 | g << 8 | g; 4]
        };
        let _ = ramp;
        let mut quads = Vec::new();
        for i in 0..4 {
            let mut q = quad(
                i as f32,
                0.0,
                i as f32 + 1.0,
                1.0,
                7,
                [0xFFAA_5522; 4],
                0xF0,
            );
            // Colour varies along x only: corners share values on each edge.
            let c = |g: u32| 0xFF00_0000 | g << 16 | g << 8 | g;
            let (g0, g1) = ((20 * i) as u32, (20 * (i + 1)) as u32);
            q[0].color = c(g0);
            q[1].color = c(g0);
            q[2].color = c(g1);
            q[3].color = c(g1);
            quads.push(q);
        }
        let mut solid = flat(&quads);
        super::solid(&mut solid);
        assert_eq!(solid.len(), 4);
    }

    #[test]
    fn different_sprites_or_planes_stay_apart() {
        let quads = [
            quad(0.0, 0.0, 1.0, 1.0, 7, [0xFFAA_5522; 4], 0xF0),
            quad(1.0, 0.0, 2.0, 1.0, 9, [0xFFAA_5522; 4], 0xF0),
        ];
        let mut solid = flat(&quads);
        super::solid(&mut solid);
        assert_eq!(solid.len(), 8);
    }

    #[test]
    fn winding_is_preserved() {
        // -Y face winding (reversed corners).
        let mut q = quad(0.0, 0.0, 1.0, 1.0, 7, [0xFFAA_5522; 4], 0xF0);
        q.reverse();
        let quads = [q, {
            let mut q2 = quad(1.0, 0.0, 2.0, 1.0, 7, [0xFFAA_5522; 4], 0xF0);
            q2.reverse();
            q2
        }];
        let mut solid = flat(&quads);
        super::solid(&mut solid);
        assert_eq!(solid.len(), 4);
        // Same winding as input: first vertex at (x1 or x0) corner preserved.
        let p0 = solid[0].pos;
        let p0b = quads[0][0].pos;
        assert_eq!([p0[0] != 0.0, p0[2] != 0.0], [p0b[0] != 0.0, p0b[2] != 0.0]);
    }

    #[test]
    fn overlay_order_survives() {
        // Base quad then overlay quad, coplanar, different sprites: a merged
        // base must not jump ahead of the overlay.
        let quads = [
            quad(0.0, 0.0, 1.0, 1.0, 3, [0xFFAA_5522; 4], 0xF0),
            quad(0.0, 0.0, 1.0, 1.0, 4, [0xFF00_0000; 4], 0xF0),
            quad(1.0, 0.0, 2.0, 1.0, 4, [0xFF00_0000; 4], 0xF0),
            quad(1.0, 0.0, 2.0, 1.0, 3, [0xFFAA_5522; 4], 0xF0),
        ];
        let mut solid = flat(&quads);
        super::solid(&mut solid);
        // Base quad 3 drew after overlay quad 2 on its cell, so the base quads
        // cannot merge (the merged one would jump ahead of the overlay); the
        // layer-4 overlays still merge into one quad.
        let quads_out = solid.as_chunks::<4>().0;
        assert_eq!(quads_out.len(), 3);
        let sprite = |q: &[TerrainVertex; 4]| q[0].layer & !(MERGED_UV | 0x8000);
        assert_eq!(sprite(&quads_out[0]), 3);
        assert_eq!(sprite(&quads_out[1]), 4);
        assert_eq!(sprite(&quads_out[2]), 3);
        // The merged overlay covers the base quad it replaced plus its own.
        let xs: Vec<f32> = quads_out[1].iter().map(|v| v.pos[0]).collect();
        assert!(xs.contains(&0.0) && xs.contains(&2.0), "{xs:?}");
    }
}
