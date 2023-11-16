#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, to_binary, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, IbcTimeout,
    StdError, Timestamp,
};

use mesh_apis::ibc::{PriceFeedProviderAck, ProtocolVersion, RemotePriceFeedPacket};

use crate::{contract::OsmosisPriceProvider, error::ContractError};

const PROTOCOL_NAME: &str = "mesh-security-price-feed";
/// This is the maximum version of the price feed protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "0.1.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "0.1.0";

const TIMEOUT_IN_SECS: u64 = 600;

pub fn packet_timeout(now: &Timestamp) -> IbcTimeout {
    let timeout = now.plus_seconds(TIMEOUT_IN_SECS);
    IbcTimeout::with_timestamp(timeout)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// enforces ordering and versioning constraints
pub fn ibc_channel_open(
    _deps: DepsMut,
    _env: Env,
    msg: IbcChannelOpenMsg,
) -> Result<IbcChannelOpenResponse, ContractError> {
    // ensure we are called with OpenInit
    let channel = match msg {
        IbcChannelOpenMsg::OpenInit { channel } => channel,
        IbcChannelOpenMsg::OpenTry { .. } => return Err(ContractError::IbcOpenTryDisallowed),
    };

    // Check the version. If provided, ensure it is compatible.
    // If not provided, use our most recent version.
    let version = if channel.version.is_empty() {
        ProtocolVersion {
            protocol: PROTOCOL_NAME.to_string(),
            version: SUPPORTED_IBC_PROTOCOL_VERSION.to_string(),
        }
    } else {
        let v: ProtocolVersion = from_slice(channel.version.as_bytes())?;
        // if we can build a response to this, then it is compatible. And we use the highest version there
        v.build_response(SUPPORTED_IBC_PROTOCOL_VERSION, MIN_IBC_PROTOCOL_VERSION)?
    };

    let response = Ibc3ChannelOpenResponse {
        version: version.to_string()?,
    };
    Ok(Some(response))
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// once it's established, we store data
pub fn ibc_channel_connect(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelConnectMsg,
) -> Result<IbcBasicResponse, ContractError> {
    // ensure we are called with OpenAck
    let (channel, counterparty_version) = match msg {
        IbcChannelConnectMsg::OpenAck {
            channel,
            counterparty_version,
        } => (channel, counterparty_version),
        IbcChannelConnectMsg::OpenConfirm { .. } => {
            return Err(ContractError::IbcOpenTryDisallowed)
        }
    };

    // Ensure the counterparty responded with a version we support.
    // Note: here, we error if it is higher than what we proposed originally
    let v: ProtocolVersion = from_slice(counterparty_version.as_bytes())?;
    v.verify_compatibility(SUPPORTED_IBC_PROTOCOL_VERSION, MIN_IBC_PROTOCOL_VERSION)?;

    let contract = OsmosisPriceProvider::new();
    contract
        .channels
        .update(deps.storage, |mut v| -> Result<_, StdError> {
            v.push(channel);
            Ok(v)
        })?;

    Ok(IbcBasicResponse::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_close(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelCloseMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let contract = OsmosisPriceProvider::new();
    contract
        .channels
        .update(deps.storage, |mut v| -> Result<_, ContractError> {
            let ix = v
                .iter()
                .enumerate()
                .find(|(_, c)| c == &msg.channel())
                .ok_or(ContractError::IbcChannelNotOpen)?
                .0;
            v.remove(ix);
            Ok(v)
        })?;

    Ok(IbcBasicResponse::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_receive(
    deps: DepsMut,
    env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    let RemotePriceFeedPacket::QueryTwap {
        pool_id,
        base_asset,
        quote_asset,
    } = from_slice(&msg.packet.data)?;
    let contract = OsmosisPriceProvider::new();

    let time = env.block.time;
    let twap = contract.query_twap(deps, pool_id, base_asset, quote_asset)?;
    Ok(IbcReceiveResponse::new().set_ack(to_binary(&PriceFeedProviderAck::Update { time, twap })?))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcPacketAckDisallowed)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcPacketAckDisallowed)
}
