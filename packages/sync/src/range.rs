use std::{
    iter::Sum,
    ops::{Add, Mul, Sub},
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

/// Problem: We have a list of ValueRanges, and we want to know the maximum value.
/// This is not one clear value, as we consider the maximum if all commit and maximum if all rollback.
/// The result is the range of possible maximum values (different than spread)  
pub fn max_range<'a, I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.copied()
        .reduce(|acc, x| {
            ValueRange(
                std::cmp::max(acc.min(), x.min()),
                std::cmp::max(acc.max(), x.max()),
            )
        })
        .unwrap_or_default()
}

/// Problem: We have a list of ValueRanges, and we want to know the minimum value.
/// This is not one clear value, as we consider the minimum if all commit and minimum if all rollback.
/// The result is the range of possible minimum values (different than spread)  
pub fn min_range<'a, I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.copied()
        .reduce(|acc, x| {
            ValueRange(
                std::cmp::min(acc.min(), x.min()),
                std::cmp::min(acc.max(), x.max()),
            )
        })
        .unwrap_or_default()
}

/// Captures the spread from the lowest low to the highest high
pub fn spread<'a, I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.copied()
        .reduce(|acc, x| {
            ValueRange(
                std::cmp::min(acc.min(), x.min()),
                std::cmp::max(acc.max(), x.max()),
            )
        })
        .unwrap_or_default()
}

impl<T, U> Mul<U> for ValueRange<T>
where
    T: Mul<U, Output = T>,
    U: Copy,
{
    type Output = ValueRange<T>;

    fn mul(self, rhs: U) -> Self::Output {
        ValueRange(self.0 * rhs, self.1 * rhs)
    }
}

impl<T, U> Mul<U> for &ValueRange<T>
where
    T: Mul<U, Output = T> + Copy,
    U: Copy,
{
    type Output = ValueRange<T>;

    fn mul(self, rhs: U) -> Self::Output {
        ValueRange(self.0 * rhs, self.1 * rhs)
    }
}

