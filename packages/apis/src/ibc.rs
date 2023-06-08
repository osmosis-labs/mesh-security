use cosmwasm_schema::cw_serde;
use cosmwasm_std::Coin;

/// These are messages sent from provider -> consumer
/// ibc_packet_receive in converter must handle them all.
/// Each one has a different ack to be used in the reply.
#[cw_serde]
pub enum ProviderMsg {
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

/// Ack sent for ProviderMsg::Stake
#[cw_serde]
pub struct StakeAck {}

/// Ack sent for ProviderMsg::Unstake
#[cw_serde]
pub struct UnstakeAck {}

/// These are messages sent from consumer -> provider
/// ibc_packet_receive in external-staking must handle them all.
#[cw_serde]
pub enum ConsumerMsg {
    /// This is sent when a new validator registers and is available to receive
    /// delegations.
    /// This packet is sent right after the channel is opened to sync initial state
    AddValidators(Vec<Validator>),
    /// This is sent when a validator is tombstoned. Not just leaving the active state,
    /// but when it is no longer a valid target to delegate to.
    /// It contains a list of `valoper_address` to be removed
    RemoveValidators(Vec<String>),
    /// This is sent a validator changes the pubkey
    UpdatePubkey {
        /// This is the validator address that is changing the pubkey
        valoper_address: String,
        /// This is the block height (on the consumer) at which the pubkey was changed
        height: u64,
        /// This is the pubkey signing all blocks after `height`
        new_pubkey: String,
        /// This is the pubkey signing all blocks up to and including `height`
        old_pubkey: String,
    },
}

#[cw_serde]
pub struct Validator {
    /// This is the validator address used for delegations and rewards
    pub valoper_address: String,

    // TODO: is there a better type for this? what encoding is used
    /// This is the *Tendermint* public key, used for signing blocks.
    /// This is needed to detect slashing conditions
    pub pub_key: String,
}

/// Ack sent for ConsumerMsg::AddValidators
#[cw_serde]
pub struct AddValidatorsAck {}

/// Ack sent for ConsumerMsg::RemoveValidators
#[cw_serde]
pub struct RemoveValidatorsAck {}

/// Ack sent for ConsumerMsg::UpdatePubkey
#[cw_serde]
pub struct UpdatePubkeyAck {}
