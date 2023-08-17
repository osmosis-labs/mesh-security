use std::{
    iter::Sum,
    ops::{Add, Mul, Sub},
};
use thiserror::Error;

use cosmwasm_schema::cw_serde;

/// This is designed to work with two numeric primitives that can be added, subtracted, and compared.
#[cw_serde]
#[derive(Default, Copy)]
pub struct ValueRange<T> {
    #[serde(rename = "l")]
    low: T,
    #[serde(rename = "h")]
    high: T,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RangeError {
    #[error("Underflow minimum value")]
    Underflow,
    #[error("Overflow maximum value")]
    Overflow,
    #[error("Range is not one value")]
    NotOneValue,
}

impl<T> ValueRange<T> {
    #[inline]
    pub fn new(min: T, max: T) -> Self {
        Self {
            low: min,
            high: max,
        }
    }
}

impl<T> ValueRange<T>
where
    T: Copy,
{
    #[inline]
    pub fn new_val(value: T) -> Self {
        Self {
            low: value,
            high: value,
        }
    }

    #[inline]
    pub fn low(&self) -> T {
        self.low
    }

    #[inline]
    pub fn high(&self) -> T {
        self.high
    }
}

impl<T> ValueRange<T>
where
    T: Copy + PartialEq,
{
    /// If lo == hi, then return this value, otherwise an error
    pub fn val(&self) -> Result<T, RangeError> {
        if self.low == self.high {
            Ok(self.low)
        } else {
            Err(RangeError::NotOneValue)
        }
    }
}

pub fn max_range<T: Ord + Copy>(a: ValueRange<T>, b: ValueRange<T>) -> ValueRange<T> {
    ValueRange {
        low: std::cmp::max(a.low(), b.low()),
        high: std::cmp::max(a.high(), b.high()),
    }
}

// TODO: deprecate this?
/// Problem: We have a list of ValueRanges, and we want to know the maximum value.
/// This is not one clear value, as we consider the maximum if all commit and maximum if all rollback.
/// The result is the range of possible maximum values (different than spread)
pub fn reduce_max_range<'a, I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.copied().reduce(max_range).unwrap_or_default()
}

pub fn min_range<T: Ord + Copy>(a: ValueRange<T>, b: ValueRange<T>) -> ValueRange<T> {
    ValueRange {
        low: std::cmp::min(a.low(), b.low()),
        high: std::cmp::min(a.high(), b.high()),
    }
}

/// Problem: We have a list of ValueRanges, and we want to know the minimum value.
/// This is not one clear value, as we consider the minimum if all commit and minimum if all rollback.
/// The result is the range of possible minimum values (different than spread)
pub fn reduce_min_range<'a, I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.copied().reduce(min_range).unwrap_or_default()
}

/// Captures the spread from the lowest low to the highest high
pub fn spread<'a, I, T>(iter: I) -> ValueRange<T>
where
    I: Iterator<Item = &'a ValueRange<T>> + 'a,
    T: Ord + Copy + Default + 'a,
{
    iter.copied()
        .reduce(|a, b| ValueRange {
            low: std::cmp::min(a.low(), b.low()),
            high: std::cmp::max(a.high(), b.high()),
        })
        .unwrap_or_default()
}

impl<T, U> Mul<U> for ValueRange<T>
where
    T: Mul<U, Output = T>,
    U: Copy,
{
    type Output = ValueRange<T>;

    #[inline]
    fn mul(self, rhs: U) -> Self::Output {
        ValueRange::new(self.low * rhs, self.high * rhs)
    }
}

impl<T, U> Mul<U> for &ValueRange<T>
where
    T: Mul<U, Output = T> + Copy,
    U: Copy,
{
    type Output = ValueRange<T>;

    #[inline]
    fn mul(self, rhs: U) -> Self::Output {
        ValueRange::new(self.low * rhs, self.high * rhs)
    }
}

impl<T: Add<Output = T>> Add for ValueRange<T> {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        ValueRange::new(self.low + rhs.low, self.high + rhs.high)
    }
}

impl<T: Add<Output = T> + Default> Sum for ValueRange<T> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(ValueRange::default(), |acc, x| acc + x)
    }
}

