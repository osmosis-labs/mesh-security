use std::error::Error;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{to_json_binary, Binary, Coin, StdError, StdResult};
use osmosis_std::shim::Timestamp as OsmosisTimestamp;
use osmosis_std::types::tendermint::abci::RequestQuery;
use prost::Message;

use crate::converter_api::{RewardInfo, ValidatorSlashInfo};

/// These are messages sent from provider -> consumer
/// ibc_packet_receive in converter must handle them all.
/// Each one has a different ack to be used in the reply.
#[cw_serde]
pub enum ProviderPacket {
    /// This should be called when we lock more tokens to virtually stake on a given validator
    Stake {
        delegator: String,
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
        delegator: String,
        validator: String,
        /// This is the local (provider-side) denom that is held in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        unstake: Coin,
        /// This is local to the sending side to track the transaction, should be passed through opaquely on the consumer
        tx_id: u64,
    },
    /// This should be called when we burn tokens from the given validators, because of slashing
    /// propagation / vault invariants keeping.
    /// If there is more than one validator, the burn amount will be split evenly between them.
    /// This is non-transactional, as if it fails we cannot do much about it, besides logging the failure.
    Burn {
        validators: Vec<String>,
        /// This is the local (provider-side) denom that is being burned in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        burn: Coin,
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
        /// This is sent when a validator is slashed.
        /// If the validator doesn't exist or is inactive at the infraction height, this is a no-op
        /// for that validator.
        /// This has precedence over all other events in the same packet.
        slashed: Vec<ValidatorSlashInfo>,
    },
    /// This is a part of zero max cap process
    /// The consumer chain will send this packet to provider, force user to unbond token
    InternalUnstake {
        delegator: String,
        validator: String,
        /// This is the local (provider-side) denom that is held in the vault.
        /// It will be converted to the consumer-side staking token in the converter with help
        /// of the price feed.
        normalize_amount: Coin,
        inverted_amount: Coin,
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

#[cw_serde]
pub struct PriceFeedAck {}

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
    let res = AckWrapper::Result(to_json_binary(data)?);
    to_json_binary(&res)
}

// create a serialized error message
pub fn ack_fail<E: Error>(err: E) -> StdResult<Binary> {
    let res = AckWrapper::Error(err.to_string());
    to_json_binary(&res)
}

#[cw_serde]
pub struct InterchainQueryPacketData {
    data: Binary,
    memo: String,
}

pub fn ibc_query_packet(packet: CosmosQuery) -> InterchainQueryPacketData {
    InterchainQueryPacketData {
        data: Binary::new(packet.encode_to_vec()),
        memo: "".to_string(),
    }
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    Clone,
    PartialEq,
    Eq,
    ::prost::Message,
    ::serde::Serialize,
    ::serde::Deserialize,
    ::schemars::JsonSchema,
)]
pub struct CosmosQuery {
    #[prost(message, repeated, tag = "1")]
    pub requests: ::prost::alloc::vec::Vec<RequestQuery>,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    Clone,
    PartialEq,
    Eq,
    ::prost::Message,
    ::serde::Serialize,
    ::serde::Deserialize,
    ::schemars::JsonSchema,
)]
pub struct ArithmeticTwapToNowRequest {
    #[prost(uint64, tag = "1")]
    #[serde(alias = "poolID")]
    pub pool_id: u64,
    #[prost(string, tag = "2")]
    pub base_asset: ::prost::alloc::string::String,
    #[prost(string, tag = "3")]
    pub quote_asset: ::prost::alloc::string::String,
    #[prost(message, optional, tag = "4")]
    pub start_time: ::core::option::Option<OsmosisTimestamp>,
}

pub fn encode_request(request: &ArithmeticTwapToNowRequest) -> Vec<u8> {
    request.encode_to_vec()
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    Clone,
    PartialEq,
    Eq,
    ::prost::Message,
    ::serde::Serialize,
    ::serde::Deserialize,
    ::schemars::JsonSchema,
)]
pub struct CosmosResponse {
    #[prost(message, repeated, tag = "1")]
    pub responses: ::prost::alloc::vec::Vec<ResponseQuery>,
}

pub fn decode_response(bytes: &[u8]) -> StdResult<CosmosResponse> {
    CosmosResponse::decode(bytes)
        .map_err(|err| StdError::generic_err(format!("fail to decode response query: {}", err)))
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    Clone,
    PartialEq,
    Eq,
    ::prost::Message,
    ::serde::Serialize,
    ::serde::Deserialize,
    ::schemars::JsonSchema,
)]
pub struct ResponseQuery {
    #[prost(uint32, tag = "1")]
    pub code: u32,

    #[prost(int64, tag = "2")]
    pub index: i64,

    #[prost(bytes = "vec", tag = "3")]
    pub key: ::prost::alloc::vec::Vec<u8>,

    #[prost(bytes = "vec", tag = "4")]
    pub value: ::prost::alloc::vec::Vec<u8>,

    #[prost(int64, tag = "5")]
    pub height: i64,
}

#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(
    Clone,
    PartialEq,
    Eq,
    ::prost::Message,
    ::serde::Serialize,
    ::serde::Deserialize,
    ::schemars::JsonSchema,
)]
pub struct QueryArithmeticTwapToNowResponse {
    #[prost(string, tag = "1")]
    pub arithmetic_twap: ::prost::alloc::string::String,
}

pub fn decode_twap_response(bytes: &[u8]) -> StdResult<QueryArithmeticTwapToNowResponse> {
    QueryArithmeticTwapToNowResponse::decode(bytes)
        .map_err(|err| StdError::generic_err(format!("fail to decode twap: {}", err)))
}

#[cw_serde]
pub struct AcknowledgementResult {
    pub result: Binary,
}

#[cw_serde]
pub struct InterchainQueryPacketAck {
    pub data: Binary,
}
