//Crate import
use crate::constants::*;
use crate::viewing_keys::{ViewingKey, VIEWING_KEY_SIZE};
use crate::state::{SupplyPool, UserInfo, Config, Lottery, LastLotteryResults,SecretContract};
use crate::msg::{HandleAnswer, HandleMsg, InitMsg, LPStakingRewardsResponse, QueryAnswer, QueryMsg, LPStakingQueryMsg, LPStakingHandleMsg, ResponseStatus::Success, space_pad};
//
// //Cosmwasm import
use cosmwasm_storage::{PrefixedStorage, ReadonlyPrefixedStorage};
use cosmwasm_std::{Api, Binary, CosmosMsg, Env, Extern, HandleResponse, HumanAddr, InitResponse, Querier, ReadonlyStorage, StdError, StdResult, Storage, Uint128, WasmMsg, from_binary, to_binary,  CanonicalAddr};

//secret toolkit import
use secret_toolkit::storage::{TypedStore};
use secret_toolkit::snip20::{transfer_msg, send_msg};
use secret_toolkit::utils::{Query, pad_handle_result, pad_query_result};
use secret_toolkit::{crypto::sha_256, storage::TypedStoreMut, snip20};

//Rust functions
use rand::prelude::*;
use sha2::{Digest, Sha256};
use rand_core::SeedableRng;
use rand_chacha::ChaChaRng;
use rand::distributions::WeightedIndex;
use crate::msg::ResponseStatus::Failure;


pub fn init<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: InitMsg,
) -> StdResult<InitResponse> {
    // Initialize state
    let prng_seed_hashed = sha_256(&msg.prng_seed.0);
    let admin;
    if msg.admin.clone().is_some() {
        admin = msg.admin.clone().unwrap();
    } else {
        admin = env.message.sender.clone();
    }

    let triggerer;
    if msg.triggerer.clone().is_some() {
        triggerer = msg.triggerer.clone().unwrap();
    } else {
        triggerer = env.message.sender.clone();
    }

    TypedStoreMut::attach(&mut deps.storage).store(
        CONFIG_KEY,
        &Config {
            admin,
            triggerer,
            triggerer_share_percentage: msg.triggerer_share_percentage,
            token: msg.token.clone(),
            staking_contract: msg.staking_contract.clone(),
            viewing_key: msg.viewing_key.clone(),
            prng_seed: prng_seed_hashed.to_vec(),
            is_stopped: false,
            own_addr: env.contract.address,
            stopped_emergency_redeem_jackpot: Uint128(0),
        },
    )?;

    TypedStoreMut::attach(&mut deps.storage).store(
        SUPPLY_POOL_KEY,
        &SupplyPool {
            total_tokens_staked: Uint128(0),
            total_rewards_restaked: Uint128(0),
            pending_staking_rewards: Uint128(0),
            triggering_cost: Uint128(0),
        },
    )?;

    //lottery init
    let time = env.block.time;
    let duration = 600u64;
    //Create first lottery
    // Save to state
    TypedStoreMut::attach(&mut deps.storage).store(
        LOTTERY_KEY,
        &Lottery {
            entries: Vec::default(),
            entropy: prng_seed_hashed.to_vec(),
            start_time: time + 1,
            end_time: time + duration + 1,
            seed: prng_seed_hashed.to_vec(),
            duration,
        },
    )?;

    TypedStoreMut::attach(&mut deps.storage).store(
        LAST_LOTTERY_KEY,
        &LastLotteryResults {
            past_winners: vec![],
            past_number_of_entries: vec![],
            past_total_deposits: vec![],
            past_rewards: vec![], //add time in this vector
        },
    )?;

    // Register sSCRT and incentive token, set vks
    let messages = vec![
        snip20::register_receive_msg(
            env.contract_code_hash.clone(),
            None,
            1, // This is public data, no need to pad
            msg.token.contract_hash.clone(),
            msg.token.address.clone(),
        )?,
        snip20::set_viewing_key_msg(
            msg.viewing_key,
            None,
            RESPONSE_BLOCK_SIZE, // This is private data, need to pad
            msg.token.contract_hash,
            msg.token.address,
        )?,
        snip20::set_viewing_key_msg(
            STAKING_VK.to_string(),
            None,
            RESPONSE_BLOCK_SIZE,
            msg.staking_contract.contract_hash,
            msg.staking_contract.address,
        )?,
    ];

    Ok(InitResponse {
        messages,
        log: vec![],
    })



}

pub fn handle<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    msg: HandleMsg,
) -> StdResult<HandleResponse> {
    let config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;

    if config.is_stopped {

            let response = match msg {

                //USER->Viewing Key
                HandleMsg::CreateViewingKey { entropy, .. } => { create_viewing_key(deps, env, entropy) }
                HandleMsg::SetViewingKey { key, .. } => set_viewing_key(deps, env, key),


                //Admin  ---> ChangeStakingContractFlow
                // => 1.set_stop_all_status 2.EmergencyRedeemFromStaking 4.ChangeStakingContract  5.RedelegateToNewContract 6.SetNormalStatus
                HandleMsg::EmergencyRedeemFromStaking {} => emergency_redeem_from_staking(deps, env),
                HandleMsg::ChangeStakingContract { address, contract_hash } => change_staking_contract(deps, env, address, contract_hash),
                HandleMsg::RedelegateToNewContract {} => redelegate_to_contract(deps, env),
                HandleMsg::ResumeContract {}=>resume_contract(deps,env),
                HandleMsg::TriggeringCostWithdraw {} => triggering_cost_withdraw(deps, env),

                HandleMsg::Withdraw { amount } => withdraw(deps, env, amount),
                HandleMsg::TriggerWithdraw { amount } => trigger_withdraw(deps, env, amount),


                _ => Err(StdError::generic_err(
                    "This contract is stopped and this action is not allowed",
                )),
            };
        return pad_handle_result(response, RESPONSE_BLOCK_SIZE)
        }




    let response = match msg {

        // Triggerer
        HandleMsg::ClaimRewards {} => claim_rewards(deps, env),

        //USER
        HandleMsg::Receive { from, amount, msg, .. } => receive(deps, env, from, amount, msg),
        HandleMsg::Withdraw { amount } => withdraw(deps, env, amount),
        HandleMsg::TriggerWithdraw { amount } => trigger_withdraw(deps, env, amount),
        HandleMsg::Redelegate { amount } => redelegate(deps, env, amount),
        //USER->Viewing Key
        HandleMsg::CreateViewingKey { entropy, .. } => { create_viewing_key(deps, env, entropy) }
        HandleMsg::SetViewingKey { key, .. } => set_viewing_key(deps, env, key),

        //Admin
        HandleMsg::ChangeAdmin { admin } => change_admin(deps, env, admin),
        HandleMsg::ChangeTriggerer { admin } => change_triggerer(deps, env, admin),
        HandleMsg::ChangeTriggererShare { percentage, .. } => change_triggerer_share(deps, env, percentage),
        HandleMsg::ChangeLotteryDuration { duration } => change_lottery_duration(deps, env, duration),
        HandleMsg::TriggeringCostWithdraw {} => triggering_cost_withdraw(deps, env),
        HandleMsg::StopContract {}=>stop_contract(deps,env),


        _ => Err(StdError::generic_err("Unavailable or unknown handle message")),
    };
    pad_handle_result(response, RESPONSE_BLOCK_SIZE)
}

fn resume_contract<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config_store = TypedStoreMut::attach(&mut deps.storage);
    let mut config: Config = config_store.load(CONFIG_KEY)?;

    if env.message.sender == config.admin && config.is_stopped {
        config.is_stopped = false;
        config_store.store(CONFIG_KEY, &config)?;

        return Ok(HandleResponse {
            messages: vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::ResumeContract { status: Success })?),
        })
    } else {
        return Err(StdError::generic_err(format!(
            "User does not permissions to resume contract!"
        )));
    }
}


fn stop_contract<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config_store = TypedStoreMut::attach(&mut deps.storage);
    let mut config: Config = config_store.load(CONFIG_KEY)?;

    if env.message.sender == config.admin && !config.is_stopped {
        config.is_stopped = true;
        config_store.store(CONFIG_KEY, &config)?;

        return Ok(HandleResponse {
            messages: vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::StopContract { status: Success })?),
        })
    } else {
        return Err(StdError::generic_err(format!(
            "User does not permissions to resume contract!"
        )));
    }
}





pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    let response = match msg {
        QueryMsg::ContractStatus {} => query_contract_status(deps),
        QueryMsg::LotteryInfo {} => {
            // query_lottery_info(&deps.storage)
            let lottery:Lottery = TypedStore::attach(&deps.storage).load(LOTTERY_KEY)?;
            to_binary(&QueryAnswer::LotteryInfo {
                start_time: lottery.start_time,
                end_time: lottery.end_time,
                duration: lottery.duration,
            })
        }
        QueryMsg::RewardToken {} => query_token(deps),
        QueryMsg::TotalRewards { height } => query_total_rewards(deps, height),
        QueryMsg::TotalDeposits {} => query_total_deposit(deps),
        QueryMsg::PastAllRecords {} => query_all_past_results(deps),
        QueryMsg::PastRecords {} => query_past_results(deps),

        //Temporary functions



        _ => authenticated_queries(deps, msg),
    };

    pad_query_result(response, RESPONSE_BLOCK_SIZE)
}

