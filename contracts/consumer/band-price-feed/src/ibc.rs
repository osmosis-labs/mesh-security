#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_json, Decimal, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse, IbcPacket,
    IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, StdError,
    Uint128,
};
use cw_band::{OracleResponsePacketData, Output, ResolveStatus};
use mesh_apis::ibc::{
    ack_fail, ack_success, validate_channel_order, PriceFeedAck, ProtocolVersion,
};
use obi::OBIDecode;

use crate::contract::RemotePriceFeedContract;
use crate::error::ContractError;

/// This is the maximum version of the Mesh Security protocol that we support
const SUPPORTED_IBC_PROTOCOL_VERSION: &str = "0.1.0";
/// This is the minimum version that we are compatible with
const MIN_IBC_PROTOCOL_VERSION: &str = "0.1.0";

#[cfg_attr(not(feature = "library"), entry_point)]
/// enforces ordering and versioning constraints
pub fn ibc_channel_open(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelOpenMsg,
) -> Result<IbcChannelOpenResponse, ContractError> {
    // ensure we have no channel yet
    let contract = RemotePriceFeedContract::new();
    if contract.channel.may_load(deps.storage)?.is_some() {
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
    let config = contract.config.load(deps.storage)?;
    if config.connection_id != channel.connection_id
        || config.endpoint.port_id != channel.counterparty_endpoint.port_id
    {
        // FIXME: do we need a better error here?
        return Err(ContractError::Unauthorized);
    }

    // we handshake with the counterparty version, it must not be empty
    let v: ProtocolVersion = from_json(counterparty_version.as_bytes())?;
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
    let contract = RemotePriceFeedContract::new();

    // ensure we have no channel yet
    if contract.channel.may_load(deps.storage)?.is_some() {
        return Err(ContractError::IbcChannelAlreadyOpen);
    }
    // ensure we are called with OpenConfirm
    let channel = match msg {
        IbcChannelConnectMsg::OpenConfirm { channel } => channel,
        IbcChannelConnectMsg::OpenAck { .. } => return Err(ContractError::IbcOpenInitDisallowed),
    };

    // Version negotiation over, we can only store the channel
    let contract = RemotePriceFeedContract::new();
    contract.channel.save(deps.storage, &channel)?;

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
pub fn ibc_packet_receive(
    deps: DepsMut,
    env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    let packet = msg.packet;

    do_ibc_packet_receive(deps, env, &packet).or_else(|err| {
        let error = err.to_string();
        let ack_fail = ack_fail(err)?;
        Ok(IbcReceiveResponse::new()
            .set_ack(ack_fail)
            .add_attributes(vec![
                ("action", "receive"),
                ("success", "false"),
                ("error", &error),
            ]))
    })
}

fn do_ibc_packet_receive(
    deps: DepsMut,
    env: Env,
    packet: &IbcPacket,
) -> Result<IbcReceiveResponse, ContractError> {
    let contract = RemotePriceFeedContract::new();

    let resp: OracleResponsePacketData = from_json(&packet.data)?;
    if resp.resolve_status != ResolveStatus::Success {
        return Err(ContractError::RequestNotSuccess {});
    }
    let result: Output = OBIDecode::try_from_slice(&resp.result)
        .map_err(|err| StdError::parse_err("Oracle response packet", err.to_string()))?;

    let trading_pair = contract.trading_pair.load(deps.storage)?;
    let mut base_price = Uint128::zero();
    let mut quote_price = Uint128::zero();

    if result.responses.len() != 2 {
        return Err(ContractError::InvalidResponsePacket {});
    }
    for r in result.responses {
        if r.response_code == 0 {
            if r.symbol == trading_pair.base_asset {
                base_price = Uint128::from(r.rate);
            } else if r.symbol == trading_pair.quote_asset {
                quote_price = Uint128::from(r.rate);
            } else {
                return Err(ContractError::SymbolsNotMatch {});
            }
        }
    }
    if base_price.is_zero() || quote_price.is_zero() {
        return Err(ContractError::InvalidPrice {});
    }

    let rate = Decimal::from_ratio(base_price, quote_price);
    contract.price_keeper.update(deps, env.block.time, rate)?;
    let ack = ack_success(&PriceFeedAck {})?;
    Ok(IbcReceiveResponse::new()
        .set_ack(ack)
        .add_attribute("action", "ibc_packet_received"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcAckNotAccepted)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcTimeoutNotAccepted)
}
