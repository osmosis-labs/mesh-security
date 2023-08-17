use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal, Uint128};
use std::fmt::Formatter;

#[cw_serde]
pub enum Tx {
    InFlightStaking {
        /// Transaction id
        id: u64,
        /// Associated amount
        amount: Uint128,
        /// Slashable portion of lien
        slashable: Decimal,
        /// Associated user
        user: Addr,
        /// Remote staking contract
        lienholder: Addr,
    },
    InFlightRemoteStaking {
        /// Transaction id
        id: u64,
        /// Associated amount
        amount: Uint128,
        /// Associated owner
        user: Addr,
        /// Remote validator
        validator: String,
    },
    InFlightRemoteUnstaking {
        /// Transaction id
        id: u64,
        /// Associated amount
        amount: Uint128,
        /// Associated owner
        user: Addr,
        /// Remote validator
        validator: String,
    },
    /// This is stored on the provider side when releasing funds
    InFlightTransferFunds {
        id: u64,
        /// Amount of rewards being withdrawn
        amount: Uint128,
        /// The staker sending the funds
        staker: Addr,
        /// The validator whose rewards they come from (to revert)
        validator: String,
    },
}

impl Tx {
    pub fn id(&self) -> u64 {
        match self {
            Tx::InFlightStaking { id, .. } => *id,
            Tx::InFlightRemoteStaking { id, .. } => *id,
            Tx::InFlightRemoteUnstaking { id, .. } => *id,
            Tx::InFlightTransferFunds { id, .. } => *id,
        }
    }
}

// Use Debug output for Display as well
impl std::fmt::Display for Tx {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
