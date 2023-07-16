use crate::error::ContractError;
use cosmwasm_std::{
    to_binary, Addr, BalanceResponse as NativeBalanceResponse, BankMsg, BankQuery, Coin, CosmosMsg,
    QuerierWrapper, QueryRequest, Uint128, WasmMsg, WasmQuery,
};
use cw20::{BalanceResponse as CW20BalanceResponse, Cw20ExecuteMsg, Cw20QueryMsg, Denom};

pub fn get_token_amount(
    querier: QuerierWrapper,
    denom: Denom,
    contract_addr: Addr,
) -> Result<Uint128, ContractError> {
    match denom.clone() {
        Denom::Native(native_str) => {
            let native_response: NativeBalanceResponse =
                querier.query(&QueryRequest::Bank(BankQuery::Balance {
                    address: contract_addr.clone().into(),
                    denom: native_str,
                }))?;
            return Ok(native_response.amount.amount);
        }
        Denom::Cw20(cw20_address) => {
            let balance_response: CW20BalanceResponse =
                querier.query(&QueryRequest::Wasm(WasmQuery::Smart {
                    contract_addr: cw20_address.clone().into(),
                    msg: to_binary(&Cw20QueryMsg::Balance {
                        address: contract_addr.clone().into(),
                    })?,
                }))?;
            return Ok(balance_response.balance);
        }
    }
}

pub fn transfer_token_message(
    denom: Denom,
    amount: Uint128,
    receiver: Addr,
) -> Result<CosmosMsg, ContractError> {
    match denom.clone() {
        Denom::Native(native_str) => {
            return Ok(BankMsg::Send {
                to_address: receiver.clone().into(),
                amount: vec![Coin {
                    denom: native_str,
                    amount,
                }],
            }
            .into());
        }
        Denom::Cw20(cw20_address) => {
            return Ok(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_address.clone().into(),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient: receiver.clone().into(),
                    amount,
                })?,
            }));
        }
    }
}

pub fn transfer_from_token_message(
    owner: Addr,
    denom: Denom,
    amount: Uint128,
    receiver: Addr,
) -> Result<CosmosMsg, ContractError> {
    match denom.clone() {
        Denom::Native(_) => {
            return Err(ContractError::UnacceptableToken {});
        }
        Denom::Cw20(cw20_address) => {
            return Ok(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: cw20_address.clone().into(),
                funds: vec![],
                msg: to_binary(&Cw20ExecuteMsg::TransferFrom {
                    owner: owner.to_string(),
                    recipient: receiver.clone().into(),
                    amount,
                })?,
            }));
        }
    }
}
