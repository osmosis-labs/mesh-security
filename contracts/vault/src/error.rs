use cosmwasm_std::{StdError, Uint128};
use cw_utils::{ParseReplyError, PaymentError};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("{0}")]
    ParseReply(#[from] ParseReplyError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("All denoms are expected to be {0}")]
    UnexpectedDenom(String),

    #[error("Claim is locked, only {0} can be unbonded")]
    ClaimsLocked(Uint128),

    #[error("The address doesn't have sufficient balance for this operation")]
    InsufficentBalance,

    #[error("The lienholder doesn't have any claims")]
    UnknownLienholder,

    #[error("The lienholder doesn't have enough claims for the action")]
    InsufficientLien,

    #[error("Invalid reply id: {0}")]
    InvalidReplyId(u64),
}
