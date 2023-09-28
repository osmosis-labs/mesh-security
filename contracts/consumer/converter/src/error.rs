use cosmwasm_std::{StdError, Uint128};
use cw_utils::{ParseReplyError, PaymentError};
use mesh_apis::ibc::VersionError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("{0}")]
    IbcVersion(#[from] VersionError),

    #[error("{0}")]
    ParseReply(#[from] ParseReplyError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Contract already has an open IBC channel")]
    IbcChannelAlreadyOpen,

    #[error("You must start the channel handshake on this side, it doesn't support OpenTry")]
    IbcOpenTryDisallowed,

    #[error("Sent wrong denom over IBC: {sent}, expected {expected}")]
    WrongDenom { sent: String, expected: String },

    #[error("Invalid reply id: {0}")]
    InvalidReplyId(u64),

    #[error("Invalid discount, must be between 0.0 and 1.0")]
    InvalidDiscount,

    #[error("Invalid denom: {0}")]
    InvalidDenom(String),

    #[error("Sum of rewards ({sum}) doesn't match funds sent ({sent})")]
    DistributeRewardsInvalidAmount { sum: Uint128, sent: Uint128 },
}
