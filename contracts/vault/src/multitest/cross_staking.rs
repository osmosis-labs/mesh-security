use cosmwasm_std::{to_binary, Binary, Coin, Decimal, Response, StdError, StdResult, WasmMsg};
use cw_storage_plus::Item;
use mesh_apis::cross_staking_api::{self, CrossStakingApi};
use mesh_apis::local_staking_api::MaxSlashResponse;
use mesh_apis::vault_api;
use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

/// This is a stub implementation of cross staking contract, for test purposes only
/// When proper cross staking contract is available, this should be replaced
/// in multitests
pub struct CrossStaking<'a> {
    max_slash: Item<'a, Decimal>,
}

#[contract]
#[messages(cross_staking_api as CrossStakingApi)]
impl CrossStaking<'_> {
    const fn new() -> Self {
        Self {
            max_slash: Item::new("max_slash"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(&self, ctx: InstantiateCtx, max_slash: Decimal) -> StdResult<Response> {
        self.max_slash.save(ctx.deps.storage, &max_slash)?;
        Ok(Response::new())
    }

    #[msg(exec)]
    pub fn unstake(
        &self,
        _ctx: ExecCtx,
        vault: String,
        owner: String,
        amount: Coin,
    ) -> StdResult<Response> {
        let msg = vault_api::ExecMsg::release_cross_stake(owner, amount);
        let msg = WasmMsg::Execute {
            contract_addr: vault,
            msg: to_binary(&msg)?,
            funds: vec![],
        };

        let resp = Response::new().add_message(msg);
        Ok(resp)
    }
}

#[contract]
#[messages(cross_staking_api as CrossStakingApi)]
impl CrossStakingApi for CrossStaking<'_> {
    type Error = StdError;

    #[msg(exec)]
    fn receive_virtual_stake(
        &self,
        _ctx: ExecCtx,
        _owner: String,
        _amount: Coin,
        _msg: Binary,
    ) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> StdResult<MaxSlashResponse> {
        let resp = MaxSlashResponse {
            max_slash: self.max_slash.load(ctx.deps.storage)?,
        };

        Ok(resp)
    }
}
