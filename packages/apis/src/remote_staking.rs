use cosmwasm_std::{Binary, Response, StdError, Uint128};
use sylvia::types::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

use crate::local_staking::MaxSlashResponse;

/// This is the interface to any remote staking contract needed by the vault contract.
/// Users will need to use the custom methods to actually manage funds
#[interface]
pub trait RemoteStakingApi {
    type Error: From<StdError>;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_virtual_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Uint128,
        msg: Binary,
    ) -> Result<Response, Self::Error>;

    /// Returns the maximum percentage that can be slashed
    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error>;
}
