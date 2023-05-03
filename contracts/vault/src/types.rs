use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Binary, Uint128};

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
    pub claims: Vec<LienAddr>,
}

#[cw_serde]
pub struct LienAddr {
    pub lienholder: Addr,
    pub amount: Uint128,
}

/**** api ****/

/// This is the info used to construct the native staking contract
#[cw_serde]
pub struct StakingInitInfo {
    /// Admin for the local staking contract. If empty, it is immutable
    pub admin: Option<String>,
    /// Code id used to instantiate the local staking contract
    pub code_id: u64,
    /// JSON-encoded local staking `InstantiateMsg` struct (as raw `Binary`)
    pub msg: Binary,
    /// A human-readable label for the local staking contract (will use a default if not provided)
    pub label: Option<String>,
}

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub local_staking: String,
}

#[cw_serde]
pub struct AccountResponse {
    pub bonded: Uint128,
    pub free: Uint128,
    pub claims: Vec<Lien>,
}

#[cw_serde]
pub struct Lien {
    pub lienholder: String,
    pub amount: Uint128,
}
