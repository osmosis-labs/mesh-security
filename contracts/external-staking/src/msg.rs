use cosmwasm_schema::cw_serde;
use cosmwasm_std::Uint128;

use crate::state::{Config, PendingUnbond};

/// Config information returned with query
#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,
    pub vault: String,
    /// In seconds
    pub unbonding_period: u64,
}

impl From<Config> for ConfigResponse {
    fn from(value: Config) -> Self {
        Self {
            denom: value.denom,
            vault: value.vault.0.into(),
            unbonding_period: value.unbonding_period,
        }
    }
}

/// User-related information including user address
#[cw_serde]
pub struct UserInfo {
    pub addr: String,
    pub stake: Uint128,
    pub pending_unbonds: Vec<PendingUnbond>,
}

/// Aggregated mutiple users response
#[cw_serde]
pub struct UsersResponse {
    pub users: Vec<UserInfo>,
}
