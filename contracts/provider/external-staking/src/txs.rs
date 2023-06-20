use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};

#[cw_serde]
pub enum TxType {
    InFlightRemoteStaking,
    InFlightRemoteUnstaking,
    // TODO
    // Slash,
}

impl std::fmt::Display for TxType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxType::InFlightRemoteStaking => write!(f, "RemoteStaking"),
            TxType::InFlightRemoteUnstaking => write!(f, "RemoteUnstaking"),
        }
    }
}

#[cw_serde]
pub struct Tx {
    /// Transaction id
    pub id: u64,
    /// Transaction type
    pub ty: TxType,
    /// Associated amount
    pub amount: Uint128,
    /// Associated owner
    pub user: Addr,
    /// Remote validator
    pub validator: String,
}
