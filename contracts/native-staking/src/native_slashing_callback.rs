use cosmwasm_std::Response;
use cw_utils::must_pay;
use sylvia::contract;
use sylvia::types::ExecCtx;

use mesh_apis::vault_api::VaultApiHelper;
use mesh_native_staking_proxy::native_staking_callback::{self, NativeStakingCallback};

use crate::contract::NativeStakingContract;
use crate::error::ContractError;

#[contract]
#[messages(native_staking_callback as NativeStakingCallback)]
impl NativeStakingCallback for NativeStakingContract<'_> {
    type Error = ContractError;

    /// This sends tokens back from the proxy to native-staking. (See info.funds)
    /// The native-staking contract can determine which user it belongs to via an internal Map.
    /// The native-staking contract will then send those tokens back to vault and release the claim.
    #[msg(exec)]
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
        let msg = VaultApiHelper(cfg.vault)
            .release_local_stake(owner_addr.to_string(), ctx.info.funds)?;

        Ok(Response::new().add_message(msg))
    }
}
