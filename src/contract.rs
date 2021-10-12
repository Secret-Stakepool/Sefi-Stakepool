//Crate import
use crate::constants::*;
use crate::viewing_keys::{ViewingKey, VIEWING_KEY_SIZE};
use crate::state::{SupplyPool, UserInfo, Config, Lottery, LastLotteryResults, SecretContract, UserWinningHistory, LotteryEntries};
use crate::msg::{HandleAnswer, HandleMsg, InitMsg, LPStakingRewardsResponse, QueryAnswer, QueryMsg, LPStakingQueryMsg, LPStakingHandleMsg, ResponseStatus::Success, ResponseStatus::Failure};

//Cosmwasm import
use cosmwasm_storage::{PrefixedStorage, ReadonlyPrefixedStorage};
use cosmwasm_std::{Api, Binary, CosmosMsg, Env, Extern, HandleResponse, InitResponse, Querier, ReadonlyStorage, StdError, StdResult, Storage, Uint128, WasmMsg, from_binary, to_binary};
use cosmwasm_std::HumanAddr;
//secret toolkit import
use secret_toolkit::storage::{TypedStore, AppendStore, AppendStoreMut};
use secret_toolkit::snip20::{transfer_msg, send_msg};
use secret_toolkit::utils::{Query, pad_handle_result, pad_query_result};
use secret_toolkit::{crypto::sha_256, storage::TypedStoreMut, snip20};
use secret_toolkit::incubator::{GenerationalStore, GenerationalStoreMut};
use secret_toolkit::incubator::generational_store::Entry;

