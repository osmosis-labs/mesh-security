use cosmwasm_schema::cw_serde;
use cosmwasm_std::Decimal;
use mesh_apis::vault_api::VaultApiHelper;

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The code id for the `native-staking-proxy` contracts we will be managing
    pub proxy_code_id: u64,

    /// The address of the vault contract (where we get and return stake)
    pub vault: VaultApiHelper,

    /// Max slash percentage (from InstantiateMsg, maybe later from the chain)
    pub max_slashing: Decimal,
}