pub fn authenticated_queries<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    let (address, key) = msg.get_validation_params();

    let vk_store = ReadonlyPrefixedStorage::new(VIEWING_KEY_KEY, &deps.storage);
    let expected_key = vk_store.get(address.0.as_bytes());

    if expected_key.is_none() {
        // Checking the key will take significant time. We don't want to exit immediately if it isn't set
        // in a way which will allow to time the command and determine if a viewing key doesn't exist
        key.check_viewing_key(&[0u8; VIEWING_KEY_SIZE]);
    } else if key.check_viewing_key(expected_key.unwrap().as_slice()) {
        return match msg {
            QueryMsg::Balance { address, .. } => query_deposit(deps, &address),
            QueryMsg::AvailableTokensForWithdrawl { address, .. } => query_available_funds(deps, &address),
            QueryMsg::UserPastRecords { address, .. } => query_user_past_records(deps, address),

            _ => panic!("Unavailable or unknown query message"),
        };
    }

    Ok(to_binary(&QueryAnswer::ViewingKeyError {
        msg: "Wrong viewing key for this address or viewing key not set".to_string(),
    })?)
}
// USER FUNCTIONS
pub fn create_viewing_key<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    entropy: String,
) -> StdResult<HandleResponse> {
    let config: Config = TypedStore::attach(& deps.storage).load(CONFIG_KEY)?;
    let prng_seed = config.prng_seed;

    let key = ViewingKey::new(&env, &prng_seed, (&entropy).as_ref());

    let mut vk_store = PrefixedStorage::new(VIEWING_KEY_KEY, &mut deps.storage);
    vk_store.set(env.message.sender.0.as_bytes(), &key.to_hashed());

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::CreateViewingKey { key })?),
    })
}

pub fn set_viewing_key<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    key: String,
) -> StdResult<HandleResponse> {
    let vk = ViewingKey(key);

    let mut vk_store = PrefixedStorage::new(VIEWING_KEY_KEY, &mut deps.storage);
    vk_store.set(env.message.sender.0.as_bytes(), &vk.to_hashed());

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::SetViewingKey {
            status: Success,
        })?),
    })
}

// Handle functions

fn receive<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    from: HumanAddr,
    amount: Uint128,
    msg: Binary,
) -> StdResult<HandleResponse> {
    let msg: HandleMsg = from_binary(&msg)?;

    match msg {
        HandleMsg::Deposit {} => deposit(deps, env, from, amount),
        _ => {
            Err(StdError::generic_err(
                "Handle msg not correct",
            ))
        }
    }
}

fn valid_amount(amt: Uint128) -> bool {
    amt >= Uint128(1000000)
}

fn deposit<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    from: HumanAddr,
    amount_to_deposit: Uint128,
) -> StdResult<HandleResponse> {

    // CHECKING Ensure that the sent tokens are from an expected contract address
    let config = TypedStore::<Config, S>::attach(&deps.storage).load(CONFIG_KEY)?;
    if env.message.sender != config.token.address {
        return Err(StdError::generic_err(format!(
            "This token is not supported. Supported: {}, given: {}",
            config.token.address, env.message.sender
        )));
    }
    if !valid_amount(amount_to_deposit) {
        return Err(StdError::generic_err(
            "Must deposit a minimum of 1000000 usefi, or 1 sefi",
        ));
    }

    //UPDATING USER DATA
    let mut user = TypedStore::<UserInfo, S>::attach(&deps.storage)
        .load(from.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), winning_history: vec![] }); // NotFound is the only possible error
    user.amount_delegated += amount_to_deposit;
    TypedStoreMut::<UserInfo, S>::attach(&mut deps.storage).store(from.0.as_bytes(), &user)?;

    //Updating lottery
    let sender_address = deps.api.canonical_address(&from)?;
    let mut a_lottery:Lottery = TypedStore::attach(&deps.storage).load(LOTTERY_KEY)?;
    &a_lottery.entries.push((sender_address, amount_to_deposit, env.block.time, ));
    &a_lottery.entropy.extend(&env.block.height.to_be_bytes());
    &a_lottery.entropy.extend(&env.block.time.to_be_bytes());
    let _ = TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;

    //Updating Rewards store
    let supply_store = TypedStore::attach(& deps.storage);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    let amount_to_stake = amount_to_deposit + supply_pool.pending_staking_rewards;
    supply_pool.total_tokens_staked += amount_to_deposit;
    supply_pool.total_rewards_restaked += supply_pool.pending_staking_rewards;

    //QUERYING PENDING_REWARDS
    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps,&env,&config)?;

    if staking_rewards_response.rewards.rewards > Uint128(0) {
        supply_pool.pending_staking_rewards = staking_rewards_response.rewards.rewards;
    } else {
        supply_pool.pending_staking_rewards = Uint128(0);
    }
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;


    Ok(HandleResponse {
        messages: vec![
            send_msg(
                config.staking_contract.address,
                amount_to_stake,
                Some(to_binary(&LPStakingHandleMsg::Deposit {})?),
                None,
                RESPONSE_BLOCK_SIZE,
                config.token.contract_hash,
                config.token.address,
            )?
        ],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::Deposit {
            status: Success,
        })?),
    })
}


fn trigger_withdraw<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Option<Uint128>,
) -> StdResult<HandleResponse> {


    //loading user info
    let mut user = TypedStore::<UserInfo, S>::attach(&deps.storage)
        .load(env.message.sender.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), winning_history: vec![] }); // NotFound is the only possible error

    let withdraw_amount = amount.unwrap_or(user.amount_delegated);
    //checking if withdraw is possible or not
    if withdraw_amount <= Uint128(0) {
        return Err(StdError::generic_err("No sefi staked"));
    }
    if user.amount_delegated < withdraw_amount {
        return Err(StdError::generic_err("Trying to withdrawing more amount than staked"));
    }

    //updating user info
    // let account_balance = user.amount_delegated;
    user.amount_delegated = (user.amount_delegated - withdraw_amount).unwrap();
    user.available_tokens_for_withdraw += withdraw_amount;
    TypedStoreMut::<UserInfo, S>::attach(&mut deps.storage).store(env.message.sender.0.as_bytes(), &user)?;

    let config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;
    let sender_address = deps.api.canonical_address(&env.message.sender)?;
    let mut a_lottery = lottery_adjustment(&deps, withdraw_amount, sender_address)?;
    a_lottery.entropy.extend(&env.block.height.to_be_bytes());
    a_lottery.entropy.extend(&env.block.time.to_be_bytes());
    let _ = TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;


    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps,&env,&config)?;

    let mut supply_pool: SupplyPool = TypedStore::attach(& deps.storage).load(SUPPLY_POOL_KEY)?;
    if staking_rewards_response.rewards.rewards > Uint128(0) {
        supply_pool.pending_staking_rewards += staking_rewards_response.rewards.rewards
    }
    //updating the reward pool
    supply_pool.total_tokens_staked = (supply_pool.total_tokens_staked - withdraw_amount).unwrap();
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;

    let mut messages: Vec<CosmosMsg> = vec![];

    messages.push(
        WasmMsg::Execute {
            contract_addr: config.staking_contract.address.clone(),
            callback_code_hash: config.staking_contract.contract_hash.clone(),
            msg: to_binary(&LPStakingHandleMsg::Redeem {
                amount: Uint128::from(withdraw_amount)
            })?,
            send: vec![],
        }
            .into()
    );

    Ok(HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::TriggerWithdraw {
            status: Success,
        })?),
    })
}

fn withdraw<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Option<Uint128>,
) -> StdResult<HandleResponse> {
    let config: Config = TypedStore::attach(& deps.storage).load(CONFIG_KEY)?;

    let mut user = TypedStore::<UserInfo, S>::attach(&deps.storage)
        .load(env.message.sender.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), winning_history: vec![] }); // NotFound is the only possible error

    let withdraw_amount = amount.unwrap_or(user.available_tokens_for_withdraw);
    if withdraw_amount <= Uint128(0) {
        return Err(StdError::generic_err("No tokens available for withdraw"));
    }
    if user.available_tokens_for_withdraw < withdraw_amount {
        return Err(StdError::generic_err("Withdrawing more amount than available tokens for withdraw"));
    }

    user.available_tokens_for_withdraw = (user.available_tokens_for_withdraw - withdraw_amount).unwrap();
    TypedStoreMut::<UserInfo, S>::attach(&mut deps.storage).store(env.message.sender.0.as_bytes(), &user)?;


    let messages: Vec<CosmosMsg> = vec![
        // Transfer Trigger fee to triggerer wallet
        transfer_msg(
            env.message.sender,
            Uint128::from(withdraw_amount),
            None,
            RESPONSE_BLOCK_SIZE,
            config.token.contract_hash.clone(),
            config.token.address.clone(),
        )?
    ];


    Ok(HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::Withdraw {
            status: Success,
        })?),
    })
}


