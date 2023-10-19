use std::error::Error;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{to_binary, Binary, Coin, StdResult};

use crate::converter_api::RewardInfo;

/// These are messages sent from provider -> consumer
/// ibc_packet_receive in converter must handle them all.
/// Each one has a different ack to be used in the reply.
#[cw_serde]
pub enum ProviderPacket {
    /// This should be called when we lock more tokens to virtually stake on a given validator
    Stake {
        validator: String,
        /// This is the local (provider-side) denom that is held in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        stake: Coin,
        /// This is local to the sending side to track the transaction, should be passed through opaquely on the consumer
        tx_id: u64,
    },
    /// This should be called when we begin the unbonding period of some more tokens previously virtually staked
    Unstake {
        validator: String,
        /// This is the local (provider-side) denom that is held in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        unstake: Coin,
        /// This is local to the sending side to track the transaction, should be passed through opaquely on the consumer
        tx_id: u64,
    },
    /// This is part of the rewards protocol
    TransferRewards {
        /// Amount previously received by ConsumerPacket::Distribute
        rewards: Coin,
        /// A valid address on the consumer chain to receive these rewards
        recipient: String,
        /// This is local to the sending side to track the transaction, should be passed through opaquely on the consumer
        tx_id: u64,
    },
}

/// Ack sent for ProviderPacket::Stake
#[cw_serde]
pub struct StakeAck {}

/// Ack sent for ProviderPacket::Unstake
#[cw_serde]
pub struct UnstakeAck {}

/// Ack sent for ProviderPacket::TransferRewards
#[cw_serde]
pub struct TransferRewardsAck {}

/// These are messages sent from consumer -> provider
/// ibc_packet_receive in external-staking must handle them all.
#[cw_serde]
pub enum ConsumerPacket {
    ValsetUpdate {
        /// This is the height of the validator set update event.
        /// It is used to index the validator update events on the Provider.
        /// It can be used to detect slashing conditions, e.g. which header heights are punishable.
        height: u64,
        /// This is the timestamp of the update event.
        /// It may be used for unbonding_period issues, maybe just for informational purposes.
        /// Stored as unix seconds.
        time: u64,
        /// This is sent when a new validator registers and is available to receive delegations.
        /// One such packet is sent right after the channel is opened to sync initial state.
        /// If the validator already exists, or is tombstoned, this is a no-op for that validator.
        additions: Vec<AddValidator>,
        /// This is sent when a validator is removed from the active set because it doesn't have
        /// enough stake to be part of it.
        /// If the validator doesn't exist or is tombstoned, this is a no-op for that validator.
        removals: Vec<String>,
        /// This is sent sent when a validator changes pubkey. It will not change the validator's state.
        /// If the validator doesn't exist or is tombstoned, this is a no-op for that validator.
        updated: Vec<AddValidator>,
        /// This is sent when a validator is removed from the active set because it's being jailed for
        /// misbehaviour.
        /// The validator will be slashed for being offline as well.
        /// If the validator doesn't exist or is tombstoned, this is a no-op for that validator.
        jailed: Vec<String>,
        /// This is sent when a validator is a candidate to be added to the active set again.
        /// If the validator is also in the `removals` list, it will be marked as inactive /
        /// unbonded instead.
        /// If the validator doesn't exist or is tombstoned, this is a no-op for that validator.
        unjailed: Vec<String>,
        /// This is sent when a validator is tombstoned. Not just leaving the active state,
        /// but when it is no longer a valid target to delegate to.
        /// The validator will be slashed for double signing as well.
        /// If the validator doesn't exist or is already tombstoned, this is a no-op for that validator.
        /// This has precedence over all other events in the same packet
        tombstoned: Vec<String>,
    },
    /// This is part of the rewards protocol
    Distribute {
        /// The validator whose stakers should receive these rewards
        validator: String,
        /// The amount of rewards held on the consumer side to be released later
        rewards: Coin,
    },
    /// This is part of the rewards protocol
    DistributeBatch {
        /// Per-validator reward amounts
        rewards: Vec<RewardInfo>,
        /// Rewards denom
        denom: String,
    },
}

#[cw_serde]
pub struct AddValidator {
    /// This is the validator operator (valoper) address used for delegations and rewards
    pub valoper: String,

    // TODO: is there a better type for this? what encoding is used
    /// This is the *Tendermint* public key, used for signing blocks.
    /// This is needed to detect slashing conditions
    pub pub_key: String,
}

impl AddValidator {
    pub fn mock(valoper: &str) -> Self {
        Self {
            valoper: valoper.to_string(),
            pub_key: "mock-pubkey".to_string(),
        }
    }
}

/// Ack sent for ConsumerPacket::ValsetUpdate
#[cw_serde]
pub struct ValsetUpdateAck {}

/// Ack sent for ConsumerPacket::Distribute and ConsumerPacket::DistributeBatch
#[cw_serde]
pub struct DistributeAck {}

/// This is a generic ICS acknowledgement format.
/// Protobuf defined here: https://github.com/cosmos/cosmos-sdk/blob/v0.42.0/proto/ibc/core/channel/v1/channel.proto#L141-L147
/// This is compatible with the JSON serialization.
/// Wasmd uses this same wrapper for unhandled errors.
#[cw_serde]
pub enum AckWrapper {
    Result(Binary),
    Error(String),
}

// create a serialized success message
pub fn ack_success<T: serde::Serialize>(data: &T) -> StdResult<Binary> {
    let res = AckWrapper::Result(to_binary(data)?);
    to_binary(&res)
}

// create a serialized error message
pub fn ack_fail<E: Error>(err: E) -> StdResult<Binary> {
    let res = AckWrapper::Error(err.to_string());
    to_binary(&res)
}
