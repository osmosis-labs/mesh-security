use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};
use thiserror::Error;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, Binary, Coin, Decimal, Response, StdError};
use cw_storage_plus::Item;
use cw_utils::{nonpayable, PaymentError};

use mesh_apis::cross_staking_api::{self, CrossStakingApi, MaxSlashResponse};
use mesh_apis::vault_api::VaultApiHelper;

#[derive(Error, Debug)]
pub enum StakingError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Staking must be in this denom: {0}")]
    WrongDenom(String),
}

#[cw_serde]
pub struct Config {
    /// The address of the vault contract (where we get and return stake)
    pub vault: VaultApiHelper,

    pub max_slash: Decimal,

    pub stake_denom: String,
}

pub struct MockCrossStakingContract<'a> {
    config: Item<'a, Config>,
}

#[contract]
#[messages(cross_staking_api as CrossStakingApi)]
#[error(StakingError)]
impl MockCrossStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    /// Anyone can create a cross-staking contract. It must know who the vault is
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        max_slash: Decimal,
        vault: String,
        stake_denom: String,
    ) -> Result<Response, StakingError> {
        let config = Config {
            vault: VaultApiHelper(ctx.deps.api.addr_validate(&vault)?),
            max_slash,
            stake_denom,
        };
        self.config.save(ctx.deps.storage, &config)?;
        Ok(Response::new())
    }

    #[msg(exec)]
    fn release_stake(&self, ctx: ExecCtx, amount: Coin) -> Result<Response, StakingError> {
        nonpayable(&ctx.info)?;

        // assert proper denom
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(
            cfg.stake_denom,
            amount.denom,
            StakingError::WrongDenom(cfg.stake_denom)
        );

        // blindly reduce lien on vault
        let wasm = cfg
            .vault
            .release_cross_stake(ctx.info.sender.into_string(), amount, vec![])?;
        Ok(Response::new().add_message(wasm))
    }
}

#[contract]
#[messages(cross_staking_api as CrossStakingApi)]
impl CrossStakingApi for MockCrossStakingContract<'_> {
    type Error = StakingError;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_virtual_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Coin,
        tx_id: u64,
        msg: Binary,
    ) -> Result<Response, Self::Error> {
        nonpayable(&ctx.info)?;

        // only can be called by the vault
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(
            cfg.vault.addr(),
            &ctx.info.sender,
            StakingError::Unauthorized {}
        );
        // assert proper denom
        ensure_eq!(
            cfg.stake_denom,
            amount.denom,
            StakingError::WrongDenom(cfg.stake_denom)
        );

        // ignore args
        let _ = (owner, msg, amount, tx_id);
        Ok(Response::new())
    }

    /// Returns the maximum percentage that can be slashed (hardcoded on instantiate)
    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error> {
        let Config { max_slash, .. } = self.config.load(ctx.deps.storage)?;
        Ok(MaxSlashResponse { max_slash })
    }
}
