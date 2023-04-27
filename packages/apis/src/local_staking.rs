use cosmwasm_std::{CosmosMsg, Response, StdError, StdResult};
use serde::{Deserialize, Serialize};
use sylvia::types::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

// TODO: no cw_serde equivalent??
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, sylvia::schemars::JsonSchema, Debug, Default)]
pub struct CanExecuteResp {
    pub can_execute: bool,
}

#[interface]
pub trait VaultApi {
    type Error: From<StdError>;

    /// Execute requests the contract to re-dispatch all these messages with the
    /// contract's address as sender. Every implementation has it's own logic to
    /// determine in
    #[msg(exec)]
    fn execute(&self, ctx: ExecCtx, msgs: Vec<CosmosMsg>) -> Result<Response, Self::Error>;

    /// Checks permissions of the caller on this proxy.
    /// If CanExecute returns true then a call to `Execute` with the same message,
    /// from the given sender, before any further state changes, should also succeed.
    #[msg(query)]
    fn can_execute(
        &self,
        ctx: QueryCtx,
        sender: String,
        msg: CosmosMsg,
    ) -> StdResult<CanExecuteResp>;
}
