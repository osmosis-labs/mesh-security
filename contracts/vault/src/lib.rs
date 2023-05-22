mod collateral;
pub mod contract;
pub mod error;
pub mod msg;
mod state;

#[allow(dead_code)]
#[cfg(not(any(feature = "library", tarpaulin_include)))]
mod entry_points {
    use cosmwasm_std::{entry_point, Binary, Deps, DepsMut, Env, MessageInfo, Response};

    use crate::contract::{ContractExecMsg, ContractQueryMsg, InstantiateMsg, VaultContract};
    use crate::error::ContractError;

    const CONTRACT: VaultContract = VaultContract::new();

    #[entry_point]
    pub fn instantiate(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: InstantiateMsg,
    ) -> Result<Response, ContractError> {
        msg.dispatch(&CONTRACT, (deps, env, info))
    }

    #[entry_point]
    pub fn execute(
        deps: DepsMut,
        env: Env,
        info: MessageInfo,
        msg: ContractExecMsg,
    ) -> Result<Response, ContractError> {
        msg.dispatch(&CONTRACT, (deps, env, info))
    }

    #[entry_point]
    pub fn query(deps: Deps, env: Env, msg: ContractQueryMsg) -> Result<Binary, ContractError> {
        msg.dispatch(&CONTRACT, (deps, env))
    }
}
