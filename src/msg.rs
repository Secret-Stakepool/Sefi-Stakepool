use crate::state::{SecretContract};
use crate::viewing_keys::ViewingKey;
use cosmwasm_std::{Binary, HumanAddr, Uint128};
use serde::{Deserialize, Serialize};
use secret_toolkit::utils::Query;
use schemars::{JsonSchema};

pub const RESPONSE_BLOCK_SIZE: usize = 256;


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct InitMsg {
    pub admin: Option<HumanAddr>,
    pub triggerer: Option<HumanAddr>,
    pub token: SecretContract,
    pub staking_contract: SecretContract,
    pub viewing_key: String,
    pub prng_seed: Binary,
    pub triggerer_share_percentage: u64,
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HandleMsg {
    // Registered commands
    Receive {
        sender: HumanAddr,
        from: HumanAddr,
        amount: Uint128,
        msg: Binary,
    },

    //User
    Deposit {},
    TriggerWithdraw {
        amount: Option<Uint128>,
    },
    Withdraw {
        amount: Option<Uint128>,
    },
    Redelegate {
        amount: Option<Uint128>,
    },
    CreateViewingKey {
        entropy: String,
        padding: Option<String>,
    },
    SetViewingKey {
        key: String,
        padding: Option<String>,
    },

    //Triggerer
    ClaimRewards {},

    //Admin
    TriggeringCostWithdraw {},
    WithdrawExcess{},
    ChangeAdmin {
        admin: HumanAddr
    },
    ChangeTriggerer {
        admin: HumanAddr
    },
    ChangeTriggererShare {
        percentage: u64,
    },

    ChangeLotteryDuration {
        duration: u64
    },
    StopContract {},
    AllowWithdrawWhenStopped {},
    ResumeContract {},


    //Admin--> Changing contract
    // ChangeStakingContractFlow => 1. stopContact 2.EmergencyRedeemFromStaking 4.ChangeStakingContract 5.RedelegateToNewContract 6.ResumeContract
    EmergencyRedeemFromStaking {},
    ChangeStakingContract {
        address: HumanAddr,
        contract_hash: String,
    },
    RedelegateToNewContract {},


    //Test

}


#[derive(Serialize, Deserialize, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum HandleAnswer {
    Redeem { status: LPStakingResponseStatus },
    CreateViewingKey { key: ViewingKey },
    SetViewingKey { status: ResponseStatus },
    StopContract { status: ResponseStatus },
    AllowWithdrawWhenStopped { status: ResponseStatus },
    ResumeContract { status: ResponseStatus },
    ChangeAdmin { status: ResponseStatus },
    ChangeTriggerer { status: ResponseStatus },
    ChangeTriggererShare { status: ResponseStatus },

    ChangeStakingContract { status: ResponseStatus },
    ChangeLotteryDuration {
        status: ResponseStatus,
    },

    TriggeringCostWithdraw { status: ResponseStatus },
    WithdrawExcess { status: ResponseStatus },


    ClaimRewards { status: ResponseStatus, winner: HumanAddr },
    EmergencyRedeemFromStaking { status: ResponseStatus },
    Deposit { status: ResponseStatus },
    Redelegate { status: ResponseStatus },
    RedelegateToContract { status: ResponseStatus },
    TriggerWithdraw { status: ResponseStatus },
    Withdraw { status: ResponseStatus },

    //Tests
    TestingDandC {
        status: ResponseStatus
    },
}


#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Success,
    Failure,
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryMsg {
    //PUBLIC
    TotalRewards { height: Uint128 },
    TotalDeposits {},
    TokenInfo {},
    ContractStatus {},
    RewardToken {},
    IncentivizedToken {},
    LotteryInfo {},

    // Authenticated
    Rewards {
        address: HumanAddr,
        key: String,
        height: u64,
    },
    Balance {
        address: HumanAddr,
        key: String,
    },
    AvailableTokensForWithdrawl {
        address: HumanAddr,
        key: String,
    },
    UserPastRecords {
        address: HumanAddr,
        key: String,
    },
    UserAllPastRecords {
        address: HumanAddr,
        key: String,
    },
    PastRecords {},
    PastAllRecords {},

    //AUTHENTICATED
}

impl Query for QueryMsg {
    const BLOCK_SIZE: usize = RESPONSE_BLOCK_SIZE;
}

impl QueryMsg {
    pub fn get_validation_params(&self) -> (&HumanAddr, ViewingKey) {
        match self {
            QueryMsg::Rewards { address, key, .. } => (address, ViewingKey(key.clone())),
            QueryMsg::Balance { address, key } => (address, ViewingKey(key.clone())),
            QueryMsg::AvailableTokensForWithdrawl { address, key } => (address, ViewingKey(key.clone())),
            QueryMsg::UserPastRecords { address, key } => (address, ViewingKey(key.clone())),
            QueryMsg::UserAllPastRecords { address, key } => (address, ViewingKey(key.clone())),

            _ => panic!("This should never happen"),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum QueryAnswer {
    TotalDeposits {
        deposits: Uint128,
    },
    TokenInfo {
        name: String,
        symbol: String,
        decimals: u8,
        total_supply: Option<Uint128>,
    },
    TotalRewards {
        rewards: Uint128,
    },
    Rewards {
        rewards: Uint128,
    },
    Balance {
        amount: Uint128,
    },
    AvailableTokensForWithdrawl {
        amount: Uint128
    },
    ContractStatus {
        is_stopped: bool,
    },
    RewardToken {
        token: SecretContract,
    },
    IncentivizedToken {
        token: SecretContract,
    },
    ViewingKeyError {
        msg: String,
    },

    QueryError {
        msg: String,
    },
    UserPastRecords {
        winning_history: Vec<(u64, u64)>,
    },

    UserAllPastRecords {
        winning_history: Vec<(u64, u64)>,
    },

    LotteryInfo {
        start_time: u64,
        end_time: u64,
        duration: u64,
        is_stopped:bool,
        is_stopped_with_withdraw:bool,
    },

    PastRecords {
        past_rewards: Vec<(u64, u64)>,
    },

    PastAllRecords {
        past_rewards: Vec<(u64, u64)>,
    },

}

#[derive(Serialize, Deserialize, Clone, PartialEq, JsonSchema, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LPStakingResponseStatus {
    Success,
    Failure,
}


#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct LPStakingRewardsResponse {
    pub rewards: RewardsInfo,
}


#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LPStakingHandleMsg {
    Redeem {
        amount: Uint128
    },
    Deposit {},
}

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct RewardsInfo {
    pub rewards: Uint128,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LPStakingQueryMsg {
    Rewards {
        address: HumanAddr,
        key: String,
        height: u64,
    }
}

impl Query for LPStakingQueryMsg {
    const BLOCK_SIZE: usize = RESPONSE_BLOCK_SIZE;
}

// Take a Vec<u8> and pad it up to a multiple of `block_size`, using spaces at the end.
pub fn space_pad(block_size: usize, message: &mut Vec<u8>) -> &mut Vec<u8> {
    let len = message.len();
    let surplus = len % block_size;
    if surplus == 0 {
        return message;
    }

    let missing = block_size - surplus;
    message.reserve(missing);
    message.extend(std::iter::repeat(b' ').take(missing));
    message
}

