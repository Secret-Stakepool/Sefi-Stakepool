use cosmwasm_std::{HumanAddr,CanonicalAddr, Uint128};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    pub own_addr: HumanAddr,
    pub stopped_emergency_redeem_jackpot: Uint128
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone, JsonSchema)]
pub struct SecretContract {
    pub address: HumanAddr,
    pub contract_hash: String,
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Lottery {
    //sender_address, amount, entry_time
    pub entries: Vec<(CanonicalAddr, Uint128,u64)>,
    pub entropy: Vec<u8>,
    pub seed: Vec<u8>,
    pub duration: u64,
    pub start_time: u64,
    pub end_time: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct LastLotteryResults {
    pub past_winners:Vec<String>,
    pub past_number_of_entries: Vec<u64>,
    pub past_total_deposits:Vec<u64>,
    pub past_rewards:Vec<(u64,u64)>
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone)]
pub struct SupplyPool {
    pub total_tokens_staked: Uint128,
    pub total_rewards_restaked:Uint128,
    pub pending_staking_rewards:Uint128,
    pub triggering_cost:Uint128

}

#[derive(Serialize, Deserialize, Debug)]
pub struct UserInfo {
    pub amount_delegated: Uint128,
    pub available_tokens_for_withdraw:Uint128,
    pub total_won:Uint128,
    pub winning_history:Vec<(u64,u64)>,

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