impl<T> ValueRange<T>
where
    T: Add<Output = T> + Sub<Output = T> + Ord + Copy,
{
    /// Returns true iff all values of the range are <= max.
    /// This can be used to assert invariants
    #[inline]
    pub fn is_under_max(&self, max: T) -> bool {
        self.high <= max
    }

    /// Returns true iff all values of the range are >= min.
    /// This can be used to assert invariants
    #[inline]
    pub fn is_over_min(&self, min: T) -> bool {
        self.low >= min
    }

    /// This is to be called at the beginning of a transaction, to reserve the ability to commit (or rollback) an addition.
    /// If the last value is set, it enforces that the new maximum will remain under that limit.
    /// Usage: `range.prepare_add(20, None)?;` or `range.prepare_add(20, 100)?;`
    pub fn prepare_add(&mut self, value: T, max: impl Into<Option<T>>) -> Result<(), RangeError> {
        if let Some(max) = max.into() {
            if self.high + value > max {
                return Err(RangeError::Overflow);
            }
        }
        self.high = self.high + value;
        Ok(())
    }

    /// The caller should limit these to only previous `prepare_add` calls.
    /// We will panic on mistake as this should never happen
    pub fn rollback_add(&mut self, value: T) {
        self.high = self.high - value;
        self.assert_valid_range();
    }

    /// The caller should limit these to only previous `prepare_add` calls.
    /// We will panic on mistake as this should never happen
    pub fn commit_add(&mut self, value: T) {
        self.low = self.low + value;
        self.assert_valid_range();
    }

    /// This is a convenience method for non-transactional addition.
    /// Similar to `prepare_add`, if the last value is set it enforces that the new maximum
    /// will remain under that limit.
    /// Usage: `range.add(20, None)?;` or `range.add(20, 100)?;`
    pub fn add(&mut self, value: T, max: impl Into<Option<T>>) -> Result<(), RangeError> {
        self.prepare_add(value, max)?;
        self.commit_add(value);
        Ok(())
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
            if self.low < min + value {
                return Err(RangeError::Underflow);
            }
        }
        self.low = self.low - value;
        Ok(())
    }

    /// The caller should limit these to only previous `prepare_sub` calls.
    /// We will panic on mistake as this should never happen
    pub fn rollback_sub(&mut self, value: T) {
        self.low = self.low + value;
        self.assert_valid_range();
    }

    /// The caller should limit these to only previous `prepare_sub` calls.
    /// We will panic on mistake as this should never happen
    pub fn commit_sub(&mut self, value: T) {
        self.high = self.high - value;
        self.assert_valid_range();
    }

    /// This is a convenience method for non-transactional subtraction.
    /// Similar to `prepare_sub`, if the last value is set it enforces that the new minimum
    /// will remain above that limit.
    /// No minimum: `range.sub(20, None)?;`
    /// Minimum of 0 (for uints): `range.sub(20, 0)?;`
    /// Higher minimum :  `range.sub(20, 100)?;`
    pub fn sub(&mut self, value: T, min: impl Into<Option<T>>) -> Result<(), RangeError> {
        self.prepare_sub(value, min)?;
        self.commit_sub(value);
        Ok(())
    }

    #[inline]
    fn assert_valid_range(&self) {
        assert!(self.low <= self.high);
    }
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{Decimal, Uint128};

    use super::*;

    #[test]
    fn comparisons() {
        // check for one point - it behaves like an integer
        let mut range = ValueRange::new_val(50);
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
        let mut range = ValueRange::new_val(80);
        range.prepare_add(40, None).unwrap();

        // (100, 200)
        let mut other = ValueRange::new_val(200);
        other.prepare_sub(100, 0).unwrap();

        let total = range + other;
        assert_eq!(total, ValueRange::new(180, 320));
    }

    #[test]
    fn add_ranges_committed() {
        // (120, 160)
        let mut range = ValueRange::new(80, 120);
        // Avoid name clashing with std::ops::Add :-/
        ValueRange::add(&mut range, 40, None).unwrap();

        // (90, 190)
        let mut other = ValueRange::new(100, 200);
        other.sub(10, 0).unwrap();

        let total = range + other;
        assert_eq!(total, ValueRange::new(210, 350));
    }

    #[test]
    fn value() {
        // (80, 120)
        let range = ValueRange::new_val(80);
        let v = range.val().unwrap();
        assert_eq!(v, 80);

        let range: ValueRange<i32> = ValueRange::new(200, 200);
        let v = range.val().unwrap();
        assert_eq!(v, 200);

        let range: ValueRange<i32> = ValueRange::new(190, 200);
        let err = range.val().unwrap_err();
        assert_eq!(err, RangeError::NotOneValue);
    }

    #[test]
    fn sums() {
        let ranges = [
            ValueRange::new_val(100),
            ValueRange::new(0, 250),
            ValueRange::new_val(200),
            ValueRange::new(170, 380),
        ];
        let total: ValueRange<u32> = ranges.into_iter().sum();
        assert_eq!(total, ValueRange::new(470, 930));
    }

    #[test]
    fn min_max() {
        let ranges = [
            ValueRange::new_val(100),
            ValueRange::new(40, 250),
            ValueRange::new_val(200),
            ValueRange::new(170, 380),
        ];

        // (max value if all rollback, max value if all commit)
        let max = reduce_max_range(ranges.iter());
        assert_eq!(max, ValueRange::new(200, 380));

        // (min value if all rollback, min value if all commit)
        let min = reduce_min_range(ranges.iter());
        assert_eq!(min, ValueRange::new(40, 100));

        // (min value if all rollback, max value if all commit)
        let all = spread(ranges.iter());
        assert_eq!(all, ValueRange::new(40, 380));
    }

    // most tests will use i32 for simplicity - just ensure APIs work properly with Uint128
    #[test]
    fn works_with_uint128() {
        // (80, 120)
        let mut range = ValueRange::new_val(Uint128::new(80));
        range.prepare_add(Uint128::new(40), None).unwrap();

        // (100, 200)
        let mut other = ValueRange::new_val(Uint128::new(200));
        other
            .prepare_sub(Uint128::new(100), Uint128::zero())
            .unwrap();

        let total = range + other;
        assert_eq!(total, ValueRange::new(Uint128::new(180), Uint128::new(320)));
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
        let mut lien = ValueRange::new_val(0u64);

        // prepare some lien
        lien.prepare_add(2_000, collateral).unwrap();
        lien.prepare_add(5_000, collateral).unwrap();

        // cannot add too much
        let err = lien.prepare_add(3_500, collateral).unwrap_err();
        assert_eq!(err, RangeError::Overflow);

        // let's commit the second pending lien (only 2000 left)
        // QUESTION: should we enforce the min/max on commit/rollback explicitly and pass them in?
        lien.commit_add(5_000);
        assert_eq!(lien, ValueRange::new(5_000, 7_000));

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
        assert_eq!(lien, ValueRange::new(2_000, 5_000));
        lien.prepare_add(1_500, collateral).unwrap();
    }

    // idea here is to model the liens as in vault, and ensure we can calculate aggregates over them properly
    // we want to track max lien (which will be a range) and maximum slashable.
    #[test]
    fn invariants_over_set_of_liens() {
        // some existing outstanding liens
        let liens = vec![
            ValueRange::new(Uint128::new(5000), Uint128::new(7000)),
            ValueRange::new(Uint128::new(2000), Uint128::new(8000)),
            ValueRange::new(Uint128::new(3000), Uint128::new(12000)),
        ];
        // for simplicity, assume all slash rates are the same, easier for writing tests, but ensures operations are allowed
        let slash_rate = Decimal::percent(10);

        // the max lien is actually a range of (max if all rollback, max if all commit)
        let max_lien = reduce_max_range(liens.iter());
        assert_eq!(
            max_lien,
            ValueRange::new(Uint128::new(5000), Uint128::new(12000))
        );
        // check if this is less than some collateral
        assert!(max_lien.is_under_max(Uint128::new(15000)));
        assert!(max_lien.is_under_max(Uint128::new(12000)));
        assert!(!max_lien.is_under_max(Uint128::new(11900)));

        // max slashable is a sum of all liens * slash_rate
        let max_slashable: ValueRange<Uint128> = liens.iter().map(|l| l * slash_rate).sum();
        assert_eq!(
            max_slashable,
            ValueRange::new(Uint128::new(1000), Uint128::new(2700))
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
            ValueRange::new(Uint128::new(2000), Uint128::new(12000))
        );
    }
}

