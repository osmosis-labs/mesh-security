use cosmwasm_std::{Response, StdResult};
use sylvia::{contract, types::InstantiateCtx};

/// This is a stub implementation of local staking contract, for test purposes only
/// When proper local staking contract is available, this should be replaced
/// in multitests
pub struct LocalStaking;

#[contract]
impl LocalStaking {
    #[allow(dead_code)]
    const fn new() -> Self {
        Self
    }

    #[msg(instantiate)]
    pub fn instantiate(&self, _ctx: InstantiateCtx) -> StdResult<Response> {
        Ok(Response::new())
    }
}