//Rust functions
use rand::prelude::*;
use sha2::{Digest, Sha256};
use rand_core::SeedableRng;
use rand_chacha::ChaChaRng;
use rand::distributions::WeightedIndex;
use std::borrow::Borrow;

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
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>, _>::attach(&mut config_prefixed);

    configstore.store(
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
            is_stopped_can_withdraw: false,
            own_addr: env.contract.address,
        },
    )?;

    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);

    supply_store.store(
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
    let duration = 86400u64;

    //Create first lottery
    // Save to state
    let mut lottery_prefixed = PrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mut deps.storage);
    let mut lottery_store = TypedStoreMut::<Lottery, PrefixedStorage<'_, S>>::attach(&mut lottery_prefixed);
    lottery_store.store(
        LOTTERY_KEY,
        &Lottery {
            entropy: prng_seed_hashed.to_vec(),
            start_time: time + 0,
            end_time: time + duration + 0,
            seed: prng_seed_hashed.to_vec(),
            duration,
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
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;

    if config.is_stopped {
        let response = match msg {
            //USER->Viewing Key
            HandleMsg::CreateViewingKey { entropy, .. } => { create_viewing_key(deps, env, entropy) }
            HandleMsg::SetViewingKey { key, .. } => set_viewing_key(deps, env, key),

            //Admin  ---> ChangeStakingContractFlow
            // => 1.StopContract 2.EmergencyRedeemFromStaking
            HandleMsg::EmergencyRedeemFromStaking {} => emergency_redeem_from_staking(deps, env),

            //Option 1)     3. Allow users to Withdraw their amount
            HandleMsg::AllowWithdrawWhenStopped {} => allow_withdraw_when_stopped(deps, env),
            HandleMsg::WithdrawExcess {} => withdraw_excess(deps, env),

            //Option 2)     3. Redelegate the contract
            HandleMsg::ChangeStakingContract { address, contract_hash } => change_staking_contract(deps, env, address, contract_hash),
            HandleMsg::RedelegateToNewContract {} => redelegate_to_contract(deps, env),

            HandleMsg::ResumeContract {} => resume_contract(deps, env),
            HandleMsg::TriggeringCostWithdraw {} => triggering_cost_withdraw(deps, env),

            //Allow withdraw
            HandleMsg::Withdraw { amount } => withdraw(deps, env, amount),

            _ => Err(StdError::generic_err(
                "This contract is stopped and this action is not allowed",
            )),
        };
        return pad_handle_result(response, RESPONSE_BLOCK_SIZE);
    }

    let response = match msg {

        // Triggerer
        HandleMsg::ClaimRewards {} => claim_rewards(deps, env),

        //USER
        HandleMsg::Receive { from, amount, msg, .. } => receive(deps, env, from, amount, msg),
        HandleMsg::Withdraw { amount } => withdraw(deps, env, amount),
        HandleMsg::TriggerWithdraw { amount } => trigger_withdraw(deps, env, amount),

        //USER->Viewing Key
        HandleMsg::CreateViewingKey { entropy, .. } => { create_viewing_key(deps, env, entropy) }
        HandleMsg::SetViewingKey { key, .. } => set_viewing_key(deps, env, key),

        //Admin
        HandleMsg::ChangeAdmin { admin } => change_admin(deps, env, admin),
        HandleMsg::ChangeTriggerer { admin } => change_triggerer(deps, env, admin),
        HandleMsg::ChangeTriggererShare { percentage, .. } => change_triggerer_share(deps, env, percentage),
        HandleMsg::ChangeLotteryDuration { duration } => change_lottery_duration(deps, env, duration),
        HandleMsg::TriggeringCostWithdraw {} => triggering_cost_withdraw(deps, env),
        HandleMsg::StopContract {} => stop_contract(deps, env),

        _ => Err(StdError::generic_err("Unavailable or unknown handle message")),
    };
    pad_handle_result(response, RESPONSE_BLOCK_SIZE)
}

pub fn query<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    msg: QueryMsg,
) -> StdResult<Binary> {
    let response = match msg {
        QueryMsg::LotteryInfo {} => {
            // query_lottery_info(&deps.storage)
            let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
            let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
            let config: Config = configstore.load(CONFIG_KEY)?;

            let lottery_prefixed = ReadonlyPrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &deps.storage);
            let lottery_store = TypedStore::<Lottery, ReadonlyPrefixedStorage<'_, S>>::attach(&lottery_prefixed);
            let lottery: Lottery = lottery_store.load(LOTTERY_KEY)?;
            to_binary(&QueryAnswer::LotteryInfo {
                start_time: lottery.start_time,
                end_time: lottery.end_time,
                duration: lottery.duration,
                is_stopped: config.is_stopped,
                is_stopped_with_withdraw: config.is_stopped_can_withdraw,
            })
        }
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
            QueryMsg::UserAllPastRecords { address, .. } => query_user_all_past_records(deps, address),

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
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
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

    // Checking that the sent tokens are from an expected contract address
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    if env.message.sender != config.token.address {
        return Err(StdError::generic_err(format!(
            "This token is not supported. Supported: {}, given: {}",
            config.token.address, env.message.sender
        )));
    }
    // Checking if the deposit is greater than 1 sefi
    if !valid_amount(amount_to_deposit) {
        return Err(StdError::generic_err(
            "Must deposit a minimum of 1000000 usefi, or 1 sefi",
        ));
    }

    //UPDATING USER DATA
    let user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, from.0.as_bytes()], &deps.storage);
    let user_store = TypedStore::attach(&user_prefixed);
    let mut user = user_store.load(from.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), entries: vec![], entry_index: vec![] }); // NotFound is the only possible error
    user.amount_delegated += amount_to_deposit;

    let mut lottery_entries = PrefixedStorage::multilevel(&[LOTTERY_ENTRY_KEY], &mut deps.storage);
    let mut lottery_entries_append = GenerationalStoreMut::<LotteryEntries, PrefixedStorage<S>>::attach_or_create(&mut lottery_entries)?;
    user.entry_index.push(lottery_entries_append.insert(LotteryEntries {
        user_address: from.clone(),
        amount: amount_to_deposit,
        entry_time: env.block.time,
    }));

    let mut user_prefixed = PrefixedStorage::multilevel(&[USER_INFO_KEY, from.0.as_bytes()], &mut deps.storage);
    let mut user_store = TypedStoreMut::attach(&mut user_prefixed);
    user_store.store(from.0.as_bytes(), &user)?;

    //QUERYING PENDING_REWARDS
    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps, &env, &config)?;
    //Updating Supply store
    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    let amount_to_stake = amount_to_deposit + supply_pool.pending_staking_rewards;
    supply_pool.total_tokens_staked += amount_to_deposit;
    supply_pool.total_rewards_restaked += supply_pool.pending_staking_rewards;

    if staking_rewards_response.rewards.rewards > Uint128(0) {
        supply_pool.pending_staking_rewards = staking_rewards_response.rewards.rewards;
    } else {
        supply_pool.pending_staking_rewards = Uint128(0);
    }
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;

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

    //LOADING USER INFO
    let user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, env.message.sender.0.as_bytes()], &deps.storage);
    let user_store = TypedStore::attach(&user_prefixed);
    let mut user = user_store
        .load(env.message.sender.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), entries: vec![], entry_index: vec![] });

    //If withdraw amount in not send then all delegated amount is unstaked
    let withdraw_amount = amount.unwrap_or(user.amount_delegated);

    //Checking if withdraw is possible or not
    if withdraw_amount <= Uint128(0) {
        return Err(StdError::generic_err("No sefi staked"));
    }
    if user.amount_delegated < withdraw_amount {
        return Err(StdError::generic_err("Trying to withdrawing more amount than staked"));
    }

    //Updating User Info
    user.amount_delegated = (user.amount_delegated - withdraw_amount).unwrap();
    user.available_tokens_for_withdraw += withdraw_amount;

    //Updating Lottery Entries
    let mut temp_withdraw_amount = withdraw_amount.clone();
    let mut lottery_entries = PrefixedStorage::multilevel(&[LOTTERY_ENTRY_KEY], &mut deps.storage);
    let mut lottery_entries_store = GenerationalStoreMut::<LotteryEntries, PrefixedStorage<S>>::attach_or_create(&mut lottery_entries)?;
    for ind in user.clone().entry_index {
        let entry = lottery_entries_store.get(ind.clone()).unwrap();
        if entry.amount == temp_withdraw_amount {
            temp_withdraw_amount = Uint128(0);
            let _ = lottery_entries_store.remove(ind.clone());
            user.entry_index.retain(|index| index.borrow().clone() != ind);
        } else if entry.amount < temp_withdraw_amount {
            temp_withdraw_amount = (temp_withdraw_amount - entry.amount).unwrap();
            let _ = lottery_entries_store.remove(ind.clone());
            user.entry_index.retain(|index| index.borrow().clone() != ind);
        } else {
            let _ = lottery_entries_store.update(ind, LotteryEntries {
                user_address: entry.user_address,
                amount: (entry.amount - temp_withdraw_amount).unwrap(),
                entry_time: entry.entry_time,
            });
            break;
        }
    }

    //Updating UserInfo
    let mut user_mut_prefixed = PrefixedStorage::multilevel(&[USER_INFO_KEY, env.message.sender.0.as_bytes()], &mut deps.storage);
    let mut user_mut_store = TypedStoreMut::attach(&mut user_mut_prefixed);
    user_mut_store.store(env.message.sender.0.as_bytes(), &user)?;

    //Updating Supply store
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps, &env, &config)?;
    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    if staking_rewards_response.rewards.rewards > Uint128(0) {
        supply_pool.pending_staking_rewards += staking_rewards_response.rewards.rewards
    }
    supply_pool.total_tokens_staked = (supply_pool.total_tokens_staked - withdraw_amount).unwrap();
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;

    //Sending message for Withdraw
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
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;

    //If contract is stopped but users are not allowed to withdraw
    if config.is_stopped && !config.is_stopped_can_withdraw {
        return Err(StdError::generic_err(
            "This contract is stopped and this action is not allowed",
        ));
    }

    let mut user_prefixed = PrefixedStorage::multilevel(&[USER_INFO_KEY, env.message.sender.0.as_bytes()], &mut deps.storage);
    let mut user_store = TypedStoreMut::<UserInfo, PrefixedStorage<'_, S>>::attach(&mut user_prefixed);
    let mut user = user_store
        .load(env.message.sender.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), entries: vec![], entry_index: vec![] }); // NotFound is the only possible error

    let withdraw_amount = amount.unwrap_or(user.available_tokens_for_withdraw);
    if withdraw_amount <= Uint128(0) {
        return Err(StdError::generic_err("No tokens available for withdraw"));
    }

    if !config.is_stopped_can_withdraw {
        if user.available_tokens_for_withdraw < withdraw_amount {
            return Err(StdError::generic_err("Withdrawing more amount than Available tokens for withdraw"));
        }

        user.available_tokens_for_withdraw = (user.available_tokens_for_withdraw - withdraw_amount).unwrap();
        user_store.store(env.message.sender.0.as_bytes(), &user)?;
    }
    if config.is_stopped_can_withdraw {
        if user.amount_delegated + user.available_tokens_for_withdraw < withdraw_amount {
            return Err(StdError::generic_err("Withdrawing more amount than Total Delegated and Reduced Staked tokens"));
        }
        if user.available_tokens_for_withdraw < withdraw_amount {
            let temp_variable = (withdraw_amount - user.available_tokens_for_withdraw).unwrap();
            user.available_tokens_for_withdraw = Uint128(0);
            user.amount_delegated = (user.amount_delegated - temp_variable).unwrap();
        } else {
            user.available_tokens_for_withdraw = (user.available_tokens_for_withdraw - withdraw_amount).unwrap();
        }
        user_store.store(env.message.sender.0.as_bytes(), &user)?;
    }

    let messages: Vec<CosmosMsg> = vec![
        // Transfer Trigger fee to triggerer wallet
        transfer_msg(
            env.message.sender,
            withdraw_amount,
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

//Triggerer
fn claim_rewards<'a, S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    //Checking if msg send by Triggerer
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    check_if_triggerer(&config, &env.message.sender)?;

    //Checking if start time starts
    let mut lottery_prefixed = PrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mut deps.storage);
    let mut lottery_store = TypedStoreMut::<Lottery, PrefixedStorage<'_, S>>::attach(&mut lottery_prefixed);
    let mut a_lottery: Lottery = lottery_store.load(LOTTERY_KEY)?;
    validate_end_time(a_lottery.end_time, env.block.time)?;
    validate_start_time(a_lottery.start_time, env.block.time)?;

    // This way every time we call the claim_rewards function we will get a different result.
    // Plus it's going to be pretty hard to predict the exact time of the block, so less chance of cheating
    a_lottery.entropy.extend(&env.block.height.to_be_bytes());
    a_lottery.entropy.extend(&env.block.time.to_be_bytes());
    a_lottery.start_time = &env.block.time + 0;
    a_lottery.end_time = &env.block.time + a_lottery.duration + 0;
    lottery_store.store(LOTTERY_KEY, &a_lottery)?;

    //Launching the lottery
    let lottery_entries_append: GenerationalStore::<LotteryEntries, ReadonlyPrefixedStorage<S>>;
    let lottery_entries = ReadonlyPrefixedStorage::multilevel(&[LOTTERY_ENTRY_KEY], &deps.storage);
    if let Ok(res) = GenerationalStore::<LotteryEntries, ReadonlyPrefixedStorage<S>>::attach(&lottery_entries).unwrap_or(
        Err(StdError::generic_err("Lottery Restarted. Error due to no entries "))
    ) {
        lottery_entries_append = res;
    } else {
        return Ok(HandleResponse {
            messages: vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::ClaimRewards {
                status: Failure,
                winner: HumanAddr("Lottery Restarted. Error due to no entries ".to_string()),
            })?),
        });
    }
    let data = lottery_entries_append;
    if data.iter().count() == 0 {
        return Ok(HandleResponse {
            messages: vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::ClaimRewards {
                status: Failure,
                winner: HumanAddr("Lottery Restarted. Error due to no entries ".to_string()),
            })?),
        });
    }

    //Choosing Winner
    let mut entries: Vec<HumanAddr> = vec![];
    let mut weights: Vec<u128> = vec![];
    let iterator = data.iter().filter(|item| matches!(item, (_, Entry::Occupied { .. })));
    for user_address in iterator {
        let user_address = match user_address.1 {
            Entry::Occupied { generation: _, value } => value,
            _ => panic!("Unexpected result "),
        };
        if a_lottery.end_time <= user_address.entry_time {
            entries.push(user_address.user_address);
            weights.push(0 as u128)
        } else if ((&a_lottery.end_time - user_address.entry_time) / &a_lottery.duration) >= 1 {
            entries.push(user_address.user_address);
            weights.push(user_address.amount.0)
        } else {
            entries.push(user_address.user_address);
            weights.push((user_address.amount.0 / 1000000) * ((((a_lottery.end_time - user_address.entry_time) * 1000000) / a_lottery.duration) as u128))
        }
    }
    let prng_seed = config.clone().prng_seed;
    let mut hasher = Sha256::new();
    hasher.update(&prng_seed);
    hasher.update(&a_lottery.entropy);
    let hash = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(hash.as_slice());
    let mut rng: ChaChaRng = ChaChaRng::from_seed(result);
    let dist: WeightedIndex<u128>;
    if let Ok(distribution) = WeightedIndex::new(&weights) {
        dist = distribution
    } else {
        return Ok(HandleResponse {
            messages: vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::ClaimRewards {
                status: Success,
                winner: HumanAddr("NONE!!! All entries had weight zero. Lottery restarted".to_string()),
            })?),
        });
    }
    let sample = dist.sample(&mut rng);
    let winner_human = entries[sample].clone();

    //Getting the pending_rewards
    let response: LPStakingRewardsResponse = query_pending_rewards(&deps, &env, &config)?;
    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    let mut winning_amount = supply_pool.total_rewards_restaked + supply_pool.pending_staking_rewards + response.rewards.rewards;

    let trigger_percentage = config.triggerer_share_percentage;
    let trigger_share = Uint128(winning_amount.0 * ((trigger_percentage * 1000000) as u128) / 10000000000);
    winning_amount = (winning_amount - trigger_share).unwrap();
    supply_pool.triggering_cost = trigger_share;
    supply_pool.pending_staking_rewards = Uint128(0);
    let redeeming_amount = supply_pool.total_rewards_restaked;
    supply_pool.total_rewards_restaked = Uint128(0);
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;
    if winning_amount == Uint128(0) {
        return Err(StdError::generic_err(
            "No rewards available",
        ));
    }
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

    let mut user_prefixed = PrefixedStorage::multilevel(&[USER_INFO_KEY, winner_human.0.as_bytes()], &mut deps.storage);
    let mut user_store = TypedStoreMut::<UserInfo, PrefixedStorage<'_, S>>::attach(&mut user_prefixed);
    let mut user = user_store.load(winner_human.0.as_bytes()).unwrap(); // NotFound is the only possible error
    user.total_won += winning_amount;
    user.available_tokens_for_withdraw += winning_amount;
    user_store.store(winner_human.0.as_bytes(), &user)?;

    let mut user_history = PrefixedStorage::multilevel(&[USER_WINNING_HISTORY_KEY, winner_human.0.as_bytes()], &mut deps.storage);
    let mut user_history_append = AppendStoreMut::attach_or_create(&mut user_history)?;
    user_history_append.push(&UserWinningHistory { winning_amount: winning_amount.0 as u64, time: env.block.time })?;

    let mut last_lottery_result = PrefixedStorage::multilevel(&[LAST_LOTTERY_KEY], &mut deps.storage);
    let mut last_lottery_result_append = AppendStoreMut::attach_or_create(&mut last_lottery_result)?;
    last_lottery_result_append.push(&LastLotteryResults { winning_amount: winning_amount.0 as u64, time: env.block.time })?;

    Ok(HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ClaimRewards {
            status: Success,
            winner: winner_human,
        })?),
    })
}

