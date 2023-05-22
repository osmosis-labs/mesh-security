use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary, Uint128};

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking (only native tokens)
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub local_staking: Addr,
}

/// All values are in Config.denom
#[cw_serde]
pub struct Balance {
    pub bonded: Uint128,
    pub claims: Vec<LienAddr>,
}

#[cw_serde]
pub struct LienAddr {
    pub lienholder: Addr,
    pub amount: Uint128,
}
