use std::str::FromStr;

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_json, to_json_binary, Decimal, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse,
    IbcChannel, IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg,
    IbcChannelOpenResponse, IbcMsg, IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg,
    IbcReceiveResponse, IbcTimeout, Timestamp,
};
use mesh_apis::ibc::{
    decode_response, decode_twap_response, validate_channel_order, AcknowledgementResult,
    InterchainQueryPacketAck, InterchainQueryPacketData,
};

use crate::contract::RemotePriceFeedContract;
use crate::error::ContractError;

pub const IBC_APP_VERSION: &str = "icq-1";

const TIMEOUT: u64 = 10 * 60;

pub fn packet_timeout(now: &Timestamp) -> IbcTimeout {
    let timeout = now.plus_seconds(TIMEOUT);
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
    let contract = RemotePriceFeedContract::new();
    if contract.channel.may_load(deps.storage)?.is_some() {
        return Err(ContractError::IbcChannelAlreadyOpen);
    }
    let channel = msg.channel();
    let counterparty_version = msg.counterparty_version();

    // verify the ordering is correct
    validate_channel_order(&channel.order)?;
    if channel.version != IBC_APP_VERSION {
        return Err(ContractError::InvalidIbcVersion {
            version: channel.version.clone(),
        });
    }
    if let Some(version) = counterparty_version {
        if version != IBC_APP_VERSION {
            return Err(ContractError::InvalidIbcVersion {
                version: version.to_string(),
            });
        }
    }
    let response = Ibc3ChannelOpenResponse {
        version: channel.version.clone(),
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
    let channel = msg.channel();

    // Version negotiation over, we can only store the channel
    let contract = RemotePriceFeedContract::new();
    contract.channel.save(deps.storage, channel)?;

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
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, ContractError> {
    Err(ContractError::IbcReceiveNotAccepted)
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_ack(
    deps: DepsMut,
    env: Env,
    msg: IbcPacketAckMsg,
) -> Result<IbcBasicResponse, ContractError> {
    let ack_result: AcknowledgementResult = from_json(msg.acknowledgement.data)?;
    let packet_ack: InterchainQueryPacketAck = from_json(ack_result.result)?;

    let responses = decode_response(&packet_ack.data)?.responses;
    if responses.len() != 1 {
        return Err(ContractError::InvalidResponseQuery);
    }

    let response = responses[0].clone();
    if response.code != 0 {
        return Err(ContractError::InvalidResponseQueryCode);
    }

    if response.key.is_empty() {
        return Err(ContractError::EmptyTwap);
    }

    let twap_response: mesh_apis::ibc::QueryArithmeticTwapToNowResponse =
        decode_twap_response(&response.key)?;
    let twap_price: Decimal = Decimal::from_str(&twap_response.arithmetic_twap)?;

    let contract = RemotePriceFeedContract::new();
    contract.update_twap(deps, env.block.time, twap_price)?;

    Ok(IbcBasicResponse::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcReceiveNotAccepted)
}

pub(crate) fn make_ibc_packet(
    now: &Timestamp,
    channel: IbcChannel,
    packet: InterchainQueryPacketData,
) -> Result<IbcMsg, ContractError> {
    Ok(IbcMsg::SendPacket {
        channel_id: channel.endpoint.channel_id,
        data: to_json_binary(&packet)?,
        timeout: packet_timeout(now),
    })
}
