use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Coin, CosmosMsg, CustomMsg, Uint128};

/// A top-level Custom message for the token factory.
/// It is embedded like this to easily allow adding other variants that are custom
/// to your chain, or other "standardized" extensions along side it.
#[cw_serde]
pub enum VirtualStakeCustomMsg {
    VirtualStake(VirtualStakeMsg),
}

/// Special messages to be supported by any chain that supports token_factory
#[cw_serde]
pub enum VirtualStakeMsg {
    /// Bond will enforce the calling contract has a max cap established.
    /// It ensures amount.denom is the native staking denom,
    /// and that (currently minted + amount.amount <= max_cap)
    ///
    /// If these conditions are met, it will mint amount.amount tokens
    /// to the caller's account and delegate them to the named validator.
    /// It will also update the currently minted amount.
    Bond { amount: Coin, validator: String },
    /// Unbond ensures that amount.denom is the native staking denom,
    /// that caller is able to mint and (currently minted >= amount.amount).
    /// It also checks that the caller has at least amoubt.amount tokens
    /// currently bonded to the named validator.
    ///
    /// If these conditions are met, it will instantly undelegate
    /// amount.amount tokens from the caller to the named validator.
    /// It will then burn those tokens from the caller's account,
    /// and update the currently minted amount.
    Unbond { amount: Coin, validator: String },
}

impl VirtualStakeMsg {
    pub fn bond(denom: &str, amount: impl Into<Uint128>, validator: &str) -> VirtualStakeMsg {
        let coin = Coin {
            amount: amount.into(),
            denom: denom.into(),
        };
        VirtualStakeMsg::Bond {
            amount: coin,
            validator: validator.to_string(),
        }
    }

    pub fn unbond(denom: &str, amount: impl Into<Uint128>, validator: &str) -> VirtualStakeMsg {
        let coin = Coin {
            amount: amount.into(),
            denom: denom.into(),
        };
        VirtualStakeMsg::Unbond {
            amount: coin,
            validator: validator.to_string(),
        }
    }
}

impl From<VirtualStakeMsg> for CosmosMsg<VirtualStakeCustomMsg> {
    fn from(msg: VirtualStakeMsg) -> CosmosMsg<VirtualStakeCustomMsg> {
        CosmosMsg::Custom(VirtualStakeCustomMsg::VirtualStake(msg))
    }
}

impl CustomMsg for VirtualStakeCustomMsg {}

/// A top-level Custom message for the meshsecurityprovider module.
/// It is embedded like this to easily allow adding other variants that are custom
/// to your chain, or other "standardized" extensions along side it.
#[cw_serde]
pub enum ProviderCustomMsg {
    Provider(ProviderMsg),
}

/// Special messages to be supported by any chain that supports meshsecurityprovider
#[cw_serde]
pub enum ProviderMsg {
    /// Bond will enforce the calling contract is the vault contract.
    /// It ensures amount.denom is the native staking denom.
    ///
    /// If these conditions are met, it will bond amount.amount tokens
    /// to the vault.
    Bond { delegator: String, amount: Coin },
    /// Unbond ensures that amount.denom is the native staking denom and
    /// the calling contract is the vault contract.
    ///
    /// If these conditions are met, it will instantly unbond
    /// amount.amount tokens from the vault contract.
    Unbond { delegator: String, amount: Coin },
    /// Unstake ensures that amount.denom is the native staking denom and
    /// the calling contract is the native staking proxy contract.
    ///
    /// If these conditions are met, it will instantly unstake
    /// amount.amount tokens from the native staking proxy contract.
    Unstake { validator: String, amount: Coin },
}

impl ProviderMsg {
    pub fn bond(denom: &str, delegator: &str, amount: impl Into<Uint128>) -> ProviderMsg {
        let coin = Coin {
            amount: amount.into(),
            denom: denom.into(),
        };
        ProviderMsg::Bond {
            delegator: delegator.to_string(),
            amount: coin,
        }
    }

    pub fn unbond(denom: &str, delegator: &str, amount: impl Into<Uint128>) -> ProviderMsg {
        let coin = Coin {
            amount: amount.into(),
            denom: denom.into(),
        };
        ProviderMsg::Unbond {
            delegator: delegator.to_string(),
            amount: coin,
        }
    }
    
    pub fn unstake(denom: &str, validator: &str, amount: impl Into<Uint128>) -> ProviderMsg {
        let coin = Coin {
            amount: amount.into(),
            denom: denom.into(),
        };
        ProviderMsg::Unstake {
            validator: validator.to_string(),
            amount: coin,
        }
    }
}

impl From<ProviderMsg> for CosmosMsg<ProviderCustomMsg> {
    fn from(msg: ProviderMsg) -> CosmosMsg<ProviderCustomMsg> {
        CosmosMsg::Custom(ProviderCustomMsg::Provider(msg))
    }
}

impl CustomMsg for ProviderCustomMsg {}