fn redelegate<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    amount: Option<Uint128>,
) -> StdResult<HandleResponse> {
    let mut user = TypedStore::<UserInfo, S>::attach(&deps.storage)
        .load(env.message.sender.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), winning_history: vec![] }); // NotFound is the only possible error
    let redelegation_amount = amount.unwrap_or(user.available_tokens_for_withdraw);

    if user.available_tokens_for_withdraw < redelegation_amount {
        return Err(StdError::generic_err("Number of tokens available for redelegation are less than the number of tokens requested to redelegate"));
    }

    let config: Config = TypedStore::attach(& deps.storage).load(CONFIG_KEY)?;
    user.available_tokens_for_withdraw = (user.available_tokens_for_withdraw - redelegation_amount).unwrap();
    user.amount_delegated += redelegation_amount;
    TypedStoreMut::<UserInfo, S>::attach(&mut deps.storage).store(env.message.sender.0.as_bytes(), &user)?;

    //Updating Lottery
    let sender_address = deps.api.canonical_address(&env.message.sender)?;
    let mut a_lottery:Lottery = TypedStore::attach(&deps.storage).load(LOTTERY_KEY)?;
    &a_lottery.entries.push((
        sender_address,
        redelegation_amount,
        env.block.time,
    ));
    &a_lottery.entropy.extend(&env.block.height.to_be_bytes());
    &a_lottery.entropy.extend(&env.block.time.to_be_bytes());
    let _ = TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;


    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps,&env,&config)?;

    let mut lp_pending_staking_rewards = Uint128(0);
    if staking_rewards_response.rewards.rewards > Uint128(0) {
        lp_pending_staking_rewards = staking_rewards_response.rewards.rewards
    }
    let mut supply_pool: SupplyPool = TypedStore::attach(& deps.storage).load(SUPPLY_POOL_KEY)?;
    let amount_to_deposit = redelegation_amount + supply_pool.pending_staking_rewards;


    //query for recent rewards
    supply_pool.total_tokens_staked += redelegation_amount;
    supply_pool.total_rewards_restaked += supply_pool.pending_staking_rewards;
    supply_pool.pending_staking_rewards = lp_pending_staking_rewards;
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;


    Ok(HandleResponse {
        messages: vec![
            send_msg(
                config.staking_contract.address,
                Uint128::from(amount_to_deposit),
                Some(to_binary(&LPStakingHandleMsg::Deposit {})?),
                None,
                RESPONSE_BLOCK_SIZE,
                config.token.contract_hash.clone(),
                config.token.address.clone(),
            )?
        ],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::Redelegate {
            status: Success,
        })?),
    })
}


//Triggerer
fn claim_rewards<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let config = TypedStore::<Config, S>::attach(&deps.storage).load(CONFIG_KEY)?;
    check_if_triggerer(&config, &env.message.sender)?;

    let mut a_lottery:Lottery = TypedStore::attach(&deps.storage).load(LOTTERY_KEY)?;
    validate_end_time(a_lottery.end_time, env.block.time)?;
    validate_start_time(a_lottery.start_time, env.block.time)?;


    if a_lottery.entries.len()==0{
        a_lottery.entropy.extend(&env.block.height.to_be_bytes());
        a_lottery.entropy.extend(&env.block.time.to_be_bytes());

        a_lottery.start_time = &env.block.time + 10;
        a_lottery.end_time = &env.block.time + a_lottery.duration + 10;
        TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;

        return     Ok(HandleResponse {
            messages:vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::ClaimRewards {
                status: Failure,
                winner: HumanAddr("No entries".to_string()),
            })?),
        })
    }

    //Getting the pending_rewards
    let response: LPStakingRewardsResponse = query_pending_rewards(&deps,&env,&config)?;

    let mut supply_pool: SupplyPool = TypedStore::attach(& deps.storage).load(SUPPLY_POOL_KEY)?;
    let mut winning_amount = supply_pool.total_rewards_restaked + response.rewards.rewards + supply_pool.pending_staking_rewards;
    //1 percent
    let trigger_percentage = config.triggerer_share_percentage;
    let trigger_share = Uint128(winning_amount.0 * ((trigger_percentage * 1000000) as u128) / 100000000);

    // this way every time we call the claim_rewards function we will get a different result.
    // Plus it's going to be pretty hard to predict the exact time of the block, so less chance of cheating
    winning_amount = (winning_amount - trigger_share).unwrap();

    supply_pool.triggering_cost = trigger_share;
    supply_pool.pending_staking_rewards = Uint128(0);
    let redeeming_amount = supply_pool.total_rewards_restaked;
    supply_pool.total_rewards_restaked = Uint128(0);
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;
    if winning_amount == Uint128(0) {
        return Err(StdError::generic_err(
            "No rewards available",
        ));
    }

    //Launching the lottery
    a_lottery.entropy.extend(&env.block.height.to_be_bytes());
    a_lottery.entropy.extend(&env.block.time.to_be_bytes());

    let entries: Vec<_> = (&a_lottery.entries).into_iter().map(|(k, _, _)| k).collect();
    let weights: Vec<u128> = (&a_lottery.entries).into_iter().map(|(_, user_staked_amount, deposit_time)|
        if &a_lottery.end_time <= deposit_time  {
            (0 as u128)
        }
        else if ((&a_lottery.end_time - deposit_time) / &a_lottery.duration) >= 1 {
            user_staked_amount.0
        } else {
            (user_staked_amount.0 / 1000000) * ((((a_lottery.end_time - deposit_time) * 1000000) / a_lottery.duration) as u128)
        }
    ).collect();

    let prng_seed = config.prng_seed;
    let mut hasher = Sha256::new();
    hasher.update(&prng_seed);
    hasher.update(&a_lottery.entropy);
    let hash = hasher.finalize();

    let mut result = [0u8; 32];
    result.copy_from_slice(hash.as_slice());
    let mut rng: ChaChaRng = ChaChaRng::from_seed(result);

    if let Err(_err) = WeightedIndex::new(&weights){

        a_lottery.entropy.extend(&env.block.height.to_be_bytes());
        a_lottery.entropy.extend(&env.block.time.to_be_bytes());

        a_lottery.start_time = &env.block.time + 10;
        a_lottery.end_time = &env.block.time + a_lottery.duration + 10;
        TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;

        return     Ok(HandleResponse {
            messages:vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::ClaimRewards {
                status: Failure,
                winner: HumanAddr("NONE!!! All entries had weight zero. Lottery restarted".to_string()),
            })?),
        })
    }

    let dist = WeightedIndex::new(&weights).unwrap();

    let sample = dist.sample(&mut rng);
    let winner = entries[sample];
    let  winner_human = deps.api.human_address(&winner)?;

    // restart the lottery in the next block
    a_lottery.start_time = &env.block.time + 10;
    a_lottery.end_time = &env.block.time + a_lottery.duration + 10;
    TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;



    //Redeeming amount from the staking contract
    let mut messages: Vec<CosmosMsg> = vec![];

    messages.push(
        WasmMsg::Execute {
            contract_addr: config.staking_contract.address,
            callback_code_hash: config.staking_contract.contract_hash,
            msg: to_binary(&LPStakingHandleMsg::Redeem {
                amount: redeeming_amount
            })?,
            send: vec![],
        }
            .into()
    );

    let mut user = TypedStore::<UserInfo, S>::attach(&deps.storage).load(winner_human.0.as_bytes()).unwrap(); // NotFound is the only possible error
    user.total_won += winning_amount;
    user.available_tokens_for_withdraw += winning_amount;
    user.winning_history.push((winning_amount.0 as u64, env.block.time));
    TypedStoreMut::<UserInfo, S>::attach(&mut deps.storage).store(winner_human.0.as_bytes(), &user)?;

    let mut last_lottery_result:LastLotteryResults = TypedStore::attach(&deps.storage).load(LAST_LOTTERY_KEY)?;
    last_lottery_result.past_number_of_entries.push(a_lottery.entries.len() as u64);
    last_lottery_result.past_rewards.push(((winning_amount.0 as u64), env.block.time));
    last_lottery_result.past_total_deposits.push(supply_pool.total_tokens_staked.0 as u64);
    last_lottery_result.past_winners.push(winner_human.0.clone());
    TypedStoreMut::attach(&mut deps.storage).store(LAST_LOTTERY_KEY,&last_lottery_result)?;


    Ok(HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ClaimRewards {
            status: Success,
            winner: winner_human,
        })?),
    })

}


