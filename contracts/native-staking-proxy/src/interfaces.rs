
use cosmwasm_std::{Binary, Response, StdError, Uint128};
use sylvia::types::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

/// This defines the interfaces the native-staking contract can count on from the native-staking-proxy
#[interface]
pub trait NativeToProxy {
    type Error: From<StdError>;

    /// The caller of the instantiation will be the native-staking contract.
    /// This call will come along with info.funds which should be staked on the given validator.
    /// Set the owner of the contract so they can handle restaking, voting, and unstaking later on
    #[msg(instantiate)]
    fn instantiate(
        &self,
        ctx: InstantiateCtx,
        owner: String,
        validator: String,
    ) -> Result<Response, ContractError>;

    /// Receives stake (info.funds) from native-staking contract on behalf of owner and
    /// stakes to the provided validator.
    #[msg(exec)]
    fn stake(
        &self,
        ctx: ExecCtx,
        validator: String,
    ) -> Result<Response, Self::Error>;
}

/// This defines the interfaces the native-staking-proxy contract can call on native-staking
#[interface]
pub trait ProxyToNative {
    type Error: From<StdError>;

    /// This sends tokens back from the proxy to native-staking. (See info.funds)
    /// The native-staking contract can determine which user it belongs to via an internal Map.
    /// The native-staking contract will then send those tokens back to vault and release the claim.
    #[msg(exec)]
    fn release_proxy_stake(
        &self,
        _ctx: ExecCtx,
    ) -> Result<Response, Self::Error>;
}
