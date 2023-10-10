use cosmwasm_std::{StdError, Uint128};
use cw_utils::PaymentError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Try to send wrong denom: {0}")]
    InvalidDenom(String),

    #[error("Validator {0} has not enough delegated funds: {1}")]
    InsufficientDelegation(String, Uint128),
}
