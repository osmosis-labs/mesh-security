use cosmwasm_std::StdError;
use cw_utils::{ParseReplyError, PaymentError};
use thiserror::Error;

#[derive(Error, Debug)]
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

    #[error("You cannot use a max slashing rate over 1.0 (100%)")]
    InvalidMaxSlashing,
}
