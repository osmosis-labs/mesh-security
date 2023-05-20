use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};
use thiserror::Error;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, Binary, Coin, Decimal, Response, StdError};
use cw_storage_plus::Item;
use cw_utils::{must_pay, nonpayable, PaymentError};

use mesh_apis::local_staking_api::{self, LocalStakingApi, MaxSlashResponse};
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
    /// The denom we accept for staking
    pub stake_denom: String,

    /// The address of the vault contract (where we get and return stake)
    pub vault: VaultApiHelper,

    pub max_slash: Decimal,
}

pub struct MockLocalStakingContract<'a> {
    config: Item<'a, Config>,
}

#[contract]
#[messages(local_staking_api as LocalStakingApi)]
#[error(StakingError)]
impl MockLocalStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    /// The caller of the instantiation will be the vault contract
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        stake_denom: String,
        max_slash: Decimal,
    ) -> Result<Response, StakingError> {
        let config = Config {
            stake_denom,
            vault: VaultApiHelper(ctx.info.sender),
            max_slash,
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

        // blindly send money back to vault
        let wasm = cfg
            .vault
            .release_local_stake(ctx.info.sender.into_string(), vec![amount])?;
        Ok(Response::new().add_message(wasm))
    }
}

#[contract]
impl LocalStakingApi for MockLocalStakingContract<'_> {
    type Error = StakingError;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        msg: Binary,
    ) -> Result<Response, Self::Error> {
        // only can be called by the vault
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(
            cfg.vault.addr(),
            &ctx.info.sender,
            StakingError::Unauthorized {}
        );

        // assert funds passed in
        let _paid = must_pay(&ctx.info, &cfg.stake_denom)?;

        // ignore args
        let _ = (owner, msg);
        Ok(Response::new())
    }

    /// Returns the maximum percentage that can be slashed (hardcoded on instantiate)
    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error> {
        let Config { max_slash, .. } = self.config.load(ctx.deps.storage)?;
        Ok(MaxSlashResponse { max_slash })
    }
}
