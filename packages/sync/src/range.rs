use std::ops::{Add, Sub};
use thiserror::Error;

use cosmwasm_schema::cw_serde;

/// This is designed to work with two numeric primitives that can be added, subtracted, and compared.
#[cw_serde]
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

impl<T> ValueRange<T>
where
    T: Add<Output = T> + Sub<Output = T> + PartialOrd + Ord + Copy,
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
        if value < self.0 {
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

#[cfg(test)]
mod tests {
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
}
