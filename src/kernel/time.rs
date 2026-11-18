//! Exact, nonnegative rational picoseconds; no floating point or period rounding.
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{cmp::Ordering, fmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct Time {
    numerator: u64,
    denominator: u64,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Time {
    pub const ZERO: Self = Self::from_ps(0);

    pub const fn from_ps(value: u64) -> Self {
        Self {
            numerator: value,
            denominator: 1,
        }
    }
    pub fn new(numerator: u64, denominator: u64) -> Result<Self> {
        Self::reduced(u128::from(numerator), u128::from(denominator))
    }
    fn reduced(numerator: u128, denominator: u128) -> Result<Self> {
        ensure!(denominator != 0, "time denominator is zero");
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: (numerator / divisor)
                .try_into()
                .context("time numerator exceeds u64 after reduction")?,
            denominator: (denominator / divisor)
                .try_into()
                .context("time denominator exceeds u64 after reduction")?,
        })
    }
    pub fn ratio(self) -> (u64, u64) {
        (self.numerator, self.denominator)
    }
    pub fn checked_add(self, other: Self) -> Result<Self> {
        let divisor = gcd(u128::from(self.denominator), u128::from(other.denominator));
        let left = u128::from(other.denominator) / divisor;
        let right = u128::from(self.denominator) / divisor;
        Self::reduced(
            (u128::from(self.numerator) * left)
                .checked_add(u128::from(other.numerator) * right)
                .context("time addition exceeds u128")?,
            u128::from(self.denominator) * left,
        )
    }
    pub fn checked_mul(self, count: u64) -> Result<Self> {
        Self::reduced(
            u128::from(self.numerator) * u128::from(count),
            u128::from(self.denominator),
        )
    }
    pub fn checked_sub(self, other: Self) -> Result<Self> {
        ensure!(self >= other, "time subtraction would be negative");
        let divisor = gcd(u128::from(self.denominator), u128::from(other.denominator));
        let left = u128::from(other.denominator) / divisor;
        let right = u128::from(self.denominator) / divisor;
        Self::reduced(
            u128::from(self.numerator) * left - u128::from(other.numerator) * right,
            u128::from(self.denominator) * left,
        )
    }
    /// `self × numerator / denominator`, exactly.
    pub fn scaled(self, numerator: u64, denominator: u64) -> Result<Self> {
        ensure!(denominator != 0, "time scale denominator is zero");
        Self::reduced(
            u128::from(self.numerator) * u128::from(numerator),
            u128::from(self.denominator) * u128::from(denominator),
        )
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Self) -> Ordering {
        (u128::from(self.numerator) * u128::from(other.denominator))
            .cmp(&(u128::from(other.numerator) * u128::from(self.denominator)))
    }
}
impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(f, "{} ps", self.numerator)
        } else {
            write!(f, "{}/{} ps", self.numerator, self.denominator)
        }
    }
}
