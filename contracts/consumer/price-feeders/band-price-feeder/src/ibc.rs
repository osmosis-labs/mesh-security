#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    attr, from_slice, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse, IbcChannel,
    IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse, IbcOrder,
    IbcPacket, IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse,
    StdError, StdResult, Uint64,
};

use crate::error::{ContractError, Never};
use crate::state::{Rate, ENDPOINT, RATES};
use obi::dec::OBIDecode;

use cw_band::{
    ack_fail, ack_success, OracleResponsePacketData, Output, ResolveStatus, IBC_APP_VERSION,
};

#[cfg_attr(not(feature = "library"), entry_point)]
/// enforces ordering and versioning constraints
pub fn ibc_channel_open(
    _deps: DepsMut,
    _env: Env,
    msg: IbcChannelOpenMsg,
) -> Result<IbcChannelOpenResponse, ContractError> {
    enforce_order_and_version(msg.channel(), msg.counterparty_version())?;

    Ok(Some(Ibc3ChannelOpenResponse {
        version: msg.channel().version.clone(),
    }))
}

#[cfg_attr(not(feature = "library"), entry_point)]
/// record the channel in ENDPOINT
pub fn ibc_channel_connect(
    deps: DepsMut,
    _env: Env,
    msg: IbcChannelConnectMsg,
) -> Result<IbcBasicResponse, ContractError> {
    // we need to check the counter party version in try and ack (sometimes here)
    enforce_order_and_version(msg.channel(), msg.counterparty_version())?;

    ENDPOINT.save(deps.storage, &msg.channel().endpoint)?;
    Ok(IbcBasicResponse::default())
}

fn enforce_order_and_version(
    channel: &IbcChannel,
    counterparty_version: Option<&str>,
) -> Result<(), ContractError> {
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
    if channel.order != IbcOrder::Unordered {
        return Err(ContractError::OnlyUnorderedChannel {});
    }
    Ok(())
}

#[entry_point]
pub fn ibc_channel_close(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcChannelCloseMsg,
) -> StdResult<IbcBasicResponse> {
    unimplemented!();
}

#[entry_point]
pub fn ibc_packet_receive(
    deps: DepsMut,
    _env: Env,
    msg: IbcPacketReceiveMsg,
) -> Result<IbcReceiveResponse, Never> {
    let packet = msg.packet;

    do_ibc_packet_receive(deps, &packet).or_else(|err| {
        Ok(IbcReceiveResponse::new()
            .set_ack(ack_fail(err.to_string()))
            .add_attributes(vec![
                attr("action", "receive"),
                attr("success", "false"),
                attr("error", err.to_string()),
            ]))
    })
}

fn do_ibc_packet_receive(
    deps: DepsMut,
    packet: &IbcPacket,
) -> Result<IbcReceiveResponse, ContractError> {
    let resp: OracleResponsePacketData = from_slice(&packet.data)?;
    if resp.resolve_status != ResolveStatus::Success {
        return Err(ContractError::RequestNotSuccess {});
    }
    let result: Output =
        OBIDecode::try_from_slice(&resp.result).map_err(|err| StdError::ParseErr {
            target_type: "Oracle response packet".into(),
            msg: err.to_string(),
        })?;

    for r in result.responses {
        if r.response_code == 0 {
            let rate = RATES.may_load(deps.storage, &r.symbol)?;
            if rate.is_none() || rate.unwrap().resolve_time < resp.resolve_time {
                RATES.save(
                    deps.storage,
                    &r.symbol,
                    &Rate {
                        rate: Uint64::from(r.rate),
                        resolve_time: resp.resolve_time,
                        request_id: resp.request_id,
                    },
                )?;
            }
        }
    }
    Ok(IbcReceiveResponse::new()
        .set_ack(ack_success())
        .add_attribute("action", "ibc_packet_received"))
}

#[entry_point]
pub fn ibc_packet_ack(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketAckMsg,
) -> StdResult<IbcBasicResponse> {
    // We ignore acknowledgement from BandChain becuase it doesn't neccessary to know request id when handle result.
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_packet_ack"))
}

#[entry_point]
/// TODO: Handle when didn't get response packet in time
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> StdResult<IbcBasicResponse> {
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_packet_timeout"))
}