fn lottery_adjustment<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    mut withdraw_amount: Uint128,
    sender_address: CanonicalAddr,
) -> StdResult<Lottery> {
    let mut a_lottery:Lottery = TypedStore::attach(&deps.storage).load(LOTTERY_KEY)?;
    let results: Vec<(CanonicalAddr, Uint128, u64)> = (a_lottery.entries).into_iter().map(|(address, mut user_staked_amount, deposit_time)|
        if address == sender_address {
            if user_staked_amount == withdraw_amount {
                user_staked_amount = Uint128(0);
                withdraw_amount = Uint128(0);
            } else if user_staked_amount < withdraw_amount {
                withdraw_amount = (withdraw_amount - user_staked_amount).unwrap();
                user_staked_amount = Uint128(0);
            } else if user_staked_amount > withdraw_amount {
                user_staked_amount = (user_staked_amount - withdraw_amount).unwrap();
            }
            (address, user_staked_amount, deposit_time)
        } else {
            (address, user_staked_amount, deposit_time)
        }
    ).collect();
    a_lottery.entries = results;
    a_lottery.entries.retain(|(_, amount, _)| amount != &Uint128(0));
    Ok(a_lottery)
}

fn triggering_cost_withdraw<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let  config = TypedStore::<Config, S>::attach(&deps.storage).load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    let mut supply_pool: SupplyPool = TypedStore::attach(& deps.storage).load(SUPPLY_POOL_KEY)?;
    if supply_pool.triggering_cost <= Uint128(0)
    {
        return Err(StdError::generic_err("Triggerer share not sufficient"));
    }

    let messages: Vec<CosmosMsg> = vec![
        // Transfer Trigger fee to triggerer wallet
        transfer_msg(
            env.message.sender,
            supply_pool.triggering_cost,
            None,
            RESPONSE_BLOCK_SIZE,
            config.token.contract_hash.clone(),
            config.token.address.clone(),
        )?
    ];

    supply_pool.triggering_cost = Uint128(0);
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;

    let res = HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::TriggeringCostWithdraw { status: Success })?),
    };
    Ok(res)
}



fn query_pending_rewards<S: Storage, A: Api, Q: Querier>(
    deps: & Extern<S, A, Q>,
    env:&Env,
    config: &Config,
) -> StdResult<LPStakingRewardsResponse> {

    let staking_rewards_response: LPStakingRewardsResponse = LPStakingQueryMsg::Rewards {
        address: env.clone().contract.address,
        key: STAKING_VK.to_string(),
        height: env.block.height,
    }.query(&deps.querier, config.staking_contract.contract_hash.clone(), config.staking_contract.address.clone())?;

   Ok( staking_rewards_response)


}





fn change_admin<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    address: HumanAddr,
) -> StdResult<HandleResponse> {
    let mut config = TypedStore::<Config, S>::attach(&deps.storage).load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;
    config.admin = address;
    TypedStoreMut::attach(&mut deps.storage).store(CONFIG_KEY, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeAdmin { status: Success })?),
    })
}

fn is_admin(config: &Config, account: &HumanAddr) -> StdResult<bool> {
    if &config.admin != account {
        return Ok(false);
    }

    Ok(true)
}

fn check_if_admin(config: &Config, account: &HumanAddr) -> StdResult<()> {
    if !is_admin(config, account)? {
        return Err(StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));
    }

    Ok(())
}

fn change_triggerer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    address: HumanAddr,
) -> StdResult<HandleResponse> {
    let mut config = TypedStore::<Config, S>::attach(&deps.storage).load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;
    config.triggerer = address;
    TypedStoreMut::<Config, S>::attach(&mut deps.storage).store(CONFIG_KEY, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeTriggerer { status: Success })?),
    })
}

fn change_lottery_duration<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    duration: u64,
) -> StdResult<HandleResponse> {
    let config = TypedStore::<Config, S>::attach(&deps.storage).load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;

    let mut a_lottery:Lottery = TypedStore::attach(&deps.storage).load(LOTTERY_KEY)?;
    a_lottery.duration = duration;
    TypedStoreMut::attach(&mut deps.storage).store(LOTTERY_KEY,&a_lottery)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeLotteryDuration { status: Success })?),
    })
}

fn change_triggerer_share<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    percentage: u64,
) -> StdResult<HandleResponse> {
    let mut config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    config.triggerer_share_percentage = percentage;
    TypedStoreMut::attach(&mut deps.storage).store(CONFIG_KEY, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeTriggererShare { status: Success })?),
    })
}

pub fn change_staking_contract<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    address: HumanAddr,
    contract_hash: String,
) -> StdResult<HandleResponse> {
    let mut config = TypedStore::attach(& deps.storage).load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;


    config.staking_contract = SecretContract {
        address,
        contract_hash,
    };
    TypedStoreMut::attach(&mut deps.storage).store(CONFIG_KEY, &config)?;

    return Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeStakingContract {
            status: Success,
        })?),
    });
}


fn is_triggerer(config: &Config, account: &HumanAddr) -> StdResult<bool> {
    if &config.triggerer != account {
        return Ok(false);
    }
    Ok(true)
}

fn check_if_triggerer(config: &Config, account: &HumanAddr) -> StdResult<()> {
    if !is_triggerer(config, account)? {
        return Err(StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address and triggerer address",
        ));
    }
    Ok(())
}





fn emergency_redeem_from_staking<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;

    if !config.is_stopped{
        return Err(StdError::generic_err(format!(
            "Need to stop contract first"
        )));
    }

    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps,&env,&config)?;

    let supply_store = TypedStore::attach(&deps.storage);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;

    config.stopped_emergency_redeem_jackpot = supply_pool.pending_staking_rewards + supply_pool.total_tokens_staked;
    TypedStoreMut::attach(&mut deps.storage).store(CONFIG_KEY, &config)?;

    let amount_to_redeem = supply_pool.total_rewards_restaked + supply_pool.total_tokens_staked;
    supply_pool.pending_staking_rewards += staking_rewards_response.rewards.rewards;
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;


    let mut messages: Vec<CosmosMsg> = vec![];
    messages.push(
        WasmMsg::Execute {
            contract_addr: config.staking_contract.address,
            callback_code_hash: config.staking_contract.contract_hash,
            msg: to_binary(&LPStakingHandleMsg::Redeem {
                amount: amount_to_redeem
            })?,
            send: vec![],
        }
            .into()
    );

    Ok(HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::EmergencyRedeemFromStaking {
            status: Success,
        })?),
    })
}

pub fn redelegate_to_contract<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;

    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps,&env,&config)?;

    let mut lp_pending_staking_rewards = Uint128(0);
    if staking_rewards_response.rewards.rewards > Uint128(0) {
        lp_pending_staking_rewards = staking_rewards_response.rewards.rewards
    }

    let mut supply_pool: SupplyPool = TypedStore::attach(&deps.storage).load(SUPPLY_POOL_KEY)?;
    let amount_to_restake = config.stopped_emergency_redeem_jackpot + supply_pool.pending_staking_rewards;

    config.stopped_emergency_redeem_jackpot = Uint128(0);
    TypedStoreMut::attach(&mut deps.storage).store(CONFIG_KEY, &config)?;

    supply_pool.total_rewards_restaked += supply_pool.pending_staking_rewards;
    supply_pool.pending_staking_rewards = lp_pending_staking_rewards;
    TypedStoreMut::attach(&mut deps.storage).store(SUPPLY_POOL_KEY, &supply_pool)?;

    Ok(HandleResponse {
        messages: vec![
            send_msg(
                config.staking_contract.address,
                amount_to_restake,
                Some(to_binary(&LPStakingHandleMsg::Deposit {})?),
                None,
                RESPONSE_BLOCK_SIZE,
                config.token.contract_hash.clone(),
                config.token.address.clone(),
            )?
        ],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::RedelegateToContract {
            status: Success,
        })?),
    })
}



/// validate_start_height returns an error if the lottery hasn't started
fn validate_start_time(start_time: u64, current_time: u64) -> StdResult<()> {
    if current_time < start_time {
        Err(StdError::generic_err("Lottery start height is in the future"))
    } else {
        Ok(())
    }
}

/// validate_end_height returns an error if the lottery ends in the future
fn validate_end_time(end_time: u64, current_time: u64) -> StdResult<()> {
    if current_time < end_time {
        Err(StdError::generic_err("Lottery end height is in the future"))
    } else {
        Ok(())
    }
}


fn query_contract_status<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
) -> StdResult<Binary> {
    let config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;

    to_binary(&QueryAnswer::ContractStatus {
        is_stopped: config.is_stopped,
    })
}

fn query_token<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    let config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;

    to_binary(&QueryAnswer::RewardToken {
        token: config.token,
    })
}

