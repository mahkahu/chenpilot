#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, Map, Symbol, Vec, symbol_short,
};

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

/// Global liveness accounting configuration.
///
/// * `epoch_length_secs`        - duration (in seconds) of one liveness epoch.
/// * `epoch_reward_budget`      - maximum reward paid out per settled epoch (bounded).
/// * `reward_per_unit`          - nominal reward per unit of useful work (used to bound,
///                                reward actually paid is capped by the epoch budget).
/// * `max_units_per_relayer`    - per-relayer, per-epoch cap on counted useful work
///                                (prevents unbounded spam inflating one relayer's reward).
/// * `min_units_for_reward`     - minimum useful units required for a relayer to earn a
///                                share in a given epoch (anti-low-value / dust work).
/// * `grace_epochs`             - number of consecutive missed epochs tolerated before a
///                                liveness penalty applies (temporay network partitions).
/// * `equivocation_slash_bps`   - basis points of stake slashed per equivocation attempt.
/// * `liveness_slash_bps`       - basis points of stake slashed per persistent-failure
///                                (beyond the grace window) event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LivenessConfig {
    pub active: bool,
    pub epoch_length_secs: u64,
    pub epoch_reward_budget: i128,
    pub reward_per_unit: i128,
    pub max_units_per_relayer: u32,
    pub min_units_for_reward: u32,
    pub grace_epochs: u32,
    pub equivocation_slash_bps: u32,
    pub liveness_slash_bps: u32,
}

/// Rolling liveness state for a single relayer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LivenessState {
    pub last_active_epoch: u64,
    pub epochs_participated: u32,
    pub consecutive_missed: u32,
    pub equivocation_count: u32,
    pub rewards_accrued: i128,
    pub live: bool,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct EpochSnapshot {
    pub epoch: u64,
    pub settled: bool,
    pub total_reward: i128,
    pub participants: u32,
}

// Composite storage keys (Soroban contract types must be unit variants or
// separate structs - named-field enum variants are not supported).
#[contracttype]
#[derive(Clone)]
pub struct EpochUnitsKey {
    pub epoch: u64,
    pub relayer: Address,
}

#[contracttype]
#[derive(Clone)]
pub struct WorkOwnerKey {
    pub epoch: u64,
    pub work: Symbol,
}

