use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use classic_terraswap::asset::PairInfoRaw;
use cw_storage_plus::Item;
use cosmwasm_std::{Addr};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct Config {
    pub team_addr: Addr,
}

pub const CONFIG: Item<Config> = Item::new("config");

pub const PAIR_INFO: Item<PairInfoRaw> = Item::new("pair_info");