fn query_total_rewards<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>, height: Uint128) -> StdResult<Binary> {
    //Getting the pending_rewards
    let config: Config = TypedStore::attach(&deps.storage).load(CONFIG_KEY)?;

    let response: LPStakingRewardsResponse = LPStakingQueryMsg::Rewards {
        address: config.clone().own_addr,
        key: STAKING_VK.to_string(),
        height: height.0 as u64,
    }.query(&deps.querier, config.clone().staking_contract.contract_hash, config.clone().staking_contract.address)?;
    let rewards_in_lp_contract = response.rewards.rewards;

    let reward_pool = TypedStore::<SupplyPool, S>::attach(&deps.storage).load(SUPPLY_POOL_KEY)?;


    let total_rewards = rewards_in_lp_contract + reward_pool.total_rewards_restaked + reward_pool.pending_staking_rewards;

    to_binary(&QueryAnswer::TotalRewards {
        rewards: total_rewards,
    })
}

fn query_total_deposit<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    //Getting the pending_rewards
    let reward_pool = TypedStore::<SupplyPool, S>::attach(&deps.storage).load(SUPPLY_POOL_KEY)?;

    to_binary(&QueryAnswer::TotalDeposits {
        deposits: reward_pool.total_tokens_staked,
    })
}


fn query_deposit<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: &HumanAddr,
) -> StdResult<Binary> {
    let user = TypedStore::attach(&deps.storage)
        .load(address.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), winning_history: vec![] });

    to_binary(&QueryAnswer::Balance {
        amount: (user.amount_delegated),
    })
}

fn query_available_funds<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: &HumanAddr,
) -> StdResult<Binary> {
    let user = TypedStore::attach(&deps.storage)
        .load(address.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), winning_history: vec![] });

    to_binary(&QueryAnswer::AvailableTokensForWithdrawl {
        amount: (user.available_tokens_for_withdraw),
    })
}

fn query_user_past_records<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<Binary> {
    let user = TypedStore::attach(&deps.storage)
        .load(address.0.as_bytes())
        .unwrap_or(UserInfo {
            amount_delegated: Uint128(0),
            available_tokens_for_withdraw: Uint128(0),
            total_won: Uint128(0),
            winning_history: vec![],
        });

    to_binary(&QueryAnswer::UserPastRecords {
        winning_history: user.winning_history,
    })
}

fn query_all_past_results<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    //Getting the pending_rewards
    let  last_lottery_result:LastLotteryResults = TypedStore::attach(&deps.storage).load(LAST_LOTTERY_KEY)?;

    to_binary(&QueryAnswer::PastAllRecords {
        past_winners: last_lottery_result.past_winners,
        past_number_of_entries: last_lottery_result.past_number_of_entries,
        past_total_deposits: last_lottery_result.past_total_deposits,
        past_rewards: last_lottery_result.past_rewards,
    })
}

fn query_past_results<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    //Getting the pending_rewards
    let  last_lottery_result:LastLotteryResults = TypedStore::attach(&deps.storage).load(LAST_LOTTERY_KEY)?;
    let mut lenght= last_lottery_result.past_winners.len();
    if (lenght>=5){
        lenght=5
    }

    to_binary(&QueryAnswer::PastRecords {
        past_winners: last_lottery_result.past_winners[(last_lottery_result.past_winners.len() - lenght)..].to_owned(),
        past_number_of_entries: last_lottery_result.past_number_of_entries[(last_lottery_result.past_number_of_entries.len() - lenght)..].to_owned(),
        past_total_deposits: last_lottery_result.past_total_deposits[(last_lottery_result.past_total_deposits.len() - lenght)..].to_owned(),
        past_rewards: last_lottery_result.past_rewards[(last_lottery_result.past_rewards.len() - lenght)..].to_owned(),
    })

}




#[cfg(test)]
mod tests {
    use cosmwasm_std::{StdResult, InitResponse, Extern, to_binary, Uint128, HumanAddr, Coin, Env, BlockInfo, MessageInfo, ContractInfo, Querier, Binary, from_binary, ReadonlyStorage, QuerierResult, StdError, };
    use cosmwasm_std::testing::{MockStorage, MockApi, MockQuerier, mock_dependencies, MOCK_CONTRACT_ADDR};
    use secret_toolkit::storage::{TypedStoreMut, TypedStore};
    use crate::state::{Config, UserInfo, SupplyPool, Lottery,SecretContract};
    use crate::constants::{CONFIG_KEY, VIEWING_KEY_KEY, SUPPLY_POOL_KEY, STAKING_VK,  LOTTERY_KEY};
    use crate::contract::{init, handle, deposit, claim_rewards, query, trigger_withdraw, withdraw, check_if_admin, check_if_triggerer, change_admin, change_triggerer, query_past_results, query_all_past_results};
    use crate::msg::{HandleMsg, HandleAnswer, ResponseStatus, InitMsg, LPStakingRewardsResponse, RewardsInfo, QueryMsg, QueryAnswer, LPStakingQueryMsg};
    use crate::viewing_keys::{ViewingKey};
    use cosmwasm_storage::PrefixedStorage;
    use secret_toolkit::utils::Query;
    use std::any::Any;
    use cosmwasm_std::QueryResponse;


    fn extract_error_msg<T: Any>(error: StdResult<T>) -> String {
    match error {
        Ok(response) => {
            let bin_err = (&response as &dyn Any)
                .downcast_ref::<QueryResponse>()
                .expect("An error was expected, but no error could be extracted");
            match from_binary(bin_err).unwrap() {
                QueryAnswer::ViewingKeyError { msg } => msg,
                _ => panic!("Unexpected query answer"),
            }
        }
        Err(err) => match err {
            StdError::GenericErr { msg, .. } => msg,
            _ => panic!("Unexpected result from init"),
        },
    }
}


    fn init_helper(amount: Option<u128>) -> (
        StdResult<InitResponse>,
        Extern<MockStorage, MockApi, MockQuerier>,
    ) {
        let mut deps = mock_dependencies(20, &[Coin {
            amount: Uint128(amount.unwrap_or(0)),
            denom: "sefi".to_string(),
        }]);
        let env = mock_env("admin", &[], 0);

        let init_msg = InitMsg {
            admin: Option::from(HumanAddr("admin".to_string())),
            triggerer: Option::from(HumanAddr("triggerer".to_string())),
            token: SecretContract {
                address: HumanAddr("sefi".to_string()),
                contract_hash: "".to_string(),
            },
            viewing_key: "123".to_string(),
            staking_contract: SecretContract {
                address: HumanAddr("staking_contract".to_string()),
                contract_hash: "".to_string(),
            },

            prng_seed: Binary::from("I'm Batman".as_bytes()),
            triggerer_share_percentage: 1
        };

        (init(&mut deps, env, init_msg), deps)
    }

    /// Just set sender and sent funds for the message. The rest uses defaults.
    /// The sender will be canonicalized internally to allow developers passing in human readable senders.
    /// This is intended for use in test code only.
    pub fn mock_env<U: Into<HumanAddr>>(sender: U, sent: &[Coin], time: u64) -> Env {
        Env {
            block: BlockInfo {
                height: time,
                time,
                chain_id: "secret-testnet".to_string(),
            },
            message: MessageInfo {
                sender: sender.into(),
                sent_funds: sent.to_vec(),
            },
            contract: ContractInfo {
                address: HumanAddr::from(MOCK_CONTRACT_ADDR),
            },
            contract_key: Some("".to_string()),
            contract_code_hash: "".to_string(),
        }
    }

    pub struct MyMockQuerier {}

    impl Querier for MyMockQuerier {

        fn raw_query(&self, _request: &[u8]) -> QuerierResult {
            let response = LPStakingRewardsResponse {
                rewards: RewardsInfo {
                    rewards: Uint128(1000)
                }
            };
            Ok(to_binary(&response))
        }
    }




