#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_binary, Binary, Deps, DepsMut, Empty, Env, IbcMsg, IbcTimeout, MessageInfo, Response,
    StdResult, Uint256, Uint64,
};
use cw2::set_contract_version;

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use crate::state::{Config, Rate, ReferenceData, BAND_CONFIG, ENDPOINT, RATES};
use obi::enc::OBIEncode;

use cw_band::{Input, OracleRequestPacketData};

// WARNING /////////////////////////////////////////////////////////////////////////
// THIS CONTRACT IS AN EXAMPLE HOW TO USE CW_BAND TO WRITE CONTRACT.              //
// PLEASE USE THIS CODE AS THE REFERENCE AND NOT USE THIS CODE IN PRODUCTION.     //
////////////////////////////////////////////////////////////////////////////////////

const E9: Uint64 = Uint64::new(1_000_000_000u64);
const E18: Uint256 = Uint256::from_u128(1_000_000_000_000_000_000u128);

// Version info for migration
const CONTRACT_NAME: &str = "crates.io:band-ibc-price-feed";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    BAND_CONFIG.save(
        deps.storage,
        &Config {
            client_id: msg.client_id,
            oracle_script_id: msg.oracle_script_id,
            ask_count: msg.ask_count,
            min_count: msg.min_count,
            fee_limit: msg.fee_limit,
            prepare_gas: msg.prepare_gas,
            execute_gas: msg.execute_gas,
            minimum_sources: msg.minimum_sources,
        },
    )?;

    Ok(Response::new().add_attribute("method", "instantiate"))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Request { symbols } => try_request(deps, env, symbols),
    }
}

// TODO: Possible features
// - Request fee + Bounty logic to prevent request spam and incentivize relayer
// - Whitelist who can call update price
pub fn try_request(
    deps: DepsMut,
    env: Env,
    symbols: Vec<String>,
) -> Result<Response, ContractError> {
    let endpoint = ENDPOINT.load(deps.storage)?;
    let config = BAND_CONFIG.load(deps.storage)?;

    let raw_calldata = Input {
        symbols,
        minimum_sources: config.minimum_sources,
    }
    .try_to_vec()
    .map(Binary)
    .map_err(|err| ContractError::CustomError {
        val: err.to_string(),
    })?;

    let packet = OracleRequestPacketData {
        client_id: config.client_id,
        oracle_script_id: config.oracle_script_id,
        calldata: raw_calldata,
        ask_count: config.ask_count,
        min_count: config.min_count,
        prepare_gas: config.prepare_gas,
        execute_gas: config.execute_gas,
        fee_limit: config.fee_limit,
    };

    Ok(Response::new().add_message(IbcMsg::SendPacket {
        channel_id: endpoint.channel_id,
        data: to_binary(&packet)?,
        timeout: IbcTimeout::with_timestamp(env.block.time.plus_seconds(60)),
    }))
}

/// this is a no-op
#[cfg_attr(not(feature = "library"), entry_point)]
pub fn migrate(_deps: DepsMut, _env: Env, _msg: Empty) -> StdResult<Response> {
    Ok(Response::default())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetRate { symbol } => to_binary(&query_rate(deps, &symbol)?),
        QueryMsg::GetReferenceData { symbol_pair } => {
            to_binary(&query_reference_data(deps, &symbol_pair)?)
        }
        QueryMsg::GetReferenceDataBulk { symbol_pairs } => {
            to_binary(&query_reference_data_bulk(deps, &symbol_pairs)?)
        }
    }
}

fn query_rate(deps: Deps, symbol: &str) -> StdResult<Rate> {
    if symbol == "USD" {
        Ok(Rate::new(E9, Uint64::MAX, Uint64::new(0)))
    } else {
        RATES.load(deps.storage, symbol)
    }
}

fn query_reference_data(deps: Deps, symbol_pair: &(String, String)) -> StdResult<ReferenceData> {
    let base = query_rate(deps, &symbol_pair.0)?;
    let quote = query_rate(deps, &symbol_pair.1)?;

    Ok(ReferenceData::new(
        Uint256::from(base.rate)
            .checked_mul(E18)?
            .checked_div(Uint256::from(quote.rate))?,
        base.resolve_time,
        quote.resolve_time,
    ))
}

fn query_reference_data_bulk(
    deps: Deps,
    symbol_pairs: &[(String, String)],
) -> StdResult<Vec<ReferenceData>> {
    symbol_pairs
        .iter()
        .map(|pair| query_reference_data(deps, pair))
        .collect()
}

// TODO: Writing test
#[cfg(test)]
mod tests {}
