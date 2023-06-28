use cosmwasm_schema::cw_serde;
use cosmwasm_std::{coin, Coin, IbcChannel, Uint128};

use crate::msg::MaybePendingRewards::{Locked, Rewards};
use crate::{error::ContractError, state::Config};

#[cw_serde]
pub struct AuthorizedEndpoint {
    pub connection_id: String,
    pub port_id: String,
}

impl AuthorizedEndpoint {
    pub fn new(connection_id: &str, port_id: &str) -> Self {
        Self {
            connection_id: connection_id.into(),
            port_id: port_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        // FIXME: can we add more checks here? is this formally defined in some ibc spec?
        if self.connection_id.is_empty() || self.port_id.is_empty() {
            return Err(ContractError::InvalidEndpoint(format!("{:?}", self)));
        }
        Ok(())
    }
}

pub type AuthorizedEndpointResponse = AuthorizedEndpoint;

#[cw_serde]
pub struct IbcChannelResponse {
    pub channel: IbcChannel,
}

#[cw_serde]
pub struct ListRemoteValidatorsResponse {
    pub validators: Vec<String>,
}

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

/// Stake-related information including user address and validator
#[cw_serde]
pub struct StakeInfo {
    pub owner: String,
    pub validator: String,
    pub stake: Uint128,
}

/// Aggregated mutiple stakes response
#[cw_serde]
pub struct StakesResponse {
    pub stakes: Vec<StakeInfo>,
}

/// Message to be send as `msg` field on `receive_virtual_staking`
#[cw_serde]
pub struct ReceiveVirtualStake {
    pub validator: String,
}

/// User-related information including user address
#[cw_serde]
pub struct UserInfo {
    pub addr: String,
}

/// Aggregated multiple users response
#[cw_serde]
pub struct UsersResponse {
    pub users: Vec<UserInfo>,
}

/// Response for pending rewards query on one validator
#[cw_serde]
pub enum MaybePendingRewards {
    Rewards(Coin),
    Locked {},
}

impl MaybePendingRewards {
    /// Designed for test code, unwrap or panic if Locked
    pub fn unwrap(self) -> Coin {
        match self {
            Rewards(coin) => coin,
            Locked {} => panic!("Pending rewards are locked"),
        }
    }
}
/// Response for pending rewards query on all validator
#[cw_serde]
pub struct AllPendingRewards {
    pub rewards: Vec<ValidatorPendingRewards>,
}

#[cw_serde]
pub struct ValidatorPendingRewards {
    pub validator: String,
    pub rewards: MaybePendingRewards,
}

impl ValidatorPendingRewards {
    pub fn new(validator: impl Into<String>, amount: u128, denom: impl Into<String>) -> Self {
        Self {
            validator: validator.into(),
            rewards: Rewards(coin(amount, denom)),
        }
    }
}

pub type TxResponse = mesh_sync::Tx;

#[cw_serde]
pub struct AllTxsResponse {
    pub txs: Vec<TxResponse>,
}
