use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};
use thiserror::Error;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, Binary, Coin, Decimal, Response, StdError, Uint128};
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
}

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the vault contract (where we get and return stake)
    pub vault: VaultApiHelper,

    pub max_slash: Decimal,
}

pub struct MockLocalStakingContract<'a> {
    config: Item<'a, Config>,
}

#[contract(error=StakingError)]
#[messages(local_staking_api as LocalStakingApi)]
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
        denom: String,
        max_slash: Decimal,
    ) -> Result<Response, StakingError> {
        let config = Config {
            denom,
            vault: VaultApiHelper(ctx.info.sender),
            max_slash,
        };
        self.config.save(ctx.deps.storage, &config)?;
        Ok(Response::new())
    }

    #[msg(exec)]
    fn release_stake(&self, ctx: ExecCtx, amount: Uint128) -> Result<Response, StakingError> {
        nonpayable(&ctx.info)?;

        // blindly send money back to vault
        let cfg = self.config.load(ctx.deps.storage)?;
        let funds = Coin {
            denom: cfg.denom,
            amount,
        };
        let wasm = cfg
            .vault
            .release_local_stake(ctx.info.sender.into_string(), vec![funds])?;
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
        let _paid = must_pay(&ctx.info, &cfg.denom)?;

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
