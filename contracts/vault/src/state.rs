use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, Uint128};
use mesh_apis::local_staking_api::LocalStakingApiHelper;

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking (only native tokens)
    pub denom: String,

    /// info about the local staking contract (where actual tokens go)
    pub local_staking: LocalStaking,
}

#[cw_serde]
pub struct LocalStaking {
    /// Local staking address
    pub contract: LocalStakingApiHelper,

    /// Max slashing on local staking
    pub max_slash: Decimal,
}

/// Single Lien description
#[cw_serde]
pub struct Lien {
    /// Credit amount (denom is in `Config::denom`)
    pub amount: Uint128,
    /// Slashable part - restricted to [0; 1] range
    pub slashable: Decimal,
}

#[cw_serde]
#[derive(Default)]
pub struct UserInfo {
    // Highes user lien
    pub max_lien: Uint128,
    // Tatal slashable amount for user
    pub total_slashable: Uint128,
}

impl UserInfo {
    // Return total used collateral
    pub fn used_collateral(&self) -> Uint128 {
        self.max_lien.max(self.total_slashable)
    }
}
