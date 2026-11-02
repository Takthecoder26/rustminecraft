//! Authored uniform defaults, folded and type-checked by naga, then stored in the actual
//! uniform member's layout. The shader still reads its UBO; these are only its initial bytes.

use std::collections::BTreeMap;

use naga::{ArraySize, Expression, Handle, Literal, Module, ScalarKind, Type, TypeInner};

use super::{Error, scan::Stage};

pub(super) fn read(
    stages: &[(&Stage<'_>, &Module)],
) -> Result<BTreeMap<String, Vec<u8>>, Vec<Error>> {
    let mut defaults = BTreeMap::new();
    let mut initial_values = BTreeMap::new();
    let mut errors = Vec::new();
    for &(stage, module) in stages {
        let members = module.global_variables.iter().find_map(|(_, var)| {
            if var
                .binding
                .as_ref()
                .is_some_and(|b| b.group == 0 && b.binding == 0)
                && let TypeInner::Struct { members, .. } = &module.types[var.ty].inner
            {
                Some(members)
            } else {
                None
            }
        });
        for (name, mirror, at) in &stage.uniform_defaults {
            // Dedicated suppliers override initial values, but their linked declarations
            // must still agree even when the frame UBO does not contain that uniform.
            let member = members.and_then(|members| {
                members.iter().find(|m| {
                    m.name
                        .as_deref()
                        .is_some_and(|n| n.strip_prefix("hz_bool_").unwrap_or(n) == name)
                })
            });
            let bytes = module
                .constants
                .iter()
                .find(|(_, c)| c.name.as_ref() == Some(mirror))
                .and_then(|(_, c)| encode(module, c.init, member.map_or(c.ty, |m| m.ty)));
            match bytes {
                Some((values, bytes)) => {
                    // Compare typed values: +0.0 and -0.0 are equal, while integer high bits
                    // remain significant. Keep the first seed's original zero sign.
                    if initial_values.get(name).is_some_and(|old| old != &values) {
                        errors.push(stage.error(
                            *at,
                            &format!("`{name}` has different initializers in linked declarations"),
                        ));
                    } else {
                        initial_values.entry(name.clone()).or_insert(values);
                        defaults.entry(name.clone()).or_insert(bytes);
                    }
                }
                None => errors.push(stage.error(
                    *at,
                    &format!("`{name}`: unsupported constant uniform initializer"),
                )),
            }
        }
    }
    if errors.is_empty() {
        Ok(defaults)
    } else {
        Err(errors)
    }
}

fn encode(
    module: &Module,
    expression: Handle<Expression>,
    ty: Handle<Type>,
) -> Option<(Vec<Literal>, Vec<u8>)> {
    let mut components = Vec::new();
    flatten(module, expression, &mut components)?;
    let mut remaining = components.iter().copied();
    let mut bytes = vec![0; module.types[ty].inner.size(module.to_ctx()) as usize];
    write(module, ty, &mut remaining, &mut bytes, 0)?;
    if remaining.next().is_some() {
        return None;
    }
    Some((components, bytes))
}

fn flatten(module: &Module, expression: Handle<Expression>, out: &mut Vec<Literal>) -> Option<()> {
    match &module.global_expressions[expression] {
        Expression::Literal(value) => out.push(*value),
        Expression::Constant(c) => flatten(module, module.constants[*c].init, out)?,
        Expression::Compose { components, .. } => {
            for &c in components {
                flatten(module, c, out)?;
            }
        }
        Expression::Splat { size, value } => {
            for _ in 0..*size as usize {
                flatten(module, *value, out)?;
            }
        }
        Expression::ZeroValue(ty) => zeros(module, *ty, out)?,
        _ => return None,
    }
    Some(())
}

fn zeros(module: &Module, ty: Handle<Type>, out: &mut Vec<Literal>) -> Option<()> {
    let (count, scalar) = match module.types[ty].inner {
        TypeInner::Scalar(s) => (1, s),
        TypeInner::Vector { size, scalar } => (size as usize, scalar),
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => (columns as usize * rows as usize, scalar),
        TypeInner::Array {
            base,
            size: ArraySize::Constant(n),
            ..
        } => {
            for _ in 0..n.get() {
                zeros(module, base, out)?;
            }
            return Some(());
        }
        TypeInner::Struct { ref members, .. } => {
            for m in members {
                zeros(module, m.ty, out)?;
            }
            return Some(());
        }
        _ => return None,
    };
    let zero = match (scalar.kind, scalar.width) {
        (ScalarKind::Float, 4) => Literal::F32(0.0),
        (ScalarKind::Float, 8) => Literal::F64(0.0),
        (ScalarKind::Float, 2) => Literal::F16(half::f16::from_bits(0)),
        (ScalarKind::Sint, 4) => Literal::I32(0),
        (ScalarKind::Sint, 8) => Literal::I64(0),
        (ScalarKind::Sint, 2) => Literal::I16(0),
        (ScalarKind::Uint, 4) => Literal::U32(0),
        (ScalarKind::Uint, 8) => Literal::U64(0),
        (ScalarKind::Uint, 2) => Literal::U16(0),
        (ScalarKind::Bool, _) => Literal::Bool(false),
        _ => return None,
    };
    out.extend(std::iter::repeat_n(zero, count));
    Some(())
}

fn put(
    values: &mut impl Iterator<Item = Literal>,
    out: &mut [u8],
    at: usize,
    width: u8,
) -> Option<()> {
    let bytes = match values.next()? {
        Literal::F32(v) => v.to_ne_bytes().to_vec(),
        Literal::I32(v) => v.to_ne_bytes().to_vec(),
        Literal::U32(v) => v.to_ne_bytes().to_vec(),
        Literal::Bool(v) => u32::from(v).to_ne_bytes().to_vec(),
        Literal::F64(v) => v.to_ne_bytes().to_vec(),
        Literal::I64(v) => v.to_ne_bytes().to_vec(),
        Literal::U64(v) => v.to_ne_bytes().to_vec(),
        Literal::F16(v) => v.to_bits().to_ne_bytes().to_vec(),
        Literal::I16(v) => v.to_ne_bytes().to_vec(),
        Literal::U16(v) => v.to_ne_bytes().to_vec(),
        _ => return None,
    };
    if bytes.len() != width as usize {
        return None;
    }
    out.get_mut(at..at + bytes.len())?.copy_from_slice(&bytes);
    Some(())
}

fn write(
    module: &Module,
    ty: Handle<Type>,
    values: &mut impl Iterator<Item = Literal>,
    out: &mut [u8],
    at: usize,
) -> Option<()> {
    match module.types[ty].inner {
        TypeInner::Scalar(s) => put(values, out, at, s.width)?,
        TypeInner::Vector { size, scalar } => {
            for i in 0..size as usize {
                put(values, out, at + i * scalar.width as usize, scalar.width)?;
            }
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => {
            let stride = naga::proc::Alignment::from(rows) * u32::from(scalar.width);
            for c in 0..columns as usize {
                for r in 0..rows as usize {
                    put(
                        values,
                        out,
                        at + c * stride as usize + r * scalar.width as usize,
                        scalar.width,
                    )?;
                }
            }
        }
        TypeInner::Array {
            base,
            size: ArraySize::Constant(n),
            stride,
        } => {
            for i in 0..n.get() as usize {
                write(module, base, values, out, at + i * stride as usize)?;
            }
        }
        TypeInner::Struct { ref members, .. } => {
            for m in members {
                write(module, m.ty, values, out, at + m.offset as usize)?;
            }
        }
        _ => return None,
    }
    Some(())
}
