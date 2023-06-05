use std::cmp::Ordering;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Uint128, Uint256};

/// Points alignment is how much points should be added / subtracted from points calculated per
/// user due to stake changes. It has to be signed type, but no signed integrals are right now
/// in CosmWasm - using `Uint256` here as a "fake" type, so for calculations it is shifted - the
/// real value stored is `points_alignment - Uint256::MAX / 2` - this is not ideal, but it makes
/// calculations always fit in U256.
#[cw_serde]
#[derive(Copy)]
pub struct PointsAlignment(Uint256);

impl PointsAlignment {
    pub fn new() -> Self {
        Self(Uint256::MAX >> 1)
    }

    /// Align points with alignment
    pub fn align(self, points: Uint256) -> Uint256 {
        match self.0.cmp(&(Uint256::MAX >> 1)) {
            // Points aligment negative - first we need to add alignment and then add offset
            // to avoid exceeding limit
            Ordering::Less => points + self.0 - (Uint256::MAX >> 1),
            // Points alignment is positive - first we reduce it by offset and then add to the
            // poits
            Ordering::Greater => points + (self.0 - (Uint256::MAX >> 1)),
            // Alignment is `0`, no math to be done
            Ordering::Equal => points,
        }
    }

    /// Modify points alignment due to increased stake - increasing weight immediately "adds" points
    /// distributed to owner, so they need to be reduced
    ///
    /// * amount - amouont just staken
    /// * pps - points per stake right now
    pub fn stake_increased(&mut self, amount: Uint128, pps: Uint256) {
        self.0 -= Uint256::from(amount) * pps;
    }

    /// Modify points alignment due to decreased stake - increasing weight immediately "removes" points
    /// distributed to owner, so they need to be increased
    ///
    /// * amount - amouont just staken
    /// * pps - points per stake right now
    pub fn stake_decreased(&mut self, amount: Uint128, pps: Uint256) {
        self.0 += Uint256::from(amount) * pps;
    }
}

impl Default for PointsAlignment {
    fn default() -> Self {
        Self::new()
    }
}