    #[test]
    fn testing_deposit() {
        let (_init_result,  deps) = init_helper(None);
        let env = mock_env("sef", &[], 601);

        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        //1)Checking if wrong token is supported
        let response = deposit(&mut mocked_deps, mock_env("sef", &[], 0), HumanAddr("Batman".to_string()), Uint128(1000000)).unwrap_err();
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        assert_eq!(response, StdError::generic_err(format!(
            "This token is not supported. Supported: {}, given: {}",
            config.token.address, env.message.sender
        )));

        //2 If amount less than 1 scrt or 1000000 uscrt
        let response = deposit(&mut mocked_deps, mock_env("sefi", &[], 0), HumanAddr("Batman".to_string()), Uint128(1)).unwrap_err();
        assert_eq!(response, StdError::generic_err(
            "Must deposit a minimum of 1000000 usefi, or 1 sefi",
        ));

        //3)Final checking
        let _response = deposit(&mut mocked_deps, mock_env("sefi", &[], 0), HumanAddr("Batman".to_string()), Uint128(100000000)).unwrap();

        //checking the amount delegated
        let user: UserInfo = TypedStoreMut::attach(&mut mocked_deps.storage).load(HumanAddr("Batman".to_string()).0.as_bytes()).unwrap();
        assert_eq!(user.amount_delegated, Uint128(100000000));
        assert_eq!(user.available_tokens_for_withdraw, Uint128(0));


        //checking total supply stats
        let supply_pool: SupplyPool = TypedStoreMut::attach(&mut mocked_deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.total_tokens_staked, Uint128(100000000));
        assert_eq!(supply_pool.pending_staking_rewards, Uint128(1000));
        assert_eq!(supply_pool.total_rewards_restaked, Uint128(0));


        //checking lottery
        let  a_lottery:Lottery = TypedStore::attach(&mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.entries.len(), 1);
        assert_eq!(a_lottery.duration, 600);
        assert_eq!(a_lottery.start_time, 1);
        assert_eq!(a_lottery.end_time, 601);

        //checking config
        let _config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        // println!("BORROW {:?}",*config.staking_contract.address.borrow());
        // println!("ORIGINAL {:?}",config.staking_contract.address);


        let (_init_result,  deps) = init_helper(None);
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        //deposits
        {
            let env = mock_env("sefi", &[], 601);
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(100000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
        }

        //checking the amount delegated
        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load(HumanAddr("Batman".to_string()).0.as_bytes()).unwrap();
        assert_eq!(user.amount_delegated, Uint128(100000000));
        assert_eq!(user.available_tokens_for_withdraw, Uint128(0));

        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load(HumanAddr("Loki".to_string()).0.as_bytes()).unwrap();
        assert_eq!(user.amount_delegated, Uint128(1000000));
        assert_eq!(user.available_tokens_for_withdraw, Uint128(0));


        // checking total supply stats
        let supply_pool: SupplyPool = TypedStoreMut::attach(&mut mocked_deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.total_tokens_staked, Uint128(108000000));
        assert_eq!(supply_pool.pending_staking_rewards, Uint128(1000));
        assert_eq!(supply_pool.total_rewards_restaked, Uint128(8000));

        //checking lottery
        let  a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.entries.len(), 9);
        assert_eq!(a_lottery.duration, 600);
        assert_eq!(a_lottery.start_time, 1);
        assert_eq!(a_lottery.end_time, 601);
    }

