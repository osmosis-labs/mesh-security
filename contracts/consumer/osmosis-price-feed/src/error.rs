use cosmwasm_std::StdError;
use cw_utils::PaymentError;
use mesh_apis::ibc::VersionError;
use thiserror::Error;

use mesh_price_feed::PriceKeeperError;

#[derive(Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("{0}")]
    IbcVersion(#[from] VersionError),

    #[error("{0}")]
    PriceKeeper(#[from] PriceKeeperError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Invalid authorized endpoint: {0}")]
    InvalidEndpoint(String),

    #[error("Only supports channel with ibc version icq-1, got {version}")]
    InvalidIbcVersion { version: String },

    #[error("invalid ibc packet, result should only contains 1 ResponseQuery")]
    InvalidResponseQuery,

    #[error("failed to send interchain query")]
    InvalidResponseQueryCode,

    #[error("twap data is empty")]
    EmptyTwap,

    #[error("Contract doesn't have an open IBC channel")]
    IbcChannelNotOpen,

    #[error("Contract already has an open IBC channel")]
    IbcChannelAlreadyOpen,

    #[error("You must start the channel handshake on the other side, it doesn't support OpenInit")]
    IbcOpenInitDisallowed,

    #[error("Contract does not receive packets except for acknowledgements")]
    IbcReceiveNotAccepted,

    #[error("The oracle hasn't received any price data")]
    NoPriceData,

    #[error("The oracle's price data is outdated")]
    OutdatedPriceData,
}
