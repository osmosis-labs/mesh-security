#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, to_binary, DepsMut, Env, Event, Ibc3ChannelOpenResponse, IbcBasicResponse,
    IbcChannel, IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg,
    IbcChannelOpenResponse, IbcMsg, IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg,
    IbcReceiveResponse, IbcTimeout,
};
use cw_storage_plus::Item;

use mesh_apis::ibc::{
    ack_success, validate_channel_order, AckWrapper, AddValidator, ConsumerPacket, ProtocolVersion,
    ProviderPacket, StakeAck, UnstakeAck, PROTOCOL_NAME,
};

use crate::{contract::ConverterContract, error::ContractError};

/// This is the maximum version of the Mesh Security protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "1.0.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "1.0.0";

// IBC specific state
const IBC_CHANNEL: Item<IbcChannel> = Item::new("ibc_channel");

// Let those validator syncs take a day...
const DEFAULT_TIMEOUT: u64 = 24 * 60 * 60;

fn packet_timeout(env: &Env) -> IbcTimeout {
    // No idea about their blocktime, but 24 hours ahead of our view of the clock
    // should be decently in the future.
    let timeout = env.block.time.plus_seconds(DEFAULT_TIMEOUT);
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

    // store the channel
    IBC_CHANNEL.save(deps.storage, &channel)?;

    // Send a validator sync packet to arrive with the newly established channel
    let validators = deps.querier.query_all_validators()?;
    let updates = validators
        .into_iter()
        .map(|v| AddValidator {
            valoper: v.address,
            // TODO: not yet available in CosmWasm APIs
            pub_key: "TODO".to_string(),
            // Use current height/time as start height/time (no slashing before mesh starts)
            start_height: env.block.height,
            start_time: env.block.time.seconds(),
        })
        .collect();
    let packet = ConsumerPacket::AddValidators(updates);
    let msg = IbcMsg::SendPacket {
        channel_id: channel.endpoint.channel_id,
        data: to_binary(&packet)?,
        timeout: packet_timeout(&env),
    };

    Ok(IbcBasicResponse::new().add_message(msg))
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
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    let packet: ProviderPacket = from_slice(&msg.packet.data)?;
    let contract = ConverterContract::new();
    let res = match packet {
        ProviderPacket::Stake {
            validator,
            stake,
            tx_id,
        } => {
            let response = contract.stake(deps, validator, stake)?;
            let ack = ack_success(&StakeAck { tx_id })?;
            IbcReceiveResponse::new()
                .set_ack(ack)
                .add_submessages(response.messages)
                .add_events(response.events)
                .add_attributes(response.attributes)
        }
        ProviderPacket::Unstake {
            validator,
            unstake,
            tx_id,
        } => {
            let response = contract.unstake(deps, validator, unstake)?;
            let ack = ack_success(&UnstakeAck { tx_id })?;
            IbcReceiveResponse::new()
                .set_ack(ack)
                .add_submessages(response.messages)
                .add_events(response.events)
                .add_attributes(response.attributes)
        }
    };
    Ok(res)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// We get acks on sync state without much to do.
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
            let event = Event::new("mesh_ibc_error")
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