    #[test]
    fn test_claim_rewards() {
        //1)Checking for errors
        let (_init_result, deps) = init_helper(None);
        let _env = mock_env("triggerer", &[], 700);


        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let res = claim_rewards(&mut mocked_deps,_env);
            // println!("{:?}",res.unwrap());



            let _env = mock_env("sefi", &[], 0);
            deposit(&mut mocked_deps, mock_env("sefi", &[], 0), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 10), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 20), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 30), HumanAddr("Batman".to_string()), Uint128(1000000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 40), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 50), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 60), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 70), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 1800), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
        }
        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        let _entries: Vec<_> = (&a_lottery.entries).into_iter().map(|(k, _, _)| k).collect();

        let weights: Vec<u128> = (&a_lottery.entries).into_iter().map(|(_, user_staked_amount, deposit_time)|
            if (&a_lottery.end_time <= deposit_time) {
                0 as u128
            }
            else if ((&a_lottery.end_time - deposit_time) / &a_lottery.duration) >= 1 {
                user_staked_amount.0
            } else {
                (user_staked_amount.0 / 1000000) * ((((a_lottery.end_time - deposit_time) * 1000000) / a_lottery.duration) as u128)
            }
        ).collect();

        println!("WEIGHTS>>>>>>>>{:?}",weights);
        // println!("{:?}",a_lottery.end_time);

        let env = mock_env("triggerer", &[], a_lottery.end_time);
        let response = claim_rewards(&mut mocked_deps, env);

        let winner = match from_binary(&response.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::ClaimRewards { status: ResponseStatus::Success, winner: winner_addr } => winner_addr,
            _ => panic!("Unexpected result from handle"),
        };
        // println!("{:?}",winner);


        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load(winner.0.as_bytes()).unwrap();
        assert_eq!(user.available_tokens_for_withdraw.0, 9900);
        assert_eq!(user.total_won.0, 9900);
        assert_eq!(user.amount_delegated.0, 1000000000);

        let supply_pool: SupplyPool = TypedStore::attach(&mocked_deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.total_rewards_restaked.0, 0);
        assert_eq!(supply_pool.pending_staking_rewards.0, 0);
        assert_eq!(supply_pool.total_tokens_staked.0, 1008000000);

        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.start_time, 1320);
        assert_eq!(a_lottery.end_time, 1920);
    }

    #[test]
    fn claim_rewards_2(){
        //trying on errors
        //1)Checking for errors
        let (_init_result, deps) = init_helper(None);
        let _env = mock_env("triggerer", &[], 0);


        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {




            let _env = mock_env("sefi", &[], 0);
            deposit(&mut mocked_deps, mock_env("sefi", &[], 1000), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 1000), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 2000), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 3000), HumanAddr("Batman".to_string()), Uint128(1000000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 4000), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 5000), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 6000), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 7000), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, mock_env("sefi", &[], 1800), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
        }
        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        let _entries: Vec<_> = (&a_lottery.entries).into_iter().map(|(k, _, _)| k).collect();

        let weights: Vec<u128> = (&a_lottery.entries).into_iter().map(|(_, user_staked_amount, deposit_time)|
            if (&a_lottery.end_time <= deposit_time) {
                0 as u128
            }
            else if ((&a_lottery.end_time - deposit_time) / &a_lottery.duration) >= 1 {
                user_staked_amount.0
            } else {
                (user_staked_amount.0 / 1000000) * ((((a_lottery.end_time - deposit_time) * 1000000) / a_lottery.duration) as u128)
            }
        ).collect();

        println!("WEIGHTS>>>>>>>>{:?}",weights);

        let env = mock_env("triggerer", &[], a_lottery.end_time);
        let response = claim_rewards(&mut mocked_deps, env);

        let winner = match from_binary(&response.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::ClaimRewards { status: ResponseStatus::Failure, winner: winner_addr } => winner_addr,
            _ => panic!("Unexpected result from handle"),
        };

        println!("Winner{:?}",winner);

    }

    #[test]
    fn test_trigger_withdraw() {
        let (_init_result, deps) = init_helper(None);
        let _env = mock_env("sefi", &[], 0);


        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 601);
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
        }


        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.entries.len(), 10);
        // println!("{:?}",a_lottery.entries);
        let _res = trigger_withdraw(&mut mocked_deps, mock_env("Batman", &[], 0), Option::from(Uint128(60000000))).unwrap();
        // print!("{:?}",res);

        let a_lottery :Lottery= TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.entries.len(), 9);
        // println!("{:?}",a_lottery.entries);


        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load("Batman".as_bytes()).unwrap();
        assert_eq!(user.available_tokens_for_withdraw.0, 60000000);
        assert_eq!(user.total_won.0, 0);
        assert_eq!(user.amount_delegated.0, 40000000);

        let supply_pool: SupplyPool = TypedStore::attach(&mocked_deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.total_rewards_restaked.0, 9000);
        assert_eq!(supply_pool.pending_staking_rewards.0, 2000);
        assert_eq!(supply_pool.total_tokens_staked.0, 48000000);
    }


    #[test]
    fn test_redelegate() {
        //1)Checking for errors
        let (_init_result, deps) = init_helper(None);

        let _env = mock_env("sefi", &[], 0);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 0);
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
        }
        let env = mock_env("triggerer", &[], 601);
        let response = claim_rewards(&mut mocked_deps, env.clone());


        let winner = match from_binary(&response.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::ClaimRewards { status: ResponseStatus::Success, winner: winner_addr } => winner_addr,
            _ => panic!("Unexpected result from handle"),
        };

        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load("Batman".as_bytes()).unwrap();
        assert_eq!(Uint128(100000000), user.amount_delegated);
        assert_eq!(Uint128(10890), user.available_tokens_for_withdraw);


        let handle_msg = HandleMsg::Redelegate {
            amount: None
        };
        let _ = handle(&mut mocked_deps, mock_env(winner.clone(), &[], 601), handle_msg);
        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load("Batman".as_bytes()).unwrap();


        assert_eq!(Uint128(100010890), user.amount_delegated);
        assert_eq!(Uint128(0), user.available_tokens_for_withdraw);
    }


    #[test]
    fn test_trigger_withdraw_part_two() {
        //1)Checking for errors
        let (_init_result, deps) = init_helper(None);

        let _env = mock_env("sefi", &[], 0);


        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 0);
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
        }
        let env = mock_env("Batman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env, Option::from(Uint128(60000000)));
        // println!("{:?}",res.unwrap());

        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load("Batman".as_bytes()).unwrap();
        assert_eq!(Uint128(40000000), user.amount_delegated);
        assert_eq!(Uint128(60000000), user.available_tokens_for_withdraw);

        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.entries.len(), 9);

        let supply_pool: SupplyPool = TypedStore::attach(&mocked_deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.total_rewards_restaked.0, 9000);
        assert_eq!(supply_pool.pending_staking_rewards.0, 2000);
        assert_eq!(supply_pool.total_tokens_staked.0, 48000000);


        let handle_msg = HandleMsg::Redelegate {
            amount: None
        };
        let _ = handle(&mut mocked_deps, mock_env("Batman", &[], 601), handle_msg);
        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load("Batman".as_bytes()).unwrap();

        assert_eq!(Uint128(100000000), user.amount_delegated);
        assert_eq!(Uint128(0), user.available_tokens_for_withdraw);
    }

    #[test]
    fn test_withdraw() {
        //1)Checking for errors
        let (_init_result,  deps) = init_helper(None);
        let _env = mock_env("sefi", &[], 0);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 0);
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
        }
        let env = mock_env("Batman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env.clone(), Option::from(Uint128(60000000)));
        let _=withdraw(&mut mocked_deps, env, None);
        let user: UserInfo = TypedStore::attach(&mocked_deps.storage).load("Batman".as_bytes()).unwrap();
        assert_eq!(user.available_tokens_for_withdraw.0, 0);
        assert_eq!(user.amount_delegated.0, 40000000);
    }


    #[test]
    fn test_handle_create_viewing_key() {
        let (_init_result, mut deps) = init_helper(None);
        let handle_msg = HandleMsg::CreateViewingKey {
            entropy: "ghgxfhgfhgfhfghdfgfhfghfggh".to_string(),
            padding: None,
        };
        let handle_result = handle(&mut deps, mock_env("bob", &[], 601), handle_msg);
        assert!(
            handle_result.is_ok(),
            "handle() failed: {}",
            handle_result.err().unwrap()
        );
        let answer: HandleAnswer = from_binary(&handle_result.unwrap().data.unwrap()).unwrap();

        let key = match answer {
            HandleAnswer::CreateViewingKey { key } => key,
            _ => panic!("NOPE"),
        };


        let vk_store = PrefixedStorage::new(VIEWING_KEY_KEY, &mut deps.storage);
        let saved_vk = vk_store.get("bob".as_bytes()).unwrap();

        assert!(key.check_viewing_key(saved_vk.as_slice()));
    }

    #[test]
    fn test_handle_set_viewing_key() {
        let (_init_result, mut deps) = init_helper(None);

        let handle_msg = HandleMsg::SetViewingKey {
            key: "just_a_key".to_string(),
            padding: None,
        };
        let handle_result = handle(&mut deps, mock_env("bob", &[], 601), handle_msg);
        assert!(
            handle_result.is_ok(),
            "handle() failed: {}",
            handle_result.err().unwrap()
        );
        let _answer: HandleAnswer = from_binary(&handle_result.unwrap().data.unwrap()).unwrap();

        let vk = ViewingKey("just_a_key".to_string());
        let vk_store = PrefixedStorage::new(VIEWING_KEY_KEY, &mut deps.storage);
        let saved_vk = vk_store.get("bob".as_bytes()).unwrap();


        assert_eq!(saved_vk, &vk.to_hashed());
    }
    //testing Queries

    #[test]
    fn test_query_lottery() {
        let (_init_result, mut deps) = init_helper(None);

        let a_lottery:Lottery = TypedStoreMut::attach(&mut deps.storage).load(LOTTERY_KEY).unwrap();
        let query_msg = QueryMsg::LotteryInfo {};
        let query_result = query(&deps, query_msg);


        let (start_height, end_height, duration) = match from_binary(&query_result.unwrap()).unwrap() {
            QueryAnswer::LotteryInfo {
                start_time: start,
                end_time: end, duration
            } => (start, end, duration),
            _ => panic!("Unexpected result from handle"),
        };

        assert_eq!(a_lottery.end_time, end_height);
        assert_eq!(a_lottery.start_time, start_height);
        assert_eq!(a_lottery.duration, duration);
    }


    #[test]
    fn test_query_total_rewards() {
        let (_init_result, deps) = init_helper(None);
        let env = mock_env("sefi", &[], 601);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(9000000000)).unwrap();//c.p:1000 deposit:0
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(6000000000)).unwrap();//c.p:1000 deposit:1000
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(7000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(100000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(9000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(2000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(2000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(2000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(2000000000)).unwrap();//c.p:1000 deposit:8000


        let height = Uint128(env.block.height as u128);

        let query_msg = QueryMsg::TotalRewards { height };
        let query_result = query(&mocked_deps, query_msg);

        let _total_rewards = match from_binary(&query_result.unwrap()).unwrap() {
            QueryAnswer::TotalRewards { rewards } => (rewards),
            _ => panic!("Unexpected result from handle"),
        };
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();

        let response: LPStakingRewardsResponse = LPStakingQueryMsg::Rewards {
            address: env.contract.address,
            key: STAKING_VK.to_string(),
            height: env.block.height,
        }.query(&mocked_deps.querier, config.clone().staking_contract.contract_hash, config.clone().staking_contract.address).unwrap();
        let _rewards_in_lp_contract = response.rewards.rewards;

        let _reward_pool: SupplyPool = TypedStore::attach(&mocked_deps.storage).load(SUPPLY_POOL_KEY).unwrap();

        // let current_round: RoundStruct = round_read(& mocked_deps.storage).load().unwrap();

        //3)Checking rewards_pool
        // assert_eq!(total_rewards,reward_pool.total_rewards_restaked +rewards_in_lp_contract+current_round.pending_staking_rewards);
    }

    #[test]
    fn test_query_total_deposits() {
        let (_init_result, deps) = init_helper(None);
        let env = mock_env("sefi", &[], 601);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(9000000000)).unwrap();//c.p:1000 deposit:0
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(6000000000)).unwrap();//c.p:1000 deposit:1000
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(7000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(100000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(9000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(2000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(2000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(2000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(2000000000)).unwrap();//c.p:1000 deposit:8000


        let query_msg = QueryMsg::TotalDeposits {};
        let query_result = query(&mocked_deps, query_msg);

        let total_deposits = match from_binary(&query_result.unwrap()).unwrap() {
            QueryAnswer::TotalDeposits { deposits } => (deposits),
            _ => panic!("Unexpected result from handle"),
        };

        assert_eq!(total_deposits, Uint128(139000000000))
    }

    // Query tests
    #[test]
    fn test_authenticated_queries() {
        let (_init_result, deps) = init_helper(None);
        let env = mock_env("sefi", &[], 601);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();


        let no_vk_yet_query_msg = QueryMsg::Balance {
            address: HumanAddr("batman".to_string()),
            key: "no_vk_yet".to_string(),
        };
        let query_result = query(&mocked_deps, no_vk_yet_query_msg);

        let error = extract_error_msg(query_result);
        assert_eq!(
            error,
            "Wrong viewing key for this address or viewing key not set".to_string()
        );

        // print!("this is an error{}",error);

        let create_vk_msg = HandleMsg::CreateViewingKey {
            entropy: "heheeehe".to_string(),
            padding: None,
        };
        let handle_response = handle(&mut mocked_deps, mock_env("batman", &[], 601), create_vk_msg).unwrap();
        let vk = match from_binary(&handle_response.data.unwrap()).unwrap() {
            HandleAnswer::CreateViewingKey { key } => key,
            _ => panic!("Unexpected result from handle"),
        };

        let query_balance_msg = QueryMsg::Balance {
            address: HumanAddr("batman".to_string()),
            key: vk.0,
        };

        let query_response = query(&mocked_deps, query_balance_msg).unwrap();
        let balance = match from_binary(&query_response).unwrap() {
            QueryAnswer::Balance { amount } => amount,
            _ => panic!("Unexpected result from query"),
        };
        assert_eq!(balance, Uint128(5000000000));
        let wrong_vk_query_msg = QueryMsg::Balance {
            address: HumanAddr("batman".to_string()),
            key: "wrong_vk".to_string(),
        };
        let query_result = query(&mocked_deps, wrong_vk_query_msg);
        let error = extract_error_msg(query_result);
        assert_eq!(
            error,
            "Wrong viewing key for this address or viewing key not set".to_string()
        );
    }

    #[test]
    fn test_query_past_results() {
        let (_init_result, deps) = init_helper(None);

        let _env = mock_env("sefi", &[], 0);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 0);
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Spiderman".to_string()), Uint128(100000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(100000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(100000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Blackwidow".to_string()), Uint128(100000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
            // deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(100000000)).unwrap();//c.p:1000 deposit:8000
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
        }
        // let _env = mock_env("triggerer", &[], 601);
        // let env = mock_env("triggerer", &[], 700);
        // let _res = claim_rewards(&mut mocked_deps, env).unwrap();
        // let env = mock_env("triggerer", &[], 1700);
        // let _res = claim_rewards(&mut mocked_deps, env).unwrap();
        // let env = mock_env("triggerer", &[], 2700);
        // let _res = claim_rewards(&mut mocked_deps, env);
        // let env = mock_env("triggerer", &[], 3700);
        // let res1 = claim_rewards(&mut mocked_deps, env);
        // let _winner1 = match from_binary(&res1.unwrap().data.unwrap()).unwrap() {
        //     HandleAnswer::ClaimRewardPool { status, winner } => winner,
        //     _ => panic!("Unexpected result from handle"),
        // };
        // // println!("Winner 1 .....................{:?}",winner1);

        let env = mock_env("triggerer", &[], 4700);
        let _res2 = claim_rewards(&mut mocked_deps, env);
        let env = mock_env("triggerer", &[], 5700);
        let _res3 = claim_rewards(&mut mocked_deps, env);
        let env = mock_env("triggerer", &[], 6700);
        let _res4 = claim_rewards(&mut mocked_deps, env);
        let env = mock_env("triggerer", &[], 7700);
        let res5 = claim_rewards(&mut mocked_deps, env);
        let _winner5 = match from_binary(&res5.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::ClaimRewards { status, winner } => winner,
            _ => panic!("Unexpected result from handle"),
        };
            // println!("Winner 5 .....................{:?}",_winner5);

            let res:QueryAnswer =from_binary(&query_past_results(&mocked_deps).unwrap()).unwrap();
            println!("{:?}",res);

            let res:QueryAnswer =from_binary(&query_all_past_results(&mocked_deps).unwrap()).unwrap();
            println!("{:?}",res);
    }

    #[test]
    fn test_user_past_records() {
        let (_init_result, deps) = init_helper(None);
        let env = mock_env("sefi", &[], 600);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();


        let env = mock_env("triggerer", &[], 1000000);

        claim_rewards(&mut mocked_deps, env.clone()).unwrap();


        let no_vk_yet_query_msg = QueryMsg::Balance {
            address: HumanAddr("batman".to_string()),
            key: "no_vk_yet".to_string(),
        };
        let query_result = query(&mocked_deps, no_vk_yet_query_msg);

        let error = extract_error_msg(query_result);
        assert_eq!(
            error,
            "Wrong viewing key for this address or viewing key not set".to_string()
        );

        // print!("this is an error{}",error);

        let create_vk_msg = HandleMsg::CreateViewingKey {
            entropy: "heheeehe".to_string(),
            padding: None,
        };
        let handle_response = handle(&mut mocked_deps, mock_env("batman", &[], 601), create_vk_msg).unwrap();
        let vk = match from_binary(&handle_response.data.unwrap()).unwrap() {
            HandleAnswer::CreateViewingKey { key } => key,
            _ => panic!("Unexpected result from handle"),
        };

        let query_balance_msg = QueryMsg::UserPastRecords {
            address: HumanAddr("batman".to_string()),
            key: vk.0,
        };

        let query_response = query(&mocked_deps, query_balance_msg).unwrap();
        let _results: QueryAnswer = from_binary(&query_response).unwrap();
        // println!(".................................................................................................... {:?}",results);

        // println!("The balance is {:?}",results)
    }

    #[test]
    fn test_change_admin_triggerer() {
        let (_init_result, deps) = init_helper(None);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        let env = mock_env("not-admin", &[], 600);
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let res = check_if_admin(&config, &env.message.sender).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));

        let env = mock_env("admin", &[], 600);
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let res = check_if_admin(&config, &env.message.sender);
        assert_eq!(res, Ok(()));

        let env = mock_env("not-triggerer", &[], 600);
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let res = check_if_triggerer(&config, &env.message.sender).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address and triggerer address",
        ));

        let env = mock_env("triggerer", &[], 600);
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let res = check_if_triggerer(&config, &env.message.sender);
        assert_eq!(res, Ok(()));

        //change admin
        let env = mock_env("not-admin", &[], 600);
        let _config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let res = change_admin(&mut mocked_deps, env, HumanAddr("triggerer".to_string())).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));

        let env = mock_env("admin", &[], 600);
        let _config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let _res = change_admin(&mut mocked_deps, env, HumanAddr("someone".to_string())).unwrap();
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        assert_eq!(config.admin, HumanAddr("someone".to_string()));

        let env = mock_env("not-admin", &[], 600);
        let _config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let res = change_admin(&mut mocked_deps, env, HumanAddr("triggerer".to_string())).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));

        let env = mock_env("someone", &[], 600);
        let _config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        let _res = change_triggerer(&mut mocked_deps, env, HumanAddr("someone".to_string())).unwrap();
        let config: Config = TypedStore::attach(&mocked_deps.storage).load(CONFIG_KEY).unwrap();
        assert_eq!(config.triggerer, HumanAddr("someone".to_string()));
    }

    #[test]
    fn test_checking_contract_status() {
        //Contract balance > than
        let (_init_result,  deps) = init_helper(Some(500000000));

        let env = mock_env("sefi", &[], 600);

        // deposit rewards on the staking contract
        let mut deps = deps.change_querier(|_| MyMockQuerier {});

        deposit(&mut deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(500000000)).unwrap();

        let env = mock_env("admin", &[], 600);

        let handle_msg = HandleMsg::StopContract {};
        let _res = handle(&mut deps, env.clone(), handle_msg);

        let env = mock_env("Batman", &[], 600);


        let handle_msg = HandleMsg::TriggerWithdraw { amount: Option::from(Uint128(500000000)) };
        let res = handle(&mut deps, env.clone(), handle_msg);

        // assert_eq!(res.unwrap_err(), StdError::generic_err(
        //     "This contract is stopped and this action is not allowed",
        // ));


        let handle_msg = HandleMsg::StopContract {};
        let _res = handle(&mut deps, env.clone(), handle_msg);
        let env = mock_env("Batman", &[], 600);

        let handle_msg = HandleMsg::TriggerWithdraw { amount: Option::from(Uint128(500000000)) };
        let _res = handle(&mut deps, env, handle_msg);


        let env = mock_env("admin", &[], 600);
        let handle_msg = HandleMsg::StopContract {};
        let _res = handle(&mut deps, env.clone(), handle_msg);

        let env = mock_env("Batman", &[], 600);
        let handle_msg = HandleMsg::TriggerWithdraw { amount: Option::from(Uint128(500000000)) };
        let res = handle(&mut deps, env, handle_msg);
        assert_eq!(res.unwrap_err(), StdError::generic_err(
            "This contract is stopped and this action is not allowed",
        ));


        let env = mock_env("admin", &[], 600);
        let handle_msg = HandleMsg::ResumeContract {};
        let _res = handle(&mut deps, env.clone(), handle_msg);

        let env = mock_env("Batman", &[], 10000000);
        let handle_msg = HandleMsg::Withdraw { amount: Option::from(Uint128(500000000)) };
        let _res = handle(&mut deps, env, handle_msg);
    }

    #[test]
    fn testing_triggerer_withdraw_rewards() {
        //Depositing amount
        let (_init_result,  deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 600);

        // deposit rewards on the staking contract
        let mut deps = deps.change_querier(|_| MyMockQuerier {});

        deposit(&mut deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();
        deposit(&mut deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();

         let a_lottery:Lottery = TypedStoreMut::attach(&mut deps.storage).load(LOTTERY_KEY).unwrap();

        //Computing weights for the lottery
        let _lottery_entries: Vec<_> = (&a_lottery.entries).into_iter().map(|(address, _, _)| address).collect();

        let _response = claim_rewards(&mut deps, mock_env("triggerer", &[], 10000)).unwrap();

        let supply_pool: SupplyPool = TypedStoreMut::attach(&mut deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.triggering_cost, Uint128(30));

        let handlemsg = HandleMsg::TriggeringCostWithdraw {};
        let _res = handle(&mut deps, mock_env("admin", &[], 10), handlemsg);


        let supply_pool: SupplyPool = TypedStoreMut::attach(&mut deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.triggering_cost, Uint128(0));
    }

    #[test]
    fn testing_change_triggerer_share() {
        //Depositing amount
        let (_init_result,  deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 600);

        let mut deps = deps.change_querier(|_| MyMockQuerier {});
        deposit(&mut deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();
        deposit(&mut deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();


        let handlemsg = HandleMsg::ChangeTriggererShare { percentage: 2 };
        let _res = handle(&mut deps, mock_env("admin", &[], 10), handlemsg);
        let _response = claim_rewards(&mut deps, mock_env("triggerer", &[], 10000)).unwrap();

        let supply_pool: SupplyPool = TypedStoreMut::attach(&mut deps.storage).load(SUPPLY_POOL_KEY).unwrap();
        assert_eq!(supply_pool.triggering_cost, Uint128(60));
    }

    #[test]
    fn testing_change_lottery_duration() {
        //Depositing amount
        let (_init_result,  deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 0);

        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();

        let handlemsg = HandleMsg::ChangeLotteryDuration { duration: 100 };
        let _res = handle(&mut mocked_deps, mock_env("admin", &[], 10), handlemsg);

        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();

        assert_eq!(a_lottery.duration,100);

        let _response = claim_rewards(&mut mocked_deps, mock_env("triggerer", &[], 601)).unwrap();
        let a_lottery:Lottery = TypedStoreMut::attach(&mut mocked_deps.storage).load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.start_time,611);
        assert_eq!(a_lottery.end_time,711);

        //
    }


}
