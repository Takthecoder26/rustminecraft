use super::*;
use std::any::Any;

/// A maximal run of pointwise-arithmetic samplers flattened into one op list.
/// Bulk evaluation walks the program once instead of dispatching a buffer pass
/// per tree node. Scalar evaluation delegates to the retained subtree: its
/// per-node short-circuits (skipped branches, early min/max and zero-product
/// returns) suppress child errors and cache-cell writes, which a flat program
/// evaluating every operand could not reproduce.
///
/// Ops are emitted in the tree's post-order, so leaf samplers fill slots in
/// exactly the tree's left-to-right volume order. Slot 0 is the caller's
/// buffer: nodes evaluating their first operand in place (unary input, binary
/// left, power base, round input, lerp alpha, range-choice inside branch,
/// interval-select input) share it with that child, preserving both the
/// strictest buffer contract and writes past `volume.size()` in oversized
/// caller buffers.
#[derive(Debug)]
struct FusedSampler {
    /// The compiled subtree this program replaces.
    scalar: Arc<dyn DensitySampler>,
    leaves: Box<[Arc<dyn DensitySampler>]>,
    ops: Box<[Op]>,
    /// Volume-sized scratch slots live at once (slot 0 excluded).
    scratch: u32,
    /// Strictest buffer contract along the output spine.
    exact: bool,
}
#[derive(Clone, Debug)]
enum Op {
    Leaf {
        slot: u32,
        index: u32,
    },
    Constant {
        slot: u32,
        value: f32,
    },
    Unary {
        slot: u32,
        operation: UnaryOp,
    },
    Binary {
        operation: BinaryOp,
        left: u32,
        right: u32,
    },
    Power {
        base: u32,
        exponent: u32,
    },
    Round {
        slot: u32,
        mode: RoundMode,
        multiple: Option<u32>,
    },
    Lerp {
        alpha: u32,
        first: Operand,
        second: Operand,
    },
    RangeChoice {
        inside: u32,
        input: u32,
        outside: u32,
        min_inclusive: f32,
        max_exclusive: f32,
    },
    IntervalSelect {
        input: u32,
        thresholds: Arc<[f32]>,
        branches: Box<[u32]>,
    },
}
impl Op {
    /// Combine and leaf ops rewrite existing values; the rest only fill a slot.
    fn combines(&self) -> bool {
        !matches!(self, Self::Leaf { .. } | Self::Constant { .. })
    }
    fn remap(&self, slots: &[u32], leaf_base: u32) -> Self {
        let s = |slot: u32| slots[slot as usize];
        match *self {
            Self::Leaf { slot, index } => Self::Leaf {
                slot: s(slot),
                index: index + leaf_base,
            },
            Self::Constant { slot, value } => Self::Constant {
                slot: s(slot),
                value,
            },
            Self::Unary { slot, operation } => Self::Unary {
                slot: s(slot),
                operation,
            },
            Self::Binary {
                operation,
                left,
                right,
            } => Self::Binary {
                operation,
                left: s(left),
                right: s(right),
            },
            Self::Power { base, exponent } => Self::Power {
                base: s(base),
                exponent: s(exponent),
            },
            Self::Round {
                slot,
                mode,
                multiple,
            } => Self::Round {
                slot: s(slot),
                mode,
                multiple: multiple.map(s),
            },
            Self::Lerp {
                alpha,
                ref first,
                ref second,
            } => Self::Lerp {
                alpha: s(alpha),
                first: first.remap(slots),
                second: second.remap(slots),
            },
            Self::RangeChoice {
                inside,
                input,
                outside,
                min_inclusive,
                max_exclusive,
            } => Self::RangeChoice {
                inside: s(inside),
                input: s(input),
                outside: s(outside),
                min_inclusive,
                max_exclusive,
            },
            Self::IntervalSelect {
                input,
                ref thresholds,
                ref branches,
            } => Self::IntervalSelect {
                input: s(input),
                thresholds: thresholds.clone(),
                branches: branches.iter().map(|&slot| s(slot)).collect(),
            },
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Operand {
    Slot(u32),
    Constant(f32),
}
impl Operand {
    fn remap(&self, slots: &[u32]) -> Self {
        match *self {
            Self::Slot(slot) => Self::Slot(slots[slot as usize]),
            Self::Constant(value) => Self::Constant(value),
        }
    }
    fn get(self, slots: &[&mut [f32]], index: usize) -> f32 {
        match self {
            Self::Slot(slot) => slots[slot as usize][index],
            Self::Constant(value) => value,
        }
    }
}
fn select(thresholds: &[f32], branches: usize, value: f32) -> usize {
    thresholds
        .iter()
        .position(|threshold| value < *threshold)
        .unwrap_or(branches - 1)
}
/// Two disjoint operand slots; builders never alias distinct operands.
fn two<'s>(slots: &'s mut [&mut [f32]], i: usize, j: usize) -> (&'s mut [f32], &'s mut [f32]) {
    debug_assert_ne!(i, j);
    let (head, tail) = slots.split_at_mut(i.max(j));
    let pair = (&mut *head[i.min(j)], &mut *tail[0]);
    if i < j { pair } else { (pair.1, pair.0) }
}
/// Move a slot out so other slots stay readable while it is written. The
/// caller restores it once the combine pass is done.
fn take<'d>(slots: &mut [&'d mut [f32]], slot: u32) -> &'d mut [f32] {
    std::mem::take(&mut slots[slot as usize])
}
impl DensitySampler for FusedSampler {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        self.scalar.sample_value(context, position)
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        if self.exact {
            check_exact_buffer(buffer, volume)?;
        } else {
            check_buffer(buffer, volume)?;
        }
        let size = volume.size();
        let scratch = self.scratch as usize;
        let total = scratch
            .checked_mul(size)
            .filter(|total| *total <= isize::MAX as usize / size_of::<f32>());
        // Degenerate cases delegate to the subtree: zero-size volumes take no
        // scratch, and an oversized slab would fail where sequential
        // evaluation still succeeds.
        let Some(total) = total.filter(|_| size > 0) else {
            return self.scalar.sample_volume(context, buffer, volume);
        };
        context.with_buffer(total, |context, slab| {
            let mut slots: Vec<&mut [f32]> = Vec::with_capacity(scratch + 1);
            slots.push(buffer);
            slots.extend(slab.chunks_exact_mut(size));
            self.run(context, &mut slots, volume)
        })
    }
}
impl FusedSampler {
    fn run(
        &self,
        context: &mut SamplerContext,
        slots: &mut [&mut [f32]],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        for op in &*self.ops {
            match *op {
                Op::Leaf { slot, index } => self.leaves[index as usize].sample_volume(
                    context,
                    &mut *slots[slot as usize],
                    volume,
                )?,
                Op::Constant { slot, value } => slots[slot as usize].fill(value),
                Op::Unary { slot, operation } => {
                    operation.apply_bulk(&mut *slots[slot as usize]);
                }
                Op::Binary {
                    operation,
                    left,
                    right,
                } => {
                    let (left, right) = two(slots, left as usize, right as usize);
                    operation.apply_bulk(left, right);
                }
                Op::Power { base, exponent } => {
                    let (base, exponent) = two(slots, base as usize, exponent as usize);
                    for (base, exponent) in base.iter_mut().zip(exponent.iter()) {
                        *base = arithmetic::java_power(*base as f64, *exponent as f64) as f32;
                    }
                }
                Op::Round {
                    slot,
                    mode,
                    multiple,
                } => match multiple {
                    None => {
                        for value in slots[slot as usize].iter_mut() {
                            *value = mode.integer(*value);
                        }
                    }
                    Some(multiple) => {
                        let (input, multiple) = two(slots, slot as usize, multiple as usize);
                        for (input, multiple) in input.iter_mut().zip(multiple.iter()) {
                            *input = mode.apply(*input, *multiple);
                        }
                    }
                },
                Op::Lerp {
                    alpha,
                    first,
                    second,
                } => {
                    let index = alpha as usize;
                    let alpha = take(slots, alpha);
                    for (i, value) in alpha.iter_mut().enumerate() {
                        *value = selection::interpolate(
                            *value,
                            first.get(slots, i),
                            second.get(slots, i),
                        );
                    }
                    slots[index] = alpha;
                }
                Op::RangeChoice {
                    inside,
                    input,
                    outside,
                    min_inclusive,
                    max_exclusive,
                } => {
                    let index = inside as usize;
                    let inside = take(slots, inside);
                    for (i, value) in inside.iter_mut().enumerate() {
                        let selector = slots[input as usize][i];
                        if !(selector >= min_inclusive && selector < max_exclusive) {
                            *value = slots[outside as usize][i];
                        }
                    }
                    slots[index] = inside;
                }
                Op::IntervalSelect {
                    input,
                    ref thresholds,
                    ref branches,
                } => {
                    let index = input as usize;
                    let input = take(slots, input);
                    for (i, value) in input.iter_mut().enumerate() {
                        let branch = select(thresholds, branches.len(), *value);
                        *value = slots[branches[branch] as usize][i];
                    }
                    slots[index] = input;
                }
            }
        }
        Ok(())
    }
}
/// Scratch slots a node needs beyond its in-place operand; checked against
/// the live budget before emission, so allocation inside `build` cannot fail.
fn extra_slots(node: &dyn Any) -> Option<u32> {
    if node.is::<ConstantSampler>() || node.is::<UnarySampler>() {
        Some(0)
    } else if node.is::<BinarySampler>() || node.is::<PowerSampler>() {
        Some(1)
    } else if let Some(round) = node.downcast_ref::<RoundSampler>() {
        Some(u32::from(round.multiple.is_some()))
    } else if let Some(lerp) = node.downcast_ref::<LerpSampler>() {
        Some(
            u32::from(matches!(lerp.first, DensityValue::Sampler(_)))
                + u32::from(matches!(lerp.second, DensityValue::Sampler(_))),
        )
    } else if node.is::<RangeChoiceSampler>() {
        Some(2)
    } else {
        node.downcast_ref::<IntervalSelectSampler>()
            .map(|select| select.branches.len() as u32)
    }
}
/// Post-order program builder. Every slot a combine op reads is either the
/// caller's buffer (slot 0) or a volume-sized scratch slot allocated per
/// operand and released after the combine, mirroring `with_buffer` lifetimes.
struct Builder {
    ops: Vec<Op>,
    leaves: Vec<Arc<dyn DensitySampler>>,
    free: Vec<u32>,
    fresh: u32,
    live: u32,
    scratch: u32,
    /// Combining ops emitted so far, including merged child programs.
    work: u32,
    exact: bool,
}
impl Builder {
    const MAX_OPS: usize = 1 << 15;
    const MAX_SLOTS: u32 = 1 << 12;
    fn new() -> Self {
        Self {
            ops: Vec::new(),
            leaves: Vec::new(),
            free: Vec::new(),
            fresh: 1,
            live: 0,
            scratch: 0,
            work: 0,
            exact: false,
        }
    }
    fn alloc(&mut self) -> u32 {
        self.live += 1;
        if let Some(slot) = self.free.pop() {
            return slot;
        }
        let slot = self.fresh;
        self.fresh += 1;
        self.scratch = self.scratch.max(slot);
        slot
    }
    fn release(&mut self, slot: u32) {
        self.live -= 1;
        self.free.push(slot);
    }
    fn push(&mut self, op: Op) {
        self.work += u32::from(op.combines());
        self.ops.push(op);
    }
    fn leaf(&mut self, node: &Arc<dyn DensitySampler>, dest: u32) {
        let index = self.leaves.len() as u32;
        self.leaves.push(node.clone());
        self.ops.push(Op::Leaf { slot: dest, index });
    }
    fn exact_at(&mut self, dest: u32) {
        self.exact |= dest == 0;
    }
    /// Splice a child program in at `dest`, remapping its slot 0 to `dest` and
    /// its scratch slots to fresh parent slots. Order is preserved verbatim.
    fn merge(&mut self, child: &FusedSampler, dest: u32) {
        if dest == 0 && child.exact {
            self.exact = true;
        }
        let mut map = Vec::with_capacity(child.scratch as usize + 1);
        map.push(dest);
        for _ in 0..child.scratch {
            map.push(self.alloc());
        }
        let leaf_base = self.leaves.len() as u32;
        self.leaves.extend_from_slice(&child.leaves);
        for op in &*child.ops {
            self.push(op.remap(&map, leaf_base));
        }
        for slot in map.drain(1..) {
            self.release(slot);
        }
    }
    fn operand(&mut self, value: &DensityValue) -> Operand {
        match *value {
            DensityValue::Constant(value) => Operand::Constant(value),
            DensityValue::Sampler(ref sampler) => {
                let slot = self.alloc();
                self.build(sampler, slot);
                Operand::Slot(slot)
            }
        }
    }
    fn build(&mut self, node: &Arc<dyn DensitySampler>, dest: u32) {
        if self.ops.len() >= Self::MAX_OPS {
            return self.leaf(node, dest);
        }
        let Some(any) = node.as_any() else {
            return self.leaf(node, dest);
        };
        if let Some(fused) = any.downcast_ref::<FusedSampler>() {
            if self.live + fused.scratch > Self::MAX_SLOTS {
                return self.leaf(node, dest);
            }
            return self.merge(fused, dest);
        }
        let Some(slots) = extra_slots(any) else {
            return self.leaf(node, dest);
        };
        if self.live + slots > Self::MAX_SLOTS {
            return self.leaf(node, dest);
        }
        if let Some(constant) = any.downcast_ref::<ConstantSampler>() {
            self.ops.push(Op::Constant {
                slot: dest,
                value: constant.0,
            });
        } else if let Some(unary) = any.downcast_ref::<UnarySampler>() {
            self.build(&unary.input, dest);
            self.push(Op::Unary {
                slot: dest,
                operation: unary.operation,
            });
        } else if let Some(binary) = any.downcast_ref::<BinarySampler>() {
            self.exact_at(dest);
            self.build(&binary.left, dest);
            let right = self.alloc();
            self.build(&binary.right, right);
            self.push(Op::Binary {
                operation: binary.operation,
                left: dest,
                right,
            });
            self.release(right);
        } else if let Some(power) = any.downcast_ref::<PowerSampler>() {
            self.exact_at(dest);
            self.build(&power.base, dest);
            let exponent = self.alloc();
            self.build(&power.exponent, exponent);
            self.push(Op::Power {
                base: dest,
                exponent,
            });
            self.release(exponent);
        } else if let Some(round) = any.downcast_ref::<RoundSampler>() {
            if round.multiple.is_some() {
                self.exact_at(dest);
            }
            self.build(&round.input, dest);
            let multiple = round.multiple.as_ref().map(|multiple| {
                let slot = self.alloc();
                self.build(multiple, slot);
                slot
            });
            self.push(Op::Round {
                slot: dest,
                mode: round.mode,
                multiple,
            });
            if let Some(slot) = multiple {
                self.release(slot);
            }
        } else if let Some(lerp) = any.downcast_ref::<LerpSampler>() {
            self.exact_at(dest);
            self.build(&lerp.alpha, dest);
            let first = self.operand(&lerp.first);
            let second = self.operand(&lerp.second);
            self.push(Op::Lerp {
                alpha: dest,
                first,
                second,
            });
            for operand in [first, second] {
                if let Operand::Slot(slot) = operand {
                    self.release(slot);
                }
            }
        } else if let Some(choice) = any.downcast_ref::<RangeChoiceSampler>() {
            self.exact_at(dest);
            self.build(&choice.when_in_range, dest);
            let input = self.alloc();
            self.build(&choice.input, input);
            let outside = self.alloc();
            self.build(&choice.when_out_of_range, outside);
            self.push(Op::RangeChoice {
                inside: dest,
                input,
                outside,
                min_inclusive: choice.min_inclusive,
                max_exclusive: choice.max_exclusive,
            });
            self.release(input);
            self.release(outside);
        } else if let Some(select) = any.downcast_ref::<IntervalSelectSampler>() {
            self.exact_at(dest);
            self.build(&select.input, dest);
            let mut branches = Vec::with_capacity(select.branches.len());
            for branch in &select.branches {
                let slot = self.alloc();
                self.build(branch, slot);
                branches.push(slot);
            }
            self.push(Op::IntervalSelect {
                input: dest,
                thresholds: select.thresholds.as_slice().into(),
                branches: branches.clone().into(),
            });
            for slot in branches {
                self.release(slot);
            }
        }
    }
}
/// Wrap a maximal fusible region in a `FusedSampler`; pass anything else
/// through. A region needs at least two combining ops to beat tree dispatch.
pub(super) fn wrap(sampler: Arc<dyn DensitySampler>) -> Arc<dyn DensitySampler> {
    let Some(any) = sampler.as_any() else {
        return sampler;
    };
    if any.is::<FusedSampler>() || extra_slots(any).is_none() {
        return sampler;
    }
    let mut builder = Builder::new();
    builder.build(&sampler, 0);
    if builder.work < 2 {
        return sampler;
    }
    Arc::new(FusedSampler {
        scalar: sampler,
        leaves: builder.leaves.into(),
        ops: builder.ops.into(),
        scratch: builder.scratch,
        exact: builder.exact,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
        fn pick<'a, T>(&mut self, values: &'a [T]) -> &'a T {
            &values[self.below(values.len() as u64) as usize]
        }
    }
    const EDGES: [u32; 12] = [
        0x0000_0000,
        0x8000_0000,
        0x0000_0001,
        0x8000_0001,
        0x3f00_0000,
        0xbf00_0000,
        0x3f80_0000,
        0xbf80_0000,
        0x7f7f_ffff,
        0xff7f_ffff,
        0x7f80_0000,
        0xff80_0000,
    ];
    fn float(rng: &mut Rng) -> f32 {
        match rng.below(3) {
            0 => f32::from_bits(*rng.pick(&EDGES)),
            1 => f32::from_bits(*rng.pick(&EDGES)) + 1.0,
            _ => f32::from_bits(rng.next() as u32),
        }
    }
    fn coordinate(rng: &mut Rng) -> i32 {
        const EDGES: [i32; 5] = [i32::MIN, -64, 0, 64, i32::MAX];
        match rng.below(2) {
            0 => *rng.pick(&EDGES),
            _ => rng.next() as i32,
        }
    }
    fn axis(rng: &mut Rng) -> Axis {
        match rng.below(3) {
            0 => Axis::X,
            1 => Axis::Y,
            _ => Axis::Z,
        }
    }
    /// Leaves, including coordinate remaps and a deliberately broken repeating
    /// gradient so error propagation is exercised.
    fn leaf(rng: &mut Rng, depth: u32) -> Arc<dyn DensitySampler> {
        match rng.below(4) {
            0 => Arc::new(ConstantSampler(float(rng))),
            1 => {
                let from = coordinate(rng);
                Arc::new(GradientSampler::new(
                    axis(rng),
                    match rng.below(3) {
                        0 => TilingMode::ClampToEdge,
                        1 => TilingMode::Repeat,
                        _ => TilingMode::MirroredRepeat,
                    },
                    from,
                    from.wrapping_add(coordinate(rng)),
                    float(rng),
                    float(rng),
                ))
            }
            2 => Arc::new(DistanceToPointSampler {
                point: [coordinate(rng); 3],
                metric: *rng.pick(&[
                    DistanceMetric::Euclidean,
                    DistanceMetric::EuclideanSquared,
                    DistanceMetric::Manhattan,
                    DistanceMetric::Chebyshev,
                ]),
            }),
            _ => Arc::new(SliceSampler {
                input: tree(rng, depth.saturating_sub(1)),
                axes: match rng.below(4) {
                    0 => SliceAxes::X(coordinate(rng)),
                    1 => SliceAxes::Y(coordinate(rng)),
                    2 => SliceAxes::Z(coordinate(rng)),
                    _ => SliceAxes::Xz {
                        x: coordinate(rng),
                        z: coordinate(rng),
                    },
                },
            }),
        }
    }
    fn unary(rng: &mut Rng) -> UnaryOp {
        match rng.below(21) {
            0 => UnaryOp::Abs,
            1 => UnaryOp::Negate,
            2 => UnaryOp::Square,
            3 => UnaryOp::Cube,
            4 => UnaryOp::Sqrt,
            5 => UnaryOp::Log,
            6 => UnaryOp::PowerBase(float(rng) as f64),
            7 => UnaryOp::PowerExponent(float(rng) as f64),
            8 => UnaryOp::Reciprocal,
            9 => UnaryOp::Sign,
            10 => UnaryOp::Squeeze,
            11 => UnaryOp::LeakyRelu(float(rng)),
            12 => UnaryOp::Add(float(rng)),
            13 => UnaryOp::SubtractFrom(float(rng)),
            14 => UnaryOp::Multiply(float(rng)),
            15 => UnaryOp::DivideInto(float(rng)),
            16 => UnaryOp::Minimum(float(rng)),
            17 => UnaryOp::Maximum(float(rng)),
            18 => UnaryOp::ConstantRangeChoice {
                min_inclusive: float(rng),
                max_exclusive: float(rng),
                inside: float(rng),
                outside: float(rng),
            },
            19 => UnaryOp::Clamp {
                min: float(rng),
                max: float(rng),
            },
            _ => UnaryOp::Add(-0.0),
        }
    }
    fn binary(rng: &mut Rng) -> BinaryOp {
        match rng.below(6) {
            0 => BinaryOp::Add,
            1 => BinaryOp::Subtract,
            2 => BinaryOp::Multiply,
            3 => BinaryOp::Divide,
            4 => BinaryOp::Minimum {
                right_min: float(rng),
            },
            _ => BinaryOp::Maximum {
                right_max: float(rng),
            },
        }
    }
    fn tree(rng: &mut Rng, depth: u32) -> Arc<dyn DensitySampler> {
        if depth == 0 || rng.below(4) == 0 {
            return leaf(rng, depth);
        }
        match rng.below(8) {
            0 => Arc::new(UnarySampler {
                input: tree(rng, depth - 1),
                operation: unary(rng),
            }),
            1 => Arc::new(BinarySampler {
                left: tree(rng, depth - 1),
                right: tree(rng, depth - 1),
                operation: binary(rng),
            }),
            2 => Arc::new(PowerSampler {
                base: tree(rng, depth - 1),
                exponent: tree(rng, depth - 1),
            }),
            3 => Arc::new(RoundSampler {
                mode: *rng.pick(&[
                    RoundMode::Floor,
                    RoundMode::Round,
                    RoundMode::Ceil,
                    RoundMode::Truncate,
                ]),
                input: tree(rng, depth - 1),
                multiple: if rng.below(2) == 0 {
                    None
                } else {
                    Some(tree(rng, depth - 1))
                },
            }),
            4 => Arc::new(LerpSampler {
                alpha: tree(rng, depth - 1),
                first: if rng.below(2) == 0 {
                    DensityValue::Constant(float(rng))
                } else {
                    DensityValue::Sampler(tree(rng, depth - 1))
                },
                second: if rng.below(2) == 0 {
                    DensityValue::Constant(float(rng))
                } else {
                    DensityValue::Sampler(tree(rng, depth - 1))
                },
            }),
            5 => Arc::new(RangeChoiceSampler {
                input: tree(rng, depth - 1),
                min_inclusive: float(rng),
                max_exclusive: float(rng),
                when_in_range: tree(rng, depth - 1),
                when_out_of_range: tree(rng, depth - 1),
            }),
            6 => {
                let count = 1 + rng.below(4) as usize;
                Arc::new(
                    IntervalSelectSampler::new(
                        tree(rng, depth - 1),
                        (0..count).map(|_| float(rng)).collect(),
                        (0..=count).map(|_| tree(rng, depth - 1)).collect(),
                    )
                    .unwrap(),
                )
            }
            _ => leaf(rng, depth),
        }
    }
    /// Bit-for-bit comparison over scalar and bulk evaluation, including the
    /// caller-buffer tail and the error contract.
    fn compare(tree: &Arc<dyn DensitySampler>, fused: &Arc<dyn DensitySampler>) {
        let volumes = [
            DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap(),
            DensityVolume::new([4, 3, 5], [-17, 100, 5], [1; 3]).unwrap(),
            DensityVolume::new([3, 2, 4], [i32::MAX - 8, -2, 0], [4, 8, 2]).unwrap(),
            DensityVolume::new([2; 3], [7; 3], [i32::MAX; 3]).unwrap(),
        ];
        let mut context = SamplerContext::cached();
        let mut fused_context = SamplerContext::cached();
        for volume in volumes {
            for tail in [0usize, 7] {
                let size = volume.size();
                let mut expected = Vec::with_capacity(size + tail);
                let mut actual = Vec::with_capacity(size + tail);
                for i in 0..size + tail {
                    expected.push(f32::from_bits(0xa500_0000 + i as u32));
                    actual.push(f32::from_bits(0xa500_0000 + i as u32));
                }
                let expected_result = tree.sample_volume(&mut context, &mut expected, volume);
                let actual_result = fused.sample_volume(&mut fused_context, &mut actual, volume);
                assert_eq!(expected_result.err(), actual_result.err());
                for (index, (expected, actual)) in expected.iter().zip(&actual).enumerate() {
                    assert_eq!(
                        expected.to_bits(),
                        actual.to_bits(),
                        "volume {volume:?} tail {tail} index {index}"
                    );
                }
            }
            for position in [
                [0; 3],
                [-5, 70, 300],
                [i32::MAX, i32::MIN, 0],
                [64, -64, i32::MAX - 1],
            ] {
                let expected = tree.sample_value(&mut context, position);
                let actual = fused.sample_value(&mut fused_context, position);
                match (expected, actual) {
                    (Ok(expected), Ok(actual)) => assert_eq!(expected.to_bits(), actual.to_bits()),
                    (expected, actual) => assert_eq!(expected.err(), actual.err()),
                }
            }
        }
    }
    #[test]
    fn fused_programs_match_tree_evaluation() {
        let mut rng = Rng(0x485a_4646_5553_4544);
        for _ in 0..400 {
            let tree = tree(&mut rng, 5);
            compare(&tree, &wrap(tree.clone()));
        }
    }
    #[test]
    fn fused_children_merge_into_parent_programs() {
        let mut rng = Rng(0x485a_4d45_5247_4521);
        for _ in 0..200 {
            // A fused child under a new fusible root must splice its program.
            let child = wrap(tree(&mut rng, 5));
            let tree: Arc<dyn DensitySampler> = match rng.below(3) {
                0 => Arc::new(UnarySampler {
                    input: child,
                    operation: unary(&mut rng),
                }),
                1 => Arc::new(BinarySampler {
                    left: child,
                    right: tree(&mut rng, 3),
                    operation: binary(&mut rng),
                }),
                _ => Arc::new(BinarySampler {
                    left: tree(&mut rng, 3),
                    right: child,
                    operation: binary(&mut rng),
                }),
            };
            compare(&tree, &wrap(tree.clone()));
        }
    }
    #[test]
    fn single_combine_regions_are_not_fused() {
        let unary: Arc<dyn DensitySampler> = Arc::new(UnarySampler {
            input: Arc::new(ConstantSampler(1.0)),
            operation: UnaryOp::Negate,
        });
        assert!(
            !wrap(unary)
                .as_any()
                .is_some_and(|any| any.is::<FusedSampler>())
        );
        let chain: Arc<dyn DensitySampler> = Arc::new(UnarySampler {
            input: Arc::new(UnarySampler {
                input: Arc::new(ConstantSampler(1.0)),
                operation: UnaryOp::Negate,
            }),
            operation: UnaryOp::Multiply(3.0),
        });
        let fused = wrap(chain);
        assert!(fused.as_any().is_some_and(|any| any.is::<FusedSampler>()));
        let mut context = SamplerContext::default();
        let mut buffer = [1.0f32; 4];
        let volume = DensityVolume::new([2, 2, 1], [0; 3], [1; 3]).unwrap();
        fused
            .sample_volume(&mut context, &mut buffer, volume)
            .unwrap();
        assert!(buffer.iter().all(|value| *value == -3.0));
    }
}
