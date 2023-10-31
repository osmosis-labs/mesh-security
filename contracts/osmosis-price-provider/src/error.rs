use cosmwasm_std::StdError;
use cw_utils::PaymentError;
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

    #[error("The provided IBC channel is not open")]
    IbcChannelNotOpen,

    #[error("You must start the channel handshake on this side, it doesn't support OpenTry")]
    IbcOpenTryDisallowed,

    #[error("The price provider contract does not accept packets")]
    IbcPacketRecvDisallowed,

    #[error("A subscription for the provided denom does not exist")]
    SubscriptionDoesNotExist,

    #[error("A subscription already exists for the provided denom")]
    SubscriptionAlreadyExists,
}
