use crate::state::Config;
use cosmwasm_schema::cw_serde;

pub type ConfigResponse = Config;

/// The message that is binary encoded in a proxy contract's `Instantiate` message's data
#[cw_serde]
pub struct OwnerMsg {
    pub owner: String,
}
