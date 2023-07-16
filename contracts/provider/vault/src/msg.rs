use crate::msg::MaybeAccountResponse::{Account, Locked};
use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Binary, Uint128};
use mesh_sync::Tx;

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
pub enum MaybeAccountResponse {
    Account(AccountResponse),
    Locked {},
}

impl MaybeAccountResponse {
    pub fn new_unlocked(denom: &str, bonded: Uint128, free: Uint128) -> Self {
        Account(AccountResponse {
            denom: denom.to_owned(),
            bonded,
            free,
        })
    }
    /// Designed for test code, unwrap or panic if Locked
    pub fn unwrap(self) -> AccountResponse {
        match self {
            Account(acct) => acct,
            Locked {} => panic!("Account is locked"),
        }
    }
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
    pub user: String,
    pub account: MaybeAccountResponse,
}

#[cw_serde]
pub struct AccountClaimsResponse {
    pub claims: Vec<MaybeLienResponse>,
}

#[cw_serde]
pub enum MaybeLienResponse {
    Lien { lienholder: String, amount: Uint128 },
    Locked {},
}

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,
    pub local_staking: String,
}

pub type TxResponse = Tx;
pub type AllTxsResponseItem = TxResponse;

#[cw_serde]
pub struct AllTxsResponse {
    pub txs: Vec<AllTxsResponseItem>,
}
