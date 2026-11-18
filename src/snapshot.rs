//! Fallible allocation of the generation snapshot's owned buffers.
use std::{collections::HashMap, hash::Hash};

pub(crate) type AllocationResult<T> = Result<T, std::collections::TryReserveError>;

pub(crate) fn buffer<T>(capacity: usize) -> AllocationResult<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(capacity)?;
    Ok(out)
}

/// Reserves the outer buffer first; owners handle any large nested buffers.
pub(crate) fn cloned<T: Clone>(values: &[T]) -> AllocationResult<Vec<T>> {
    let mut out = buffer(values.len())?;
    out.extend_from_slice(values);
    Ok(out)
}

pub(crate) fn string(value: &str) -> AllocationResult<String> {
    let mut out = String::new();
    out.try_reserve_exact(value.len())?;
    out.push_str(value);
    Ok(out)
}

pub(crate) fn map<K: Copy + Eq + Hash, V: Clone>(
    values: &HashMap<K, V>,
) -> AllocationResult<HashMap<K, V>> {
    let mut out = HashMap::new();
    out.try_reserve(values.len())?;
    out.extend(values.iter().map(|(&k, v)| (k, v.clone())));
    Ok(out)
}
