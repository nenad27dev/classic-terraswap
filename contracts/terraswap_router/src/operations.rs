use std::str::FromStr;

use cosmwasm_std::{
    to_binary, Addr, Coin, CosmosMsg, Decimal, Deps, DepsMut, Env, MessageInfo, Response, StdError,
    StdResult, WasmMsg,
};

use crate::querier::compute_tax;
use crate::state::{Config, CONFIG};

use classic_bindings::{TerraMsg, TerraQuery};

use classic_terraswap::asset::{Asset, AssetInfo, PairInfo};
use classic_terraswap::pair::ExecuteMsg as PairExecuteMsg;
use classic_terraswap::querier::{query_balance, query_pair_info, query_token_balance};
use classic_terraswap::router::SwapOperation;
use classic_terraswap::util::assert_deadline;
use cw20::Cw20ExecuteMsg;

/// Execute swap operation
/// swap all offer asset to ask asset
pub fn execute_swap_operation(
    deps: DepsMut<TerraQuery>,
    env: Env,
    info: MessageInfo,
    operation: SwapOperation,
    to: Option<String>,
    deadline: Option<u64>,
) -> StdResult<Response<TerraMsg>> {
    if env.contract.address != info.sender {
        return Err(StdError::generic_err("unauthorized"));
    }

    assert_deadline(env.block.time.seconds(), deadline)?;

    let messages: Vec<CosmosMsg<TerraMsg>> = match operation {
        SwapOperation::NativeSwap {
            offer_denom,
            ask_denom,
        } => {
            let amount =
                query_balance(&deps.querier, env.contract.address, offer_denom.to_string())?;
            if let Some(to) = to {
                // if the operation is last, and requires send
                // deduct tax from the offer_coin
                let amount =
                    amount.checked_sub(compute_tax(&deps.querier, amount, offer_denom.clone())?)?;
                vec![CosmosMsg::from(TerraMsg::create_swap_send_msg(
                    to,
                    Coin {
                        denom: offer_denom,
                        amount,
                    },
                    ask_denom,
                ))]
            } else {
                vec![CosmosMsg::from(TerraMsg::create_swap_msg(
                    Coin {
                        denom: offer_denom,
                        amount,
                    },
                    ask_denom,
                ))]
            }
        }
        SwapOperation::TerraSwap {
            offer_asset_info,
            ask_asset_info,
        } => {
            let config: Config = CONFIG.load(deps.as_ref().storage)?;
            let terraswap_factory = deps.api.addr_humanize(&config.terraswap_factory)?;
            let pair_info: PairInfo = query_pair_info(
                &deps.querier,
                terraswap_factory,
                &[offer_asset_info.clone(), ask_asset_info],
            )?;

            let amount = match offer_asset_info.clone() {
                AssetInfo::NativeToken { denom } => {
                    query_balance(&deps.querier, env.contract.address, denom)?
                }
                AssetInfo::Token { contract_addr } => query_token_balance(
                    &deps.querier,
                    deps.api.addr_validate(contract_addr.as_str())?,
                    env.contract.address,
                )?,
            };
            let offer_asset: Asset = Asset {
                info: offer_asset_info,
                amount,
            };

            vec![asset_into_swap_msg(
                deps.as_ref(),
                Addr::unchecked(pair_info.contract_addr),
                offer_asset,
                None,
                to,
            )?]
        }
    };

    Ok(Response::new().add_messages(messages))
}

pub fn asset_into_swap_msg(
    deps: Deps<TerraQuery>,
    pair_contract: Addr,
    offer_asset: Asset,
    max_spread: Option<Decimal>,
    to: Option<String>,
) -> StdResult<CosmosMsg<TerraMsg>> {
    match offer_asset.info.clone() {
        AssetInfo::NativeToken { denom } => {
            // deduct tax first
            let amount = offer_asset.amount.checked_sub(compute_tax(
                &deps.querier,
                offer_asset.amount,
                denom.clone(),
            )?)?;

            Ok(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: pair_contract.to_string(),
                funds: vec![Coin { denom, amount }],
                msg: to_binary(&PairExecuteMsg::Swap {
                    offer_asset: Asset {
                        amount,
                        ..offer_asset
                    },
                    belief_price: None,
                    max_spread,
                    to,
                    deadline: None,
                })?,
            }))
        }
        AssetInfo::Token { contract_addr } => Ok(CosmosMsg::Wasm(WasmMsg::Execute {
            contract_addr,
            funds: vec![],
            msg: to_binary(&Cw20ExecuteMsg::Send {
                contract: pair_contract.to_string(),
                amount: offer_asset.amount,
                msg: to_binary(&PairExecuteMsg::Swap {
                    offer_asset,
                    belief_price: None,
                    max_spread,
                    to,
                    deadline: None,
                })?,
            })?,
        })),
    }
}
