#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{token, vec, Address, BytesN, Env, Vec};

const T0: u64 = 1000;
const EPOCH_LENGTH: u64 = 100;
const GRACE_EPOCHS: u32 = 2;
const SLASH_AFTER_MISSED: u32 = 4;
const BUDGET: i128 = 1000;
const PER_WORK: i128 = 300;
const PER_RELAYER: i128 = 400;
const MIN_EVENTS: u32 = 2;
const LIVENESS_SLASH_BPS: u32 = 4000;

struct Setup<'a> {
    client: RelayerSlashingContractClient<'a>,
    treasury: Address,
    token_addr: Address,
    relayers: Vec<Address>,
}

fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let contract_id = sac.address();
    let token = token::Client::new(env, &contract_id);
    let stellar_asset_client = token::StellarAssetClient::new(env, &contract_id);
    (contract_id, token, stellar_asset_client)
}

fn intent(env: &Env, tag: u8, n: u8) -> BytesN<32> {
    let mut arr = [0u8; 32];
    arr[0] = tag;
    arr[1] = n;
    BytesN::from_array(env, &arr)
}

fn intents(env: &Env, tag: u8, count: u32) -> Vec<BytesN<32>> {
    let mut out = Vec::new(env);
    for i in 0..count {
        out.push_back(intent(env, tag, i as u8));
    }
    out
}

fn make_work(env: &Env, tag: u8, count: u32, epoch: u64) -> WorkSubmission {
    let ids = intents(env, tag, count);
    let work_id = RelayerSlashingContract::_canonical_work_id(env, &ids);
    WorkSubmission { work_id, epoch, intent_ids: ids }
}

fn liveness_cfg(token: Address) -> LivenessConfig {
    LivenessConfig {
        epoch_length: EPOCH_LENGTH,
        grace_epochs: GRACE_EPOCHS,
        slash_after_missed: SLASH_AFTER_MISSED,
        reward_token: token,
        epoch_reward_budget: BUDGET,
        max_reward_per_work: PER_WORK,
        max_reward_per_relayer: PER_RELAYER,
        min_events_per_submission: MIN_EVENTS,
        liveness_slash_bps: LIVENESS_SLASH_BPS,
    }
}

fn setup_with<'a>(env: &'a Env, relayers: u32) -> Setup<'a> {
    env.ledger().set_timestamp(T0);
    env.mock_all_auths();
    let admin = Address::generate(env);
    let treasury = Address::generate(env);
    let (token_addr, _token_client, stellar_asset) = create_token(env, &admin);
    let contract_id = env.register(RelayerSlashingContract, ());
    let client = RelayerSlashingContractClient::new(env, &contract_id);
    client.initialize(&admin, &token_addr, &treasury, &5000, &60);

    let mut relayers_vec = Vec::new(env);
    for _ in 0..relayers {
        let relayer = Address::generate(env);
        stellar_asset.mint(&relayer, &10_000);
        client.register_relayer(&relayer, &1000);
        relayers_vec.push_back(relayer);
    }

    client.configure_liveness(&liveness_cfg(token_addr.clone()));

    Setup { client, treasury, token_addr, relayers: relayers_vec }
}

#[test]
fn test_registration_and_staking() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, token_client, stellar_asset) = create_token(&env, &admin);
    let contract_id = env.register(RelayerSlashingContract, ());
    let client = RelayerSlashingContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_addr, &treasury, &5000, &10);
    stellar_asset.mint(&relayer, &1000);
    client.register_relayer(&relayer, &1000);
    let info = client.get_relayer_info(&relayer).unwrap();
    assert_eq!(info.stake_amount, 1000);
    assert_eq!(info.status, RelayerStatus::Active);
    assert_eq!(token_client.balance(&contract_id), 1000);
}

#[test]
fn test_dispute_and_slash_relayer() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, token_client, stellar_asset) = create_token(&env, &admin);
    let contract_id = env.register(RelayerSlashingContract, ());
    let client = RelayerSlashingContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_addr, &treasury, &5000, &10);
    stellar_asset.mint(&relayer, &1000);
    client.register_relayer(&relayer, &1000);
    client.dispute_relayer(&relayer);
    client.slash_relayer(&relayer);
    let info = client.get_relayer_info(&relayer).unwrap();
    assert_eq!(info.stake_amount, 500);
    assert_eq!(info.status, RelayerStatus::Slashed);
    assert_eq!(token_client.balance(&treasury), 500);
}

