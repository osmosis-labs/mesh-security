use cosmwasm_schema::cw_serde;
use cosmwasm_std::Uint128;
use cw_utils::Duration;

use crate::state::{Config, PendingUnbound};

/// Config information returned with query
#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,
    pub vault: String,
    pub unbounding_period: Duration,
}

impl From<Config> for ConfigResponse {
    fn from(value: Config) -> Self {
        Self {
            denom: value.denom,
            vault: value.vault.0.into(),
            unbounding_period: value.unbounding_period,
        }
    }
}

/// User-related information including user address
#[cw_serde]
pub struct UserInfo {
    pub addr: String,
    pub stake: Uint128,
    pub pending_unbounds: Vec<PendingUnbound>,
    pub released: Uint128,
}

/// Aggregated mutiple users response
#[cw_serde]
pub struct UsersResponse {
    pub users: Vec<UserInfo>,
}
