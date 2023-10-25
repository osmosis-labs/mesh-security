#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_slice, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse, IbcChannel,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, IbcTimeout,
};
use cw_storage_plus::Item;
use mesh_apis::ibc::{
    ack_success, validate_channel_order, AckWrapper, ConsumerPacket, DistributeAck,
    ProtocolVersion, ProviderPacket, ValsetUpdateAck,
};

use crate::contract::ExternalStakingContract;
use crate::error::ContractError;
use crate::msg::AuthorizedEndpoint;

/// This is the maximum version of the Mesh Security protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "0.11.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "0.11.0";

// IBC specific state
pub const AUTH_ENDPOINT: Item<AuthorizedEndpoint> = Item::new("auth_endpoint");
pub const IBC_CHANNEL: Item<IbcChannel> = Item::new("ibc_channel");

// If we don't hear anything within 10 minutes, let's abort, for better UX
// This is long enough to allow some clock drift between chains
const DEFAULT_TIMEOUT: u64 = 10 * 60;

pub fn packet_timeout(env: &Env) -> IbcTimeout {
    // No idea about their block time, but 24 hours ahead of our view of the clock
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
    env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    // There is only one channel, so we don't need to switch.
    // We also don't care about packet sequence as this is being ordered by height.
    // If a validator is in more than one of the events, the end result will depend on the
    // processing order below.
    let contract = ExternalStakingContract::new();
    let packet: ConsumerPacket = from_slice(&msg.packet.data)?;
    let resp = match packet {
        ConsumerPacket::ValsetUpdate {
            height,
            time,
            additions,
            removals,
            updated,
            jailed,
            unjailed,
            tombstoned,
        } => {
            let (evt, msgs) = contract.valset_update(
                deps,
                env,
                height,
                time,
                &additions,
                &removals,
                &updated,
                &jailed,
                &unjailed,
                &tombstoned,
            )?;
            let ack = ack_success(&ValsetUpdateAck {})?;
            IbcReceiveResponse::new()
                .set_ack(ack)
                .add_event(evt)
                .add_messages(msgs)
        }
        ConsumerPacket::Distribute { validator, rewards } => {
            let evt = contract.distribute_rewards(deps, &validator, rewards)?;
            let ack = ack_success(&DistributeAck {})?;
            IbcReceiveResponse::new().set_ack(ack).add_event(evt)
        }
        ConsumerPacket::DistributeBatch { rewards, denom } => {
            let evts = contract.distribute_rewards_batch(deps, &rewards, &denom)?;
            let ack = ack_success(&DistributeAck {})?;
            IbcReceiveResponse::new().set_ack(ack).add_events(evts)
        }
    };

    // return empty success ack
    Ok(resp)
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
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "stake");
        }
        (ProviderPacket::Stake { tx_id, .. }, AckWrapper::Error(e)) => {
            let msg = contract.rollback_stake(deps, tx_id)?;
            resp = resp
                .add_message(msg)
                .add_attribute("error", e)
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "stake");
        }
        (ProviderPacket::Unstake { tx_id, .. }, AckWrapper::Result(_)) => {
            contract.commit_unstake(deps, env, tx_id)?;
            resp = resp
                .add_attribute("success", "true")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "unstake");
        }
        (ProviderPacket::Unstake { tx_id, .. }, AckWrapper::Error(e)) => {
            contract.rollback_unstake(deps, tx_id)?;
            resp = resp
                .add_attribute("error", e)
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "unstake");
        }
        (ProviderPacket::Burn { tx_id, .. }, AckWrapper::Result(_)) => {
            contract.commit_burn(deps, tx_id)?;
            resp = resp
                .add_attribute("success", "true")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "burn");
        }
        (ProviderPacket::Burn { tx_id, .. }, AckWrapper::Error(e)) => {
            contract.rollback_burn(deps, tx_id)?;
            resp = resp
                .add_attribute("error", e)
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "burn");
        }
        (ProviderPacket::TransferRewards { tx_id, .. }, AckWrapper::Result(_)) => {
            contract.commit_withdraw_rewards(deps, tx_id)?;
            resp = resp
                .add_attribute("success", "true")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "transfer_rewards");
        }
        (ProviderPacket::TransferRewards { tx_id, .. }, AckWrapper::Error(e)) => {
            contract.rollback_withdraw_rewards(deps, tx_id)?;
            resp = resp
                .add_attribute("error", e)
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "transfer_rewards");
        }
    }
    Ok(resp)
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// This should trigger a rollback of staking/unstaking/burning
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
                .add_attribute("error", "timeout")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "stake");
        }
        ProviderPacket::Unstake { tx_id, .. } => {
            contract.rollback_unstake(deps, tx_id)?;
            resp = resp
                .add_attribute("error", "timeout")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "unstake");
        }
        ProviderPacket::Burn { tx_id, .. } => {
            contract.rollback_burn(deps, tx_id)?;
            resp = resp
                .add_attribute("error", "timeout")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "burn");
        }
        ProviderPacket::TransferRewards { tx_id, .. } => {
            contract.rollback_withdraw_rewards(deps, tx_id)?;
            resp = resp
                .add_attribute("error", "timeout")
                .add_attribute("tx_id", tx_id.to_string())
                .add_attribute("tx_type", "transfer_rewards");
        }
    };
    Ok(resp)
}