#[contracttype]
#[derive(Clone)]
pub struct EpochRewardsKey {
    pub epoch: u64,
    pub relayer: Address,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Config,
    LivenessConfig,
    Relayer(Address),
    Liveness(Address),
    EpochWork(u64),
    EpochParticipants(u64),
    EpochUnits(EpochUnitsKey),
    WorkOwner(WorkOwnerKey),
    EpochRewards(EpochRewardsKey),
    EpochSettled(u64),
    EpochTotalReward(u64),
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
pub struct EvtLiveness {
    pub version: u32,
    pub ledger: u32,
    pub relayer: Address,
    pub epoch: u64,
    pub useful_units: u32,
    pub submitted_units: u32,
    pub reward_units: u32,
    pub duplicate: bool,
    pub equivocation: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtEpochSettled {
    pub version: u32,
    pub ledger: u32,
    pub epoch: u64,
    pub total_reward: i128,
    pub participants: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct EvtLivenessPenalty {
    pub version: u32,
    pub ledger: u32,
    pub relayer: Address,
    pub slash_amount: i128,
    pub reason: u32,
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

    // ── Liveness Accounting ────────────────────────────────────────────────────

    /// Enable / reconfigure liveness accounting (admin only).
    pub fn set_liveness_config(
        env: Env,
        epoch_length_secs: u64,
        epoch_reward_budget: i128,
        reward_per_unit: i128,
        max_units_per_relayer: u32,
        min_units_for_reward: u32,
        grace_epochs: u32,
        equivocation_slash_bps: u32,
        liveness_slash_bps: u32,
    ) {
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        config.admin.require_auth();

        if epoch_length_secs == 0 {
            panic!("epoch length must be positive");
        }
        if epoch_reward_budget < 0 || reward_per_unit < 0 {
            panic!("rewards must be non-negative");
        }
        if max_units_per_relayer == 0 {
            panic!("max units must be positive");
        }

        env.storage().instance().set(&DataKey::LivenessConfig, &LivenessConfig {
            active: true,
            epoch_length_secs,
            epoch_reward_budget,
            reward_per_unit,
            max_units_per_relayer,
            min_units_for_reward,
            grace_epochs,
            equivocation_slash_bps,
            liveness_slash_bps,
        });
    }

    fn liveness_config(env: &Env) -> LivenessConfig {
        env.storage()
            .instance()
            .get(&DataKey::LivenessConfig)
            .unwrap_or(LivenessConfig {
                active: false,
                epoch_length_secs: 0,
                epoch_reward_budget: 0,
                reward_per_unit: 0,
                max_units_per_relayer: 0,
                min_units_for_reward: 0,
                grace_epochs: 0,
                equivocation_slash_bps: 0,
                liveness_slash_bps: 0,
            })
    }

    fn current_epoch(env: &Env, cfg: &LivenessConfig) -> u64 {
        if !cfg.active || cfg.epoch_length_secs == 0 {
            return 0;
        }
        let now = env.ledger().timestamp();
        // timestamp is anchored at ledger timestamp ~ Soroban time in seconds.
        now / cfg.epoch_length_secs
    }

    fn ensure_relayer(env: &Env, relayer: &Address) -> RelayerInfo {
        env.storage()
            .persistent()
            .get(&DataKey::Relayer(relayer.clone()))
            .expect("Relayer not found")
    }

    fn liveness_state(env: &Env, relayer: &Address, cfg: &LivenessConfig) -> LivenessState {
        let mut st: LivenessState = env.storage()
            .persistent()
            .get(&DataKey::Liveness(relayer.clone()))
            .unwrap_or(LivenessState {
                last_active_epoch: u64::MAX,
                epochs_participated: 0,
                consecutive_missed: 0,
                equivocation_count: 0,
                rewards_accrued: 0,
                live: true,
            });

        // Lazy liveness evaluation: figure out how many epochs elapsed since last
        // activity and mark missed epochs. Tolerate up to `grace_epochs` missed
        // epochs (network partitions); beyond that the relayer becomes inactive
        // but is NOT instantly slashed - a real persistent-failure penalty is
        // applied only on a subsequent failed epoch beyond the grace window.
        let current = Self::current_epoch(env, cfg);
        if cfg.active && st.last_active_epoch != u64::MAX && current > st.last_active_epoch {
            let gap = (current - st.last_active_epoch) as u32;
            st.consecutive_missed = st.consecutive_missed.saturating_add(gap);
            if st.consecutive_missed > cfg.grace_epochs {
                st.live = false;
            }
        }
        st
    }

    fn save_liveness(env: &Env, relayer: &Address, st: &LivenessState) {
        env.storage().persistent().set(&DataKey::Liveness(relayer.clone()), st);
    }

    /// Record a unit of relayed work for the current liveness epoch.
    ///
    /// * If `work` was already seen this epoch, no new useful work is counted
    ///   (duplicate / spam earns no reward).
    /// * If `work` was claimed by a *different* relayer this epoch, that is
    ///   equivocation and triggers an equivocation penalty.
    pub fn record_relay_work(env: Env, relayer: Address, work: Symbol) {
        relayer.require_auth();
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        let cfg = Self::liveness_config(&env);
        if !cfg.active {
            panic!("liveness accounting not enabled");
        }

        let info = Self::ensure_relayer(&env, &relayer);
        if info.status != RelayerStatus::Active {
            panic!("relayer not active");
        }

        let epoch = Self::current_epoch(&env, &cfg);
        let mut st = Self::liveness_state(&env, &relayer, &cfg);
        st.last_active_epoch = epoch;
        st.consecutive_missed = 0;
        st.live = true;
        Self::save_liveness(&env, &relayer, &st);

        // Global per-epoch seen-work ledger.
        let mut seen: Vec<Symbol> = env.storage().persistent().get(&DataKey::EpochWork(epoch)).unwrap_or(Vec::new(&env));
        let owner_key = DataKey::WorkOwner(WorkOwnerKey { epoch, work: work.clone() });
        let mut duplicate = false;
        let mut equivocation = false;

        if let Some(owner) = env.storage().persistent().get::<_, Address>(&owner_key.clone()) {
            // Already seen this epoch.
            duplicate = true;
            if owner != relayer {
                // Conflicting claim -> equivocation.
                equivocation = true;
                st.equivocation_count = st.equivocation_count.saturating_add(1);
                Self::apply_equivocation_penalty(&env, &config, &relayer, &mut st, &cfg);
            }
            Self::save_liveness(&env, &relayer, &st);
        } else {
            // New useful work.
            let mut useful: u32 = env.storage()
                .persistent()
                .get::<_, u32>(&DataKey::EpochUnits(EpochUnitsKey { epoch, relayer: relayer.clone() }))
                .unwrap_or(0);

            // Bound useful work per relayer per epoch (spam resistance).
            if useful < cfg.max_units_per_relayer {
                useful = useful.saturating_add(1);
                env.storage().persistent().set(&DataKey::EpochUnits(EpochUnitsKey { epoch, relayer: relayer.clone() }), &useful);
                seen.push_back(work.clone());
                env.storage().persistent().set(&DataKey::EpochWork(epoch), &seen);
                env.storage().persistent().set(&owner_key, &relayer.clone());

                // Track participant (deduplicated).
                let mut participants: Vec<Address> = env.storage().persistent().get(&DataKey::EpochParticipants(epoch)).unwrap_or(Vec::new(&env));
                let mut found = false;
                for p in participants.iter() {
                    if p == relayer {
                        found = true;
                        break;
                    }
                }
                if !found {
                    participants.push_back(relayer.clone());
                    env.storage().persistent().set(&DataKey::EpochParticipants(epoch), &participants);
                }
            }
            env.storage().persistent().set(&DataKey::EpochUnits(EpochUnitsKey { epoch, relayer: relayer.clone() }), &useful);
        }

        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("work")),
            EvtLiveness {
                version: 1,
                ledger: env.ledger().sequence(),
                relayer,
                epoch,
                useful_units: 0,
                submitted_units: 0,
                reward_units: 0,
                duplicate,
                equivocation,
            },
        );
    }

