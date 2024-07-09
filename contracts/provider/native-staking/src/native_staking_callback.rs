use cosmwasm_std::Response;
use cw_utils::must_pay;
use sylvia::types::ExecCtx;

#[allow(unused_imports)]
use mesh_native_staking_proxy::native_staking_callback::{self, NativeStakingCallback};

use crate::contract::{custom, NativeStakingContract};
use crate::error::ContractError;

impl NativeStakingCallback for NativeStakingContract<'_> {
    type Error = ContractError;
    type ExecC = custom::NativeStakingMsg;

    /// This sends tokens back from the proxy to native-staking. (See info.funds)
    /// The native-staking contract can determine which user it belongs to via an internal Map.
    /// The native-staking contract will then send those tokens back to vault and release the claim.
    fn release_proxy_stake(&self, ctx: ExecCtx) -> Result<Response, Self::Error> {
        let cfg = self.config.load(ctx.deps.storage)?;

        // Assert funds are passed in
        let _paid = must_pay(&ctx.info, &cfg.denom)?;

        // Look up account owner by proxy address (info.sender). This asserts the caller is a valid
        // proxy
        let owner_addr = self
            .owner_by_proxy
            .load(ctx.deps.storage, &ctx.info.sender)?;

        // Send the tokens to the vault contract
        let msg = cfg
            .vault
            .release_local_stake(owner_addr.to_string(), ctx.info.funds)?;

        Ok(Response::new().add_message(msg))
    }
}