impl<T> ValueRange<T>
where
    T: Add<Output = T> + Sub<Output = T> + Ord + Copy,
{
    /// Returns true iff all values of the range are <= max.
    /// This can be used to assert invariants
    pub fn is_under_max(&self, max: T) -> bool {
        self.1 <= max
    }

    /// Returns true iff all values of the range are >= min.
    /// This can be used to assert invariants
    pub fn is_over_min(&self, min: T) -> bool {
        self.0 >= min
    }

    /// This is to be called at the beginning of a transaction, to reserve the ability to commit (or rollback) an addition.
    /// If the last value is set, it enforces that the new maximum will remain under that limit.
    /// Usage: `range.prepare_add(20, None)?;` or `range.prepare_add(20, 100)?;`
    pub fn prepare_add(&mut self, value: T, max: impl Into<Option<T>>) -> Result<(), RangeError> {
        if let Some(max) = max.into() {
            if self.1 + value > max {
                return Err(RangeError::Overflow);
            }
        }
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

    /// This is to be called at the beginning of a transaction, to reserve the ability to commit
    /// (or rollback) a subtraction.
    /// You can specify a minimum value that the range must never go below, which is enforced here.
    /// No minimum: `range.prepare_sub(20, None)?;`
    /// Minimum of 0 (for uints): `range.prepare_sub(20, 0)?;`
    /// Higher minimum :  `range.prepare_sub(20, 100)?;`
    pub fn prepare_sub(&mut self, value: T, min: impl Into<Option<T>>) -> Result<(), RangeError> {
        if let Some(min) = min.into() {
            // use plus not minus here, as we are much more likely to have underflow on u64 or Uint128 than overflow
            if self.0 < min + value {
                return Err(RangeError::Underflow);
            }
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
    use cosmwasm_std::{Decimal, Uint128};

    use super::*;

    #[test]
    fn comparisons() {
        // check for one point - it behaves like an integer
        let mut range = ValueRange::new(50);
        // valid_min + valid_max is like equals
        assert!(range.is_under_max(50));
        assert!(range.is_over_min(50));
        // is_under_max + !is_over_min is >=
        assert!(range.is_under_max(51));
        assert!(!range.is_over_min(51));
        // is_over_min + !is_under_max is <=
        assert!(!range.is_under_max(49));
        assert!(range.is_over_min(49));

        // make a range (50, 80), it should compare properly to those outside the range
        range.prepare_add(30, None).unwrap();
        assert!(!range.is_under_max(49));
        assert!(range.is_over_min(49));
        assert!(range.is_under_max(81));
        assert!(!range.is_over_min(81));

        // all comparisons inside the range lead to false
        assert!(!range.is_under_max(60));
        assert!(!range.is_over_min(60));
    }

    #[test]
    fn add_ranges() {
        // (80, 120)
        let mut range = ValueRange::new(80);
        range.prepare_add(40, None).unwrap();

        // (100, 200)
        let mut other = ValueRange::new(200);
        other.prepare_sub(100, 0).unwrap();

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

        // (max value if all rollback, max value if all commit)
        let max = max_range(ranges.iter());
        assert_eq!(max, ValueRange(200, 380));

        // (min value if all rollback, min value if all commit)
        let min = min_range(ranges.iter());
        assert_eq!(min, ValueRange(40, 100));

        // (min value if all rollback, max value if all commit)
        let all = spread(ranges.iter());
        assert_eq!(all, ValueRange(40, 380));
    }

    // most tests will use i32 for simplicity - just ensure APIs work properly with Uint128
    #[test]
    fn works_with_uint128() {
        // (80, 120)
        let mut range = ValueRange::new(Uint128::new(80));
        range.prepare_add(Uint128::new(40), None).unwrap();

        // (100, 200)
        let mut other = ValueRange::new(Uint128::new(200));
        other
            .prepare_sub(Uint128::new(100), Uint128::zero())
            .unwrap();

        let total = range + other;
        assert_eq!(total, ValueRange(Uint128::new(180), Uint128::new(320)));
    }

    // This test attempts to use the API in a realistic scenario.
    // A user has X collateral and makes some liens on this collateral, which execute asynchronously.
    // That is, we want to process other transactions while the liens are being executed, while ensuring there
    // will not be a conflict on rollback or commit.
    //
    // using u64 not Uint128 here as less verbose
    #[test]
    fn real_world_usage() {
        let mut collateral = 10_000u64;
        let mut lien = ValueRange::new(0u64);

        // prepare some lien
        lien.prepare_add(2_000, collateral).unwrap();
        lien.prepare_add(5_000, collateral).unwrap();

        // cannot add too much
        let err = lien.prepare_add(3_500, collateral).unwrap_err();
        assert_eq!(err, RangeError::Overflow);

        // let's commit the second pending lien (only 2000 left)
        // QUESTION: should we enforce the min/max on commit/rollback explicitly and pass them in?
        lien.commit_add(5_000);
        assert_eq!(lien, ValueRange(5_000, 7_000));

        // See we cannot reduce this by 4_000
        assert!(!lien.is_under_max(collateral - 4_000));
        // See we can reduce this by 2_000
        assert!(lien.is_under_max(collateral - 2_000));
        collateral -= 2_000;

        // start unbonding 3_000
        lien.prepare_sub(3_000, 0).unwrap();
        // still; cannot increase max (7_000) over the new cap of 8_000
        let err = lien.prepare_add(1_500, collateral).unwrap_err();
        assert_eq!(err, RangeError::Overflow);

        // if we rollback the other pending lien, this works
        lien.rollback_add(2_000);
        assert_eq!(lien, ValueRange(2_000, 5_000));
        lien.prepare_add(1_500, collateral).unwrap();
    }

    // idea here is to model the liens as in vault, and ensure we can calculate aggregates over them properly
    // we want to track max lien (which will be a range) and maximum slashable.
    #[test]
    fn invariants_over_set_of_liens() {
        // some existing outstanding liens
        let liens = vec![
            ValueRange(Uint128::new(5000), Uint128::new(7000)),
            ValueRange(Uint128::new(2000), Uint128::new(8000)),
            ValueRange(Uint128::new(3000), Uint128::new(12000)),
        ];
        // for simplicity, assume all slash rates are the same, easier for writing tests, but ensures operations are allowed
        let slash_rate = Decimal::percent(10);

        // the max lien is actually a range of (max if all rollback, max if all commit)
        let max_lien = max_range(liens.iter());
        assert_eq!(
            max_lien,
            ValueRange(Uint128::new(5000), Uint128::new(12000))
        );
        // check if this is less than some collateral
        assert!(max_lien.is_under_max(Uint128::new(15000)));
        assert!(max_lien.is_under_max(Uint128::new(12000)));
        assert!(!max_lien.is_under_max(Uint128::new(11900)));

        // max slashable is a sum of all liens * slash_rate
        let max_slashable: ValueRange<Uint128> = liens.iter().map(|l| l * slash_rate).sum();
        assert_eq!(
            max_slashable,
            ValueRange(Uint128::new(1000), Uint128::new(2700))
        );
        // check if this is less than some collateral
        assert!(max_slashable.is_under_max(Uint128::new(5000)));
        assert!(!max_slashable.is_under_max(Uint128::new(2600)));

        // check if this is over some limit or not
        assert!(max_slashable.is_over_min(Uint128::new(1000)));
        assert!(!max_slashable.is_over_min(Uint128::new(1100)));

        // we can also see the aggregate range (not needed here, but let's check anyway)
        let lien_spread = spread(liens.iter());
        assert_eq!(
            lien_spread,
            ValueRange(Uint128::new(2000), Uint128::new(12000))
        );
    }
}
