use cosmwasm_std::{StdError, Uint128};
use cw_utils::{ParseReplyError, PaymentError};
use mesh_apis::ibc::VersionError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    PaymentError(#[from] PaymentError),

    #[error("{0}")]
    IbcVersion(#[from] VersionError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Contract already has an open IBC channel")]
    IbcChannelAlreadyOpen,

    #[error("You must start the channel handshake on this side, it doesn't support OpenTry")]
    IbcOpenTryDisallowed,

    #[error("The price provider contract does not accept packets")]
    IbcPacketRecvDisallowed,

    #[error("A subscription for the provided denom does not exist")]
    SubscriptionDoesNotExist,

    #[error("There is no subscription for the provided denom")]
    SubscriptionAlreadyExists,
}
