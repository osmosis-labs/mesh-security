use cosmwasm_schema::cw_serde;
use cosmwasm_std::Uint128;

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
        /// We can't really use Coin here, as which denom? Remote staking denom, or how that
        /// token appears as ibc denom. Just encode the amount.
        amount: Uint128,
    },
    /// This should be called when we begin the unbonding period of some more tokens previously virtually staked
    Unstake {
        validator: String,
        /// We can't really use Coin here, as which denom? Remote staking denom, or how that
        /// token appears as ibc denom. Just encode the amount.
        amount: Uint128,
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

    /// This is the moniker of the validator, used for display purposes
    pub moniker: String,
}

/// These are messages sent from consumer -> provider
/// ibc_packet_receive in external-staking must handle them all.
#[cw_serde]
pub enum ConsumerMsg {
    /// This should be sent when the validator set changes.
    /// Add includes full validator info to add.
    /// Remove includes only the valoper_address to remove.
    ///
    /// Question: This should not be sent when a validator enters/leaves the "active set",
    /// but rather when they are tombstoned.
    UpdateValidatorSet {
        add: Vec<Validator>,
        remove: Vec<String>,
    },
}

/// Ack sent for ConsumerMsg::UpdateValidatorSet
#[cw_serde]
pub struct UpdateValidatorSetAck {}
