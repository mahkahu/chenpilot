#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, vec, Address, Bytes, BytesN, Env, Vec, symbol_short, token,
};

/// Max intents (distinct events) a single work submission may commit to.
const MAX_INTENTS_PER_SUBMISSION: u32 = 32;
/// Max (intent, work) pairs retained per relayer per epoch. Bounds storage and
/// the cost of equivocation detection.
const MAX_INTENT_PAIRS_PER_EPOCH: u32 = 128;
/// Max distinct, rewarded work submissions retained per epoch. Bounds storage
/// and the size of the duplicate-work index.
const MAX_CLAIMED_WORK_PER_EPOCH: u32 = 128;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayerStatus {
    Active,
    UnstakeRequested,
    InDispute,
    Slashed,
    Withdrawn,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayerInfo {
    pub stake_amount: i128,
    pub status: RelayerStatus,
    pub unstake_requested_at: u64,
    pub dispute_count: u32,
    pub last_transition_at: u64,
}

#[contracttype]
pub enum DataKey {
    Config,
    Relayer(Address),
    LivenessConfig,
    EpochState,
    EpochReward,
    Liveness(Address),
    RelayerEpoch(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub admin: Address,
    pub staking_token: Address,
    pub treasury: Address,
    pub slashing_bps: u32,
    pub unbonding_period: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LivenessConfig {
    pub epoch_length: u64,
    pub grace_epochs: u32,
    pub slash_after_missed: u32,
    pub reward_token: Address,
    pub epoch_reward_budget: i128,
    pub max_reward_per_work: i128,
    pub max_reward_per_relayer: i128,
    pub min_events_per_submission: u32,
    pub liveness_slash_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochState {
    pub epoch_zero: u64,
    pub epoch: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochReward {
    pub epoch: u64,
    pub budget_spent: i128,
    pub claimed_work: Vec<BytesN<32>>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayerLiveness {
    pub last_epoch_submitted: u64,
    pub consecutive_missed: u32,
    pub last_checked_epoch: u64,
    pub pending_reward: i128,
    pub equivocation_count: u32,
    pub failure_locked: bool,
    pub liveness_failures: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntentWorkPair {
    pub intent: BytesN<32>,
    pub work: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayerEpochRecord {
    pub epoch: u64,
    pub intent_pairs: Vec<IntentWorkPair>,
    pub reward_earned: i128,
    pub equivocated: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkSubmission {
    pub work_id: BytesN<32>,
    pub epoch: u64,
    pub intent_ids: Vec<BytesN<32>>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkOutcome {
    pub reward: i128,
    pub new_intents: u32,
    pub duplicate: bool,
    pub equivocated: bool,
    pub rejected: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LivenessAssessment {
    pub epoch: u64,
    pub missed_epochs: u32,
    pub failed: bool,
    pub recoverable: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtInit {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub admin: Address,
    pub staking_token: Address,
    pub treasury: Address,
    pub slashing_bps: u32,
    pub unbonding_period: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtStake {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub stake_amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtUnstake {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub requested_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtDispute {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub dispute_count: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtSlashed {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub slash_amount: i128,
    pub new_stake: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtWithdraw {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub withdrawn_amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtLivenessConfigured {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub epoch_length: u64,
    pub grace_epochs: u32,
    pub slash_after_missed: u32,
    pub reward_token: Address,
    pub epoch_reward_budget: i128,
    pub max_reward_per_work: i128,
    pub max_reward_per_relayer: i128,
    pub min_events_per_submission: u32,
    pub liveness_slash_bps: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtWorkSubmitted {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub epoch: u64,
    pub work_id: BytesN<32>,
    pub new_intents: u32,
    pub reward: i128,
    pub duplicate: bool,
    pub equivocated: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtRewardClaimed {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtLivenessAssessed {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub epoch: u64,
    pub missed_epochs: u32,
    pub failed: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtLivenessSlashed {
    pub version: u32,
    pub ledger: u32,
    pub actor: Address,
    pub relayer: Address,
    pub slash_amount: i128,
    pub new_stake: i128,
}

#[contract]
pub struct RelayerSlashingContract;

#[contractimpl]
impl RelayerSlashingContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        staking_token: Address,
        treasury: Address,
        slashing_bps: u32,
        unbonding_period: u64,
    ) {
        if env.storage().instance().has(&DataKey::Config) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::Config, &Config {
            admin: admin.clone(),
            staking_token: staking_token.clone(),
            treasury: treasury.clone(),
            slashing_bps,
            unbonding_period,
        });

        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("init")),
            EvtInit {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: admin.clone(),
                admin,
                staking_token,
                treasury,
                slashing_bps,
                unbonding_period,
            },
        );
    }

    pub fn register_relayer(env: Env, relayer: Address, amount: i128) {
        relayer.require_auth();
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        let token_client = token::Client::new(&env, &config.staking_token);
        token_client.transfer(&relayer, &env.current_contract_address(), &amount);

        let now = env.ledger().timestamp();
        let mut info = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).unwrap_or(RelayerInfo {
            stake_amount: 0,
            status: RelayerStatus::Active,
            unstake_requested_at: 0,
            dispute_count: 0,
            last_transition_at: now,
        });

        info.status = RelayerStatus::Active;
        info.stake_amount += amount;
        info.last_transition_at = now;
        env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &info);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("stake")),
            EvtStake {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: relayer.clone(),
                relayer,
                stake_amount: info.stake_amount,
            },
        );
    }

    pub fn request_unstake(env: Env, relayer: Address) {
        relayer.require_auth();
        let mut info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");
        info.status = RelayerStatus::UnstakeRequested;
        info.unstake_requested_at = env.ledger().timestamp();
        info.last_transition_at = info.unstake_requested_at;
        env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &info);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("unstake")),
            EvtUnstake {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: relayer.clone(),
                relayer,
                requested_at: info.unstake_requested_at,
            },
        );
    }

    pub fn dispute_relayer(env: Env, relayer: Address) {
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        config.admin.require_auth();
        let mut info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");
        info.status = RelayerStatus::InDispute;
        info.dispute_count = info.dispute_count.saturating_add(1);
        info.last_transition_at = env.ledger().timestamp();
        env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &info);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("dispute")),
            EvtDispute {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: config.admin.clone(),
                relayer,
                dispute_count: info.dispute_count,
            },
        );
    }

    pub fn slash_relayer(env: Env, relayer: Address) {
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        config.admin.require_auth();
        let mut info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");
        if info.status == RelayerStatus::Slashed {
            return;
        }

        let slash_amount = (info.stake_amount * config.slashing_bps as i128) / 10_000;
        info.stake_amount = info.stake_amount.checked_sub(slash_amount).expect("Underflow");
        info.status = RelayerStatus::Slashed;
        info.last_transition_at = env.ledger().timestamp();
        let token_client = token::Client::new(&env, &config.staking_token);
        token_client.transfer(&env.current_contract_address(), &config.treasury, &slash_amount);
        env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &info);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("slashed")),
            EvtSlashed {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: config.admin.clone(),
                relayer,
                slash_amount,
                new_stake: info.stake_amount,
            },
        );
    }

    pub fn withdraw_stake(env: Env, relayer: Address) {
        relayer.require_auth();
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        let info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");

        if info.status == RelayerStatus::Slashed {
            panic!("slashed relayers cannot withdraw");
        }
        if info.status != RelayerStatus::UnstakeRequested {
            panic!("Unstake not requested");
        }
        if env.ledger().timestamp() < info.unstake_requested_at + config.unbonding_period {
            panic!("Unbonding period not met");
        }

        let token_client = token::Client::new(&env, &config.staking_token);
        token_client.transfer(&env.current_contract_address(), &relayer, &info.stake_amount);
        let withdrawn = RelayerInfo { status: RelayerStatus::Withdrawn, ..info };
        env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &withdrawn);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("withdraw")),
            EvtWithdraw {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: relayer.clone(),
                relayer,
                withdrawn_amount: withdrawn.stake_amount,
            },
        );
    }

    pub fn get_relayer_info(env: Env, relayer: Address) -> Option<RelayerInfo> {
        env.storage().persistent().get(&DataKey::Relayer(relayer))
    }

    /// One-time or upgradeable configuration that enables relayer liveness
    /// accounting and bound liveness rewards. Admin-gated.
    pub fn configure_liveness(env: Env, config: LivenessConfig) {
        let base: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        base.admin.require_auth();

        if config.epoch_length == 0 {
            panic!("epoch_length must be positive");
        }
        if config.slash_after_missed <= config.grace_epochs {
            panic!("slash_after_missed must exceed grace_epochs");
        }
        if config.epoch_reward_budget <= 0 || config.max_reward_per_work <= 0 || config.max_reward_per_relayer <= 0 {
            panic!("reward bounds must be positive");
        }
        if config.min_events_per_submission == 0 || config.min_events_per_submission > MAX_INTENTS_PER_SUBMISSION {
            panic!("invalid min_events_per_submission");
        }
        if config.liveness_slash_bps > 10_000 {
            panic!("invalid liveness_slash_bps");
        }

        let now = env.ledger().timestamp();
        // First activation starts the epoch clock. Reconfiguration preserves it.
        if !env.storage().instance().has(&DataKey::EpochState) {
            let empty: Vec<BytesN<32>> = vec![&env];
            env.storage().instance().set(&DataKey::EpochState, &EpochState { epoch_zero: now, epoch: 0 });
            env.storage().instance().set(&DataKey::EpochReward, &EpochReward { epoch: 0, budget_spent: 0, claimed_work: empty });
        }
        env.storage().instance().set(&DataKey::LivenessConfig, &config);

        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("liveness")),
            EvtLivenessConfigured {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: base.admin.clone(),
                epoch_length: config.epoch_length,
                grace_epochs: config.grace_epochs,
                slash_after_missed: config.slash_after_missed,
                reward_token: config.reward_token.clone(),
                epoch_reward_budget: config.epoch_reward_budget,
                max_reward_per_work: config.max_reward_per_work,
                max_reward_per_relayer: config.max_reward_per_relayer,
                min_events_per_submission: config.min_events_per_submission,
                liveness_slash_bps: config.liveness_slash_bps,
            },
        );
    }

    /// Current epoch index. Rolls internal epoch state forward if time passed.
    pub fn current_epoch(env: Env) -> u64 {
        let cfg: LivenessConfig = env.storage().instance().get(&DataKey::LivenessConfig).expect("Liveness not configured");
        Self::_roll_epoch(&env, &cfg)
    }

    /// Submit a canonical, time-bound useful-work proof. Rewards are bounded by
    /// the per-work cap, per-relayer per-epoch cap and the shared epoch budget.
    /// Duplicate work, stale work, insufficient work and equivocated work earn
    /// nothing.
    pub fn submit_work(env: Env, relayer: Address, work: WorkSubmission) -> WorkOutcome {
        relayer.require_auth();
        let cfg: LivenessConfig = env.storage().instance().get(&DataKey::LivenessConfig).expect("Liveness not configured");
        let epoch = Self::_roll_epoch(&env, &cfg);
        if work.epoch != epoch {
            panic!("work must target the current epoch");
        }
        let n = work.intent_ids.len();
        if n < cfg.min_events_per_submission || n > MAX_INTENTS_PER_SUBMISSION {
            panic!("insufficient or excessive intent count");
        }
        if !Self::_sorted(&work.intent_ids) {
            panic!("intent ids must be sorted");
        }
        if Self::_canonical_work_id(&env, &work.intent_ids) != work.work_id {
            panic!("work_id does not commit to the intent set");
        }

        let info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");
        if info.status != RelayerStatus::Active {
            panic!("relayer must be Active to earn rewards");
        }

        let mut live: RelayerLiveness = env.storage().persistent().get(&DataKey::Liveness(relayer.clone())).unwrap_or(Self::_default_liveness(epoch));

        let mut rec: RelayerEpochRecord = env.storage().persistent().get(&DataKey::RelayerEpoch(relayer.clone())).unwrap_or(RelayerEpochRecord {
            epoch,
            intent_pairs: Vec::new(&env),
            reward_earned: 0,
            equivocated: false,
        });
        if rec.epoch != epoch {
            rec = RelayerEpochRecord { epoch, intent_pairs: Vec::new(&env), reward_earned: 0, equivocated: false };
        }

        let mut er: EpochReward = env.storage().instance().get(&DataKey::EpochReward).expect("Liveness not configured");

        let mut duplicate_work = false;
        for claimed in er.claimed_work.iter() {
            if claimed == work.work_id {
                duplicate_work = true;
                break;
            }
        }

        let mut new_count: u32 = 0;
        let mut equivocated = rec.equivocated;
        for intent in work.intent_ids.iter() {
            let mut seen = false;
            let mut conflict = false;
            for p in rec.intent_pairs.iter() {
                if p.intent == intent {
                    seen = true;
                    if p.work != work.work_id {
                        conflict = true;
                    }
                    break;
                }
            }
            if conflict {
                equivocated = true;
            }
            if seen {
                continue;
            }
            if rec.intent_pairs.len() < MAX_INTENT_PAIRS_PER_EPOCH {
                rec.intent_pairs.push_back(IntentWorkPair { intent: intent.clone(), work: work.work_id.clone() });
                new_count += 1;
            }
        }
        rec.equivocated = equivocated;

        let mut reward: i128 = 0;
        if equivocated {
            live.equivocation_count += 1;
        } else if !duplicate_work && new_count > 0 {
            let available_budget = cfg.epoch_reward_budget - er.budget_spent;
            let relayer_cap_left = cfg.max_reward_per_relayer - rec.reward_earned;
            let mut slated = cfg.max_reward_per_work;
            if slated > available_budget {
                slated = available_budget;
            }
            if slated > relayer_cap_left {
                slated = relayer_cap_left;
            }
            if slated > 0 && er.claimed_work.len() < MAX_CLAIMED_WORK_PER_EPOCH {
                er.claimed_work.push_back(work.work_id.clone());
                er.budget_spent += slated;
                rec.reward_earned += slated;
                reward = slated;
            }
        }

        // Only genuinely useful work (>=1 new distinct intent) counts as a
        // liveness signal, so keep-alive spam cannot game liveness.
        if new_count > 0 {
            live.last_epoch_submitted = epoch;
            live.consecutive_missed = 0;
            live.failure_locked = false;
        }
        live.last_checked_epoch = epoch;
        live.pending_reward += reward;

        env.storage().instance().set(&DataKey::EpochReward, &er);
        env.storage().persistent().set(&DataKey::RelayerEpoch(relayer.clone()), &rec);
        env.storage().persistent().set(&DataKey::Liveness(relayer.clone()), &live);

        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("work")),
            EvtWorkSubmitted {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: relayer.clone(),
                relayer: relayer.clone(),
                epoch,
                work_id: work.work_id.clone(),
                new_intents: new_count,
                reward,
                duplicate: duplicate_work,
                equivocated,
            },
        );

        WorkOutcome { reward, new_intents: new_count, duplicate: duplicate_work, equivocated, rejected: false }
    }

    /// Transfer accrued, unclaimed rewards to the relayer.
    pub fn claim_rewards(env: Env, relayer: Address) -> i128 {
        relayer.require_auth();
        let cfg: LivenessConfig = env.storage().instance().get(&DataKey::LivenessConfig).expect("Liveness not configured");
        let mut live: RelayerLiveness = env.storage().persistent().get(&DataKey::Liveness(relayer.clone())).expect("Relayer not found");
        let amount = live.pending_reward;
        if amount <= 0 {
            panic!("no pending rewards");
        }
        let token_client = token::Client::new(&env, &cfg.reward_token);
        token_client.transfer(&env.current_contract_address(), &relayer, &amount);
        live.pending_reward = 0;
        env.storage().persistent().set(&DataKey::Liveness(relayer.clone()), &live);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("reward")),
            EvtRewardClaimed {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: relayer.clone(),
                relayer,
                amount,
            },
        );
        amount
    }

    /// Evaluate one relayer's liveness. Missed epochs inside the grace window
    /// (partition tolerance) do not fail the relayer. Persistent failure locks
    /// the relayer until it proves useful work again.
    pub fn evaluate_liveness(env: Env, target: Address) -> LivenessAssessment {
        let cfg: LivenessConfig = env.storage().instance().get(&DataKey::LivenessConfig).expect("Liveness not configured");
        let epoch = Self::_roll_epoch(&env, &cfg);
        let mut live: RelayerLiveness = env.storage()
            .persistent()
            .get(&DataKey::Liveness(target.clone()))
            .unwrap_or(Self::_default_liveness(epoch));
        let missed = epoch.saturating_sub(live.last_epoch_submitted).saturating_sub(1);
        let effective = missed.saturating_sub(cfg.grace_epochs as u64);
        live.consecutive_missed = effective.min(u32::MAX as u64) as u32;
        live.last_checked_epoch = epoch;
        let failed = effective >= cfg.slash_after_missed as u64;
        if failed && !live.failure_locked {
            live.failure_locked = true;
            live.liveness_failures += 1;
        }
        env.storage().persistent().set(&DataKey::Liveness(target.clone()), &live);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("assess")),
            EvtLivenessAssessed {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: target.clone(),
                relayer: target.clone(),
                epoch,
                missed_epochs: effective.min(u32::MAX as u64) as u32,
                failed,
            },
        );
        LivenessAssessment { epoch, missed_epochs: effective.min(u32::MAX as u64) as u32, failed, recoverable: !failed }
    }

    /// Evaluate liveness for a batch of relayers.
    pub fn evaluate_liveness_batch(env: Env, targets: Vec<Address>) -> Vec<LivenessAssessment> {
        let mut out: Vec<LivenessAssessment> = Vec::new(&env);
        for t in targets.iter() {
            out.push_back(Self::evaluate_liveness(env.clone(), t));
        }
        out
    }

    /// Admin finalization for persistent liveness failure. Only applies while
    /// the relayer is still locked as failed; a relayer who recovered (proved
    /// useful work again) cannot be penalized retroactively.
    pub fn slash_relayer_for_liveness(env: Env, relayer: Address) {
        let base: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        base.admin.require_auth();
        let cfg: LivenessConfig = env.storage().instance().get(&DataKey::LivenessConfig).expect("Liveness not configured");
        let epoch = Self::_roll_epoch(&env, &cfg);
        let live: RelayerLiveness = env.storage().persistent().get(&DataKey::Liveness(relayer.clone())).expect("Relayer not found");
        if !live.failure_locked {
            panic!("no active persistent liveness failure");
        }
        let missed = epoch.saturating_sub(live.last_epoch_submitted).saturating_sub(1);
        let effective = missed.saturating_sub(cfg.grace_epochs as u64);
        if effective < cfg.slash_after_missed as u64 {
            panic!("persistent failure resolved");
        }

        let mut info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");
        if info.status == RelayerStatus::Slashed {
            return;
        }
        let slash_amount = (info.stake_amount * cfg.liveness_slash_bps as i128) / 10_000;
        info.stake_amount = info.stake_amount.checked_sub(slash_amount).expect("Underflow");
        info.status = RelayerStatus::Slashed;
        info.last_transition_at = env.ledger().timestamp();
        let token_client = token::Client::new(&env, &base.staking_token);
        token_client.transfer(&env.current_contract_address(), &base.treasury, &slash_amount);
        env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &info);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("lslash")),
            EvtLivenessSlashed {
                version: 1,
                ledger: env.ledger().sequence(),
                actor: base.admin.clone(),
                relayer,
                slash_amount,
                new_stake: info.stake_amount,
            },
        );
    }

    pub fn get_liveness_config(env: Env) -> Option<LivenessConfig> {
        env.storage().instance().get(&DataKey::LivenessConfig)
    }

    pub fn get_epoch_state(env: Env) -> Option<EpochState> {
        env.storage().instance().get(&DataKey::EpochState)
    }

    pub fn get_epoch_reward(env: Env) -> Option<EpochReward> {
        env.storage().instance().get(&DataKey::EpochReward)
    }

    pub fn get_relayer_liveness(env: Env, relayer: Address) -> Option<RelayerLiveness> {
        env.storage().persistent().get(&DataKey::Liveness(relayer))
    }

    pub fn get_relayer_epoch_record(env: Env, relayer: Address) -> Option<RelayerEpochRecord> {
        env.storage().persistent().get(&DataKey::RelayerEpoch(relayer))
    }

    fn _default_liveness(epoch: u64) -> RelayerLiveness {
        RelayerLiveness {
            last_epoch_submitted: 0,
            consecutive_missed: 0,
            last_checked_epoch: epoch,
            pending_reward: 0,
            equivocation_count: 0,
            failure_locked: false,
            liveness_failures: 0,
        }
    }

    fn _roll_epoch(env: &Env, cfg: &LivenessConfig) -> u64 {
        let es: EpochState = env.storage().instance().get(&DataKey::EpochState).expect("Liveness not configured");
        let now = env.ledger().timestamp();
        let next = now.saturating_sub(es.epoch_zero) / cfg.epoch_length;
        if next > es.epoch {
            let empty: Vec<BytesN<32>> = vec![env];
            env.storage().instance().set(&DataKey::EpochState, &EpochState { epoch_zero: es.epoch_zero, epoch: next });
            env.storage().instance().set(&DataKey::EpochReward, &EpochReward { epoch: next, budget_spent: 0, claimed_work: empty });
        }
        next
    }

    fn _sorted(intents: &Vec<BytesN<32>>) -> bool {
        let n = intents.len();
        for i in 1..n {
            let prev = intents.get(i - 1).unwrap();
            let cur = intents.get(i).unwrap();
            if prev > cur {
                return false;
            }
        }
        true
    }

    /// Canonical work commitment: SHA-256 over the concatenation of the sorted
    /// 32-byte intent ids. Two relayers that relay the exact same batch commit
    /// to the same work_id, so duplicate delivery is automatically de-rewarded.
    pub(crate) fn _canonical_work_id(env: &Env, intents: &Vec<BytesN<32>>) -> BytesN<32> {
        let mut joined = Bytes::new(env);
        for intent in intents.iter() {
            joined.extend_from_array(&intent.to_array());
        }
        env.crypto().sha256(&joined).to_bytes()
    }
}

mod test;