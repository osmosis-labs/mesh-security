#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, to_binary, DepsMut, Env, Event, Ibc3ChannelOpenResponse, IbcBasicResponse,
    IbcChannel, IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg,
    IbcChannelOpenResponse, IbcMsg, IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg,
    IbcReceiveResponse, IbcTimeout,
};
use cw_storage_plus::Item;

use mesh_apis::ibc::{validate_channel_order, AckWrapper, ConsumerPacket, ProtocolVersion};
use sylvia::types::ExecCtx;

use crate::error::ContractError;

const PROTOCOL_NAME: &str = "mesh-security-price-feed";
/// This is the maximum version of the price feed protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "0.1.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "0.1.0";

// IBC specific state
pub const IBC_CHANNEL: Item<IbcChannel> = Item::new("ibc_channel");

const TIMEOUT: u64 = 60 * 60;

pub fn packet_timeout(env: &Env) -> IbcTimeout {
    let timeout = env.block.time.plus_seconds(TIMEOUT);
    IbcTimeout::with_timestamp(timeout)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// enforces ordering and versioning constraints
pub fn ibc_channel_open(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelOpenMsg,
) -> Result<IbcChannelOpenResponse, ContractError> {
    // ensure we have no channel yet
    if IBC_CHANNEL.may_load(deps.storage)?.is_some() {
        return Err(ContractError::IbcChannelAlreadyOpen);
    }
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
    env: Env,
    msg: IbcChannelConnectMsg,
) -> Result<IbcBasicResponse, ContractError> {
    // ensure we have no channel yet
    if IBC_CHANNEL.may_load(deps.storage)?.is_some() {
        return Err(ContractError::IbcChannelAlreadyOpen);
    }
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

    todo!("store the channel in subscriptions");

    Ok(IbcBasicResponse::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcChannelCloseMsg,
) -> Result<IbcBasicResponse, ContractError> {
    todo!("remove subscription");
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_receive(
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    // this contract only sends out update packets over IBC - it's not meant to receive any
    Err(ContractError::IbcPacketRecvDisallowed)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// We get ACKs on sync state without much to do.
/// If it succeeded, take no action. If it errored, we can't do anything else and let it go.
/// We just log the error cases so they can be detected.
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let ack: AckWrapper = from_slice(&msg.acknowledgement.data)?;
    let mut res = IbcBasicResponse::new();
    match ack {
        AckWrapper::Result(_) => {}
        AckWrapper::Error(e) => {
            // The wasmd framework will label this with the contract_addr, which helps us find the port and issue.
            // Provide info to find the actual packet.
            let event = Event::new("mesh_price_feed_ibc_error")
                .add_attribute("error", e)
                .add_attribute("channel", msg.original_packet.src.channel_id)
                .add_attribute("sequence", msg.original_packet.sequence.to_string());
            res = res.add_event(event);
        }
    }
    Ok(res)
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
        timeout: packet_timeout(&env),
    };
    Ok(IbcBasicResponse::new().add_message(msg))
}

pub(crate) fn make_ibc_packet(
    ctx: &mut ExecCtx,
    packet: ConsumerPacket,
) -> Result<IbcMsg, ContractError> {
    let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
    Ok(IbcMsg::SendPacket {
        channel_id: channel.endpoint.channel_id,
        data: to_binary(&packet)?,
        timeout: packet_timeout(&ctx.env),
    })
}
