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
    InFlightRemoteStaking { // IBC flight
        /// Transaction id
        id: u64,
        /// Associated amount
        amount: Uint128,
        /// Associated owner
        user: Addr,
        /// Remote validator
        validator: String,
    },
    InFlightRemoteUnstaking { // IBC flight
        /// Transaction id
        id: u64,
        /// Associated amount
        amount: Uint128,
        /// Associated owner
        user: Addr,
        /// Remote validator
        validator: String,
    }, // TODO
       // InFlightSlashing
}

// Implement display for Tx
impl std::fmt::Display for Tx {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Tx::InFlightStaking {
                id,
                amount,
                slashable,
                user,
                lienholder,
            } => {
                write!(f, "InFlightStaking {{ id: {}, amount: {}, slashable: {}, user: {}, lienholder: {} }}", id, amount, slashable, user, lienholder)
            }
            Tx::InFlightRemoteStaking {
                id,
                amount,
                user,
                validator,
            } => {
                write!(
                    f,
                    "InFlightRemoteStaking {{ id: {}, amount: {}, user: {}, validator: {} }}",
                    id, amount, user, validator
                )
            }
            Tx::InFlightRemoteUnstaking {
                id,
                amount,
                user,
                validator,
            } => {
                write!(
                    f,
                    "InFlightRemoteUnstaking {{ id: {}, amount: {}, user: {}, validator: {} }}",
                    id, amount, user, validator
                )
            }
        }
    }
}
