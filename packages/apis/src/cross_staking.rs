use cosmwasm_std::{Binary, Response, StdError, Uint128};
use sylvia::types::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

use crate::local_staking::MaxSlashResponse;

/// This is the interface to any cross staking contract needed by the vault contract.
/// That is, using the vault collateral to stake on a system that doesn't use the collateral
/// as the native staking token. This involves the concept of "virtual stake"
///
/// Users will need to use implementation-specific methods to actually manage funds,
/// this just clarifies the interaction with the Vault contract
#[interface]
pub trait CrossStakingApi {
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
