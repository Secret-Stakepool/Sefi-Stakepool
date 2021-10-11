use cosmwasm_std::{HumanAddr, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use secret_toolkit::incubator::generational_store::Index;

#[derive(Serialize, Deserialize,  PartialEq, Debug, Clone)]
pub struct Config {
    pub admin: HumanAddr,
    pub triggerer: HumanAddr,
    pub triggerer_share_percentage: u64,
    pub token: SecretContract,
    pub staking_contract: SecretContract,
    pub viewing_key: String,
    pub prng_seed: Vec<u8>,
    pub is_stopped: bool,
    pub is_stopped_can_withdraw:bool,
    pub own_addr: HumanAddr,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone, JsonSchema)]
pub struct SecretContract {
    pub address: HumanAddr,
    pub contract_hash: String,
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Lottery {
    //sender_address, amount, entry_time
    pub entropy: Vec<u8>,
    pub seed: Vec<u8>,
    pub duration: u64,
    pub start_time: u64,
    pub end_time: u64,
}

//Append store

//Append store
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LastLotteryResults {
    //winning amount and time
    pub winning_amount:u64, //Append store
    pub time:u64,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub struct SupplyPool {
    pub total_tokens_staked: Uint128,
    pub total_rewards_restaked:Uint128,
    pub pending_staking_rewards:Uint128,
    pub triggering_cost:Uint128
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub struct UserInfo {
    pub amount_delegated: Uint128,
    pub available_tokens_for_withdraw:Uint128,
    pub total_won:Uint128,
    pub entries: Vec<( Uint128,u64)>,
    pub entry_index:Vec<Index>,
}


#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub struct UserWinningHistory{
    //winning amount and rewards
    pub winning_amount:u64, //Append store
    pub time:u64,
}

//Testing
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LotteryEntries{
    pub user_address: HumanAddr,
    pub amount:Uint128,
    pub entry_time:u64,
}




// pub fn write_viewing_key<S: Storage>(store: &mut S, owner: &CanonicalAddr, key: &ViewingKey) {
//     let mut balance_store = PrefixedStorage::new(PREFIX_VIEW_KEY, store);
//     balance_store.set(owner.as_slice(), &key.to_hashed());
// }
//
// pub fn read_viewing_key<S: Storage>(store: &S, owner: &CanonicalAddr) -> Option<Vec<u8>> {
//     let balance_store = ReadonlyPrefixedStorage::new(VIEWING_KEY_KEY, store);
//     balance_store.get(owner.as_slice())
// }
