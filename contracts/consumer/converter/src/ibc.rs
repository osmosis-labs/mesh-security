#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse, IbcChannel,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse,
};
use cw_storage_plus::Item;

use mesh_apis::ibc::{validate_channel_order, ProtocolVersion, PROTOCOL_NAME};

use crate::error::ContractError;

/// This is the maximum version of the Mesh Security protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "1.0.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "1.0.0";

// IBC specific state
const IBC_CHANNEL: Item<IbcChannel> = Item::new("ibc_channel");

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
    _deps: DepsMut,
    _env: Env,
    msg: IbcChannelConnectMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let _channel = msg.channel();
    todo!();
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// On closed channel, we take all tokens from reflect contract to this contract.
/// We also delete the channel entry from accounts.
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcChannelCloseMsg,
) -> Result<IbcBasicResponse, ContractError> {
    todo!();
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// we look for a the proper reflect contract to relay to and send the message
/// We cannot return any meaningful response value as we do not know the response value
/// of execution. We just return ok if we dispatched, error if we failed to dispatch
pub fn ibc_packet_receive(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    todo!();
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// never should be called as we do not send packets
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_packet_ack"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// never should be called as we do not send packets
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_packet_timeout"))
}
