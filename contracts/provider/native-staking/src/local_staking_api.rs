use cosmwasm_std::{ensure_eq, from_slice, to_binary, Binary, Coin, Response, SubMsg, WasmMsg};
use cw_utils::{must_pay, nonpayable};
use sylvia::types::QueryCtx;
use sylvia::{contract, types::ExecCtx};

#[allow(unused_imports)]
use mesh_apis::local_staking_api::{self, LocalStakingApi, SlashRatioResponse};

use crate::contract::{NativeStakingContract, REPLY_ID_INSTANTIATE};
use crate::error::ContractError;
use crate::msg::StakeMsg;

// FIXME: Move to sylvia contract macro
use crate::contract::BoundQuerier;
use crate::state::Config;

#[contract]
#[messages(local_staking_api as LocalStakingApi)]
impl LocalStakingApi for NativeStakingContract<'_> {
    type Error = ContractError;

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
        // Can only be called by the vault
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.vault, ctx.info.sender, ContractError::Unauthorized {});

        // Assert funds are passed in
        let _paid = must_pay(&ctx.info, &cfg.denom)?;

        // Parse message to find validator to stake on
        let StakeMsg { validator } = from_slice(&msg)?;

        let owner_addr = ctx.deps.api.addr_validate(&owner)?;

        // Add it to the delegators map
        self.delegators
            .save(ctx.deps.storage, (&validator, &owner_addr), &true)?;

        // Look up if there is a proxy to match. Instantiate or call stake on existing
        match self
            .proxy_by_owner
            .may_load(ctx.deps.storage, &owner_addr)?
        {
            None => {
                // Instantiate proxy contract and send funds to stake, with reply handling on success
                let msg = to_binary(&mesh_native_staking_proxy::contract::InstantiateMsg {
                    denom: cfg.denom,
                    owner: owner.clone(),
                    validator,
                })?;
                let wasm_msg = WasmMsg::Instantiate {
                    admin: Some(ctx.env.contract.address.into()),
                    code_id: cfg.proxy_code_id,
                    msg,
                    funds: ctx.info.funds,
                    label: format!("LSP for {owner}"),
                };
                let sub_msg = SubMsg::reply_on_success(wasm_msg, REPLY_ID_INSTANTIATE);
                Ok(Response::new().add_submessage(sub_msg))
            }
            Some(proxy_addr) => {
                // Send stake message with funds to the proxy contract
                let msg =
                    to_binary(&mesh_native_staking_proxy::contract::ExecMsg::Stake { validator })?;
                let wasm_msg = WasmMsg::Execute {
                    contract_addr: proxy_addr.into(),
                    msg,
                    funds: ctx.info.funds,
                };
                Ok(Response::new().add_message(wasm_msg))
            }
        }
    }

    /// Burns stake. This is called when the user's collateral is slashed and, as part of slashing
    /// propagation, the native staking contract needs to burn / discount the indicated slashing amount.
    /// If `validator` is set, undelegate preferentially from it first.
    /// If it is not set, undelegate evenly from all validators the user has stake in.
    #[msg(exec)]
    fn burn_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Coin,
        validator: Option<String>,
    ) -> Result<Response, Self::Error> {
        // Can only be called by the vault
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.vault, ctx.info.sender, ContractError::Unauthorized {});
        // Assert no funds are passed in
        nonpayable(&ctx.info)?;

        let owner_addr = ctx.deps.api.addr_validate(&owner)?;

        // Look up if there is a proxy to match. Fail or call burn on existing
        match self
            .proxy_by_owner
            .may_load(ctx.deps.storage, &owner_addr)?
        {
            None => Err(ContractError::NoProxy(owner)),
            Some(proxy_addr) => {
                // Send burn message to the proxy contract
                let msg = to_binary(&mesh_native_staking_proxy::contract::ExecMsg::Burn {
                    validator,
                    amount,
                })?;
                let wasm_msg = WasmMsg::Execute {
                    contract_addr: proxy_addr.into(),
                    msg,
                    funds: ctx.info.funds,
                };
                Ok(Response::new().add_message(wasm_msg))
            }
        }
    }

    /// Returns the maximum percentage that can be slashed
    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<SlashRatioResponse, Self::Error> {
        let Config {
            slash_ratio_dsign,
            slash_ratio_offline,
            ..
        } = self.config.load(ctx.deps.storage)?;
        Ok(SlashRatioResponse {
            slash_ratio_dsign,
            slash_ratio_offline,
        })
    }
}
