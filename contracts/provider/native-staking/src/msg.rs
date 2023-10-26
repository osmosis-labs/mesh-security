use crate::state::Config;
use cosmwasm_schema::cw_serde;

pub type ConfigResponse = Config;

#[cw_serde]
pub struct ProxyByOwnerResponse {
    pub proxy: String,
}

#[cw_serde]
pub struct OwnerByProxyResponse {
    pub owner: String,
}

/// The message that is binary encoded in `receive_stake(..msg)`
#[cw_serde]
pub struct StakeMsg {
    pub validator: String,
}

/// Message to be sent as `msg` field on `burn_stake(..msg)`
/// If `validator` is set, undelegate preferentially from it first.
/// If it is not set, undelegate evenly from all validators the user has stake in.
#[cw_serde]
pub struct BurnMsg {
    pub validator: Option<String>,
}
