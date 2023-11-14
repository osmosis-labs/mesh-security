use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, Decimal};

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The code id for the `native-staking-proxy` contracts we will be managing
    pub proxy_code_id: u64,

    /// The address of the vault contract (where we get and return stake)
    pub vault: Addr,

    /// The slash ratio for double signing
    pub slash_ratio_dsign: Decimal,

    /// The slash ratio for being offline
    pub slash_ratio_offline: Decimal,
}
