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
    // TODO:
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
        }
    }
}