fn triggering_cost_withdraw<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    //Checking if admin
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    //checking if triggering cost greater than 0
    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    if supply_pool.triggering_cost == Uint128(0)
    {
        return Err(StdError::generic_err("Triggerer share not sufficient"));
    }

    //Send triggering cost to admin
    let messages: Vec<CosmosMsg> = vec![
        // Transfer Trigger fee to triggerer wallet
        transfer_msg(
            config.admin,
            supply_pool.triggering_cost,
            None,
            RESPONSE_BLOCK_SIZE,
            config.token.contract_hash.clone(),
            config.token.address.clone(),
        )?
    ];

    supply_pool.triggering_cost = Uint128(0);
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;

    let res = HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::TriggeringCostWithdraw { status: Success })?),
    };
    Ok(res)
}

fn withdraw_excess<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    let excess_amount = supply_pool.pending_staking_rewards + supply_pool.total_rewards_restaked;
    if excess_amount <= Uint128(0)
    {
        return Err(StdError::generic_err("Triggerer share not sufficient"));
    }

    supply_pool.pending_staking_rewards = Uint128(0);
    supply_pool.total_rewards_restaked = Uint128(0);
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;

    let messages: Vec<CosmosMsg> = vec![
        // Transfer Trigger fee to triggerer wallet
        transfer_msg(
            config.admin,
            excess_amount,
            None,
            RESPONSE_BLOCK_SIZE,
            config.token.contract_hash.clone(),
            config.token.address.clone(),
        )?
    ];

    let res = HandleResponse {
        messages,
        log: vec![],
        data: Some(to_binary(&HandleAnswer::WithdrawExcess { status: Success })?),
    };
    Ok(res)
}

//Admin COMMANDS ONLY
fn change_admin<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    address: HumanAddr,
) -> StdResult<HandleResponse> {
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;
    config.admin = address;
    configstore.store(CONFIG_KEY, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeAdmin { status: Success })?),
    })
}

fn change_triggerer<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
    address: HumanAddr,
) -> StdResult<HandleResponse> {
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;
    config.triggerer = address;
    configstore.store(CONFIG_KEY, &config)?;

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
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;

    check_if_admin(&config, &env.message.sender)?;

    let mut lottery_prefixed = PrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mut deps.storage);
    let mut lottery_store = TypedStoreMut::<Lottery, PrefixedStorage<'_, S>>::attach(&mut lottery_prefixed);
    let mut a_lottery: Lottery = lottery_store.load(LOTTERY_KEY)?;
    a_lottery.duration = duration;
    lottery_store.store(LOTTERY_KEY, &a_lottery)?;

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
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    config.triggerer_share_percentage = percentage;
    configstore.store(CONFIG_KEY, &config)?;

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
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    config.staking_contract = SecretContract {
        address,
        contract_hash,
    };
    configstore.store(CONFIG_KEY, &config)?;

    return Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ChangeStakingContract {
            status: Success,
        })?),
    });
}

fn resume_contract<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    config.is_stopped = false;
    config.is_stopped_can_withdraw = false;
    configstore.store(CONFIG_KEY, &config)?;

    return Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::ResumeContract { status: Success })?),
    });
}

fn stop_contract<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    //Checking if admin
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    //Can't stop contract if already stopped
    if !config.is_stopped {
        config.is_stopped = true;
        configstore.store(CONFIG_KEY, &config)?;

        return Ok(HandleResponse {
            messages: vec![],
            log: vec![],
            data: Some(to_binary(&HandleAnswer::StopContract { status: Success })?),
        });
    } else {
        return Err(StdError::generic_err(format!(
            "Contract is already stopped."
        )));
    }
}

fn allow_withdraw_when_stopped<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    //checking Admin
    let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut deps.storage);
    let mut configstore = TypedStoreMut::<Config, PrefixedStorage<'_, S>>::attach(&mut config_prefixed);
    let mut config: Config = configstore.load(CONFIG_KEY)?;
    let _ = check_if_admin(&config, &env.message.sender);

    //setting Config
    config.is_stopped_can_withdraw = true;
    configstore.store(CONFIG_KEY, &config)?;

    Ok(HandleResponse {
        messages: vec![],
        log: vec![],
        data: Some(to_binary(&HandleAnswer::AllowWithdrawWhenStopped { status: Success })?),
    })
}

