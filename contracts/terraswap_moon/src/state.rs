use classic_terraswap::asset::MoonInfoRaw;
use cw_storage_plus::Item;

pub const MOON_CONFIG: Item<MoonInfoRaw> = Item::new("moon_config");
