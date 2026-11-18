use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Logic {
    Zero,
    One,
    X,
    #[default]
    Z,
}

impl Logic {
    pub fn from_bool(value: bool) -> Self {
        if value {
            Self::One
        } else {
            Self::Zero
        }
    }
    pub fn bit(self) -> Option<bool> {
        match self {
            Self::Zero => Some(false),
            Self::One => Some(true),
            _ => None,
        }
    }
    pub fn invert(self) -> Self {
        match self {
            Self::Zero => Self::One,
            Self::One => Self::Zero,
            _ => Self::X,
        }
    }
    pub fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Zero, _) | (_, Self::Zero) => Self::Zero,
            (Self::One, Self::One) => Self::One,
            _ => Self::X,
        }
    }
    pub fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::One, _) | (_, Self::One) => Self::One,
            (Self::Zero, Self::Zero) => Self::Zero,
            _ => Self::X,
        }
    }
    pub fn xor(self, other: Self) -> Self {
        match (self.bit(), other.bit()) {
            (Some(a), Some(b)) => Self::from_bool(a ^ b),
            _ => Self::X,
        }
    }
    pub fn mux(select: Self, low: Self, high: Self) -> Self {
        match select {
            Self::Zero => low,
            Self::One => high,
            _ if low == high => low,
            _ => Self::X,
        }
    }
    pub fn resolve(drivers: impl IntoIterator<Item = Self>) -> Self {
        drivers.into_iter().fold(Self::Z, |a, b| match (a, b) {
            (Self::Z, v) | (v, Self::Z) => v,
            (a, b) if a == b => a,
            _ => Self::X,
        })
    }
    /// Four-input table in physical LSB address order. Unknown addresses merge
    /// every possible entry; Z is unknown as an address, never a fifth bit value.
    pub fn table(table: &[Self; 16], inputs: &[Self; 4]) -> Self {
        let mut result = None;
        for (address, &stored) in table.iter().enumerate() {
            let value = if stored == Self::Z { Self::X } else { stored };
            if inputs
                .iter()
                .enumerate()
                .any(|(i, v)| v.bit().is_some_and(|b| b != (address & (1 << i) != 0)))
            {
                continue;
            }
            if result.is_some_and(|old| old != value) {
                return Self::X;
            }
            result = Some(value);
        }
        result.unwrap_or(Self::X)
    }
    /// Unknown addresses only produce X when the possible table entries differ.
    pub fn lut(mask: u16, inputs: &[Self]) -> Self {
        if let Some(address) = inputs.iter().enumerate().try_fold(0, |address, (i, v)| {
            Some(address | ((v.bit()? as usize) << i))
        }) {
            return Self::from_bool(mask & (1 << address) != 0);
        }
        let mut possible = None;
        for address in 0..(1 << inputs.len()) {
            if inputs
                .iter()
                .enumerate()
                .any(|(i, v)| v.bit().is_some_and(|b| b != (address & (1 << i) != 0)))
            {
                continue;
            }
            let value = Self::from_bool(mask & (1 << address) != 0);
            if possible.is_some_and(|old| old != value) {
                return Self::X;
            }
            possible = Some(value);
        }
        possible.unwrap_or(Self::X)
    }
}

impl fmt::Display for Logic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Zero => "0",
            Self::One => "1",
            Self::X => "x",
            Self::Z => "z",
        })
    }
}

impl std::str::FromStr for Logic {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "0" => Ok(Self::Zero),
            "1" => Ok(Self::One),
            "x" | "X" => Ok(Self::X),
            "z" | "Z" => Ok(Self::Z),
            _ => anyhow::bail!("expected 0, 1, x or z, got {s}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Logic;
    use super::Logic::*;
    #[test]
    fn unknowns_do_not_destroy_known_results() {
        assert_eq!(Logic::lut(0xffff, &[X, Z, X, Z]), One);
        assert_eq!(Logic::lut(0xaaaa, &[Zero, X, X, X]), Zero);
        assert_eq!(Logic::lut(0xaaaa, &[X, Zero, Zero, Zero]), X);
        assert_eq!(Logic::resolve([Z, One, One]), One);
        assert_eq!(Logic::resolve([Zero, One]), X);
        assert_eq!(Logic::mux(X, One, One), One);
    }
}
