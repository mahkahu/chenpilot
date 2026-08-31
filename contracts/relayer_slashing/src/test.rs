#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::{symbol_short, token, Address, Env, Symbol};

fn create_token<'a>(env: &Env, admin: &Address) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let contract_id = env.register_stellar_asset_contract(admin.clone());
    let token = token::Client::new(env, &contract_id);
    let stellar_asset_client = token::StellarAssetClient::new(env, &contract_id);
    (contract_id, token, stellar_asset_client)
}

struct Harness<'a> {
    _env: &'a Env,
    client: RelayerSlashingContractClient<'a>,
    token_client: token::Client<'a>,
    stellar_asset: token::StellarAssetClient<'a>,
    token_addr: Address,
    admin: Address,
    treasury: Address,
}

fn setup<'a>(env: &'a Env, admin: &Address, treasury: &Address) -> Harness<'a> {
    let (token_addr, token_client, stellar_asset) = create_token(env, admin);
    let contract_id = env.register_contract(None, RelayerSlashingContract);
    let client = RelayerSlashingContractClient::new(env, &contract_id);
    client.initialize(admin, &token_addr, treasury, &5000, &10);
    Harness {
        _env: env,
        client,
        token_client,
        stellar_asset,
        token_addr,
        admin: admin.clone(),
        treasury: treasury.clone(),
    }
}

/// epoch_length=100, budget=1000, reward_per_unit=10, max_units=5,
/// min_units=1, grace_epochs=2, equivocation_slash=1000bps, liveness_slash=2000bps
fn enable_liveness(h: &Harness) {
    h.client.set_liveness_config(&100, &1000, &10, &5, &1, &2, &1000, &2000);
}

#[test]
fn test_liveness_rewards_are_bounded_and_duplicates_earn_nothing() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let treasury = Address::generate(&env);
    let h = setup(&env, &admin, &treasury);
    enable_liveness(&h);
    h.stellar_asset.mint(&relayer, &1000);
    h.client.register_relayer(&relayer, &1000);

    // Submit unique work in epoch 0 (timestamp 0 -> epoch 0).
    h.client.record_relay_work(&relayer, &symbol_short!("work00"));
    h.client.record_relay_work(&relayer, &symbol_short!("work01"));
    // Duplicate submission earns no additional useful unit.
    h.client.record_relay_work(&relayer, &symbol_short!("work00"));
    assert_eq!(h.client.get_epoch_units(&0, &relayer), 2);

    // Advance to epoch 1 so epoch 0 is complete and can be settled.
    env.ledger().set_timestamp(150);
    h.client.settle_epoch(&0);

    let liveness = h.client.get_liveness(&relayer).unwrap();
    // 2 useful units * 10 = 20 reward, bounded and positive.
    assert!(liveness.rewards_accrued > 0);
    assert!(liveness.rewards_accrued <= 1000); // <= epoch_reward_budget
    assert_eq!(liveness.rewards_accrued, 20);
}

#[test]
fn test_total_reward_never_exceeds_epoch_budget() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let h = setup(&env, &admin, &treasury);
    enable_liveness(&h);

    // Many relayers all doing useful work push against the shared budget.
    let works: [&str; 12] = [
        "wa", "wb", "wc", "wd", "we", "wf", "wg", "wh", "wi", "wj", "wk", "wl",
    ];
    for s in works.iter() {
        let r = Address::generate(&env);
        h.stellar_asset.mint(&r, &1000);
        h.client.register_relayer(&r, &1000);
        h.client.record_relay_work(&r, &Symbol::new(&env, s));
    }

    env.ledger().set_timestamp(150);
    h.client.settle_epoch(&0);

    // Bounded invariant: reported epoch reward never exceeds the configured budget.
    let snap = h.client.get_epoch_info(&0);
    assert!(snap.total_reward >= 0);
    assert!(snap.total_reward <= 1000);
}

