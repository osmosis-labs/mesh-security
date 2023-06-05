use cosmwasm_std::{to_binary, Binary, Coin, Decimal, Response, StdError, StdResult, WasmMsg};
use mesh_apis::local_staking_api::{self, LocalStakingApi, MaxSlashResponse};
use mesh_apis::vault_api;
use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

/// This is a stub implementation of local staking contract, for test purposes only
/// When proper local staking contract is available, this should be replaced
/// in multitests
pub struct LocalStaking;

#[contract]
#[messages(local_staking_api as LocalStakingApi)]
impl LocalStaking {
    const fn new() -> Self {
        Self
    }

    #[msg(instantiate)]
    pub fn instantiate(&self, _ctx: InstantiateCtx) -> StdResult<Response> {
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
        let msg = vault_api::ExecMsg::release_local_stake(owner);
        let msg = WasmMsg::Execute {
            contract_addr: vault,
            msg: to_binary(&msg)?,
            funds: vec![amount],
        };

        let resp = Response::new().add_message(msg);
        Ok(resp)
    }
}

#[contract]
#[messages(local_staking_api as LocalStakingApi)]
impl LocalStakingApi for LocalStaking {
    type Error = StdError;

    #[msg(exec)]
    fn receive_stake(&self, _ctx: ExecCtx, _owner: String, _msg: Binary) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(query)]
    fn max_slash(&self, _ctx: QueryCtx) -> StdResult<MaxSlashResponse> {
        let resp = MaxSlashResponse {
            max_slash: Decimal::percent(10),
        };

        Ok(resp)
    }
}
