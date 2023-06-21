use cosmwasm_std::StdError;
use cw_utils::PaymentError;
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

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Contract already has an open IBC channel")]
    IbcChannelAlreadyOpen,

    #[error("You must start the channel handshake on this side, it doesn't support OpenTry")]
    IbcOpenTryDisallowed,
}
