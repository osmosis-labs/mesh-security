use std::{
    iter::Sum,
    ops::{Add, Sub},
};
use thiserror::Error;

use cosmwasm_schema::cw_serde;

/// This is designed to work with two numeric primitives that can be added, subtracted, and compared.
#[cw_serde]
#[derive(Default, Copy)]
pub struct ValueRange<T>(T, T);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RangeError {
    #[error("Underflow minimum value")]
    Underflow,
    #[error("Overflow maximum value")]
    Overflow,
}

impl<T> ValueRange<T>
where
    T: Copy,
{
    pub fn new(value: T) -> Self {
        Self(value, value)
    }

    pub fn min(&self) -> T {
        self.0
    }

    pub fn max(&self) -> T {
        self.1
    }
}

pub fn max_val<'a, I, T>(iter: I) -> T
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.map(|r| r.max()).max().unwrap_or_default()
}

pub fn min_val<'a, I, T>(iter: I) -> T
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.map(|r| r.min()).min().unwrap_or_default()
}

/// Captures the spread from the lowest low to the highest high
pub fn spread<I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = ValueRange<T>>,
    T: Ord + Copy + Default,
{
    iter.reduce(|acc, x| {
        ValueRange(
            std::cmp::min(acc.min(), x.min()),
            std::cmp::max(acc.max(), x.max()),
        )
    })
    .unwrap_or_default()
}

impl<T: Ord> ValueRange<T> {
    pub fn contains(&self, value: T) -> bool {
        self.0 <= value && value <= self.1
    }
}

impl<T> ValueRange<T>
where
    T: Add<Output = T> + Sub<Output = T> + Ord + Copy,
{
    /// This is to be called at the beginning of a transaction, to reserve the ability to commit (or rollback) an addition
    pub fn prepare_add(&mut self, value: T) -> Result<(), RangeError> {
        // FIXME: assert some max?
        self.1 = self.1 + value;
        Ok(())
    }

    /// The caller should limit these to only previous `prepare_add` calls.
    /// We will panic on mistake as this should never happen
    pub fn rollback_add(&mut self, value: T) {
        self.1 = self.1 - value;
        self.assert_valid_range();
    }

    /// The caller should limit these to only previous `prepare_add` calls.
    /// We will panic on mistake as this should never happen
    pub fn commit_add(&mut self, value: T) {
        self.0 = self.0 + value;
        self.assert_valid_range();
    }

    /// This is to be called at the beginning of a transaction, to reserve the ability to commit (or rollback) a subtraction
    pub fn prepare_sub(&mut self, value: T) -> Result<(), RangeError> {
        if self.0 < value {
            return Err(RangeError::Underflow);
        }
        self.0 = self.0 - value;
        Ok(())
    }

    /// The caller should limit these to only previous `prepare_sub` calls.
    /// We will panic on mistake as this should never happen
    pub fn rollback_sub(&mut self, value: T) {
        self.0 = self.0 + value;
        self.assert_valid_range();
    }

    /// The caller should limit these to only previous `prepare_sub` calls.
    /// We will panic on mistake as this should never happen
    pub fn commit_sub(&mut self, value: T) {
        self.1 = self.1 - value;
        self.assert_valid_range();
    }

    #[inline]
    fn assert_valid_range(&self) {
        assert!(self.0 <= self.1);
    }
}

impl<T: PartialOrd> PartialEq<T> for ValueRange<T> {
    fn eq(&self, other: &T) -> bool {
        self.0 == self.1 && self.0 == *other
    }
}

impl<T: PartialOrd> PartialOrd<T> for ValueRange<T> {
    fn partial_cmp(&self, other: &T) -> Option<std::cmp::Ordering> {
        if other < &self.0 {
            Some(std::cmp::Ordering::Greater)
        } else if other > &self.1 {
            Some(std::cmp::Ordering::Less)
        } else if self.eq(other) {
            Some(std::cmp::Ordering::Equal)
        } else {
            None
        }
    }
}

impl<T: Add<Output = T>> Add for ValueRange<T> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        ValueRange(self.0 + rhs.0, self.1 + rhs.1)
    }
}

impl<T: Add<Output = T> + Default> Sum for ValueRange<T> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(ValueRange::default(), |acc, x| acc + x)
    }
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::Uint128;

    use super::*;

    #[test]
    fn comparisons() {
        // check for one point - it behaves like an integer
        let mut range = ValueRange::new(50);
        assert!(range == 50);
        assert!(range > 49);
        assert!(range < 51);

        // make a range (50, 80), it should compare normally to those outside the range
        range.prepare_add(30).unwrap();
        assert!(range > 49);
        assert!(range < 81);

        // all comparisons inside the range lead to false
        assert!(!(range < 60));
        assert!(!(range > 60));
        assert!(range != 60);
    }

    #[test]
    fn add_ranges() {
        // (80, 120)
        let mut range = ValueRange::new(80);
        range.prepare_add(40).unwrap();

        // (100, 200)
        let mut other = ValueRange::new(200);
        other.prepare_sub(100).unwrap();

        let total = range + other;
        assert_eq!(total, ValueRange(180, 320));
    }

    #[test]
    fn sums() {
        let ranges = [
            ValueRange::new(100),
            ValueRange(0, 250),
            ValueRange::new(200),
            ValueRange(170, 380),
        ];
        let total: ValueRange<u32> = ranges.into_iter().sum();
        assert_eq!(total, ValueRange(470, 930));
    }

    #[test]
    fn min_max() {
        let ranges = [
            ValueRange::new(100),
            ValueRange(40, 250),
            ValueRange::new(200),
            ValueRange(170, 380),
        ];
        let max = max_val(ranges.iter());
        assert_eq!(max, 380);

        let min = min_val(ranges.iter());
        assert_eq!(min, 40);

        let all = spread(ranges.into_iter());
        assert_eq!(all, ValueRange(40, 380));
    }

    // most tests will use i32 for simplicity - just ensure APIs work properly with Uint128
    #[test]
    fn works_with_uint128() {
        // check for one point - it behaves like an integer
        let mut range = ValueRange::new(Uint128::new(500));
        assert!(range == Uint128::new(500));
        assert!(range > Uint128::new(499));
        assert!(range < Uint128::new(501));

        // make a range (50, 80), it should compare normally to those outside the range
        range.prepare_add(Uint128::new(250)).unwrap();
        assert!(range > Uint128::new(499));
        assert!(range < Uint128::new(751));

        // all comparisons inside the range lead to false
        assert!(!(range < Uint128::new(600)));
        assert!(!(range > Uint128::new(600)));
        assert!(range != Uint128::new(600));
    }
}
