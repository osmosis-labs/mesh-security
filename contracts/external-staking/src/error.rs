use cosmwasm_std::{StdError, Uint128};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Invalid denom, {0} expected")]
    InvalidDenom(String),

    #[error("Not enough tokens staked, up to {0} can be unbond")]
    NotEnoughStake(Uint128),

    #[error("Not enough tokens released, up to {0} can be claimed")]
    NotEnoughRelease(Uint128),
}