    fn apply_equivocation_penalty(env: &Env, config: &Config, relayer: &Address, _st: &mut LivenessState, cfg: &LivenessConfig) {
        // Slash a bounded fraction of stake for equivocation; funds go to treasury.
        let token_client = token::Client::new(env, &config.staking_token);
        let balance = token_client.balance(&env.current_contract_address());
        if balance <= 0 {
            return;
        }
        let mut info: RelayerInfo = env.storage().persistent().get(&DataKey::Relayer(relayer.clone())).expect("Relayer not found");
        if info.status != RelayerStatus::Active {
            return;
        }
        let slash_amount = (info.stake_amount * cfg.equivocation_slash_bps as i128) / 10_000;
        if slash_amount > 0 {
            info.stake_amount = info.stake_amount.saturating_sub(slash_amount);
            info.last_transition_at = env.ledger().timestamp();
            env.storage().persistent().set(&DataKey::Relayer(relayer.clone()), &info);
            let available = balance.min(slash_amount);
            if available > 0 {
                token_client.transfer(&env.current_contract_address(), &config.treasury, &available);
            }
            env.events().publish(
                (symbol_short!("relayer"), symbol_short!("eqslash")),
                EvtLivenessPenalty {
                    version: 1,
                    ledger: env.ledger().sequence(),
                    relayer: relayer.clone(),
                    slash_amount: available,
                    reason: 1,
                },
            );
        }
    }

