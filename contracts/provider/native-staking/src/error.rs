use cosmwasm_std::StdError;
use cw_utils::{ParseReplyError, PaymentError};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    ParseReply(#[from] ParseReplyError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("Unauthorized")]
    Unauthorized {},

    #[error("Invalid reply id: {0}")]
    InvalidReplyId(u64),

    #[error("Missing instantiate reply data")]
    NoInstantiateData {},

    #[error("Missing proxy contract for {0}")]
    NoProxy(String),

    #[error("You cannot specify a slash ratio over 1.0 (100%)")]
    InvalidSlashRatio,
}