#[test]
fn test_withdraw_success_after_unbonding() {
    let env = Env::default();
    env.ledger().set_timestamp(0);
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let (token_addr, token_client, stellar_asset) = create_token(&env, &admin);
    let contract_id = env.register(RelayerSlashingContract, ());
    let client = RelayerSlashingContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_addr, &admin, &5000, &60);
    stellar_asset.mint(&relayer, &1000);
    client.register_relayer(&relayer, &1000);
    client.request_unstake(&relayer);
    env.ledger().set_timestamp(70);
    client.withdraw_stake(&relayer);
    assert_eq!(token_client.balance(&relayer), 1000);
    assert_eq!(client.get_relayer_info(&relayer).unwrap().status, RelayerStatus::Withdrawn);
}

#[test]
#[should_panic(expected = "Unbonding period not met")]
fn test_withdraw_fails_within_unbonding_period() {
    let env = Env::default();
    env.ledger().set_timestamp(0);
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let (token_addr, _token_client, stellar_asset) = create_token(&env, &admin);
    let contract_id = env.register(RelayerSlashingContract, ());
    let client = RelayerSlashingContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_addr, &admin, &5000, &60);
    stellar_asset.mint(&relayer, &1000);
    client.register_relayer(&relayer, &1000);
    client.request_unstake(&relayer);
    env.ledger().set_timestamp(30);
    client.withdraw_stake(&relayer);
}

#[test]
fn test_duplicate_batch_earns_no_reward() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    let first = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(first.reward, PER_WORK);
    assert!(!first.duplicate);
    let second = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(second.reward, 0);
    assert!(second.duplicate);
}

#[test]
#[should_panic(expected = "insufficient or excessive intent count")]
fn test_unnecessary_work_rejected() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS - 1, 0));
}

#[test]
#[should_panic(expected = "work must target the current epoch")]
fn test_stale_work_rejected() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 1));
}

#[test]
fn test_reward_bounded_by_work_and_relayer_caps() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    let r1 = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    let r2 = setup.client.submit_work(&relayer, &make_work(&env, 2, MIN_EVENTS, 0));
    let r3 = setup.client.submit_work(&relayer, &make_work(&env, 3, MIN_EVENTS, 0));
    assert_eq!(r1.reward, PER_WORK);
    assert_eq!(r2.reward, PER_RELAYER - PER_WORK);
    assert_eq!(r3.reward, 0);
    let rec = setup.client.get_relayer_epoch_record(&relayer).unwrap();
    assert_eq!(rec.reward_earned, PER_RELAYER);
}

#[test]
fn test_epoch_budget_shared_across_cartel() {
    let env = Env::default();
    let setup = setup_with(&env, 6);
    let mut total: i128 = 0;
    let mut last: WorkOutcome = WorkOutcome { reward: 0, new_intents: 0, duplicate: false, equivocated: false, rejected: false };
    for i in 0..6 {
        let relayer = setup.relayers.get(i).unwrap();
        let out = setup.client.submit_work(&relayer, &make_work(&env, (i as u8) + 1, MIN_EVENTS, 0));
        assert!(out.reward <= PER_WORK);
        total += out.reward;
        last = out;
    }
    let er = setup.client.get_epoch_reward().unwrap();
    assert_eq!(er.budget_spent, BUDGET);
    assert_eq!(total, BUDGET);
    assert_eq!(last.reward, 0);
}

#[test]
fn test_equivocation_earns_nothing() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    let first = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(first.reward, PER_WORK);

    let shared = intent(&env, 1, 0);
    let extra = intent(&env, 9, 0);
    let ids = vec![&env, shared, extra];
    let work_id = RelayerSlashingContract::_canonical_work_id(&env, &ids);
    let second = setup.client.submit_work(
        &relayer,
        &WorkSubmission { work_id, epoch: 0, intent_ids: ids },
    );
    assert!(second.equivocated);
    assert_eq!(second.reward, 0);
    let live = setup.client.get_relayer_liveness(&relayer).unwrap();
    assert_eq!(live.equivocation_count, 1);
}