fn emergency_redeem_from_staking<S: Storage, A: Api, Q: Querier>(
    deps: &mut Extern<S, A, Q>,
    env: Env,
) -> StdResult<HandleResponse> {
    //Check if Admin
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    //Querying Rewards
    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps, &env, &config)?;

    //Setting supply params
    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    let amount_to_redeem = supply_pool.total_rewards_restaked + supply_pool.total_tokens_staked;
    supply_pool.pending_staking_rewards += staking_rewards_response.rewards.rewards;
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;

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
    //Checking if Admin
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;
    check_if_admin(&config, &env.message.sender)?;

    //Querying rewards
    let staking_rewards_response: LPStakingRewardsResponse = query_pending_rewards(&deps, &env, &config)?;
    let mut sefi_pending_staking_rewards = Uint128(0);
    if staking_rewards_response.rewards.rewards > Uint128(0) {
        sefi_pending_staking_rewards = staking_rewards_response.rewards.rewards
    }

    //Updating SupplyPool
    let mut supply_pool_prefixed = PrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mut deps.storage);
    let mut supply_store = TypedStoreMut::<SupplyPool, PrefixedStorage<'_, S>>::attach(&mut supply_pool_prefixed);
    let mut supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;
    let amount_to_restake = supply_pool.total_tokens_staked + supply_pool.total_rewards_restaked + supply_pool.pending_staking_rewards;
    supply_pool.total_rewards_restaked += supply_pool.pending_staking_rewards;
    supply_pool.pending_staking_rewards = sefi_pending_staking_rewards;
    supply_store.store(SUPPLY_POOL_KEY, &supply_pool)?;

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

//HELPER FUNCTIONS

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

//Queries
fn query_pending_rewards<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    env: &Env,
    config: &Config,
) -> StdResult<LPStakingRewardsResponse> {
    let staking_rewards_response: LPStakingRewardsResponse = LPStakingQueryMsg::Rewards {
        address: env.clone().contract.address,
        key: STAKING_VK.to_string(),
        height: env.block.height,
    }.query(&deps.querier, config.staking_contract.contract_hash.clone(), config.staking_contract.address.clone())?;

    Ok(staking_rewards_response)
}

fn query_total_rewards<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>, height: Uint128) -> StdResult<Binary> {
    //Getting the pending_rewards
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;

    let response: LPStakingRewardsResponse = LPStakingQueryMsg::Rewards {
        address: config.clone().own_addr,
        key: STAKING_VK.to_string(),
        height: height.0 as u64,
    }.query(&deps.querier, config.clone().staking_contract.contract_hash, config.clone().staking_contract.address)?;
    let rewards_in_lp_contract = response.rewards.rewards;

    let supply_pool_prefixed = ReadonlyPrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &deps.storage);
    let supply_store = TypedStore::<SupplyPool, ReadonlyPrefixedStorage<'_, S>>::attach(&supply_pool_prefixed);
    let supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;

    let total_rewards = rewards_in_lp_contract + supply_pool.total_rewards_restaked + supply_pool.pending_staking_rewards;

    to_binary(&QueryAnswer::TotalRewards {
        rewards: total_rewards,
    })
}

fn query_total_deposit<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    //Getting the pending_rewards

    let supply_pool_prefixed = ReadonlyPrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &deps.storage);
    let supply_store = TypedStore::<SupplyPool, ReadonlyPrefixedStorage<'_, S>>::attach(&supply_pool_prefixed);
    let supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY)?;

    to_binary(&QueryAnswer::TotalDeposits {
        deposits: supply_pool.total_tokens_staked,
    })
}

fn query_deposit<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: &HumanAddr,
) -> StdResult<Binary> {
    let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, address.0.as_bytes()], &deps.storage);
    let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, S>>::attach(&mut user_prefixed);
    let user = user_store
        .load(address.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), entries: vec![], entry_index: vec![] });

    to_binary(&QueryAnswer::Balance {
        amount: (user.amount_delegated),
    })
}

fn query_available_funds<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: &HumanAddr,
) -> StdResult<Binary> {
    let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, address.0.as_bytes()], &deps.storage);
    let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, S>>::attach(&mut user_prefixed);
    let user = user_store
        .load(address.0.as_bytes())
        .unwrap_or(UserInfo { amount_delegated: Uint128(0), available_tokens_for_withdraw: Uint128(0), total_won: Uint128(0), entries: vec![], entry_index: vec![] });

    //Getting the pending_rewards
    let config_prefixed = ReadonlyPrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &deps.storage);
    let configstore = TypedStore::<Config, ReadonlyPrefixedStorage<'_, S>>::attach(&config_prefixed);
    let config: Config = configstore.load(CONFIG_KEY)?;

    if config.is_stopped_can_withdraw {
        return to_binary(&QueryAnswer::AvailableTokensForWithdrawl {
            amount: (user.available_tokens_for_withdraw + user.amount_delegated),
        });
    }

    to_binary(&QueryAnswer::AvailableTokensForWithdrawl {
        amount: (user.available_tokens_for_withdraw),
    })
}

fn query_user_past_records<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<Binary> {
    let mut results_vec = vec![];
    let user_history = ReadonlyPrefixedStorage::multilevel(&[USER_WINNING_HISTORY_KEY, address.0.as_bytes()], &deps.storage);

    if let Err(_err) = AppendStore::<'_, UserWinningHistory, ReadonlyPrefixedStorage<'_, S>>::attach(&user_history).unwrap_or(Err(StdError::generic_err("No entries yet"))) {
        return to_binary(&QueryAnswer::UserPastRecords {
            winning_history: results_vec,
        });
    }

    let user_history_append: Result<AppendStore<'_, UserWinningHistory, ReadonlyPrefixedStorage<'_, S>>, cosmwasm_std::StdError> = AppendStore::attach(&user_history).unwrap();
    let data = user_history_append.unwrap();
    let mut number_of_entries = data.len();

    if number_of_entries > 5 {
        number_of_entries = 5
    }

    for i in 0..number_of_entries {
        results_vec.push((data.get_at(data.len() - i - 1).unwrap().winning_amount, data.get_at(data.len() - i - 1).unwrap().time))
    }
    to_binary(&QueryAnswer::UserPastRecords {
        winning_history: results_vec,
    })
}

fn query_user_all_past_records<S: Storage, A: Api, Q: Querier>(
    deps: &Extern<S, A, Q>,
    address: HumanAddr,
) -> StdResult<Binary> {
    let user_history = ReadonlyPrefixedStorage::multilevel(&[USER_WINNING_HISTORY_KEY, address.0.as_bytes()], &deps.storage);
    let mut results_vec = vec![];
    if let Err(_err) = AppendStore::<'_, UserWinningHistory, ReadonlyPrefixedStorage<'_, S>>::attach(&user_history).unwrap_or(Err(StdError::generic_err("No entries yet"))) {
        return to_binary(&QueryAnswer::UserPastRecords {
            winning_history: results_vec,
        });
    }

    let user_history_append: Result<AppendStore<'_, UserWinningHistory, ReadonlyPrefixedStorage<'_, S>>, cosmwasm_std::StdError> = AppendStore::attach(&user_history).unwrap();
    let data = user_history_append.unwrap();
    let number_of_entries = data.len();

    for i in 0..number_of_entries {
        results_vec.push((data.get_at(i).unwrap().winning_amount, data.get_at(i).unwrap().time))
    }
    to_binary(&QueryAnswer::UserAllPastRecords {
        winning_history: results_vec,
    })
}

fn query_all_past_results<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    //Getting the pending_rewards

    let last_lottery_results = ReadonlyPrefixedStorage::multilevel(&[LAST_LOTTERY_KEY], &deps.storage);
    let mut results_vec = vec![];

    if let Err(_err) = AppendStore::<'_, LastLotteryResults, ReadonlyPrefixedStorage<'_, S>>::attach(&last_lottery_results).unwrap_or(Err(StdError::generic_err("No entries yet"))) {
        return to_binary(&QueryAnswer::PastRecords {
            past_rewards: results_vec.to_owned(),
        });
    }
    let last_lottery_results_append: Result<AppendStore<'_, LastLotteryResults, ReadonlyPrefixedStorage<'_, S>>, cosmwasm_std::StdError> = AppendStore::attach(&last_lottery_results).unwrap();
    let data = last_lottery_results_append.unwrap();
    let number_of_entries = data.len();

    for i in 0..number_of_entries {
        results_vec.push((data.get_at(i).unwrap().winning_amount, data.get_at(i).unwrap().time))
    }

    to_binary(&QueryAnswer::PastAllRecords {
        past_rewards: results_vec,
    })
}

