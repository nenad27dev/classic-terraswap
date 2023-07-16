use crate::error::ContractError;
use crate::response::MsgInstantiateContractResponse;
use crate::state::MOON_CONFIG;
use crate::util;
use classic_terraswap::querier::{
    query_balance, query_pair_info, query_token_balance, query_token_total_supply,
};

#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{
    from_binary, to_binary, Addr, Binary, CanonicalAddr, CosmosMsg, Decimal, Decimal256, Deps,
    DepsMut, Env, MessageInfo, Reply, ReplyOn, Response, StdError, StdResult, SubMsg, Uint128,
    Uint256, WasmMsg,
};

use classic_bindings::{TerraMsg, TerraQuery};

use classic_terraswap::asset::{Asset, AssetInfo, MoonInfo, MoonInfoRaw, VestInfo, VestInfoRaw};
use classic_terraswap::moon::{
    Cw20HookMsg, ExecuteMsg, InstantiateMsg, MigrateMsg, PoolResponse, QueryMsg,
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
        minter_addr: deps.api.addr_canonicalize(&msg.minter_addr.as_str())?,
        timer_trigger: deps.api.addr_canonicalize(&msg.timer_trigger.as_str())?,
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
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::MintCLSMToPairContract {} => emission2pair_contract(deps, env, info),
        ExecuteMsg::MintCLSMToNFTMinters {} => emission2nft_minter(deps, env, info),
        ExecuteMsg::MintCLSMToMarketing {} => emission2marketing(deps, env, info),
        ExecuteMsg::MintCLSMToMiniGames {} => emission2minigames(deps, env, info),
        ExecuteMsg::MintCLSMToTeam {} => emission2team(deps, env, info),
        ExecuteMsg::DynamicMintFromLunc { amount } => dynamic_mint(deps, env, info, amount),
        ExecuteMsg::DynamicMintFromUstc { amount } => dynamic_mint(deps, env, info, amount),
        ExecuteMsg::AutomaticBurn {} => automatic_burn(deps, env, info),
        ExecuteMsg::SendLUNC { amount } => sendLunc(deps, env, amount),
    }
}

pub fn emission2pair_contract(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

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

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        pair_contract_monthly_amount,
        deps.api.addr_humanize(&pair_contract_address)?,
    )?);

    moon_config.pair_vest.month_index = pair_contract_month_index + Uint128::from(1 as u8);
    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn emission2nft_minter(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

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

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        nft_minter_monthly_amount,
        deps.api.addr_humanize(&nft_minter_address)?,
    )?);

    moon_config.nft_vest.month_index = nft_minter_month_index + Uint128::from(1 as u8);
    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn emission2marketing(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

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

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        marketing_monthly_amount,
        deps.api.addr_humanize(&marketing_address)?,
    )?);

    moon_config.marketing_vest.month_index = marketing_month_index + Uint128::from(1 as u8);
    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn emission2minigames(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

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

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        game_monthly_amount,
        deps.api.addr_humanize(&game_address)?,
    )?);

    moon_config.game_vest.month_index = game_month_index + Uint128::from(1 as u8);
    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn emission2team(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

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

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        team_monthly_amount,
        deps.api.addr_humanize(&team_address)?,
    )?);

    moon_config.team_vest.month_index = team_month_index + Uint128::from(1 as u8);
    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn dynamic_mint(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
    amount: Uint128,
) -> Result<Response, ContractError> {
    let mut moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_addr = moon_config.clsm_addr.clone();
    let pair_contract_address = moon_config.pair_vest.address.clone();
    let pair_contract_monthly_amount = moon_config.pair_vest.monthly_amount;
    let pair_contract_month_count = moon_config.pair_vest.month_count;
    let pair_contract_month_index = moon_config.pair_vest.month_index;

    if pair_contract_month_index >= pair_contract_month_count {
        return Err(ContractError::Unauthorized {});
    }

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(util::transfer_token_message(
        Denom::Cw20(deps.api.addr_humanize(&clsm_addr)?),
        pair_contract_monthly_amount,
        deps.api.addr_humanize(&pair_contract_address)?,
    )?);

    moon_config.pair_vest.month_index = pair_contract_month_index + Uint128::from(1 as u8);
    MOON_CONFIG.save(deps.storage, &moon_config)?;

    Ok(Response::new().add_messages(messages))
}

pub fn automatic_burn(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
) -> Result<Response, ContractError> {
    let moon_config = MOON_CONFIG.load(deps.storage)?;

    // permission check
    if deps.api.addr_canonicalize(info.sender.as_str())? != moon_config.timer_trigger {
        return Err(ContractError::Unauthorized {});
    }

    let clsm_addr = moon_config.clsm_addr.clone();
    let pair_contract_address = moon_config.pair_vest.address.clone();
    let total_supply = query_token_total_supply(
        &deps.querier,
        deps.api.addr_humanize(&clsm_addr)?,
        Addr::unchecked(env.contract.address.as_str()),
    )?;
    let mut burn_amount = total_supply;
    if total_supply >= Uint128::from(1000000000u64) {
        burn_amount = total_supply / Uint128::from(4u32);
    } else {
        burn_amount = total_supply / Uint128::from(100u32);
    }

    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: clsm_addr.to_string(),
        msg: to_binary(&Cw20ExecuteMsg::BurnFrom {
            owner: pair_contract_address.to_string(),
            amount: burn_amount,
        })?,
        funds: vec![],
    }));

    Ok(Response::new().add_messages(messages))
}

pub fn sendLunc (amount: Uint128) -> Result<Response, ContractError> {
 let mut messags: Vec<CosmosMsg> = vec![];
 message.push(util::transfer_token_message(
    Denom::Native("uluna"),
    env.contract.address,
    amount
 )?);

 

 Ok(Response::new().add_messages(messages))
}

// Define the receive function to handle incoming funds
pub fn receive(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
    _msg: Binary,
) -> StdResult<()> {
    // Check if the incoming funds are in LUNA denomination
    if info.funds.len() == 1 && info.funds[0].denom == "uluna" {
        // Create a `MsgBurn` message with the specified amount of LUNA
        let msg = MsgBurn {
            amount: coins(amount, "uluna"),
            from_address: info.sender.into(),
        };

        // Create a Cosmos SDK `Message` object from the `MsgBurn` message
        let cosmos_msg = create_msg(&msg)?;

        // Send the message using the Cosmos SDK `Message` object
        // For example, using the `execute` function provided by CosmWasm
        let res = cosmwasm_std::execute(vec![cosmos_msg.into()])?;
    }
    Ok(())
}
