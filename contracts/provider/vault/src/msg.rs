use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, Uint128};

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
pub struct AccountResponse {
    // Everything is denom, changing all Uint128 to coin with the same denom seems very inefficient
    pub denom: String,
    pub bonded: Uint128,
    pub free: Uint128,
}

#[cw_serde]
pub struct AllAccountsResponse {
    pub accounts: Vec<AllAccountsResponseItem>,
}

#[cw_serde]
pub struct AllAccountsResponseItem {
    pub account: String,
    // Everything is denom, changing all Uint128 to coin with the same denom seems very inefficient
    pub denom: String,
    pub bonded: Uint128,
    pub free: Uint128,
}

#[cw_serde]
pub struct AccountClaimsResponse {
    pub claims: Vec<LienInfo>,
}

#[cw_serde]
pub struct LienInfo {
    pub lienholder: String,
    pub amount: Uint128,
}

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,
    pub local_staking: String,
}

pub type AllTxsResponseItem = crate::txs::Tx;

#[cw_serde]
pub struct AllTxsResponse {
    pub txs: Vec<AllTxsResponseItem>,
}
