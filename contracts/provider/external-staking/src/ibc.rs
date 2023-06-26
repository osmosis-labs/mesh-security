#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse, IbcChannel,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, IbcTimeout,
};
use cw_storage_plus::Item;
use mesh_apis::ibc::{
    ack_success, validate_channel_order, AckWrapper, AddValidator, AddValidatorsAck,
    ConsumerPacket, ProtocolVersion, ProviderPacket, RemoveValidatorsAck,
};

use crate::contract::ExternalStakingContract;
use crate::crdt::ValUpdate;
use crate::error::ContractError;
use crate::msg::AuthorizedEndpoint;

/// This is the maximum version of the Mesh Security protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "0.10.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "0.10.0";

// IBC specific state
pub const AUTH_ENDPOINT: Item<AuthorizedEndpoint> = Item::new("auth_endpoint");
pub const IBC_CHANNEL: Item<IbcChannel> = Item::new("ibc_channel");

// If we don't hear anything within 10 minutes, let's abort, for better UX
// This is long enough to allow some clock drift between chains
const DEFAULT_TIMEOUT: u64 = 10 * 60;

pub fn packet_timeout(env: &Env) -> IbcTimeout {
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
    let (channel, counterparty_version) = match msg {
        IbcChannelOpenMsg::OpenInit { .. } => return Err(ContractError::IbcOpenInitDisallowed),
        IbcChannelOpenMsg::OpenTry {
            channel,
            counterparty_version,
        } => (channel, counterparty_version),
    };

    // verify the ordering is correct
    validate_channel_order(&channel.order)?;

    // assert expected endpoint
    let authorized = AUTH_ENDPOINT.load(deps.storage)?;
    if authorized.connection_id != channel.connection_id
        || authorized.port_id != channel.counterparty_endpoint.port_id
    {
        // FIXME: do we need a better error here?
        return Err(ContractError::Unauthorized);
    }

    // we handshake with the counterparty version, it must not be empty
    let v: ProtocolVersion = from_slice(counterparty_version.as_bytes())?;
    // if we can build a response to this, then it is compatible. And we use the highest version there
    let version = v.build_response(SUPPORTED_IBC_PROTOCOL_VERSION, MIN_IBC_PROTOCOL_VERSION)?;

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
    // ensure we have no channel yet
    if IBC_CHANNEL.may_load(deps.storage)?.is_some() {
        return Err(ContractError::IbcChannelAlreadyOpen);
    }
    // ensure we are called with OpenConfirm
    let channel = match msg {
        IbcChannelConnectMsg::OpenConfirm { channel } => channel,
        IbcChannelConnectMsg::OpenAck { .. } => return Err(ContractError::IbcOpenInitDisallowed),
    };

    // Version negotiation over, we can only store the channel
    IBC_CHANNEL.save(deps.storage, &channel)?;

    Ok(IbcBasicResponse::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcChannelCloseMsg,
) -> Result<IbcBasicResponse, ContractError> {
    todo!();
}

#[cfg_attr(not(feature = "library"), entry_point)]
// this accepts validator sync packets and updates the crdt state
pub fn ibc_packet_receive(
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    // There is only one channel, so we don't need to switch.
    // We also don't care about packet sequence as this is fully commutative.
    let contract = ExternalStakingContract::new();
    let packet: ConsumerPacket = from_slice(&msg.packet.data)?;
    let ack = match packet {
        ConsumerPacket::AddValidators(to_add) => {
            for AddValidator {
                valoper,
                pub_key,
                start_height,
                start_time,
            } in to_add
            {
                let update = ValUpdate {
                    pub_key,
                    start_height,
                    start_time,
                };
                contract
                    .val_set
                    .add_validator(deps.storage, &valoper, update)?;
            }
            ack_success(&AddValidatorsAck {})?
        }
        ConsumerPacket::RemoveValidators(to_remove) => {
            for valoper in to_remove {
                contract.val_set.remove_validator(deps.storage, &valoper)?;
            }
            ack_success(&RemoveValidatorsAck {})?
        }
    };

    // return empty success ack
    Ok(IbcReceiveResponse::new().set_ack(ack))
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// never should be called as we do not send packets
pub fn ibc_packet_ack(
    deps: DepsMut,
    env: Env,
    msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let packet: ProviderPacket = from_slice(&msg.original_packet.data)?;
    let contract = ExternalStakingContract::new();
    let ack: AckWrapper = from_slice(&msg.acknowledgement.data)?;
    let mut resp = IbcBasicResponse::new();

    match (packet, ack) {
        (ProviderPacket::Stake { tx_id, .. }, AckWrapper::Result(_)) => {
            let msg = contract.commit_stake(deps, tx_id)?;
            resp = resp
                .add_message(msg)
                .add_attribute("success", "true")
                .add_attribute("tx_id", tx_id.to_string());
        }
        (ProviderPacket::Stake { tx_id, .. }, AckWrapper::Error(e)) => {
            let msg = contract.rollback_stake(deps, tx_id)?;
            resp = resp
                .add_message(msg)
                .add_attribute("error", e)
                .add_attribute("tx_id", tx_id.to_string());
        }
        (ProviderPacket::Unstake { tx_id, .. }, AckWrapper::Result(_)) => {
            contract.commit_unstake(deps, env, tx_id)?;
            resp = resp
                .add_attribute("success", "true")
                .add_attribute("tx_id", tx_id.to_string());
        }
        (ProviderPacket::Unstake { tx_id, .. }, AckWrapper::Error(e)) => {
            contract.rollback_unstake(deps, tx_id)?;
            resp = resp
                .add_attribute("error", e)
                .add_attribute("tx_id", tx_id.to_string());
        }
        (ProviderPacket::TransferRewards { .. }, AckWrapper::Result(_)) => {
            // do nothing, funds already transferred
        }
        (
            ProviderPacket::TransferRewards {
                rewards, staker, ..
            },
            AckWrapper::Error(e),
        ) => {
            // TODO: rollback the transfer by reducing the withdrawn amount for this staker
            let _ = (rewards, staker);
            resp = resp
                .add_attribute("error", e)
                .add_attribute("packet", msg.original_packet.sequence.to_string());
        }
    }

    // Question: do we need a special event with all this info on error?

    //         // Provide info to find the actual packet.
    //         let event = Event::new("mesh_ibc_error")
    //             .add_attribute("error", e)
    //             .add_attribute("channel", msg.original_packet.src.channel_id)
    //             .add_attribute("sequence", msg.original_packet.sequence.to_string());
    //         resp = resp.add_event(event);
    //     }
    Ok(resp)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// This should trigger a rollback of staking/unstaking
pub fn ibc_packet_timeout(
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let packet: ProviderPacket = from_slice(&msg.packet.data)?;
    let contract = ExternalStakingContract::new();
    let mut resp = IbcBasicResponse::new().add_attribute("action", "ibc_packet_timeout");
    match packet {
        ProviderPacket::Stake { tx_id, .. } => {
            let msg = contract.rollback_stake(deps, tx_id)?;
            resp = resp
                .add_message(msg)
                .add_attribute("tx_id", tx_id.to_string());
        }
        ProviderPacket::Unstake { tx_id, .. } => {
            contract.rollback_unstake(deps, tx_id)?;
            resp = resp.add_attribute("tx_id", tx_id.to_string());
        }
        ProviderPacket::TransferRewards {
            rewards, staker, ..
        } => {
            // TODO: rollback the transfer by reducing the withdrawn amount for this staker
            let _ = (rewards, staker);
            resp = resp.add_attribute("packet", msg.packet.sequence.to_string());
        }
    };
    Ok(resp)
}