#[cfg(test)]
mod examples {
    use cosmwasm_std::{testing::MockStorage, Decimal, Order, StdError, Storage, Uint128};
    use cw_storage_plus::Map;

    use super::*;

    #[derive(Error, Debug, PartialEq)]
    pub enum TestsError {
        #[error("{0}")]
        Std(#[from] StdError),
        #[error("{0}")]
        Range(#[from] RangeError),
    }

    #[cw_serde]
    #[derive(Default)]
    pub struct UserInfo {
        // User collateral - this is set locally and never a range
        pub collateral: Uint128,
        // Highest user lien
        pub max_lien: ValueRange<Uint128>,
        // Total slashable amount for user
        pub total_slashable: ValueRange<Uint128>,
    }

    impl UserInfo {
        fn is_valid(&self) -> bool {
            self.max_lien.is_under_max(self.collateral)
                && self.total_slashable.is_under_max(self.collateral)
        }
    }

    #[cw_serde]
    pub struct Lien {
        /// Credit amount (denom is in `Config::denom`)
        pub amount: ValueRange<Uint128>,
        /// Slashable part - restricted to [0; 1] range
        pub slashable: Decimal,
    }

    const LIENS: Map<(&str, &str), Lien> = Map::new("liens");

    const USERS: Map<&str, UserInfo> = Map::new("users");

    // Recalculate user aggregate info from the liens (and the original user collateral)
    // Question: return (Vec<_>, Vec<_>) here instead and don't pass in collateral?
    fn collect_liens_for_user(
        storage: &dyn Storage,
        user: &str,
        collateral: impl Into<Uint128>,
        // if set, we don't include this lien in the sum (to use when reducing one)
        skip: Option<&str>,
    ) -> Result<UserInfo, TestsError> {
        // Create (amount, slashable) extract from all liens (possibly skipping one)
        let all_liens = LIENS
            .prefix(user)
            .range(storage, None, None, Order::Ascending)
            .filter(|r| match (skip, r) {
                (Some(x), Ok((k, _))) => x != k,
                _ => true,
            })
            .map(|r| r.map(|(_, lien)| (lien.amount, lien.amount * lien.slashable)));

        // collect into two vectors, which we can reduce / sum independently
        let (liens, slashable): (Vec<_>, Vec<_>) =
            itertools::process_results(all_liens, |iter| iter.unzip())?;
        let max_lien = liens.into_iter().reduce(max_range).unwrap_or_default();
        let total_slashable = slashable.into_iter().sum();
        Ok(UserInfo {
            collateral: collateral.into(),
            max_lien,
            total_slashable,
        })
    }

    #[test]
    fn map_methods_with_value_ranges() {
        let mut store = MockStorage::new();

        let alice = "Alice";
        let bob = "Bob";
        let carl = "Carl";
        let stake1 = "Stake1";
        let stake2 = "Stake2";

        let alice_collateral = Uint128::new(6000);
        let bob_collateral = Uint128::new(4000);
        let carl_collateral = Uint128::new(7000);

        // no inflight transactions for Alice
        LIENS
            .save(
                &mut store,
                (alice, stake1),
                &Lien {
                    amount: ValueRange::new(Uint128::new(5000), Uint128::new(5000)),
                    slashable: Decimal::percent(50),
                },
            )
            .unwrap();
        // one inflight transactions for Bob
        LIENS
            .save(
                &mut store,
                (bob, stake1),
                &Lien {
                    amount: ValueRange::new(Uint128::new(3000), Uint128::new(3000)),
                    slashable: Decimal::percent(50),
                },
            )
            .unwrap();
        LIENS
            .save(
                &mut store,
                (bob, stake2),
                &Lien {
                    amount: ValueRange::new(Uint128::new(0), Uint128::new(2000)),
                    slashable: Decimal::percent(80),
                },
            )
            .unwrap();
        // add a few liens with inflight transactions
        LIENS
            .save(
                &mut store,
                (carl, stake1),
                &Lien {
                    amount: ValueRange::new(Uint128::new(1000), Uint128::new(2000)),
                    slashable: Decimal::percent(50),
                },
            )
            .unwrap();
        LIENS
            .save(
                &mut store,
                (carl, stake2),
                &Lien {
                    amount: ValueRange::new(Uint128::new(5000), Uint128::new(6000)),
                    slashable: Decimal::percent(80),
                },
            )
            .unwrap();

        let mut alice_user = collect_liens_for_user(&store, alice, alice_collateral, None).unwrap();
        assert!(alice_user.is_valid());
        USERS.save(&mut store, alice, &alice_user).unwrap();

        let bob_user = collect_liens_for_user(&store, bob, bob_collateral, None).unwrap();
        assert!(bob_user.is_valid());
        USERS.save(&mut store, bob, &bob_user).unwrap();

        let mut carl_user = collect_liens_for_user(&store, carl, carl_collateral, None).unwrap();
        assert!(carl_user.is_valid());
        USERS.save(&mut store, carl, &carl_user).unwrap();

        // This shows how to check without storing

        // let's make an invalid change, which may go below min...
        // Bob tried to withdraw on the inflight lien (0, 2000)
        let mut lien = LIENS.load(&store, (bob, stake2)).unwrap();
        let err = lien
            .amount
            .prepare_sub(Uint128::new(1000), Uint128::zero())
            .unwrap_err();
        assert_eq!(err, RangeError::Underflow);

        // let's make an invalid change, which may go above max...
        // adding 3000 to stake1 will not pass collateral or max_lien check, but max_slashable
        let mut lien = LIENS.load(&store, (carl, stake1)).unwrap();
        // pass the local check
        lien.amount
            .prepare_add(Uint128::new(3000), carl_collateral)
            .unwrap();
        // safely update max
        carl_user.max_lien = max_range(carl_user.max_lien, lien.amount);
        assert!(carl_user.is_valid());
        // now, let's modify the slashable part
        let err = carl_user
            .total_slashable
            .prepare_add(Uint128::new(3000) * lien.slashable, carl_collateral)
            .unwrap_err();
        assert_eq!(err, RangeError::Overflow);

        // let's make a valid change...
        // alice makes another 1000 to hit her cap
        let mut lien = LIENS.load(&store, (alice, stake1)).unwrap();
        lien.amount
            .prepare_add(Uint128::new(1000), alice_collateral)
            .unwrap();
        alice_user.max_lien = max_range(alice_user.max_lien, lien.amount);
        alice_user
            .total_slashable
            .prepare_add(Uint128::new(1000) * lien.slashable, alice_collateral)
            .unwrap();
        assert!(alice_user.is_valid());
        LIENS.save(&mut store, (alice, stake1), &lien).unwrap();
        USERS.save(&mut store, alice, &alice_user).unwrap();
        // verify this matches the full calculation
        let alice_user2 =
            collect_liens_for_user(&store, alice, alice_user.collateral, None).unwrap();
        assert_eq!(alice_user, alice_user2);

        // but 500 more is too much
        let mut lien = LIENS.load(&store, (alice, stake1)).unwrap();
        lien.amount
            .prepare_add(Uint128::new(500), alice_collateral)
            .unwrap_err();
        assert_eq!(err, RangeError::Overflow);

        // bob doing a valid unstake
        let mut lien = LIENS.load(&store, (bob, stake1)).unwrap();
        lien.amount
            .prepare_sub(Uint128::new(2000), Uint128::zero())
            .unwrap();
        // this requires a bit more tricky version to get max_lien
        let mut bob_user =
            collect_liens_for_user(&store, bob, bob_user.collateral, Some(stake1)).unwrap();
        // and add this lien fully
        bob_user.total_slashable = bob_user.total_slashable + (lien.amount * lien.slashable);
        bob_user.max_lien = max_range(bob_user.max_lien, lien.amount);
        // and finally, check validity
        assert!(bob_user.is_valid());
        // it is, so we save
        LIENS.save(&mut store, (bob, stake1), &lien).unwrap();
        USERS.save(&mut store, bob, &bob_user).unwrap();
        // verify this matches the full calculation
        let bob_user2 = collect_liens_for_user(&store, bob, bob_user.collateral, None).unwrap();
        assert_eq!(bob_user, bob_user2);
    }
}
