use crate::error::ContractError;
use crate::response::MsgInstantiateContractResponse;
use crate::state::MOON_CONFIG;
use crate::util;
use classic_terraswap::querier::{
    query_balance, query_pair_info, query_token_balance
};

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_binary, to_binary, Addr, Binary, CanonicalAddr, CosmosMsg, Decimal, Decimal256, Deps,
    DepsMut, Env, MessageInfo, Reply, ReplyOn, Response, StdError, StdResult, SubMsg, Uint128,
    Uint256, WasmMsg, WasmQuery
};

use classic_bindings::{TerraMsg, TerraQuery};

use classic_terraswap::asset::{Asset, AssetInfo, MoonInfo, MoonInfoRaw, VestInfo, VestInfoRaw};
use classic_terraswap::moon::{
    Cw20HookMsg, MoonExecuteMsg, InstantiateMsg, MigrateMsg, PoolResponse,
    ReverseSimulationResponse, SimulationResponse,
};
use classic_terraswap::querier::query_token_info;
use classic_terraswap::token::InstantiateMsg as TokenInstantiateMsg;
use classic_terraswap::util::{assert_deadline, migrate_version};
use cw2::set_contract_version;
use cw20::{Cw20ExecuteMsg, Cw20ReceiveMsg, Denom, MinterResponse};
use protobuf::Message;
use std::cmp::Ordering;
use std::convert::TryInto;
use std::ops::Mul;

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:terraswap-moon";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const INSTANTIATE_REPLY_ID: u64 = 1;

/// Commission rate == 0.3%
const COMMISSION_RATE: u64 = 3;

const MINIMUM_LIQUIDITY_AMOUNT: u128 = 1_000;

const BURN_ADDRESS: &str = "terra1sk06e3dyexuq4shw77y3dsv480xv42mq73anxu";

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut<TerraQuery>,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> StdResult<Response<TerraMsg>> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    let pair_vesting: VestInfoRaw = VestInfoRaw {
        address: deps
            .api
            .addr_canonicalize(&msg.pair_vest.address.as_str())?,
        monthly_amount: msg.pair_vest.monthly_amount,
        month_count: msg.pair_vest.month_count,
        month_index: Uint128::zero(),
    };
    let nft_vesting: VestInfoRaw = VestInfoRaw {
        address: deps.api.addr_canonicalize(&msg.nft_vest.address.as_str())?,
        monthly_amount: msg.nft_vest.monthly_amount,
        month_count: msg.nft_vest.month_count,
        month_index: Uint128::zero(),
    };
    let marketing_vesting: VestInfoRaw = VestInfoRaw {
        address: deps
            .api
            .addr_canonicalize(&msg.marketing_vest.address.as_str())?,
        monthly_amount: msg.marketing_vest.monthly_amount,
        month_count: msg.marketing_vest.month_count,
        month_index: Uint128::zero(),
    };
    let game_vesting: VestInfoRaw = VestInfoRaw {
        address: deps
            .api
            .addr_canonicalize(&msg.game_vest.address.as_str())?,
        monthly_amount: msg.game_vest.monthly_amount,
        month_count: msg.game_vest.month_count,
        month_index: Uint128::zero(),
    };
    let team_vesting: VestInfoRaw = VestInfoRaw {
        address: deps
            .api
            .addr_canonicalize(&msg.team_vest.address.as_str())?,
        monthly_amount: msg.team_vest.monthly_amount,
        month_count: msg.team_vest.month_count,
        month_index: Uint128::zero(),
    };

    let moon_config: &MoonInfoRaw = &MoonInfoRaw {
        clsm_addr: deps.api.addr_canonicalize(&msg.clsm_addr.as_str())?,
        pair_vest: pair_vesting,
        nft_vest: nft_vesting,
        marketing_vest: marketing_vesting,
        game_vest: game_vesting,
        team_vest: team_vesting,
    };

    MOON_CONFIG.save(deps.storage, moon_config)?;
    Ok(Response::new())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
    msg: MoonExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        MoonExecuteMsg::VestingMint {} => vesting_mint(deps, env, info),
        MoonExecuteMsg::DynamicMintFromLunc { amount, price } => lunc_dynamic_mint(deps, &env, info, amount, price),
        MoonExecuteMsg::DynamicMintFromUstc { amount, price } => ustc_dynamic_mint(deps, &env, info, amount, price),
    }
}