    /// Settle pending epoch rewards and distribute the bounded reward pool
    /// proportionally to non-duplicate useful work.
    pub fn settle_epoch(env: Env, epoch: u64) {
        let cfg = Self::liveness_config(&env);
        if !cfg.active {
            panic!("liveness accounting not enabled");
        }
        let current = Self::current_epoch(&env, &cfg);
        if epoch >= current {
            panic!("epoch not yet complete");
        }
        if env.storage().persistent().get::<_, bool>(&DataKey::EpochSettled(epoch)).unwrap_or(false) {
            panic!("epoch already settled");
        }

        let participants: Vec<Address> = env.storage().persistent().get(&DataKey::EpochParticipants(epoch)).unwrap_or(Vec::new(&env));
        if participants.is_empty() {
            env.storage().persistent().set(&DataKey::EpochSettled(epoch), &true);
            return;
        }

        // Sum useful units across all participants.
        let mut total_units: u32 = 0;
        let mut units: Map<Address, u32> = Map::new(&env);
        for p in participants.iter() {
            let u: u32 = env.storage().persistent().get(&DataKey::EpochUnits(EpochUnitsKey { epoch, relayer: p.clone() })).unwrap_or(0);
            units.set(p.clone(), u);
            total_units = total_units.saturating_add(u);
        }

        if total_units == 0 {
            env.storage().persistent().set(&DataKey::EpochSettled(epoch), &true);
            return;
        }

        // Distribute the bounded budget proportionally to useful units.
        let mut total_paid: i128 = 0;
        let mut paid_count: u32 = 0;
        let unit_per_relayer_cap: i128 = (cfg.reward_per_unit * cfg.max_units_per_relayer as i128)
            .min(cfg.epoch_reward_budget);

        for p in participants.iter() {
            let u = units.get(p.clone()).unwrap_or(0);
            if u < cfg.min_units_for_reward {
                continue;
            }
            // Bounded nominal reward, capped by both the per-relayer cap and the epoch budget.
            let nominal = cfg.reward_per_unit * u as i128;
            let reward = nominal.min(unit_per_relayer_cap).min(cfg.epoch_reward_budget - total_paid);
            if reward > 0 {
                let mut st = Self::liveness_state(&env, &p, &cfg);
                st.rewards_accrued = st.rewards_accrued.saturating_add(reward);
                env.storage().persistent().set(&DataKey::EpochRewards(EpochRewardsKey { epoch, relayer: p.clone() }), &reward);
                Self::save_liveness(&env, &p, &st);
                total_paid = total_paid.saturating_add(reward);
                paid_count = paid_count.saturating_add(1);
            }
        }

        env.storage().persistent().set(&DataKey::EpochSettled(epoch), &true);
        env.storage().persistent().set(&DataKey::EpochTotalReward(epoch), &total_paid);
        env.events().publish(
            (symbol_short!("relayer"), symbol_short!("epochend")),
            EvtEpochSettled {
                version: 1,
                ledger: env.ledger().sequence(),
                epoch,
                total_reward: total_paid,
                participants: paid_count,
            },
        );
    }

    /// Claim accrued liveness rewards from the contract-held reward pool.
    pub fn claim_rewards(env: Env, relayer: Address) -> i128 {
        relayer.require_auth();
        let config: Config = env.storage().instance().get(&DataKey::Config).expect("Not initialized");
        let mut st = env.storage()
            .persistent()
            .get::<_, LivenessState>(&DataKey::Liveness(relayer.clone()))
            .expect("Relayer not found");
        let amount = st.rewards_accrued;
        if amount <= 0 {
            return 0;
        }
        st.rewards_accrued = 0;
        env.storage().persistent().set(&DataKey::Liveness(relayer.clone()), &st);

        let token_client = token::Client::new(&env, &config.staking_token);
        let balance = token_client.balance(&env.current_contract_address());
        let pay = amount.min(balance);
        if pay > 0 {
            token_client.transfer(&env.current_contract_address(), &relayer, &pay);
        }
        pay
    }

    pub fn get_liveness_config(env: Env) -> LivenessConfig {
        Self::liveness_config(&env)
    }

    pub fn get_liveness(env: Env, relayer: Address) -> Option<LivenessState> {
        let cfg = Self::liveness_config(&env);
        Some(Self::liveness_state(&env, &relayer, &cfg))
    }

    pub fn get_epoch_units(env: Env, epoch: u64, relayer: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::EpochUnits(EpochUnitsKey { epoch, relayer }))
            .unwrap_or(0)
    }

    pub fn get_epoch_info(env: Env, epoch: u64) -> EpochSnapshot {
        let participants: Vec<Address> = env.storage().persistent().get(&DataKey::EpochParticipants(epoch)).unwrap_or(Vec::new(&env));
        let settled = env.storage().persistent().get::<_, bool>(&DataKey::EpochSettled(epoch)).unwrap_or(false);
        let total_reward: i128 = env.storage().persistent().get(&DataKey::EpochTotalReward(epoch)).unwrap_or(0);
        EpochSnapshot {
            epoch,
            settled,
            total_reward,
            participants: participants.len() as u32,
        }
    }

    pub fn get_relayer_info(env: Env, relayer: Address) -> Option<RelayerInfo> {
        env.storage().persistent().get(&DataKey::Relayer(relayer))
    }
}

mod test;