#[test]
fn test_cross_relayer_duplicate_earns_nothing() {
    let env = Env::default();
    let setup = setup_with(&env, 2);
    let relayer_a = setup.relayers.get(0).unwrap();
    let relayer_b = setup.relayers.get(1).unwrap();
    let first = setup.client.submit_work(&relayer_a, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(first.reward, PER_WORK);
    let second = setup.client.submit_work(&relayer_b, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(second.reward, 0);
    assert!(second.duplicate);
}

#[test]
fn test_partition_grace_and_recovery() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    let out = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(out.reward, PER_WORK);

    env.ledger().set_timestamp(T0 + 2 * EPOCH_LENGTH + 50);
    let a1 = setup.client.evaluate_liveness(&relayer);
    assert!(!a1.failed);

    let fail_ts = T0 + (GRACE_EPOCHS as u64 + SLASH_AFTER_MISSED as u64 + 1) * EPOCH_LENGTH + 50;
    env.ledger().set_timestamp(fail_ts);
    let a2 = setup.client.evaluate_liveness(&relayer);
    assert!(a2.failed);
    let live = setup.client.get_relayer_liveness(&relayer).unwrap();
    assert_eq!(live.consecutive_missed, SLASH_AFTER_MISSED);
    assert_eq!(live.liveness_failures, 1);

    env.ledger().set_timestamp(fail_ts + 10);
    let recovered = setup.client.submit_work(&relayer, &make_work(&env, 2, MIN_EVENTS, a2.epoch));
    assert_eq!(recovered.reward, PER_WORK);
    assert!(!recovered.equivocated);
    let live2 = setup.client.get_relayer_liveness(&relayer).unwrap();
    assert!(!live2.failure_locked);

    let a3 = setup.client.evaluate_liveness(&relayer);
    assert!(!a3.failed);
    assert_eq!(a3.missed_epochs, 0);
}

#[test]
fn test_persistent_failure_slashing() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    let out = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(out.reward, PER_WORK);

    let fail_ts = T0 + (GRACE_EPOCHS as u64 + SLASH_AFTER_MISSED as u64 + 1) * EPOCH_LENGTH + 50;
    env.ledger().set_timestamp(fail_ts);
    let a = setup.client.evaluate_liveness(&relayer);
    assert!(a.failed);

    let token_client = token::Client::new(&env, &setup.token_addr);
    assert_eq!(token_client.balance(&setup.treasury), 0);
    setup.client.slash_relayer_for_liveness(&relayer);
    let info = setup.client.get_relayer_info(&relayer).unwrap();
    assert_eq!(info.status, RelayerStatus::Slashed);
    assert_eq!(info.stake_amount, 600);
    assert_eq!(token_client.balance(&setup.treasury), 400);

    setup.client.slash_relayer_for_liveness(&relayer);
    let info2 = setup.client.get_relayer_info(&relayer).unwrap();
    assert_eq!(info2.status, RelayerStatus::Slashed);
    assert_eq!(info2.stake_amount, 600);
}

#[test]
#[should_panic(expected = "no active persistent liveness failure")]
fn test_recovered_relayer_cannot_be_slashed() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    let fail_ts = T0 + (GRACE_EPOCHS as u64 + SLASH_AFTER_MISSED as u64 + 1) * EPOCH_LENGTH + 50;
    env.ledger().set_timestamp(fail_ts);
    let a = setup.client.evaluate_liveness(&relayer);
    assert!(a.failed);
    env.ledger().set_timestamp(fail_ts + 10);
    setup.client.submit_work(&relayer, &make_work(&env, 2, MIN_EVENTS, a.epoch));
    setup.client.slash_relayer_for_liveness(&relayer);
}

#[test]
fn test_claim_rewards() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    let out = setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    assert_eq!(out.reward, PER_WORK);

    let token_client = token::Client::new(&env, &setup.token_addr);
    let before = token_client.balance(&relayer);
    let claimed = setup.client.claim_rewards(&relayer);
    assert_eq!(claimed, PER_WORK);
    assert_eq!(token_client.balance(&relayer), before + PER_WORK);
    let live = setup.client.get_relayer_liveness(&relayer).unwrap();
    assert_eq!(live.pending_reward, 0);
}

#[test]
#[should_panic(expected = "no pending rewards")]
fn test_claim_rewards_twice_rejected() {
    let env = Env::default();
    let setup = setup_with(&env, 1);
    let relayer = setup.relayers.get(0).unwrap();
    setup.client.submit_work(&relayer, &make_work(&env, 1, MIN_EVENTS, 0));
    setup.client.claim_rewards(&relayer);
    setup.client.claim_rewards(&relayer);
}

#[test]
fn test_liveness_batch_evaluation() {
    let env = Env::default();
    let setup = setup_with(&env, 2);
    let relayer_a = setup.relayers.get(0).unwrap();
    let relayer_b = setup.relayers.get(1).unwrap();
    setup.client.submit_work(&relayer_a, &make_work(&env, 1, MIN_EVENTS, 0));
    env.ledger().set_timestamp(T0 + 2 * EPOCH_LENGTH + 50);
    let targets = vec![&env, relayer_a, relayer_b];
    let res = setup.client.evaluate_liveness_batch(&targets);
    assert_eq!(res.len(), 2);
    assert!(!res.get(0).unwrap().failed);
    assert!(!res.get(1).unwrap().failed);
}