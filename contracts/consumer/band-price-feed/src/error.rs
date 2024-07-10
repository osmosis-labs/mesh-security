use cosmwasm_std::StdError;
use cw_utils::PaymentError;
use mesh_apis::ibc::VersionError;
use thiserror::Error;

use crate::price_keeper::PriceKeeperError;

/// Never is a placeholder to ensure we don't return any errors
#[derive(Error, Debug)]
pub enum Never {}

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("{0}")]
    PriceKeeper(#[from] PriceKeeperError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Request didn't suceess")]
    RequestNotSuccess {},

    #[error("{0}")]
    IbcVersion(#[from] VersionError),

    #[error("The provided IBC channel is not open")]
    IbcChannelNotOpen,

    #[error("Contract already has an open IBC channel")]
    IbcChannelAlreadyOpen,

    #[error("You must start the channel handshake on the other side, it doesn't support OpenInit")]
    IbcOpenInitDisallowed,

    #[error("Contract does not receive packets ack")]
    IbcAckNotAccepted,

    #[error("Contract does not receive packets timeout")]
    IbcTimeoutNotAccepted,

    #[error("Response packet should only contains 2 symbols")]
    InvalidResponsePacket,

    #[error("Symbol must be base denom or quote denom")]
    SymbolsNotMatch,

    #[error("Invalid price, must be greater than 0.0")]
    InvalidPrice,

    #[error("Custom Error val: {val:?}")]
    CustomError { val: String },
    // Add any other custom errors you like here.
    // Look at https://docs.rs/thiserror/1.0.21/thiserror/ for details.
}
