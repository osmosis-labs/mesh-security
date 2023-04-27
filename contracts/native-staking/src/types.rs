use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

/*** state ***/

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    // FIXME: support cw20 as well later
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub vault: Option<Addr>,
}

/**** api ****/

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub vault: Option<String>,
}

#[cw_serde]
pub struct ClaimsResponse {
    // TODO
}

/// This is the message that is binary encoded in receive_stake(..msg)
#[cw_serde]
pub struct StakeMsg {
    validator: String,
}
