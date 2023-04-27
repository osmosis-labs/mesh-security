use cosmwasm_std::{Response, StdError, Uint128};
use sylvia::types::ExecCtx;
use sylvia::{interface, schemars};

// TODO: question - local staking should know which is vault, vault should know what is local staking...
// How to best handle the chicken and egg problem (2 step init with Option?)

/// This is the interface to the vault contract needed by staking contracts to release funds.
/// Users will need to use the other contract methods to actually manage funds
#[interface]
pub trait VaultApi {
    type Error: From<StdError>;

    /// This must be called by the remote staking contract to release this claim
    #[msg(exec)]
    fn release_remote(
        &self,
        ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
        // amount to unstake on that contract
        amount: Uint128,
    ) -> Result<Response, Self::Error>;

    /// This must be called by the local staking contract to release this claim
    /// Amount of tokens unstaked are those included in ctx.info.funds
    #[msg(exec)]
    fn release_local(
        &self,
        ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
    ) -> Result<Response, Self::Error>;
}
