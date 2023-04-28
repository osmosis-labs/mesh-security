use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Uint128};

/*** state ***/

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking (only native tokens)
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub local_staking: Addr,
}

#[cw_serde]
pub struct Balance {
    pub bonded: Uint128,
    pub claims: Vec<LeinAddr>,
}

#[cw_serde]
pub struct LeinAddr {
    pub leinholder: Addr,
    pub amount: Uint128,
}

/**** api ****/

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub local_staking: String,
}

#[cw_serde]
pub struct BalanceResponse {
    pub bonded: Uint128,
    pub free: Uint128,
    pub claims: Vec<Lein>,
}

#[cw_serde]
pub struct Lein {
    pub leinholder: String,
    pub amount: Uint128,
}