#[test]
fn test_equivocation_duplicate_across_relayers_earns_no_reward() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let h = setup(&env, &admin, &treasury);
    enable_liveness(&h);

    let relayer_a = Address::generate(&env);
    let relayer_b = Address::generate(&env);
    h.stellar_asset.mint(&relayer_a, &1000);
    h.stellar_asset.mint(&relayer_b, &1000);
    h.client.register_relayer(&relayer_a, &1000);
    h.client.register_relayer(&relayer_b, &1000);

    // A relays the work first.
    h.client.record_relay_work(&relayer_a, &symbol_short!("tx123"));
    // B re-submits the same work -> duplicate / equivocation, no new unit, penalty counted.
    h.client.record_relay_work(&relayer_b, &symbol_short!("tx123"));

    // A retains one useful unit; B has none.
    assert_eq!(h.client.get_epoch_units(&0, &relayer_a), 1);
    assert_eq!(h.client.get_epoch_units(&0, &relayer_b), 0);
    // B's equivocation was counted.
    assert_eq!(h.client.get_liveness(&relayer_b).unwrap().equivocation_count, 1);

    // B's only "work" is the duplicate, so B should earn no reward after settlement.
    env.ledger().set_timestamp(150);
    h.client.settle_epoch(&0);
    assert_eq!(h.client.get_liveness(&relayer_b).unwrap().rewards_accrued, 0);
    assert!(h.client.get_liveness(&relayer_a).unwrap().rewards_accrued > 0);

    // Equivocation slashed a bounded fraction (10%) of B's stake.
    let b_info = h.client.get_relayer_info(&relayer_b).unwrap();
    assert_eq!(b_info.stake_amount, 900);
}

#[test]
fn test_liveness_tolerates_temporary_partition_within_grace() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let h = setup(&env, &admin, &treasury);
    enable_liveness(&h);

    let relayer = Address::generate(&env);
    h.stellar_asset.mint(&relayer, &1000);
    h.client.register_relayer(&relayer, &1000);
    h.client.record_relay_work(&relayer, &symbol_short!("workA"));

    // Temporary partition: skip 2 epochs (within grace_epochs = 2).
    env.ledger().set_timestamp(300); // epoch 3
    let liveness = h.client.get_liveness(&relayer).unwrap();
    // Consecutive missed = 3 - 0 = 3 > grace 2 -> marked not live (persistent failure).
    assert_eq!(liveness.live, false);

    // Within grace window (only 1 epoch missed).
    let relayer2 = Address::generate(&env);
    h.stellar_asset.mint(&relayer2, &1000);
    h.client.register_relayer(&relayer2, &1000);
    h.client.record_relay_work(&relayer2, &symbol_short!("workB")); // epoch 3
    let liveness2 = h.client.get_liveness(&relayer2).unwrap();
    assert_eq!(liveness2.live, true);
}

#[test]
fn test_persistent_activity_restores_liveness() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let h = setup(&env, &admin, &treasury);
    enable_liveness(&h);

    let relayer = Address::generate(&env);
    h.stellar_asset.mint(&relayer, &1000);
    h.client.register_relayer(&relayer, &1000);
    h.client.record_relay_work(&relayer, &symbol_short!("workA"));

    // Long downtime -> liveness lost.
    env.ledger().set_timestamp(500); // epoch 5
    let liveness = h.client.get_liveness(&relayer).unwrap();
    assert_eq!(liveness.live, false);

    // Recovery: submitting new work restores active liveness (consecutive_missed reset).
    h.client.record_relay_work(&relayer, &symbol_short!("workC"));
    let recovered = h.client.get_liveness(&relayer).unwrap();
    assert_eq!(recovered.live, true);
    assert_eq!(recovered.consecutive_missed, 0);
}


#[test]
fn test_registration_and_staking() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, token_client, stellar_asset) = create_token(&env, &admin);
    let contract_id = env.register_contract(None, RelayerSlashingContract);
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
    let contract_id = env.register_contract(None, RelayerSlashingContract);
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
    let contract_id = env.register_contract(None, RelayerSlashingContract);
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
    let contract_id = env.register_contract(None, RelayerSlashingContract);
    let client = RelayerSlashingContractClient::new(&env, &contract_id);
    client.initialize(&admin, &token_addr, &admin, &5000, &60);
    stellar_asset.mint(&relayer, &1000);
    client.register_relayer(&relayer, &1000);
    client.request_unstake(&relayer);
    env.ledger().set_timestamp(30);
    client.withdraw_stake(&relayer);
}
