#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_json, Decimal, DepsMut, Env, Ibc3ChannelOpenResponse, IbcBasicResponse, IbcChannelCloseMsg, IbcChannelConnectMsg, IbcChannelOpenMsg, IbcChannelOpenResponse, IbcOrder, IbcPacket, IbcPacketAckMsg, IbcPacketReceiveMsg, IbcPacketTimeoutMsg, IbcReceiveResponse, StdError, Uint128
};
use cw_band::{OracleResponsePacketData, Output, ResolveStatus};
use mesh_apis::ibc::{
    ack_fail, ack_success, validate_channel_order, PriceFeedAck, ProtocolVersion,
};
use obi::OBIDecode;

use crate::contract::RemotePriceFeedContract;
use crate::error::ContractError;

pub const IBC_APP_VERSION: &str = "bandchain-1";

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
    let channel = msg.channel();
    let counterparty_version = msg.counterparty_version();
    
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
    let counterparty_version = msg.counterparty_version();
    
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
    unimplemented!();
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
    // We ignore acknowledgement from BandChain becuase it doesn't neccessary to know request id when handle result.
    Ok(IbcBasicResponse::new().add_attribute("action", "ibc_packet_ack"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn ibc_packet_timeout(
    _deps: DepsMut,
    _env: Env,
    _msg: IbcPacketTimeoutMsg,
) -> Result<IbcBasicResponse, ContractError> {
    Err(ContractError::IbcTimeoutNotAccepted)
}