fn query_past_results<S: Storage, A: Api, Q: Querier>(deps: &Extern<S, A, Q>) -> StdResult<Binary> {
    //Getting the pending_rewards
    let last_lottery_results = ReadonlyPrefixedStorage::multilevel(&[LAST_LOTTERY_KEY], &deps.storage);
    let mut results_vec = vec![];

    if let Err(_err) = AppendStore::<'_, LastLotteryResults, ReadonlyPrefixedStorage<'_, S>>::attach(&last_lottery_results).unwrap_or(Err(StdError::generic_err("No entries yet"))) {
        return to_binary(&QueryAnswer::PastRecords {
            past_rewards: results_vec.to_owned(),
        });
    }

    let last_lottery_results_append: Result<AppendStore<'_, LastLotteryResults, ReadonlyPrefixedStorage<'_, S>>, cosmwasm_std::StdError> =
        AppendStore::attach(&last_lottery_results).unwrap();
    let data = last_lottery_results_append.unwrap();
    let number_of_entries = data.len();

    for i in 0..number_of_entries {
        results_vec.push((data.get_at(data.len() - i - 1).unwrap().winning_amount, data.get_at(data.len() - i - 1).unwrap().time))
    }

    to_binary(&QueryAnswer::PastRecords {
        past_rewards: results_vec,
    })
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{StdResult, InitResponse, Extern, to_binary, Uint128, HumanAddr, Coin, Env, BlockInfo, MessageInfo, ContractInfo, Querier, Binary, from_binary, ReadonlyStorage, QuerierResult, StdError};
    use cosmwasm_std::testing::{MockStorage, MockApi, MockQuerier, mock_dependencies, MOCK_CONTRACT_ADDR};
    use secret_toolkit::storage::{TypedStoreMut, TypedStore};
    use crate::state::{Config, UserInfo, SupplyPool, Lottery, SecretContract, LotteryEntries};
    use crate::constants::{CONFIG_KEY, VIEWING_KEY_KEY, SUPPLY_POOL_KEY, STAKING_VK, LOTTERY_KEY, USER_INFO_KEY, CONFIG_KEY_PREFIX, SUPPLY_POOL_KEY_PREFIX, LOTTERY_KEY_PREFIX, LOTTERY_ENTRY_KEY};
    use crate::contract::{init, handle, deposit, claim_rewards, query, trigger_withdraw, withdraw, check_if_admin, check_if_triggerer, change_admin, change_triggerer, query_past_results, query_all_past_results, withdraw_excess, change_staking_contract, redelegate_to_contract, resume_contract};
    use crate::msg::{HandleMsg, HandleAnswer, ResponseStatus, InitMsg, LPStakingRewardsResponse, RewardsInfo, QueryMsg, QueryAnswer, LPStakingQueryMsg};
    use crate::viewing_keys::{ViewingKey};
    use cosmwasm_storage::{PrefixedStorage, ReadonlyPrefixedStorage};
    use secret_toolkit::utils::Query;
    use std::any::Any;
    use cosmwasm_std::QueryResponse;
    use secret_toolkit::incubator::{GenerationalStoreMut};
    use secret_toolkit::incubator::generational_store::Entry;
    use crate::msg::ResponseStatus::Success;

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
            triggerer_share_percentage: 100,
        };

        (init(&mut deps, env, init_msg), deps)
    }

    fn deposit_helper(mut mocked_deps: Extern<MockStorage, MockApi, MyMockQuerier>, env: Env) -> Extern<MockStorage, MockApi, MyMockQuerier>
    {
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Superman".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Spider-man".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Flash".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(500000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Thor".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Captain_America".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Black-widow".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Ironman".to_string()), Uint128(1000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Loki".to_string()), Uint128(1000000)).unwrap();//c.p:1000 deposit:8000
        deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(500000000)).unwrap();

        return mocked_deps;
    }

    fn config_helper(mut mocked_deps: Extern<MockStorage, MockApi, MyMockQuerier>) -> (Extern<MockStorage, MockApi, MyMockQuerier>, Config) {
        let mut config_prefixed = PrefixedStorage::multilevel(&[CONFIG_KEY_PREFIX], &mut mocked_deps.storage);
        let configstore = TypedStoreMut::<Config, PrefixedStorage<'_, MockStorage>>::attach(&mut config_prefixed);
        let config: Config = configstore.load(CONFIG_KEY).unwrap();
        (mocked_deps, config)
    }

    fn supply_pool_helper(mocked_deps: Extern<MockStorage, MockApi, MyMockQuerier>) -> (Extern<MockStorage, MockApi, MyMockQuerier>, SupplyPool) {
        let supply_pool_prefixed = ReadonlyPrefixedStorage::multilevel(&[SUPPLY_POOL_KEY_PREFIX], &mocked_deps.storage);
        let supply_store = TypedStore::<SupplyPool, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&supply_pool_prefixed);
        let supply_pool: SupplyPool = supply_store.load(SUPPLY_POOL_KEY).unwrap();
        (mocked_deps, supply_pool)
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
    fn test_handle_create_viewing_key() {
        let (_init_result, mut deps) = init_helper(None);
        let handle_msg = HandleMsg::CreateViewingKey {
            entropy: "entropy".to_string(),
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

    #[test]
    fn testing_deposit() {
        //INITIALISING
        let (_init_result, deps) = init_helper(None);
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        //1)Checking if wrong token is supported
        let response = deposit(&mut mocked_deps, mock_env("sef", &[], 0), HumanAddr("Batman".to_string()), Uint128(1000000)).unwrap_err();
        let (mut mocked_deps, config) = config_helper(mocked_deps);
        assert_eq!(response, StdError::generic_err(format!(
            "This token is not supported. Supported: {}, given: {}",
            config.token.address, mock_env("sef", &[], 601).message.sender
        )));

        //2 If amount less than 1 scrt or 1000000 uscrt
        let response = deposit(&mut mocked_deps, mock_env("sefi", &[], 0), HumanAddr("Batman".to_string()), Uint128(1)).unwrap_err();
        assert_eq!(response, StdError::generic_err(
            "Must deposit a minimum of 1000000 usefi, or 1 sefi",
        ));

        //3)Final checking
        let _response = deposit(&mut mocked_deps, mock_env("sefi", &[], 0), HumanAddr("Batman".to_string()), Uint128(100000000)).unwrap();

        ////checking the amount delegated
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, HumanAddr("Batman".to_string()).0.as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load(HumanAddr("Batman".to_string()).0.as_bytes()).unwrap();
        assert_eq!(user.amount_delegated, Uint128(100000000));
        assert_eq!(user.available_tokens_for_withdraw, Uint128(0));

        ////Checking SUPPLY POOL
        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.total_tokens_staked, Uint128(100000000));
        assert_eq!(supply_pool.pending_staking_rewards, Uint128(1000));
        assert_eq!(supply_pool.total_rewards_restaked, Uint128(0));

        ////Checking lottery entries
        mocked_deps = deposit_helper(mocked_deps, mock_env("sefi", &[], 0));
        let mut lottery_entries = PrefixedStorage::multilevel(&[LOTTERY_ENTRY_KEY], &mut mocked_deps.storage);
        let lottery_entries_append = GenerationalStoreMut::<LotteryEntries, PrefixedStorage<'_, MockStorage>>::attach_or_create(&mut lottery_entries).unwrap();
        let iterator = lottery_entries_append.iter().filter(|item| matches!(item, (_, Entry::Occupied { .. })));
        assert_eq!(iterator.count(), 11);
    }

    #[test]
    fn test_trigger_withdraw() {
        //INITIALISING
        let (_init_result, deps) = init_helper(None);
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        { mocked_deps = deposit_helper(mocked_deps, mock_env("sefi", &[], 601)); }

        //LOTTERY ENTRIES
        let mut lottery_entries = PrefixedStorage::multilevel(&[LOTTERY_ENTRY_KEY], &mut mocked_deps.storage);
        let lottery_entries_append = GenerationalStoreMut::<LotteryEntries, PrefixedStorage<MockStorage>>::attach_or_create(&mut lottery_entries).unwrap();
        for i in lottery_entries_append.iter() {
            let _user_address = match i.1 {
                Entry::Occupied { generation: _, value } => value,
                _ => panic!("Unexpected result "),
            };
        }

        //TRIGGERING WITHDRAW
        let _res = trigger_withdraw(&mut mocked_deps, mock_env("Batman", &[], 0), Option::from(Uint128(400000000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Batman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Batman".as_bytes()).unwrap();
        assert_eq!(user.available_tokens_for_withdraw.0, 400000000);
        assert_eq!(user.entry_index.len(), 2);

        //Checking Lottery Entries
        let _res = trigger_withdraw(&mut mocked_deps, mock_env("Batman", &[], 0), Option::from(Uint128(400000000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Batman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Batman".as_bytes()).unwrap();
        assert_eq!(user.available_tokens_for_withdraw.0, 800000000);
        assert_eq!(user.entry_index.len(), 1);
    }

    #[test]
    fn test_claim_rewards() {
        //1)Checking for errors
        let (_init_result, deps) = init_helper(None);
        let _env = mock_env("triggerer", &[], 700);

        // DEPOSIT HELPER
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let _ = claim_rewards(&mut mocked_deps, _env);
            let env = mock_env("sefi", &[], 0);
            mocked_deps = deposit_helper(mocked_deps, env);
        }
        //LOADING LOTTERY
        let lottery_prefixed = ReadonlyPrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mocked_deps.storage);
        let lottery_store = TypedStore::<Lottery, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&lottery_prefixed);
        let a_lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();

        let env = mock_env("triggerer", &[], a_lottery.end_time);
        let response = claim_rewards(&mut mocked_deps, env);

        let winner = match from_binary(&response.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::ClaimRewards { status: ResponseStatus::Success, winner: winner_addr } => winner_addr,
            HandleAnswer::ClaimRewards { status: ResponseStatus::Failure, winner: winner_addr } => winner_addr,
            _ => panic!("Unexpected result from handle"),
        };

        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, winner.0.as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load(winner.0.as_bytes()).unwrap();
        assert_eq!(user.available_tokens_for_withdraw.0, 10890);
        assert_eq!(user.total_won.0, 10890);

        let (_, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.total_rewards_restaked.0, 0);
        assert_eq!(supply_pool.pending_staking_rewards.0, 0);
        assert_eq!(supply_pool.total_tokens_staked.0, 1008000000);
    }

    #[test]
    fn test_withdraw() {
        //1)Checking for errors
        let (_init_result, deps) = init_helper(None);
        let _env = mock_env("sefi", &[], 0);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 0);
            mocked_deps = deposit_helper(mocked_deps, env);
        }

        //Triggered withdraw all token before stopping contract
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Batman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Batman".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 1000000000);

        let env = mock_env("Batman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env.clone(), Option::from(Uint128(1000000000)));
        let res = withdraw(&mut mocked_deps, env, Option::from(Uint128(10000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err("Withdrawing more amount than Available tokens for withdraw"));

        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Batman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Batman".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 0);
    }

    #[test]
    fn testing_triggerer_withdraw_rewards() {
        //Depositing amount
        let (_init_result, deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 600);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();

        let mut lottery_prefix = PrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mut mocked_deps.storage);
        let lottery_store = TypedStoreMut::<Lottery, PrefixedStorage<'_, MockStorage>>::attach(&mut lottery_prefix);
        let lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();

        let _response = claim_rewards(&mut mocked_deps, mock_env("triggerer", &[], lottery.end_time)).unwrap();

        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.triggering_cost, Uint128(30));

        let handlemsg = HandleMsg::TriggeringCostWithdraw {};
        let _res = handle(&mut mocked_deps, mock_env("admin", &[], 10), handlemsg);

        let (_, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.triggering_cost, Uint128(0));
    }

    //Stop contract
    //EmergencyRedeemFromStaking
    //AllowWithdrawWhenStopped
    //withdraw_excess
    #[test]
    fn emergency_stoppage_route_one() {
        //initialise
        let (_init_result, deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 0);
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        mocked_deps = deposit_helper(mocked_deps, env);

        //SETTING UP SOME TRIGGERS TO BE USED AT A LATER POINT
        //Triggered before allowed
        let env = mock_env("Batman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env.clone(), Option::from(Uint128(1000000000)));
        //Half triggered half delegated
        let env = mock_env("Superman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env.clone(), Option::from(Uint128(500000)));

        //STOPPING CONTRACT
        ////ERROR CHECK
        let env = mock_env("haseeb", &[], 0);//for error checking
        let msg = HandleMsg::StopContract {};
        let res = handle(&mut mocked_deps, env, msg);
        assert_eq!(res.unwrap_err(), StdError::generic_err(format!(
            "This is an admin command. Admin commands can only be run from admin address"
        )));

        ////NORMAL CHECK
        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::StopContract {};
        let res = handle(&mut mocked_deps, env, msg);
        let res = match from_binary(&res.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::StopContract { status: Success, } => "success",
            _ => panic!("Unexpected result from handle"),
        };
        assert_eq!(res, "success".to_string());

        //EMERGENCY REDEEM
        ////STATE CHANGES AFTER THE WITHDRAW
        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        let pending_rewards = supply_pool.pending_staking_rewards;
        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::EmergencyRedeemFromStaking {};
        let _res = handle(&mut mocked_deps, env, msg);
        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.pending_staking_rewards, pending_rewards + Uint128(1000));

        //Allowing Withdraw When Stopped
        ////testing before allowed
        let res = withdraw(&mut mocked_deps, mock_env("Batman", &[], 10), Option::from(Uint128(1000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err(format!("This contract is stopped and this action is not allowed")));

        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::AllowWithdrawWhenStopped {};
        let _res = handle(&mut mocked_deps, env, msg);

        //TESTING DIFFERENT SCENARIOS OF WITHDRAW
        let res = withdraw(&mut mocked_deps, mock_env("Batman", &[], 10), Option::from(Uint128(10000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err("Withdrawing more amount than Total Delegated and Reduced Staked tokens"));
        let _res = withdraw(&mut mocked_deps, mock_env("Batman", &[], 10), Option::from(Uint128(1000000000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Batman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Batman".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 0);
        assert_eq!(user.available_tokens_for_withdraw.0, 0);

        ////Half triggered half delegated
        let res = withdraw(&mut mocked_deps, mock_env("Superman", &[], 10), Option::from(Uint128(10000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err("Withdrawing more amount than Total Delegated and Reduced Staked tokens"));
        let _res = withdraw(&mut mocked_deps, mock_env("Superman", &[], 10), Option::from(Uint128(1000000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Superman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Superman".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 0);
        assert_eq!(user.available_tokens_for_withdraw.0, 0);

        let res = withdraw(&mut mocked_deps, mock_env("Spider-man", &[], 10), Option::from(Uint128(10000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err("Withdrawing more amount than Total Delegated and Reduced Staked tokens"));
        let _res = withdraw(&mut mocked_deps, mock_env("Spider-man", &[], 10), Option::from(Uint128(1000000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Spider-man".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Spider-man".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 0);
        assert_eq!(user.available_tokens_for_withdraw.0, 0);

        //WITHDRAW EXTRA FUNDS BY ADMIN
        let res = withdraw_excess(&mut mocked_deps, mock_env("non-admin", &[], 10));
        assert_eq!(res.unwrap_err(), StdError::generic_err("This is an admin command. Admin commands can only be run from admin address"));

        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.pending_staking_rewards, Uint128(4000)); // 1 last deposit + 2 triggers +emergency redeem
        assert_eq!(supply_pool.total_rewards_restaked, Uint128(9000));

        let _res = withdraw_excess(&mut mocked_deps, mock_env("admin", &[], 10));
        let (_, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.pending_staking_rewards, Uint128(0)); // 1 last deposit + 2 triggers +emergency redeem
        assert_eq!(supply_pool.total_rewards_restaked, Uint128(0));
    }

    //Stop contract
    //EmergencyRedeemFromStaking
    //change_staking_contract
    //redelegate_to_contract
    //resume_contract
    #[test]
    fn emergency_stoppage_route_two() {
        //initialise
        let (_init_result, deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 0);
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        mocked_deps = deposit_helper(mocked_deps, env);

        //Triggered before allowed
        let env = mock_env("Batman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env.clone(), Option::from(Uint128(1000000000)));

        //Half triggered half delegated
        let env = mock_env("Superman", &[], 601);
        let _res = trigger_withdraw(&mut mocked_deps, env.clone(), Option::from(Uint128(500000)));

        //Full delegated  ----- SPIDER-MAN

        //stop contract
        //for error checking
        let env = mock_env("haseeb", &[], 0);
        let msg = HandleMsg::StopContract {};
        let res = handle(&mut mocked_deps, env, msg);
        assert_eq!(res.unwrap_err(), StdError::generic_err(format!(
            "This is an admin command. Admin commands can only be run from admin address"
        )));
        //works fine and stop contract
        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::StopContract {};
        let res = handle(&mut mocked_deps, env, msg);
        let res = match from_binary(&res.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::StopContract { status: Success, } => "success",
            _ => panic!("Unexpected result from handle"),
        };
        assert_eq!(res, "success".to_string());

        //emergency redeem
        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        let pending_rewards = supply_pool.pending_staking_rewards;

        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::EmergencyRedeemFromStaking {};
        let _res = handle(&mut mocked_deps, env, msg);
        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.pending_staking_rewards, pending_rewards + Uint128(1000));

        //Change staking contract
        let env = mock_env("non-admin", &[], 0);
        let res = change_staking_contract(&mut mocked_deps, env, HumanAddr("staking_contract".to_string()), "".to_string());
        assert_eq!(res.unwrap_err(), StdError::generic_err(format!(
            "This is an admin command. Admin commands can only be run from admin address"
        )));

        let env = mock_env("admin", &[], 0);
        let _res = change_staking_contract(&mut mocked_deps, env, HumanAddr("new_staking_contract".to_string()), "".to_string());
        let (mut mocked_deps, config) = config_helper(mocked_deps);
        assert_eq!(config.staking_contract.address, HumanAddr("new_staking_contract".to_string()));

        //Redelegating to new contract
        let env = mock_env("non-admin", &[], 0);
        let res = redelegate_to_contract(&mut mocked_deps, env);
        assert_eq!(res.unwrap_err(), StdError::generic_err(format!(
            "This is an admin command. Admin commands can only be run from admin address"
        )));
        let (mut mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.pending_staking_rewards, Uint128(4000));

        let env = mock_env("admin", &[], 0);
        let _res = redelegate_to_contract(&mut mocked_deps, env);

        let env = mock_env("non-admin", &[], 0);
        let res = resume_contract(&mut mocked_deps, env);
        assert_eq!(res.unwrap_err(), StdError::generic_err(format!(
            "This is an admin command. Admin commands can only be run from admin address"
        )));

        let env = mock_env("admin", &[], 0);
        let _res = resume_contract(&mut mocked_deps, env);

        let res = withdraw(&mut mocked_deps, mock_env("Batman", &[], 10), Option::from(Uint128(10000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err("Withdrawing more amount than Available tokens for withdraw"));
        let _res = withdraw(&mut mocked_deps, mock_env("Batman", &[], 10), Option::from(Uint128(1000000000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Batman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Batman".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 0);
        assert_eq!(user.available_tokens_for_withdraw.0, 0);

        //Half triggered half delegated
        let res = withdraw(&mut mocked_deps, mock_env("Superman", &[], 10), Option::from(Uint128(10000000000)));
        assert_eq!(res.unwrap_err(), StdError::generic_err("Withdrawing more amount than Available tokens for withdraw"));
        let _res = withdraw(&mut mocked_deps, mock_env("Superman", &[], 10), Option::from(Uint128(500000))).unwrap();
        let mut user_prefixed = ReadonlyPrefixedStorage::multilevel(&[USER_INFO_KEY, "Superman".as_bytes()], &mocked_deps.storage);
        let user_store = TypedStore::<UserInfo, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&mut user_prefixed);
        let user: UserInfo = user_store.load("Superman".as_bytes()).unwrap();
        assert_eq!(user.amount_delegated.0, 500000);
        assert_eq!(user.available_tokens_for_withdraw.0, 0);
    }

    //CHANGING COMMANDS
    #[test]
    fn test_change_admin_triggerer() {
        let (_init_result, deps) = init_helper(None);

        // Deposit rewards on the staking contract
        let mocked_deps = deps.change_querier(|_| MyMockQuerier {});

        let env = mock_env("not-admin", &[], 600);
        let (mocked_deps, config) = config_helper(mocked_deps);
        let res = check_if_admin(&config, &env.message.sender).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));

        let env = mock_env("admin", &[], 600);
        let (mocked_deps, config) = config_helper(mocked_deps);

        let res = check_if_admin(&config, &env.message.sender);
        assert_eq!(res, Ok(()));

        let env = mock_env("not-triggerer", &[], 600);
        let (mocked_deps, config) = config_helper(mocked_deps);

        let res = check_if_triggerer(&config, &env.message.sender).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address and triggerer address",
        ));

        let env = mock_env("triggerer", &[], 600);
        let (mocked_deps, config) = config_helper(mocked_deps);
        let res = check_if_triggerer(&config, &env.message.sender);
        assert_eq!(res, Ok(()));

        //change admin
        let env = mock_env("not-admin", &[], 600);
        let (mut mocked_deps, _) = config_helper(mocked_deps);
        let res = change_admin(&mut mocked_deps, env, HumanAddr("triggerer".to_string())).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));

        let env = mock_env("admin", &[], 600);
        let (mut mocked_deps, _config) = config_helper(mocked_deps);
        let _res = change_admin(&mut mocked_deps, env, HumanAddr("someone".to_string())).unwrap();
        let (mocked_deps, config) = config_helper(mocked_deps);
        assert_eq!(config.admin, HumanAddr("someone".to_string()));

        let (mut mocked_deps, _config) = config_helper(mocked_deps);
        let res = change_admin(&mut mocked_deps, mock_env("not-admin", &[], 600), HumanAddr("triggerer".to_string())).unwrap_err();
        assert_eq!(res, StdError::generic_err(
            "This is an admin command. Admin commands can only be run from admin address",
        ));

        let _res = change_triggerer(&mut mocked_deps, mock_env("someone", &[], 600), HumanAddr("someone".to_string())).unwrap();
        let (_mocked_deps, config) = config_helper(mocked_deps);
        assert_eq!(config.triggerer, HumanAddr("someone".to_string()));
    }

    #[test]
    fn test_checking_contract_status() {
        //Contract balance > than
        let (_init_result, deps) = init_helper(Some(500000000));

        // deposit rewards on the staking contract
        let mut deps = deps.change_querier(|_| MyMockQuerier {});

        deposit(&mut deps, mock_env("sefi", &[], 600), HumanAddr("Batman".to_string()), Uint128(500000000)).unwrap();

        let _res = handle(&mut deps, mock_env("admin", &[], 600), HandleMsg::StopContract {});

        let _res = handle(&mut deps, mock_env("Batman", &[], 600), HandleMsg::TriggerWithdraw { amount: Option::from(Uint128(500000000)) });

        let _res = handle(&mut deps, mock_env("Batman", &[], 600), HandleMsg::StopContract {});

        let _res = handle(&mut deps, mock_env("Batman", &[], 600), HandleMsg::TriggerWithdraw { amount: Option::from(Uint128(500000000)) });

        let _res = handle(&mut deps, mock_env("admin", &[], 600), HandleMsg::StopContract {});

        let _res = handle(&mut deps, mock_env("admin", &[], 600), HandleMsg::ResumeContract {});

        let _res = handle(&mut deps, mock_env("Batman", &[], 10000000), HandleMsg::Withdraw { amount: Option::from(Uint128(500000000)) });
    }

    #[test]
    fn testing_change_triggerer_share() {
        //Depositing amount
        let (_init_result, deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 600);

        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();

        let mut lottery_prefix = PrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mut mocked_deps.storage);
        let lottery_store = TypedStoreMut::<Lottery, PrefixedStorage<'_, MockStorage>>::attach(&mut lottery_prefix);
        let lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();

        let handlemsg = HandleMsg::ChangeTriggererShare { percentage: 200 };
        let _res = handle(&mut mocked_deps, mock_env("admin", &[], 10), handlemsg);
        let _response = claim_rewards(&mut mocked_deps, mock_env("triggerer", &[], lottery.end_time)).unwrap();
        // println!("{:?}",_response);

        let (_mocked_deps, supply_pool) = supply_pool_helper(mocked_deps);
        assert_eq!(supply_pool.triggering_cost, Uint128(60));
    }

    #[test]
    fn testing_change_lottery_duration() {
        //Depositing amount
        let (_init_result, deps) = init_helper(Some(800000000));
        let env = mock_env("sefi", &[], 0);

        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();
        deposit(&mut mocked_deps, env.clone(), HumanAddr("batman".to_string()), Uint128(5000000000)).unwrap();

        let handlemsg = HandleMsg::ChangeLotteryDuration { duration: 100 };
        let _res = handle(&mut mocked_deps, mock_env("admin", &[], 10), handlemsg);

        let lottery_prefixed = ReadonlyPrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mocked_deps.storage);
        let lottery_store = TypedStore::<Lottery, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&lottery_prefixed);
        let a_lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();

        assert_eq!(a_lottery.duration, 100);

        let lottery_end = a_lottery.end_time;

        let _response = claim_rewards(&mut mocked_deps, mock_env("triggerer", &[], a_lottery.end_time)).unwrap();
        let lottery_prefixed = ReadonlyPrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mocked_deps.storage);
        let lottery_store = TypedStore::<Lottery, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&lottery_prefixed);
        let a_lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();
        assert_eq!(a_lottery.start_time, lottery_end);
        assert_eq!(a_lottery.end_time, lottery_end + 100);
    }

    //Testing Queries
    #[test]
    fn test_query_lottery() {
        let (_init_result, mocked_deps) = init_helper(None);

        let lottery_prefixed = ReadonlyPrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mocked_deps.storage);
        let lottery_store = TypedStore::<Lottery, ReadonlyPrefixedStorage<'_, MockStorage>>::attach(&lottery_prefixed);
        let a_lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();
        let query_msg = QueryMsg::LotteryInfo {};
        let query_result = query(&mocked_deps, query_msg);

        let (start_height, end_height, duration) = match from_binary(&query_result.unwrap()).unwrap() {
            QueryAnswer::LotteryInfo {
                start_time: start,
                end_time: end, duration, ..
            } => (start, end, duration),
            _ => panic!("Unexpected result from handle"),
        };

        assert_eq!(a_lottery.end_time, end_height);
        assert_eq!(a_lottery.start_time, start_height);
        assert_eq!(a_lottery.duration, duration);
    }

    #[test]
    fn test_available_funds() {
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

        let create_vk_msg = HandleMsg::CreateViewingKey {
            entropy: "entropy".to_string(),
            padding: None,
        };
        let handle_response = handle(&mut mocked_deps, mock_env("batman", &[], 601), create_vk_msg).unwrap();
        let vk = match from_binary(&handle_response.data.unwrap()).unwrap() {
            HandleAnswer::CreateViewingKey { key } => key,
            _ => panic!("Unexpected result from handle"),
        };

        let query_balance_msg = QueryMsg::AvailableTokensForWithdrawl {
            address: HumanAddr("batman".to_string()),
            key: vk.clone().0,
        };
        let query_response = query(&mocked_deps, query_balance_msg).unwrap();
        let balance = match from_binary(&query_response).unwrap() {
            QueryAnswer::AvailableTokensForWithdrawl { amount } => amount,
            _ => panic!("Unexpected result from query"),
        };
        assert_eq!(balance, Uint128(0));

        //IN CASE THE CONTRACT IS STOPPED
        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::StopContract {};
        let _res = handle(&mut mocked_deps, env, msg);

        let env = mock_env("admin", &[], 0);
        let msg = HandleMsg::AllowWithdrawWhenStopped {};
        let _res = handle(&mut mocked_deps, env, msg);

        let query_balance_msg = QueryMsg::AvailableTokensForWithdrawl {
            address: HumanAddr("batman".to_string()),
            key: vk.0,
        };
        let query_response = query(&mocked_deps, query_balance_msg).unwrap();
        let balance = match from_binary(&query_response).unwrap() {
            QueryAnswer::AvailableTokensForWithdrawl { amount } => amount,
            _ => panic!("Unexpected result from query"),
        };
        assert_eq!(balance, Uint128(5000000000));
    }

    #[test]
    fn test_query_total_rewards() {
        let (_init_result, deps) = init_helper(None);
        let env = mock_env("sefi", &[], 601);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        mocked_deps = deposit_helper(mocked_deps, env.clone());

        let height = Uint128(env.block.height as u128);

        let query_msg = QueryMsg::TotalRewards { height };
        let query_result = query(&mocked_deps, query_msg);

        let _total_rewards = match from_binary(&query_result.unwrap()).unwrap() {
            QueryAnswer::TotalRewards { rewards } => (rewards),
            _ => panic!("Unexpected result from handle"),
        };
        let (mocked_deps, config) = config_helper(mocked_deps);

        let response: LPStakingRewardsResponse = LPStakingQueryMsg::Rewards {
            address: env.contract.address,
            key: STAKING_VK.to_string(),
            height: env.block.height,
        }.query(&mocked_deps.querier, config.clone().staking_contract.contract_hash, config.clone().staking_contract.address).unwrap();
        let _rewards_in_lp_contract = response.rewards.rewards;
    }

    #[test]
    fn test_query_total_deposits() {
        let (_init_result, deps) = init_helper(None);
        let env = mock_env("sefi", &[], 601);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        mocked_deps = deposit_helper(mocked_deps, env);

        let query_msg = QueryMsg::TotalDeposits {};
        let query_result = query(&mocked_deps, query_msg);

        let total_deposits = match from_binary(&query_result.unwrap()).unwrap() {
            QueryAnswer::TotalDeposits { deposits } => (deposits),
            _ => panic!("Unexpected result from handle"),
        };

        assert_eq!(total_deposits, Uint128(1008000000))
    }

    #[test]
    fn test_query_past_results() {
        let (_init_result, deps) = init_helper(None);

        let _env = mock_env("sefi", &[], 0);

        // deposit rewards on the staking contract
        let mut mocked_deps = deps.change_querier(|_| MyMockQuerier {});
        {
            let env = mock_env("sefi", &[], 1);
            deposit(&mut mocked_deps, env.clone(), HumanAddr("Batman".to_string()), Uint128(50000000)).unwrap();
        }

        let mut lottery_prefix = PrefixedStorage::multilevel(&[LOTTERY_KEY_PREFIX], &mut mocked_deps.storage);
        let lottery_store = TypedStoreMut::<Lottery, PrefixedStorage<'_, MockStorage>>::attach(&mut lottery_prefix);
        let lottery: Lottery = lottery_store.load(LOTTERY_KEY).unwrap();

        let env = mock_env("triggerer", &[], lottery.end_time);
        let _res2 = claim_rewards(&mut mocked_deps, env);
        let env = mock_env("triggerer", &[], lottery.end_time + lottery.duration);
        let _res3 = claim_rewards(&mut mocked_deps, env);
        let env = mock_env("triggerer", &[], lottery.end_time + lottery.duration + lottery.duration);
        let _res4 = claim_rewards(&mut mocked_deps, env);
        let env = mock_env("triggerer", &[], lottery.end_time + lottery.duration + lottery.duration + lottery.duration);
        let res5 = claim_rewards(&mut mocked_deps, env);
        let _winner5 = match from_binary(&res5.unwrap().data.unwrap()).unwrap() {
            HandleAnswer::ClaimRewards { status: _, winner } => winner,
            _ => panic!("Unexpected result from handle"),
        };
        let _res: QueryAnswer = from_binary(&query_past_results(&mocked_deps).unwrap()).unwrap();
        // println!("{:?}",_res);

        let _res: QueryAnswer = from_binary(&query_all_past_results(&mocked_deps).unwrap()).unwrap();
        // println!("{:?}",_res);
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
            entropy: "entropy".to_string(),
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

        let create_vk_msg = HandleMsg::CreateViewingKey {
            entropy: "entropy".to_string(),
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
        // println!("{:?}",_results);
    }
}
