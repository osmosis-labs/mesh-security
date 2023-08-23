use cosmwasm_std::{Addr, StdError, Uint128};
use cw_utils::{ParseReplyError, PaymentError};
use mesh_sync::{RangeError, Tx, ValueRange};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("{0}")]
    ParseReply(#[from] ParseReplyError),

    #[error("{0}")]
    Range(#[from] RangeError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("All denoms are expected to be {0}")]
    UnexpectedDenom(String),

    #[error("Claim is locked, only {0} can be unbonded")]
    ClaimsLocked(ValueRange<Uint128>),

    #[error("The address doesn't have sufficient balance for this operation")]
    InsufficentBalance,

    #[error("The lienholder doesn't have any claims")]
    UnknownLienholder,

    #[error("The lienholder doesn't have enough claims for the action")]
    InsufficientLien,

    #[error("Invalid reply id: {0}")]
    InvalidReplyId(u64),

    #[error("The tx {0} exists but is of the wrong type: {1}")]
    WrongTypeTx(u64, Tx),

    #[error("The tx {0} exists but comes from the wrong address: {1}")]
    WrongContractTx(u64, Addr),
}
