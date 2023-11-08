#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, to_binary, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse, IbcMsg,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, IbcTimeout,
    StdError, Timestamp,
};

use mesh_apis::ibc::{
    validate_channel_order, PriceFeedProviderAck, ProtocolVersion, RemotePriceFeedPacket,
};

use crate::{contract::OsmosisPriceProvider, error::ContractError};

const PROTOCOL_NAME: &str = "mesh-security-price-feed";
/// This is the maximum version of the price feed protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "0.1.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "0.1.0";

const TIMEOUT: u64 = 60 * 60;

pub fn packet_timeout(now: &Timestamp) -> IbcTimeout {
    let timeout = now.plus_seconds(TIMEOUT);
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

    // verify the ordering is correct
    validate_channel_order(&channel.order)?;

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
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    let RemotePriceFeedPacket::QueryTwap {
        pool_id,
        base_asset,
        quote_asset,
    } = from_slice(&msg.packet.data)?;
    let contract = OsmosisPriceProvider::new();

    let twap = contract.query_twap(deps, pool_id, base_asset, quote_asset)?;
    Ok(IbcReceiveResponse::new().set_ack(to_binary(&PriceFeedProviderAck::Update { twap })?))
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// We get ACKs on sync state without much to do.
/// If it succeeded, take no action. If it errored, we can't do anything else and let it go.
/// We just log the error cases so they can be detected.
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcPacketAckDisallowed)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// The most we can do here is retry the packet, hoping it will eventually arrive.
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    env: Env,
    msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    // Play it again, Sam.
    let msg = IbcMsg::SendPacket {
        channel_id: msg.packet.src.channel_id,
        data: msg.packet.data,
        timeout: packet_timeout(&env.block.time),
    };
    Ok(IbcBasicResponse::new().add_message(msg))
}
