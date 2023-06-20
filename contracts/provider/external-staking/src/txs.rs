use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};

#[cw_serde]
pub enum TxType {
    InFlightRemoteStaking,
    // TODO
    // Slash,
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