pub fn vesting_mint(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut messages: Vec<CosmosMsg> = vec![];

    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    messages.push(emission2pair_contract(&deps, &env, &info, & mut moon_config)?);
    messages.push(emission2nft_minter(&deps, &env, &info, & mut moon_config)?);
    messages.push(emission2marketing(&deps, &env, &info, & mut moon_config)?);
    messages.push(emission2minigames(&deps, &env, &info, & mut moon_config)?);
    messages.push(emission2team(&deps, &env, &info, & mut moon_config)?);

    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn emission2pair_contract(
    deps: &DepsMut<TerraQuery>,
    env: &Env,
    info: &MessageInfo,
    moon_config:& mut MoonInfoRaw,
) -> Result<CosmosMsg, ContractError> {
    let clsm_addr = moon_config.clsm_addr.clone();
    let pair_contract_address = moon_config.pair_vest.address.clone();
    let pair_contract_monthly_amount = moon_config.pair_vest.monthly_amount;
    let pair_contract_month_count = moon_config.pair_vest.month_count;
    let pair_contract_month_index = moon_config.pair_vest.month_index;

    if pair_contract_month_index >= pair_contract_month_count {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_amount = query_token_balance(
        &deps.as_ref().querier,
        deps.api.addr_humanize(&clsm_addr)?,
        Addr::unchecked(env.contract.address.as_str()),
    )?;

    if clsm_amount < pair_contract_monthly_amount {
        return Err(ContractError::LessThanVesting {});
    }

    moon_config.pair_vest.month_index = pair_contract_month_index + Uint128::from(1 as u8);

    Ok(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        pair_contract_monthly_amount,
        deps.api.addr_humanize(&pair_contract_address)?,
    )?)
}

pub fn emission2nft_minter(
    deps: &DepsMut<TerraQuery>,
    env: &Env,
    info: &MessageInfo,
    moon_config: & mut MoonInfoRaw,
) -> Result<CosmosMsg, ContractError> {
    let clsm_addr = moon_config.clsm_addr.clone();
    let nft_minter_address = moon_config.nft_vest.address.clone();
    let nft_minter_monthly_amount = moon_config.nft_vest.monthly_amount;
    let nft_minter_month_count = moon_config.nft_vest.month_count;
    let mut nft_minter_month_index = moon_config.nft_vest.month_index;

    if nft_minter_month_index >= nft_minter_month_count {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_amount = query_token_balance(
        &deps.as_ref().querier,
        deps.api.addr_humanize(&clsm_addr)?,
        Addr::unchecked(env.contract.address.as_str()),
    )?;

    if clsm_amount < nft_minter_monthly_amount {
        return Err(ContractError::LessThanVesting {});
    }

    // let all_nfts = WasmQuery::Smart {
    //     contract_addr: nft_minter_address,
    //     msg: to_binary(&QueryMsg::AllNftInfo { start_from: None, limit: None })?,
    // };
    // let all_nfts_response: AllNftInfoResponse = deps.querier.query(&all_nfts)?;
    // let owners = all_nfts_response
    //     .nfts
    //     .into_iter()
    //     .map(|nft_info| nft_info.owner)
    //     .collect();

    moon_config.nft_vest.month_index = nft_minter_month_index + Uint128::from(1 as u8);

    Ok(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        nft_minter_monthly_amount,
        deps.api.addr_humanize(&nft_minter_address)?,
    )?)
}

pub fn emission2marketing(
    deps: &DepsMut<TerraQuery>,
    env: &Env,
    info: &MessageInfo,
    moon_config: & mut MoonInfoRaw,
) -> Result<CosmosMsg, ContractError> {
    let clsm_addr = moon_config.clsm_addr.clone();
    let marketing_address = moon_config.marketing_vest.address.clone();
    let marketing_monthly_amount = moon_config.marketing_vest.monthly_amount;
    let marketing_month_count = moon_config.marketing_vest.month_count;
    let marketing_month_index = moon_config.marketing_vest.month_index;

    if marketing_month_index >= marketing_month_count {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_amount = query_token_balance(
        &deps.as_ref().querier,
        deps.api.addr_humanize(&clsm_addr)?,
        Addr::unchecked(env.contract.address.as_str()),
    )?;

    if clsm_amount < marketing_monthly_amount {
        return Err(ContractError::LessThanVesting {});
    }

    moon_config.marketing_vest.month_index = marketing_month_index + Uint128::from(1 as u8);

    Ok(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        marketing_monthly_amount,
        deps.api.addr_humanize(&marketing_address)?,
    )?)
}

pub fn emission2minigames(
    deps: &DepsMut<TerraQuery>,
    env: &Env,
    info: &MessageInfo,
    moon_config: & mut MoonInfoRaw,
) -> Result<CosmosMsg, ContractError> {
    let clsm_addr = moon_config.clsm_addr.clone();
    let game_address = moon_config.game_vest.address.clone();
    let game_monthly_amount = moon_config.game_vest.monthly_amount;
    let game_month_count = moon_config.game_vest.month_count;
    let game_month_index = moon_config.game_vest.month_index;

    if game_month_index >= game_month_count {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_amount = query_token_balance(
        &deps.as_ref().querier,
        deps.api.addr_humanize(&clsm_addr)?,
        Addr::unchecked(env.contract.address.as_str()),
    )?;

    if clsm_amount < game_monthly_amount {
        return Err(ContractError::LessThanVesting {});
    }

    moon_config.game_vest.month_index = game_month_index + Uint128::from(1 as u8);

    Ok(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        game_monthly_amount,
        deps.api.addr_humanize(&game_address)?,
    )?)
}

pub fn emission2team(
    deps: &DepsMut<TerraQuery>,
    env: &Env,
    info: &MessageInfo,
    moon_config: & mut MoonInfoRaw,
) -> Result<CosmosMsg, ContractError> {
    let clsm_addr = moon_config.clsm_addr.clone();
    let team_address = moon_config.team_vest.address.clone();
    let team_monthly_amount = moon_config.team_vest.monthly_amount;
    let team_month_count = moon_config.team_vest.month_count;
    let team_month_index = moon_config.team_vest.month_index;

    if team_month_index >= team_month_count {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_amount = query_token_balance(
        &deps.as_ref().querier,
        deps.api.addr_humanize(&clsm_addr)?,
        Addr::unchecked(env.contract.address.as_str()),
    )?;

    if clsm_amount < team_monthly_amount {
        return Err(ContractError::LessThanVesting {});
    }

    moon_config.team_vest.month_index = team_month_index + Uint128::from(1 as u8);

    Ok(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        team_monthly_amount,
        deps.api.addr_humanize(&team_address)?,
    )?)
}

pub fn lunc_dynamic_mint (
    deps: DepsMut<TerraQuery>,
    env: &Env,
    info: MessageInfo,
    amount: Uint128,
    price: Decimal
) -> Result<Response, ContractError> {
    let moon_config = MOON_CONFIG.load(deps.storage)?;
    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Native(String::from("uluna")),
        amount,
        Addr::unchecked(BURN_ADDRESS)
    )?);

    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps
            .api
            .addr_humanize(&moon_config.clsm_addr)?
            .to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Mint {
            recipient: info.sender.to_string(),
            amount: amount * price,
        })?,
        funds: vec![],
    }));

    Ok(Response::new().add_messages(messages))
}

pub fn ustc_dynamic_mint (
    deps: DepsMut<TerraQuery>,
    env: &Env,
    info: MessageInfo,
    amount: Uint128,
    price: Decimal
) -> Result<Response, ContractError> {
    let moon_config = MOON_CONFIG.load(deps.storage)?;
    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Native(String::from("uusd")),
        amount,
        Addr::unchecked(BURN_ADDRESS)
    )?);

    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: deps
            .api
            .addr_humanize(&moon_config.clsm_addr)?
            .to_string(),
        msg: to_binary(&Cw20ExecuteMsg::Mint {
            recipient: info.sender.to_string(),
            amount: amount * price,
        })?,
        funds: vec![],
    }));

    Ok(Response::new().add_messages(messages))
}
