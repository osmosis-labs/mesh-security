use cosmwasm_schema::cw_serde;
use cosmwasm_std::Coin;

/// These are messages sent from provider -> consumer
/// ibc_packet_receive in converter must handle them all.
/// Each one has a different ack to be used in the reply.
#[cw_serde]
pub enum ProviderMsg {
    /// This should be called on initialization to get current list of validators.
    /// Any changes to the set should be sent as a ConsumerMsg::UpdateValidatorSet
    ListValidators {},
    /// This should be called when we lock more tokens to virtually stake on a given validator
    Stake {
        validator: String,
        /// This is the local (provider-side) denom that is held in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        stake: Coin,
    },
    /// This should be called when we begin the unbonding period of some more tokens previously virtually staked
    Unstake {
        validator: String,
        /// This is the local (provider-side) denom that is held in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        unstake: Coin,
    },
}

/// Ack sent for ProviderMsg::ListValidators
#[cw_serde]
pub struct ListValidatorsAck {
    pub validators: Vec<Validator>,
}

/// Ack sent for ProviderMsg::Stake
#[cw_serde]
pub struct StakeAck {}

/// Ack sent for ProviderMsg::Unstake
#[cw_serde]
pub struct UnstakeAck {}

#[cw_serde]
pub struct Validator {
    /// This is the validator address used for delegations and rewards
    pub valoper_address: String,

    // TODO: is there a better type for this? what encoding is used
    /// This is the *Tendermint* public key, used for signing blocks.
    /// This is needed to detect slashing conditions
    pub pub_key: String,
}

/// These are messages sent from consumer -> provider
/// ibc_packet_receive in external-staking must handle them all.
#[cw_serde]
pub enum ConsumerMsg {
    /// This is sent when a new validator registers and is available to receive
    /// delegations.
    AddValidator(Validator),
    /// This is sent when a validator is tombstoned. Not just leaving the active state,
    /// but when it is no longer a valid target to delegate to.
    RemoveValidator { valoper_address: String },
}

/// Ack sent for ConsumerMsg::UpdateValidatorSet
#[cw_serde]
pub struct UpdateValidatorSetAck {}
